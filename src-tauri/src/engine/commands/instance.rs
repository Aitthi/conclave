use crate::engine::{repo, AppError, AppState};
use serde::Deserialize;
use serde_json::Value;

// ── Request types ────────────────────────────────────────────────────────────

/// Payload for `instance.list` — filter by workspace.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListInstancesReq {
    workspace_id: String,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// Return all workspace_agent instances for a workspace, ordered by added_at.
///
/// Maps to `instance.list` on the IPC bus.
/// Pulled forward from M2 so instances are observable after `addToWorkspace`;
/// the roster UI wiring remains in M2.
pub async fn list(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: ListInstancesReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    let rows = repo::workspace_agent::list_by_workspace(&state.db, &req.workspace_id).await?;

    serde_json::to_value(rows).map_err(|e| AppError::Internal(e.to_string()))
}

// TODO(M2): real impl — spawn a new agent process/PTY session
#[allow(dead_code)]
pub async fn spawn(_state: &AppState, _payload: Value) -> Result<Value, AppError> {
    Ok(serde_json::json!({ "stub": "instance.spawn", "todo": true }))
}
