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
use sqlx::{SqliteConnection, SqlitePool};
use std::collections::HashSet;
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

/// Return the ordered rank for a position level.
///
/// Unknown values (and `NULL`, which callers represent by not calling this
/// helper) are unranked and sort below every known level.
#[allow(dead_code)] // consumed by the dependent Position System lanes
pub fn level_rank(level: &str) -> u8 {
    match level {
        "junior" => 1,
        "mid" => 2,
        "senior" => 3,
        "principal" => 4,
        _ => 0,
    }
}

/// Return one hop up the supervisor chain, or `None` for a root/unknown agent.
#[allow(dead_code)] // consumed by the dependent Position System lanes
pub async fn supervisor_of(pool: &SqlitePool, id: &str) -> sqlx::Result<Option<String>> {
    let supervisor: Option<Option<String>> =
        sqlx::query_scalar("SELECT supervisor_agent_id FROM workspace_agent WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    Ok(supervisor.flatten())
}

/// Return `[self, supervisor, supervisor², …]`, stopping at the workspace root.
///
/// The walk is capped at the workspace's agent count, so even a corrupt cycle
/// inserted outside the command layer cannot loop forever.
#[allow(dead_code)] // consumed by the dependent Position System lanes
pub async fn supervisor_chain(pool: &SqlitePool, id: &str) -> sqlx::Result<Vec<String>> {
    let workspace_id: Option<String> =
        sqlx::query_scalar("SELECT workspace_id FROM workspace_agent WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    let Some(workspace_id) = workspace_id else {
        return Ok(Vec::new());
    };

    let depth: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM workspace_agent WHERE workspace_id = ?")
            .bind(workspace_id)
            .fetch_one(pool)
            .await?;

    let mut chain = Vec::with_capacity(depth as usize);
    let mut current = id.to_owned();
    for _ in 0..depth {
        chain.push(current.clone());
        let Some(supervisor) = supervisor_of(pool, &current).await? else {
            break;
        };
        current = supervisor;
    }
    Ok(chain)
}

/// Return the nearest workspace agent with authority over both agents.
///
/// `None` means the two chains meet only at the implicit human root.
#[allow(dead_code)] // consumed by the dependent Position System lanes
pub async fn lowest_common_supervisor(
    pool: &SqlitePool,
    a: &str,
    b: &str,
) -> sqlx::Result<Option<String>> {
    let chain_a: HashSet<String> = supervisor_chain(pool, a).await?.into_iter().collect();
    Ok(supervisor_chain(pool, b)
        .await?
        .into_iter()
        .find(|id| chain_a.contains(id)))
}

/// Return whether setting `agent_id`'s supervisor to `supervisor_agent_id`
/// would close a cycle.
///
/// Scope/existence validation remains the command layer's responsibility.
#[allow(dead_code)] // consumed by the dependent Position System lanes
pub async fn would_create_cycle(
    pool: &SqlitePool,
    agent_id: &str,
    supervisor_agent_id: &str,
) -> sqlx::Result<bool> {
    if agent_id == supervisor_agent_id {
        return Ok(true);
    }
    Ok(supervisor_chain(pool, supervisor_agent_id)
        .await?
        .iter()
        .any(|id| id == agent_id))
}

/// Persist an instance's level and supervisor. Validation is performed by the
/// command layer; this repository primitive is intentionally a plain update.
#[allow(dead_code)] // consumed by the dependent Position System lanes
pub async fn set_position(
    pool: &SqlitePool,
    id: &str,
    level: Option<&str>,
    supervisor_agent_id: Option<&str>,
) -> sqlx::Result<()> {
    QueryBuilder::<Sqlite>::table("workspace_agent")
        .update([
            (
                "level",
                level
                    .map(|value| Bind::Text(value.to_owned()))
                    .unwrap_or(Bind::Null),
            ),
            (
                "supervisor_agent_id",
                supervisor_agent_id
                    .map(|value| Bind::Text(value.to_owned()))
                    .unwrap_or(Bind::Null),
            ),
        ])
        .where_eq("id", id)
        .execute(pool)
        .await
        .map_err(cb_err)?;
    Ok(())
}

/// Tri-state intent for one position field on [`set_position_validated`]:
/// `Keep` leaves the stored value, `Clear` sets it NULL, `Set` writes a value.
#[derive(Debug, Clone, PartialEq)]
pub enum PositionField {
    Keep,
    Clear,
    Set(String),
}

/// A validated-position write failed for a business reason. The command layer
/// maps these to the wire-facing `AppError`; `Db` carries a raw sqlx error.
#[derive(Debug)]
pub enum SetPositionError {
    AgentNotFound,
    WorkspaceMismatch,
    LevelInvalid,
    SupervisorSelf,
    SupervisorNotFound,
    SupervisorCrossWorkspace,
    Cycle,
    Db(sqlx::Error),
}

impl From<sqlx::Error> for SetPositionError {
    fn from(e: sqlx::Error) -> Self {
        SetPositionError::Db(e)
    }
}

/// Atomic read-validate-write of an instance's position (spec position-system
/// §3.5; escalation ruling ef969027 / 5c9df684). The whole operation runs in a
/// single `BEGIN IMMEDIATE` transaction on ONE connection, so concurrent
/// `setPosition` calls SERIALIZE on the write lock — closing the TOCTOU where
/// two racing writes each validate against the pre-write forest (a cycle could
/// land, or a partial update clobber the other's field). This is the same
/// one-transaction shape as the F1 memory-approve fix (@ 6d87530).
///
/// Validation, all inside the tx: the agent exists and is in `workspace_id`
/// (646ec73a scope check, ruling 71a00512); a `Set` level is in the enum; a
/// `Set` supervisor is not self, exists, is in the same workspace, and does not
/// close a cycle (walked against the in-tx forest). `Keep` resolves against the
/// row read inside the tx, so a concurrent partial update cannot be lost.
pub async fn set_position_validated(
    pool: &SqlitePool,
    workspace_id: &str,
    agent_id: &str,
    level: PositionField,
    supervisor: PositionField,
) -> Result<(), SetPositionError> {
    let mut conn = pool.acquire().await.map_err(SetPositionError::Db)?;
    // BEGIN IMMEDIATE takes the write lock up front, so a second setPosition
    // blocks here until this one commits (avoids the DEFERRED upgrade deadlock).
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *conn)
        .await
        .map_err(SetPositionError::Db)?;

    match position_txn(&mut conn, workspace_id, agent_id, level, supervisor).await {
        Ok(()) => {
            sqlx::query("COMMIT")
                .execute(&mut *conn)
                .await
                .map_err(SetPositionError::Db)?;
            Ok(())
        }
        Err(e) => {
            // Best-effort rollback; return the original business error.
            let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
            Err(e)
        }
    }
}

