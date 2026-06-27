use crate::engine::{bus, repo, runtime, AppError, AppState};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

// ── Request types ────────────────────────────────────────────────────────────

/// Payload for `instance.list` — filter by workspace.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListInstancesReq {
    workspace_id: String,
}

/// Payload for `instance.spawn` / `instance.stop` — target a single instance.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstanceReq {
    workspace_agent_id: String,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// Return all workspace_agent instances for a workspace, ordered by added_at.
///
/// Maps to `instance.list` on the IPC bus.
/// Pulled forward from M2 so instances are observable after `addToWorkspace`;
/// the roster UI wiring remains in M2.
pub async fn list(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: ListInstancesReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    let rows = repo::workspace_agent::list_by_workspace(&state.db, &req.workspace_id).await?;

    serde_json::to_value(rows).map_err(|e| AppError::Internal(e.to_string()))
}

/// Spawn (or attach to) the live session for a workspace_agent instance.
///
/// Maps to `instance.spawn` on the IPC bus and returns the `Session` row.
///
/// Lifecycle:
/// 1. Validate the instance exists.
/// 2. Load its session (created atomically by `instantiate`).
/// 3. Idempotent: if already live, return the existing session unchanged.
/// 4. Dispatch on the agent type:
///    - `cli`: spawn the real CLI process inside a PTY (M2.2) and stream its
///      output back over the bus; the detached forwarder also marks the
///      instance `idle` when the child self-terminates.
///    - otherwise (`chat` / `orchestrator`): attach the placeholder backend
///      (the provider chat loop arrives in M2.4).
/// 5. Register it in the runtime, persist status `running`, and emit a
///    `session:status` event.
pub async fn spawn(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: InstanceReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    let id = req.workspace_agent_id;

    if !repo::workspace_agent::exists(&state.db, &id).await? {
        return Err(AppError::NotFound(format!(
            "workspace_agent id={id} not found"
        )));
    }

    let session = repo::session::get_by_instance(&state.db, &id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!("session for workspace_agent id={id} not found"))
        })?;

    // Idempotency: already live → return the existing session unchanged.
    if state.runtime.is_live(&id) {
        return serde_json::to_value(&session).map_err(|e| AppError::Internal(e.to_string()));
    }

    // Load the instance row + its definition + workspace to choose the backend.
    let instance = repo::workspace_agent::get(&state.db, &id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("workspace_agent id={id} not found")))?;
    let def = repo::agent_definition::get(&state.db, &instance.agent_def_id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "agent_definition id={} not found",
                instance.agent_def_id
            ))
        })?;
    let ws = repo::workspace::get(&state.db, &instance.workspace_id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!("workspace id={} not found", instance.workspace_id))
        })?;

    // Build the backend and register it. For CLI we keep the PTY output stream
    // to forward; for chat/orchestrator the placeholder has none.
    let output_rx = if def.r#type == "cli" {
        // Map the configured CLI kind to a concrete launcher command. `custom`
        // and unset both defer to M5 settings.
        let command = match def.cli_kind.as_deref() {
            Some("claude-code") => "claude",
            Some("codex") => "codex",
            _ => {
                return Err(AppError::NotImplemented(
                    "custom CLI command is not configurable yet (M5 settings)".into(),
                ))
            }
        };

        let backend = runtime::pty::spawn_cli(&session.id, command, &[], &ws.folder_path)
            .map_err(|e| AppError::Internal(format!("spawn {command}: {e}")))?;

        // Register; if we lost a race with a concurrent spawn, the handle is
        // dropped (its shutdown closure tears down the just-spawned child) and
        // we return the existing session without double-persisting.
        if !state.runtime.register(&id, backend.handle) {
            return serde_json::to_value(&session).map_err(|e| AppError::Internal(e.to_string()));
        }
        Some(backend.output_rx)
    } else {
        // chat / orchestrator: placeholder backend (chat loop arrives in M2.4).
        if !state
            .runtime
            .register(&id, runtime::LiveHandle::placeholder(&session.id))
        {
            return serde_json::to_value(&session).map_err(|e| AppError::Internal(e.to_string()));
        }
        None
    };

    // Persist `running` and emit BEFORE spawning the forwarder. The forwarder
    // only flips the instance to `idle` after the child's output stream hits
    // EOF; committing `running` first means a fast-exiting child can't have its
    // `idle` overwritten back to `running` by this handler (the ordering bug a
    // post-spawn persist would create).
    repo::workspace_agent::set_status(&state.db, &id, "running").await?;
    state.emit(
        bus::SESSION_STATUS,
        bus::SessionStatus {
            session_id: session.id.clone(),
            status: "running".into(),
        },
    );

    // Detached forwarder: bridge PTY output → bus, and mark the instance idle
    // when the child self-terminates (EOF closes output_rx).
    if let Some(output_rx) = output_rx {
        tokio::spawn(forward_pty_output(
            state.db.clone(),
            Arc::clone(&state.runtime),
            state.app().cloned(),
            id.clone(),
            session.id.clone(),
            output_rx,
        ));
    }

    serde_json::to_value(&session).map_err(|e| AppError::Internal(e.to_string()))
}

