use crate::engine::{AppError, AppState};
use serde_json::{json, Value};

// TODO(M2): real impl — exec a CLI command in the workspace PTY/shell
#[allow(dead_code)]
pub async fn exec(_state: &AppState, _payload: Value) -> Result<Value, AppError> {
    Ok(json!({ "stub": "cli.exec", "todo": true }))
}