/// The validated read-then-write body, executed on the already-opened
/// transaction connection. Kept separate so [`set_position_validated`] can
/// COMMIT/ROLLBACK around it uniformly.
async fn position_txn(
    conn: &mut SqliteConnection,
    workspace_id: &str,
    agent_id: &str,
    level: PositionField,
    supervisor: PositionField,
) -> Result<(), SetPositionError> {
    // Read the current row inside the tx (existence + workspace + current values).
    let current: Option<(Option<String>, Option<String>, String)> = sqlx::query_as(
        "SELECT level, supervisor_agent_id, workspace_id FROM workspace_agent WHERE id = ?",
    )
    .bind(agent_id)
    .fetch_optional(&mut *conn)
    .await?;
    let (cur_level, cur_supervisor, agent_ws) = current.ok_or(SetPositionError::AgentNotFound)?;
    if agent_ws != workspace_id {
        return Err(SetPositionError::WorkspaceMismatch);
    }

    // Resolve tri-state against the in-tx current values (Keep can't be lost).
    let new_level = match level {
        PositionField::Keep => cur_level,
        PositionField::Clear => None,
        PositionField::Set(v) => Some(v),
    };
    let new_supervisor = match supervisor {
        PositionField::Keep => cur_supervisor,
        PositionField::Clear => None,
        PositionField::Set(v) => Some(v),
    };

    if let Some(l) = new_level.as_deref() {
        if level_rank(l) == 0 {
            return Err(SetPositionError::LevelInvalid);
        }
    }

    if let Some(s) = new_supervisor.as_deref() {
        if s == agent_id {
            return Err(SetPositionError::SupervisorSelf);
        }
        let sup_ws: Option<String> =
            sqlx::query_scalar("SELECT workspace_id FROM workspace_agent WHERE id = ?")
                .bind(s)
                .fetch_optional(&mut *conn)
                .await?;
        let sup_ws = sup_ws.ok_or(SetPositionError::SupervisorNotFound)?;
        if sup_ws != workspace_id {
            return Err(SetPositionError::SupervisorCrossWorkspace);
        }
        if chain_hits_in_tx(conn, workspace_id, s, agent_id).await? {
            return Err(SetPositionError::Cycle);
        }
    }

    sqlx::query("UPDATE workspace_agent SET level = ?, supervisor_agent_id = ? WHERE id = ?")
        .bind(&new_level)
        .bind(&new_supervisor)
        .bind(agent_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// In-transaction supervisor-chain walk: starting at `start`, follow
/// `supervisor_agent_id` upward and return whether `target` is reached. The
/// tx-scoped analogue of [`would_create_cycle`]/[`supervisor_chain`] (depth-
/// bounded by the workspace's agent count, so even a pre-existing corrupt cycle
/// terminates).
async fn chain_hits_in_tx(
    conn: &mut SqliteConnection,
    workspace_id: &str,
    start: &str,
    target: &str,
) -> Result<bool, sqlx::Error> {
    let depth: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM workspace_agent WHERE workspace_id = ?")
            .bind(workspace_id)
            .fetch_one(&mut *conn)
            .await?;
    let mut current = start.to_owned();
    for _ in 0..depth {
        if current == target {
            return Ok(true);
        }
        let next: Option<Option<String>> =
            sqlx::query_scalar("SELECT supervisor_agent_id FROM workspace_agent WHERE id = ?")
                .bind(&current)
                .fetch_optional(&mut *conn)
                .await?;
        match next.flatten() {
            Some(s) => current = s,
            None => break,
        }
    }
    Ok(false)
}

/// A self-describing roster entry (ADR 0005): a `workspace_agent` annotated
/// with its paired session's `launched_skill_ids` (raw JSON text, `None`
/// before any launch — so the Roster can detect skill drift without a second
/// IPC round-trip) AND the resolved human-facing description of who the agent
/// IS: its display name, its role name + one-paragraph description, and the
/// NAMES (not ids) of the skills it launched with. Consumed by
/// `commands::instance::list` and, through it, `conclave agent list` — so
/// running agents can see who their peers are.
///
/// The four description fields (`name`, `role_name`, `role_description`,
/// `skill_names`) are ADDITIVE — `id`/`status`/`launchedSkillIds` keep their
/// exact names and meaning (the enriched JSON is consumed by live agents).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceAgentWithSkills {
    pub id: String,
    pub workspace_id: String,
    pub agent_def_id: String,
    pub status: String,
    pub added_at: String,
    /// The agent definition's display name.
    pub name: String,
    /// Ordered seniority within the agent's track.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    /// The supervising workspace-agent instance; `None` reports to human/top.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supervisor_agent_id: Option<String>,
    /// Resolved display name of `supervisor_agent_id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supervisor_name: Option<String>,
    /// The role's display name — from the resolved role (builtin or custom),
    /// falling back to the legacy free-text `agent_definition.role` label when
    /// no `role_id` is set (or it dangles). `None` for a role-less agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_name: Option<String>,
    /// The role's one-paragraph job description. Only present when a
    /// first-class role resolved (the legacy free-text fallback has none).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_description: Option<String>,
    /// The NAMES of the skills this agent launched with, in `launched_skill_ids`
    /// order. Ids that no longer resolve to a shipped/DB skill (e.g. a deleted
    /// custom skill) are dropped, mirroring the ignore-unknown pattern.
    pub skill_names: Vec<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_launched_ids"
    )]
    pub launched_skill_ids: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_limit: Option<i64>,
    /// The agent definition's configured model id (e.g. `"claude-sonnet-5"`),
    /// `None` when unset — lets a lead factor model into delegation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The agent definition's CLI harness kind (`claude-code` / `codex` /
    /// `custom`), `None` for a chat/orchestrator agent or an unset `cli`
    /// agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli_kind: Option<String>,
    /// Whether the live backend emitted output within `WORKING_WINDOW`
    /// (commands::instance) — `Some` only for currently-live instances.
    /// Always `None` here (this repo layer stays DB-pure); the HANDLER
    /// (`commands::instance::list`) populates it from `Runtime`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working: Option<bool>,
    /// ISO-8601 UTC of the last recorded activity — handler-populated, see
    /// `working`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<String>,
    /// The live session id (for mapping `session:output` events back to this
    /// row) — handler-populated, see `working`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// Same shape as `session::serialize_json_text` — kept local (small, trivial,
