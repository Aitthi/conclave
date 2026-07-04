//! Task repository (ADR 0008) — first-class work items with an append-only
//! event log and per-agent watch subscriptions.
//!
//! Three tables back this module:
//!   - `task` — one row per (workspace_id, slug); current state + metadata.
//!   - `task_event` — append-only history (`note`/`state`/`gate`/`challenge`/
//!     `ruling`), each optionally attributed to an actor agent.
//!   - `task_watch` — (task_id, agent_id) subscriptions (Lane B notify hook).
//!
//! # state machine
//!
//! [`valid_transition`] is the single source of truth for legal `task.state`
//! moves. Every state-changing operation ([`set_state`], [`claim`], [`close`])
//! routes through it inside a transaction, so "merged → claimed" (etc.) is
//! rejected consistently everywhere, not just in `task.setState`.
//!
//! # value encoding
//!
//! `file_boundary` and `task_event.payload` are stored as TEXT columns holding
//! serialized JSON (arrays / objects respectively) — the same "stringly
//! stored, richly returned" split as `blackboard_entry.value`. The command
//! layer parses them back into real `serde_json::Value`s for the wire shape;
//! this module returns the raw TEXT so it never needs `serde_json` types in
//! its own signatures.

use super::cb_err;
use chain_builder::{Order, QueryBuilder, Sqlite};
use chrono::Utc;
use serde::Serialize;
use sqlx::{Sqlite as SqliteDb, SqlitePool, Transaction};
use uuid::Uuid;

// ── Row structs ──────────────────────────────────────────────────────────────

/// Decoded row from the `task` table. `file_boundary` is raw JSON-array TEXT;
/// the command layer parses it into a real JSON array for the wire shape
/// (`#[serde(skip)]` here, same rationale as `BlackboardEntryRow::value`).
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct TaskRow {
    pub id: String,
    pub workspace_id: String,
    pub slug: String,
    pub title: String,
    pub state: String,
    pub owner_agent_id: Option<String>,
    pub implementer_agent_id: Option<String>,
    #[serde(skip)]
    pub file_boundary: String,
    pub design_canon: Option<String>,
    pub plan: String,
    pub created_at: String,
    pub updated_at: String,
}

/// A [`TaskRow`] plus its `task_event` count — the shape `task.list` returns
/// (frozen: "each row = frozen task shape PLUS `eventCount`").
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TaskListRow {
    pub id: String,
    pub workspace_id: String,
    pub slug: String,
    pub title: String,
    pub state: String,
    pub owner_agent_id: Option<String>,
    pub implementer_agent_id: Option<String>,
    pub file_boundary: String,
    pub design_canon: Option<String>,
    pub plan: String,
    pub created_at: String,
    pub updated_at: String,
    pub event_count: i64,
}

/// Decoded row from the `task_event` table. `payload` is raw JSON-object TEXT
/// (parsed by the command layer; unparseable → `{}` per the frozen wire
/// contract — never a panic on a corrupt row).
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct TaskEventRow {
    pub id: String,
    pub task_id: String,
    pub kind: String,
    pub actor_agent_id: Option<String>,
    #[serde(skip)]
    pub payload: String,
    pub created_at: String,
}

const TASK_COLS: [&str; 12] = [
    "id",
    "workspace_id",
    "slug",
    "title",
    "state",
    "owner_agent_id",
    "implementer_agent_id",
    "file_boundary",
    "design_canon",
    "plan",
    "created_at",
    "updated_at",
];

// ── State machine ────────────────────────────────────────────────────────────

/// The single source of truth for legal `task.state` transitions (ADR 0008).
/// No self-transitions (a no-op "change" is not a state-machine move — note
/// or gate events cover that instead). Terminal states (`merged`,
/// `abandoned`) have no outgoing edges.
pub fn valid_transition(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        ("planned", "claimed")
            | ("planned", "abandoned")
            | ("claimed", "in_progress")
            | ("claimed", "abandoned")
            | ("in_progress", "review")
            | ("in_progress", "abandoned")
            | ("review", "merged")
            | ("review", "in_progress")
            | ("review", "abandoned")
    )
}

