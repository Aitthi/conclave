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

    let rows =
        repo::workspace_agent::list_by_workspace_with_launched_skills(&state.db, &req.workspace_id)
            .await?;

    serde_json::to_value(rows).map_err(|e| AppError::Internal(e.to_string()))
}

/// Compute this instance's skill content (builtin + attached custom, via
/// `repo::skill::content_for_agent`) and, if non-empty, write it to a
/// per-instance sidecar file and append ONE sanitized pointer sentence to
/// `preamble` — never the raw content, which may contain '\n'/'=' and would
/// violate `bootstrap_preamble`'s single-line/'='-free contract (ADR 0001).
///
/// Does NOT persist `session.launched_skill_ids` — the caller must do that
/// ONLY after the launch this preamble is used for has actually succeeded
/// (see `spawn`'s `cli` branch), so a failed spawn or a lost registration
/// race never records a launch snapshot for content that was never used by a
/// live process.
///
/// Extracted out of `spawn`'s `cli` branch so it's unit-testable without
/// spawning a real PTY (this file's other tests avoid the `cli` dispatch
/// branch entirely — see `fixture_instance`'s doc comment).
async fn apply_skills_to_preamble(
    state: &AppState,
    agent_def_id: &str,
    instance_id: &str,
    preamble: String,
) -> Result<(String, Vec<String>), AppError> {
    let (skill_body, skill_ids) = repo::skill::content_for_agent(&state.db, agent_def_id).await?;
    let preamble = if skill_body.is_empty() {
        preamble
    } else {
        let path = crate::engine::agentctx::write_skill_sidecar(instance_id, &skill_body)
            .map_err(|e| AppError::Internal(format!("write skill sidecar: {e}")))?;
        format!(
            "{preamble} {}",
            crate::engine::agentctx::skill_pointer_sentence(&path)
        )
    };
    Ok((preamble, skill_ids))
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
            let base = match def.cli_kind.as_deref() {
                Some("claude-code") => "claude",
                Some("codex") => "codex",
                _ => {
                    return Err(AppError::NotImplemented(
                        "custom CLI command is not configurable yet (M5 settings)".into(),
                    ))
                }
            };

            // Build the launch command with the agent's configured flags. The
            // 1M-context variant of a model is its id with a `[1m]` suffix —
            // single-quoted so the shell doesn't try to glob the brackets.
            // Claude-specific flags are gated to claude; custom args apply to any.
            // Single-quote a value so the shell doesn't glob it (e.g. claude's
            // `[1m]`); POSIX-escape any embedded quote so it can't break out.
            let shell_quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));

            // Awareness briefing — injected via each harness's system-prompt layer
            // (NOT a chat turn), so it survives `/clear`. See engine::agentctx.
            let preamble = crate::engine::agentctx::bootstrap_preamble(
                &def.name,
                def.role.as_deref(),
                &ws.name,
                &ws.id,
                &id,
            );

            let (preamble, skill_ids) =
                apply_skills_to_preamble(state, &def.id, &id, preamble).await?;

            let mut launch = String::from(base);
            if base == "claude" {
                if let Some(mode) = def.permission_mode.as_deref() {
                    // Validated to an allowlist at save time, but quote anyway so a
                    // future bypass can't inject a second shell command here.
                    launch.push_str(&format!(" --permission-mode {}", shell_quote(mode)));
                }
                if let Some(model) = def.model.as_deref().filter(|m| !m.is_empty()) {
                    let eff = if def.context_window.as_deref() == Some("1m") {
                        format!("{model}[1m]")
                    } else {
                        model.to_string()
                    };
                    launch.push_str(&format!(" --model {}", shell_quote(&eff)));
                }
                // Persistent system-prompt append → survives /clear.
                launch.push_str(&format!(
                    " --append-system-prompt {}",
                    shell_quote(&preamble)
                ));
            } else if base == "codex" {
                if let Some(model) = def.model.as_deref().filter(|m| !m.is_empty()) {
                    launch.push_str(&format!(" --model {}", shell_quote(model)));
                }
                // Codex's mode flags differ from claude's; map the shared
                // permission_mode value to them. "auto" = never pause for
                // approval but keep the sandbox; "bypass" = --yolo (alias of
                // --dangerously-bypass-approvals-and-sandbox: no approvals AND no
                // sandbox).
                match def.permission_mode.as_deref() {
                    Some("auto") => launch.push_str(" --ask-for-approval never"),
                    Some("bypassPermissions") => launch.push_str(" --yolo"),
                    _ => {}
                }
                // Codex has no --append-system-prompt; its developer-instructions
                // config key is the equivalent persistent layer (survives /clear).
                // The value parses as TOML if it can, else as a literal string —
                // the preamble is one line with no '=', so it lands as a literal.
                launch.push_str(&format!(
                    " -c {}",
                    shell_quote(&format!("developer_instructions={preamble}"))
                ));
            }
            if let Some(extra) = def.custom_args.as_deref().filter(|s| !s.trim().is_empty()) {
                launch.push(' ');
                launch.push_str(extra.trim());
            }

            // Put `conclave` on the agent's PATH so the briefing's commands
            // resolve. The login+interactive shell sources its rc files BEFORE
            // running this `-c` command, so prepending the export here wins over
            // whatever PATH the rc files set. Best-effort: if the CLI binary isn't
            // found beside the app, skip it and launch without `conclave`.
            if let Some(bin) = crate::engine::agentctx::ensure_conclave_shim() {
                launch = format!(
                    "export PATH={}:\"$PATH\"; {}",
                    shell_quote(&bin.to_string_lossy()),
                    launch
                );
            }

            // Env overrides: non-secret vars from the DB JSON object + secret
            // values fetched back from the Keychain by their recorded names.
            let mut extra_env: Vec<(String, String)> = Vec::new();
            // Identity the conclave CLI reads: CONCLAVE_INSTANCE_ID is the sender
            // `conclave tell` fills in (server then tags the message [from <name>]);
            // CONCLAVE_WORKSPACE_ID saves the agent repeating its id on every call.
            extra_env.push(("CONCLAVE_WORKSPACE_ID".to_string(), ws.id.clone()));
            extra_env.push(("CONCLAVE_INSTANCE_ID".to_string(), id.clone()));
            if let Some(text) = def.custom_env.as_deref() {
                if let Ok(serde_json::Value::Object(map)) =
                    serde_json::from_str::<serde_json::Value>(text)
                {
                    for (k, v) in map {
                        if let Some(s) = v.as_str() {
                            extra_env.push((k, s.to_owned()));
                        }
                    }
                }
            }
            if let Some(text) = def.secret_env_keys.as_deref() {
                if let Ok(serde_json::Value::Array(names)) =
                    serde_json::from_str::<serde_json::Value>(text)
                {
                    for name in names.iter().filter_map(|n| n.as_str()) {
                        let account = format!("agent_env:{}:{}", def.id, name);
                        if let Ok(Some(val)) = crate::engine::secrets::get_key(&account) {
                            extra_env.push((name.to_owned(), val));
                        }
                    }
                }
            }

            // Launch the CLI INSIDE the user's login + interactive shell, the way
            // VS Code's integrated terminal does (it spawns the shell, not the
            // program). A Tauri app started from Finder has only a bare
            // environment; running `claude` directly from it both hides tools the
            // user has on their PATH and leaves the TUI mis-rendering (it never
            // sees the real terminal setup the user's rc files establish). `-l -i`
            // sources ~/.zprofile + ~/.zshrc so the child gets the exact
            // environment a normal terminal would, then `-c <cli>` runs it; the
            // shell exits when the CLI does, so the forwarder still sees EOF for
            // idle cleanup. `$SHELL` honors the user's chosen shell (zsh default).
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
            let shell_args = [
                "-l".to_string(),
                "-i".to_string(),
                "-c".to_string(),
                launch.clone(),
            ];

            let backend = runtime::pty::spawn_cli(
                &session.id,
                &shell,
                &shell_args,
                &ws.folder_path,
                &extra_env,
            )
            .map_err(|e| AppError::Internal(format!("spawn {shell} -c {launch}: {e}")))?;

            // Register; if we lost a race with a concurrent spawn, the handle is
            // dropped (its shutdown closure tears down the just-spawned child)
            // and we return the existing session without double-persisting.
            if !state.runtime.register(&id, backend.handle) {
                return serde_json::to_value(&session)
                    .map_err(|e| AppError::Internal(e.to_string()));
            }
            repo::session::set_launched_skill_ids(&state.db, &session.id, &skill_ids).await?;
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
    //
    // Only `chat` backends get a live context estimate: there we own the
    // provider loop, so streamed assistant text is a genuine (if rough) proxy
    // for the conversation. A CLI/PTY child streams terminal redraw bytes
    // (escape sequences, full-screen TUI repaints) that bear no relation to its
    // real context window — and tools like Claude Code track and display their
    // own context. Estimating from those bytes produced a meter that visibly
    // disagreed with the child's own `/context`, so we don't fabricate one.
    if let Some(output_rx) = output_rx {
        let track_context = def.r#type == "chat";
        tokio::spawn(forward_session_output(
            state.db.clone(),
            Arc::clone(&state.runtime),
            state.app().cloned(),
            id.clone(),
            session.id.clone(),
            output_rx,
            track_context,
        ));
    }

    serde_json::to_value(&session).map_err(|e| AppError::Internal(e.to_string()))
}

