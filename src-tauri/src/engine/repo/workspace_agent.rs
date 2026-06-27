//! WorkspaceAgent repository — an "instance" of an AgentDefinition within a Workspace.
//!
//! One `workspace_agent` row is created per (workspace, agent_definition) pair.
//! The UNIQUE(workspace_id, agent_def_id) constraint enforces this at the DB level;
//! [`find`] is used by callers for idempotency before attempting an INSERT.
//!
//! # chain-builder usage
//!
//! All queries use chain-builder. Multiple `.where_eq()` calls are AND-joined
//! (chain-builder joins sibling predicates with `Conj::And` by default).
//! The `execute` / `fetch_*` methods accept any `sqlx::Executor`, including
//! `&mut SqliteConnection` obtained via `&mut *tx` from a `Transaction` —
//! see [`instantiate`] for the find-or-create transactional pattern shared by
//! `workspace.link` and `agentDef.addToWorkspace` (M1.2).

use super::cb_err;
use chain_builder::{Order, QueryBuilder, Sqlite, Value as Bind};
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

// ── Row struct ──────────────────────────────────────────────────────────────

/// Decoded row from the `workspace_agent` table.
///
/// `sqlx::FromRow` maps snake_case column names to snake_case fields.
/// `serde(rename_all = "camelCase")` emits camelCase JSON matching the
/// `WorkspaceAgent` interface in `src/ipc/types.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceAgentRow {
    pub id: String,
    pub workspace_id: String, // → "workspaceId"
    pub agent_def_id: String, // → "agentDefId"
    pub status: String,
    pub added_at: String, // → "addedAt"
}

// ── Column list (shared by SELECT queries) ───────────────────────────────────

const COLS: [&str; 5] = ["id", "workspace_id", "agent_def_id", "status", "added_at"];

// ── CRUD ────────────────────────────────────────────────────────────────────

/// Insert a new workspace_agent and return the constructed row.
///
/// Generates a UUID v4 `id` and ISO-8601 UTC `added_at` timestamp.
/// `status` should be `"idle"` for newly-linked agents (the schema CHECK
/// allows `running | idle | waiting`).
///
/// Returns the row directly without a re-fetch round-trip.
///
/// Note: production handlers (`workspace.link`, `agentDef.addToWorkspace`) go
/// through [`instantiate`], which pairs this INSERT with the session INSERT in
/// one transaction. This standalone `create` is test-only.
#[allow(dead_code)] // test-only; production handlers use `instantiate`
pub async fn create(
    pool: &SqlitePool,
    workspace_id: &str,
    agent_def_id: &str,
    status: &str,
) -> sqlx::Result<WorkspaceAgentRow> {
    let id = Uuid::new_v4().to_string();
    let added_at = Utc::now().to_rfc3339();
    let workspace_id = workspace_id.to_owned();
    let agent_def_id = agent_def_id.to_owned();
    let status = status.to_owned();

    QueryBuilder::<Sqlite>::table("workspace_agent")
        .insert([
            ("id", Bind::Text(id.clone())),
            ("workspace_id", Bind::Text(workspace_id.clone())),
            ("agent_def_id", Bind::Text(agent_def_id.clone())),
            ("status", Bind::Text(status.clone())),
            ("added_at", Bind::Text(added_at.clone())),
        ])
        .execute(pool)
        .await
        .map_err(cb_err)?;

    Ok(WorkspaceAgentRow {
        id,
        workspace_id,
        agent_def_id,
        status,
        added_at,
    })
}

/// Fetch a single workspace_agent by primary key `id`, or `None` if not found.
pub async fn get(pool: &SqlitePool, id: &str) -> sqlx::Result<Option<WorkspaceAgentRow>> {
    QueryBuilder::<Sqlite>::table("workspace_agent")
        .select(COLS)
        .where_eq("id", id)
        .fetch_optional::<WorkspaceAgentRow, _>(pool)
        .await
        .map_err(cb_err)
}

/// Return all workspace_agents for a workspace, ordered by `added_at` asc with `id`
/// as a stable tie-breaker.
pub async fn list_by_workspace(
    pool: &SqlitePool,
    workspace_id: &str,
) -> sqlx::Result<Vec<WorkspaceAgentRow>> {
    QueryBuilder::<Sqlite>::table("workspace_agent")
        .select(COLS)
        .where_eq("workspace_id", workspace_id)
        .order_by("added_at", Order::Asc)
        .order_by("id", Order::Asc)
        .fetch_all::<WorkspaceAgentRow, _>(pool)
        .await
        .map_err(cb_err)
}

/// Find the workspace_agent for a specific (workspace_id, agent_def_id) pair.
///
/// Returns `None` if the pair has not been linked yet. Used by
/// `add_to_workspace` for idempotency before attempting an INSERT that
/// would violate the UNIQUE(workspace_id, agent_def_id) constraint.
///
/// Multiple `.where_eq()` calls are AND-joined by chain-builder.
pub async fn find(
    pool: &SqlitePool,
    workspace_id: &str,
    agent_def_id: &str,
) -> sqlx::Result<Option<WorkspaceAgentRow>> {
    QueryBuilder::<Sqlite>::table("workspace_agent")
        .select(COLS)
        .where_eq("workspace_id", workspace_id)
        .where_eq("agent_def_id", agent_def_id)
        .fetch_optional::<WorkspaceAgentRow, _>(pool)
        .await
        .map_err(cb_err)
}

