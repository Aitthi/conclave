use std::path::Path;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::engine::{repo, AppError, AppState};

/// `workspace.list` — return all workspaces as a JSON array.
///
/// No payload fields are expected; the TS call site sends `null`.
pub async fn list(state: &AppState, _payload: Value) -> Result<Value, AppError> {
    let rows = repo::workspace::list(&state.db).await?;
    serde_json::to_value(rows).map_err(|e| AppError::Internal(e.to_string()))
}

// ── workspace.use ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UseReq {
    workspace_id: String,
}

/// `workspace.use` — validate that the workspace exists; active-workspace
/// selection is client-side React state.
pub async fn use_workspace(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req =
        serde_json::from_value::<UseReq>(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    if !repo::workspace::exists(&state.db, &req.workspace_id).await? {
        return Err(AppError::NotFound(format!(
            "workspace {}",
            req.workspace_id
        )));
    }

    // TS contract: `res: void` — return JSON null, not a wrapper object.
    Ok(Value::Null)
}

// ── workspace.link ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinkReq {
    folder_path: String,
    name: Option<String>,
    color: Option<String>,
    // agentDefIds: ignored until M1.2
}

/// `workspace.link` — create a workspace for a folder path and return it.
///
/// `name` defaults to the folder's basename when omitted.
///
/// `agentDefIds` in the payload is intentionally ignored — initial-agent
/// instantiation is wired in M1.2.
pub async fn link(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req =
        serde_json::from_value::<LinkReq>(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    // Default name to folder basename if the caller did not supply one.
    let name = req.name.unwrap_or_else(|| {
        Path::new(&req.folder_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Workspace")
            .to_owned()
    });

    let row =
        repo::workspace::create(&state.db, &name, &req.folder_path, req.color.as_deref()).await?;

    let workspace_json =
        serde_json::to_value(&row).map_err(|e| AppError::Internal(e.to_string()))?;

    // TODO(M1.2): instantiate agentDefIds as workspace_agents
    Ok(json!({ "workspace": workspace_json, "agents": [] }))
}
