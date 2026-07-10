//! Snapshot command handlers — the snapshot manager.
//!
//! Snapshots mark a point on a session's rolling context window. A `manual`
//! snapshot is created from the UI / CLI; an `auto` snapshot is created by the
//! output forwarder when the context estimate crosses the auto-compact threshold
//! (≈90%); a `handoff` snapshot carries the agent's own self-written context and
//! is created by the strategic-compact loop (`save` / `compact`).
//!
//! Handlers: `create` (manual marker) · `list` · `read` · `save` (agent writes a
//! handoff) · `last` (read the latest handoff back) · `compact` (the full
//! save→clear→restore loop) · `delete` / `send` (the Memory list's row actions).
//!
//! # honesty seams
//!
//! - `tokens` is a labelled ESTIMATE from streamed output bytes — there is no
//!   real provider token-usage telemetry yet (see `commands::instance`).
//! - `summary` is a deterministic honest string. For `auto`/`manual` markers
//!   `carried_forward` is NULL; a `handoff` carries REAL content (the agent's
//!   self-written handoff text). `diff` is still always NULL — real diffing is
//!   out of scope.

use crate::engine::runtime::Runtime;
use crate::engine::{agentctx, bus, repo, AppError, AppState};
use repo::snapshot::{NewSnapshot, SnapshotRow};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;

// ── Request types ────────────────────────────────────────────────────────────

/// Payload for `snapshot.create` — checkpoint a session's context window.
///
/// The JSON `"type"` key deserialises into `kind` (`type` is a Rust keyword).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateReq {
    session_id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    label: Option<String>,
}

/// Payload for `snapshot.list` — all snapshots for one session.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListReq {
    session_id: String,
}

/// Payload for `snapshot.read` — fetch one snapshot's real detail by id (M4.2).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadReq {
    snapshot_id: String,
}

// ── Pure helpers ──────────────────────────────────────────────────────────────

/// Should the context window be auto-compacted? True once `tokens` reaches
/// `trigger_pct`% of `limit`. Integer-only (no float rounding) so the boundary
/// is exact: `tokens * 100 >= trigger_pct * limit`. A non-positive `limit`
/// disables auto-compaction (avoids a divide-by-zero / always-true trap).
pub fn should_auto_compact(tokens: i64, limit: i64, trigger_pct: i64) -> bool {
    limit > 0 && tokens * 100 >= trigger_pct * limit
}

/// Build the honest, deterministic summary line for a snapshot. Names the
/// estimate explicitly — this is NOT a real LLM summary (deferred).
fn honest_summary(kind: &str, tokens: i64, limit: i64) -> String {
    format!("{kind} snapshot · ~{tokens} / {limit} tokens (estimate)")
}

/// Round a token count to a whole-percent of the limit (for `trigger_pct`).
fn pct_of(tokens: i64, limit: i64) -> Option<i64> {
    if limit <= 0 {
        return None;
    }
    Some(((tokens as f64 / limit as f64) * 100.0).round() as i64)
}

// ── Shared persist + emit ─────────────────────────────────────────────────────

/// Persist a snapshot (chaining `prev_snapshot_id` to the session's latest) and
/// emit `snapshot:created` when an `AppHandle` is available. The SINGLE place
/// that creates+emits a snapshot, shared by the manual `create` handler and the
/// auto-compact path so the emit logic never drifts.
///
/// `app` is `None` in non-Tauri contexts (tests) — the emit is then skipped and
/// only the DB write happens.
async fn persist_and_emit(
    db: &SqlitePool,
    app: Option<&AppHandle>,
    input: NewSnapshot,
) -> Result<SnapshotRow, AppError> {
    // `prev_snapshot_id` is chained ATOMICALLY inside `repo::snapshot::create`
    // (a correlated subquery in the INSERT) — callers leave it unset and manual +
    // auto chain identically, with no read-then-insert race between a manual
    // `snapshot.create` and a concurrent auto-compact for the same session.
    let row = repo::snapshot::create(db, input).await?;

    if let Some(app) = app {
        let _ = bus::snapshot_created(
            app,
            bus::SnapshotCreated {
                session_id: row.session_id.clone(),
                snapshot_id: row.id.clone(),
                kind: row.r#type.clone(),
                tokens: row.tokens,
                trigger_pct: row.trigger_pct,
            },
        );
    }

    Ok(row)
}

