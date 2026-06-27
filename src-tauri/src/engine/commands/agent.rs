use crate::engine::{AppError, AppState};
use serde_json::{json, Value};

// TODO(M1): real impl — return all agent definitions from DB
#[allow(dead_code)]
pub async fn list(_state: &AppState, _payload: Value) -> Result<Value, AppError> {
    Ok(json!([]))
}

// TODO(M1): real impl — upsert an agent definition into DB
#[allow(dead_code)]
pub async fn save(_state: &AppState, _payload: Value) -> Result<Value, AppError> {
    Ok(json!({ "stub": "agentDef.save", "todo": true }))
}

// TODO(M1): real impl — link an agent definition to a workspace
#[allow(dead_code)]
pub async fn add_to_workspace(_state: &AppState, _payload: Value) -> Result<Value, AppError> {
    Ok(json!({ "stub": "agentDef.addToWorkspace", "todo": true }))
}
