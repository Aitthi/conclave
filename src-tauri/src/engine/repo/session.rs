//! Session repository — one session per workspace_agent (1:1, UNIQUE(workspace_agent_id)).
//!
//! A session tracks the rolling context window for a single workspace_agent instance.
//! It is created atomically alongside its parent workspace_agent in M1.5's
//! `add_to_workspace` handler (see `commands::agent`).
//!
//! # chain-builder usage
//!
//! Both `create_for_instance` and `get_by_instance` use chain-builder.
//! No raw sqlx is needed: queries are single-table / single-condition.
//!
//! # Context limit constant
//!
//! [`DEFAULT_CONTEXT_LIMIT`] (200 000 tokens) is the compile-time default applied
//! to every new session. The `add_to_workspace` handler mirrors this value in its
//! raw-sqlx transaction INSERT (kept in sync via a comment there).

use super::cb_err;
use chain_builder::{QueryBuilder, Sqlite, Value as Bind};
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

/// Default context-window limit for new sessions (200 000 tokens).
///
/// Mirrored as the literal `200000` in the raw-sqlx INSERT inside
/// `commands::agent::add_to_workspace` — keep both in sync if changed.
#[allow(dead_code)] // referenced by session tests and future runtime handlers
pub const DEFAULT_CONTEXT_LIMIT: i64 = 200_000;

// ── Row struct ──────────────────────────────────────────────────────────────

/// Decoded row from the `session` table.
///
/// `sqlx::FromRow` maps snake_case column names to snake_case fields.
/// `serde(rename_all = "camelCase")` emits camelCase JSON matching the
/// `Session` interface in `src/ipc/types.ts`.
/// Optional columns use `skip_serializing_if` to match `field?: T` TS types
/// (key absent when `None`, not `null`).
#[allow(dead_code)] // constructed by create_for_instance (not yet called from handlers)
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SessionRow {
    pub id: String,
    pub workspace_agent_id: String, // → "workspaceAgentId"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<i64>, // → "contextTokens"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_limit: Option<i64>, // → "contextLimit"
    pub started_at: String,         // → "startedAt"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_active_at: Option<String>, // → "lastActiveAt"
}

// ── Column list ──────────────────────────────────────────────────────────────

const COLS: [&str; 6] = [
    "id",
    "workspace_agent_id",
    "context_tokens",
    "context_limit",
    "started_at",
    "last_active_at",
];

// ── CRUD ────────────────────────────────────────────────────────────────────

/// Create a session for a workspace_agent and return the constructed row.
// Not yet called from production handlers — add_to_workspace uses raw sqlx
// inside a transaction. Future read-path callers (M2 session.get) will use this.
#[allow(dead_code)]
///
/// - `id`: UUID v4
/// - `started_at`: now (UTC ISO-8601)
/// - `context_tokens`: 0 (fresh session, no tokens consumed)
/// - `context_limit`: [`DEFAULT_CONTEXT_LIMIT`]
/// - `last_active_at`: NULL (no activity yet)
///
/// The UNIQUE(workspace_agent_id) constraint means calling this twice for
/// the same `workspace_agent_id` returns a `sqlx::Error::Database`
/// unique-violation rather than silently inserting a duplicate.
///
/// In production, this is called inside a transaction in `add_to_workspace`
/// (via raw sqlx) so the workspace_agent + session pair is created atomically.
/// This pool-based helper is used by tests and future callers that don't
/// need transactional coupling.
pub async fn create_for_instance(
    pool: &SqlitePool,
    workspace_agent_id: &str,
) -> sqlx::Result<SessionRow> {
    let id = Uuid::new_v4().to_string();
    let started_at = Utc::now().to_rfc3339();
    let workspace_agent_id = workspace_agent_id.to_owned();

    QueryBuilder::<Sqlite>::table("session")
        .insert([
            ("id", Bind::Text(id.clone())),
            ("workspace_agent_id", Bind::Text(workspace_agent_id.clone())),
            ("context_tokens", Bind::I64(0)),
            ("context_limit", Bind::I64(DEFAULT_CONTEXT_LIMIT)),
            ("started_at", Bind::Text(started_at.clone())),
            ("last_active_at", Bind::Null),
        ])
        .execute(pool)
        .await
        .map_err(cb_err)?;

    Ok(SessionRow {
        id,
        workspace_agent_id,
        context_tokens: Some(0),
        context_limit: Some(DEFAULT_CONTEXT_LIMIT),
        started_at,
        last_active_at: None,
    })
}

/// Fetch the session for a given workspace_agent, or `None` if not yet created.
///
/// Reached in production from `commands::instance::spawn`.
pub async fn get_by_instance(
    pool: &SqlitePool,
    workspace_agent_id: &str,
) -> sqlx::Result<Option<SessionRow>> {
    QueryBuilder::<Sqlite>::table("session")
        .select(COLS)
        .where_eq("workspace_agent_id", workspace_agent_id)
        .fetch_optional::<SessionRow, _>(pool)
        .await
        .map_err(cb_err)
}