/// Create an `auto` snapshot for the given context estimate. Called by the
/// output forwarder (`commands::instance`) when the estimate crosses the
/// auto-compact threshold. Takes `(db, app)` directly — the forwarder holds a
/// `SqlitePool` and an `Option<AppHandle>`, NOT an `&AppState`.
///
/// `tokens` is the current (estimated) token count; `trigger_pct` is derived
/// from `tokens` / `limit`. Reuses [`persist_and_emit`] so manual + auto share
/// one create+emit path.
pub(crate) async fn create_auto(
    db: &SqlitePool,
    app: Option<&AppHandle>,
    session_id: &str,
    tokens: i64,
    limit: i64,
) -> Result<SnapshotRow, AppError> {
    persist_and_emit(
        db,
        app,
        NewSnapshot {
            session_id: session_id.to_owned(),
            kind: "auto".to_owned(),
            label: None,
            summary: Some(honest_summary("auto", tokens, limit)),
            tokens: Some(tokens),
            trigger_pct: pct_of(tokens, limit),
            prev_snapshot_id: None, // resolved inside persist_and_emit
            carried_forward: None,  // markers carry only the estimate
        },
    )
    .await
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// Create a `manual` (or `handoff`) snapshot for a session from its current
/// context estimate. Maps to `snapshot.create` and returns the `Snapshot` row.
pub async fn create(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: CreateReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    // Validate the session exists (FK + an honest NotFound rather than a raw
    // constraint error).
    let session = repo::session::get(&state.db, &req.session_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("session id={} not found", req.session_id)))?;

    let tokens = session.context_tokens;
    let limit = match session.context_limit {
        Some(limit) => limit,
        None => {
            let cli_kind =
                repo::workspace_agent::cli_kind_for(&state.db, &session.workspace_agent_id).await?;
            repo::session::default_context_limit_for(cli_kind.as_deref().unwrap_or(""))
        }
    };
    let trigger_pct = tokens.and_then(|t| pct_of(t, limit));
    let summary = Some(honest_summary(&req.kind, tokens.unwrap_or(0), limit));

    let row = persist_and_emit(
        &state.db,
        state.app(),
        NewSnapshot {
            session_id: req.session_id,
            kind: req.kind,
            label: req.label,
            summary,
            tokens,
            trigger_pct,
            prev_snapshot_id: None, // resolved inside persist_and_emit
            carried_forward: None,  // a manual marker carries only the estimate
        },
    )
    .await?;

    serde_json::to_value(&row).map_err(|e| AppError::Internal(e.to_string()))
}

/// List all snapshots for a session, newest-first. Maps to `snapshot.list`.
pub async fn list(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: ListReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    let rows = repo::snapshot::list_for_session(&state.db, &req.session_id).await?;

    serde_json::to_value(rows).map_err(|e| AppError::Internal(e.to_string()))
}

/// Read one snapshot's real detail by id — the Timeline's by-id read (M4.2).
/// Returns the persisted `SnapshotRow` as JSON, or `NotFound` for an unknown id.
///
/// Fork/Restore are NOT handled here: they need persisted conversation history,
/// which does not exist yet, so the Timeline renders them as honestly disabled.
pub async fn read(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: ReadReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    let row = repo::snapshot::get(&state.db, &req.snapshot_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("snapshot id={} not found", req.snapshot_id)))?;

    serde_json::to_value(&row).map_err(|e| AppError::Internal(e.to_string()))
}

