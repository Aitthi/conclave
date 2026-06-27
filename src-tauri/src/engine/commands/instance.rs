use crate::engine::{AppError, AppState};
use serde_json::{json, Value};

// TODO(M2): real impl — list running agent instances from DB/runtime
#[allow(dead_code)]
pub async fn list(_state: &AppState, _payload: Value) -> Result<Value, AppError> {
    Ok(json!([]))
}

// TODO(M2): real impl — spawn a new agent process/PTY session
#[allow(dead_code)]
pub async fn spawn(_state: &AppState, _payload: Value) -> Result<Value, AppError> {
    Ok(json!({ "stub": "instance.spawn", "todo": true }))
}
