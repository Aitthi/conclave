//! Snapshot command handlers — the snapshot manager (M4.1).
//!
//! Snapshots mark a point on a session's rolling context window. A `manual`
//! snapshot is created from the UI / CLI; an `auto` snapshot is created by the
//! output forwarder when the context estimate crosses the auto-compact
//! threshold (≈90%); a `handoff` snapshot is reserved for agent→agent context
//! transfer (not created from the UI).
//!
//! # honesty seams (M4.1)
//!
//! - `tokens` is a labelled ESTIMATE from streamed output bytes — there is no
//!   real provider token-usage telemetry yet (see `commands::instance`).
//! - `summary` is a deterministic honest string; real LLM summarisation /
//!   context carry-forward is out of scope (NULL `carried_forward` / `diff`).
//! - `read` (Read/Fork/Restore) is deferred to M4.2 and returns
//!   `NotImplemented` rather than a fabricated success.

use crate::engine::{bus, repo, AppError, AppState};
use repo::snapshot::{NewSnapshot, SnapshotRow};
use serde::Deserialize;
use serde_json::Value;
use sqlx::SqlitePool;
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

/// Payload for `snapshot.read` — open a snapshot in a given mode (M4.2).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadReq {
    #[allow(dead_code)]
    snapshot_id: String,
    #[allow(dead_code)]
    mode: String,
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
    let limit = session
        .context_limit
        .unwrap_or(repo::session::DEFAULT_CONTEXT_LIMIT);
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

/// Read / Fork / Restore a snapshot — deferred to M4.2 (Timeline). Returns an
/// honest `NotImplemented` rather than a fabricated success.
pub async fn read(_state: &AppState, payload: Value) -> Result<Value, AppError> {
    // Deserialise to validate the shape, then refuse honestly.
    let _req: ReadReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    Err(AppError::NotImplemented(
        "snapshot.read (Read/Fork/Restore) lands in M4.2".into(),
    ))
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

    /// read is honestly deferred to M4.2.
    #[tokio::test]
    async fn read_is_not_implemented() {
        let state = AppState::for_tests().await;
        let err = read(&state, json!({ "snapshotId": "x", "mode": "read" }))
            .await
            .expect_err("read must be NotImplemented in M4.1");
        assert!(matches!(err, AppError::NotImplemented(_)));
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
}