/// Drain a CLI backend's PTY output onto the event bus as `session:output`
/// chunks, then perform idle cleanup when the child self-terminates (the read
/// side hits EOF and `output_rx` closes). Spawned detached by [`spawn`].
///
/// The idle transition is gated on `unregister` returning `true`, so a
/// concurrent `stop` and this EOF path can't both emit `idle` — only the winner
/// does. `app` is `None` in non-Tauri contexts (tests); emits are then skipped.
async fn forward_pty_output(
    db: sqlx::SqlitePool,
    runtime: Arc<runtime::Runtime>,
    app: Option<tauri::AppHandle>,
    instance_id: String,
    session_id: String,
    mut output_rx: tokio::sync::mpsc::Receiver<String>,
) {
    while let Some(chunk) = output_rx.recv().await {
        if let Some(app) = &app {
            let _ = bus::session_output(
                app,
                bus::SessionOutput {
                    session_id: session_id.clone(),
                    chunk,
                },
            );
        }
    }
    // Child exited / EOF. Idempotent self-termination cleanup.
    if runtime.unregister(&instance_id) {
        let _ = repo::workspace_agent::set_status(&db, &instance_id, "idle").await;
        if let Some(app) = &app {
            let _ = bus::session_status(
                app,
                bus::SessionStatus {
                    session_id,
                    status: "idle".into(),
                },
            );
        }
    }
}

