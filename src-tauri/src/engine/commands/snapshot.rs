use crate::engine::{AppError, AppState};
use serde_json::{json, Value};

// TODO(M3): real impl — create a new conversation snapshot
#[allow(dead_code)]
pub async fn create(_state: &AppState, _payload: Value) -> Result<Value, AppError> {
    Ok(json!({ "stub": "snapshot.create", "todo": true }))
}

// TODO(M3): real impl — list all snapshots for an instance
#[allow(dead_code)]
pub async fn list(_state: &AppState, _payload: Value) -> Result<Value, AppError> {
    Ok(json!([]))
}

// TODO(M3): real impl — read the full content of a snapshot by ID
#[allow(dead_code)]
pub async fn read(_state: &AppState, _payload: Value) -> Result<Value, AppError> {
    Ok(json!({ "stub": "snapshot.read", "todo": true }))
}
