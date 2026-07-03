//! InterAgentMessage repository — a line of text injected from one instance into
//! another instance's live input (the inter-agent messaging backbone, M3.1).
//!
//! One `inter_agent_message` row is persisted per `message.inject` call. The
//! `status` reflects whether the text reached a live backend (`delivered`) or
//! was merely recorded because the target wasn't running (`queued`).
//!
//! # chain-builder usage
//!
//! [`create`] uses chain-builder for a single-table INSERT — same idiom as
//! `session::create_for_instance` / `workspace_agent::create`.
//!
//! [`list_for_instance`] is the M3.2 inbox/outbox reader. It uses raw
//! `sqlx::query_as` (the codebase's documented fallback — see
//! `agent_definition::list_with_counts`) because the filter is an OR across two
//! columns (`to_instance_id = ? OR from_instance_id = ?`), which chain-builder
//! 3.1's `where_eq` chain (AND-only) cannot express cleanly.

use super::cb_err;
use chain_builder::{QueryBuilder, Sqlite, Value as Bind};
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

// ── Row struct ──────────────────────────────────────────────────────────────

/// Decoded row from the `inter_agent_message` table.
///
/// `sqlx::FromRow` maps snake_case column names to snake_case fields.
/// `serde(rename_all = "camelCase")` emits camelCase JSON matching the
/// `InterAgentMessage` interface in `src/ipc/types.ts`.
///
/// SQLite stores booleans as INTEGER; sqlx maps the nullable `auto_submitted`
/// column to `Option<bool>` fine. `skip_serializing_if` keeps the key absent
/// (not `null`) when `None`, matching the optional `autoSubmitted?` TS field.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct InterAgentMessageRow {
    pub id: String,
    pub from_instance_id: String, // → "fromInstanceId"
    pub to_instance_id: String,   // → "toInstanceId"
    pub text: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_submitted: Option<bool>, // → "autoSubmitted"
    pub created_at: String, // → "createdAt"
}

// ── CRUD ────────────────────────────────────────────────────────────────────

/// Insert a new inter_agent_message and return the constructed row.
///
/// Generates a UUID v4 `id` and ISO-8601 UTC `created_at` timestamp.
/// `status` must be one of `"queued"` | `"delivered"` (the schema CHECK
/// enforces this). `auto_submitted` is always `true` for an injection.
///
/// Returns the row directly without a re-fetch round-trip (same as the other
/// repo `create` helpers).
pub async fn create(
    pool: &SqlitePool,
    from_instance_id: &str,
    to_instance_id: &str,
    text: &str,
    status: &str,
    auto_submitted: bool,
) -> sqlx::Result<InterAgentMessageRow> {
    let id = Uuid::new_v4().to_string();
    let created_at = Utc::now().to_rfc3339();
    let from_instance_id = from_instance_id.to_owned();
    let to_instance_id = to_instance_id.to_owned();
    let text = text.to_owned();
    let status = status.to_owned();

    QueryBuilder::<Sqlite>::table("inter_agent_message")
        .insert([
            ("id", Bind::Text(id.clone())),
            ("from_instance_id", Bind::Text(from_instance_id.clone())),
            ("to_instance_id", Bind::Text(to_instance_id.clone())),
            ("text", Bind::Text(text.clone())),
            ("status", Bind::Text(status.clone())),
            ("auto_submitted", Bind::Bool(auto_submitted)),
            ("created_at", Bind::Text(created_at.clone())),
        ])
        .execute(pool)
        .await
        .map_err(cb_err)?;

    Ok(InterAgentMessageRow {
        id,
        from_instance_id,
        to_instance_id,
        text,
        status,
        auto_submitted: Some(auto_submitted),
        created_at,
    })
}