/// different module) rather than shared.
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

/// Raw roster row straight from SQL, before name-resolution. Private — callers
/// get the enriched [`WorkspaceAgentWithSkills`].
#[derive(sqlx::FromRow)]
struct RosterQueryRow {
    id: String,
    workspace_id: String,
    agent_def_id: String,
    status: String,
    added_at: String,
    launched_skill_ids: Option<String>,
    context_tokens: Option<i64>,
    context_limit: Option<i64>,
    agent_name: String,
    level: Option<String>,
    supervisor_agent_id: Option<String>,
    supervisor_name: Option<String>,
    role_id: Option<String>,
    role_text: Option<String>,
    model: Option<String>,
    cli_kind: Option<String>,
}

/// Return the self-describing roster for a workspace (same ordering as
/// `list_by_workspace`): every `workspace_agent` LEFT JOINed with its session's
/// `launched_skill_ids` and its `agent_definition`'s name + role, then enriched
/// in Rust to resolve role name/description and skill NAMES.
///
/// Enrichment reads the builtin role/skill folders (sync, cheap) and the custom
/// `role`/`skill` tables ONCE each, then resolves every row against those maps
/// — no per-row query. Skill names come from the bundled reader (builtin) or
/// the `skill` table (custom); a launched id that resolves to neither (a since-
/// deleted custom skill) is dropped rather than surfaced as a bare id.
pub async fn list_by_workspace_with_launched_skills(
    pool: &SqlitePool,
    workspace_id: &str,
) -> sqlx::Result<Vec<WorkspaceAgentWithSkills>> {
    // The `JOIN agent_definition` below is an INNER join, safe ONLY because
    // `workspace_agent.agent_def_id` is a NOT NULL foreign key whose parent
    // delete is restricted (NO ACTION) — a definition can't be removed while an
    // instance references it (agent_definition::delete removes instances first).
    // If that FK is ever dropped or made ON DELETE SET NULL, a widowed instance
    // would silently VANISH from the roster here; make it a LEFT JOIN then.
    let rows: Vec<RosterQueryRow> = sqlx::query_as(
        "SELECT wa.id, wa.workspace_id, wa.agent_def_id, wa.status, wa.added_at, \
         wa.level, wa.supervisor_agent_id, supervisor_def.name AS supervisor_name, \
         sess.launched_skill_ids, sess.context_tokens, sess.context_limit, \
         ad.name AS agent_name, ad.role_id AS role_id, ad.role AS role_text, \
         ad.model AS model, ad.cli_kind AS cli_kind \
         FROM workspace_agent wa \
         LEFT JOIN session sess ON sess.workspace_agent_id = wa.id \
         LEFT JOIN workspace_agent supervisor ON supervisor.id = wa.supervisor_agent_id \
         LEFT JOIN agent_definition supervisor_def ON supervisor_def.id = supervisor.agent_def_id \
         JOIN agent_definition ad ON ad.id = wa.agent_def_id \
         WHERE wa.workspace_id = ? \
         ORDER BY wa.added_at ASC, wa.id ASC",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await?;

    // Build id→name / id→role lookup maps ONCE (builtin folders + DB tables).
    let skill_names: std::collections::HashMap<String, String> = super::skill::list_builtin()
        .into_iter()
        .chain(super::skill::list(pool).await?)
        .map(|s| (s.id, s.name))
        .collect();
    let roles: std::collections::HashMap<String, super::role::RoleRow> =
        super::role::list_builtin()
            .into_iter()
            .chain(super::role::list(pool).await?)
            .map(|r| (r.id.clone(), r))
            .collect();

    Ok(rows
        .into_iter()
        .map(|r| {
            let (role_name, role_description) = match r.role_id.as_deref() {
                Some(rid) => match roles.get(rid) {
                    Some(role) => (Some(role.name.clone()), Some(role.description.clone())),
                    // Dangling role_id (shouldn't happen — delete NULLs it) —
                    // fall back to the legacy free-text label.
                    None => (r.role_text.clone(), None),
                },
                // No first-class role — the legacy free-text label, if any.
                None => (r.role_text.clone(), None),
            };

            let skill_names_resolved = r
                .launched_skill_ids
                .as_deref()
                .and_then(|text| serde_json::from_str::<Vec<String>>(text).ok())
                .unwrap_or_default()
                .into_iter()
                .filter_map(|id| skill_names.get(&id).cloned())
                .collect();

            WorkspaceAgentWithSkills {
                id: r.id,
                workspace_id: r.workspace_id,
                agent_def_id: r.agent_def_id,
                status: r.status,
                added_at: r.added_at,
                name: r.agent_name,
                level: r.level,
                supervisor_agent_id: r.supervisor_agent_id,
                supervisor_name: r.supervisor_name,
                role_name,
                role_description,
                skill_names: skill_names_resolved,
                launched_skill_ids: r.launched_skill_ids,
                context_tokens: r.context_tokens,
                context_limit: r.context_limit,
                model: r.model,
                cli_kind: r.cli_kind,
                working: None,
                last_activity_at: None,
                session_id: None,
            }
        })
        .collect())
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
         (id, workspace_id, agent_def_id, status, added_at, level) \
         SELECT ?, ?, id, 'idle', ?, default_level \
         FROM agent_definition WHERE id = ?",
    )
    .bind(&wa_id)
    .bind(workspace_id)
    .bind(&added_at)
    .bind(agent_def_id)
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

    sqlx::query(
        "UPDATE workspace_agent SET supervisor_agent_id = NULL WHERE supervisor_agent_id = ?",
    )
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

    async fn instance_named(
        pool: &SqlitePool,
        workspace_id: &str,
        name: &str,
    ) -> WorkspaceAgentRow {
        let def = agent_definition::create(
            pool,
            &AgentDefinitionInput {
                name: name.into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create named agent");
        instantiate(pool, workspace_id, &def.id)
            .await
            .expect("instantiate named agent")
    }

    #[test]
    fn level_rank_orders_known_levels_and_defaults_unknown_to_zero() {
        assert_eq!(level_rank("junior"), 1);
        assert_eq!(level_rank("mid"), 2);
        assert_eq!(level_rank("senior"), 3);
        assert_eq!(level_rank("principal"), 4);
        assert_eq!(level_rank(""), 0);
        assert_eq!(level_rank("lead"), 0);
    }

    #[tokio::test]
    async fn supervisor_chain_walks_to_root_and_is_depth_bounded_on_cycle() {
        let pool = connect_in_memory().await;
        let ws = workspace::create(&pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace");
        let a = instance_named(&pool, &ws.id, "A").await;
        let b = instance_named(&pool, &ws.id, "B").await;
        let c = instance_named(&pool, &ws.id, "C").await;

        set_position(&pool, &a.id, None, Some(&b.id))
            .await
            .expect("set a supervisor");
        set_position(&pool, &b.id, None, Some(&c.id))
            .await
            .expect("set b supervisor");

        assert_eq!(
            supervisor_of(&pool, &a.id).await.unwrap(),
            Some(b.id.clone())
        );
        assert_eq!(
            supervisor_chain(&pool, &a.id).await.unwrap(),
            vec![a.id.clone(), b.id.clone(), c.id.clone()]
        );
        assert!(
            would_create_cycle(&pool, &c.id, &a.id).await.unwrap(),
            "c -> a would close the existing a -> b -> c chain"
        );
        assert!(would_create_cycle(&pool, &a.id, &a.id).await.unwrap());

        // Bypass the command guard to model a corrupt/direct-DB cycle. The
        // chain returns at most the workspace's three members.
        set_position(&pool, &c.id, None, Some(&a.id))
            .await
            .expect("seed cycle");
        assert_eq!(
            supervisor_chain(&pool, &a.id).await.unwrap(),
            vec![a.id, b.id, c.id]
        );
    }

    #[tokio::test]
    async fn lowest_common_supervisor_handles_same_cross_and_disjoint_chains() {
        let pool = connect_in_memory().await;
        let ws = workspace::create(&pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace");
        let root = instance_named(&pool, &ws.id, "Root").await;
        let left = instance_named(&pool, &ws.id, "Left").await;
        let leaf = instance_named(&pool, &ws.id, "Leaf").await;
        let right = instance_named(&pool, &ws.id, "Right").await;
        let other_root = instance_named(&pool, &ws.id, "OtherRoot").await;

        set_position(&pool, &left.id, None, Some(&root.id))
            .await
            .unwrap();
        set_position(&pool, &leaf.id, None, Some(&left.id))
            .await
            .unwrap();
        set_position(&pool, &right.id, None, Some(&root.id))
            .await
            .unwrap();

        assert_eq!(
            lowest_common_supervisor(&pool, &leaf.id, &left.id)
                .await
                .unwrap(),
            Some(left.id)
        );
        assert_eq!(
            lowest_common_supervisor(&pool, &leaf.id, &right.id)
                .await
                .unwrap(),
            Some(root.id)
        );
        assert_eq!(
            lowest_common_supervisor(&pool, &leaf.id, &other_root.id)
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn set_position_round_trips_through_roster() {
        let pool = connect_in_memory().await;
        let ws = workspace::create(&pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace");
        let supervisor = instance_named(&pool, &ws.id, "Supervisor").await;
        let report = instance_named(&pool, &ws.id, "Report").await;

        set_position(&pool, &report.id, Some("senior"), Some(&supervisor.id))
            .await
            .expect("set position");
        let roster = list_by_workspace_with_launched_skills(&pool, &ws.id)
            .await
            .expect("list roster");
        let row = roster.iter().find(|row| row.id == report.id).unwrap();
        assert_eq!(row.level.as_deref(), Some("senior"));
        assert_eq!(
            row.supervisor_agent_id.as_deref(),
            Some(supervisor.id.as_str())
        );
        assert_eq!(row.supervisor_name.as_deref(), Some("Supervisor"));

        set_position(&pool, &report.id, None, None)
            .await
            .expect("clear position");
        let roster = list_by_workspace_with_launched_skills(&pool, &ws.id)
            .await
            .expect("list cleared roster");
        let row = roster.iter().find(|row| row.id == report.id).unwrap();
        assert!(row.level.is_none());
        assert!(row.supervisor_agent_id.is_none());
        assert!(row.supervisor_name.is_none());
        let json = serde_json::to_value(row).expect("serialize roster");
        assert!(json.get("level").is_none());
        assert!(json.get("supervisorAgentId").is_none());
        assert!(json.get("supervisorName").is_none());
    }

    #[tokio::test]
    async fn deleting_supervisor_row_nulls_reports_via_fk_action() {
        let pool = connect_in_memory().await;
        let ws = workspace::create(&pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace");
        let supervisor = instance_named(&pool, &ws.id, "Supervisor").await;
        let report = instance_named(&pool, &ws.id, "Report").await;

        set_position(&pool, &report.id, Some("mid"), Some(&supervisor.id))
            .await
            .expect("set position");

        sqlx::query("DELETE FROM workspace_agent WHERE id = ?")
            .bind(&supervisor.id)
            .execute(&pool)
            .await
            .expect("delete supervisor");

        let roster = list_by_workspace_with_launched_skills(&pool, &ws.id)
            .await
            .expect("list roster after supervisor delete");
        let row = roster.iter().find(|row| row.id == report.id).unwrap();
        assert_eq!(row.level.as_deref(), Some("mid"));
        assert!(
            row.supervisor_agent_id.is_none(),
            "FK ON DELETE SET NULL should clear supervisor_agent_id"
        );
        assert!(
            row.supervisor_name.is_none(),
            "resolved supervisor name should disappear with the FK"
        );

        let stored_supervisor: Option<String> =
            sqlx::query_scalar("SELECT supervisor_agent_id FROM workspace_agent WHERE id = ?")
                .bind(&report.id)
                .fetch_one(&pool)
                .await
                .expect("fetch surviving report");
        assert!(stored_supervisor.is_none(), "stored FK should be NULL");
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
        set_position(&pool, &b.id, None, Some(&a.id))
            .await
            .expect("seed supervisor link");
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
        let b_supervisor: Option<String> =
            sqlx::query_scalar("SELECT supervisor_agent_id FROM workspace_agent WHERE id = ?")
                .bind(&b.id)
                .fetch_one(&pool)
                .await
                .expect("fetch surviving report");
        assert!(
            b_supervisor.is_none(),
            "workspace_agent.supervisor_agent_id should be NULL"
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

    #[tokio::test]
    async fn instantiate_seeds_level_from_agent_definition_default() {
        let pool = connect_in_memory().await;
        let ws = workspace::create(&pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace");
        let def = agent_definition::create(
            &pool,
            &AgentDefinitionInput {
                name: "Senior Seed".into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                default_level: Some("senior".into()),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def");

        let row = instantiate(&pool, &ws.id, &def.id)
            .await
            .expect("instantiate failed");

        assert_eq!(level_of(&pool, &row.id).await.as_deref(), Some("senior"));
    }

    #[tokio::test]
    async fn instantiate_reuse_keeps_manual_instance_level() {
        let pool = connect_in_memory().await;
        let ws = workspace::create(&pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace");
        let def = agent_definition::create(
            &pool,
            &AgentDefinitionInput {
                name: "Seeded".into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                default_level: Some("senior".into()),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def");

        let first = instantiate(&pool, &ws.id, &def.id)
            .await
            .expect("first instantiate");
        set_position(&pool, &first.id, Some("mid"), None)
            .await
            .expect("manual level change");

        let second = instantiate(&pool, &ws.id, &def.id)
            .await
            .expect("second instantiate");

        assert_eq!(first.id, second.id, "reuse path must return the same row");
        assert_eq!(level_of(&pool, &second.id).await.as_deref(), Some("mid"));
    }

    #[tokio::test]
    async fn instantiate_after_remove_reseeds_from_definition_default() {
        let pool = connect_in_memory().await;
        let ws = workspace::create(&pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace");
        let def = agent_definition::create(
            &pool,
            &AgentDefinitionInput {
                name: "ReSeed".into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                default_level: Some("senior".into()),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def");

        let first = instantiate(&pool, &ws.id, &def.id)
            .await
            .expect("first instantiate");
        set_position(&pool, &first.id, Some("mid"), None)
            .await
            .expect("manual level change");
        remove(&pool, &first.id).await.expect("remove failed");

        let second = instantiate(&pool, &ws.id, &def.id)
            .await
            .expect("second instantiate");

        assert_ne!(
            first.id, second.id,
            "remove + instantiate must create a new row"
        );
        assert_eq!(level_of(&pool, &second.id).await.as_deref(), Some("senior"));
    }

    #[tokio::test]
    async fn updating_definition_default_level_does_not_touch_existing_instance() {
        let pool = connect_in_memory().await;
        let ws = workspace::create(&pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace");
        let def = agent_definition::create(
            &pool,
            &AgentDefinitionInput {
                name: "Pinned".into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                default_level: Some("senior".into()),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def");

        let row = instantiate(&pool, &ws.id, &def.id)
            .await
            .expect("instantiate failed");
        assert_eq!(level_of(&pool, &row.id).await.as_deref(), Some("senior"));

        sqlx::query("UPDATE agent_definition SET default_level = 'principal' WHERE id = ?")
            .bind(&def.id)
            .execute(&pool)
            .await
            .expect("update default_level");

        assert_eq!(level_of(&pool, &row.id).await.as_deref(), Some("senior"));
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

        // Before any launch snapshot: NULL. The agent display name resolves;
        // a role-less agent carries no role fields.
        let before = list_by_workspace_with_launched_skills(&pool, &ws.id)
            .await
            .expect("query failed");
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].context_tokens, Some(0));
        assert_eq!(
            before[0].context_limit,
            Some(crate::engine::repo::session::DEFAULT_CONTEXT_LIMIT)
        );
        assert!(before[0].launched_skill_ids.is_none());
        assert_eq!(before[0].name, "A");
        assert!(before[0].role_name.is_none());
        assert!(before[0].role_description.is_none());
        assert!(before[0].skill_names.is_empty());

        let serialized = serde_json::to_value(&before[0]).expect("serialize failed");
        assert!(
            serialized.get("contextTokens").is_some(),
            "must have contextTokens"
        );
        assert!(
            serialized.get("contextLimit").is_some(),
            "must have contextLimit"
        );

        crate::engine::repo::session::set_launched_skill_ids(
            &pool,
            &session.id,
            &["sk-1".to_string()],
        )
        .await
        .expect("set failed");
        crate::engine::repo::session::set_context_reading(&pool, &session.id, 321, 8_000)
            .await
            .expect("set context reading failed");

        let after = list_by_workspace_with_launched_skills(&pool, &ws.id)
            .await
            .expect("query failed");
        assert_eq!(after[0].launched_skill_ids.as_deref(), Some(r#"["sk-1"]"#));
        assert_eq!(after[0].context_tokens, Some(321));
        assert_eq!(after[0].context_limit, Some(8_000));
        // "sk-1" resolves to no shipped/DB skill → dropped from skill_names.
        assert!(
            after[0].skill_names.is_empty(),
            "an unresolvable launched id must be dropped, not surfaced as a bare id"
        );
    }

    /// context meter values are omitted for roster rows without a session.
    #[tokio::test]
    async fn list_by_workspace_with_launched_skills_omits_context_without_session() {
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

        let row = create(&pool, &ws.id, &def.id, "idle")
            .await
            .expect("create workspace_agent");
        let roster = list_by_workspace_with_launched_skills(&pool, &ws.id)
            .await
            .expect("query failed");

        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].id, row.id);
        assert!(roster[0].context_tokens.is_none());
        assert!(roster[0].context_limit.is_none());

        let serialized = serde_json::to_value(&roster[0]).expect("serialize failed");
        assert!(
            serialized.get("contextTokens").is_none(),
            "must omit contextTokens"
        );
        assert!(
            serialized.get("contextLimit").is_none(),
            "must omit contextLimit"
        );
    }

    /// A3: the enriched roster resolves all four new fields — agent name, role
    /// name, role description, and launched skill NAMES — for an agent that has
    /// a first-class role and two attached custom skills.
    #[tokio::test]
    async fn roster_resolves_name_role_and_skill_names() {
        let pool = connect_in_memory().await;
        let ws = crate::engine::repo::workspace::create(&pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");

        // A custom role and two custom skills.
        let role = crate::engine::repo::role::create(
            &pool,
            "My Lead",
            "Leads this particular effort.",
            &[],
        )
        .await
        .expect("create role failed");
        let s1 = crate::engine::repo::skill::create(&pool, "First Skill", None, "c1")
            .await
            .expect("create skill failed");
        let s2 = crate::engine::repo::skill::create(&pool, "Second Skill", None, "c2")
            .await
            .expect("create skill failed");

        let def = crate::engine::repo::agent_definition::create(
            &pool,
            &crate::engine::repo::agent_definition::AgentDefinitionInput {
                name: "Alice".into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed");
        // Persist role_id directly (Phase B wires agent.save; A3 only reads it).
        sqlx::query("UPDATE agent_definition SET role_id = ? WHERE id = ?")
            .bind(&role.id)
            .bind(&def.id)
            .execute(&pool)
            .await
            .expect("set role_id failed");

        let inst = instantiate(&pool, &ws.id, &def.id)
            .await
            .expect("instantiate failed");
        let session = crate::engine::repo::session::get_by_instance(&pool, &inst.id)
            .await
            .expect("get session failed")
            .expect("session exists");
        crate::engine::repo::session::set_launched_skill_ids(
            &pool,
            &session.id,
            &[s1.id.clone(), s2.id.clone()],
        )
        .await
        .expect("set launched failed");

        let roster = list_by_workspace_with_launched_skills(&pool, &ws.id)
            .await
            .expect("query failed");
        assert_eq!(roster.len(), 1);
        let entry = &roster[0];
        assert_eq!(entry.name, "Alice");
        assert_eq!(entry.role_name.as_deref(), Some("My Lead"));
        assert_eq!(
            entry.role_description.as_deref(),
            Some("Leads this particular effort.")
        );
        assert_eq!(
            entry.skill_names,
            vec!["First Skill".to_string(), "Second Skill".to_string()],
            "skill NAMES must resolve in launched-id order"
        );

        // camelCase contract for the new fields.
        let json = serde_json::to_value(entry).expect("serialize failed");
        assert!(json.get("roleName").is_some());
        assert!(json.get("roleDescription").is_some());
        assert!(json.get("skillNames").is_some());
        assert!(json.get("role_name").is_none());
    }

    /// The roster surfaces the agent definition's `model` + `cli_kind`
    /// (camelCase `cliKind`) so a lead can factor model/harness into
    /// delegation without reading the database directly.
    #[tokio::test]
    async fn roster_exposes_model_and_cli_kind() {
        let pool = connect_in_memory().await;
        let ws = crate::engine::repo::workspace::create(&pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        let def = crate::engine::repo::agent_definition::create(
            &pool,
            &crate::engine::repo::agent_definition::AgentDefinitionInput {
                name: "Sonnet Sam".into(),
                agent_type: "cli".into(),
                cli_kind: Some("claude-code".into()),
                model: Some("claude-sonnet-5".into()),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed");
        instantiate(&pool, &ws.id, &def.id)
            .await
            .expect("instantiate failed");

        let roster = list_by_workspace_with_launched_skills(&pool, &ws.id)
            .await
            .expect("query failed");
        assert_eq!(roster[0].model.as_deref(), Some("claude-sonnet-5"));
        assert_eq!(roster[0].cli_kind.as_deref(), Some("claude-code"));

        let json = serde_json::to_value(&roster[0]).expect("serialize failed");
        assert_eq!(
            json.get("model").and_then(|v| v.as_str()),
            Some("claude-sonnet-5")
        );
        assert_eq!(
            json.get("cliKind").and_then(|v| v.as_str()),
            Some("claude-code")
        );
        assert!(json.get("cli_kind").is_none(), "must NOT have cli_kind");
    }

    /// An agent definition with no `model`/`cli_kind` set (e.g. a chat agent)
    /// omits both keys entirely from the JSON — null-safe, not `null`.
    #[tokio::test]
    async fn roster_model_and_cli_kind_are_null_safe_when_unset() {
        let pool = connect_in_memory().await;
        let ws = crate::engine::repo::workspace::create(&pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        let def = crate::engine::repo::agent_definition::create(
            &pool,
            &crate::engine::repo::agent_definition::AgentDefinitionInput {
                name: "Chatty".into(),
                agent_type: "chat".into(),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed");
        instantiate(&pool, &ws.id, &def.id)
            .await
            .expect("instantiate failed");

        let roster = list_by_workspace_with_launched_skills(&pool, &ws.id)
            .await
            .expect("query failed");
        assert!(roster[0].model.is_none());
        assert!(roster[0].cli_kind.is_none());

        let json = serde_json::to_value(&roster[0]).expect("serialize failed");
        assert!(json.get("model").is_none(), "absent key, not null");
        assert!(json.get("cliKind").is_none(), "absent key, not null");
    }

    /// A builtin role id set on an agent_definition resolves to the bundled
    /// role's name + description via the enriched roster.
    #[tokio::test]
    async fn roster_resolves_builtin_role() {
        let _fx = crate::engine::repo::role::test_support::fixture_roles_dir("roster-builtin-role");
        let pool = connect_in_memory().await;
        let ws = crate::engine::repo::workspace::create(&pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        let def = crate::engine::repo::agent_definition::create(
            &pool,
            &crate::engine::repo::agent_definition::AgentDefinitionInput {
                name: "Bob".into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed");
        sqlx::query("UPDATE agent_definition SET role_id = 'fix-lead' WHERE id = ?")
            .bind(&def.id)
            .execute(&pool)
            .await
            .expect("set role_id failed");
        instantiate(&pool, &ws.id, &def.id)
            .await
            .expect("instantiate failed");

        let roster = list_by_workspace_with_launched_skills(&pool, &ws.id)
            .await
            .expect("query failed");
        assert_eq!(roster[0].role_name.as_deref(), Some("Fixture Lead"));
        assert_eq!(
            roster[0].role_description.as_deref(),
            Some("Leads the fixture team.")
        );
    }

    /// The legacy free-text `agent_definition.role` label is used when no
    /// first-class `role_id` is set — with no role description.
    #[tokio::test]
    async fn roster_falls_back_to_legacy_role_text() {
        let pool = connect_in_memory().await;
        let ws = crate::engine::repo::workspace::create(&pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        let def = crate::engine::repo::agent_definition::create(
            &pool,
            &crate::engine::repo::agent_definition::AgentDefinitionInput {
                name: "Carol".into(),
                role: Some("Legacy Role Label".into()),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed");
        instantiate(&pool, &ws.id, &def.id)
            .await
            .expect("instantiate failed");

        let roster = list_by_workspace_with_launched_skills(&pool, &ws.id)
            .await
            .expect("query failed");
        assert_eq!(roster[0].role_name.as_deref(), Some("Legacy Role Label"));
        assert!(
            roster[0].role_description.is_none(),
            "the legacy free-text label carries no description"
        );
    }

    // ── set_position_validated: atomic validated write (ruling ef969027) ──

    async fn level_of(pool: &SqlitePool, id: &str) -> Option<String> {
        sqlx::query_scalar("SELECT level FROM workspace_agent WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .expect("read level")
    }

    #[tokio::test]
    async fn set_position_validated_happy_path_and_tri_state() {
        let pool = connect_in_memory().await;
        let ws = workspace::create(&pool, "WS", "/tmp/ws", None)
            .await
            .expect("ws");
        let a = instance_named(&pool, &ws.id, "A").await;
        let sup = instance_named(&pool, &ws.id, "Sup").await;

        // Set both.
        set_position_validated(
            &pool,
            &ws.id,
            &a.id,
            PositionField::Set("senior".into()),
            PositionField::Set(sup.id.clone()),
        )
        .await
        .expect("set both");
        assert_eq!(level_of(&pool, &a.id).await.as_deref(), Some("senior"));
        assert_eq!(
            supervisor_of(&pool, &a.id).await.unwrap(),
            Some(sup.id.clone())
        );

        // Keep level, clear supervisor.
        set_position_validated(
            &pool,
            &ws.id,
            &a.id,
            PositionField::Keep,
            PositionField::Clear,
        )
        .await
        .expect("keep level, clear supervisor");
        assert_eq!(
            level_of(&pool, &a.id).await.as_deref(),
            Some("senior"),
            "level kept"
        );
        assert_eq!(
            supervisor_of(&pool, &a.id).await.unwrap(),
            None,
            "supervisor cleared"
        );
    }

    #[tokio::test]
    async fn set_position_validated_rejects_every_invalid_case() {
        let pool = connect_in_memory().await;
        let ws = workspace::create(&pool, "WS", "/tmp/ws", None)
            .await
            .expect("ws");
        let ws2 = workspace::create(&pool, "WS2", "/tmp/ws2", None)
            .await
            .expect("ws2");
        let a = instance_named(&pool, &ws.id, "A").await;
        let b = instance_named(&pool, &ws.id, "B").await;
        let foreign = instance_named(&pool, &ws2.id, "Foreign").await;

        let set = |agent: &str, level: PositionField, sup: PositionField, wsid: &str| {
            let (pool, agent, wsid) = (pool.clone(), agent.to_owned(), wsid.to_owned());
            async move { set_position_validated(&pool, &wsid, &agent, level, sup).await }
        };
        macro_rules! err {
            ($e:expr, $pat:pat) => {
                assert!(
                    matches!($e.await, Err($pat)),
                    "expected {}",
                    stringify!($pat)
                )
            };
        }
        err!(
            set("ghost", PositionField::Keep, PositionField::Keep, &ws.id),
            SetPositionError::AgentNotFound
        );
        err!(
            set(&a.id, PositionField::Keep, PositionField::Keep, &ws2.id),
            SetPositionError::WorkspaceMismatch
        );
        err!(
            set(
                &a.id,
                PositionField::Set("wizard".into()),
                PositionField::Keep,
                &ws.id
            ),
            SetPositionError::LevelInvalid
        );
        err!(
            set(
                &a.id,
                PositionField::Keep,
                PositionField::Set(a.id.clone()),
                &ws.id
            ),
            SetPositionError::SupervisorSelf
        );
        err!(
            set(
                &a.id,
                PositionField::Keep,
                PositionField::Set("ghost".into()),
                &ws.id
            ),
            SetPositionError::SupervisorNotFound
        );
        err!(
            set(
                &a.id,
                PositionField::Keep,
                PositionField::Set(foreign.id.clone()),
                &ws.id
            ),
            SetPositionError::SupervisorCrossWorkspace
        );
        // Cycle: a -> b, then b -> a.
        set_position_validated(
            &pool,
            &ws.id,
            &a.id,
            PositionField::Keep,
            PositionField::Set(b.id.clone()),
        )
        .await
        .expect("a -> b");
        err!(
            set(
                &b.id,
                PositionField::Keep,
                PositionField::Set(a.id.clone()),
                &ws.id
            ),
            SetPositionError::Cycle
        );
    }

    /// A MULTI-CONNECTION, file-backed WAL pool + its cleanup path (the temp
    /// file gets `-wal`/`-shm` siblings). `connect_in_memory` is
    /// max_connections(1) and would serialise concurrent calls on the single
    /// connection regardless of the fix — so the atomicity regression tests need
    /// a real pool where `BEGIN IMMEDIATE` is the ONLY thing serialising two
    /// racing writes on separate connections (mirrors the prod config,
    /// db.rs:42-49). Unique UUID path so concurrent `cargo test` runs never
    /// collide (see the flaky fixed-temp-path lesson).
    async fn connect_file_multi() -> (SqlitePool, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("conclave-pos-race-{}.db", Uuid::new_v4()));
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_secs(5));
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await
            .expect("open file-backed pool");
        crate::engine::db::migrate(&pool)
            .await
            .expect("migrate file pool");
        (pool, path)
    }

    fn cleanup_file(pool: SqlitePool, path: std::path::PathBuf) {
        drop(pool);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    /// FORCED-interleaving cycle race (ruling ef969027 harness clause): a bare
    /// `tokio::join!` can run sequentially and pass even against broken code, so
    /// this test MAKES the contention deterministic — writer A holds a
    /// `BEGIN IMMEDIATE` tx open (a->b written, uncommitted), then the
    /// conflicting `set_position_validated(b->a)` is launched on another
    /// connection and PROBED (bounded timeout) to confirm it BLOCKS on A's write
    /// lock rather than running to completion. Only after A commits can B
    /// proceed — and it must then see the committed a->b forest and REJECT the
    /// cycle. Demonstrated once to FAIL against the pre-c56d48e non-atomic path
    /// (see the task ledger note): there B's reads observe the pre-commit forest,
    /// find no cycle, and land b->a.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn forced_race_reciprocal_supervisor_rejects_the_cycle() {
        let (pool, path) = connect_file_multi().await;
        let ws = workspace::create(&pool, "WS", "/tmp/ws", None)
            .await
            .expect("ws");
        let a = instance_named(&pool, &ws.id, "A").await;
        let b = instance_named(&pool, &ws.id, "B").await;

        // Writer A: BEGIN IMMEDIATE, write a->b, HOLD (post-validate pre-commit).
        let mut conn_a = pool.acquire().await.expect("acquire A");
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *conn_a)
            .await
            .expect("begin A");
        sqlx::query("UPDATE workspace_agent SET supervisor_agent_id = ? WHERE id = ?")
            .bind(&b.id)
            .bind(&a.id)
            .execute(&mut *conn_a)
            .await
            .expect("A writes a->b");

        // Writer B: the conflicting b->a on a different connection.
        let (pb, wsb, aid, bid) = (pool.clone(), ws.id.clone(), a.id.clone(), b.id.clone());
        let mut b_task = tokio::spawn(async move {
            set_position_validated(
                &pb,
                &wsb,
                &bid,
                PositionField::Keep,
                PositionField::Set(aid),
            )
            .await
        });

        // PROBE: B must still be blocked after a bounded wait (not sleep-hope).
        let probe = tokio::time::timeout(std::time::Duration::from_millis(400), &mut b_task).await;
        assert!(
            probe.is_err(),
            "B must BLOCK on A's write lock, not complete"
        );

        // Release A → B unblocks, sees a->b, rejects the cycle.
        sqlx::query("COMMIT")
            .execute(&mut *conn_a)
            .await
            .expect("commit A");
        drop(conn_a);

        let b_result = b_task.await.expect("join B");
        assert!(
            matches!(b_result, Err(SetPositionError::Cycle)),
            "B must reject: {b_result:?}"
        );
        assert_eq!(
            supervisor_of(&pool, &b.id).await.unwrap(),
            None,
            "no cycle landed"
        );
        assert_eq!(
            supervisor_of(&pool, &a.id).await.unwrap(),
            Some(b.id.clone())
        );
        cleanup_file(pool, path);
    }

    /// FORCED-interleaving partial-update race: writer A holds a `BEGIN IMMEDIATE`
    /// tx open having set X.level='senior' (uncommitted); a `set_position_validated`
    /// that touches ONLY X's supervisor is launched, PROBED to confirm it blocks,
    /// then A commits. B must read the committed level inside its own tx and keep
    /// it — so BOTH fields survive. Against the pre-fix path B's `Keep` resolves
    /// to the stale pre-commit level (NULL) and the write wipes A's 'senior'.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn forced_race_partial_updates_preserve_both_fields() {
        let (pool, path) = connect_file_multi().await;
        let ws = workspace::create(&pool, "WS", "/tmp/ws", None)
            .await
            .expect("ws");
        let x = instance_named(&pool, &ws.id, "X").await;
        let sup = instance_named(&pool, &ws.id, "Sup").await;

        // Writer A: BEGIN IMMEDIATE, set X.level='senior', HOLD.
        let mut conn_a = pool.acquire().await.expect("acquire A");
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *conn_a)
            .await
            .expect("begin A");
        sqlx::query("UPDATE workspace_agent SET level = 'senior' WHERE id = ?")
            .bind(&x.id)
            .execute(&mut *conn_a)
            .await
            .expect("A writes level");

        // Writer B: supervisor-only (level Keep).
        let (pb, wsb, xid, sid) = (pool.clone(), ws.id.clone(), x.id.clone(), sup.id.clone());
        let mut b_task = tokio::spawn(async move {
            set_position_validated(
                &pb,
                &wsb,
                &xid,
                PositionField::Keep,
                PositionField::Set(sid),
            )
            .await
        });

        let probe = tokio::time::timeout(std::time::Duration::from_millis(400), &mut b_task).await;
        assert!(
            probe.is_err(),
            "B must BLOCK on A's write lock, not complete"
        );

        sqlx::query("COMMIT")
            .execute(&mut *conn_a)
            .await
            .expect("commit A");
        drop(conn_a);

        b_task
            .await
            .expect("join B")
            .expect("supervisor-only write");
        assert_eq!(
            level_of(&pool, &x.id).await.as_deref(),
            Some("senior"),
            "level survived"
        );
        assert_eq!(
            supervisor_of(&pool, &x.id).await.unwrap(),
            Some(sup.id.clone()),
            "supervisor survived"
        );
        cleanup_file(pool, path);
    }
}
