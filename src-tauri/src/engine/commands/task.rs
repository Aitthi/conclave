//! Task command handlers (ADR 0008) — first-class work items, the gate
//! ledger, and the watch/challenge/ruling event trail.
//!
//! # wire shapes (frozen — see docs/2026-07-04-plan-agent-work-system.md §Frozen)
//!
//! `Task` = `{"id","workspaceId","slug","title","state","ownerAgentId"?,
//! "implementerAgentId"?,"fileBoundary":[],"designCanon"?,"plan",
//! "createdAt","updatedAt"}` — optional fields are OMITTED when absent (same
//! convention as `BlackboardEntry.lastWriterId`), never emitted as `null`.
//!
//! `task.list` items are a `Task` plus `"eventCount"`. `task.get` returns
//! `{"task": Task, "events": TaskEvent[]}` (last 20, newest-first).
//! `TaskEvent` = `{"id","taskId","kind","actorAgentId"?,"payload","createdAt"}`
//! — `payload` is parsed from its stored TEXT column into a real JSON object;
//! an unparseable/non-object payload becomes `{}` rather than erroring (a
//! corrupt row must never break `task.get`).
//!
//! `task:changed` is emitted by every mutating handler (see `engine::bus`).
//!
//! # actor attribution
//!
//! Every mutating verb takes an optional/required `actorId` supplied by the
//! CLI layer (self-keyed from `CONCLAVE_INSTANCE_ID`, see
//! `bin/conclave-cli.rs`). When present it is validated to belong to the
//! task's workspace (same access-scope rule as `blackboard`'s writer/reader),
//! so a stale or foreign id fails loudly instead of silently attributing an
//! event to the wrong agent.

use crate::engine::repo::task::{TaskEventRow, TaskListRow, TaskOpError, TaskRow};
use crate::engine::{repo, AppError, AppState};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// Map a [`TaskOpError`] (business-rule rejection from the repo layer) to the
/// wire-facing [`AppError`].
impl From<TaskOpError> for AppError {
    fn from(e: TaskOpError) -> Self {
        match e {
            TaskOpError::NotFound => AppError::NotFound("task not found".into()),
            TaskOpError::Invalid(msg) => AppError::Invalid(msg),
            TaskOpError::Db(e) => AppError::from(e),
        }
    }
}

/// Validate that `workspace_id` exists, else [`AppError::NotFound`].
async fn require_workspace(state: &AppState, workspace_id: &str) -> Result<(), AppError> {
    if !repo::workspace::exists(&state.db, workspace_id).await? {
        return Err(AppError::NotFound(format!(
            "workspace id={workspace_id} not found"
        )));
    }
    Ok(())
}

/// Enforce that `instance_id` exists AND belongs to `workspace_id` (mirrors
/// `commands::blackboard::enforce_scope`) — a foreign/typo'd actor id fails
/// loudly rather than silently attributing an event to the wrong agent.
async fn enforce_scope(
    state: &AppState,
    workspace_id: &str,
    instance_id: &str,
    role: &str,
) -> Result<(), AppError> {
    let agent = repo::workspace_agent::get(&state.db, instance_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("{role} id={instance_id} not found")))?;
    if agent.workspace_id != workspace_id {
        return Err(AppError::Invalid(format!(
            "{role} does not belong to this workspace"
        )));
    }
    Ok(())
}

// ── wire-shape builders ──────────────────────────────────────────────────────

/// Parse a JSON-array TEXT column into a real JSON array; an unparseable or
/// non-array value falls back to `[]` (a task's `file_boundary` must always
/// round-trip as an array on the wire, never a raw string or `null`).
fn parse_array(text: &str) -> Value {
    match serde_json::from_str::<Value>(text) {
        Ok(v @ Value::Array(_)) => v,
        _ => Value::Array(vec![]),
    }
}

/// Parse a JSON-object TEXT column into a real JSON object; an unparseable or
/// non-object value falls back to `{}` (frozen contract: a corrupt
/// `task_event.payload` row must never break `task.get`).
fn parse_object(text: &str) -> Value {
    match serde_json::from_str::<Value>(text) {
        Ok(v @ Value::Object(_)) => v,
        _ => json!({}),
    }
}

/// Build the frozen `Task` wire shape from a [`TaskRow`].
fn task_to_json(row: &TaskRow) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("id".into(), json!(row.id));
    obj.insert("workspaceId".into(), json!(row.workspace_id));
    obj.insert("slug".into(), json!(row.slug));
    obj.insert("title".into(), json!(row.title));
    obj.insert("state".into(), json!(row.state));
    if let Some(owner) = &row.owner_agent_id {
        obj.insert("ownerAgentId".into(), json!(owner));
    }
    if let Some(implementer) = &row.implementer_agent_id {
        obj.insert("implementerAgentId".into(), json!(implementer));
    }
    obj.insert("fileBoundary".into(), parse_array(&row.file_boundary));
    if let Some(canon) = &row.design_canon {
        obj.insert("designCanon".into(), json!(canon));
    }
    obj.insert("plan".into(), json!(row.plan));
    obj.insert("createdAt".into(), json!(row.created_at));
    obj.insert("updatedAt".into(), json!(row.updated_at));
    Value::Object(obj)
}

