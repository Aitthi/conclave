use crate::engine::repo::agent_definition::AgentDefinitionInput;
use crate::engine::{repo, AppError, AppState};
use serde::Deserialize;
use serde_json::{json, Value};

// ── Request types ────────────────────────────────────────────────────────────

/// Payload for `agentDef.save` — create if `id` is absent, update if present.
///
/// `toolIds` / `skillIds` are accepted and forwarded without error but deferred
/// to M5 (when the `agent_tool` / `agent_skill` join tables will be persisted).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveAgentReq {
    id: Option<String>,
    name: String,
    #[serde(rename = "type")]
    agent_type: String,
    role: Option<String>,
    cli_kind: Option<String>,
    color: Option<String>,
    provider_id: Option<String>,
    model: Option<String>,
    harness_mode: Option<String>,
    share_blackboard: Option<bool>,
    auto_submit_injected: Option<bool>,
    allowed_senders: Option<String>,
    // Accepted but deferred — TODO(M5): persist agent_tool / agent_skill joins.
    #[allow(dead_code)]
    tool_ids: Option<Vec<String>>,
    #[allow(dead_code)]
    skill_ids: Option<Vec<String>>,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// Return all agent definitions annotated with their workspace count.
///
/// Maps to `agentDef.list` on the IPC bus.
pub async fn list(state: &AppState, _payload: Value) -> Result<Value, AppError> {
    let items = repo::agent_definition::list_with_counts(&state.db).await?;
    serde_json::to_value(items).map_err(|e| AppError::Internal(e.to_string()))
}

/// Create or update an agent definition.
///
/// Maps to `agentDef.save` on the IPC bus.
/// - `id` absent → INSERT (new UUID assigned).
/// - `id` present → UPDATE; `NotFound` error if the id doesn't exist.
pub async fn save(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: SaveAgentReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    let input = AgentDefinitionInput {
        name: req.name,
        role: req.role,
        agent_type: req.agent_type,
        cli_kind: req.cli_kind,
        color: req.color,
        provider_id: req.provider_id,
        model: req.model,
        harness_mode: req.harness_mode.unwrap_or_else(|| "own".to_owned()),
        share_blackboard: req.share_blackboard,
        auto_submit_injected: req.auto_submit_injected,
        allowed_senders: req.allowed_senders,
    };

    let row = match req.id.as_deref() {
        None => repo::agent_definition::create(&state.db, &input).await?,
        Some(id) => repo::agent_definition::update(&state.db, id, &input)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("agent_definition id={id} not found")))?,
    };

    serde_json::to_value(row).map_err(|e| AppError::Internal(e.to_string()))
}

/// Link an agent definition to one or more workspaces.
///
/// TODO(M1.5): real impl — create a `workspace_agent` + `session` row per workspace.
#[allow(dead_code)]
pub async fn add_to_workspace(_state: &AppState, _payload: Value) -> Result<Value, AppError> {
    Ok(json!({ "stub": "agentDef.addToWorkspace", "todo": true }))
}