/// Return `true` if a workspace_agent with the given `id` exists.
///
/// Used by M2 spawn handler to validate the target instance before spawning.
// M2 spawn will call this; suppress the dead_code lint until then.
#[allow(dead_code)] // called by M2 instance.spawn
pub async fn exists(pool: &SqlitePool, id: &str) -> sqlx::Result<bool> {
    get(pool, id).await.map(|opt| opt.is_some())
}

/// Idempotently ensure a `workspace_agent` + `session` pair exists for the
/// given `(workspace_id, agent_def_id)` and return the `workspace_agent` row.
///
/// - If the pair is already linked, the existing row is returned unchanged
///   (no duplicate INSERT).
/// - Otherwise, a `workspace_agent` + `session` are created atomically in a
///   single SQLite transaction so a failed session INSERT cannot leave an
///   orphan instance.
///
/// # Preconditions
///
/// **Does not validate** that `workspace_id` or `agent_def_id` exist in their
/// respective tables — callers must check existence before calling this
/// function (foreign-key enforcement also guards against invalid ids at the DB
/// level, but caller-side validation gives cleaner `NotFound` errors).
///
/// # Usage
///
/// Called from both `commands::agent::add_to_workspace` (per workspace_id in
/// the loop) and `commands::workspace::link` (per agentDefId in the payload).
/// This is the single source of truth for the atomic instance+session creation
/// logic — do NOT duplicate the transaction in either handler.
pub async fn instantiate(
    pool: &SqlitePool,
    workspace_id: &str,
    agent_def_id: &str,
) -> sqlx::Result<WorkspaceAgentRow> {
    // ── Idempotency: reuse existing instance if already linked ──────────
    if let Some(existing) = find(pool, workspace_id, agent_def_id).await? {
        return Ok(existing);
    }

    // ── Atomically create workspace_agent + session ──────────────────────
    //
    // Raw sqlx inside a transaction — mirrors the pattern in db::migrate.
    // `super::session::DEFAULT_CONTEXT_LIMIT` is the single source of truth
    // for the initial context window size.
    let wa_id = Uuid::new_v4().to_string();
    let added_at = Utc::now().to_rfc3339();
    let session_id = Uuid::new_v4().to_string();
    let started_at = Utc::now().to_rfc3339();

    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT INTO workspace_agent \
         (id, workspace_id, agent_def_id, status, added_at) \
         VALUES (?, ?, ?, 'idle', ?)",
    )
    .bind(&wa_id)
    .bind(workspace_id)
    .bind(agent_def_id)
    .bind(&added_at)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO session \
         (id, workspace_agent_id, context_tokens, context_limit, started_at, last_active_at) \
         VALUES (?, ?, 0, ?, ?, NULL)",
    )
    .bind(&session_id)
    .bind(&wa_id)
    .bind(super::session::DEFAULT_CONTEXT_LIMIT)
    .bind(&started_at)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(WorkspaceAgentRow {
        id: wa_id,
        workspace_id: workspace_id.to_owned(),
        agent_def_id: agent_def_id.to_owned(),
        status: "idle".to_owned(),
        added_at,
    })
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{
        db::connect_in_memory,
        repo::{
            agent_definition::{self, AgentDefinitionInput},
            workspace,
        },
    };

    /// Helper: insert a workspace + agent_definition and return their ids.
    async fn fixtures(pool: &SqlitePool) -> (String, String) {
        let ws = workspace::create(pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        let def = agent_definition::create(
            pool,
            &AgentDefinitionInput {
                name: "TestAgent".into(),
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
        (ws.id, def.id)
    }

    /// create → get round-trip: every field is preserved.
    #[tokio::test]
    async fn create_then_get_roundtrip() {
        let pool = connect_in_memory().await;
        let (ws_id, def_id) = fixtures(&pool).await;

        let row = create(&pool, &ws_id, &def_id, "idle")
            .await
            .expect("create failed");

        assert_eq!(row.workspace_id, ws_id);
        assert_eq!(row.agent_def_id, def_id);
        assert_eq!(row.status, "idle");
        assert!(!row.id.is_empty());
        assert!(!row.added_at.is_empty());

        let fetched = get(&pool, &row.id)
            .await
            .expect("get failed")
            .expect("row should exist");
        assert_eq!(fetched, row);
    }

    /// list_by_workspace returns only rows for the given workspace, in order.
    #[tokio::test]
    async fn list_by_workspace_scoped() {
        let pool = connect_in_memory().await;
        let (ws_id, def_id) = fixtures(&pool).await;

        // Second workspace + second agent def
        let ws2 = workspace::create(&pool, "WS2", "/tmp/ws2", None)
            .await
            .expect("create ws2 failed");
        let def2 = agent_definition::create(
            &pool,
            &AgentDefinitionInput {
                name: "Agent2".into(),
                role: None,
                agent_type: "chat".into(),
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
        .expect("create def2 failed");

        let row1 = create(&pool, &ws_id, &def_id, "idle")
            .await
            .expect("create row1 failed");
        let _row2 = create(&pool, &ws2.id, &def2.id, "idle")
            .await
            .expect("create row2 failed");

        let ws1_rows = list_by_workspace(&pool, &ws_id)
            .await
            .expect("list_by_workspace failed");
        assert_eq!(ws1_rows.len(), 1, "ws1 should have exactly 1 agent");
        assert_eq!(ws1_rows[0].id, row1.id);

        let ws2_rows = list_by_workspace(&pool, &ws2.id)
            .await
            .expect("list_by_workspace failed");
        assert_eq!(ws2_rows.len(), 1, "ws2 should have exactly 1 agent");
    }

    /// find() returns Some for a linked pair, None for unknown combinations.
    #[tokio::test]
    async fn find_returns_existing_instance() {
        let pool = connect_in_memory().await;
        let (ws_id, def_id) = fixtures(&pool).await;

        // Before create: None
        assert!(find(&pool, &ws_id, &def_id)
            .await
            .expect("find before create failed")
            .is_none());

        let row = create(&pool, &ws_id, &def_id, "idle")
            .await
            .expect("create failed");

        // After create: Some with matching id
        let found = find(&pool, &ws_id, &def_id)
            .await
            .expect("find after create failed")
            .expect("should find after create");
        assert_eq!(found.id, row.id);

        // Wrong workspace or wrong def → None
        assert!(find(&pool, "no-such-ws", &def_id)
            .await
            .expect("find with bad ws")
            .is_none());
        assert!(find(&pool, &ws_id, "no-such-def")
            .await
            .expect("find with bad def")
            .is_none());
    }

    /// exists() returns true for a known id, false otherwise.
    #[tokio::test]
    async fn exists_true_false() {
        let pool = connect_in_memory().await;
        let (ws_id, def_id) = fixtures(&pool).await;

        assert!(!exists(&pool, "no-such-id")
            .await
            .expect("exists pre-create"));

        let row = create(&pool, &ws_id, &def_id, "idle")
            .await
            .expect("create failed");
        assert!(exists(&pool, &row.id).await.expect("exists post-create"));
        assert!(!exists(&pool, "wrong-id").await.expect("exists wrong-id"));
    }

    /// instantiate() creates workspace_agent + session atomically; returns the row.
    #[tokio::test]
    async fn instantiate_creates_instance_and_session() {
        let pool = connect_in_memory().await;
        let (ws_id, def_id) = fixtures(&pool).await;

        let row = instantiate(&pool, &ws_id, &def_id)
            .await
            .expect("instantiate failed");

        assert_eq!(row.workspace_id, ws_id);
        assert_eq!(row.agent_def_id, def_id);
        assert_eq!(row.status, "idle");

        // Session must have been created alongside the workspace_agent.
        let session = crate::engine::repo::session::get_by_instance(&pool, &row.id)
            .await
            .expect("get_by_instance failed")
            .expect("session should exist after instantiate");
        assert_eq!(session.workspace_agent_id, row.id);
        assert_eq!(
            session.context_limit,
            Some(crate::engine::repo::session::DEFAULT_CONTEXT_LIMIT)
        );
    }

    /// instantiate() is idempotent: a second call returns the existing row.
    #[tokio::test]
    async fn instantiate_is_idempotent() {
        let pool = connect_in_memory().await;
        let (ws_id, def_id) = fixtures(&pool).await;

        let first = instantiate(&pool, &ws_id, &def_id)
            .await
            .expect("first instantiate failed");
        let second = instantiate(&pool, &ws_id, &def_id)
            .await
            .expect("second instantiate failed");

        assert_eq!(first.id, second.id, "idempotent: same row id returned");

        // Still only one workspace_agent row in the DB.
        let all = list_by_workspace(&pool, &ws_id).await.expect("list failed");
        assert_eq!(
            all.len(),
            1,
            "must be exactly 1 workspace_agent after two instantiate calls"
        );
    }

    /// JSON serialization uses camelCase — locks the TS WorkspaceAgent contract.
    #[tokio::test]
    async fn camel_case_contract() {
        let pool = connect_in_memory().await;
        let (ws_id, def_id) = fixtures(&pool).await;

        let row = create(&pool, &ws_id, &def_id, "idle")
            .await
            .expect("create failed");

        let json = serde_json::to_value(&row).expect("serialize failed");

        // camelCase keys must be present
        assert!(json.get("workspaceId").is_some(), "must have workspaceId");
        assert!(json.get("agentDefId").is_some(), "must have agentDefId");
        assert!(json.get("addedAt").is_some(), "must have addedAt");

        // snake_case must NOT appear
        assert!(
            json.get("workspace_id").is_none(),
            "must NOT have workspace_id"
        );
        assert!(
            json.get("agent_def_id").is_none(),
            "must NOT have agent_def_id"
        );
        assert!(json.get("added_at").is_none(), "must NOT have added_at");
    }
}