/// Per-task derived board fields for `task.list` (RULED 2026-07-04 #2,
/// AMENDED same day — Arta fidelity F1): `lastGates` (the newest `gate` event
/// PER DISTINCT `cmd`, capped, so a card can show "test green + clippy red"
/// simultaneously) and `challenges` (every `challenge` event, `status`
/// resolved by whether a matching `ruling` exists, `deadlineAt` read
/// straight from the stored payload — never re-derived from minutes).
struct BoardExtras {
    last_gates: HashMap<String, Vec<Value>>,
    challenges: HashMap<String, Vec<Value>>,
}

/// One `challenge` event, pending the `ruled`/`open` verdict resolved after
/// every event has been scanned (see [`derive_board_extras`]).
struct PendingChallenge {
    id: String,
    claim: String,
    deadline_at: Option<String>,
}

/// Cap on distinct `cmd`s tracked per task in `lastGates` (RULED 2026-07-04,
/// Arta fidelity F1) — full gate history stays available via `task.get`.
const MAX_LAST_GATES: usize = 6;

/// Derive [`BoardExtras`] from one workspace's `gate`/`challenge`/`ruling`
/// events (oldest-first — see `repo::task::board_events_for_workspace`) in a
/// single pass, so `task.list` stays O(tasks + relevant events) instead of
/// O(tasks) round trips (the plan's explicit "not O(events)" note is about
/// avoiding N+1 `task.get` calls, not about scanning these rows once).
fn derive_board_extras(events: &[TaskEventRow]) -> BoardExtras {
    // task_id -> cmd -> that cmd's newest gate summary.
    let mut gate_by_task_cmd: HashMap<String, HashMap<String, Value>> = HashMap::new();
    let mut challenge_rows: HashMap<String, Vec<PendingChallenge>> = HashMap::new();
    let mut ruled_challenge_ids: HashSet<String> = HashSet::new();

    for e in events {
        let payload = parse_object(&e.payload);
        match e.kind.as_str() {
            // Oldest-first input means the LAST write per (task_id, cmd) is
            // the newest gate for that cmd — a plain insert overwrite is
            // correct; no sort needed until the final per-task collection.
            "gate" => {
                let cmd = payload
                    .get("cmd")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                gate_by_task_cmd.entry(e.task_id.clone()).or_default().insert(
                    cmd,
                    json!({
                        "cmd": payload.get("cmd").cloned().unwrap_or(Value::Null),
                        "exit": payload.get("exit").cloned().unwrap_or(Value::Null),
                        "sha": payload.get("sha").cloned().unwrap_or(Value::Null),
                        "createdAt": e.created_at,
                    }),
                );
            }
            "challenge" => {
                let claim = payload
                    .get("claim")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                // deadlineAt is read straight from the stored payload (an
                // absolute ISO instant the `challenge` handler computed at
                // insert time) — this layer never re-derives it from minutes.
                let deadline_at = payload
                    .get("deadlineAt")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                challenge_rows
                    .entry(e.task_id.clone())
                    .or_default()
                    .push(PendingChallenge {
                        id: e.id.clone(),
                        claim,
                        deadline_at,
                    });
            }
            "ruling" => {
                if let Some(challenge_id) = payload.get("challengeId").and_then(Value::as_str) {
                    ruled_challenge_ids.insert(challenge_id.to_string());
                }
            }
            _ => {}
        }
    }

    let last_gates = gate_by_task_cmd
        .into_iter()
        .map(|(task_id, by_cmd)| {
            let mut gates: Vec<Value> = by_cmd.into_values().collect();
            // Most-recent-first; RFC3339 timestamps parse into a total order
            // (string comparison alone risks mismatched fractional-second
            // widths, so parse rather than compare lexicographically).
            gates.sort_by(|a, b| {
                let parse = |v: &Value| {
                    v.get("createdAt")
                        .and_then(Value::as_str)
                        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                };
                parse(b).cmp(&parse(a))
            });
            gates.truncate(MAX_LAST_GATES);
            (task_id, gates)
        })
        .collect();

    let challenges = challenge_rows
        .into_iter()
        .map(|(task_id, rows)| {
            let items = rows
                .into_iter()
                .map(|c| {
                    let status = if ruled_challenge_ids.contains(&c.id) {
                        "ruled"
                    } else {
                        "open"
                    };
                    let mut obj = json!({ "id": c.id, "status": status, "claim": c.claim });
                    if let Some(deadline_at) = c.deadline_at {
                        obj["deadlineAt"] = json!(deadline_at);
                    }
                    obj
                })
                .collect();
            (task_id, items)
        })
        .collect();

    BoardExtras {
        last_gates,
        challenges,
    }
}

/// Build a `task.list` row: the frozen `Task` shape plus `eventCount`, plus
/// always-present `lastGates` and `challenges` — see [`BoardExtras`].
fn task_list_item_to_json(row: &TaskListRow, extras: &BoardExtras) -> Value {
    let task = TaskRow {
        id: row.id.clone(),
        workspace_id: row.workspace_id.clone(),
        slug: row.slug.clone(),
        title: row.title.clone(),
        state: row.state.clone(),
        owner_agent_id: row.owner_agent_id.clone(),
        implementer_agent_id: row.implementer_agent_id.clone(),
        file_boundary: row.file_boundary.clone(),
        design_canon: row.design_canon.clone(),
        plan: row.plan.clone(),
        created_at: row.created_at.clone(),
        updated_at: row.updated_at.clone(),
    };
    let mut obj = match task_to_json(&task) {
        Value::Object(m) => m,
        _ => unreachable!("task_to_json always returns an object"),
    };
    obj.insert("eventCount".into(), json!(row.event_count));
    obj.insert(
        "lastGates".into(),
        json!(extras.last_gates.get(&row.id).cloned().unwrap_or_default()),
    );
    obj.insert(
        "challenges".into(),
        json!(extras.challenges.get(&row.id).cloned().unwrap_or_default()),
    );
    Value::Object(obj)
}

