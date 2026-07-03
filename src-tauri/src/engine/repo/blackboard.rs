//! Blackboard repository — the per-workspace shared key/value store plus an
//! activity log attributing each read/write to an instance (M3.3).
//!
//! Two tables back this module:
//!   - `blackboard_entry` — one row per (workspace_id, key); the current value.
//!   - `blackboard_activity` — an append-only log of `read`/`write` actions, each
//!     attributed to a `workspace_agent` instance (NOT NULL `instance_id`).
//!
//! # value encoding
//!
//! The `value` column is TEXT and holds a JSON **string** (the caller passes a
//! `serde_json::to_string(value)` result). The COMMAND layer parses it back to a
//! real `serde_json::Value` for the wire shape — so the row's raw `value` is
//! deliberately NOT serialized here (it would emit a string-of-JSON).
//!
//! # chain-builder usage
//!
//! [`get`] and [`list`] are single-table SELECTs and use chain-builder, mirroring
//! `session` / `workspace_agent`. [`set`] uses raw `sqlx` because it is an UPSERT
//! (`INSERT … ON CONFLICT … DO UPDATE … RETURNING`) which chain-builder 3.1 has no
//! API for — the same documented-fallback rationale as
//! `inter_agent_message::list_for_instance`. [`list_activity`] also uses raw
//! `sqlx` because it JOINs `blackboard_activity` to `blackboard_entry` to scope by
//! workspace.

use super::cb_err;
use chain_builder::{Order, QueryBuilder, Sqlite};
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

// ── Row structs ──────────────────────────────────────────────────────────────

/// Decoded row from the `blackboard_entry` table.
///
/// `sqlx::FromRow` maps snake_case columns to snake_case fields. The raw `value`
/// is a JSON **string**; the command layer parses it into a real JSON value for
/// the `BlackboardEntry` wire shape, so `value` is `#[serde(skip)]`-ed here to
/// avoid accidentally emitting a string-of-JSON.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct BlackboardEntryRow {
    pub id: String,
    pub workspace_id: String, // → "workspaceId"
    pub key: String,
    /// Raw JSON-string text from the TEXT column. NOT serialized — the command
    /// layer parses this into a real JSON value for the wire response.
    #[serde(skip)]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_writer_id: Option<String>, // → "lastWriterId"
    pub updated_at: String, // → "updatedAt"
}

/// Decoded row from the `blackboard_activity` table.
///
/// Unlike [`BlackboardEntryRow`], this serializes directly to the wire — it has
/// no special fields. `action` is always `"read"` or `"write"` (DB CHECK).
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct BlackboardActivityRow {
    pub id: String,
    pub entry_id: String,    // → "entryId"
    pub instance_id: String, // → "instanceId"
    pub action: String,
    pub at: String,
}

// ── Column list (shared by chain-builder SELECTs) ────────────────────────────

const ENTRY_COLS: [&str; 6] = [
    "id",
    "workspace_id",
    "key",
    "value",
    "last_writer_id",
    "updated_at",
];

// ── CRUD ────────────────────────────────────────────────────────────────────