/// A task-mutating operation failed for a business reason (as opposed to a
/// raw DB error) — the command layer maps these to [`AppError::NotFound`] /
/// [`AppError::Invalid`] (`crate::engine::AppError`, not imported here to
/// keep this module DB-error-shaped like every other repo module).
#[derive(Debug)]
pub enum TaskOpError {
    NotFound,
    Invalid(String),
    Db(sqlx::Error),
}

impl From<sqlx::Error> for TaskOpError {
    fn from(e: sqlx::Error) -> Self {
        TaskOpError::Db(e)
    }
}

// ── CRUD ────────────────────────────────────────────────────────────────────

/// Fields accepted by [`create`]. A struct (rather than a long positional arg
/// list) because `task.create` carries five optional-ish inputs beyond the
/// required workspace/slug/title.
pub struct NewTask<'a> {
    pub workspace_id: &'a str,
    pub slug: &'a str,
    pub title: &'a str,
    pub owner_agent_id: Option<&'a str>,
    pub file_boundary_json: &'a str, // pre-serialized JSON array text, e.g. "[]"
    pub design_canon: Option<&'a str>,
    pub plan: &'a str,
}

/// Insert a new task and return the constructed row. Does NOT check
/// `(workspace_id, slug)` uniqueness itself — the `UNIQUE` constraint raises a
/// `sqlx::Error` the command layer maps to a friendly `AppError::Invalid`
/// (mirrors `memory_chunk`'s content-hash uniqueness handling).
pub async fn create(pool: &SqlitePool, input: NewTask<'_>) -> sqlx::Result<TaskRow> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO task \
         (id, workspace_id, slug, title, state, owner_agent_id, implementer_agent_id, \
          file_boundary, design_canon, plan, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, 'planned', ?5, NULL, ?6, ?7, ?8, ?9, ?9)",
    )
    .bind(&id)
    .bind(input.workspace_id)
    .bind(input.slug)
    .bind(input.title)
    .bind(input.owner_agent_id)
    .bind(input.file_boundary_json)
    .bind(input.design_canon)
    .bind(input.plan)
    .bind(&now)
    .execute(pool)
    .await?;

    Ok(TaskRow {
        id,
        workspace_id: input.workspace_id.to_owned(),
        slug: input.slug.to_owned(),
        title: input.title.to_owned(),
        state: "planned".to_owned(),
        owner_agent_id: input.owner_agent_id.map(str::to_owned),
        implementer_agent_id: None,
        file_boundary: input.file_boundary_json.to_owned(),
        design_canon: input.design_canon.map(str::to_owned),
        plan: input.plan.to_owned(),
        created_at: now.clone(),
        updated_at: now,
    })
}

/// Fetch a task by `(workspace_id, slug)`, or `None` if absent.
pub async fn get(pool: &SqlitePool, workspace_id: &str, slug: &str) -> sqlx::Result<Option<TaskRow>> {
    QueryBuilder::<Sqlite>::table("task")
        .select(TASK_COLS)
        .where_eq("workspace_id", workspace_id)
        .where_eq("slug", slug)
        .fetch_optional::<TaskRow, _>(pool)
        .await
        .map_err(cb_err)
}

/// List a workspace's tasks (optionally filtered by `state`), each carrying
/// its `task_event` count, newest-updated first.
///
/// Raw `sqlx` (not chain-builder): a `LEFT JOIN … GROUP BY` aggregate, the
/// same documented fallback case as `blackboard::list_activity`.
pub async fn list(
    pool: &SqlitePool,
    workspace_id: &str,
    state: Option<&str>,
) -> sqlx::Result<Vec<TaskListRow>> {
    if let Some(state) = state {
        sqlx::query_as::<_, TaskListRow>(
            "SELECT t.id, t.workspace_id, t.slug, t.title, t.state, t.owner_agent_id, \
             t.implementer_agent_id, t.file_boundary, t.design_canon, t.plan, \
             t.created_at, t.updated_at, COUNT(e.id) AS event_count \
             FROM task t LEFT JOIN task_event e ON e.task_id = t.id \
             WHERE t.workspace_id = ?1 AND t.state = ?2 \
             GROUP BY t.id ORDER BY t.updated_at DESC",
        )
        .bind(workspace_id)
        .bind(state)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, TaskListRow>(
            "SELECT t.id, t.workspace_id, t.slug, t.title, t.state, t.owner_agent_id, \
             t.implementer_agent_id, t.file_boundary, t.design_canon, t.plan, \
             t.created_at, t.updated_at, COUNT(e.id) AS event_count \
             FROM task t LEFT JOIN task_event e ON e.task_id = t.id \
             WHERE t.workspace_id = ?1 \
             GROUP BY t.id ORDER BY t.updated_at DESC",
        )
        .bind(workspace_id)
        .fetch_all(pool)
        .await
    }
}