/// Build the frozen `TaskEvent` wire shape from a [`TaskEventRow`].
fn task_event_to_json(row: &TaskEventRow) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("id".into(), json!(row.id));
    obj.insert("taskId".into(), json!(row.task_id));
    obj.insert("kind".into(), json!(row.kind));
    if let Some(actor) = &row.actor_agent_id {
        obj.insert("actorAgentId".into(), json!(actor));
    }
    obj.insert("payload".into(), parse_object(&row.payload));
    obj.insert("createdAt".into(), json!(row.created_at));
    Value::Object(obj)
}

/// Emit `task:changed` for a task after a mutation. Non-fatal (mirrors every
/// other `bus::*` emit call — a UI refresh miss is not a request failure).
fn emit_changed(state: &AppState, task: &TaskRow) {
    state.emit(
        crate::engine::bus::TASK_CHANGED,
        crate::engine::bus::TaskChanged {
            workspace_id: task.workspace_id.clone(),
            task_id: task.id.clone(),
            slug: task.slug.clone(),
            state: task.state.clone(),
        },
    );
}

// ── task.create ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateReq {
    workspace_id: String,
    slug: String,
    title: String,
    owner_agent_id: Option<String>,
    #[serde(default)]
    file_boundary: Vec<String>,
    design_canon: Option<String>,
    #[serde(default)]
    plan: String,
}

/// Create a task. `ownerAgentId`, when supplied, must belong to `workspaceId`.
/// Rejects a duplicate `(workspaceId, slug)` with [`AppError::Invalid`].
pub async fn create(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: CreateReq = serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;
    if let Some(owner) = &req.owner_agent_id {
        enforce_scope(state, &req.workspace_id, owner, "owner").await?;
    }
    if repo::task::get(&state.db, &req.workspace_id, &req.slug)
        .await?
        .is_some()
    {
        return Err(AppError::Invalid(format!(
            "task '{}' already exists in this workspace",
            req.slug
        )));
    }

    let file_boundary_json =
        serde_json::to_string(&req.file_boundary).map_err(|e| AppError::Internal(e.to_string()))?;
    let row = repo::task::create(
        &state.db,
        repo::task::NewTask {
            workspace_id: &req.workspace_id,
            slug: &req.slug,
            title: &req.title,
            owner_agent_id: req.owner_agent_id.as_deref(),
            file_boundary_json: &file_boundary_json,
            design_canon: req.design_canon.as_deref(),
            plan: &req.plan,
        },
    )
    .await?;

    emit_changed(state, &row);
    Ok(task_to_json(&row))
}

// ── task.list ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListReq {
    workspace_id: String,
    state: Option<String>,
}

/// List a workspace's tasks (optionally filtered by `state`), each row
/// carrying its `eventCount` plus the always-present derived board fields
/// `lastGates` and `challenges` (both `[]` when none) — RULED 2026-07-04 #2
/// (amended same day, Arta fidelity F1), see [`BoardExtras`].
pub async fn list(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: ListReq = serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;

    let rows = repo::task::list(&state.db, &req.workspace_id, req.state.as_deref()).await?;
    let events = repo::task::board_events_for_workspace(&state.db, &req.workspace_id).await?;
    let extras = derive_board_extras(&events);
    Ok(Value::Array(
        rows.iter()
            .map(|row| task_list_item_to_json(row, &extras))
            .collect(),
    ))
}

// ── task.get ──────────────────────────────────────────────────────────────

const LAST_EVENTS_LIMIT: i64 = 20;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetReq {
    workspace_id: String,
    slug: String,
}

/// Fetch one task plus its last 20 events (newest-first). Returns
/// [`AppError::NotFound`] when the task does not exist.
pub async fn get(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: GetReq = serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;

    let row = repo::task::get(&state.db, &req.workspace_id, &req.slug)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("task '{}' not found", req.slug)))?;
    let events = repo::task::events_for(&state.db, &row.id, LAST_EVENTS_LIMIT).await?;

    Ok(json!({
        "task": task_to_json(&row),
        "events": events.iter().map(task_event_to_json).collect::<Vec<_>>(),
    }))
}

// ── task.claim ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaimReq {
    workspace_id: String,
    slug: String,
    actor_id: String,
}

/// Claim a task: sets `implementerAgentId = actorId`, `planned -> claimed`.
/// Rejects a non-`planned` task with [`AppError::Invalid`] (already claimed,
/// merged, …).
pub async fn claim(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: ClaimReq = serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;
    enforce_scope(state, &req.workspace_id, &req.actor_id, "actor").await?;

    let row = repo::task::claim(&state.db, &req.workspace_id, &req.slug, &req.actor_id).await?;
    emit_changed(state, &row);
    Ok(task_to_json(&row))
}

// ── task.setState ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetStateReq {
    workspace_id: String,
    slug: String,
    state: String,
    actor_id: Option<String>,
}