/// Delete a snapshot by id (the Memory list's row delete). Maps to
/// `snapshot.delete`; returns `{ deleted: <id> }`, or `NotFound` for an unknown
/// id. A child snapshot chained to it is re-linked in the repo (no dangling FK).
pub async fn delete(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: ReadReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    let removed = repo::snapshot::delete(&state.db, &req.snapshot_id).await?;
    if !removed {
        return Err(AppError::NotFound(format!(
            "snapshot id={} not found",
            req.snapshot_id
        )));
    }
    Ok(json!({ "deleted": req.snapshot_id }))
}

/// Payload for `snapshot.send` — inject a chosen snapshot's content into a live
/// agent's terminal ("ref → send to agent").
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendReq {
    instance_id: String,
    snapshot_id: String,
}

/// Submit a snapshot's content into a live agent's input — the "send to agent"
/// row action. Sends the handoff text (`carried_forward`) when present, else the
/// honest `summary` marker line; auto-submitted like an injection. Maps to
/// `snapshot.send`.
///
/// Errors: unknown snapshot → NotFound; agent not running → NotFound; a snapshot
/// with no content at all → Invalid (nothing to send).
pub async fn send(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: SendReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    if !state.runtime.is_live(&req.instance_id) {
        return Err(AppError::NotFound(format!(
            "instance {} is not running — spawn it first",
            req.instance_id
        )));
    }

    let snap = repo::snapshot::get(&state.db, &req.snapshot_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("snapshot id={} not found", req.snapshot_id)))?;

    let content = snap.carried_forward.or(snap.summary).ok_or_else(|| {
        AppError::Invalid(format!(
            "snapshot id={} has no content to send",
            req.snapshot_id
        ))
    })?;

    submit_line(&state.runtime, &req.instance_id, &content).await;
    Ok(json!({ "status": "sent", "instanceId": req.instance_id }))
}

// ── Strategic-compact loop (agent self-handoff) ───────────────────────────────

/// Settle pause around `/clear` so the harness finishes one input (rendering the
/// "saved" output, then clearing) before the next is typed.
const COMPACT_SETTLE_MS: u64 = 2_000;

/// Payload for `snapshot.save` — the agent's own handoff text, instance-keyed.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveReq {
    instance_id: String,
    text: String,
}

/// Payload for `snapshot.last` / `snapshot.compact` — instance-keyed.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstanceReq {
    instance_id: String,
}

/// The harness command that clears a CLI agent's conversation history. Claude
/// Code and Codex both use `/clear`; centralised so a future divergence is a
/// one-line edit. (Runtime-verify per harness — clearing is destructive.)
fn clear_command(_cli_kind: Option<&str>) -> &'static str {
    "/clear"
}

/// Type a line into a live agent's stdin and submit it. A TUI's Enter is CR
/// (`\r`) sent as a SEPARATE write after a short beat, so Codex's paste-burst
/// detection treats it as a real Enter (mirrors [`super::message::inject`]).
/// Best-effort: a dead/closed backend is ignored — the caller owns liveness.
/// `pub(crate)`: also used by the restart loop in `commands::instance`.
pub(crate) async fn submit_line(runtime: &Runtime, instance_id: &str, text: &str) {
    if runtime.send_stdin(instance_id, text).is_ok() {
        tokio::time::sleep(Duration::from_millis(40)).await;
        let _ = runtime.send_stdin(instance_id, "\r");
    }
}