/// The last `limit` events for a task, newest-first (frozen: `task.get` = "last
/// 20 events, sorted createdAt DESC").
pub async fn events_for(pool: &SqlitePool, task_id: &str, limit: i64) -> sqlx::Result<Vec<TaskEventRow>> {
    QueryBuilder::<Sqlite>::table("task_event")
        .select([
            "id",
            "task_id",
            "kind",
            "actor_agent_id",
            "payload",
            "created_at",
        ])
        .where_eq("task_id", task_id)
        .order_by("created_at", Order::Desc)
        .limit(limit.max(1))
        .fetch_all::<TaskEventRow, _>(pool)
        .await
        .map_err(cb_err)
}

/// Every `gate`/`challenge`/`ruling` event across a workspace's tasks,
/// oldest-first. Backs `task.list`'s derived board fields (`lastGate`,
/// `challenges`) with ONE query instead of one `task.get` per row (the plan's
/// "list rows stay O(tasks), never O(events)" — this is O(relevant events)
/// across the whole workspace, not O(tasks) round trips).
///
/// Raw `sqlx` (not chain-builder): a `JOIN` filtered by workspace through
/// `task`, the same documented fallback as `blackboard::list_activity`.
pub async fn board_events_for_workspace(
    pool: &SqlitePool,
    workspace_id: &str,
) -> sqlx::Result<Vec<TaskEventRow>> {
    sqlx::query_as::<_, TaskEventRow>(
        "SELECT e.id, e.task_id, e.kind, e.actor_agent_id, e.payload, e.created_at \
         FROM task_event e JOIN task t ON t.id = e.task_id \
         WHERE t.workspace_id = ?1 AND e.kind IN ('gate', 'challenge', 'ruling') \
         ORDER BY e.created_at ASC",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await
}

/// Append one `task_event` row and return it. Generic over the executor so it
/// can run on a `&SqlitePool` OR inside a transaction (mirrors
/// `blackboard::log_activity`).
async fn append_event<'e, E>(
    executor: E,
    task_id: &str,
    kind: &str,
    actor_agent_id: Option<&str>,
    payload_json: &str,
) -> sqlx::Result<TaskEventRow>
where
    E: sqlx::Executor<'e, Database = SqliteDb>,
{
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO task_event (id, task_id, kind, actor_agent_id, payload, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(&id)
    .bind(task_id)
    .bind(kind)
    .bind(actor_agent_id)
    .bind(payload_json)
    .bind(&now)
    .execute(executor)
    .await?;

    Ok(TaskEventRow {
        id,
        task_id: task_id.to_owned(),
        kind: kind.to_owned(),
        actor_agent_id: actor_agent_id.map(str::to_owned),
        payload: payload_json.to_owned(),
        created_at: now,
    })
}

/// Append a `note` event to a task identified by `(workspace_id, slug)`.
pub async fn add_note(
    pool: &SqlitePool,
    workspace_id: &str,
    slug: &str,
    actor_agent_id: Option<&str>,
    payload_json: &str,
) -> Result<TaskEventRow, TaskOpError> {
    let task = get(pool, workspace_id, slug).await?.ok_or(TaskOpError::NotFound)?;
    Ok(append_event(pool, &task.id, "note", actor_agent_id, payload_json).await?)
}

/// Append a `gate` event — non-zero exit codes are recorded too (a red gate is
/// evidence, not an error).
pub async fn add_gate(
    pool: &SqlitePool,
    workspace_id: &str,
    slug: &str,
    actor_agent_id: Option<&str>,
    payload_json: &str,
) -> Result<TaskEventRow, TaskOpError> {
    let task = get(pool, workspace_id, slug).await?.ok_or(TaskOpError::NotFound)?;
    Ok(append_event(pool, &task.id, "gate", actor_agent_id, payload_json).await?)
}

/// Append a `challenge` event.
pub async fn add_challenge(
    pool: &SqlitePool,
    workspace_id: &str,
    slug: &str,
    actor_agent_id: Option<&str>,
    payload_json: &str,
) -> Result<TaskEventRow, TaskOpError> {
    let task = get(pool, workspace_id, slug).await?.ok_or(TaskOpError::NotFound)?;
    Ok(append_event(pool, &task.id, "challenge", actor_agent_id, payload_json).await?)
}

/// Append a `ruling` event.
pub async fn add_ruling(
    pool: &SqlitePool,
    workspace_id: &str,
    slug: &str,
    actor_agent_id: Option<&str>,
    payload_json: &str,
) -> Result<TaskEventRow, TaskOpError> {
    let task = get(pool, workspace_id, slug).await?.ok_or(TaskOpError::NotFound)?;
    Ok(append_event(pool, &task.id, "ruling", actor_agent_id, payload_json).await?)
}

/// Transition a task's state, validating the move via [`valid_transition`]
/// and recording a `state` event (`payload: {"from","to"}`). Runs inside a
/// transaction so the read-validate-write is atomic (no lost-update race
/// between two concurrent transitions).
pub async fn set_state(
    pool: &SqlitePool,
    workspace_id: &str,
    slug: &str,
    to: &str,
    actor_agent_id: Option<&str>,
) -> Result<TaskRow, TaskOpError> {
    let mut tx = pool.begin().await?;
    let current = fetch_for_update(&mut tx, workspace_id, slug)
        .await?
        .ok_or(TaskOpError::NotFound)?;

    if !valid_transition(&current.state, to) {
        return Err(TaskOpError::Invalid(format!(
            "cannot transition '{}' -> '{to}'",
            current.state
        )));
    }

    apply_state_change(&mut tx, &current, to, actor_agent_id).await?;
    tx.commit().await?;
    get(pool, workspace_id, slug).await?.ok_or(TaskOpError::NotFound)
}

/// Claim a task: `planned -> claimed` plus `implementer_agent_id = actor`.
/// Routes through [`valid_transition`] like every other move, so claiming a
/// non-`planned` task (already claimed, merged, …) is rejected the same way
/// an invalid `task.setState` would be.
pub async fn claim(
    pool: &SqlitePool,
    workspace_id: &str,
    slug: &str,
    actor_agent_id: &str,
) -> Result<TaskRow, TaskOpError> {
    let mut tx = pool.begin().await?;
    let current = fetch_for_update(&mut tx, workspace_id, slug)
        .await?
        .ok_or(TaskOpError::NotFound)?;

    if !valid_transition(&current.state, "claimed") {
        return Err(TaskOpError::Invalid(format!(
            "cannot claim a task in state '{}'",
            current.state
        )));
    }

    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE task SET state = 'claimed', implementer_agent_id = ?1, updated_at = ?2 WHERE id = ?3",
    )
    .bind(actor_agent_id)
    .bind(&now)
    .bind(&current.id)
    .execute(&mut *tx)
    .await?;
    append_event(
        &mut *tx,
        &current.id,
        "state",
        Some(actor_agent_id),
        &serde_json::json!({"from": current.state, "to": "claimed"}).to_string(),
    )
    .await?;

    tx.commit().await?;
    get(pool, workspace_id, slug).await?.ok_or(TaskOpError::NotFound)
}