/// Fetch a session by its primary-key `id`, or `None` if no such session.
///
/// Mirror of [`get_by_instance`] keyed on the session `id` instead of the
/// owning `workspace_agent_id`. Used by `commands::message::send` to resolve a
/// session before routing stdin to its live backend.
pub async fn get(pool: &SqlitePool, id: &str) -> sqlx::Result<Option<SessionRow>> {
    QueryBuilder::<Sqlite>::table("session")
        .select(COLS)
        .where_eq("id", id)
        .fetch_optional::<SessionRow, _>(pool)
        .await
        .map_err(cb_err)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{
        db::connect_in_memory,
        repo::{
            agent_definition::{self, AgentDefinitionInput},
            workspace, workspace_agent,
        },
    };

    /// Helper: create a workspace_agent and return its id.
    async fn fixture_instance(pool: &SqlitePool) -> String {
        let ws = workspace::create(pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        let def = agent_definition::create(
            pool,
            &AgentDefinitionInput {
                name: "SessionTestAgent".into(),
                role: None,
                agent_type: "cli".into(),
                cli_kind: None,
                color: None,
                provider_id: None,
                model: None,
                harness_mode: "own".into(),
                share_blackboard: None,
                auto_submit_injected: None,
                allowed_senders: None,
            },
        )
        .await
        .expect("create agent_def failed");
        workspace_agent::create(pool, &ws.id, &def.id, "idle")
            .await
            .expect("create workspace_agent failed")
            .id
    }

    /// create_for_instance → get_by_instance round-trip: all fields preserved.
    #[tokio::test]
    async fn create_then_get_roundtrip() {
        let pool = connect_in_memory().await;
        let wa_id = fixture_instance(&pool).await;

        let row = create_for_instance(&pool, &wa_id)
            .await
            .expect("create_for_instance failed");

        assert_eq!(row.workspace_agent_id, wa_id);
        assert_eq!(row.context_tokens, Some(0));
        assert_eq!(row.context_limit, Some(DEFAULT_CONTEXT_LIMIT));
        assert!(row.last_active_at.is_none());
        assert!(!row.started_at.is_empty());
        assert!(!row.id.is_empty());

        let fetched = get_by_instance(&pool, &wa_id)
            .await
            .expect("get_by_instance failed")
            .expect("session should exist after create");
        assert_eq!(fetched, row);
    }

    /// create_for_instance → get (by session id) round-trip: same row, and an
    /// unknown id yields `None`.
    #[tokio::test]
    async fn get_by_id_roundtrip() {
        let pool = connect_in_memory().await;
        let wa_id = fixture_instance(&pool).await;

        let row = create_for_instance(&pool, &wa_id)
            .await
            .expect("create_for_instance failed");

        let fetched = get(&pool, &row.id)
            .await
            .expect("get failed")
            .expect("session should exist for its own id");
        assert_eq!(fetched, row);

        let missing = get(&pool, "no-such-session-id").await.expect("get failed");
        assert!(missing.is_none(), "unknown session id must yield None");
    }

    /// UNIQUE(workspace_agent_id) is enforced — creating a second session for
    /// the same instance must return an error.
    #[tokio::test]
    async fn unique_per_instance_enforced() {
        let pool = connect_in_memory().await;
        let wa_id = fixture_instance(&pool).await;

        create_for_instance(&pool, &wa_id)
            .await
            .expect("first create_for_instance failed");

        let second = create_for_instance(&pool, &wa_id).await;
        assert!(
            second.is_err(),
            "second session for same workspace_agent must fail (UNIQUE constraint)"
        );
    }

    /// JSON serialization uses camelCase — locks the TS Session contract.
    #[tokio::test]
    async fn camel_case_contract() {
        let pool = connect_in_memory().await;
        let wa_id = fixture_instance(&pool).await;

        let row = create_for_instance(&pool, &wa_id)
            .await
            .expect("create failed");

        let json = serde_json::to_value(&row).expect("serialize failed");

        // camelCase keys present
        assert!(
            json.get("workspaceAgentId").is_some(),
            "must have workspaceAgentId"
        );
        assert!(
            json.get("contextTokens").is_some(),
            "must have contextTokens"
        );
        assert!(json.get("contextLimit").is_some(), "must have contextLimit");
        assert!(json.get("startedAt").is_some(), "must have startedAt");

        // lastActiveAt is None → skip_serializing_if → key absent
        assert!(
            json.get("lastActiveAt").is_none(),
            "lastActiveAt must be absent from JSON when None"
        );

        // snake_case must NOT appear
        assert!(
            json.get("workspace_agent_id").is_none(),
            "must NOT have workspace_agent_id"
        );
        assert!(
            json.get("context_tokens").is_none(),
            "must NOT have context_tokens"
        );
        assert!(
            json.get("context_limit").is_none(),
            "must NOT have context_limit"
        );
        assert!(json.get("started_at").is_none(), "must NOT have started_at");
    }
}