/// Persist the agent's self-written handoff as a `handoff` snapshot (M4.2's
/// deferred `carried_forward`, now filled). Instance-keyed: a spawned agent knows
/// its own `CONCLAVE_INSTANCE_ID`, not its session id. The real text lands in
/// `carried_forward`; [`last`] reads it back verbatim after `/clear`.
pub async fn save(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: SaveReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    let session = repo::session::get_by_instance(&state.db, &req.instance_id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!("no session for instance id={}", req.instance_id))
        })?;

    // A handoff's size is its OWN saved text, not the session context meter —
    // which reads 0 for CLI agents (they track context themselves). Estimate it
    // from the handoff length (~4 chars/token, rounded up so any non-empty
    // handoff is ≥1) so the row shows "~N tok" of real saved content, not ~0.
    let saved_tokens = req.text.chars().count().div_ceil(4) as i64;

    let row = persist_and_emit(
        &state.db,
        state.app(),
        NewSnapshot {
            session_id: session.id,
            kind: "handoff".to_owned(),
            label: None,
            summary: Some(format!("handoff · ~{saved_tokens} tokens saved")),
            tokens: Some(saved_tokens),
            trigger_pct: None, // a handoff is saved content, not a context-window %
            prev_snapshot_id: None, // resolved inside persist_and_emit
            carried_forward: Some(req.text),
        },
    )
    .await?;

    // If a restart or compact is armed for this agent, THIS save is the trigger:
    // now that the handoff is durably persisted, fire the destructive tail.
    // Doing it here — not in a poll from `compact`/`restart` — is what
    // guarantees the kill / `/clear` runs strictly AFTER the save completes
    // (the agent has finished writing and called us). Detached so the agent's
    // `snapshot save` returns promptly. Restart is checked FIRST: it is the
    // stronger operation, and the two arms live in separate maps so a compact
    // arm can never be consumed as a restart (or vice versa).
    if state.take_restart_pending(&req.instance_id) {
        if let Some(app) = state.app().cloned() {
            tauri::async_runtime::spawn(super::instance::run_respawn_resume(
                app,
                req.instance_id.clone(),
                true, // kill the live process first — the handoff is saved.
            ));
        }
    } else if state.take_compact_pending(&req.instance_id) {
        if let Some(clear_cmd) = resolve_clear_cmd(&state.db, &req.instance_id).await {
            let runtime = state.runtime.clone();
            let instance_id = req.instance_id.clone();
            tauri::async_runtime::spawn(run_clear_restore(runtime, instance_id, clear_cmd));
        }
    }

    serde_json::to_value(&row).map_err(|e| AppError::Internal(e.to_string()))
}

/// Return the newest snapshot for the agent's session (instance-keyed), so a
/// just-cleared agent can read back the handoff it saved. NotFound if the
/// instance has no session or no snapshot yet.
pub async fn last(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: InstanceReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    let session = repo::session::get_by_instance(&state.db, &req.instance_id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!("no session for instance id={}", req.instance_id))
        })?;

    // Type-filtered: a just-fired `auto` marker must not be returned as the
    // "handoff" to restore from (it has no `carried_forward`).
    let row = repo::snapshot::latest_handoff_for_session(&state.db, &session.id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "no handoff yet for instance id={}",
                req.instance_id
            ))
        })?;

    serde_json::to_value(&row).map_err(|e| AppError::Internal(e.to_string()))
}

/// Arm the strategic-compact loop for a live CLI agent and inject the "save your
/// handoff" prompt. Maps to `snapshot.compact`. Returns IMMEDIATELY.
///
/// The `/clear` + restore are NOT driven from here — they fire from the
/// [`save`] handler when the agent actually calls `conclave snapshot save` (we
/// arm the agent via [`AppState::mark_compact_pending`]). That ordering is the
/// whole point: `/clear` runs strictly AFTER the save completes, never racing it.
///
/// SAFETY: because the clear is gated on a real save, an agent that ignores the
/// prompt is simply never cleared — Conclave never destroys uncaptured context.
pub async fn compact(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: InstanceReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    // The instance must exist and be running — there's no TUI to drive otherwise.
    if repo::workspace_agent::get(&state.db, &req.instance_id)
        .await?
        .is_none()
    {
        return Err(AppError::NotFound(format!(
            "instance id={} not found",
            req.instance_id
        )));
    }
    if !state.runtime.is_live(&req.instance_id) {
        return Err(AppError::NotFound(format!(
            "instance {} is not running — spawn it first",
            req.instance_id
        )));
    }

    // Arm FIRST, then inject — so a fast agent that saves the instant it reads the
    // prompt still finds the arm set when its `save` lands.
    state.mark_compact_pending(&req.instance_id);
    submit_line(
        &state.runtime,
        &req.instance_id,
        &agentctx::compact_save_prompt(),
    )
    .await;

    Ok(json!({ "status": "compacting", "instanceId": req.instance_id }))
}