/// Close a task: `{claimed,in_progress,review} -> merged`. Unlike
/// [`valid_transition`]'s single-hop table, close accepts any of the three
/// live working states directly to `merged` (matching the plan's "state→
/// merged" acceptance, without forcing every close through `review` first).
pub async fn close(
    pool: &SqlitePool,
    workspace_id: &str,
    slug: &str,
    actor_agent_id: Option<&str>,
) -> Result<TaskRow, TaskOpError> {
    let mut tx = pool.begin().await?;
    let current = fetch_for_update(&mut tx, workspace_id, slug)
        .await?
        .ok_or(TaskOpError::NotFound)?;

    if !matches!(current.state.as_str(), "claimed" | "in_progress" | "review") {
        return Err(TaskOpError::Invalid(format!(
            "cannot close a task in state '{}'",
            current.state
        )));
    }

    apply_state_change(&mut tx, &current, "merged", actor_agent_id).await?;
    tx.commit().await?;
    get(pool, workspace_id, slug).await?.ok_or(TaskOpError::NotFound)
}

/// Shared UPDATE+event-append body for [`set_state`] and [`close`] (both
/// already validated the move before calling this).
async fn apply_state_change(
    tx: &mut Transaction<'_, SqliteDb>,
    current: &TaskRow,
    to: &str,
    actor_agent_id: Option<&str>,
) -> sqlx::Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE task SET state = ?1, updated_at = ?2 WHERE id = ?3")
        .bind(to)
        .bind(&now)
        .bind(&current.id)
        .execute(&mut **tx)
        .await?;
    append_event(
        &mut **tx,
        &current.id,
        "state",
        actor_agent_id,
        &serde_json::json!({"from": current.state, "to": to}).to_string(),
    )
    .await?;
    Ok(())
}

