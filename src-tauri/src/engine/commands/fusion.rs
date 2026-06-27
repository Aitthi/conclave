use crate::engine::{AppError, AppState};
use serde_json::{json, Value};

// TODO(M4): real impl — run the context-fusion pipeline across selected instances
#[allow(dead_code)]
pub async fn run(_state: &AppState, _payload: Value) -> Result<Value, AppError> {
    Ok(json!({ "stub": "fusion.run", "todo": true }))
}