/// UPSERT a blackboard entry by (workspace_id, key) and return the resulting row.
///
/// Raw `sqlx` (not chain-builder): this is an `INSERT … ON CONFLICT … DO UPDATE
/// … RETURNING`, which chain-builder 3.1 cannot express — the same documented
/// fallback case as `inter_agent_message::list_for_instance`. A fresh UUID is
/// generated for the INSERT `id`; on conflict the existing row id is kept (the
/// generated one is discarded), so `RETURNING id` yields the stable entry id.
///
/// `value_json` is the already-serialized JSON string to store; `writer_id` is
/// the attributing instance (`None` for a user write → NULL `last_writer_id`).
///
/// When `writer_id` is `Some`, a `blackboard_activity` `write` row is also
/// appended. A user write (`writer_id == None`) logs NO activity, because
/// `blackboard_activity.instance_id` is NOT NULL — only agent writes are
/// attributable.
pub async fn set(
    pool: &SqlitePool,
    workspace_id: &str,
    key: &str,
    value_json: &str,
    writer_id: Option<&str>,
) -> sqlx::Result<BlackboardEntryRow> {
    let new_id = Uuid::new_v4().to_string();
    let updated_at = Utc::now().to_rfc3339();

    // Upsert + write-log commit atomically: a failed activity insert must not
    // leave a durable entry behind a returned error (the caller would see "set
    // failed" while the value was actually written, dropping the attribution).
    let mut tx = pool.begin().await?;

    let row = sqlx::query_as::<_, BlackboardEntryRow>(
        "INSERT INTO blackboard_entry \
         (id, workspace_id, key, value, last_writer_id, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(workspace_id, key) DO UPDATE SET \
            value = excluded.value, \
            last_writer_id = excluded.last_writer_id, \
            updated_at = excluded.updated_at \
         RETURNING id, workspace_id, key, value, last_writer_id, updated_at",
    )
    .bind(&new_id)
    .bind(workspace_id)
    .bind(key)
    .bind(value_json)
    .bind(writer_id)
    .bind(&updated_at)
    .fetch_one(&mut *tx)
    .await?;

    // Attribute the write only when an agent wrote it. A user write
    // (writer_id == None) cannot be logged: instance_id is NOT NULL.
    if let Some(writer) = writer_id {
        log_activity(&mut *tx, &row.id, writer, "write").await?;
    }

    tx.commit().await?;
    Ok(row)
}

/// Fetch a blackboard entry by (workspace_id, key), or `None` if absent.
///
/// chain-builder (single-table AND filter). When `reader_id` is `Some` AND the
/// entry exists, a `blackboard_activity` `read` row is appended attributing the
/// read to that instance.
pub async fn get(
    pool: &SqlitePool,
    workspace_id: &str,
    key: &str,
    reader_id: Option<&str>,
) -> sqlx::Result<Option<BlackboardEntryRow>> {
    let row = QueryBuilder::<Sqlite>::table("blackboard_entry")
        .select(ENTRY_COLS)
        .where_eq("workspace_id", workspace_id)
        .where_eq("key", key)
        .fetch_optional::<BlackboardEntryRow, _>(pool)
        .await
        .map_err(cb_err)?;

    if let (Some(entry), Some(reader)) = (row.as_ref(), reader_id) {
        log_activity(pool, &entry.id, reader, "read").await?;
    }

    Ok(row)
}

/// List all entries for a workspace, newest-first by `updated_at`.
///
/// chain-builder (single-table filter + order).
pub async fn list(pool: &SqlitePool, workspace_id: &str) -> sqlx::Result<Vec<BlackboardEntryRow>> {
    QueryBuilder::<Sqlite>::table("blackboard_entry")
        .select(ENTRY_COLS)
        .where_eq("workspace_id", workspace_id)
        .order_by("updated_at", Order::Desc)
        .fetch_all::<BlackboardEntryRow, _>(pool)
        .await
        .map_err(cb_err)
}

