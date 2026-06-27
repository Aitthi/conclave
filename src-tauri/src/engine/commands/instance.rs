use crate::engine::{bus, repo, runtime, AppError, AppState};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

// ── Context-estimate constants ────────────────────────────────────────────────
//
// HONESTY: there is no real provider token-usage telemetry yet, so the live
// context meter is a labelled ESTIMATE derived from streamed output bytes. We
// approximate ≈4 chars per token (a common rough English ratio) and persist the
// running estimate every ~100 tokens of output, NOT on every delta — a DB write
// per chunk would be wasteful. Both values are coarse on purpose.

/// Rough characters-per-token ratio for the streamed-output estimate.
const CHARS_PER_TOKEN: usize = 4;

/// Persist + emit the context estimate once this many new chars accumulate
/// (≈100 tokens between writes).
const FLUSH_CHARS: usize = 400;

/// Auto-compact trigger as a whole percent of the context limit (≈90%).
const AUTO_COMPACT_PCT: i64 = 90;

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
///    - `chat`: spawn the provider chat loop (M2.4) which streams assistant
///      text deltas back over the same forwarder.
///    - otherwise (`orchestrator` / unknown): attach the placeholder backend
///      (fusion arrives in M4).
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

    // Build the backend and register it. `cli` (PTY child) and `chat` (provider
    // chat loop) both produce an output stream to forward; `orchestrator` and
    // anything else attach the placeholder backend (fusion arrives in M4) and
    // have no stream. In every branch a successful `register` yields an
    // `Option<output_rx>`; a lost race returns the existing session early.
    let output_rx = match def.r#type.as_str() {
        "cli" => {
            // Map the configured CLI kind to a concrete launcher command.
            // `custom` and unset both defer to M5 settings.
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
            // dropped (its shutdown closure tears down the just-spawned child)
            // and we return the existing session without double-persisting.
            if !state.runtime.register(&id, backend.handle) {
                return serde_json::to_value(&session)
                    .map_err(|e| AppError::Internal(e.to_string()));
            }
            Some(backend.output_rx)
        }
        "chat" => {
            // Resolve the provider from the agent's `provider_id`. The API key
            // comes from the macOS Keychain (or env vars) inside `from_config`;
            // the user's Settings-configured base URL (if any) is then applied
            // as an override from the DB.
            let base_url_override = match def.provider_id.as_deref() {
                Some(name) => repo::provider::get_by_name(&state.db, name)
                    .await
                    .ok()
                    .flatten()
                    .and_then(|r| r.base_url),
                None => None,
            };
            let provider = runtime::provider::Provider::from_config(def.provider_id.as_deref())
                .map_err(|e| AppError::Invalid(format!("chat agent provider: {e}")))?
                .with_base_url(base_url_override.as_deref());
            let model = def
                .model
                .clone()
                .ok_or_else(|| AppError::Invalid("chat agent has no model configured".into()))?;

            let backend = runtime::chat::spawn_chat(&session.id, provider, model);

            // Same lost-race handling as the CLI branch: the dropped handle's
            // shutdown closure aborts the just-spawned chat loop.
            if !state.runtime.register(&id, backend.handle) {
                return serde_json::to_value(&session)
                    .map_err(|e| AppError::Internal(e.to_string()));
            }
            Some(backend.output_rx)
        }
        _ => {
            // orchestrator / unknown: placeholder backend (fusion arrives in M4).
            if !state
                .runtime
                .register(&id, runtime::LiveHandle::placeholder(&session.id))
            {
                return serde_json::to_value(&session)
                    .map_err(|e| AppError::Internal(e.to_string()));
            }
            None
        }
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
        tokio::spawn(forward_session_output(
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

/// Drain a backend's output stream (CLI PTY chunks or chat-loop assistant text
/// deltas) onto the event bus as `session:output` chunks, then perform idle
/// cleanup when the backend self-terminates (the source hits EOF and
/// `output_rx` closes). Shared by the `cli` and `chat` backends; spawned
/// detached by [`spawn`].
///
/// The idle transition is gated on `unregister` returning `true`, so a
/// concurrent `stop` and this EOF path can't both emit `idle` — only the winner
/// does. `app` is `None` in non-Tauri contexts (tests); emits are then skipped.
async fn forward_session_output(
    db: sqlx::SqlitePool,
    runtime: Arc<runtime::Runtime>,
    app: Option<tauri::AppHandle>,
    instance_id: String,
    session_id: String,
    mut output_rx: tokio::sync::mpsc::Receiver<String>,
) {
    // Read the session's context limit once (default if unset or on error) — it
    // drives both the live meter denominator and the auto-compact threshold.
    let limit = match repo::session::get(&db, &session_id).await {
        Ok(Some(s)) => s
            .context_limit
            .unwrap_or(repo::session::DEFAULT_CONTEXT_LIMIT),
        _ => repo::session::DEFAULT_CONTEXT_LIMIT,
    };

    // Rolling ESTIMATE of context usage in characters. `last_flush_chars` is the
    // baseline at the previous persist, so we only write every FLUSH_CHARS.
    let mut total_chars: usize = 0;
    let mut last_flush_chars: usize = 0;

    while let Some(chunk) = output_rx.recv().await {
        // Count before moving `chunk` into the emit — avoids cloning every chunk
        // just to measure it. `chars().count()` (not `len()`) so multi-byte UTF-8
        // output isn't over-counted in the ≈4-chars/token estimate.
        total_chars += chunk.chars().count();

        if let Some(app) = &app {
            let _ = bus::session_output(
                app,
                bus::SessionOutput {
                    session_id: session_id.clone(),
                    chunk,
                },
            );
        }

        // Flush the estimate roughly every ~100 tokens of new output.
        if total_chars - last_flush_chars >= FLUSH_CHARS {
            let compacted =
                flush_context_estimate(&db, app.as_ref(), &session_id, total_chars, limit).await;
            last_flush_chars = total_chars;
            if compacted {
                // Auto-compact boundary: model the post-compaction window by
                // resetting the estimate baseline so the meter re-arms and won't
                // re-fire until it fills again. This is an ESTIMATE-based
                // compaction boundary — real summary carry-forward is deferred
                // to M4.2 (the snapshot's carried_forward stays NULL).
                total_chars = 0;
                last_flush_chars = 0;
            }
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

/// Persist + emit the current context estimate, then run the auto-compact check.
///
/// Computes `tokens` from accumulated output chars (≈4 chars/token), writes the
/// estimate to the session, and emits a `session:context` event (labelled
/// `estimated: true`). If the estimate crosses the auto-compact threshold, an
/// `auto` snapshot is created via the shared `snapshot::create_auto` path and a
/// second `session:context` with `context_tokens: 0` is emitted to model the
/// post-compaction window. DB/emit errors are non-fatal (best-effort meter).
///
/// Returns `true` iff an auto-compaction fired, so the caller can reset its
/// char baseline and re-arm the trigger.
async fn flush_context_estimate(
    db: &sqlx::SqlitePool,
    app: Option<&tauri::AppHandle>,
    session_id: &str,
    total_chars: usize,
    limit: i64,
) -> bool {
    let tokens = (total_chars / CHARS_PER_TOKEN) as i64;

    // Best-effort persist — a failed write must not kill the forwarder.
    let _ = repo::session::set_context_tokens(db, session_id, tokens).await;

    if let Some(app) = app {
        let _ = bus::session_context(
            app,
            bus::SessionContext {
                session_id: session_id.to_owned(),
                context_tokens: tokens,
                context_limit: limit,
                estimated: true,
            },
        );
    }

    if super::snapshot::should_auto_compact(tokens, limit, AUTO_COMPACT_PCT) {
        // Shared auto-snapshot path (also emits `snapshot:created`). A failed
        // create is non-fatal (the meter keeps running), but we LOG it: the meter
        // is about to reset to a zero window, so without the snapshot row the UI
        // would imply a compaction that left no timeline entry. Logging makes that
        // discrepancy observable rather than silent.
        if let Err(e) = super::snapshot::create_auto(db, app, session_id, tokens, limit).await {
            eprintln!("auto-compact snapshot failed for session {session_id}: {e}");
        }

        // Post-compaction: persist + emit a fresh (estimated) zero window. The
        // real carry-forward summary is deferred to M4.2.
        let _ = repo::session::set_context_tokens(db, session_id, 0).await;
        if let Some(app) = app {
            let _ = bus::session_context(
                app,
                bus::SessionContext {
                    session_id: session_id.to_owned(),
                    context_tokens: 0,
                    context_limit: limit,
                    estimated: true,
                },
            );
        }
        return true;
    }

    false
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
    /// Uses an `orchestrator`-type agent so the lifecycle tests exercise the
    /// placeholder backend path — deterministic and binary-free. (`cli` would
    /// take the PTY path and try to spawn `claude`; `chat` would require a
    /// configured provider + API key, neither of which CI has.)
    async fn fixture_instance(state: &AppState) -> String {
        fixture_instance_typed(state, "orchestrator", None).await
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
        let task = tokio::spawn(forward_session_output(
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

    /// The forwarder must persist a context-token ESTIMATE once enough output
    /// chars accumulate (≥ FLUSH_CHARS), before EOF. app=None in tests, so the
    /// `session:context` emit is skipped — we assert only the DB write landed.
    #[tokio::test]
    async fn forwarder_updates_context_estimate() {
        let state = AppState::for_tests().await;
        let id = fixture_instance(&state).await;
        let session = repo::session::get_by_instance(&state.db, &id)
            .await
            .expect("get_by_instance failed")
            .expect("session exists");

        assert!(state
            .runtime
            .register(&id, runtime::LiveHandle::placeholder(&session.id)));
        workspace_agent::set_status(&state.db, &id, "running")
            .await
            .expect("set running");

        let (tx, rx) = tokio::sync::mpsc::channel::<String>(8);
        let task = tokio::spawn(forward_session_output(
            state.db.clone(),
            Arc::clone(&state.runtime),
            None,
            id.clone(),
            session.id.clone(),
            rx,
        ));

        // Push > FLUSH_CHARS (400) chars so a flush fires.
        tx.send("x".repeat(500)).await.expect("send chunk failed");
        drop(tx); // EOF → forwarder finishes.
        task.await.expect("forwarder task panicked");

        let after = repo::session::get(&state.db, &session.id)
            .await
            .expect("get failed")
            .expect("session exists");
        assert!(
            after.context_tokens.unwrap_or(0) > 0,
            "forwarder must persist a positive token estimate (got {:?})",
            after.context_tokens
        );
        // 500 chars / 4 ≈ 125 tokens.
        assert_eq!(after.context_tokens, Some(125));
    }

    /// The forwarder's auto-compact branch: enough streamed output to cross 90%
    /// of the default 200_000-token limit (= 180_000 tokens = 720_000 chars)
    /// must create an `auto` snapshot AND reset the persisted estimate to the
    /// zero post-compaction window. Exercises the `if compacted { … }` reset that
    /// no other test covers (the snapshot repo test calls `create_auto` directly,
    /// bypassing the forwarder).
    #[tokio::test]
    async fn forwarder_auto_compacts_at_threshold() {
        let state = AppState::for_tests().await;
        let id = fixture_instance(&state).await;
        let session = repo::session::get_by_instance(&state.db, &id)
            .await
            .expect("get_by_instance failed")
            .expect("session exists");

        assert!(state
            .runtime
            .register(&id, runtime::LiveHandle::placeholder(&session.id)));
        workspace_agent::set_status(&state.db, &id, "running")
            .await
            .expect("set running");

        let (tx, rx) = tokio::sync::mpsc::channel::<String>(8);
        let task = tokio::spawn(forward_session_output(
            state.db.clone(),
            Arc::clone(&state.runtime),
            None,
            id.clone(),
            session.id.clone(),
            rx,
        ));

        // 720_000 chars / 4 = 180_000 tokens = 90% of the 200_000 default limit.
        tx.send("x".repeat(720_000))
            .await
            .expect("send chunk failed");
        drop(tx); // EOF → forwarder finishes.
        task.await.expect("forwarder task panicked");

        // An `auto` snapshot was persisted for the session…
        let snaps = repo::snapshot::list_for_session(&state.db, &session.id)
            .await
            .expect("list_for_session failed");
        assert_eq!(snaps.len(), 1, "auto-compact must persist one snapshot");
        assert_eq!(snaps[0].r#type, "auto");
        assert_eq!(snaps[0].tokens, Some(180_000));
        assert_eq!(snaps[0].trigger_pct, Some(90));

        // …and the estimate was reset to the zero post-compaction window.
        let after = repo::session::get(&state.db, &session.id)
            .await
            .expect("get failed")
            .expect("session exists");
        assert_eq!(
            after.context_tokens,
            Some(0),
            "estimate must reset to 0 after an auto-compact boundary"
        );
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
