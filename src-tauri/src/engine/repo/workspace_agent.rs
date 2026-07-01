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

/// Like [`WorkspaceAgentRow`] but annotated with the paired session's
/// `launched_skill_ids` (raw JSON-array text, `None` before any launch). Used
/// ONLY by `commands::instance::list` so the Roster can detect skill drift
/// without a second IPC round-trip per instance.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct WorkspaceAgentWithSkills {
    pub id: String,
    pub workspace_id: String,
    pub agent_def_id: String,
    pub status: String,
    pub added_at: String,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_launched_ids"
    )]
    pub launched_skill_ids: Option<String>,
}

/// Same shape as `session::serialize_json_text` — kept local (small, trivial,
/// different module) rather than shared.
#[allow(dead_code)]
fn serialize_launched_ids<S>(opt: &Option<String>, ser: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match opt {
        Some(text) => serde_json::from_str::<serde_json::Value>(text)
            .unwrap_or(serde_json::Value::Null)
            .serialize(ser),
        None => ser.serialize_none(),
    }
}

/// Return all workspace_agents for a workspace (same ordering as
/// `list_by_workspace`) LEFT JOINed with their session's `launched_skill_ids`.
/// A workspace_agent with no session yet (should not normally happen —
/// `instantiate` creates both atomically) yields `None`, same as one whose
/// session simply hasn't launched.
#[allow(dead_code)]
pub async fn list_by_workspace_with_launched_skills(
    pool: &SqlitePool,
    workspace_id: &str,
) -> sqlx::Result<Vec<WorkspaceAgentWithSkills>> {
    sqlx::query_as::<_, WorkspaceAgentWithSkills>(
        "SELECT wa.id, wa.workspace_id, wa.agent_def_id, wa.status, wa.added_at, \
         sess.launched_skill_ids \
         FROM workspace_agent wa \
         LEFT JOIN session sess ON sess.workspace_agent_id = wa.id \
         WHERE wa.workspace_id = ? \
         ORDER BY wa.added_at ASC, wa.id ASC",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await
}

/// All instances of a given agent definition, across every workspace.
///
/// Used when deleting an agent definition: each instance must be removed
/// (FK-safe) before the definition row can be deleted.
pub async fn list_by_agent_def(
    pool: &SqlitePool,
    agent_def_id: &str,
) -> sqlx::Result<Vec<WorkspaceAgentRow>> {
    QueryBuilder::<Sqlite>::table("workspace_agent")
        .select(COLS)
        .where_eq("agent_def_id", agent_def_id)
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
/// Validate an instance id exists. Called by `instance.spawn` (target check)
/// and by `message.inject` (sender + target validation).
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

/// Update the status of a workspace_agent. status must be one of
/// 'running' | 'idle' | 'waiting' (DB CHECK enforces this).
///
/// Uses chain-builder UPDATE with `where_eq("id", …)`. Called by the M2
/// runtime handlers (`instance.spawn` / `instance.stop`) to persist the
/// lifecycle state that mirrors the in-memory [`crate::engine::runtime`]
/// registry.
///
/// # Precondition
///
/// A non-existent `id` is a silent no-op (0 rows updated → `Ok(())`); this
/// function does NOT report "not found". Callers must validate existence first
/// (the spawn/stop handlers do, via `exists` / the runtime registry).
pub async fn set_status(pool: &SqlitePool, id: &str, status: &str) -> sqlx::Result<()> {
    QueryBuilder::<Sqlite>::table("workspace_agent")
        .update([("status", Bind::Text(status.to_owned()))])
        .where_eq("id", id)
        .execute(pool)
        .await
        .map_err(cb_err)?;
    Ok(())
}

/// Remove a workspace_agent (an agent's instance in a workspace) and everything
/// that hangs off it. Returns `true` if a row was deleted.
///
/// The agent's `session` (and that session's `message`s and `snapshot`s) cascade
/// via `ON DELETE CASCADE`. But several tables reference `workspace_agent` with
/// the default `NO ACTION`, which would abort the final DELETE if any such row
/// exists — so we clean those first, inside one transaction with the delete:
///   - `inter_agent_message` (NOT NULL from/to) and `blackboard_activity`
///     (NOT NULL instance) → delete the dependent rows
///   - `message.from_instance_id`, `blackboard_entry.last_writer_id`,
///     `fusion_panel_response.instance_id` (all nullable) → null the back-pointer
///
/// Raw sqlx (not chain-builder) because these are cross-table maintenance
/// statements, not entity CRUD. Foreign keys are enabled on the pool, so the
/// cascade fires for the owned rows.
pub async fn remove(pool: &SqlitePool, id: &str) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM inter_agent_message WHERE from_instance_id = ? OR to_instance_id = ?")
        .bind(id)
        .bind(id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM blackboard_activity WHERE instance_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("UPDATE message SET from_instance_id = NULL WHERE from_instance_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("UPDATE blackboard_entry SET last_writer_id = NULL WHERE last_writer_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("UPDATE fusion_panel_response SET instance_id = NULL WHERE instance_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    let res = sqlx::query("DELETE FROM workspace_agent WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(res.rows_affected() > 0)
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
                ..Default::default()
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

    /// remove() deletes the instance (and cascades its session), returns true;
    /// a second remove of the same id returns false.
    #[tokio::test]
    async fn remove_deletes_instance_and_cascades_session() {
        let pool = connect_in_memory().await;
        let (ws_id, def_id) = fixtures(&pool).await;

        // instantiate creates the workspace_agent + its session atomically.
        let inst = instantiate(&pool, &ws_id, &def_id)
            .await
            .expect("instantiate failed");
        assert!(
            crate::engine::repo::session::get_by_instance(&pool, &inst.id)
                .await
                .expect("session lookup failed")
                .is_some()
        );

        let removed = remove(&pool, &inst.id).await.expect("remove failed");
        assert!(removed, "first remove should report a deleted row");

        // The instance is gone, and its session cascaded away.
        assert!(!exists(&pool, &inst.id).await.expect("exists failed"));
        assert!(
            crate::engine::repo::session::get_by_instance(&pool, &inst.id)
                .await
                .expect("session lookup failed")
                .is_none()
        );

        // Removing again is a no-op.
        let again = remove(&pool, &inst.id).await.expect("second remove failed");
        assert!(!again, "second remove should report no row");
    }

    /// remove() must clear every non-cascading reference to the instance so the
    /// final DELETE can't trip a FK constraint, and must leave rows in OTHER
    /// sessions intact (back-pointers nulled, not the rows deleted). Seeds one
    /// row in each referencing table and asserts the outcome. This is the
    /// regression guard: a future FK added without updating remove() breaks here.
    #[tokio::test]
    async fn remove_clears_all_references() {
        let pool = connect_in_memory().await;
        let (ws_id, def_id) = fixtures(&pool).await;

        // Two instances: `a` is the one we remove; `b` survives and owns the
        // "other session" rows that should be nulled rather than deleted.
        let a = instantiate(&pool, &ws_id, &def_id)
            .await
            .expect("instantiate a");
        let def2 = agent_definition::create(
            &pool,
            &AgentDefinitionInput {
                name: "Agent2".into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create def2");
        let b = instantiate(&pool, &ws_id, &def2.id)
            .await
            .expect("instantiate b");
        let b_session = crate::engine::repo::session::get_by_instance(&pool, &b.id)
            .await
            .expect("b session lookup")
            .expect("b has a session");

        let ts = "2026-01-01T00:00:00Z";

        // 1. inter_agent_message in BOTH directions (NOT NULL refs → must delete).
        for (from, to) in [(&a.id, &b.id), (&b.id, &a.id)] {
            sqlx::query(
                "INSERT INTO inter_agent_message \
                 (id, from_instance_id, to_instance_id, text, status, created_at) \
                 VALUES (?, ?, ?, 'hi', 'queued', ?)",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(from)
            .bind(to)
            .bind(ts)
            .execute(&pool)
            .await
            .expect("seed inter_agent_message");
        }

        // 2. blackboard_entry (last_writer_id = a → null) + activity (instance = a → delete).
        let entry_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO blackboard_entry (id, workspace_id, key, value, last_writer_id, updated_at) \
             VALUES (?, ?, 'k', 'v', ?, ?)",
        )
        .bind(&entry_id)
        .bind(&ws_id)
        .bind(&a.id)
        .bind(ts)
        .execute(&pool)
        .await
        .expect("seed blackboard_entry");
        sqlx::query(
            "INSERT INTO blackboard_activity (id, entry_id, instance_id, action, at) \
             VALUES (?, ?, ?, 'write', ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&entry_id)
        .bind(&a.id)
        .bind(ts)
        .execute(&pool)
        .await
        .expect("seed blackboard_activity");

        // 3. message in B's session with from_instance_id = a (→ null, row survives).
        let msg_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO message (id, session_id, role, text, from_instance_id, created_at) \
             VALUES (?, ?, 'agent', 'x', ?, ?)",
        )
        .bind(&msg_id)
        .bind(&b_session.id)
        .bind(&a.id)
        .bind(ts)
        .execute(&pool)
        .await
        .expect("seed message");

        // 4. fusion_run in B's session + response with instance_id = a (→ null).
        let run_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO fusion_run (id, session_id, prompt, created_at) VALUES (?, ?, 'p', ?)",
        )
        .bind(&run_id)
        .bind(&b_session.id)
        .bind(ts)
        .execute(&pool)
        .await
        .expect("seed fusion_run");
        let resp_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO fusion_panel_response (id, fusion_run_id, instance_id, status, created_at) \
             VALUES (?, ?, ?, 'done', ?)",
        )
        .bind(&resp_id)
        .bind(&run_id)
        .bind(&a.id)
        .bind(ts)
        .execute(&pool)
        .await
        .expect("seed fusion_panel_response");

        // ── Remove `a` ───────────────────────────────────────────────────────
        let removed = remove(&pool, &a.id).await.expect("remove a");
        assert!(removed);
        assert!(!exists(&pool, &a.id).await.expect("exists a"));

        // Deleted: inter_agent_message + blackboard_activity referencing a.
        let iam: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM inter_agent_message WHERE from_instance_id = ? OR to_instance_id = ?",
        )
        .bind(&a.id)
        .bind(&a.id)
        .fetch_one(&pool)
        .await
        .expect("count iam");
        assert_eq!(iam, 0, "inter_agent_messages should be deleted");
        let act: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM blackboard_activity WHERE instance_id = ?")
                .bind(&a.id)
                .fetch_one(&pool)
                .await
                .expect("count activity");
        assert_eq!(act, 0, "blackboard_activity should be deleted");

        // Nulled (rows survive): message, blackboard_entry, fusion_panel_response.
        let msg_writer: Option<String> =
            sqlx::query_scalar("SELECT from_instance_id FROM message WHERE id = ?")
                .bind(&msg_id)
                .fetch_one(&pool)
                .await
                .expect("fetch message");
        assert!(
            msg_writer.is_none(),
            "message.from_instance_id should be NULL"
        );
        let entry_writer: Option<String> =
            sqlx::query_scalar("SELECT last_writer_id FROM blackboard_entry WHERE id = ?")
                .bind(&entry_id)
                .fetch_one(&pool)
                .await
                .expect("fetch entry");
        assert!(
            entry_writer.is_none(),
            "blackboard_entry.last_writer_id should be NULL"
        );
        let resp_inst: Option<String> =
            sqlx::query_scalar("SELECT instance_id FROM fusion_panel_response WHERE id = ?")
                .bind(&resp_id)
                .fetch_one(&pool)
                .await
                .expect("fetch response");
        assert!(
            resp_inst.is_none(),
            "fusion_panel_response.instance_id should be NULL"
        );
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
                ..Default::default()
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

    /// set_status() persists a new status that survives a re-fetch.
    #[tokio::test]
    async fn set_status_updates_row() {
        let pool = connect_in_memory().await;
        let (ws_id, def_id) = fixtures(&pool).await;

        let row = instantiate(&pool, &ws_id, &def_id)
            .await
            .expect("instantiate failed");
        assert_eq!(row.status, "idle");

        set_status(&pool, &row.id, "running")
            .await
            .expect("set_status failed");

        let fetched = get(&pool, &row.id)
            .await
            .expect("get failed")
            .expect("row should exist");
        assert_eq!(fetched.status, "running");
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

    /// list_by_workspace_with_launched_skills LEFT JOINs the paired session's
    /// launched_skill_ids — present after a launch snapshot, NULL before one.
    #[tokio::test]
    async fn list_by_workspace_with_launched_skills_joins_session() {
        let pool = connect_in_memory().await;
        let ws = crate::engine::repo::workspace::create(&pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        let def = crate::engine::repo::agent_definition::create(
            &pool,
            &crate::engine::repo::agent_definition::AgentDefinitionInput {
                name: "A".into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed");
        let inst = instantiate(&pool, &ws.id, &def.id)
            .await
            .expect("instantiate failed");
        let session = crate::engine::repo::session::get_by_instance(&pool, &inst.id)
            .await
            .expect("get session failed")
            .expect("session exists");

        // Before any launch snapshot: NULL.
        let before = list_by_workspace_with_launched_skills(&pool, &ws.id)
            .await
            .expect("query failed");
        assert_eq!(before.len(), 1);
        assert!(before[0].launched_skill_ids.is_none());

        crate::engine::repo::session::set_launched_skill_ids(
            &pool,
            &session.id,
            &["sk-1".to_string()],
        )
        .await
        .expect("set failed");

        let after = list_by_workspace_with_launched_skills(&pool, &ws.id)
            .await
            .expect("query failed");
        assert_eq!(after[0].launched_skill_ids.as_deref(), Some(r#"["sk-1"]"#));
    }
}
