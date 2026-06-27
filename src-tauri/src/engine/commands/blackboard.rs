use crate::engine::{AppError, AppState};
use serde_json::{json, Value};

// TODO(M1): real impl — list all keys in the workspace blackboard
#[allow(dead_code)]
pub async fn list(_state: &AppState, _payload: Value) -> Result<Value, AppError> {
    Ok(json!([]))
}

// TODO(M1): real impl — get a value by key from the blackboard
#[allow(dead_code)]
pub async fn get(_state: &AppState, _payload: Value) -> Result<Value, AppError> {
    Ok(json!({ "stub": "blackboard.get", "todo": true }))
}

// TODO(M1): real impl — set a key/value pair in the blackboard
#[allow(dead_code)]
pub async fn set(_state: &AppState, _payload: Value) -> Result<Value, AppError> {
    Ok(json!({ "stub": "blackboard.set", "todo": true }))
}