/// Fetch a task row inside an open transaction (read-your-own-writes within
/// the same tx, for the validate-then-update sequence every transition uses).
async fn fetch_for_update(
    tx: &mut Transaction<'_, SqliteDb>,
    workspace_id: &str,
    slug: &str,
) -> sqlx::Result<Option<TaskRow>> {
    sqlx::query_as::<_, TaskRow>(
        "SELECT id, workspace_id, slug, title, state, owner_agent_id, implementer_agent_id, \
         file_boundary, design_canon, plan, created_at, updated_at \
         FROM task WHERE workspace_id = ?1 AND slug = ?2",
    )
    .bind(workspace_id)
    .bind(slug)
    .fetch_optional(&mut **tx)
    .await
}

/// Add a `task_watch` subscription for `agent_id` on the task identified by
/// `(workspace_id, slug)`. Idempotent (`INSERT OR IGNORE` on the
/// `(task_id, agent_id)` primary key).
pub async fn watch(
    pool: &SqlitePool,
    workspace_id: &str,
    slug: &str,
    agent_id: &str,
) -> Result<(), TaskOpError> {
    let task = get(pool, workspace_id, slug).await?.ok_or(TaskOpError::NotFound)?;
    sqlx::query("INSERT OR IGNORE INTO task_watch (task_id, agent_id) VALUES (?1, ?2)")
        .bind(&task.id)
        .bind(agent_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Remove a `task_watch` subscription. Idempotent (no error if absent).
pub async fn unwatch(
    pool: &SqlitePool,
    workspace_id: &str,
    slug: &str,
    agent_id: &str,
) -> Result<(), TaskOpError> {
    let task = get(pool, workspace_id, slug).await?.ok_or(TaskOpError::NotFound)?;
    sqlx::query("DELETE FROM task_watch WHERE task_id = ?1 AND agent_id = ?2")
        .bind(&task.id)
        .bind(agent_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// All agent ids watching a task (Lane B's notify hook fan-out list).
#[allow(dead_code)]
pub async fn watchers(pool: &SqlitePool, task_id: &str) -> sqlx::Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT agent_id FROM task_watch WHERE task_id = ?1")
        .bind(task_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::db::connect_in_memory;

    async fn fixture_workspace(pool: &SqlitePool) -> String {
        crate::engine::repo::workspace::create(pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed")
            .id
    }

    async fn fixture_task(pool: &SqlitePool, ws: &str, slug: &str) -> TaskRow {
        create(
            pool,
            NewTask {
                workspace_id: ws,
                slug,
                title: "Title",
                owner_agent_id: None,
                file_boundary_json: "[]",
                design_canon: None,
                plan: "",
            },
        )
        .await
        .expect("create failed")
    }

    // ── state machine ────────────────────────────────────────────────────

    #[test]
    fn valid_transitions_allow_the_happy_path() {
        assert!(valid_transition("planned", "claimed"));
        assert!(valid_transition("claimed", "in_progress"));
        assert!(valid_transition("in_progress", "review"));
        assert!(valid_transition("review", "merged"));
    }

    #[test]
    fn valid_transitions_reject_terminal_and_backward_jumps() {
        assert!(!valid_transition("merged", "claimed"));
        assert!(!valid_transition("abandoned", "planned"));
        assert!(!valid_transition("planned", "merged"));
        assert!(!valid_transition("merged", "merged"));
        assert!(!valid_transition("planned", "planned"));
    }

    #[test]
    fn valid_transitions_allow_abandon_from_any_live_state() {
        assert!(valid_transition("planned", "abandoned"));
        assert!(valid_transition("claimed", "abandoned"));
        assert!(valid_transition("in_progress", "abandoned"));
        assert!(valid_transition("review", "abandoned"));
    }

    // ── CRUD ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn create_then_get_roundtrip() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool).await;
        let task = fixture_task(&pool, &ws, "my-slug").await;
        assert_eq!(task.state, "planned");
        assert_eq!(task.slug, "my-slug");

        let fetched = get(&pool, &ws, "my-slug")
            .await
            .expect("get failed")
            .expect("task exists");
        assert_eq!(fetched, task);
    }

    #[tokio::test]
    async fn get_missing_task_returns_none() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool).await;
        assert!(get(&pool, &ws, "nope").await.expect("get failed").is_none());
    }

    #[tokio::test]
    async fn duplicate_slug_in_same_workspace_violates_unique_constraint() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool).await;
        fixture_task(&pool, &ws, "dup").await;
        let err = create(
            &pool,
            NewTask {
                workspace_id: &ws,
                slug: "dup",
                title: "Second",
                owner_agent_id: None,
                file_boundary_json: "[]",
                design_canon: None,
                plan: "",
            },
        )
        .await;
        assert!(err.is_err(), "duplicate (workspace, slug) must fail");
    }

    #[tokio::test]
    async fn same_slug_in_different_workspaces_is_allowed() {
        let pool = connect_in_memory().await;
        let ws1 = fixture_workspace(&pool).await;
        let ws2 = fixture_workspace(&pool).await;
        fixture_task(&pool, &ws1, "shared-slug").await;
        let second = create(
            &pool,
            NewTask {
                workspace_id: &ws2,
                slug: "shared-slug",
                title: "Second",
                owner_agent_id: None,
                file_boundary_json: "[]",
                design_canon: None,
                plan: "",
            },
        )
        .await;
        assert!(second.is_ok());
    }

    #[tokio::test]
    async fn list_includes_event_count_and_filters_by_state() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool).await;
        let t1 = fixture_task(&pool, &ws, "t1").await;
        fixture_task(&pool, &ws, "t2").await;
        add_note(&pool, &ws, "t1", None, "{\"text\":\"hi\"}")
            .await
            .expect("note failed");

        let all = list(&pool, &ws, None).await.expect("list failed");
        assert_eq!(all.len(), 2);
        let t1_row = all.iter().find(|r| r.slug == "t1").expect("t1 present");
        assert_eq!(t1_row.event_count, 1);
        let t2_row = all.iter().find(|r| r.slug == "t2").expect("t2 present");
        assert_eq!(t2_row.event_count, 0);

        claim(&pool, &ws, "t1", "agent-1").await.expect("claim failed");
        let claimed_only = list(&pool, &ws, Some("claimed"))
            .await
            .expect("filtered list failed");
        assert_eq!(claimed_only.len(), 1);
        assert_eq!(claimed_only[0].id, t1.id);
    }

    // ── claim / transitions ─────────────────────────────────────────────

    #[tokio::test]
    async fn claim_sets_implementer_and_state() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool).await;
        fixture_task(&pool, &ws, "t1").await;

        let claimed = claim(&pool, &ws, "t1", "agent-1").await.expect("claim failed");
        assert_eq!(claimed.state, "claimed");
        assert_eq!(claimed.implementer_agent_id.as_deref(), Some("agent-1"));

        let events = events_for(&pool, &claimed.id, 20).await.expect("events failed");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "state");
        assert_eq!(events[0].actor_agent_id.as_deref(), Some("agent-1"));
    }

    #[tokio::test]
    async fn claim_an_already_claimed_task_is_rejected() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool).await;
        fixture_task(&pool, &ws, "t1").await;
        claim(&pool, &ws, "t1", "agent-1").await.expect("first claim failed");

        let err = claim(&pool, &ws, "t1", "agent-2").await;
        assert!(matches!(err, Err(TaskOpError::Invalid(_))));
    }

    #[tokio::test]
    async fn set_state_rejects_merged_to_claimed() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool).await;
        fixture_task(&pool, &ws, "t1").await;
        claim(&pool, &ws, "t1", "agent-1").await.expect("claim failed");
        close(&pool, &ws, "t1", Some("agent-1")).await.expect("close failed");

        let err = set_state(&pool, &ws, "t1", "claimed", Some("agent-1")).await;
        assert!(
            matches!(err, Err(TaskOpError::Invalid(_))),
            "merged -> claimed must be rejected"
        );
    }

    #[tokio::test]
    async fn set_state_happy_path_through_review() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool).await;
        fixture_task(&pool, &ws, "t1").await;
        claim(&pool, &ws, "t1", "agent-1").await.expect("claim failed");

        let in_progress = set_state(&pool, &ws, "t1", "in_progress", Some("agent-1"))
            .await
            .expect("-> in_progress failed");
        assert_eq!(in_progress.state, "in_progress");

        let review = set_state(&pool, &ws, "t1", "review", Some("agent-1"))
            .await
            .expect("-> review failed");
        assert_eq!(review.state, "review");
    }

    #[tokio::test]
    async fn set_state_missing_task_is_not_found() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool).await;
        let err = set_state(&pool, &ws, "nope", "claimed", None).await;
        assert!(matches!(err, Err(TaskOpError::NotFound)));
    }

    // ── close ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn close_from_claimed_transitions_to_merged() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool).await;
        fixture_task(&pool, &ws, "t1").await;
        claim(&pool, &ws, "t1", "agent-1").await.expect("claim failed");

        let closed = close(&pool, &ws, "t1", Some("agent-1")).await.expect("close failed");
        assert_eq!(closed.state, "merged");
    }

    #[tokio::test]
    async fn close_from_planned_is_rejected() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool).await;
        fixture_task(&pool, &ws, "t1").await;

        let err = close(&pool, &ws, "t1", None).await;
        assert!(matches!(err, Err(TaskOpError::Invalid(_))));
    }

    // ── events ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn add_note_appends_event_and_events_for_is_newest_first() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool).await;
        let task = fixture_task(&pool, &ws, "t1").await;

        add_note(&pool, &ws, "t1", Some("agent-1"), "{\"text\":\"first\"}")
            .await
            .expect("note 1 failed");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        add_note(&pool, &ws, "t1", Some("agent-1"), "{\"text\":\"second\"}")
            .await
            .expect("note 2 failed");

        let events = events_for(&pool, &task.id, 20).await.expect("events failed");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].payload, "{\"text\":\"second\"}", "newest first");
        assert!(events.iter().all(|e| e.kind == "note"));
    }

    #[tokio::test]
    async fn add_gate_records_non_zero_exit_as_evidence_not_error() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool).await;
        let task = fixture_task(&pool, &ws, "t1").await;

        let payload = serde_json::json!({
            "cmd": "cargo test", "exit": 101, "sha": "abc123", "tail": "FAILED", "cwd": "/repo"
        })
        .to_string();
        add_gate(&pool, &ws, "t1", Some("agent-1"), &payload)
            .await
            .expect("gate must record even on non-zero exit");

        let events = events_for(&pool, &task.id, 20).await.expect("events failed");
        assert_eq!(events[0].kind, "gate");
    }

    #[tokio::test]
    async fn note_on_missing_task_is_not_found() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool).await;
        let err = add_note(&pool, &ws, "nope", None, "{}").await;
        assert!(matches!(err, Err(TaskOpError::NotFound)));
    }

    // ── watch / unwatch ───────────────────────────────────────────────────

    #[tokio::test]
    async fn watch_then_unwatch_roundtrip() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool).await;
        let task = fixture_task(&pool, &ws, "t1").await;

        watch(&pool, &ws, "t1", "agent-1").await.expect("watch failed");
        assert_eq!(
            watchers(&pool, &task.id).await.expect("watchers failed"),
            vec!["agent-1".to_string()]
        );

        // Idempotent: watching twice does not duplicate.
        watch(&pool, &ws, "t1", "agent-1").await.expect("re-watch failed");
        assert_eq!(watchers(&pool, &task.id).await.expect("watchers failed").len(), 1);

        unwatch(&pool, &ws, "t1", "agent-1").await.expect("unwatch failed");
        assert!(watchers(&pool, &task.id).await.expect("watchers failed").is_empty());
    }

    #[tokio::test]
    async fn unwatch_missing_subscription_is_a_noop() {
        let pool = connect_in_memory().await;
        let ws = fixture_workspace(&pool).await;
        fixture_task(&pool, &ws, "t1").await;
        unwatch(&pool, &ws, "t1", "agent-1")
            .await
            .expect("unwatch of absent subscription must not error");
    }
}