/// Delete a blackboard entry by (workspace_id, key). Returns `true` when a row
/// was deleted, `false` when the key was absent.
///
/// Raw `sqlx` DELETE, mirroring `skill::delete`. The entry's activity rows go
/// with it via `blackboard_activity.entry_id ON DELETE CASCADE` — deleting a
/// key deliberately deletes its read/write history too (that's what "delete"
/// means here; keep the key and overwrite its value if the history matters).
pub async fn delete(pool: &SqlitePool, workspace_id: &str, key: &str) -> sqlx::Result<bool> {
    let res = sqlx::query("DELETE FROM blackboard_entry WHERE workspace_id = ?1 AND key = ?2")
        .bind(workspace_id)
        .bind(key)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// List recent activity across THIS workspace's entries, newest-first, capped at
/// `limit`.
///
/// Raw `sqlx` (not chain-builder): the JOIN of `blackboard_activity` to
/// `blackboard_entry` (to scope activity by `workspace_id`) is the documented
/// fallback case. `limit` is guarded `>= 1` here too — SQLite treats a negative
/// LIMIT as "unbounded", so the repo self-defends even though the command layer
/// clamps.
pub async fn list_activity(
    pool: &SqlitePool,
    workspace_id: &str,
    limit: i64,
) -> sqlx::Result<Vec<BlackboardActivityRow>> {
    let limit = limit.max(1);
    sqlx::query_as::<_, BlackboardActivityRow>(
        "SELECT a.id, a.entry_id, a.instance_id, a.action, a.at \
         FROM blackboard_activity a \
         JOIN blackboard_entry e ON a.entry_id = e.id \
         WHERE e.workspace_id = ?1 \
         ORDER BY a.at DESC \
         LIMIT ?2",
    )
    .bind(workspace_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Append a `blackboard_activity` row. `action` is `"read"` or `"write"`.
///
/// Generic over the executor so it can run on a `&SqlitePool` (the read path)
/// OR inside a transaction (`&mut *tx`, the write path) — see [`set`], where the
/// upsert + write-log must commit atomically.
async fn log_activity<'e, E>(
    executor: E,
    entry_id: &str,
    instance_id: &str,
    action: &str,
) -> sqlx::Result<()>
where
    // `sqlx::Sqlite` explicitly: this module imports `chain_builder::Sqlite` for
    // the QueryBuilder, so a bare `Sqlite` would bind to the wrong type.
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "INSERT INTO blackboard_activity (id, entry_id, instance_id, action, at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(entry_id)
    .bind(instance_id)
    .bind(action)
    .bind(Utc::now().to_rfc3339())
    .execute(executor)
    .await?;
    Ok(())
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

    /// Create a workspace and return its id.
    async fn fixture_workspace(pool: &SqlitePool) -> String {
        workspace::create(pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed")
            .id
    }

    /// Instantiate an instance in `workspace_id` and return its id.
    async fn fixture_instance(pool: &SqlitePool, workspace_id: &str, name: &str) -> String {
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
        workspace_agent::instantiate(pool, workspace_id, &def.id)
            .await
            .expect("instantiate failed")
            .id
    }

    /// set → get round-trips the stored JSON-string value verbatim.
    #[tokio::test]
    async fn set_get_roundtrip() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool).await;

        let stored = serde_json::to_string(&serde_json::json!({ "a": [1, 2, 3] })).unwrap();
        set(&pool, &ws, "k", &stored, None)
            .await
            .expect("set failed");

        let row = get(&pool, &ws, "k", None)
            .await
            .expect("get failed")
            .expect("entry exists");
        assert_eq!(row.value.as_deref(), Some(stored.as_str()));
        assert_eq!(row.workspace_id, ws);
        assert_eq!(row.key, "k");
    }

    /// set is an UPSERT: a second write to the same (ws,key) updates value +
    /// last_writer_id and does NOT create a second entry row.
    #[tokio::test]
    async fn set_is_upsert() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool).await;
        let agent = fixture_instance(&pool, &ws, "Writer").await;

        let r1 = set(&pool, &ws, "k", "\"first\"", None)
            .await
            .expect("first set failed");
        let r2 = set(&pool, &ws, "k", "\"second\"", Some(&agent))
            .await
            .expect("second set failed");

        assert_eq!(r1.id, r2.id, "UPSERT keeps the same entry id");
        assert_eq!(r2.value.as_deref(), Some("\"second\""));
        assert_eq!(r2.last_writer_id.as_deref(), Some(agent.as_str()));

        let all = list(&pool, &ws).await.expect("list failed");
        assert_eq!(all.len(), 1, "UPSERT must not create a second row");
    }

    /// An agent write logs a 'write' activity; a user write logs none.
    #[tokio::test]
    async fn write_activity_only_for_agents() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool).await;
        let agent = fixture_instance(&pool, &ws, "Writer").await;

        // User write — no activity.
        set(&pool, &ws, "u", "\"x\"", None)
            .await
            .expect("user set failed");
        assert!(
            list_activity(&pool, &ws, 50)
                .await
                .expect("list_activity failed")
                .is_empty(),
            "user write must log no activity"
        );

        // Agent write — one 'write' activity.
        set(&pool, &ws, "a", "\"y\"", Some(&agent))
            .await
            .expect("agent set failed");
        let acts = list_activity(&pool, &ws, 50).await.expect("list_activity");
        assert_eq!(acts.len(), 1);
        assert_eq!(acts[0].action, "write");
        assert_eq!(acts[0].instance_id, agent);
    }

    /// get with a reader logs a 'read'; without a reader logs none; missing key
    /// → None.
    #[tokio::test]
    async fn read_activity_and_missing_key() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool).await;
        let reader = fixture_instance(&pool, &ws, "Reader").await;
        set(&pool, &ws, "k", "\"v\"", None).await.expect("set");

        // Read without reader_id → no activity.
        get(&pool, &ws, "k", None).await.expect("get");
        assert!(list_activity(&pool, &ws, 50).await.unwrap().is_empty());

        // Read with reader_id → one 'read'.
        get(&pool, &ws, "k", Some(&reader)).await.expect("get");
        let acts = list_activity(&pool, &ws, 50).await.unwrap();
        assert_eq!(acts.len(), 1);
        assert_eq!(acts[0].action, "read");
        assert_eq!(acts[0].instance_id, reader);

        // Missing key → None, and no activity logged for it.
        assert!(get(&pool, &ws, "nope", Some(&reader))
            .await
            .expect("get missing")
            .is_none());
        assert_eq!(list_activity(&pool, &ws, 50).await.unwrap().len(), 1);
    }

    /// delete removes the row (and cascades its activity); missing key → false;
    /// other workspaces' same-named keys untouched.
    #[tokio::test]
    async fn delete_removes_row_and_activity() {
        let pool = connect_in_memory().await;
        let ws1 = fixture_workspace(&pool).await;
        let ws2 = fixture_workspace(&pool).await;
        let agent = fixture_instance(&pool, &ws1, "Writer").await;

        set(&pool, &ws1, "k", "\"v\"", Some(&agent))
            .await
            .expect("set ws1");
        set(&pool, &ws2, "k", "\"v2\"", None).await.expect("set ws2");
        assert_eq!(list_activity(&pool, &ws1, 50).await.unwrap().len(), 1);

        assert!(delete(&pool, &ws1, "k").await.expect("delete"));
        assert!(get(&pool, &ws1, "k", None).await.unwrap().is_none());
        // Activity cascaded away with the entry.
        assert!(list_activity(&pool, &ws1, 50).await.unwrap().is_empty());
        // Same key in another workspace survives.
        assert!(get(&pool, &ws2, "k", None).await.unwrap().is_some());
        // Deleting again → false (already gone).
        assert!(!delete(&pool, &ws1, "k").await.expect("re-delete"));
    }

    /// list is workspace-scoped and newest-first.
    #[tokio::test]
    async fn list_scoped_newest_first() {
        let pool = connect_in_memory().await;
        let ws1 = fixture_workspace(&pool).await;
        let ws2 = fixture_workspace(&pool).await;

        set(&pool, &ws1, "a", "1", None).await.expect("set a");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        set(&pool, &ws1, "b", "2", None).await.expect("set b");
        set(&pool, &ws2, "c", "3", None).await.expect("set c");

        let rows = list(&pool, &ws1).await.expect("list failed");
        assert_eq!(rows.len(), 2, "only ws1 entries");
        assert_eq!(rows[0].key, "b", "newest first");
        assert_eq!(rows[1].key, "a");
    }
}