/// Inject the resume prompt into a live agent so it reloads the last handoff
/// saved for its session (`conclave snapshot last`) and continues from it.
/// Maps to `snapshot.resume` — the UI's "Resume" action for an agent that came
/// back with an empty context (e.g. after the whole app was relaunched).
///
/// Non-destructive: nothing is killed or cleared; the prompt is just typed into
/// the live terminal. Errors: agent not running → NotFound; no handoff snapshot
/// to resume from → NotFound (injecting the prompt would only confuse the
/// agent — `conclave snapshot last` would come back empty).
pub async fn resume(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: InstanceReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    if !state.runtime.is_live(&req.instance_id) {
        return Err(AppError::NotFound(format!(
            "instance {} is not running — spawn it first",
            req.instance_id
        )));
    }

    let session = repo::session::get_by_instance(&state.db, &req.instance_id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!("no session for instance id={}", req.instance_id))
        })?;
    repo::snapshot::latest_handoff_for_session(&state.db, &session.id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "no handoff to resume from for instance id={}",
                req.instance_id
            ))
        })?;

    submit_line(
        &state.runtime,
        &req.instance_id,
        &agentctx::resume_restore_prompt(),
    )
    .await;
    Ok(json!({ "status": "resuming", "instanceId": req.instance_id }))
}

/// Resolve the harness-specific clear command for an instance (`instance → def →
/// cli_kind`). `None` if the instance/def is gone — the caller then skips the
/// clear rather than guessing.
async fn resolve_clear_cmd(db: &SqlitePool, instance_id: &str) -> Option<String> {
    let inst = repo::workspace_agent::get(db, instance_id).await.ok()??;
    let def = repo::agent_definition::get(db, &inst.agent_def_id)
        .await
        .ok()??;
    Some(clear_command(def.cli_kind.as_deref()).to_string())
}