/// Remove a workspace_agent instance from its workspace.
///
/// Maps to `instance.remove` on the IPC bus. Tears down any live backend (kills
/// the PTY child / aborts the chat loop) BEFORE deleting the rows, then deletes
/// the instance and everything that hangs off it (see
/// `repo::workspace_agent::remove`). Idempotent-ish: a missing id is `NotFound`.
pub async fn remove(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: InstanceReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    let id = req.workspace_agent_id;

    if !repo::workspace_agent::exists(&state.db, &id).await? {
        return Err(AppError::NotFound(format!(
            "workspace_agent id={id} not found"
        )));
    }

    // Stop the live backend first so no PTY child / chat loop outlives the row.
    // The `#[must_use]` bool (did THIS call perform teardown, for idle-status
    // emission) is intentionally discarded: we're deleting the instance, so
    // there is no idle status to emit.
    let _ = state.runtime.unregister(&id);

    let removed = repo::workspace_agent::remove(&state.db, &id).await?;
    if !removed {
        return Err(AppError::NotFound(format!(
            "workspace_agent id={id} not found"
        )));
    }

    Ok(Value::Null)
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
///
/// `track_context` gates the live context estimate (and the auto-compact it
/// drives). It is `true` only for `chat` backends, whose streamed assistant
/// text is a genuine proxy for the conversation. For CLI/PTY backends it is
/// `false`: their output is terminal redraw noise, so we forward it to the bus
/// but never count it, persist a token estimate, or emit `session:context` —
/// the right-hand meter then stays hidden rather than showing a fabricated
/// figure that contradicts the child's own context display.
async fn forward_session_output(
    db: sqlx::SqlitePool,
    runtime: Arc<runtime::Runtime>,
    app: Option<tauri::AppHandle>,
    instance_id: String,
    session_id: String,
    mut output_rx: tokio::sync::mpsc::Receiver<String>,
    track_context: bool,
) {
    // Read the session's context limit once (default if unset or on error) — it
    // drives both the live meter denominator and the auto-compact threshold.
    // Only needed when we actually track context.
    let limit = if track_context {
        match repo::session::get(&db, &session_id).await {
            Ok(Some(s)) => s
                .context_limit
                .unwrap_or(repo::session::DEFAULT_CONTEXT_LIMIT),
            _ => repo::session::DEFAULT_CONTEXT_LIMIT,
        }
    } else {
        0
    };

    // Rolling ESTIMATE of context usage in characters. `last_flush_chars` is the
    // baseline at the previous persist, so we only write every FLUSH_CHARS.
    let mut total_chars: usize = 0;
    let mut last_flush_chars: usize = 0;

    while let Some(chunk) = output_rx.recv().await {
        // Count before moving `chunk` into the emit — avoids cloning every chunk
        // just to measure it. `chars().count()` (not `len()`) so multi-byte UTF-8
        // output isn't over-counted in the ≈4-chars/token estimate.
        if track_context {
            total_chars += chunk.chars().count();
        }

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
        if track_context && total_chars - last_flush_chars >= FLUSH_CHARS {
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

/// Payload for `session.resize` — the frontend terminal's current size.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResizeReq {
    session_id: String,
    cols: u16,
    rows: u16,
}

/// Resize a live CLI session's PTY to match the frontend xterm `(cols, rows)`.
///
/// Payload `{ sessionId, cols, rows }`. Resolves the session to its owning
/// instance and forwards the size to the PTY so a full-screen TUI lays out at
/// the real on-screen size. Best-effort: a no-op (`null`) when the session
/// isn't running or has no PTY (chat), or when the size is degenerate.
pub async fn resize(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: ResizeReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    // A collapsed/hidden pane can report 0×0 — ignore rather than shrink the PTY.
    if req.cols == 0 || req.rows == 0 {
        return Ok(Value::Null);
    }

    let session = repo::session::get(&state.db, &req.session_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("session id={} not found", req.session_id)))?;

    // Not-live just means there's no PTY to resize yet — best-effort.
    let _ = state
        .runtime
        .resize(&session.workspace_agent_id, req.cols, req.rows);

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
                ..Default::default()
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
            true, // chat backend — context tracking enabled.
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
            true, // chat backend — context tracking enabled.
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
            true, // chat backend — context tracking enabled.
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

    /// The CLI/PTY path (`track_context = false`) must NOT fabricate a context
    /// estimate, even when far more than FLUSH_CHARS of output streams through.
    /// A CLI child emits terminal redraw bytes that are meaningless as a token
    /// count, so `context_tokens` stays at its seeded value (`Some(0)`, never
    /// updated) and the meter is hidden by the chat-only UI gate. This is the
    /// regression guard for the "context ไม่ตรง" report: the byte-derived meter
    /// visibly disagreed with Claude Code's own `/context`.
    #[tokio::test]
    async fn forwarder_skips_context_estimate_when_untracked() {
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

        // Capture the seeded estimate so we can prove the forwarder leaves it
        // untouched (a fresh session seeds context_tokens to 0, not NULL).
        let before = session.context_tokens;

        let (tx, rx) = tokio::sync::mpsc::channel::<String>(8);
        let task = tokio::spawn(forward_session_output(
            state.db.clone(),
            Arc::clone(&state.runtime),
            None,
            id.clone(),
            session.id.clone(),
            rx,
            false, // CLI/PTY backend — context tracking disabled.
        ));

        // Well over FLUSH_CHARS: a tracked forwarder would persist ~250 tokens.
        tx.send("x".repeat(1_000)).await.expect("send chunk failed");
        drop(tx); // EOF → forwarder finishes.
        task.await.expect("forwarder task panicked");

        let after = repo::session::get(&state.db, &session.id)
            .await
            .expect("get failed")
            .expect("session exists");
        assert_eq!(
            after.context_tokens, before,
            "untracked (CLI) forwarder must leave the token estimate untouched \
             (before {before:?}, after {:?})",
            after.context_tokens
        );

        // No snapshot either — the auto-compact path is gated on the estimate.
        let snaps = repo::snapshot::list_for_session(&state.db, &session.id)
            .await
            .expect("list_for_session failed");
        assert!(
            snaps.is_empty(),
            "untracked forwarder must not auto-compact"
        );
    }

    #[tokio::test]
    async fn apply_skills_to_preamble_extends_preamble_when_attached() {
        let _fx = repo::skill::test_support::fixture_skills_dir("cmd-inst-preamble-attached");
        let state = AppState::for_tests().await;
        let ws = workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        let def = agent_definition::create(
            &state.db,
            &AgentDefinitionInput {
                name: "Atlas".into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed");
        let skill = repo::skill::create(&state.db, "Reviewer", None, "Always check X")
            .await
            .expect("create skill failed");
        let inst_id = workspace_agent::instantiate(&state.db, &ws.id, &def.id)
            .await
            .expect("instantiate failed")
            .id;
        repo::skill::set_custom_attachments(&state.db, &def.id, std::slice::from_ref(&skill.id))
            .await
            .expect("attach failed");

        let (result, skill_ids) =
            apply_skills_to_preamble(&state, &def.id, &inst_id, "BASE PREAMBLE".to_string())
                .await
                .expect("apply_skills_to_preamble failed");

        assert!(
            result.starts_with("BASE PREAMBLE "),
            "must extend, not replace, the base preamble: {result}"
        );
        assert!(!result.contains('\n'), "no newline: {result}");
        assert!(!result.contains('='), "no '=': {result}");
        assert!(
            skill_ids.contains(&"fix-mandatory".to_string()),
            "must include builtin fixture: {skill_ids:?}"
        );
        assert!(
            skill_ids.contains(&skill.id),
            "must include attached custom skill: {skill_ids:?}"
        );
    }

    #[tokio::test]
    async fn apply_skills_to_preamble_is_noop_when_nothing_attached() {
        let _fx = repo::skill::test_support::fixture_skills_dir("cmd-inst-preamble-noop");
        let state = AppState::for_tests().await;
        let ws = workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        let def = agent_definition::create(
            &state.db,
            &AgentDefinitionInput {
                name: "A".into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed");
        let inst_id = workspace_agent::instantiate(&state.db, &ws.id, &def.id)
            .await
            .expect("instantiate failed")
            .id;

        let (result, skill_ids) =
            apply_skills_to_preamble(&state, &def.id, &inst_id, "BASE".to_string())
                .await
                .expect("apply_skills_to_preamble failed");
        assert!(
            result.starts_with("BASE "),
            "builtin fixture skill always extends preamble: {result}"
        );
        assert!(
            skill_ids.contains(&"fix-mandatory".to_string()),
            "builtin fixture skill always included: {skill_ids:?}"
        );
    }

    /// The persist-only-on-success invariant this fix exists for: an
    /// orchestrator-type instance (safe, non-PTY dispatch path) still goes
    /// through `spawn`'s success path and registers live — confirming
    /// `spawn` doesn't blow up now that the cli branch's persist call moved.
    /// (The cli-specific "skip persist on failed spawn" behavior itself is
    /// inherently untestable without a real process, same boundary this
    /// file's other tests already respect — see `fixture_instance`'s doc
    /// comment.)
    #[tokio::test]
    async fn spawn_orchestrator_still_succeeds_after_skill_persist_reorder() {
        let state = AppState::for_tests().await;
        let id = fixture_instance(&state).await;
        let out = spawn(&state, json!({ "workspaceAgentId": id }))
            .await
            .expect("spawn failed");
        assert!(out.get("id").is_some());
        assert!(state.runtime.is_live(&id));
    }

    #[tokio::test]
    async fn list_annotates_launched_skill_ids() {
        let state = AppState::for_tests().await;
        let id = fixture_instance(&state).await; // orchestrator type — safe, no cli/chat dispatch
        let session = repo::session::get_by_instance(&state.db, &id)
            .await
            .expect("get failed")
            .expect("exists");
        repo::session::set_launched_skill_ids(&state.db, &session.id, &["sk-x".to_string()])
            .await
            .expect("set failed");

        let ws_id = workspace_agent::get(&state.db, &id)
            .await
            .expect("get failed")
            .expect("exists")
            .workspace_id;
        let listed = list(&state, json!({ "workspaceId": ws_id }))
            .await
            .expect("list failed");
        let item = listed
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == id)
            .unwrap();
        assert_eq!(
            item["launchedSkillIds"].as_array().map(|a| a.len()),
            Some(1)
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
