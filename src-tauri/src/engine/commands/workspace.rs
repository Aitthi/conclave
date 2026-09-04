use std::path::Path;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::engine::{bus, repo, AppError, AppState};

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

// ── workspace.update ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateReq {
    workspace_id: String,
    name: String,
    color: Option<String>,
}

/// `workspace.update` — rename and/or recolor a workspace, returning the
/// updated row. `folder_path` is not editable (the workspace stays bound to
/// the folder it was linked from).
pub async fn update(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req = serde_json::from_value::<UpdateReq>(payload)
        .map_err(|e| AppError::Invalid(e.to_string()))?;

    let row = repo::workspace::update(
        &state.db,
        &req.workspace_id,
        &req.name,
        req.color.as_deref(),
    )
    .await?
    .ok_or_else(|| AppError::NotFound(format!("workspace {}", req.workspace_id)))?;

    serde_json::to_value(row).map_err(|e| AppError::Internal(e.to_string()))
}

// ── workspace lifecycle ───────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LifecycleReq {
    workspace_id: String,
}

pub async fn start(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req = serde_json::from_value::<LifecycleReq>(payload)
        .map_err(|e| AppError::Invalid(e.to_string()))?;
    let workspace_lock = state.workspace_lifecycle_lock(&req.workspace_id);
    let _workspace_guard = workspace_lock.write().await;

    let workspace = repo::workspace::get(&state.db, &req.workspace_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("workspace {}", req.workspace_id)))?;
    let workspace = if workspace.run_state == "started" {
        workspace
    } else {
        repo::workspace::set_run_state(&state.db, &req.workspace_id, "started")
            .await?
            .ok_or_else(|| AppError::NotFound(format!("workspace {}", req.workspace_id)))?
    };
    emit_lifecycle_changed(state, &workspace);

    let agents = repo::workspace_agent::list_by_workspace(&state.db, &req.workspace_id).await?;
    let mut ready_agent_ids = Vec::new();
    let mut skipped_stopped_agent_ids = Vec::new();
    let mut failures = Vec::new();
    for agent in agents {
        if agent.availability == "stopped" {
            skipped_stopped_agent_ids.push(agent.id);
            continue;
        }
        let agent_lock = state.agent_lifecycle_lock(&agent.id);
        let _agent_guard = agent_lock.lock().await;
        match super::instance::spawn_under_workspace_write(state, &agent.id).await {
            Ok(_) => ready_agent_ids.push(agent.id),
            Err(error) => failures.push(json!({
                "workspaceAgentId": agent.id,
                "error": error.to_string(),
            })),
        }
    }

    Ok(json!({
        "workspace": workspace,
        "readyAgentIds": ready_agent_ids,
        "skippedStoppedAgentIds": skipped_stopped_agent_ids,
        "failures": failures,
    }))
}

pub async fn stop(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req = serde_json::from_value::<LifecycleReq>(payload)
        .map_err(|e| AppError::Invalid(e.to_string()))?;
    let workspace_lock = state.workspace_lifecycle_lock(&req.workspace_id);
    let _workspace_guard = workspace_lock.write().await;

    let workspace = repo::workspace::set_run_state(&state.db, &req.workspace_id, "stopped")
        .await?
        .ok_or_else(|| AppError::NotFound(format!("workspace {}", req.workspace_id)))?;
    let agents = repo::workspace_agent::list_by_workspace(&state.db, &req.workspace_id).await?;
    let mut stopped_runtime_ids = Vec::new();
    for agent in agents {
        let agent_lock = state.agent_lifecycle_lock(&agent.id);
        let _agent_guard = agent_lock.lock().await;
        if super::instance::teardown_under_lifecycle_lock(state, &agent.id).await? {
            stopped_runtime_ids.push(agent.id);
        }
    }
    emit_lifecycle_changed(state, &workspace);
    Ok(json!({
        "workspace": workspace,
        "stoppedRuntimeIds": stopped_runtime_ids,
    }))
}