/// Transition a task's state. Invalid moves (e.g. `merged -> claimed`) return
/// [`AppError::Invalid`] — see `repo::task::valid_transition`.
pub async fn set_state(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: SetStateReq = serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;
    if let Some(actor) = &req.actor_id {
        enforce_scope(state, &req.workspace_id, actor, "actor").await?;
    }

    let row = repo::task::set_state(
        &state.db,
        &req.workspace_id,
        &req.slug,
        &req.state,
        req.actor_id.as_deref(),
    )
    .await?;
    emit_changed(state, &row);
    Ok(task_to_json(&row))
}

// ── task.note ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NoteReq {
    workspace_id: String,
    slug: String,
    actor_id: Option<String>,
    text: String,
}

/// Append a free-text `note` event (replaces the old bb `progress:` convention).
pub async fn note(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: NoteReq = serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;
    if let Some(actor) = &req.actor_id {
        enforce_scope(state, &req.workspace_id, actor, "actor").await?;
    }

    let payload_json = json!({ "text": req.text }).to_string();
    let event = repo::task::add_note(
        &state.db,
        &req.workspace_id,
        &req.slug,
        req.actor_id.as_deref(),
        &payload_json,
    )
    .await?;
    notify_task_changed_for_event(state, &req.workspace_id, &req.slug).await?;
    Ok(task_event_to_json(&event))
}

// ── task.gate ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GateReq {
    workspace_id: String,
    slug: String,
    actor_id: Option<String>,
    cmd: String,
    exit: i64,
    sha: String,
    tail: String,
    cwd: String,
}

/// Append a `gate` event. The command itself already ran client-side (see
/// `bin/conclave-cli.rs` — gates NEVER run engine-side, ADR 0008 risk ledger);
/// this just records the evidence. A non-zero `exit` is recorded exactly like
/// a zero one — a red gate is evidence, not a request failure.
pub async fn gate(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: GateReq = serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;
    if let Some(actor) = &req.actor_id {
        enforce_scope(state, &req.workspace_id, actor, "actor").await?;
    }

    let payload_json = json!({
        "cmd": req.cmd,
        "exit": req.exit,
        "sha": req.sha,
        "tail": req.tail,
        "cwd": req.cwd,
    })
    .to_string();
    let event = repo::task::add_gate(
        &state.db,
        &req.workspace_id,
        &req.slug,
        req.actor_id.as_deref(),
        &payload_json,
    )
    .await?;
    notify_task_changed_for_event(state, &req.workspace_id, &req.slug).await?;
    Ok(task_event_to_json(&event))
}

// ── task.challenge ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChallengeReq {
    workspace_id: String,
    slug: String,
    actor_id: Option<String>,
    claim: String,
    evidence: String,
    proposal: String,
    #[serde(rename = "default")]
    default_action: String,
    deadline_min: Option<i64>,
}

/// Append a `challenge` event. `--deadline-min N` is CLI input sugar only —
/// the STORED payload carries an absolute `deadlineAt` ISO timestamp
/// (`now + deadlineMin`), computed HERE at insert time (RULED 2026-07-04,
/// Tiësto's pre-merge audit: every downstream reader — Lane B's stall timer,
/// `task.list`'s derive, `task.get` — reads `deadlineAt`; nobody re-derives
/// from relative minutes, which would drift with read time). Absent
/// `deadlineMin` means advisory (no stall/default timer — Lane B).
pub async fn challenge(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: ChallengeReq = serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;
    if let Some(actor) = &req.actor_id {
        enforce_scope(state, &req.workspace_id, actor, "actor").await?;
    }

    let mut payload_obj = json!({
        "claim": req.claim,
        "evidence": req.evidence,
        "proposal": req.proposal,
        "default": req.default_action,
    });
    if let Some(deadline_min) = req.deadline_min {
        let deadline_at = Utc::now() + Duration::minutes(deadline_min);
        payload_obj["deadlineAt"] = json!(deadline_at.to_rfc3339());
    }
    let event = repo::task::add_challenge(
        &state.db,
        &req.workspace_id,
        &req.slug,
        req.actor_id.as_deref(),
        &payload_obj.to_string(),
    )
    .await?;
    notify_task_changed_for_event(state, &req.workspace_id, &req.slug).await?;
    Ok(task_event_to_json(&event))
}

// ── task.rule ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuleReq {
    workspace_id: String,
    slug: String,
    actor_id: String,
    challenge_event_id: String,
    text: String,
}

/// Append a `ruling` event resolving a prior `challenge`. `payload.by` is
/// always the ruling actor (frozen: `{"challengeId","text","by"}`).
pub async fn rule(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: RuleReq = serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;
    enforce_scope(state, &req.workspace_id, &req.actor_id, "actor").await?;

    let payload_json = json!({
        "challengeId": req.challenge_event_id,
        "text": req.text,
        "by": req.actor_id,
    })
    .to_string();
    let event = repo::task::add_ruling(
        &state.db,
        &req.workspace_id,
        &req.slug,
        Some(&req.actor_id),
        &payload_json,
    )
    .await?;
    notify_task_changed_for_event(state, &req.workspace_id, &req.slug).await?;
    Ok(task_event_to_json(&event))
}

// ── task.close ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloseReq {
    workspace_id: String,
    slug: String,
    actor_id: Option<String>,
}

