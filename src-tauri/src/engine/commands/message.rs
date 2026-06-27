use crate::engine::{AppError, AppState};
use serde_json::{json, Value};

// TODO(M2): real impl — send a user message to a running agent instance
#[allow(dead_code)]
pub async fn send(_state: &AppState, _payload: Value) -> Result<Value, AppError> {
    Ok(json!({ "stub": "message.send", "todo": true }))
}

// TODO(M2): real impl — inject a system/tool message into an instance conversation
#[allow(dead_code)]
pub async fn inject(_state: &AppState, _payload: Value) -> Result<Value, AppError> {
    Ok(json!({ "stub": "message.inject", "todo": true }))
}
