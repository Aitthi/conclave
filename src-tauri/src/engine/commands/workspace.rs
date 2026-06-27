use std::path::Path;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::engine::{repo, AppError, AppState};

/// `workspace.list` — return all workspaces as a JSON array.
///
/// No payload fields are expected; the TS call site sends `null`.
pub async fn list(state: &AppState, _payload: Value) -> Result<Value, AppError> {
    let rows = repo::workspace::list(&state.db).await?;
    serde_json::to_value(rows).map_err(|e| AppError::Internal(e.to_string()))
}

// ── workspace.use ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UseReq {
    workspace_id: String,
}

/// `workspace.use` — validate that the workspace exists; active-workspace
/// selection is client-side React state.
pub async fn use_workspace(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req =
        serde_json::from_value::<UseReq>(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    if !repo::workspace::exists(&state.db, &req.workspace_id).await? {
        return Err(AppError::NotFound(format!(
            "workspace {}",
            req.workspace_id
        )));
    }

    // TS contract: `res: void` — return JSON null, not a wrapper object.
    Ok(Value::Null)
}

// ── workspace.link ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinkReq {
    folder_path: String,
    name: Option<String>,
    color: Option<String>,
    /// Agent definition ids to instantiate in this workspace on creation.
    /// Each id creates a `workspace_agent` + `session` pair atomically.
    /// Missing or empty list → zero agents instantiated.
    agent_def_ids: Option<Vec<String>>,
}

/// `workspace.link` — create a workspace for a folder path, optionally
/// instantiating a set of agent definitions as workspace_agents, and return
/// `{ workspace, agents }`.
///
/// `name` defaults to the folder's basename when omitted.
///
/// For each `agentDefId` in `agentDefIds`:
/// 1. Validate the agent_definition exists (`NotFound` if not).
/// 2. Call `repo::workspace_agent::instantiate` — the shared helper that
///    idempotently creates `workspace_agent` + `session` atomically.
pub async fn link(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req =
        serde_json::from_value::<LinkReq>(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    // Default name to folder basename if the caller did not supply one.
    let name = req.name.unwrap_or_else(|| {
        Path::new(&req.folder_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Workspace")
            .to_owned()
    });

    let agent_def_ids = req.agent_def_ids.unwrap_or_default();

    // Pre-validate ALL agent definitions BEFORE creating the workspace — so an
    // invalid id can't leave an orphan workspace row behind (the workspace
    // INSERT auto-commits, so a mid-loop NotFound would otherwise be partial).
    for agent_def_id in &agent_def_ids {
        if !repo::agent_definition::exists(&state.db, agent_def_id).await? {
            return Err(AppError::NotFound(format!(
                "agent_definition id={agent_def_id} not found"
            )));
        }
    }

    let row =
        repo::workspace::create(&state.db, &name, &req.folder_path, req.color.as_deref()).await?;

    // Instantiate each requested agent definition in the new workspace.
    let mut agents: Vec<repo::workspace_agent::WorkspaceAgentRow> = Vec::new();
    for agent_def_id in &agent_def_ids {
        let wa = repo::workspace_agent::instantiate(&state.db, &row.id, agent_def_id).await?;
        agents.push(wa);
    }

    let workspace_json =
        serde_json::to_value(&row).map_err(|e| AppError::Internal(e.to_string()))?;
    let agents_json =
        serde_json::to_value(&agents).map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(json!({ "workspace": workspace_json, "agents": agents_json }))
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{
        repo::{
            agent_definition::{self, AgentDefinitionInput},
            session, workspace_agent,
        },
        AppState,
    };
    use serde_json::json;

    fn agent_input(name: &str) -> AgentDefinitionInput {
        AgentDefinitionInput {
            name: name.into(),
            role: None,
            agent_type: "cli".into(),
            cli_kind: None,
            color: None,
            provider_id: None,
            model: None,
            harness_mode: "own".into(),
            share_blackboard: None,
            auto_submit_injected: None,
            allowed_senders: None,
        }
    }

    /// Linking with 2 agentDefIds creates 2 workspace_agents + 2 sessions.
    #[tokio::test]
    async fn link_with_two_agents_creates_instances_and_sessions() {
        let state = AppState::for_tests().await;

        let def1 = agent_definition::create(&state.db, &agent_input("AgentOne"))
            .await
            .expect("create def1");
        let def2 = agent_definition::create(&state.db, &agent_input("AgentTwo"))
            .await
            .expect("create def2");

        let payload = json!({
            "folderPath": "/tmp/test-link-ws",
            "name": "TestLinkWS",
            "agentDefIds": [def1.id, def2.id]
        });

        let result = link(&state, payload).await.expect("link failed");

        let agents_arr = result["agents"].as_array().expect("agents must be array");
        assert_eq!(agents_arr.len(), 2, "should have 2 workspace_agents");

        let ws_id = result["workspace"]["id"]
            .as_str()
            .expect("workspace.id must be a string");

        // Verify DB state: 2 workspace_agent rows, each with a session.
        let instances = workspace_agent::list_by_workspace(&state.db, ws_id)
            .await
            .expect("list_by_workspace");
        assert_eq!(instances.len(), 2);

        for wa in &instances {
            let sess = session::get_by_instance(&state.db, &wa.id)
                .await
                .expect("get_by_instance")
                .unwrap_or_else(|| panic!("session missing for workspace_agent {}", wa.id));
            assert_eq!(
                sess.context_limit,
                Some(session::DEFAULT_CONTEXT_LIMIT),
                "session context_limit must match DEFAULT_CONTEXT_LIMIT"
            );
        }
    }

    /// Linking with no agentDefIds creates workspace with zero agents.
    #[tokio::test]
    async fn link_without_agents_creates_empty() {
        let state = AppState::for_tests().await;

        let payload = json!({
            "folderPath": "/tmp/test-empty-ws",
            "name": "EmptyWS"
        });

        let result = link(&state, payload).await.expect("link failed");

        let agents_arr = result["agents"].as_array().expect("agents must be array");
        assert_eq!(agents_arr.len(), 0, "no agents should be created");
    }

    /// Linking with an unknown agentDefId returns NotFound.
    #[tokio::test]
    async fn link_with_unknown_agent_def_returns_not_found() {
        let state = AppState::for_tests().await;

        let payload = json!({
            "folderPath": "/tmp/test-bad-ws",
            "agentDefIds": ["no-such-def-id"]
        });

        let err = link(&state, payload).await.expect_err("should fail");
        assert!(
            matches!(err, AppError::NotFound(_)),
            "expected NotFound, got {err:?}"
        );

        // No zombie workspace: a failed link must NOT leave a workspace row.
        let all = repo::workspace::list(&state.db)
            .await
            .expect("list workspaces");
        assert_eq!(all.len(), 0, "failed link must not create a workspace");
    }
}
