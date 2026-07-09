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

const DEFAULT_BRIEF_LIMIT: usize = 10;
const MAX_BRIEF_LIMIT: usize = 20;

fn excerpt_lines(text: &str, limit: usize) -> (String, bool) {
    let lines: Vec<&str> = text.lines().collect();
    let truncated = lines.len() > limit;
    let excerpt = lines.into_iter().take(limit).collect::<Vec<_>>().join("\n");
    (excerpt, truncated)
}

fn parse_json_array_strings(text: &str) -> Vec<String> {
    parse_array(text)
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn brief_memory_query(row: &TaskRow) -> String {
    let mut parts = vec![row.slug.clone(), row.title.clone()];
    if let Some(canon) = &row.design_canon {
        parts.push(canon.clone());
    }
    parts.extend(
        parse_json_array_strings(&row.file_boundary)
            .into_iter()
            .take(3),
    );
    parts.join(" ")
}

fn task_brief_to_json(row: &TaskRow, plan_excerpt: String, plan_truncated: bool) -> Value {
    let mut task = match task_to_json(row) {
        Value::Object(map) => map,
        _ => unreachable!("task_to_json always returns an object"),
    };
    task.remove("plan");
    task.insert("planExcerpt".into(), json!(plan_excerpt));
    task.insert("planTruncated".into(), json!(plan_truncated));
    Value::Object(task)
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

struct BriefBoardSections {
    latest_gates: Vec<Value>,
    open_challenges: Vec<Value>,
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
                gate_by_task_cmd
                    .entry(e.task_id.clone())
                    .or_default()
                    .insert(
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

fn derive_brief_board_sections(events: &[TaskEventRow], task_id: &str) -> BriefBoardSections {
    let mut gate_by_cmd: HashMap<String, Value> = HashMap::new();
    let mut challenge_rows: Vec<PendingChallenge> = Vec::new();
    let mut ruled_challenge_ids: HashSet<String> = HashSet::new();

    for e in events.iter().filter(|event| event.task_id == task_id) {
        let payload = parse_object(&e.payload);
        match e.kind.as_str() {
            "gate" => {
                let cmd = payload
                    .get("cmd")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                gate_by_cmd.insert(
                    cmd,
                    json!({
                        "id": e.id,
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
                let deadline_at = payload
                    .get("deadlineAt")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                challenge_rows.push(PendingChallenge {
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

    let mut latest_gates: Vec<Value> = gate_by_cmd.into_values().collect();
    latest_gates.sort_by(|a, b| {
        let parse = |v: &Value| {
            v.get("createdAt")
                .and_then(Value::as_str)
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        };
        parse(b).cmp(&parse(a))
    });

    let open_challenges = challenge_rows
        .into_iter()
        .filter(|challenge| !ruled_challenge_ids.contains(&challenge.id))
        .map(|challenge| {
            let mut obj = json!({
                "id": challenge.id,
                "claim": challenge.claim,
                "status": "open",
            });
            if let Some(deadline_at) = challenge.deadline_at {
                obj["deadlineAt"] = json!(deadline_at);
            }
            obj
        })
        .collect();

    BriefBoardSections {
        latest_gates,
        open_challenges,
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

/// Build a `task.list` row in the SLIM shape (CLI-side board orientation,
/// ruling 3f1ab20e on task cli-output-economy): ONLY `slug`, `state`, `title`,
/// `implementerAgentId`, `updatedAt`, and `openChallenges` (a COUNT of
/// unruled challenges, not the rows). No plan, no boundary, no `lastGates` —
/// the full shape stays available via `task.get`/`task.brief` and the no-slim
/// list.
fn slim_task_list_item_to_json(row: &TaskListRow, extras: &BoardExtras) -> Value {
    let open_challenges = extras
        .challenges
        .get(&row.id)
        .map(|items| {
            items
                .iter()
                .filter(|c| c.get("status").and_then(Value::as_str) == Some("open"))
                .count()
        })
        .unwrap_or(0);
    json!({
        "slug": row.slug,
        "state": row.state,
        "title": row.title,
        "implementerAgentId": row.implementer_agent_id,
        "updatedAt": row.updated_at,
        "openChallenges": open_challenges,
    })
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
    let req: CreateReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
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
    include_plan: Option<bool>,
    slim: Option<bool>,
    all: Option<bool>,
}

/// List a workspace's tasks (optionally filtered by `state`), each row
/// carrying its `eventCount` plus the always-present derived board fields
/// `lastGates` and `challenges` (both `[]` when none) — RULED 2026-07-04 #2
/// (amended same day, Arta fidelity F1), see [`BoardExtras`].
///
/// `slim: true` (CLI board orientation, ruling 3f1ab20e) swaps each row for
/// the low-context shape from [`slim_task_list_item_to_json`] and hides
/// merged/abandoned tasks unless `all: true` or an explicit `state` filter
/// asks for them. Payloads that do not send `slim` — the Tauri UI — get
/// today's exact full shape; that invariant is regression-tested.
pub async fn list(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: ListReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;

    let slim = req.slim.unwrap_or(false);
    let rows = repo::task::list(
        &state.db,
        &req.workspace_id,
        req.state.as_deref(),
        !slim && req.include_plan.unwrap_or(true),
    )
    .await?;
    let events = repo::task::board_events_for_workspace(&state.db, &req.workspace_id).await?;
    let extras = derive_board_extras(&events);

    if slim {
        let include_closed = req.all.unwrap_or(false) || req.state.is_some();
        return Ok(Value::Array(
            rows.iter()
                .filter(|row| {
                    include_closed || !matches!(row.state.as_str(), "merged" | "abandoned")
                })
                .map(|row| slim_task_list_item_to_json(row, &extras))
                .collect(),
        ));
    }

    Ok(Value::Array(
        rows.iter()
            .map(|row| task_list_item_to_json(row, &extras))
            .collect(),
    ))
}

// ── task.brief ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BriefReq {
    workspace_id: String,
    slug: String,
    limit: Option<usize>,
}

/// Fetch one task's low-context resume packet: compact task metadata, the
/// bounded plan excerpt, open challenges, latest gates, recent events, and
/// memory hits that point back to the existing workspace-memory record.
pub async fn brief(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: BriefReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;

    let row = repo::task::get(&state.db, &req.workspace_id, &req.slug)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("task '{}' not found", req.slug)))?;
    let limit = req
        .limit
        .unwrap_or(DEFAULT_BRIEF_LIMIT)
        .clamp(1, MAX_BRIEF_LIMIT);
    let (plan_excerpt, plan_truncated) = excerpt_lines(&row.plan, limit);

    let events = repo::task::board_events_for_workspace(&state.db, &req.workspace_id).await?;
    let board = derive_brief_board_sections(&events, &row.id);
    let open_challenges = board
        .open_challenges
        .into_iter()
        .take(limit)
        .collect::<Vec<_>>();
    let latest_gates = board
        .latest_gates
        .into_iter()
        .take(limit)
        .collect::<Vec<_>>();
    let last_events = repo::task::events_for(&state.db, &row.id, limit as i64).await?;

    let memory_limit = limit.min(5);
    let memory_query = brief_memory_query(&row);
    let (memory_hits, memory_error) = if state.memory_embedder.is_ready() {
        let result = super::memory::search(
            state,
            json!({
                "workspaceId": req.workspace_id,
                "query": memory_query,
                "limit": memory_limit,
            }),
        )
        .await?;
        (
            result
                .get("hits")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            None,
        )
    } else {
        (Vec::new(), Some("memory embedder not ready".to_string()))
    };

    Ok(json!({
        "task": task_brief_to_json(&row, plan_excerpt, plan_truncated),
        "limit": limit,
        "openChallenges": open_challenges,
        "latestGates": latest_gates,
        "lastEvents": last_events.iter().map(task_event_to_json).collect::<Vec<_>>(),
        "memoryHits": memory_hits,
        "memoryError": memory_error,
    }))
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
    let req: GetReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
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
    let req: ClaimReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;
    enforce_scope(state, &req.workspace_id, &req.actor_id, "actor").await?;

    let row = repo::task::claim(&state.db, &req.workspace_id, &req.slug, &req.actor_id).await?;
    emit_changed(state, &row);
    notify_watchers(
        state,
        &row,
        &req.actor_id,
        "state",
        "-> claimed",
        &json!({ "state": "claimed" }),
    )
    .await;
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
    let req: SetStateReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
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
    if let Some(actor) = &req.actor_id {
        let summary = format!("-> {}", req.state);
        notify_watchers(
            state,
            &row,
            actor,
            "state",
            &summary,
            &json!({ "state": req.state }),
        )
        .await;

        // Position routing (spec §3.4): review-ready routes to the integrator =
        // the task owner — notify the owner even if not on the watch list
        // (supplements + dedupes against the watcher fan-out above). GATED on the
        // position system being ENGAGED for the owner (owner has a supervisor
        // link) so the all-NULL case stays byte-for-byte today even for a
        // non-watching owner — the non-negotiable invariant outranks the literal
        // "even if not watching" (spec amended @ 8a28641 per the escalation
        // ruling). All-NULL / no owner → watchers only, exactly as today.
        if req.state == "review" {
            if let Some(owner) = &row.owner_agent_id {
                if let Ok(Some(_)) = repo::workspace_agent::supervisor_of(&state.db, owner).await {
                    notify_expected_ruler(state, &row, actor, "state", &summary, owner).await;
                }
            }
        }
    }
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
    let req: NoteReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
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
    on_task_mutated(
        state,
        &req.workspace_id,
        &req.slug,
        req.actor_id.as_deref(),
        "note",
        &req.text,
        &json!({ "text": req.text }),
    )
    .await?;
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
    /// Client-side path of the FULL gate log (`conclave-cli` writes it under
    /// its cli-output dir and ships only the 2000-byte `tail` over the wire —
    /// task cli-output-economy). Optional so pre-logPath CLI builds keep
    /// recording gates.
    log_path: Option<String>,
}

/// Append a `gate` event. The command itself already ran client-side (see
/// `bin/conclave-cli.rs` — gates NEVER run engine-side, ADR 0008 risk ledger);
/// this just records the evidence. A non-zero `exit` is recorded exactly like
/// a zero one — a red gate is evidence, not a request failure.
pub async fn gate(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: GateReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;
    if let Some(actor) = &req.actor_id {
        enforce_scope(state, &req.workspace_id, actor, "actor").await?;
    }

    let mut gate_payload = json!({
        "cmd": req.cmd,
        "exit": req.exit,
        "sha": req.sha,
        "tail": req.tail,
        "cwd": req.cwd,
    });
    if let Some(log_path) = &req.log_path {
        gate_payload["logPath"] = json!(log_path);
    }
    let payload_json = gate_payload.to_string();
    let event = repo::task::add_gate(
        &state.db,
        &req.workspace_id,
        &req.slug,
        req.actor_id.as_deref(),
        &payload_json,
    )
    .await?;
    on_task_mutated(
        state,
        &req.workspace_id,
        &req.slug,
        req.actor_id.as_deref(),
        "gate",
        &format!("{} exit={}", req.cmd, req.exit),
        &json!({ "exit": req.exit }),
    )
    .await?;
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
    let req: ChallengeReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
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
    on_task_mutated(
        state,
        &req.workspace_id,
        &req.slug,
        req.actor_id.as_deref(),
        "challenge",
        &req.claim,
        // Challenges always wake (deadlines ride them); no wake-signal needed.
        &Value::Null,
    )
    .await?;

    // Position routing (spec §3.1 + §4): ALSO notify the expected ruler even if
    // not watching — the owner (or the LCA across chains), or the first
    // supervisor up the challenger's chain when there's no owner. Supplements
    // the watcher fan-out above; deduped so a watching ruler gets one line.
    if let Some(challenger) = req.actor_id.as_deref() {
        if let Ok(Some(task)) = repo::task::get(&state.db, &req.workspace_id, &req.slug).await {
            if let Some(ruler) = challenge_expected_ruler(state, &task, challenger).await {
                notify_expected_ruler(state, &task, challenger, "challenge", &req.claim, &ruler)
                    .await;
            }
        }
    }
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
    let req: RuleReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
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
    on_task_mutated(
        state,
        &req.workspace_id,
        &req.slug,
        Some(&req.actor_id),
        "ruling",
        &req.text,
        // A ruling wakes the challenger (decision 1, amended @ 1490b6a): it
        // answers a challenge they may be waiting on. Every ruling wakes, so no
        // payload wake-signal is needed. (A deadline auto-ruling is a separate
        // notify in task_timer, unaffected here.)
        &Value::Null,
    )
    .await?;
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
    let req: CloseReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;
    if let Some(actor) = &req.actor_id {
        enforce_scope(state, &req.workspace_id, actor, "actor").await?;
    }

    let row = repo::task::close(
        &state.db,
        &req.workspace_id,
        &req.slug,
        req.actor_id.as_deref(),
    )
    .await?;
    emit_changed(state, &row);
    if let Some(actor) = &req.actor_id {
        notify_watchers(
            state,
            &row,
            actor,
            "state",
            "-> merged",
            &json!({ "state": "merged" }),
        )
        .await;
    }
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
    let req: WatchReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;
    enforce_scope(state, &req.workspace_id, &req.actor_id, "actor").await?;

    repo::task::watch(&state.db, &req.workspace_id, &req.slug, &req.actor_id).await?;
    on_task_mutated(
        state,
        &req.workspace_id,
        &req.slug,
        Some(&req.actor_id),
        "watch",
        "started watching",
        &Value::Null,
    )
    .await?;
    Ok(json!({ "watching": true }))
}

/// Unsubscribe `actorId` from a task. Idempotent.
pub async fn unwatch(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: WatchReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    require_workspace(state, &req.workspace_id).await?;
    enforce_scope(state, &req.workspace_id, &req.actor_id, "actor").await?;

    repo::task::unwatch(&state.db, &req.workspace_id, &req.slug, &req.actor_id).await?;
    on_task_mutated(
        state,
        &req.workspace_id,
        &req.slug,
        Some(&req.actor_id),
        "unwatch",
        "stopped watching",
        &Value::Null,
    )
    .await?;
    Ok(json!({ "watching": false }))
}

/// Re-fetch the task, emit `task:changed`, and (Lane B) notify its watchers —
/// shared by the event-only verbs (`note`/`gate`/`challenge`/`rule`/
/// `watch`/`unwatch`) which don't already hold the row the way
/// `claim`/`setState`/`close` do. `kind`/`summary` build the notify line (see
/// [`notify_watchers`]); `actor_id` absent means no watcher notification (no
/// real agent to attribute it to — `task.create`'s optional owner is the one
/// caller that can reach here without an actor, though it has no watchers
/// yet in practice since watching happens after creation).
async fn on_task_mutated(
    state: &AppState,
    workspace_id: &str,
    slug: &str,
    actor_id: Option<&str>,
    kind: &str,
    summary: &str,
    payload: &Value,
) -> Result<(), AppError> {
    if let Some(row) = repo::task::get(&state.db, workspace_id, slug).await? {
        emit_changed(state, &row);
        if let Some(actor_id) = actor_id {
            notify_watchers(state, &row, actor_id, kind, summary, payload).await;
        }
    }
    Ok(())
}

/// Cap on a notify line's `summary` segment — a watcher notification is a
/// one-line nudge into another agent's live PTY, not a place to paste a
/// multi-paragraph note or challenge claim.
const NOTIFY_SUMMARY_MAX_CHARS: usize = 140;

fn truncate_for_notify(text: &str, max_chars: usize) -> std::borrow::Cow<'_, str> {
    if text.chars().count() <= max_chars {
        std::borrow::Cow::Borrowed(text)
    } else {
        let head: String = text.chars().take(max_chars).collect();
        std::borrow::Cow::Owned(format!("{head}…"))
    }
}

/// Best-effort display name for a `workspace_agent` instance — falls back to
/// the raw id when the agent/definition can't be resolved (a notify-line
/// cosmetic must never block on a lookup miss).
async fn agent_display_name(state: &AppState, instance_id: &str) -> String {
    let Ok(Some(agent)) = repo::workspace_agent::get(&state.db, instance_id).await else {
        return instance_id.to_string();
    };
    match repo::agent_definition::get(&state.db, &agent.agent_def_id).await {
        Ok(Some(def)) => def.name,
        _ => instance_id.to_string(),
    }
}

/// The single wake list (plan `watch-filter`, decision 1): the ONE predicate
/// deciding whether a task event is decision-demanding enough to inject into
/// every watcher's live session, or merely records on the ledger for
/// pull-based reading. Every event emitter funnels through [`notify_watchers`],
/// which consults this once — the wake list lives here and nowhere else
/// (decision 2: no duplicated wake lists).
///
/// Wakes on: every `challenge` and its `ruling` (the dispute pair — a ruling
/// answers a challenger who may be waiting on a stated default; leaving them
/// unwoken until the stall timer would defeat the challenge protocol's latency
/// guarantee); a `state` transition to `review`/`abandoned`/`merged`; a `gate`
/// whose process `exit != 0`; and a `note` whose text starts (exact prefix,
/// case-sensitive, position 0) with `READY`, `BLOCKED`, or `ESCALATION`.
/// Everything else — `claimed`/`in_progress` states, passing gates, unmarked
/// notes, `watch`/`unwatch` — is ledger-only; the stall engine
/// ([`crate::engine::runtime::task_timer`]) is the safety net for an
/// important-but-unmarked note left on a quiet claim.
///
/// `payload` carries only the per-kind wake signal (`state`/`exit`/`text`),
/// deliberately separate from the display `summary` so the decision never
/// depends on how a line is rendered. A `gate` with no readable `exit` fails
/// safe to waking — a missed decision point is worse than one extra nudge.
fn wakes_watchers(kind: &str, payload: &Value) -> bool {
    match kind {
        "challenge" | "ruling" => true,
        "state" => matches!(
            payload.get("state").and_then(Value::as_str),
            Some("review" | "abandoned" | "merged")
        ),
        "gate" => payload
            .get("exit")
            .and_then(Value::as_i64)
            .is_none_or(|exit| exit != 0),
        "note" => payload
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(note_wakes_watchers),
        _ => false,
    }
}

/// A note wakes watchers only when its text opens with a decision marker —
/// exact prefix, case-sensitive, at position 0 (decision 1). `"ready …"`
/// (lowercase) and `" READY …"` (leading space) do NOT match.
fn note_wakes_watchers(text: &str) -> bool {
    ["READY", "BLOCKED", "ESCALATION"]
        .iter()
        .any(|marker| text.starts_with(marker))
}

/// Notify every OTHER watcher of a task mutation (ADR 0008 Lane B), one line
/// per watcher: `[task <slug>] <actor-name>: <kind> — <summary>`. Delivered
/// via `commands::message::inject` — the SAME delivery path `tell` uses (risk
/// ledger: "reuse whatever queueing `tell` already does... do not invent a
/// second injection path"). Best-effort: a delivery failure (unknown/offline
/// watcher) must never fail the mutating command that triggered it.
///
/// Injects only when [`wakes_watchers`] passes for `(kind, payload)`; a
/// filtered-out event still recorded on the ledger, it simply wakes no one.
async fn notify_watchers(
    state: &AppState,
    task: &TaskRow,
    actor_id: &str,
    kind: &str,
    summary: &str,
    payload: &Value,
) {
    if !wakes_watchers(kind, payload) {
        return;
    }
    let Ok(watcher_ids) = repo::task::watchers(&state.db, &task.id).await else {
        return;
    };
    let watcher_ids: Vec<String> = watcher_ids.into_iter().filter(|w| w != actor_id).collect();
    if watcher_ids.is_empty() {
        return;
    }

    let actor_name = agent_display_name(state, actor_id).await;
    let summary = truncate_for_notify(summary, NOTIFY_SUMMARY_MAX_CHARS);
    let text = format!("[task {}] {actor_name}: {kind} — {summary}", task.slug);

    for watcher_id in watcher_ids {
        let _ = super::message::inject(
            state,
            json!({ "fromInstanceId": actor_id, "toInstanceId": watcher_id, "text": text }),
        )
        .await;
    }
}

/// Notify the expected ruler of an event even if they don't watch the task
/// (spec position-system §3.1 challenge / §3.4 review-ready) — SUPPLEMENTS
/// [`notify_watchers`], never replaces it. Deduped against the watch list so a
/// watching ruler gets exactly ONE line. Attributed to the real `actor_id` who
/// acted (same line format as a watcher notification), so — unlike the timer
/// paths — it carries NO `AUTO` marker: the actor genuinely filed the challenge
/// / moved the state. No-op when `ruler_id` is absent, is the actor, or already
/// watches (they were covered by `notify_watchers`).
async fn notify_expected_ruler(
    state: &AppState,
    task: &TaskRow,
    actor_id: &str,
    kind: &str,
    summary: &str,
    ruler_id: &str,
) {
    if ruler_id == actor_id {
        return;
    }
    let Ok(watcher_ids) = repo::task::watchers(&state.db, &task.id).await else {
        return;
    };
    if watcher_ids.iter().any(|w| w == ruler_id) {
        return; // already notified as a watcher — dedupe to one line
    }

    let actor_name = agent_display_name(state, actor_id).await;
    let summary = truncate_for_notify(summary, NOTIFY_SUMMARY_MAX_CHARS);
    let text = format!("[task {}] {actor_name}: {kind} — {summary}", task.slug);
    let _ = super::message::inject(
        state,
        json!({ "fromInstanceId": actor_id, "toInstanceId": ruler_id, "text": text }),
    )
    .await;
}

/// Resolve the expected ruler of a challenge (spec §3.1 + §4). With an owner set
/// this is `lowest_common_supervisor(challenger, owner)`: it returns the OWNER
/// when the owner sits in the challenger's chain (§3.1 same-chain) and the
/// nearest common ancestor when the two sit in different chains (§4 cross-chain);
/// `None` (chains meet only at the human) stays advisory to watchers only — no
/// human ping is fabricated. With no owner, the first supervisor up the
/// challenger's own chain. All chain reads go through the P1 primitives.
async fn challenge_expected_ruler(
    state: &AppState,
    task: &TaskRow,
    challenger_id: &str,
) -> Option<String> {
    match &task.owner_agent_id {
        Some(owner) => {
            repo::workspace_agent::lowest_common_supervisor(&state.db, challenger_id, owner)
                .await
                .ok()
                .flatten()
        }
        None => repo::workspace_agent::supervisor_of(&state.db, challenger_id)
            .await
            .ok()
            .flatten(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::repo::{
        agent_definition::{self, AgentDefinitionInput},
        workspace, workspace_agent,
    };
    use crate::engine::runtime::embedder::{EmbedError, Embedder};
    use std::sync::Arc;

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

    struct NotReadyEmbedder;

    impl Embedder for NotReadyEmbedder {
        fn model_id(&self) -> &'static str {
            "not-ready"
        }

        fn dimension(&self) -> usize {
            384
        }

        fn is_ready(&self) -> bool {
            false
        }

        fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
            Err(EmbedError::ModelUnavailable("not ready".into()))
        }
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
        assert!(
            created.get("ownerAgentId").is_none(),
            "must omit when absent"
        );
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
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "dup", "title": "A" }),
        )
        .await
        .expect("first create failed");

        let err = create(
            &state,
            json!({ "workspaceId": ws, "slug": "dup", "title": "B" }),
        )
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
    async fn list_defaults_to_full_and_can_blank_plan_when_slim() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        create(
            &state,
            json!({
                "workspaceId": ws,
                "slug": "t1",
                "title": "T1",
                "plan": "line one\nline two"
            }),
        )
        .await
        .expect("create failed");

        let full = list(&state, json!({ "workspaceId": ws }))
            .await
            .expect("full list failed");
        let full_row = full.as_array().expect("array")[0].clone();
        assert_eq!(full_row["plan"], json!("line one\nline two"));

        let slim = list(&state, json!({ "workspaceId": ws, "includePlan": false }))
            .await
            .expect("slim list failed");
        let slim_row = slim.as_array().expect("array")[0].clone();
        assert_eq!(slim_row["plan"], json!(""));
    }

    #[tokio::test]
    async fn list_includes_event_count_and_filters_by_state() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
        .await
        .expect("create t1");
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t2", "title": "T2" }),
        )
        .await
        .expect("create t2");
        note(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": actor, "text": "hi" }),
        )
        .await
        .expect("note failed");

        let listed = list(&state, json!({ "workspaceId": ws }))
            .await
            .expect("list failed");
        let arr = listed.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        let t1 = arr.iter().find(|t| t["slug"] == "t1").expect("t1 present");
        assert_eq!(t1["eventCount"], json!(1));
        let t2 = arr.iter().find(|t| t["slug"] == "t2").expect("t2 present");
        assert_eq!(t2["eventCount"], json!(0));

        claim(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": actor }),
        )
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
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
        .await
        .expect("create failed");

        let listed = list(&state, json!({ "workspaceId": ws }))
            .await
            .expect("list failed");
        let t1 = &listed.as_array().unwrap()[0];
        assert_eq!(
            t1["lastGates"],
            json!([]),
            "no gates -> empty array, always present"
        );
        assert_eq!(
            t1["challenges"],
            json!([]),
            "no challenges -> empty array, always present"
        );
    }

    #[tokio::test]
    async fn list_last_gates_collapses_repeats_of_the_same_cmd_to_the_newest() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
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

        let listed = list(&state, json!({ "workspaceId": ws }))
            .await
            .expect("list failed");
        let t1 = &listed.as_array().unwrap()[0];
        let last_gates = t1["lastGates"].as_array().expect("lastGates present");
        assert_eq!(
            last_gates.len(),
            1,
            "same cmd re-run must collapse to one entry"
        );
        let entry = &last_gates[0];
        assert_eq!(entry["cmd"], json!("cargo test"));
        assert_eq!(
            entry["exit"],
            json!(0),
            "must be the NEWEST run of this cmd"
        );
        assert_eq!(entry["sha"], json!("sha2"));
        assert!(entry.get("createdAt").is_some());
        let mut keys: Vec<&str> = entry
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["cmd", "createdAt", "exit", "sha"],
            "no extra fields beyond the frozen shape"
        );
    }

    #[tokio::test]
    async fn list_last_gates_carries_one_entry_per_distinct_cmd_newest_first() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
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

        let listed = list(&state, json!({ "workspaceId": ws }))
            .await
            .expect("list failed");
        let t1 = &listed.as_array().unwrap()[0];
        let last_gates = t1["lastGates"].as_array().expect("lastGates present");
        assert_eq!(
            last_gates.len(),
            2,
            "distinct cmds must BOTH appear (test green + clippy red)"
        );
        assert_eq!(
            last_gates[0]["cmd"],
            json!("cargo clippy"),
            "most-recent-first"
        );
        assert_eq!(last_gates[0]["exit"], json!(101));
        assert_eq!(last_gates[1]["cmd"], json!("cargo test"));
        assert_eq!(last_gates[1]["exit"], json!(0));
    }

    #[tokio::test]
    async fn list_last_gates_caps_at_six_distinct_cmds() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
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

        let listed = list(&state, json!({ "workspaceId": ws }))
            .await
            .expect("list failed");
        let t1 = &listed.as_array().unwrap()[0];
        let last_gates = t1["lastGates"].as_array().expect("lastGates present");
        assert_eq!(last_gates.len(), 6, "capped at 6 distinct cmds");
        // Most-recent-first: the 6 KEPT must be cmd-7 down to cmd-2 (the two
        // oldest, cmd-0 and cmd-1, are evicted).
        let kept: Vec<&str> = last_gates
            .iter()
            .map(|g| g["cmd"].as_str().unwrap())
            .collect();
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
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
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

        let listed = list(&state, json!({ "workspaceId": ws }))
            .await
            .expect("list failed");
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

        let listed = list(&state, json!({ "workspaceId": ws }))
            .await
            .expect("list failed");
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
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
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

        let listed = list(&state, json!({ "workspaceId": ws }))
            .await
            .expect("list failed");
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
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
        .await
        .expect("create t1");
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t2", "title": "T2" }),
        )
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

        let listed = list(&state, json!({ "workspaceId": ws }))
            .await
            .expect("list failed");
        let arr = listed.as_array().unwrap();
        let t1 = arr.iter().find(|t| t["slug"] == "t1").unwrap();
        let t2 = arr.iter().find(|t| t["slug"] == "t2").unwrap();
        assert_eq!(
            t1["lastGates"].as_array().unwrap().len(),
            1,
            "t1 has a gate"
        );
        assert_eq!(
            t2["lastGates"],
            json!([]),
            "t2's list row must not see t1's gate"
        );
    }

    /// Walk `slug` from planned to `target` through the single-step chain
    /// claimed → in_progress → review → merged (abandoned branches off
    /// in_progress).
    async fn walk_to_state(state: &AppState, ws: &str, slug: &str, actor: &str, target: &str) {
        claim(
            state,
            json!({ "workspaceId": ws, "slug": slug, "actorId": actor }),
        )
        .await
        .expect("claim");
        let chain: &[&str] = match target {
            "merged" => &["in_progress", "review", "merged"],
            "abandoned" => &["in_progress", "abandoned"],
            other => panic!("walk_to_state: unsupported target '{other}'"),
        };
        for next in chain {
            set_state(
                state,
                json!({ "workspaceId": ws, "slug": slug, "state": next, "actorId": actor }),
            )
            .await
            .unwrap_or_else(|e| panic!("set_state {next}: {e:?}"));
        }
    }

    /// The UI invariant behind ruling 3f1ab20e: a payload that does NOT send
    /// `slim` gets today's exact full shape — plan, boundary, eventCount, and
    /// the derived `lastGates`/`challenges` all present, closed tasks included.
    #[tokio::test]
    async fn list_without_slim_keeps_full_shape_and_closed_tasks() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1", "plan": "p1" }),
        )
        .await
        .expect("create t1");
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t2", "title": "T2" }),
        )
        .await
        .expect("create t2");
        walk_to_state(&state, &ws, "t2", &actor, "merged").await;

        let listed = list(&state, json!({ "workspaceId": ws }))
            .await
            .expect("list failed");
        let arr = listed.as_array().expect("array");
        assert_eq!(arr.len(), 2, "no-slim list keeps merged tasks");
        let t1 = arr.iter().find(|t| t["slug"] == "t1").expect("t1 present");
        for key in [
            "id",
            "workspaceId",
            "slug",
            "title",
            "state",
            "fileBoundary",
            "plan",
            "createdAt",
            "updatedAt",
            "eventCount",
            "lastGates",
            "challenges",
        ] {
            assert!(t1.get(key).is_some(), "full row must carry '{key}'");
        }
        assert_eq!(t1["plan"], json!("p1"));
    }

    #[tokio::test]
    async fn list_slim_rows_carry_only_slim_keys_and_hide_closed_tasks() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        for slug in ["t1", "t2", "t3"] {
            create(
                &state,
                json!({ "workspaceId": ws, "slug": slug, "title": slug, "plan": "plan text" }),
            )
            .await
            .expect("create");
        }
        walk_to_state(&state, &ws, "t2", &actor, "merged").await;
        walk_to_state(&state, &ws, "t3", &actor, "abandoned").await;
        challenge(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": actor,
                "claim": "c", "evidence": "e", "proposal": "p", "default": "d"
            }),
        )
        .await
        .expect("challenge");

        let listed = list(&state, json!({ "workspaceId": ws, "slim": true }))
            .await
            .expect("slim list failed");
        let arr = listed.as_array().expect("array");
        assert_eq!(arr.len(), 1, "merged/abandoned hidden by default");
        let row = &arr[0];
        assert_eq!(row["slug"], json!("t1"));
        assert_eq!(row["openChallenges"], json!(1));
        let mut keys: Vec<&str> = row
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "implementerAgentId",
                "openChallenges",
                "slug",
                "state",
                "title",
                "updatedAt"
            ],
            "slim row carries ONLY the slim keys"
        );
    }

    #[tokio::test]
    async fn list_slim_all_and_state_filter_include_closed_tasks() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
        .await
        .expect("create t1");
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t2", "title": "T2" }),
        )
        .await
        .expect("create t2");
        walk_to_state(&state, &ws, "t2", &actor, "merged").await;

        let all = list(
            &state,
            json!({ "workspaceId": ws, "slim": true, "all": true }),
        )
        .await
        .expect("slim all failed");
        let all_arr = all.as_array().expect("array");
        assert_eq!(all_arr.len(), 2, "--all includes merged");
        let merged_row = all_arr
            .iter()
            .find(|t| t["slug"] == "t2")
            .expect("t2 present");
        assert!(
            merged_row.get("plan").is_none(),
            "--all stays in the slim shape"
        );

        let filtered = list(
            &state,
            json!({ "workspaceId": ws, "slim": true, "state": "merged" }),
        )
        .await
        .expect("slim state filter failed");
        let filtered_arr = filtered.as_array().expect("array");
        assert_eq!(filtered_arr.len(), 1, "explicit state filter wins");
        assert_eq!(filtered_arr[0]["slug"], json!("t2"));
    }

    #[tokio::test]
    async fn gate_records_log_path_when_sent_and_omits_it_when_absent() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
        .await
        .expect("create t1");

        let with_path = gate(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": actor,
                "cmd": "cargo test", "exit": 0, "sha": "sha1", "tail": "ok", "cwd": "/repo",
                "logPath": "/logs/gate-t1.log"
            }),
        )
        .await
        .expect("gate with logPath failed");
        assert_eq!(with_path["payload"]["logPath"], json!("/logs/gate-t1.log"));

        let without_path = gate(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": actor,
                "cmd": "cargo build", "exit": 0, "sha": "sha1", "tail": "ok", "cwd": "/repo"
            }),
        )
        .await
        .expect("gate without logPath failed");
        assert!(
            without_path["payload"].get("logPath").is_none(),
            "pre-logPath CLI builds must record byte-identical payloads"
        );
    }

    #[tokio::test]
    async fn brief_returns_bounded_resume_packet_with_memory_hits() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        create(
            &state,
            json!({
                "workspaceId": ws,
                "slug": "t1",
                "title": "T1",
                "designCanon": "canon-x",
                "fileBoundary": ["src/a.rs", "src/b.rs"],
                "plan": "line one\nline two\nline three\nline four\nline five"
            }),
        )
        .await
        .expect("create failed");
        note(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": actor, "text": "note one" }),
        )
        .await
        .expect("note failed");
        gate(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": actor,
                "cmd": "cargo test", "exit": 0, "sha": "sha1", "tail": "ok", "cwd": "/repo"
            }),
        )
        .await
        .expect("gate failed");
        challenge(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": actor,
                "claim": "still missing", "evidence": "log", "proposal": "fix it",
                "default": "escalate"
            }),
        )
        .await
        .expect("challenge failed");
        crate::engine::commands::memory::remember(
            &state,
            json!({
                "workspaceId": ws,
                "text": "t1 context-slim-task-reads-recovery resume packet"
            }),
        )
        .await
        .expect("remember failed");

        let brief = brief(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "limit": 3 }),
        )
        .await
        .expect("brief failed");
        let task = brief["task"].as_object().expect("task object");
        assert_eq!(task["slug"], json!("t1"));
        assert_eq!(task["designCanon"], json!("canon-x"));
        assert_eq!(task["fileBoundary"], json!(["src/a.rs", "src/b.rs"]));
        assert_eq!(task["planExcerpt"].as_str().unwrap().lines().count(), 3);
        assert_eq!(task["planTruncated"], json!(true));

        let open_challenges = brief["openChallenges"]
            .as_array()
            .expect("openChallenges array");
        assert_eq!(open_challenges.len(), 1);
        assert_eq!(open_challenges[0]["status"], json!("open"));
        assert_eq!(open_challenges[0]["claim"], json!("still missing"));

        let latest_gates = brief["latestGates"].as_array().expect("latestGates array");
        assert_eq!(latest_gates.len(), 1);
        assert_eq!(latest_gates[0]["cmd"], json!("cargo test"));

        let last_events = brief["lastEvents"].as_array().expect("lastEvents array");
        assert_eq!(last_events.len(), 3);
        assert_eq!(last_events[0]["kind"], json!("challenge"));
        assert_eq!(last_events[1]["kind"], json!("gate"));
        assert_eq!(last_events[2]["kind"], json!("note"));

        let memory_hits = brief["memoryHits"].as_array().expect("memoryHits array");
        assert_eq!(memory_hits.len(), 1);
        assert!(brief["memoryError"].is_null());
    }

    #[tokio::test]
    async fn brief_reports_memory_error_when_embedder_is_not_ready() {
        let mut state = AppState::for_tests().await;
        state.memory_embedder = Arc::new(NotReadyEmbedder);
        let ws = fixture_workspace(&state).await;
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
        .await
        .expect("create failed");

        let brief = brief(&state, json!({ "workspaceId": ws, "slug": "t1" }))
            .await
            .expect("brief failed");
        assert_eq!(brief["memoryHits"], json!([]));
        assert_eq!(brief["memoryError"], json!("memory embedder not ready"));
    }

    // ── task.get ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn get_returns_task_plus_events_newest_first() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
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
        assert_eq!(
            events[0]["payload"]["text"],
            json!("second"),
            "newest first"
        );
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
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
        .await
        .expect("create failed");

        let claimed = claim(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": actor }),
        )
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
        create(
            &state,
            json!({ "workspaceId": ws1, "slug": "t1", "title": "T1" }),
        )
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
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
        .await
        .expect("create failed");
        claim(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": actor }),
        )
        .await
        .expect("first claim failed");

        let err = claim(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": actor }),
        )
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
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
        .await
        .expect("create failed");
        claim(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": actor }),
        )
        .await
        .expect("claim failed");
        close(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": actor }),
        )
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
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
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
        let err = note(
            &state,
            json!({ "workspaceId": ws, "slug": "nope", "text": "x" }),
        )
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
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
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
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
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
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
        .await
        .expect("create failed");

        let err = close(&state, json!({ "workspaceId": ws, "slug": "t1" }))
            .await
            .expect_err("close from planned must fail");
        assert!(matches!(err, AppError::Invalid(_)));
    }

    // ── task.watch / task.unwatch ─────────────────────────────────────────

    /// Count messages actually RECEIVED by `instance` (message.list returns
    /// inbox+outbox; filter by `toInstanceId`).
    async fn received(state: &AppState, instance: &str) -> Vec<Value> {
        super::super::message::list(state, json!({ "instanceId": instance }))
            .await
            .expect("list failed")
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["toInstanceId"] == json!(instance))
            .cloned()
            .collect()
    }

    /// §3.1 same-chain: a challenger reporting to the owner routes the challenge
    /// to the owner even when the owner does NOT watch — attributed to the
    /// challenger (a live actor), so NO `AUTO` marker.
    #[tokio::test]
    async fn challenge_routes_to_owner_up_the_chain() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let owner = fixture_instance(&state, &ws, "Owner").await;
        let challenger = fixture_instance(&state, &ws, "Challenger").await;
        workspace_agent::set_position(&state.db, &challenger, None, Some(&owner))
            .await
            .expect("challenger reports to owner");
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1", "ownerAgentId": owner }),
        )
        .await
        .expect("create");
        challenge(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": challenger,
                    "claim": "X broken", "evidence": "e", "proposal": "p", "default": "d" }),
        )
        .await
        .expect("challenge");

        let owner_msgs = received(&state, &owner).await;
        assert_eq!(
            owner_msgs.len(),
            1,
            "owner is the expected ruler, notified even unwatching"
        );
        assert!(
            !owner_msgs[0]["text"].as_str().unwrap().contains("AUTO"),
            "live actor send, no AUTO"
        );
    }

    /// §3.1 all-NULL invariant, byte-for-byte: with NO supervisor links the
    /// challenge notification set is EXACTLY today's — the watcher receives the
    /// unchanged `(from=challenger, to=watcher, text)` line, and a non-watching
    /// owner receives NOTHING (the LCA is None). Pins the full stable tuple, not
    /// just a count, so a suppressed or altered watcher line fails here.
    #[tokio::test]
    async fn challenge_all_null_is_byte_for_byte_today() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let owner = fixture_instance(&state, &ws, "Owner").await;
        let challenger = fixture_instance(&state, &ws, "Challenger").await;
        let watcher = fixture_instance(&state, &ws, "Watcher").await;
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1", "ownerAgentId": owner }),
        )
        .await
        .expect("create");
        watch(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": watcher }),
        )
        .await
        .expect("watch");
        challenge(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": challenger,
                    "claim": "X broken", "evidence": "e", "proposal": "p", "default": "d" }),
        )
        .await
        .expect("challenge");

        // The watcher gets exactly today's line (fromInstanceId/toInstanceId/text).
        let w = received(&state, &watcher).await;
        assert_eq!(w.len(), 1, "watcher gets exactly one line");
        assert_eq!(w[0]["fromInstanceId"], json!(challenger));
        assert_eq!(w[0]["toInstanceId"], json!(watcher));
        assert_eq!(
            w[0]["text"],
            json!("[task t1] Challenger: challenge — X broken")
        );
        // The non-watching owner is not the ruler at all-NULL → nothing.
        assert_eq!(
            received(&state, &owner).await.len(),
            0,
            "no owner ping at all-NULL"
        );
    }

    /// §4 cross-chain: challenger and owner under different sub-leads → the
    /// challenge routes to their lowest common supervisor (the shared lead).
    #[tokio::test]
    async fn challenge_routes_cross_chain_to_the_lca() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let lead = fixture_instance(&state, &ws, "Lead").await;
        let sub1 = fixture_instance(&state, &ws, "Sub1").await;
        let sub2 = fixture_instance(&state, &ws, "Sub2").await;
        let challenger = fixture_instance(&state, &ws, "Challenger").await;
        let owner = fixture_instance(&state, &ws, "Owner").await;
        for (a, s) in [
            (&sub1, &lead),
            (&sub2, &lead),
            (&challenger, &sub1),
            (&owner, &sub2),
        ] {
            workspace_agent::set_position(&state.db, a, None, Some(s))
                .await
                .expect("link");
        }
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1", "ownerAgentId": owner }),
        )
        .await
        .expect("create");
        challenge(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": challenger,
                    "claim": "shared iface", "evidence": "e", "proposal": "p", "default": "d" }),
        )
        .await
        .expect("challenge");

        assert_eq!(
            received(&state, &lead).await.len(),
            1,
            "the LCA (lead) is the ruler"
        );
        assert_eq!(
            received(&state, &owner).await.len(),
            0,
            "owner is cross-chain, not the ruler"
        );
    }

    /// Q2 dedupe: a watching owner who is also the expected ruler gets exactly
    /// ONE line, not one as watcher + one as ruler.
    #[tokio::test]
    async fn challenge_dedupes_a_watching_owner_to_one_line() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let owner = fixture_instance(&state, &ws, "Owner").await;
        let challenger = fixture_instance(&state, &ws, "Challenger").await;
        workspace_agent::set_position(&state.db, &challenger, None, Some(&owner))
            .await
            .expect("link");
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1", "ownerAgentId": owner }),
        )
        .await
        .expect("create");
        watch(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": owner }),
        )
        .await
        .expect("watch");
        challenge(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": challenger,
                    "claim": "X", "evidence": "e", "proposal": "p", "default": "d" }),
        )
        .await
        .expect("challenge");
        assert_eq!(
            received(&state, &owner).await.len(),
            1,
            "watching ruler gets exactly one line"
        );
    }

    /// §3.4: review-ready routes to the owner (integrator) even when unwatching,
    /// once the position system is engaged (owner has a supervisor); no AUTO.
    #[tokio::test]
    async fn review_ready_routes_to_owner_when_position_engaged() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let lead = fixture_instance(&state, &ws, "Lead").await;
        let owner = fixture_instance(&state, &ws, "Owner").await;
        let implementer = fixture_instance(&state, &ws, "Implementer").await;
        workspace_agent::set_position(&state.db, &owner, None, Some(&lead))
            .await
            .expect("owner reports to lead");
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1", "ownerAgentId": owner }),
        )
        .await
        .expect("create");
        claim(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": implementer }),
        )
        .await
        .expect("claim");
        set_state(&state, json!({ "workspaceId": ws, "slug": "t1", "state": "in_progress", "actorId": implementer }))
            .await
            .expect("in_progress");
        set_state(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "state": "review", "actorId": implementer }),
        )
        .await
        .expect("review");

        let owner_msgs = received(&state, &owner).await;
        assert_eq!(owner_msgs.len(), 1, "owner notified as integrator");
        assert!(
            !owner_msgs[0]["text"].as_str().unwrap().contains("AUTO"),
            "live actor send, no AUTO"
        );
    }

    /// §3.4 all-NULL invariant, byte-for-byte: with NO supervisor links the
    /// review-ready notification set is EXACTLY today's — the watcher receives
    /// the unchanged `(from=implementer, to=watcher, text)` line, and a
    /// non-watching owner receives NOTHING. Pins the full stable tuple.
    #[tokio::test]
    async fn review_ready_all_null_is_byte_for_byte_today() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let owner = fixture_instance(&state, &ws, "Owner").await;
        let implementer = fixture_instance(&state, &ws, "Implementer").await;
        let watcher = fixture_instance(&state, &ws, "Watcher").await;
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1", "ownerAgentId": owner }),
        )
        .await
        .expect("create");
        watch(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": watcher }),
        )
        .await
        .expect("watch");
        claim(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": implementer }),
        )
        .await
        .expect("claim");
        set_state(&state, json!({ "workspaceId": ws, "slug": "t1", "state": "in_progress", "actorId": implementer }))
            .await
            .expect("in_progress");
        set_state(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "state": "review", "actorId": implementer }),
        )
        .await
        .expect("review");

        // claimed/in_progress are ledger-only; the watcher's only line is review.
        let w = received(&state, &watcher).await;
        assert_eq!(
            w.len(),
            1,
            "watcher gets exactly one line (the review transition)"
        );
        assert_eq!(w[0]["fromInstanceId"], json!(implementer));
        assert_eq!(w[0]["toInstanceId"], json!(watcher));
        assert_eq!(
            w[0]["text"],
            json!("[task t1] Implementer: state — -> review")
        );
        assert_eq!(
            received(&state, &owner).await.len(),
            0,
            "no owner ping at all-NULL"
        );
    }

    #[tokio::test]
    async fn watch_then_unwatch_round_trip() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Agent").await;
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
        .await
        .expect("create failed");

        let watched = watch(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": actor }),
        )
        .await
        .expect("watch failed");
        assert_eq!(watched["watching"], json!(true));

        let unwatched = unwatch(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": actor }),
        )
        .await
        .expect("unwatch failed");
        assert_eq!(unwatched["watching"], json!(false));
    }

    #[tokio::test]
    async fn watch_unknown_actor_is_not_found() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
        .await
        .expect("create failed");

        let err = watch(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": "nope" }),
        )
        .await
        .expect_err("unknown actor must fail");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    // ── Lane B: watch-notify hook ───────────────────────────────────────────

    #[tokio::test]
    async fn claimed_and_in_progress_states_do_not_wake_watchers() {
        // Plan watch-filter, decision 1 + Test 2: routine progression states
        // record on the ledger but inject NOTHING. Before this lane both of
        // these fired a watcher notify; now neither does.
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Actor").await;
        let watcher = fixture_instance(&state, &ws, "Watcher").await;
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
        .await
        .expect("create failed");
        watch(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": watcher }),
        )
        .await
        .expect("watch failed");

        claim(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": actor }),
        )
        .await
        .expect("claim failed");
        set_state(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "state": "in_progress", "actorId": actor }),
        )
        .await
        .expect("set_state in_progress failed");

        let inbox = super::super::message::list(&state, json!({ "instanceId": watcher }))
            .await
            .expect("list failed");
        assert_eq!(
            inbox.as_array().expect("array").len(),
            0,
            "claimed/in_progress must be ledger-only — no watcher injection"
        );
    }

    #[tokio::test]
    async fn review_state_wakes_watchers_via_the_real_inject_path() {
        // The complement: a `review` transition IS decision-demanding and wakes
        // watchers with the same one-line shape as before (only the filter is
        // new). Watcher is offline here, so delivery is `queued` — still proves
        // it was ATTEMPTED via the real `message::inject` path (risk ledger: no
        // second injection mechanism).
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Actor").await;
        let watcher = fixture_instance(&state, &ws, "Watcher").await;
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
        .await
        .expect("create failed");
        watch(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": watcher }),
        )
        .await
        .expect("watch failed");

        claim(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": actor }),
        )
        .await
        .expect("claim failed");
        set_state(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "state": "in_progress", "actorId": actor }),
        )
        .await
        .expect("in_progress failed");
        set_state(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "state": "review", "actorId": actor }),
        )
        .await
        .expect("review failed");

        let inbox = super::super::message::list(&state, json!({ "instanceId": watcher }))
            .await
            .expect("list failed");
        let arr = inbox.as_array().expect("array");
        assert_eq!(
            arr.len(),
            1,
            "only the review transition wakes; the two earlier states do not"
        );
        let text = arr[0]["text"].as_str().expect("text present");
        assert!(
            text.contains("[task t1]"),
            "line must name the task: {text}"
        );
        assert!(
            text.contains("review"),
            "line must summarize the mutation: {text}"
        );
        assert_eq!(
            arr[0]["fromInstanceId"],
            json!(actor),
            "attributed to the real actor"
        );
        assert_eq!(arr[0]["toInstanceId"], json!(watcher));
    }

    /// The plan's literal acceptance line: "watch a task from a second agent,
    /// mutate from first, line arrives in watcher's PTY." Registers a live
    /// placeholder backend for the watcher (same fixture pattern as
    /// `commands::message`'s own `inject_live_target_delivers` test) so the
    /// notify actually reaches a "live" stdin, not just a queued DB row.
    #[tokio::test]
    async fn waking_notify_arrives_in_a_live_watchers_pty() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Actor").await;
        let watcher = fixture_instance(&state, &ws, "Watcher").await;
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
        .await
        .expect("create failed");
        watch(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": watcher }),
        )
        .await
        .expect("watch failed");

        let watcher_session_id = crate::engine::repo::session::get_by_instance(&state.db, &watcher)
            .await
            .expect("get_by_instance failed")
            .expect("session exists")
            .id;
        assert!(
            state
                .runtime
                .register(
                    &watcher,
                    crate::engine::runtime::LiveHandle::placeholder(&watcher_session_id)
                )
                .is_some(),
            "registering the watcher's live placeholder must succeed"
        );

        // `claim` is now silent (ledger-only); drive a WAKING transition
        // (claimed -> abandoned) so a live watcher actually receives the line.
        claim(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": actor }),
        )
        .await
        .expect("claim failed");
        set_state(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "state": "abandoned", "actorId": actor }),
        )
        .await
        .expect("abandon failed");

        let inbox = super::super::message::list(&state, json!({ "instanceId": watcher }))
            .await
            .expect("list failed");
        let arr = inbox.as_array().unwrap();
        assert_eq!(
            arr.len(),
            1,
            "only the abandoned transition wakes, not the claim"
        );
        assert_eq!(
            arr[0]["status"],
            json!("delivered"),
            "a LIVE watcher's notify must be delivered, not merely queued"
        );
        assert!(arr[0]["text"].as_str().unwrap().contains("[task t1]"));
    }

    #[tokio::test]
    async fn actor_does_not_notify_itself() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Actor").await;
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
        .await
        .expect("create failed");
        // The actor watches their OWN task before claiming it.
        watch(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": actor }),
        )
        .await
        .expect("watch failed");

        // Drive a WAKING self-mutation (claimed -> abandoned): the 0-count then
        // proves self-exclusion, not merely that the event was filtered out.
        claim(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": actor }),
        )
        .await
        .expect("claim failed");
        set_state(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "state": "abandoned", "actorId": actor }),
        )
        .await
        .expect("abandon failed");

        let inbox = super::super::message::list(&state, json!({ "instanceId": actor }))
            .await
            .expect("list failed");
        assert_eq!(
            inbox.as_array().unwrap().len(),
            0,
            "the actor must never receive a notify about their own mutation"
        );
    }

    #[tokio::test]
    async fn no_watchers_means_no_notification_attempt() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Actor").await;
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
        .await
        .expect("create failed");

        // No watch() call at all — a waking transition must not panic or error.
        claim(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": actor }),
        )
        .await
        .expect("claim failed");
        set_state(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "state": "abandoned", "actorId": actor }),
        )
        .await
        .expect("abandon failed");
    }

    #[tokio::test]
    async fn marked_note_notifies_watchers_but_unmarked_note_does_not() {
        // Plan watch-filter, decision 1 + Test 2: a note wakes watchers ONLY
        // when its text opens with READY/BLOCKED/ESCALATION; the injected line
        // has the same shape as before. An unmarked note is ledger-only.
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Actor").await;
        let watcher = fixture_instance(&state, &ws, "Watcher").await;
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
        .await
        .expect("create failed");
        watch(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": watcher }),
        )
        .await
        .expect("watch failed");

        // Unmarked note: no wake.
        note(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": actor, "text": "making progress" }),
        )
        .await
        .expect("unmarked note failed");
        assert_eq!(
            super::super::message::list(&state, json!({ "instanceId": watcher }))
                .await
                .expect("list failed")
                .as_array()
                .unwrap()
                .len(),
            0,
            "an unmarked note must be ledger-only"
        );

        // Marked note: wakes, same one-line shape as before.
        note(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": actor, "text": "READY for review at abc123" }),
        )
        .await
        .expect("marked note failed");

        let inbox = super::super::message::list(&state, json!({ "instanceId": watcher }))
            .await
            .expect("list failed");
        let arr = inbox.as_array().unwrap();
        assert_eq!(arr.len(), 1, "only the READY note wakes");
        let text = arr[0]["text"].as_str().unwrap();
        assert!(text.contains("note"), "kind must be in the line: {text}");
        assert!(
            text.contains("READY for review"),
            "summary must be the note text: {text}"
        );
    }

    async fn inbox_len(state: &AppState, instance: &str) -> usize {
        super::super::message::list(state, json!({ "instanceId": instance }))
            .await
            .expect("list failed")
            .as_array()
            .expect("array")
            .len()
    }

    #[tokio::test]
    async fn failing_gate_and_challenge_wake_but_passing_gate_does_not() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Actor").await;
        let watcher = fixture_instance(&state, &ws, "Watcher").await;
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
        .await
        .expect("create failed");
        watch(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": watcher }),
        )
        .await
        .expect("watch failed");

        let gate_payload = |exit: i64| {
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": actor,
                "cmd": "cargo test", "exit": exit, "sha": "abc", "tail": "…", "cwd": "/tmp",
            })
        };
        gate(&state, gate_payload(0)).await.expect("passing gate");
        assert_eq!(
            inbox_len(&state, &watcher).await,
            0,
            "a passing gate is ledger-only"
        );

        gate(&state, gate_payload(101)).await.expect("failing gate");
        assert_eq!(inbox_len(&state, &watcher).await, 1, "a failing gate wakes");

        challenge(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": actor,
                "claim": "the plan is wrong", "evidence": "line 5", "proposal": "fix it", "default": "proceed",
            }),
        )
        .await
        .expect("challenge");
        assert_eq!(
            inbox_len(&state, &watcher).await,
            2,
            "every challenge wakes"
        );
    }

    #[tokio::test]
    async fn a_ruling_wakes_watchers_through_the_real_path() {
        // Decision 1 (amended @ 1490b6a): a ruling answers a challenger who may
        // be waiting on a stated default, so it must wake watchers too.
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Actor").await;
        let owner = fixture_instance(&state, &ws, "Owner").await;
        let watcher = fixture_instance(&state, &ws, "Watcher").await;
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1", "ownerAgentId": owner }),
        )
        .await
        .expect("create failed");
        watch(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": watcher }),
        )
        .await
        .expect("watch failed");

        let challenge_event = challenge(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": actor,
                "claim": "the plan is wrong", "evidence": "line 5", "proposal": "fix", "default": "proceed",
            }),
        )
        .await
        .expect("challenge");
        assert_eq!(inbox_len(&state, &watcher).await, 1, "the challenge wakes");

        rule(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": owner,
                "challengeEventId": challenge_event["id"], "text": "ruled: proceed on your default",
            }),
        )
        .await
        .expect("rule");
        assert_eq!(
            inbox_len(&state, &watcher).await,
            2,
            "the ruling wakes the watching challenger too"
        );
    }

    #[test]
    fn wakes_watchers_encodes_exactly_the_decision_1_list() {
        // challenge and its ruling — always, regardless of payload.
        assert!(wakes_watchers("challenge", &Value::Null));
        assert!(wakes_watchers("ruling", &Value::Null));

        // state — only review/abandoned/merged.
        for terminal in ["review", "abandoned", "merged"] {
            assert!(
                wakes_watchers("state", &json!({ "state": terminal })),
                "{terminal} must wake"
            );
        }
        for routine in ["claimed", "in_progress", "planned"] {
            assert!(
                !wakes_watchers("state", &json!({ "state": routine })),
                "{routine} must not wake"
            );
        }

        // gate — exit != 0 wakes; exit 0 does not; unreadable exit fails safe.
        assert!(!wakes_watchers("gate", &json!({ "exit": 0 })));
        assert!(wakes_watchers("gate", &json!({ "exit": 1 })));
        assert!(wakes_watchers("gate", &json!({ "exit": -1 })));
        assert!(
            wakes_watchers("gate", &json!({})),
            "an unreadable exit fails safe to waking"
        );

        // note — exact-prefix, case-sensitive markers only.
        for wake in [
            "READY at abc",
            "BLOCKED on x",
            "ESCALATION: y",
            "READY",
            "BLOCKED",
        ] {
            assert!(
                wakes_watchers("note", &json!({ "text": wake })),
                "{wake:?} must wake"
            );
        }
        for quiet in [
            "ready lower",
            " READY leading-space",
            "escalation",
            "just progress",
            "note READY mid",
        ] {
            assert!(
                !wakes_watchers("note", &json!({ "text": quiet })),
                "{quiet:?} must not wake"
            );
        }

        // everything else is ledger-only.
        for ledger_only in ["watch", "unwatch", "created"] {
            assert!(
                !wakes_watchers(ledger_only, &Value::Null),
                "{ledger_only} must not wake"
            );
        }
    }

    #[tokio::test]
    async fn notify_line_truncates_a_long_summary() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Actor").await;
        let watcher = fixture_instance(&state, &ws, "Watcher").await;
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
        .await
        .expect("create failed");
        watch(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": watcher }),
        )
        .await
        .expect("watch failed");

        // Marked so it wakes (a plain note is now ledger-only); still long
        // enough that the notify line must truncate its summary.
        let long_text = format!("READY {}", "x".repeat(500));
        note(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": actor, "text": long_text }),
        )
        .await
        .expect("note failed");

        let inbox = super::super::message::list(&state, json!({ "instanceId": watcher }))
            .await
            .expect("list failed");
        let text = inbox.as_array().unwrap()[0]["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            text.len() < 500,
            "notify line must truncate a long summary, got {} chars",
            text.len()
        );
        assert!(
            text.contains('…'),
            "truncation marker must be present: {text}"
        );
    }

    #[tokio::test]
    async fn unwatching_agent_receives_no_further_notifications() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let actor = fixture_instance(&state, &ws, "Actor").await;
        let watcher = fixture_instance(&state, &ws, "Watcher").await;
        create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }),
        )
        .await
        .expect("create failed");
        watch(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": watcher }),
        )
        .await
        .expect("watch failed");
        unwatch(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": watcher }),
        )
        .await
        .expect("unwatch failed");

        // A WAKING mutation (claimed -> abandoned) after unwatch: the 0-count
        // proves the unsubscribe, not merely that the event was filtered out.
        claim(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": actor }),
        )
        .await
        .expect("claim failed");
        set_state(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "state": "abandoned", "actorId": actor }),
        )
        .await
        .expect("abandon failed");

        let inbox = super::super::message::list(&state, json!({ "instanceId": watcher }))
            .await
            .expect("list failed");
        assert_eq!(
            inbox.as_array().unwrap().len(),
            0,
            "an unwatched agent must not be notified of later mutations"
        );
    }
}