/// Close a task: `{claimed,in_progress,review} -> merged`.
pub async fn close(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: CloseReq = serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;
    if let Some(actor) = &req.actor_id {
        enforce_scope(state, &req.workspace_id, actor, "actor").await?;
    }

    let row = repo::task::close(&state.db, &req.workspace_id, &req.slug, req.actor_id.as_deref()).await?;
    emit_changed(state, &row);
    Ok(task_to_json(&row))
}

// ── task.watch / task.unwatch ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WatchReq {
    workspace_id: String,
    slug: String,
    actor_id: String,
}

/// Subscribe `actorId` to a task's `task_event` changes (Lane B notify hook).
pub async fn watch(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: WatchReq = serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;
    enforce_scope(state, &req.workspace_id, &req.actor_id, "actor").await?;

    repo::task::watch(&state.db, &req.workspace_id, &req.slug, &req.actor_id).await?;
    notify_task_changed_for_event(state, &req.workspace_id, &req.slug).await?;
    Ok(json!({ "watching": true }))
}

/// Unsubscribe `actorId` from a task. Idempotent.
pub async fn unwatch(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: WatchReq = serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;
    enforce_scope(state, &req.workspace_id, &req.actor_id, "actor").await?;

    repo::task::unwatch(&state.db, &req.workspace_id, &req.slug, &req.actor_id).await?;
    notify_task_changed_for_event(state, &req.workspace_id, &req.slug).await?;
    Ok(json!({ "watching": false }))
}

