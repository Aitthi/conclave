use crate::engine::{repo, AppError, AppState};
use serde_json::Value;

/// `tool.list` — all registered tools (core built-ins first). Read-only.
pub async fn list(state: &AppState, _payload: Value) -> Result<Value, AppError> {
    let tools = repo::tool::list_all(&state.db).await?;
    serde_json::to_value(tools).map_err(|e| AppError::Internal(e.to_string()))
}