fn emit_lifecycle_changed(state: &AppState, workspace: &repo::workspace::WorkspaceRow) {
    state.emit(
        bus::WORKSPACE_CHANGED,
        bus::WorkspaceChanged {
            workspace_id: workspace.id.clone(),
            run_state: workspace.run_state.clone(),
        },
    );
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteReq {
    workspace_id: String,
}

/// `workspace.delete` — tear down every agent in the workspace, then delete the
/// (now agent-less) workspace row. `blackboard_entry` cascades away directly
/// via `workspace_id`, but `workspace_agent` itself must be removed through
/// `repo::workspace_agent::remove` rather than left to a bare cascade: several
/// tables reference `workspace_agent` with `NO ACTION` (`inter_agent_message`
/// NOT NULL from/to, `blackboard_activity`), so a raw `DELETE FROM workspace`
/// would abort with a FK violation the moment any instance had ever sent an
/// inter-agent message. Each live instance is also unregistered from the
/// runtime first (mirrors `agentDef.delete` / `instance.remove`).
pub async fn delete(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req = serde_json::from_value::<DeleteReq>(payload)
        .map_err(|e| AppError::Invalid(e.to_string()))?;

    let workspace_lock = state.workspace_lifecycle_lock(&req.workspace_id);
    let _workspace_guard = workspace_lock.write().await;

    if !repo::workspace::exists(&state.db, &req.workspace_id).await? {
        return Err(AppError::NotFound(format!(
            "workspace {}",
            req.workspace_id
        )));
    }

    let instances = repo::workspace_agent::list_by_workspace(&state.db, &req.workspace_id).await?;
    for inst in &instances {
        let agent_lock = state.agent_lifecycle_lock(&inst.id);
        let _agent_guard = agent_lock.lock().await;
        super::instance::remove_under_workspace_write(state, &inst.id).await?;
    }

    repo::workspace::delete(&state.db, &req.workspace_id).await?;
    state.memory_search_cache.invalidate(&req.workspace_id);

    Ok(Value::Null)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{
        repo::{
            agent_definition::{self, AgentDefinitionInput},
            inter_agent_message, session, workspace_agent,
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
            ..Default::default()
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
                // agent_input sets no cli_kind, so the resolver's conservative
                // branch is the expected stamp.
                Some(session::default_context_limit_for("")),
                "session context_limit must match the resolver default for its cli_kind"
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

    /// update() renames and recolors an existing workspace.
    #[tokio::test]
    async fn update_renames_and_recolors() {
        let state = AppState::for_tests().await;
        let row = repo::workspace::create(&state.db, "Before", "/tmp/upd", None)
            .await
            .expect("create failed");

        let payload = json!({
            "workspaceId": row.id,
            "name": "After",
            "color": "#5e5ce6",
        });
        let result = update(&state, payload).await.expect("update failed");
        assert_eq!(result["name"], "After");
        assert_eq!(result["color"], "#5e5ce6");
        assert_eq!(result["id"], row.id);
    }

    /// update() on an unknown workspace id returns NotFound.
    #[tokio::test]
    async fn update_unknown_workspace_not_found() {
        let state = AppState::for_tests().await;
        let payload = json!({ "workspaceId": "no-such-id", "name": "X" });
        let err = update(&state, payload).await.expect_err("should fail");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    /// delete() removes the workspace and cascades its workspace_agent rows,
    /// even when a `workspace_agent` has inter-agent-message history.
    /// `inter_agent_message.from_instance_id`/`to_instance_id` reference
    /// `workspace_agent` with `NO ACTION` (not a cascade), so a bare
    /// `DELETE FROM workspace` would abort with a FK violation here — this
    /// locks in that `workspace_agent::remove` is called per instance first.
    #[tokio::test]
    async fn delete_removes_workspace_and_cascades_agents() {
        let state = AppState::for_tests().await;
        let ws = repo::workspace::create(&state.db, "Doomed", "/tmp/doomed", None)
            .await
            .expect("create workspace");
        let def = agent_definition::create(&state.db, &agent_input("Resident"))
            .await
            .expect("create def");
        let wa = workspace_agent::instantiate(&state.db, &ws.id, &def.id)
            .await
            .expect("instantiate");
        let def2 = agent_definition::create(&state.db, &agent_input("Other"))
            .await
            .expect("create def2");
        let wa2 = workspace_agent::instantiate(&state.db, &ws.id, &def2.id)
            .await
            .expect("instantiate2");
        inter_agent_message::create(&state.db, &wa.id, &wa2.id, "hi", "delivered", false)
            .await
            .expect("create inter_agent_message");

        let payload = json!({ "workspaceId": ws.id });
        delete(&state, payload)
            .await
            .expect("delete failed (FK violation would surface here)");

        assert!(!repo::workspace::exists(&state.db, &ws.id)
            .await
            .expect("exists check"));
        assert!(
            !workspace_agent::exists(&state.db, &wa.id)
                .await
                .expect("exists check"),
            "workspace_agent must cascade-delete with its workspace"
        );
    }

    /// delete() on an unknown workspace id returns NotFound.
    #[tokio::test]
    async fn delete_unknown_workspace_not_found() {
        let state = AppState::for_tests().await;
        let payload = json!({ "workspaceId": "no-such-id" });
        let err = delete(&state, payload).await.expect_err("should fail");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn start_launches_active_agents_skips_stopped_and_reports_failures() {
        let state = AppState::for_tests().await;
        let mut ready_input = agent_input("Ready");
        ready_input.agent_type = "orchestrator".into();
        let ready_def = agent_definition::create(&state.db, &ready_input)
            .await
            .unwrap();
        let mut stopped_input = agent_input("Stopped");
        stopped_input.agent_type = "orchestrator".into();
        let stopped_def = agent_definition::create(&state.db, &stopped_input)
            .await
            .unwrap();
        let failed_def = agent_definition::create(&state.db, &agent_input("BadCli"))
            .await
            .unwrap();
        let linked = link(
            &state,
            json!({
                "folderPath": "/tmp/lifecycle-start",
                "name": "Lifecycle",
                "agentDefIds": [ready_def.id, stopped_def.id, failed_def.id],
            }),
        )
        .await
        .unwrap();
        assert_eq!(linked["workspace"]["runState"], "stopped");
        let agents = linked["agents"].as_array().unwrap();
        let ready_id = agents[0]["id"].as_str().unwrap().to_owned();
        let stopped_id = agents[1]["id"].as_str().unwrap().to_owned();
        let failed_id = agents[2]["id"].as_str().unwrap().to_owned();
        repo::workspace_agent::set_availability(&state.db, &stopped_id, "stopped")
            .await
            .unwrap();

        let workspace_id = linked["workspace"]["id"].as_str().unwrap();
        let first = start(&state, json!({ "workspaceId": workspace_id }))
            .await
            .unwrap();
        assert_eq!(first["workspace"]["runState"], "started");
        assert_eq!(first["readyAgentIds"], json!([ready_id]));
        assert_eq!(first["skippedStoppedAgentIds"], json!([stopped_id]));
        assert_eq!(first["failures"][0]["workspaceAgentId"], failed_id);
        assert!(state.runtime.is_live(&ready_id));
        assert!(!state.runtime.is_live(&stopped_id));
        assert!(!state.runtime.is_live(&failed_id));

        let second = start(&state, json!({ "workspaceId": workspace_id }))
            .await
            .unwrap();
        assert_eq!(second["readyAgentIds"], first["readyAgentIds"]);
        assert_eq!(
            second["skippedStoppedAgentIds"],
            first["skippedStoppedAgentIds"]
        );
        assert_eq!(second["failures"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn stop_is_idempotent_and_preserves_individual_availability() {
        let state = AppState::for_tests().await;
        let mut input = agent_input("Resident");
        input.agent_type = "orchestrator".into();
        let def = agent_definition::create(&state.db, &input).await.unwrap();
        let linked = link(
            &state,
            json!({ "folderPath": "/tmp/lifecycle-stop", "agentDefIds": [def.id] }),
        )
        .await
        .unwrap();
        let workspace_id = linked["workspace"]["id"].as_str().unwrap();
        let agent_id = linked["agents"][0]["id"].as_str().unwrap();
        start(&state, json!({ "workspaceId": workspace_id }))
            .await
            .unwrap();
        state.mark_restart_pending(agent_id);

        let first = stop(&state, json!({ "workspaceId": workspace_id }))
            .await
            .unwrap();
        assert_eq!(first["workspace"]["runState"], "stopped");
        assert_eq!(first["stoppedRuntimeIds"], json!([agent_id]));
        let retained = repo::workspace_agent::get(&state.db, agent_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retained.availability, "active");
        assert_eq!(retained.status, "idle");
        assert!(!state.take_restart_pending(agent_id));

        let second = stop(&state, json!({ "workspaceId": workspace_id }))
            .await
            .unwrap();
        assert_eq!(second["stoppedRuntimeIds"], json!([]));
    }

    #[tokio::test]
    async fn racing_start_and_stop_leave_runtime_consistent_with_persisted_state() {
        let state = std::sync::Arc::new(AppState::for_tests().await);
        let mut input = agent_input("Racer");
        input.agent_type = "orchestrator".into();
        let def = agent_definition::create(&state.db, &input).await.unwrap();
        let linked = link(
            &state,
            json!({ "folderPath": "/tmp/lifecycle-race", "agentDefIds": [def.id] }),
        )
        .await
        .unwrap();
        let workspace_id = linked["workspace"]["id"].as_str().unwrap().to_owned();
        let agent_id = linked["agents"][0]["id"].as_str().unwrap().to_owned();
        let (started, stopped) = tokio::join!(
            start(&state, json!({ "workspaceId": workspace_id })),
            stop(&state, json!({ "workspaceId": workspace_id })),
        );
        started.unwrap();
        stopped.unwrap();
        let workspace = repo::workspace::get(&state.db, &workspace_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            state.runtime.is_live(&agent_id),
            workspace.run_state == "started"
        );
    }
}