/// Re-fetch the task and emit `task:changed` — shared by the event-only verbs
/// (`note`/`gate`/`challenge`/`rule`/`watch`/`unwatch`) which don't already
/// hold the row the way `claim`/`setState`/`close` do.
async fn notify_task_changed_for_event(
    state: &AppState,
    workspace_id: &str,
    slug: &str,
) -> Result<(), AppError> {
    if let Some(row) = repo::task::get(&state.db, workspace_id, slug).await? {
        emit_changed(state, &row);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::repo::{
        agent_definition::{self, AgentDefinitionInput},
        workspace, workspace_agent,
    };

    async fn fixture_workspace(state: &AppState) -> String {
        workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed")
            .id
    }

    async fn fixture_instance(state: &AppState, workspace_id: &str, name: &str) -> String {
        let def = agent_definition::create(
            &state.db,
            &AgentDefinitionInput {
                name: name.into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed");
        workspace_agent::instantiate(&state.db, workspace_id, &def.id)
            .await
            .expect("instantiate failed")
            .id
    }

    // ── task.create ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn create_returns_frozen_wire_shape_and_omits_optionals() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;

        let created = create(
            &state,
            json!({ "workspaceId": ws, "slug": "my-task", "title": "My Task" }),
        )
        .await
        .expect("create failed");

        assert_eq!(created["slug"], json!("my-task"));
        assert_eq!(created["title"], json!("My Task"));
        assert_eq!(created["state"], json!("planned"));
        assert_eq!(created["fileBoundary"], json!([]));
        assert_eq!(created["plan"], json!(""));
        assert!(created.get("ownerAgentId").is_none(), "must omit when absent");
        assert!(created.get("implementerAgentId").is_none());
        assert!(created.get("designCanon").is_none());
        assert!(created.get("id").is_some());
        assert!(created.get("createdAt").is_some());
    }

    #[tokio::test]
    async fn create_with_owner_and_boundary_round_trips() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let owner = fixture_instance(&state, &ws, "Lead").await;

        let created = create(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "title": "T1",
                "ownerAgentId": owner, "fileBoundary": ["src/a.rs", "src/b.rs"],
                "designCanon": "canon-x", "plan": "do the thing"
            }),
        )
        .await
        .expect("create failed");

        assert_eq!(created["ownerAgentId"], json!(owner));
        assert_eq!(created["fileBoundary"], json!(["src/a.rs", "src/b.rs"]));
        assert_eq!(created["designCanon"], json!("canon-x"));
        assert_eq!(created["plan"], json!("do the thing"));
    }

    #[tokio::test]
    async fn create_duplicate_slug_is_invalid() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        create(&state, json!({ "workspaceId": ws, "slug": "dup", "title": "A" }))
            .await
            .expect("first create failed");

        let err = create(&state, json!({ "workspaceId": ws, "slug": "dup", "title": "B" }))
            .await
            .expect_err("duplicate slug must fail");
        assert!(matches!(err, AppError::Invalid(_)));
    }

    #[tokio::test]
    async fn create_unknown_owner_is_not_found() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let err = create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1", "ownerAgentId": "nope" }),
        )
        .await
        .expect_err("unknown owner must fail");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn create_unknown_workspace_is_not_found() {
        let state = AppState::for_tests().await;
        let err = create(
            &state,
            json!({ "workspaceId": "nope", "slug": "t1", "title": "T1" }),
        )
        .await
        .expect_err("unknown workspace must fail");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    // ── task.list ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn list_includes_event_count_and_filters_by_state() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        create(&state, json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }))
            .await
            .expect("create t1");
        create(&state, json!({ "workspaceId": ws, "slug": "t2", "title": "T2" }))
            .await
            .expect("create t2");
        note(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": actor, "text": "hi" }),
        )
        .await
        .expect("note failed");

        let listed = list(&state, json!({ "workspaceId": ws })).await.expect("list failed");
        let arr = listed.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        let t1 = arr.iter().find(|t| t["slug"] == "t1").expect("t1 present");
        assert_eq!(t1["eventCount"], json!(1));
        let t2 = arr.iter().find(|t| t["slug"] == "t2").expect("t2 present");
        assert_eq!(t2["eventCount"], json!(0));

        claim(&state, json!({ "workspaceId": ws, "slug": "t1", "actorId": actor }))
            .await
            .expect("claim failed");
        let filtered = list(&state, json!({ "workspaceId": ws, "state": "claimed" }))
            .await
            .expect("filtered list failed");
        let filtered_arr = filtered.as_array().expect("array");
        assert_eq!(filtered_arr.len(), 1);
        assert_eq!(filtered_arr[0]["slug"], json!("t1"));
    }

    #[tokio::test]
    async fn list_last_gates_is_empty_array_when_no_gate_events() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        create(&state, json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }))
            .await
            .expect("create failed");

        let listed = list(&state, json!({ "workspaceId": ws })).await.expect("list failed");
        let t1 = &listed.as_array().unwrap()[0];
        assert_eq!(t1["lastGates"], json!([]), "no gates -> empty array, always present");
        assert_eq!(t1["challenges"], json!([]), "no challenges -> empty array, always present");
    }

    #[tokio::test]
    async fn list_last_gates_collapses_repeats_of_the_same_cmd_to_the_newest() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        create(&state, json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }))
            .await
            .expect("create failed");

        gate(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": actor,
                "cmd": "cargo test", "exit": 101, "sha": "sha1", "tail": "fail", "cwd": "/repo"
            }),
        )
        .await
        .expect("gate 1 failed");
        gate(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": actor,
                "cmd": "cargo test", "exit": 0, "sha": "sha2", "tail": "ok", "cwd": "/repo"
            }),
        )
        .await
        .expect("gate 2 failed");

        let listed = list(&state, json!({ "workspaceId": ws })).await.expect("list failed");
        let t1 = &listed.as_array().unwrap()[0];
        let last_gates = t1["lastGates"].as_array().expect("lastGates present");
        assert_eq!(last_gates.len(), 1, "same cmd re-run must collapse to one entry");
        let entry = &last_gates[0];
        assert_eq!(entry["cmd"], json!("cargo test"));
        assert_eq!(entry["exit"], json!(0), "must be the NEWEST run of this cmd");
        assert_eq!(entry["sha"], json!("sha2"));
        assert!(entry.get("createdAt").is_some());
        let mut keys: Vec<&str> = entry.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["cmd", "createdAt", "exit", "sha"], "no extra fields beyond the frozen shape");
    }

    #[tokio::test]
    async fn list_last_gates_carries_one_entry_per_distinct_cmd_newest_first() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        create(&state, json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }))
            .await
            .expect("create failed");

        gate(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": actor,
                "cmd": "cargo test", "exit": 0, "sha": "s1", "tail": "ok", "cwd": "/repo"
            }),
        )
        .await
        .expect("gate 1 failed");
        gate(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": actor,
                "cmd": "cargo clippy", "exit": 101, "sha": "s2", "tail": "warn", "cwd": "/repo"
            }),
        )
        .await
        .expect("gate 2 failed");

        let listed = list(&state, json!({ "workspaceId": ws })).await.expect("list failed");
        let t1 = &listed.as_array().unwrap()[0];
        let last_gates = t1["lastGates"].as_array().expect("lastGates present");
        assert_eq!(last_gates.len(), 2, "distinct cmds must BOTH appear (test green + clippy red)");
        assert_eq!(last_gates[0]["cmd"], json!("cargo clippy"), "most-recent-first");
        assert_eq!(last_gates[0]["exit"], json!(101));
        assert_eq!(last_gates[1]["cmd"], json!("cargo test"));
        assert_eq!(last_gates[1]["exit"], json!(0));
    }

    #[tokio::test]
    async fn list_last_gates_caps_at_six_distinct_cmds() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        create(&state, json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }))
            .await
            .expect("create failed");

        for i in 0..8 {
            gate(
                &state,
                json!({
                    "workspaceId": ws, "slug": "t1", "actorId": actor,
                    "cmd": format!("cmd-{i}"), "exit": 0, "sha": "s", "tail": "ok", "cwd": "/repo"
                }),
            )
            .await
            .expect("gate failed");
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }

        let listed = list(&state, json!({ "workspaceId": ws })).await.expect("list failed");
        let t1 = &listed.as_array().unwrap()[0];
        let last_gates = t1["lastGates"].as_array().expect("lastGates present");
        assert_eq!(last_gates.len(), 6, "capped at 6 distinct cmds");
        // Most-recent-first: the 6 KEPT must be cmd-7 down to cmd-2 (the two
        // oldest, cmd-0 and cmd-1, are evicted).
        let kept: Vec<&str> = last_gates.iter().map(|g| g["cmd"].as_str().unwrap()).collect();
        assert_eq!(
            kept,
            vec!["cmd-7", "cmd-6", "cmd-5", "cmd-4", "cmd-3", "cmd-2"]
        );
    }

    #[tokio::test]
    async fn list_challenges_status_open_then_ruled_with_deadline() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        let lead = fixture_instance(&state, &ws, "Lead").await;
        create(&state, json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }))
            .await
            .expect("create failed");

        let challenge_event = challenge(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": actor,
                "claim": "X broken", "evidence": "log", "proposal": "fix Y",
                "default": "escalate", "deadlineMin": 30
            }),
        )
        .await
        .expect("challenge failed");
        // Storage ruling: the STORED payload carries the absolute deadlineAt,
        // never deadlineMin — verify at the source, not just at list-derive.
        assert!(challenge_event["payload"].get("deadlineMin").is_none());
        let stored_deadline_at = challenge_event["payload"]["deadlineAt"]
            .as_str()
            .expect("payload must carry deadlineAt")
            .to_string();

        let listed = list(&state, json!({ "workspaceId": ws })).await.expect("list failed");
        let t1 = &listed.as_array().unwrap()[0];
        let challenges = t1["challenges"].as_array().expect("challenges array");
        assert_eq!(challenges.len(), 1);
        assert_eq!(challenges[0]["status"], json!("open"), "no ruling yet");
        assert_eq!(challenges[0]["claim"], json!("X broken"));
        assert_eq!(challenges[0]["id"], challenge_event["id"]);
        // list must READ deadlineAt, never re-derive it from minutes.
        assert_eq!(challenges[0]["deadlineAt"], json!(stored_deadline_at));

        rule(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": lead,
                "challengeEventId": challenge_event["id"], "text": "go with fix Y"
            }),
        )
        .await
        .expect("rule failed");

        let listed = list(&state, json!({ "workspaceId": ws })).await.expect("list failed");
        let t1 = &listed.as_array().unwrap()[0];
        let challenges = t1["challenges"].as_array().expect("challenges array");
        assert_eq!(challenges.len(), 1, "still one challenge, now ruled");
        assert_eq!(challenges[0]["status"], json!("ruled"));
    }

    #[tokio::test]
    async fn list_challenge_without_deadline_omits_deadline_at() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        create(&state, json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }))
            .await
            .expect("create failed");
        challenge(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": actor,
                "claim": "c", "evidence": "e", "proposal": "p", "default": "d"
            }),
        )
        .await
        .expect("challenge failed");

        let listed = list(&state, json!({ "workspaceId": ws })).await.expect("list failed");
        let t1 = &listed.as_array().unwrap()[0];
        let challenges = t1["challenges"].as_array().unwrap();
        assert!(
            challenges[0].get("deadlineAt").is_none(),
            "advisory challenge (no deadlineMin) must omit deadlineAt"
        );
    }

    #[tokio::test]
    async fn list_derived_fields_are_scoped_per_task() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        create(&state, json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }))
            .await
            .expect("create t1");
        create(&state, json!({ "workspaceId": ws, "slug": "t2", "title": "T2" }))
            .await
            .expect("create t2");
        gate(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": actor,
                "cmd": "cargo test", "exit": 0, "sha": "sha1", "tail": "ok", "cwd": "/repo"
            }),
        )
        .await
        .expect("gate on t1 failed");

        let listed = list(&state, json!({ "workspaceId": ws })).await.expect("list failed");
        let arr = listed.as_array().unwrap();
        let t1 = arr.iter().find(|t| t["slug"] == "t1").unwrap();
        let t2 = arr.iter().find(|t| t["slug"] == "t2").unwrap();
        assert_eq!(t1["lastGates"].as_array().unwrap().len(), 1, "t1 has a gate");
        assert_eq!(t2["lastGates"], json!([]), "t2's list row must not see t1's gate");
    }

    // ── task.get ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn get_returns_task_plus_events_newest_first() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        create(&state, json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }))
            .await
            .expect("create failed");
        note(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": actor, "text": "first" }),
        )
        .await
        .expect("note 1 failed");
        note(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": actor, "text": "second" }),
        )
        .await
        .expect("note 2 failed");

        let got = get(&state, json!({ "workspaceId": ws, "slug": "t1" }))
            .await
            .expect("get failed");
        assert_eq!(got["task"]["slug"], json!("t1"));
        let events = got["events"].as_array().expect("events array");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["payload"]["text"], json!("second"), "newest first");
        assert_eq!(events[0]["kind"], json!("note"));
        assert_eq!(events[0]["actorAgentId"], json!(actor));
    }

    #[tokio::test]
    async fn get_missing_task_is_not_found() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let err = get(&state, json!({ "workspaceId": ws, "slug": "nope" }))
            .await
            .expect_err("missing task must fail");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    // ── task.claim ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn claim_sets_implementer_and_emits_state_event() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        create(&state, json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }))
            .await
            .expect("create failed");

        let claimed = claim(&state, json!({ "workspaceId": ws, "slug": "t1", "actorId": actor }))
            .await
            .expect("claim failed");
        assert_eq!(claimed["state"], json!("claimed"));
        assert_eq!(claimed["implementerAgentId"], json!(actor));
    }

    #[tokio::test]
    async fn claim_by_foreign_actor_is_invalid() {
        let state = AppState::for_tests().await;
        let ws1 = fixture_workspace(&state).await;
        let ws2 = fixture_workspace(&state).await;
        let foreigner = fixture_instance(&state, &ws2, "Foreign").await;
        create(&state, json!({ "workspaceId": ws1, "slug": "t1", "title": "T1" }))
            .await
            .expect("create failed");

        let err = claim(
            &state,
            json!({ "workspaceId": ws1, "slug": "t1", "actorId": foreigner }),
        )
        .await
        .expect_err("cross-workspace actor must fail");
        assert!(matches!(err, AppError::Invalid(_)));
    }

    #[tokio::test]
    async fn reclaim_an_already_claimed_task_is_invalid() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        create(&state, json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }))
            .await
            .expect("create failed");
        claim(&state, json!({ "workspaceId": ws, "slug": "t1", "actorId": actor }))
            .await
            .expect("first claim failed");

        let err = claim(&state, json!({ "workspaceId": ws, "slug": "t1", "actorId": actor }))
            .await
            .expect_err("re-claim must fail");
        assert!(matches!(err, AppError::Invalid(_)));
    }

    // ── task.setState ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn set_state_rejects_merged_to_claimed() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        create(&state, json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }))
            .await
            .expect("create failed");
        claim(&state, json!({ "workspaceId": ws, "slug": "t1", "actorId": actor }))
            .await
            .expect("claim failed");
        close(&state, json!({ "workspaceId": ws, "slug": "t1", "actorId": actor }))
            .await
            .expect("close failed");

        let err = set_state(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "state": "claimed", "actorId": actor }),
        )
        .await
        .expect_err("merged -> claimed must fail");
        assert!(matches!(err, AppError::Invalid(_)));
    }

    // ── task.gate ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn gate_records_non_zero_exit_with_full_payload() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        create(&state, json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }))
            .await
            .expect("create failed");

        let event = gate(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": actor,
                "cmd": "cargo test --lib", "exit": 101, "sha": "deadbeef",
                "tail": "FAILED", "cwd": "/repo"
            }),
        )
        .await
        .expect("gate must record even on non-zero exit");

        assert_eq!(event["kind"], json!("gate"));
        assert_eq!(event["payload"]["cmd"], json!("cargo test --lib"));
        assert_eq!(event["payload"]["exit"], json!(101));
        assert_eq!(event["payload"]["sha"], json!("deadbeef"));
        assert_eq!(event["payload"]["tail"], json!("FAILED"));
        assert_eq!(event["payload"]["cwd"], json!("/repo"));
    }

    // ── task.note ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn note_on_missing_task_is_not_found() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let err = note(&state, json!({ "workspaceId": ws, "slug": "nope", "text": "x" }))
            .await
            .expect_err("missing task must fail");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    // ── task.challenge / task.rule ────────────────────────────────────────

    #[tokio::test]
    async fn challenge_then_rule_round_trip() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        let lead = fixture_instance(&state, &ws, "Lead").await;
        create(&state, json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }))
            .await
            .expect("create failed");

        let challenge_event = challenge(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": actor,
                "claim": "X is broken", "evidence": "see log", "proposal": "fix Y",
                "default": "escalate", "deadlineMin": 30
            }),
        )
        .await
        .expect("challenge failed");
        assert_eq!(challenge_event["kind"], json!("challenge"));
        assert_eq!(challenge_event["payload"]["default"], json!("escalate"));
        // Storage ruling: --deadline-min is CLI input sugar; the STORED
        // payload carries the absolute deadlineAt, never deadlineMin.
        assert!(challenge_event["payload"].get("deadlineMin").is_none());
        assert!(challenge_event["payload"]["deadlineAt"].as_str().is_some());

        let ruling = rule(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": lead,
                "challengeEventId": challenge_event["id"], "text": "go with fix Y"
            }),
        )
        .await
        .expect("rule failed");
        assert_eq!(ruling["kind"], json!("ruling"));
        assert_eq!(ruling["payload"]["challengeId"], challenge_event["id"]);
        assert_eq!(ruling["payload"]["by"], json!(lead));
        assert_eq!(ruling["payload"]["text"], json!("go with fix Y"));
    }

    #[tokio::test]
    async fn challenge_without_deadline_omits_deadline_at() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        create(&state, json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }))
            .await
            .expect("create failed");

        let event = challenge(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": actor,
                "claim": "c", "evidence": "e", "proposal": "p", "default": "d"
            }),
        )
        .await
        .expect("challenge failed");
        assert!(
            event["payload"].get("deadlineMin").is_none(),
            "input field must never leak into storage"
        );
        assert!(
            event["payload"].get("deadlineAt").is_none(),
            "advisory challenge (no --deadline-min) must omit deadlineAt"
        );
    }

    // ── task.close ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn close_from_planned_is_invalid() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        create(&state, json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }))
            .await
            .expect("create failed");

        let err = close(&state, json!({ "workspaceId": ws, "slug": "t1" }))
            .await
            .expect_err("close from planned must fail");
        assert!(matches!(err, AppError::Invalid(_)));
    }

    // ── task.watch / task.unwatch ─────────────────────────────────────────

    #[tokio::test]
    async fn watch_then_unwatch_round_trip() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        create(&state, json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }))
            .await
            .expect("create failed");

        let watched = watch(&state, json!({ "workspaceId": ws, "slug": "t1", "actorId": actor }))
            .await
            .expect("watch failed");
        assert_eq!(watched["watching"], json!(true));

        let unwatched = unwatch(&state, json!({ "workspaceId": ws, "slug": "t1", "actorId": actor }))
            .await
            .expect("unwatch failed");
        assert_eq!(unwatched["watching"], json!(false));
    }

    #[tokio::test]
    async fn watch_unknown_actor_is_not_found() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        create(&state, json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }))
            .await
            .expect("create failed");

        let err = watch(&state, json!({ "workspaceId": ws, "slug": "t1", "actorId": "nope" }))
            .await
            .expect_err("unknown actor must fail");
        assert!(matches!(err, AppError::NotFound(_)));
    }
}