/// Stop a live session: abort its backend task, mark the instance idle, emit status.
///
/// Payload is `{ workspaceAgentId }`. Idempotent: a no-op (returns `null`) if
/// the instance is not live.
///
/// `#[allow(dead_code)]`: routed in a later milestone — UI stop button /
/// app teardown.
#[allow(dead_code)]
pub async fn stop(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: InstanceReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    let id = req.workspace_agent_id;

    // Capture the live session id BEFORE unregister; no-op if not live.
    let Some(session_id) = state.runtime.session_id(&id) else {
        return Ok(Value::Null);
    };

    // Tear down; if a concurrent stop already unregistered (returns false),
    // skip the redundant persist + emit.
    if !state.runtime.unregister(&id) {
        return Ok(Value::Null);
    }
    repo::workspace_agent::set_status(&state.db, &id, "idle").await?;
    state.emit(
        bus::SESSION_STATUS,
        bus::SessionStatus {
            session_id,
            status: "idle".into(),
        },
    );

    Ok(Value::Null)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::repo::{
        agent_definition::{self, AgentDefinitionInput},
        workspace, workspace_agent,
    };
    use serde_json::json;

    /// Create a workspace + agent_definition, instantiate an instance (idle,
    /// with a session), and return its workspace_agent id.
    ///
    /// Uses a `chat`-type agent so the lifecycle tests exercise the placeholder
    /// backend path — deterministic and binary-free (a `cli` agent would take
    /// the PTY path and try to spawn `claude`, which CI does not have).
    async fn fixture_instance(state: &AppState) -> String {
        fixture_instance_typed(state, "chat", None).await
    }

    /// Like [`fixture_instance`] but with an explicit agent `type` and
    /// `cli_kind`, for exercising the CLI dispatch branch.
    async fn fixture_instance_typed(
        state: &AppState,
        agent_type: &str,
        cli_kind: Option<&str>,
    ) -> String {
        let ws = workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        let def = agent_definition::create(
            &state.db,
            &AgentDefinitionInput {
                name: "SpawnTestAgent".into(),
                role: None,
                agent_type: agent_type.into(),
                cli_kind: cli_kind.map(str::to_owned),
                color: None,
                provider_id: None,
                model: None,
                harness_mode: "own".into(),
                share_blackboard: None,
                auto_submit_injected: None,
                allowed_senders: None,
            },
        )
        .await
        .expect("create agent_def failed");
        workspace_agent::instantiate(&state.db, &ws.id, &def.id)
            .await
            .expect("instantiate failed")
            .id
    }

    #[tokio::test]
    async fn spawn_marks_running_and_live() {
        let state = AppState::for_tests().await;
        let id = fixture_instance(&state).await;

        let session = repo::session::get_by_instance(&state.db, &id)
            .await
            .expect("get_by_instance failed")
            .expect("session exists");

        let out = spawn(&state, json!({ "workspaceAgentId": id }))
            .await
            .expect("spawn failed");

        assert_eq!(
            out.get("id").and_then(Value::as_str),
            Some(session.id.as_str())
        );
        assert!(state.runtime.is_live(&id));

        let row = workspace_agent::get(&state.db, &id)
            .await
            .expect("get failed")
            .expect("row exists");
        assert_eq!(row.status, "running");
    }

    #[tokio::test]
    async fn spawn_is_idempotent() {
        let state = AppState::for_tests().await;
        let id = fixture_instance(&state).await;

        let first = spawn(&state, json!({ "workspaceAgentId": id }))
            .await
            .expect("first spawn failed");
        let second = spawn(&state, json!({ "workspaceAgentId": id }))
            .await
            .expect("second spawn failed");

        assert_eq!(first.get("id"), second.get("id"));
        assert_eq!(state.runtime.live_count(), 1);
    }

    /// A `cli` agent with no `cli_kind` (or `custom`) is not launchable yet:
    /// spawn must surface `NotImplemented` and NOT mark the instance live.
    #[tokio::test]
    async fn spawn_cli_unknown_kind_not_implemented() {
        let state = AppState::for_tests().await;
        let id = fixture_instance_typed(&state, "cli", None).await;

        let err = spawn(&state, json!({ "workspaceAgentId": id }))
            .await
            .expect_err("spawn should fail for unconfigured cli kind");
        assert!(matches!(err, AppError::NotImplemented(_)));
        assert!(!state.runtime.is_live(&id));
    }

    #[tokio::test]
    async fn spawn_unknown_instance_not_found() {
        let state = AppState::for_tests().await;

        let err = spawn(&state, json!({ "workspaceAgentId": "nope" }))
            .await
            .expect_err("spawn should fail for unknown instance");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    /// The detached forwarder must mark the instance `idle` (and unregister it)
    /// when its output stream closes — the self-termination path for a CLI child
    /// that exits on its own. Driven directly with a channel (no real process)
    /// so it stays binary-free.
    #[tokio::test]
    async fn forwarder_marks_idle_on_eof() {
        let state = AppState::for_tests().await;
        let id = fixture_instance(&state).await;
        let session = repo::session::get_by_instance(&state.db, &id)
            .await
            .expect("get_by_instance failed")
            .expect("session exists");

        // Put the instance in the live+running state the forwarder expects.
        assert!(state
            .runtime
            .register(&id, runtime::LiveHandle::placeholder(&session.id)));
        workspace_agent::set_status(&state.db, &id, "running")
            .await
            .expect("set running");

        let (tx, rx) = tokio::sync::mpsc::channel::<String>(8);
        let task = tokio::spawn(forward_pty_output(
            state.db.clone(),
            Arc::clone(&state.runtime),
            None,
            id.clone(),
            session.id.clone(),
            rx,
        ));

        drop(tx); // EOF → forwarder runs its idle cleanup.
        task.await.expect("forwarder task panicked");

        assert!(!state.runtime.is_live(&id), "instance must be unregistered");
        let row = workspace_agent::get(&state.db, &id)
            .await
            .expect("get failed")
            .expect("row exists");
        assert_eq!(row.status, "idle");
    }

    #[tokio::test]
    async fn stop_marks_idle() {
        let state = AppState::for_tests().await;
        let id = fixture_instance(&state).await;

        spawn(&state, json!({ "workspaceAgentId": id }))
            .await
            .expect("spawn failed");
        assert!(state.runtime.is_live(&id));

        let out = stop(&state, json!({ "workspaceAgentId": id }))
            .await
            .expect("stop failed");
        assert_eq!(out, Value::Null);
        assert!(!state.runtime.is_live(&id));

        let row = workspace_agent::get(&state.db, &id)
            .await
            .expect("get failed")
            .expect("row exists");
        assert_eq!(row.status, "idle");
    }
}