/// List the inbox + outbox for one instance: every message where the instance
/// is either the sender OR the recipient, newest first, capped at `limit`.
///
/// Raw `sqlx::query_as` (not chain-builder): the `OR` across two columns is the
/// documented fallback case — chain-builder 3.1's `where_eq` chain only ANDs
/// conditions, so an inbox-or-outbox filter can't be built fluently. The column
/// list mirrors the `InterAgentMessageRow` fields (keep in sync with the table).
pub async fn list_for_instance(
    pool: &SqlitePool,
    instance_id: &str,
    limit: i64,
) -> sqlx::Result<Vec<InterAgentMessageRow>> {
    // Self-defend: SQLite treats a negative LIMIT as "unbounded". Callers
    // (message::list) already clamp, but the repo guards too so a future
    // crate-internal caller can't trigger an unbounded scan.
    let limit = limit.max(1);
    sqlx::query_as::<_, InterAgentMessageRow>(
        "SELECT id, from_instance_id, to_instance_id, text, status, auto_submitted, created_at \
         FROM inter_agent_message \
         WHERE to_instance_id = ?1 OR from_instance_id = ?1 \
         ORDER BY created_at DESC \
         LIMIT ?2",
    )
    .bind(instance_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// List a whole workspace's inter-agent traffic: every message whose sender
/// AND recipient both belong to the workspace, newest first, capped at
/// `limit`. Feeds the Chat Hub's merged view; per-conversation narrowing is
/// client-side.
///
/// Raw `sqlx::query_as` (not chain-builder): the membership filter is an
/// `IN (subquery)` on two columns — the documented fallback case, same as
/// `list_for_instance` above. The column list mirrors `InterAgentMessageRow`.
pub async fn list_for_workspace(
    pool: &SqlitePool,
    workspace_id: &str,
    limit: i64,
) -> sqlx::Result<Vec<InterAgentMessageRow>> {
    // Self-defend against a negative LIMIT (SQLite reads it as "unbounded"),
    // mirroring list_for_instance.
    let limit = limit.max(1);
    sqlx::query_as::<_, InterAgentMessageRow>(
        "SELECT id, from_instance_id, to_instance_id, text, status, auto_submitted, created_at \
         FROM inter_agent_message \
         WHERE from_instance_id IN (SELECT id FROM workspace_agent WHERE workspace_id = ?1) \
           AND to_instance_id   IN (SELECT id FROM workspace_agent WHERE workspace_id = ?1) \
         ORDER BY created_at DESC \
         LIMIT ?2",
    )
    .bind(workspace_id)
    .bind(limit)
    .fetch_all(pool)
    .await
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

    /// Helper: create a workspace + agent_definition, instantiate, and return
    /// the resulting `workspace_agent` id (a valid FK target).
    async fn fixture_instance(pool: &SqlitePool, name: &str) -> String {
        let ws = workspace::create(pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        let def = agent_definition::create(
            pool,
            &AgentDefinitionInput {
                name: name.into(),
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
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed");
        workspace_agent::instantiate(pool, &ws.id, &def.id)
            .await
            .expect("instantiate failed")
            .id
    }

    /// Helper: create an agent_definition and instantiate it into an EXISTING
    /// workspace — lets a test put two instances in the same workspace (the
    /// original `fixture_instance` creates a fresh workspace per call).
    async fn fixture_instance_in(pool: &SqlitePool, ws_id: &str, name: &str) -> String {
        let def = agent_definition::create(
            pool,
            &AgentDefinitionInput {
                name: name.into(),
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
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed");
        workspace_agent::instantiate(pool, ws_id, &def.id)
            .await
            .expect("instantiate failed")
            .id
    }

    /// list_for_workspace returns only messages whose BOTH endpoints are in
    /// the workspace — another workspace's traffic never leaks in.
    #[tokio::test]
    async fn list_for_workspace_scopes_to_the_workspace() {
        let pool = connect_in_memory().await;
        let ws1 = workspace::create(&pool, "WS1", "/tmp/ws1", None)
            .await
            .expect("ws1")
            .id;
        let ws2 = workspace::create(&pool, "WS2", "/tmp/ws2", None)
            .await
            .expect("ws2")
            .id;
        let a = fixture_instance_in(&pool, &ws1, "A").await;
        let b = fixture_instance_in(&pool, &ws1, "B").await;
        let x = fixture_instance_in(&pool, &ws2, "X").await;
        let y = fixture_instance_in(&pool, &ws2, "Y").await;
        create(&pool, &a, &b, "in ws1", "delivered", true)
            .await
            .expect("msg ws1");
        create(&pool, &x, &y, "in ws2", "delivered", true)
            .await
            .expect("msg ws2");

        let rows = list_for_workspace(&pool, &ws1, 50).await.expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].text, "in ws1");
    }

    /// Newest first, capped at `limit`.
    #[tokio::test]
    async fn list_for_workspace_orders_newest_first_and_limits() {
        let pool = connect_in_memory().await;
        let ws = workspace::create(&pool, "WS", "/tmp/ws", None)
            .await
            .expect("ws")
            .id;
        let a = fixture_instance_in(&pool, &ws, "A").await;
        let b = fixture_instance_in(&pool, &ws, "B").await;
        for i in 0..3 {
            create(&pool, &a, &b, &format!("m{i}"), "delivered", true)
                .await
                .expect("msg");
            // created_at is RFC3339 with sub-second precision; a tiny sleep
            // keeps the three timestamps strictly ordered.
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let rows = list_for_workspace(&pool, &ws, 2).await.expect("list");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].text, "m2");
        assert_eq!(rows[1].text, "m1");
    }

    /// create returns a fully-populated row with the given fields.
    #[tokio::test]
    async fn create_populates_row() {
        let pool = connect_in_memory().await;
        let from = fixture_instance(&pool, "Sender").await;
        let to = fixture_instance(&pool, "Target").await;

        let row = create(&pool, &from, &to, "hello", "delivered", true)
            .await
            .expect("create failed");

        assert_eq!(row.from_instance_id, from);
        assert_eq!(row.to_instance_id, to);
        assert_eq!(row.text, "hello");
        assert_eq!(row.status, "delivered");
        assert_eq!(row.auto_submitted, Some(true));
        assert!(!row.id.is_empty());
        assert!(!row.created_at.is_empty());
    }

    /// list_for_instance returns BOTH inbox (to=me) and outbox (from=me) rows
    /// for the queried instance, newest-first, and excludes unrelated messages.
    #[tokio::test]
    async fn list_for_instance_in_and_out_newest_first() {
        let pool = connect_in_memory().await;
        let a = fixture_instance(&pool, "Alpha").await;
        let b = fixture_instance(&pool, "Bravo").await;
        let c = fixture_instance(&pool, "Charlie").await;

        // Insert in a known chronological order (created_at is RFC3339 with
        // sub-second precision, so sequential awaits produce strictly-increasing
        // timestamps). Sleep a hair to guarantee distinct, orderable values.
        // 1. a → b  (outbox for a)
        let m1 = create(&pool, &a, &b, "first", "delivered", true)
            .await
            .expect("create m1 failed");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        // 2. b → a  (inbox for a)
        let m2 = create(&pool, &b, &a, "second", "delivered", true)
            .await
            .expect("create m2 failed");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        // 3. b → c  (unrelated to a)
        let _m3 = create(&pool, &b, &c, "third", "queued", true)
            .await
            .expect("create m3 failed");

        let rows = list_for_instance(&pool, &a, 50)
            .await
            .expect("list_for_instance failed");

        // Only a's two messages, the unrelated b→c excluded.
        assert_eq!(rows.len(), 2, "expected both in+out rows for a");
        // Newest first: m2 (inbox) before m1 (outbox).
        assert_eq!(rows[0].id, m2.id, "newest (inbox) first");
        assert_eq!(rows[1].id, m1.id, "older (outbox) second");
        // Both directions present.
        assert!(
            rows.iter().any(|r| r.to_instance_id == a),
            "inbox row present"
        );
        assert!(
            rows.iter().any(|r| r.from_instance_id == a),
            "outbox row present"
        );
    }

    /// The `limit` argument caps the returned row count.
    #[tokio::test]
    async fn list_for_instance_respects_limit() {
        let pool = connect_in_memory().await;
        let a = fixture_instance(&pool, "Alpha").await;
        let b = fixture_instance(&pool, "Bravo").await;

        for i in 0..4 {
            create(&pool, &a, &b, &format!("msg {i}"), "delivered", true)
                .await
                .expect("create failed");
            // Space inserts so created_at strictly increases (sub-second RFC3339
            // ties would make "which 2 rows" underdetermined below).
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        let rows = list_for_instance(&pool, &a, 2)
            .await
            .expect("list_for_instance failed");
        assert_eq!(rows.len(), 2, "limit must cap the row count");
        // ...and they must be the NEWEST two (msg 3, msg 2), not the oldest.
        assert_eq!(rows[0].text, "msg 3");
        assert_eq!(rows[1].text, "msg 2");
    }

    /// JSON serialization uses camelCase — locks the TS InterAgentMessage contract.
    #[tokio::test]
    async fn camel_case_contract() {
        let pool = connect_in_memory().await;
        let from = fixture_instance(&pool, "Sender").await;
        let to = fixture_instance(&pool, "Target").await;

        let row = create(&pool, &from, &to, "hi", "queued", true)
            .await
            .expect("create failed");

        let json = serde_json::to_value(&row).expect("serialize failed");

        // camelCase keys must be present
        assert!(
            json.get("fromInstanceId").is_some(),
            "must have fromInstanceId"
        );
        assert!(json.get("toInstanceId").is_some(), "must have toInstanceId");
        assert!(
            json.get("autoSubmitted").is_some(),
            "must have autoSubmitted"
        );
        assert!(json.get("createdAt").is_some(), "must have createdAt");

        // snake_case must NOT appear
        assert!(
            json.get("from_instance_id").is_none(),
            "must NOT have from_instance_id"
        );
        assert!(
            json.get("to_instance_id").is_none(),
            "must NOT have to_instance_id"
        );
        assert!(
            json.get("auto_submitted").is_none(),
            "must NOT have auto_submitted"
        );
        assert!(json.get("created_at").is_none(), "must NOT have created_at");
    }
}
