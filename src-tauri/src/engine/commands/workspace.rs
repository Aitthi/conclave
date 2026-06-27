use crate::engine::{AppError, AppState};
use serde_json::{json, Value};

// TODO(M1): real impl — query DB for all linked workspaces
#[allow(dead_code)]
pub async fn list(_state: &AppState, _payload: Value) -> Result<Value, AppError> {
    Ok(json!([]))
}

// TODO(M1): real impl — validate path, create workspace record
#[allow(dead_code)]
pub async fn link(_state: &AppState, _payload: Value) -> Result<Value, AppError> {
    Ok(json!({ "stub": "workspace.link", "todo": true }))
}

// TODO(M1): real impl — set the active workspace in session state
#[allow(dead_code)]
pub async fn use_workspace(_state: &AppState, _payload: Value) -> Result<Value, AppError> {
    Ok(json!({ "stub": "workspace.use", "todo": true }))
}
