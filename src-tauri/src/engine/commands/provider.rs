use crate::engine::{AppError, AppState};
use serde_json::{json, Value};

// TODO(M1): real impl — upsert an LLM provider config (API key, base URL, model list)
#[allow(dead_code)]
pub async fn upsert(_state: &AppState, _payload: Value) -> Result<Value, AppError> {
    Ok(json!({ "stub": "provider.upsert", "todo": true }))
}