/// Clear the harness, settle, then inject the restore prompt. Run as a detached
/// task from [`save`] once the handoff is persisted, so `/clear` lands strictly
/// after the save. Self-contained (owned clones) so it outlives the JSON-RPC call.
async fn run_clear_restore(runtime: Arc<Runtime>, instance_id: String, clear_cmd: String) {
    // Let the agent render its "saved" output and settle, THEN clear.
    tokio::time::sleep(Duration::from_millis(COMPACT_SETTLE_MS)).await;
    if !runtime.is_live(&instance_id) {
        return; // agent exited between the save and now — nothing to clear.
    }
    submit_line(&runtime, &instance_id, &clear_cmd).await;
    tokio::time::sleep(Duration::from_millis(COMPACT_SETTLE_MS)).await;
    submit_line(&runtime, &instance_id, &agentctx::compact_restore_prompt()).await;
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::repo::{
        agent_definition::{self, AgentDefinitionInput},
        session, workspace, workspace_agent,
    };
    use serde_json::json;

    #[test]
    fn should_auto_compact_boundary() {
        // 90% of 200_000 = 180_000 → false just below, true at/above.
        assert!(!should_auto_compact(179_999, 200_000, 90));
        assert!(should_auto_compact(180_000, 200_000, 90));
        assert!(should_auto_compact(190_000, 200_000, 90));
        // A non-positive limit disables auto-compaction.
        assert!(!should_auto_compact(180_000, 0, 90));
    }

    /// Create a workspace → agent_definition → workspace_agent → session and
    /// return the session id.
    async fn fixture_session(state: &AppState) -> String {
        let ws = workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        let def = agent_definition::create(
            &state.db,
            &AgentDefinitionInput {
                name: "SnapCmdAgent".into(),
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
        let wa = workspace_agent::instantiate(&state.db, &ws.id, &def.id)
            .await
            .expect("instantiate failed");
        // `instantiate` creates the session atomically; fetch its id.
        session::get_by_instance(&state.db, &wa.id)
            .await
            .expect("get_by_instance failed")
            .expect("session exists")
            .id
    }

    /// Manual create end-to-end: tokens/triggerPct/type populated, id non-empty,
    /// and the snapshot appears in `list`.
    #[tokio::test]
    async fn manual_create_then_list() {
        let state = AppState::for_tests().await;
        let sid = fixture_session(&state).await;

        session::set_context_tokens(&state.db, &sid, 50_000)
            .await
            .expect("set_context_tokens failed");

        let out = create(&state, json!({ "sessionId": sid, "type": "manual" }))
            .await
            .expect("snapshot.create failed");

        assert_eq!(out.get("type").and_then(Value::as_str), Some("manual"));
        assert_eq!(out.get("tokens").and_then(Value::as_i64), Some(50_000));
        // 50_000 / 200_000 = 25%.
        assert_eq!(out.get("triggerPct").and_then(Value::as_i64), Some(25));
        assert_eq!(
            out.get("sessionId").and_then(Value::as_str),
            Some(sid.as_str())
        );
        assert!(
            out.get("id")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty()),
            "id must be a non-empty string"
        );

        let listed = list(&state, json!({ "sessionId": sid }))
            .await
            .expect("snapshot.list failed");
        let arr = listed.as_array().expect("list returns an array");
        assert_eq!(arr.len(), 1, "the created snapshot must appear in the list");
        assert_eq!(
            arr[0].get("id"),
            out.get("id"),
            "listed snapshot matches the created one"
        );
    }

    /// create for an unknown session → NotFound (no fabricated row).
    #[tokio::test]
    async fn create_unknown_session_not_found() {
        let state = AppState::for_tests().await;
        let err = create(&state, json!({ "sessionId": "nope", "type": "manual" }))
            .await
            .expect_err("create must fail for an unknown session");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    /// read by id returns the real persisted row; an unknown id → NotFound.
    #[tokio::test]
    async fn read_returns_row_and_not_found() {
        let state = AppState::for_tests().await;
        let sid = fixture_session(&state).await;

        let created = create(&state, json!({ "sessionId": sid, "type": "manual" }))
            .await
            .expect("snapshot.create failed");
        let snap_id = created
            .get("id")
            .and_then(Value::as_str)
            .expect("created snapshot has an id")
            .to_owned();

        let got = read(&state, json!({ "snapshotId": snap_id }))
            .await
            .expect("read must return the row");
        assert_eq!(
            got.get("id").and_then(Value::as_str),
            Some(snap_id.as_str())
        );
        assert_eq!(got.get("type").and_then(Value::as_str), Some("manual"));
        assert_eq!(
            got.get("sessionId").and_then(Value::as_str),
            Some(sid.as_str())
        );

        let err = read(&state, json!({ "snapshotId": "no-such-id" }))
            .await
            .expect_err("read must fail for an unknown id");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    /// create_auto chains prev_snapshot_id to the prior latest snapshot.
    #[tokio::test]
    async fn create_auto_chains_prev() {
        let state = AppState::for_tests().await;
        let sid = fixture_session(&state).await;

        let first = create_auto(&state.db, None, &sid, 180_000, 200_000)
            .await
            .expect("first create_auto failed");
        assert_eq!(first.r#type, "auto");
        assert_eq!(first.trigger_pct, Some(90));
        assert!(first.prev_snapshot_id.is_none(), "first has no prev");

        let second = create_auto(&state.db, None, &sid, 181_000, 200_000)
            .await
            .expect("second create_auto failed");
        assert_eq!(
            second.prev_snapshot_id.as_deref(),
            Some(first.id.as_str()),
            "second chains to the first"
        );
    }

    /// Like `fixture_session` but returns the INSTANCE (workspace_agent) id —
    /// what the agent-facing `save`/`last`/`compact` handlers are keyed on.
    async fn fixture_instance(state: &AppState) -> String {
        let ws = workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        let def = agent_definition::create(
            &state.db,
            &AgentDefinitionInput {
                name: "CompactAgent".into(),
                role: None,
                agent_type: "cli".into(),
                cli_kind: Some("codex".into()),
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
        workspace_agent::instantiate(&state.db, &ws.id, &def.id)
            .await
            .expect("instantiate failed")
            .id
    }

    /// save persists the agent's handoff in `carried_forward`; last reads it back.
    #[tokio::test]
    async fn save_then_last_roundtrip() {
        let state = AppState::for_tests().await;
        let inst = fixture_instance(&state).await;

        let text = "goal: ship X; done: scaffolding; next: write the parser tests";
        let out = save(&state, json!({ "instanceId": inst, "text": text }))
            .await
            .expect("save failed");

        assert_eq!(out.get("type").and_then(Value::as_str), Some("handoff"));
        assert_eq!(
            out.get("carriedForward").and_then(Value::as_str),
            Some(text)
        );
        // tokens reflect the handoff's OWN size (not the 0 session meter), so the
        // UI never shows "~0 tok" for a handoff that actually has content.
        assert!(
            out.get("tokens")
                .and_then(Value::as_i64)
                .is_some_and(|t| t > 0),
            "handoff tokens must be > 0 for non-empty content"
        );

        let got = last(&state, json!({ "instanceId": inst }))
            .await
            .expect("last failed");
        assert_eq!(got.get("id"), out.get("id"), "last returns the saved row");
        assert_eq!(
            got.get("carriedForward").and_then(Value::as_str),
            Some(text),
            "handoff text round-trips verbatim"
        );
    }

    /// save for an unknown instance → NotFound (no fabricated row).
    #[tokio::test]
    async fn save_unknown_instance_not_found() {
        let state = AppState::for_tests().await;
        let err = save(&state, json!({ "instanceId": "nope", "text": "x" }))
            .await
            .expect_err("save must fail for an unknown instance");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    /// last with no snapshot yet → NotFound (nothing to restore).
    #[tokio::test]
    async fn last_with_no_snapshot_not_found() {
        let state = AppState::for_tests().await;
        let inst = fixture_instance(&state).await;
        let err = last(&state, json!({ "instanceId": inst }))
            .await
            .expect_err("last must fail when no snapshot exists");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    /// compact on a non-running instance → NotFound, and (importantly) spawns no
    /// detached tail that could clear a dead agent.
    #[tokio::test]
    async fn compact_not_running_not_found() {
        let state = AppState::for_tests().await;
        let inst = fixture_instance(&state).await;
        let err = compact(&state, json!({ "instanceId": inst }))
            .await
            .expect_err("compact must fail when the agent is not live");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    /// delete removes a snapshot; an unknown id → NotFound.
    #[tokio::test]
    async fn delete_removes_then_not_found() {
        let state = AppState::for_tests().await;
        let inst = fixture_instance(&state).await;
        let saved = save(&state, json!({ "instanceId": inst, "text": "hand off" }))
            .await
            .expect("save failed");
        let id = saved.get("id").and_then(Value::as_str).unwrap().to_owned();

        let out = delete(&state, json!({ "snapshotId": id }))
            .await
            .expect("delete failed");
        assert_eq!(
            out.get("deleted").and_then(Value::as_str),
            Some(id.as_str())
        );

        // Second delete of the same id → NotFound (already gone).
        let err = delete(&state, json!({ "snapshotId": id }))
            .await
            .expect_err("delete of a removed id must NotFound");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    /// send to a non-running agent → NotFound (no stdin to write to).
    #[tokio::test]
    async fn send_to_not_running_not_found() {
        let state = AppState::for_tests().await;
        let inst = fixture_instance(&state).await;
        let err = send(&state, json!({ "instanceId": inst, "snapshotId": "x" }))
            .await
            .expect_err("send must fail when the agent is not live");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    /// The compact arm is consumed exactly once: take is true only after a mark,
    /// and a second take is empty (so a later manual save can't re-trip a clear).
    #[tokio::test]
    async fn compact_pending_arm_is_consumed_once() {
        let state = AppState::for_tests().await;
        assert!(!state.take_compact_pending("i1"), "nothing armed yet");
        state.mark_compact_pending("i1");
        assert!(state.take_compact_pending("i1"), "armed → first take true");
        assert!(
            !state.take_compact_pending("i1"),
            "second take is empty — arm consumed"
        );
    }

    /// The restart arm mirrors the compact arm's consume-once semantics AND is
    /// isolated from it: arming a restart must not be consumable as a compact
    /// (or vice versa) — the two tails are very different in destructiveness.
    #[tokio::test]
    async fn restart_pending_arm_is_consumed_once_and_isolated() {
        let state = AppState::for_tests().await;
        assert!(!state.take_restart_pending("i1"), "nothing armed yet");
        state.mark_restart_pending("i1");
        assert!(
            !state.take_compact_pending("i1"),
            "a restart arm must not be consumable as a compact"
        );
        assert!(state.take_restart_pending("i1"), "armed → first take true");
        assert!(
            !state.take_restart_pending("i1"),
            "second take is empty — arm consumed"
        );
    }

    /// resume on a non-running agent → NotFound (no terminal to type into).
    #[tokio::test]
    async fn resume_not_running_not_found() {
        let state = AppState::for_tests().await;
        let inst = fixture_instance(&state).await;
        let err = resume(&state, json!({ "instanceId": inst }))
            .await
            .expect_err("resume must fail when the agent is not live");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    /// resume on a live agent with NO handoff → NotFound (there is nothing for
    /// `conclave snapshot last` to return — injecting would only confuse it).
    #[tokio::test]
    async fn resume_without_handoff_not_found() {
        let state = AppState::for_tests().await;
        let inst = fixture_instance(&state).await;
        let session = session::get_by_instance(&state.db, &inst)
            .await
            .expect("get_by_instance failed")
            .expect("session exists");
        assert!(state
            .runtime
            .register(
                &inst,
                crate::engine::runtime::LiveHandle::placeholder(&session.id)
            )
            .is_some());

        let err = resume(&state, json!({ "instanceId": inst }))
            .await
            .expect_err("resume must fail with no handoff to resume from");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    /// resume on a live agent WITH a handoff injects the prompt (absorbed by the
    /// placeholder backend) and acknowledges with status "resuming".
    #[tokio::test]
    async fn resume_with_handoff_ok() {
        let state = AppState::for_tests().await;
        let inst = fixture_instance(&state).await;
        save(
            &state,
            json!({ "instanceId": inst, "text": "next: finish the parser" }),
        )
        .await
        .expect("save failed");
        let session = session::get_by_instance(&state.db, &inst)
            .await
            .expect("get_by_instance failed")
            .expect("session exists");
        assert!(state
            .runtime
            .register(
                &inst,
                crate::engine::runtime::LiveHandle::placeholder(&session.id)
            )
            .is_some());

        let out = resume(&state, json!({ "instanceId": inst }))
            .await
            .expect("resume failed");
        assert_eq!(out.get("status").and_then(Value::as_str), Some("resuming"));
    }

    #[test]
    fn clear_command_is_slash_clear() {
        assert_eq!(clear_command(Some("claude-code")), "/clear");
        assert_eq!(clear_command(Some("codex")), "/clear");
        assert_eq!(clear_command(None), "/clear");
    }
}
