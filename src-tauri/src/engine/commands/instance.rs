use crate::engine::{bus, repo, runtime, AppError, AppState};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;
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

/// R-act-1: an instance reads as `working` iff its last recorded activity
/// (`Runtime::mark_activity`, stamped per streamed chunk) is within this
/// window. `Roster.tsx`'s `WORKING_WINDOW_MS` mirrors this value — keep them
/// in sync if it ever changes.
const WORKING_WINDOW: std::time::Duration = std::time::Duration::from_secs(5);

/// Persist + emit the context estimate once this many new chars accumulate
/// (≈100 tokens between writes).
const FLUSH_CHARS: usize = 400;

/// Auto-compact trigger as a whole percent of the context limit (≈90%).
const AUTO_COMPACT_PCT: i64 = 90;

/// Throttle transcript scans to keep CLI forwarders from walking the transcript
/// tree on every repaint.
const TRANSCRIPT_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

// ── Restart · resume constants ────────────────────────────────────────────────

/// Settle after the agent's handoff save before killing its process, so the
/// harness finishes rendering the "saved" confirmation (mirrors the compact
/// loop's `COMPACT_SETTLE_MS`).
const RESTART_SETTLE_MS: u64 = 2_000;

/// Delay between respawning the CLI and typing the resume prompt into it. A
/// fresh spawn boots a login+interactive shell (rc files) and then the TUI —
/// typing before the TUI reads stdin risks the prompt landing garbled. Generous
/// on purpose; a restart is a rare, human-triggered operation.
const RESTART_BOOT_SETTLE_MS: u64 = 8_000;

// Single-quote a value so the shell doesn't glob it; POSIX-escape embedded
// quotes so the value can't break out of the launch command.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn effective_claude_model(model: &str, context_window: Option<&str>) -> String {
    if context_window == Some("1m") {
        format!("{model}[1m]")
    } else {
        model.to_string()
    }
}

/// Emit codex's numeric context-window `-c` overrides for `model`, resolved
/// through [`crate::engine::codex_models::codex_model_context_window`] (plan
/// ruling R2 — "Auto" derives the value from the model, the Builder no
/// longer collects a manual number and any stored `context_window` value is
/// ignored at launch). Emits NOTHING for an unknown model so codex's own
/// default wins, matching the old sentinel/invalid-value behavior.
fn append_codex_context_window_config(launch: &mut String, model: Option<&str>) {
    let Some(tokens) =
        crate::engine::codex_models::codex_model_context_window(model.unwrap_or(""))
    else {
        return;
    };
    let auto_compact_token_limit = (i128::from(tokens) * 95 / 100) as i64;
    launch.push_str(&format!(
        " -c {}",
        shell_quote(&format!("model_context_window={tokens}"))
    ));
    launch.push_str(&format!(
        " -c {}",
        shell_quote(&format!(
            "model_auto_compact_token_limit={auto_compact_token_limit}"
        ))
    ));
}

/// Resolve the pre-detection fallback context limit for a freshly spawned
/// session (plan ruling R2b/R4): for codex agents, the per-model table wins
/// and any STORED `context_limit` is ignored ("auto" wins, matching the
/// launch-arg side); every other `cli_kind` keeps the previous
/// stored-value-then-default resolution untouched. The transcript-detected
/// window (`runtime::transcript_context`) still takes precedence over this
/// fallback once a real reading lands — this only seeds the meter before
/// that happens.
fn resolve_session_context_limit(cli_kind: &str, model: Option<&str>, stored: Option<i64>) -> i64 {
    if cli_kind == "codex" {
        return crate::engine::codex_models::codex_model_context_window(model.unwrap_or(""))
            .unwrap_or_else(|| repo::session::default_context_limit_for(cli_kind));
    }
    stored.unwrap_or_else(|| repo::session::default_context_limit_for(cli_kind))
}

/// The `ANTHROPIC_BASE_URL` override routing an agent through the loopback
/// context proxy. Claude agents default ON (`proxy_enabled` NULL/absent =
/// ON, rtk-parity — this unblocks Phase-1 measurement); codex agents default
/// OFF and unchanged, since `ctx_proxy` only rewrites the Anthropic
/// `/v1/messages` shape and would break the OpenAI-protocol codex harness.
/// Either default is overridable per-agent via `proxy_enabled`. Fail-open: a
/// down listener means a plain direct spawn, never a broken one.
fn proxy_env(
    proxy_enabled: Option<bool>,
    default_on: bool,
    active_port: Option<u16>,
) -> Option<(String, String)> {
    if !proxy_enabled.unwrap_or(default_on) {
        return None;
    }
    let port = active_port?;
    Some((
        "ANTHROPIC_BASE_URL".to_string(),
        format!("http://127.0.0.1:{port}"),
    ))
}

// ── Request types ────────────────────────────────────────────────────────────

/// Payload for `instance.list` — filter by workspace.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListInstancesReq {
    workspace_id: String,
}

/// Payload for `instance.spawn` / `instance.stop` / `instance.restart` —
/// targets a single instance. `self_triggered` (wire field `self`) is only
/// meaningful for `restart` (ADR 0006: the agent restarting itself vs. a
/// human triggering it) — absent/false for every other use, which is the
/// existing (human-triggered) behavior.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstanceReq {
    workspace_agent_id: String,
    #[serde(rename = "self", default)]
    self_triggered: bool,
}

#[derive(Debug)]
struct TranscriptPollContext {
    reader: runtime::transcript_context::TranscriptContextReader,
    workspace_folder: String,
    cli_kind: String,
    started_at: DateTime<Utc>,
}

impl TranscriptPollContext {
    fn new(
        workspace_folder: &str,
        cli_kind: &str,
        started_at: DateTime<Utc>,
        fallback_limit: i64,
    ) -> Self {
        Self {
            reader: runtime::transcript_context::TranscriptContextReader::new(
                runtime::transcript_context::TranscriptContextConfig::default_with_limit(
                    fallback_limit,
                ),
            ),
            workspace_folder: workspace_folder.to_owned(),
            cli_kind: cli_kind.to_owned(),
            started_at,
        }
    }

    #[cfg(test)]
    fn with_reader(
        reader: runtime::transcript_context::TranscriptContextReader,
        workspace_folder: &str,
        cli_kind: &str,
        started_at: DateTime<Utc>,
    ) -> Self {
        Self {
            reader,
            workspace_folder: workspace_folder.to_owned(),
            cli_kind: cli_kind.to_owned(),
            started_at,
        }
    }
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

    let mut rows =
        repo::workspace_agent::list_by_workspace_with_launched_skills(&state.db, &req.workspace_id)
            .await?;

    // Enrich live instances only (R-act-2: pull-model, no Rust timers) —
    // `working`/`lastActivityAt`/`sessionId` stay `None` for a dead instance,
    // which the repo layer already initialized.
    for row in &mut rows {
        if state.runtime.is_live(&row.id) {
            row.session_id = state.runtime.session_id(&row.id);
            row.working = Some(match state.runtime.last_activity(&row.id) {
                Some(last) => {
                    row.last_activity_at =
                        Some(chrono::DateTime::<chrono::Utc>::from(last).to_rfc3339());
                    last.elapsed().unwrap_or(std::time::Duration::MAX) <= WORKING_WINDOW
                }
                None => false,
            });
        }
    }

    serde_json::to_value(rows).map_err(|e| AppError::Internal(e.to_string()))
}

/// The fixed level vocabulary (spec position-system §2.2), used only to phrase
/// the `LevelInvalid` error; the enum itself is enforced atomically inside
/// [`repo::workspace_agent::set_position_validated`] (via `level_rank`).
const ALLOWED_LEVELS: [&str; 4] = ["junior", "mid", "senior", "principal"];

/// Look up one agent's enriched roster row in its workspace, or `NotFound`.
/// Used by [`set_position`] to return the post-write row.
async fn roster_row(
    state: &AppState,
    workspace_id: &str,
    workspace_agent_id: &str,
) -> Result<repo::workspace_agent::WorkspaceAgentWithSkills, AppError> {
    repo::workspace_agent::list_by_workspace_with_launched_skills(&state.db, workspace_id)
        .await?
        .into_iter()
        .find(|r| r.id == workspace_agent_id)
        .ok_or_else(|| {
            AppError::NotFound(format!("workspace agent id={workspace_agent_id} not found"))
        })
}

/// Parse one tri-state position field from the payload: key absent → `Keep`,
/// `null` → `Clear`, a string → `Set` (see the task's design note — the CLI's
/// `--x none`-to-clear + at-least-one-flag grammar makes absent-means-keep the
/// only consistent reading, ruled plan-conformant by the lead).
fn parse_position_field(
    obj: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<repo::workspace_agent::PositionField, AppError> {
    use repo::workspace_agent::PositionField;
    match obj.get(key) {
        None => Ok(PositionField::Keep),
        Some(Value::Null) => Ok(PositionField::Clear),
        Some(Value::String(s)) => Ok(PositionField::Set(s.clone())),
        Some(_) => Err(AppError::Invalid(format!(
            "instance.setPosition: {key} must be a string or null"
        ))),
    }
}

/// `instance.setPosition` (spec position-system §5.1, `workspaceId` REQUIRED per
/// ruling 71a00512) — set an instance's `level` and/or `supervisor`. The
/// read-validate-write (level enum + supervisor self/scope/cycle, §3.5) runs
/// atomically in [`repo::workspace_agent::set_position_validated`]'s single
/// `BEGIN IMMEDIATE` transaction (ruling ef969027), so racing writes cannot land
/// a cycle or clobber a partial update. Emits `roster:changed`, returns the
/// updated roster row.
pub async fn set_position(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let obj = payload.as_object().ok_or_else(|| {
        AppError::Invalid("instance.setPosition: expected an object payload".into())
    })?;
    let workspace_id = obj
        .get("workspaceId")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Invalid("instance.setPosition: workspaceId is required".into()))?;
    let workspace_agent_id = obj
        .get("workspaceAgentId")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            AppError::Invalid("instance.setPosition: workspaceAgentId is required".into())
        })?;

    let level = parse_position_field(obj, "level")?;
    let supervisor = parse_position_field(obj, "supervisorAgentId")?;

    use repo::workspace_agent::SetPositionError as E;
    repo::workspace_agent::set_position_validated(
        &state.db,
        workspace_id,
        workspace_agent_id,
        level,
        supervisor,
    )
    .await
    .map_err(|e| match e {
        E::AgentNotFound => {
            AppError::NotFound(format!("workspace agent id={workspace_agent_id} not found"))
        }
        E::WorkspaceMismatch => AppError::Invalid(format!(
            "workspace agent id={workspace_agent_id} is not in workspace {workspace_id}"
        )),
        E::LevelInvalid => AppError::Invalid(format!(
            "level must be one of: {}",
            ALLOWED_LEVELS.join(", ")
        )),
        E::SupervisorSelf => AppError::Invalid("an agent cannot supervise itself".into()),
        E::SupervisorNotFound => AppError::NotFound("supervisor not found".into()),
        E::SupervisorCrossWorkspace => {
            AppError::Invalid("supervisor must be in the same workspace as the agent".into())
        }
        E::Cycle => AppError::Invalid("supervisor link would create a cycle".into()),
        E::Db(err) => AppError::from(err),
    })?;

    state.emit(
        bus::ROSTER_CHANGED,
        bus::RosterChanged {
            workspace_id: workspace_id.to_owned(),
        },
    );

    let updated = roster_row(state, workspace_id, workspace_agent_id).await?;
    serde_json::to_value(updated).map_err(|e| AppError::Internal(e.to_string()))
}

/// Placeholder sidecar body for an instance with no builtin/custom skills
/// attached. The sidecar + pointer are unconditional (ADR 0004: the sidecar
/// is the LIVE source of truth for the instance's whole lifetime) — an
/// instance launched skill-less still needs a file a later `agent.save`
/// attachment can rewrite in place, and a pointer so the nudge can tell it
/// to re-read.
pub(crate) const NO_SKILLS_PLACEHOLDER: &str =
    "(no standing instructions attached right now — this file updates in place; re-read it when told to)";

/// Compute this instance's skill content (builtin + attached custom, via
/// `repo::skill::content_for_agent`) and unconditionally write it to a
/// per-instance sidecar file (falling back to `NO_SKILLS_PLACEHOLDER` when
/// empty) and append ONE sanitized pointer sentence to `preamble` — never the
/// raw content, which may contain '\n'/'=' and would violate
/// `bootstrap_preamble`'s single-line/'='-free contract (ADR 0001).
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
    let body = if skill_body.is_empty() {
        NO_SKILLS_PLACEHOLDER
    } else {
        skill_body.as_str()
    };
    let path = crate::engine::agentctx::write_skill_sidecar(instance_id, body)
        .map_err(|e| AppError::Internal(format!("write skill sidecar: {e}")))?;
    let preamble = format!(
        "{preamble} {}",
        crate::engine::agentctx::skill_pointer_sentence(&path)
    );
    Ok((preamble, skill_ids))
}

/// Recompute one agent definition's effective skill content ONCE and push it
/// to every instance of that def (ADR 0004: the sidecar is the LIVE source of
/// truth for the instance's whole lifetime, not just a launch-time snapshot).
///
/// Per instance: skip ONLY when NOTHING effective changed — the skill-id set
/// matches what's recorded as launched AND the freshly computed body equals
/// the sidecar file's CURRENT content (a missing/unreadable file always
/// counts as changed). Comparing ids alone is wrong: a `skill.save` content
/// edit on an already-attached skill keeps the id set identical, so an
/// id-only guard would silently skip the entire content-edit reload path —
/// the feature's main use case (caught in lead integration review
/// 2026-07-03; see `reload_skills_for_def_rewrites_on_content_only_edit`).
/// Otherwise rewrite the sidecar unconditionally; a LIVE instance
/// additionally gets the [`crate::engine::agentctx::skills_updated_prompt`]
/// nudge injected and its `session.launched_skill_ids` refreshed so the
/// staleness badge (ADR 0001) clears. A DEAD instance is rewritten only —
/// its next `spawn` recomputes `launched_skill_ids` fresh via
/// `apply_skills_to_preamble`, so touching it here would be redundant.
///
/// A single instance's failure (missing/corrupted session row, a sidecar
/// write error, a `set_launched_skill_ids` write error) is logged and
/// SKIPPED, never propagated — one broken/orphaned instance must not deny
/// the reload to every OTHER instance of the same def (Mellow's integration
/// review, item L7; see `reload_skills_for_def_continues_past_a_broken_instance`).
/// Only the two def-wide lookups before the loop (this def's effective
/// content, this def's instance list) are still fatal — there is nothing
/// per-instance left to try without them.
pub(crate) async fn reload_skills_for_def(
    state: &AppState,
    agent_def_id: &str,
) -> Result<(), AppError> {
    let (skill_body, skill_ids) = repo::skill::content_for_agent(&state.db, agent_def_id).await?;
    let body = if skill_body.is_empty() {
        NO_SKILLS_PLACEHOLDER
    } else {
        skill_body.as_str()
    };

    for instance in repo::workspace_agent::list_by_agent_def(&state.db, agent_def_id).await? {
        let session = match repo::session::get_by_instance(&state.db, &instance.id).await {
            Ok(Some(s)) => s,
            Ok(None) => {
                eprintln!(
                    "[skill] reload_skills_for_def: no session for workspace_agent id={} — skipping instance",
                    instance.id
                );
                continue;
            }
            Err(e) => {
                eprintln!(
                    "[skill] reload_skills_for_def: get_by_instance({}) failed: {e:?} — skipping instance",
                    instance.id
                );
                continue;
            }
        };

        let previous_ids: Vec<String> = session
            .launched_skill_ids
            .as_deref()
            .and_then(|t| serde_json::from_str(t).ok())
            .unwrap_or_default();
        // Mirrors `write_skill_sidecar`'s own path construction (agentctx.rs)
        // — duplicated rather than refactoring that reviewed function, per
        // this task's additive-only constraint on agentctx.rs.
        let sidecar_path = dirs::data_dir().map(|d| {
            d.join("Conclave")
                .join("skills")
                .join(format!("{}.md", instance.id))
        });
        let previous_body = sidecar_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok());
        if previous_ids == skill_ids && previous_body.as_deref() == Some(body) {
            continue;
        }

        let path = match crate::engine::agentctx::write_skill_sidecar(&instance.id, body) {
            Ok(p) => p,
            Err(e) => {
                eprintln!(
                    "[skill] reload_skills_for_def: write_skill_sidecar({}) failed: {e:?} — skipping instance",
                    instance.id
                );
                continue;
            }
        };

        if state.runtime.is_live(&instance.id) {
            let nudge = crate::engine::agentctx::skills_updated_prompt(&path);
            super::snapshot::submit_line(&state.runtime, &instance.id, &nudge).await;
            if let Err(e) =
                repo::session::set_launched_skill_ids(&state.db, &session.id, &skill_ids).await
            {
                eprintln!(
                    "[skill] reload_skills_for_def: set_launched_skill_ids({}) failed: {e:?}",
                    instance.id
                );
            }
        }
    }
    Ok(())
}

/// Detached tail for a skill mutation (`agent.save`, `skill.save`/`delete`):
/// resolves the shared [`AppState`] from the [`tauri::AppHandle`] (mirrors
/// `run_respawn_resume`'s pattern) rather than borrowing it, so the caller's
/// IPC handler can return immediately. Reloads each def SEQUENTIALLY, not
/// concurrently — a single skill can touch many defs across many workspaces,
/// and a live instance's PTY writes must stay orderly (Task 3 risk ledger).
/// Best-effort: a reload failure is logged UNCONDITIONALLY (mirrors
/// `run_respawn_resume`'s pattern), not propagated — the mutation that
/// triggered this already succeeded and returned to its caller, and a
/// debug-only log would make a failed live-reload invisible in a release
/// build (Mellow's integration review, item L4).
pub(crate) async fn run_reload_skills(app: tauri::AppHandle, agent_def_ids: Vec<String>) {
    use tauri::Manager;
    let state = Arc::clone(app.state::<Arc<AppState>>().inner());
    for agent_def_id in agent_def_ids {
        if let Err(e) = reload_skills_for_def(&state, &agent_def_id).await {
            eprintln!("[skill] reload_skills_for_def({agent_def_id}) failed: {e:?}");
        }
    }
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
            let cli_kind = def
                .cli_kind
                .as_deref()
                .ok_or_else(|| {
                    AppError::NotImplemented(
                        "custom CLI command is not configurable yet (M5 settings)".into(),
                    )
                })?
                .to_owned();

            // Build the launch command with the agent's configured flags. The
            // 1M-context variant of a Claude model is its id with a `[1m]`
            // suffix; Codex uses a numeric `model_context_window` config
            // override. Custom args apply to either harness.

            // Resolved BEFORE preamble assembly (not just before the PATH
            // export, as before Task 5) so a PATH-fallback sentence naming
            // this exact path can be appended to the preamble itself: the
            // launch shell's PATH export is only best-effort, since the
            // harness's own tool shells re-source rc files that frequently
            // RESET PATH instead of appending to it. The preamble (system-
            // prompt layer, present every turn) is the reliable channel.
            let conclave_bin = crate::engine::agentctx::ensure_conclave_shim();

            // Resolve the agent's first-class role (ADR 0005) for the preamble:
            // the role name drives the "a X agent" clause, its one-paragraph
            // description is baked verbatim so the agent knows its job before
            // its first roster query. Falls back to the legacy free-text `role`
            // label (which has no description) when no role_id is set.
            let resolved_role = match def.role_id.as_deref() {
                Some(rid) => crate::engine::repo::role::find_any(&state.db, rid).await?,
                None => None,
            };
            let role_name = resolved_role
                .as_ref()
                .map(|r| r.name.clone())
                .or_else(|| def.role.clone());
            let role_description = resolved_role.as_ref().map(|r| r.description.clone());

            // The agent's position (spec position-system §5.4): its own level +
            // supervisor NAME, resolved by the roster query (same resolution the
            // roster shows), so a fresh/restarted agent knows where it sits
            // before its first roster query. Absent → the preamble omits the
            // position clause (byte-identical to a pre-position-system launch).
            let position =
                repo::workspace_agent::list_by_workspace_with_launched_skills(&state.db, &ws.id)
                    .await?
                    .into_iter()
                    .find(|r| r.id == id);
            let level = position.as_ref().and_then(|r| r.level.clone());
            let supervisor_name = position.as_ref().and_then(|r| r.supervisor_name.clone());

            // Awareness briefing — injected via each harness's system-prompt layer
            // (NOT a chat turn), so it survives `/clear`. See engine::agentctx.
            let preamble = crate::engine::agentctx::bootstrap_preamble(
                &def.name,
                role_name.as_deref(),
                role_description.as_deref(),
                &ws.name,
                &ws.id,
                &id,
                level.as_deref(),
                supervisor_name.as_deref(),
            );

            let (preamble, skill_ids) =
                apply_skills_to_preamble(state, &def.id, &id, preamble).await?;

            let preamble = match &conclave_bin {
                Some(bin) => format!(
                    "{preamble} {}",
                    crate::engine::agentctx::conclave_path_sentence(&bin.join("conclave"))
                ),
                None => preamble,
            };

            // Sandbox socket allowance: poke exactly one hole so the agent's
            // sandboxed shell can reach the out-of-workspace conclave UDS
            // socket without a permission modal. Resolved at runtime from the
            // same `Conclave` data dir the server binds (never hardcoded), and
            // skipped in full-bypass mode (no sandbox to poke).
            let socket_path =
                if runtime::sandbox_config::needs_socket_hole(def.permission_mode.as_deref()) {
                    Some(
                        crate::engine::uds::socket_path()
                            .to_string_lossy()
                            .into_owned(),
                    )
                } else {
                    None
                };

            // Context-proxy routing: Claude agents default ON (rtk-parity),
            // codex agents default OFF (Anthropic-only proxy), decided via
            // `base == "claude"`. Decided ONCE here and threaded through both
            // the launch-branch sandbox allowlists and the env block below,
            // so the base-URL override and its loopback sandbox hole fire
            // together or not at all. `proxy_port` is Some only when the
            // injection fired.
            let proxy_env_var =
                proxy_env(def.proxy_enabled, base == "claude", state.ctx_proxy.active_port());
            let proxy_port = proxy_env_var.is_some().then(|| state.ctx_proxy.port);

            let mut launch = String::from(base);

            // Resolve the rtk PreToolUse hook (A5), SHARED by both CLI launch
            // branches — the human mandate is "make codex use rtk the way
            // claude-code does". Install only when the agent hasn't opted out
            // (`rtk_enabled` NULL/absent defaults to ON, per the DB column's
            // house style) AND both shim links the agent's Bash hook will
            // actually invoke are present on disk right now —
            // `ensure_conclave_shim` ran best-effort above and may have skipped
            // the `rtk` link entirely (dev run without the binary staged) or
            // linked a since-removed target. `is_usable_bin` applies the SAME
            // zero-size-counts-as-absent guard `resolve_rtk_bin` uses, so a
            // placeholder rtk build artifact never gets wired into a live hook.
            // Fail open: any gap here just means no PreToolUse hook, never a
            // blocked spawn. `base` is only ever "claude" or "codex" here (other
            // cli_kinds early-returned above), and each branch installs the hook
            // its own way — claude via the per-instance settings JSON, codex via
            // a per-spawn `-c` override.
            let rtk_on = def.rtk_enabled.unwrap_or(true);
            let rtk_hook = conclave_bin.as_ref().filter(|_| rtk_on).and_then(|bin| {
                let cli_bin = bin.join("conclave");
                let rtk_bin = bin.join("rtk");
                if cli_bin.is_file() && crate::engine::agentctx::is_usable_bin(&rtk_bin) {
                    Some(runtime::sandbox_config::RtkHook { cli_bin, rtk_bin })
                } else {
                    None
                }
            });

            // Awareness sentence appended ONLY when the hook was actually
            // installed (same append-when-installed style as the conclave path
            // sentence above) — an agent whose rtk hook never got wired has
            // nothing to be warned about. Both claude and codex install the hook
            // when `rtk_hook` is Some, so the sentence is generic and shared.
            let preamble = match &rtk_hook {
                Some(_) => format!(
                    "{preamble} {}",
                    crate::engine::agentctx::rtk_awareness_sentence()
                ),
                None => preamble,
            };

            if base == "claude" {
                if let Some(mode) = def.permission_mode.as_deref() {
                    // Validated to an allowlist at save time, but quote anyway so a
                    // future bypass can't inject a second shell command here.
                    launch.push_str(&format!(" --permission-mode {}", shell_quote(mode)));
                }
                if let Some(model) = def.model.as_deref().filter(|m| !m.is_empty()) {
                    let eff = effective_claude_model(model, def.context_window.as_deref());
                    launch.push_str(&format!(" --model {}", shell_quote(&eff)));
                }
                // Persistent system-prompt append → survives /clear.
                launch.push_str(&format!(
                    " --append-system-prompt {}",
                    shell_quote(&preamble)
                ));
                // Per-instance settings file, written on EVERY claude spawn:
                // it always carries the SessionStart owner-marker hook (the
                // transcript-recorded channel the context meter attributes
                // transcripts with — the system-prompt append above is never
                // written to the transcript), plus the sandbox socket
                // allowance when the spawn runs sandboxed (Route A — keeps
                // conclave inside the sandbox, opens only the one IPC socket,
                // and auto-approves the sandboxed call), plus the rtk
                // PreToolUse hook resolved above. Fail-soft: on a write
                // error the agent still works, just without the transcript
                // meter and with the one-time seatbelt modal.
                match runtime::sandbox_config::write_claude_settings(
                    &id,
                    socket_path.as_deref(),
                    rtk_hook.as_ref(),
                    proxy_port,
                ) {
                    Ok(path) => launch.push_str(&format!(
                        " --settings {}",
                        shell_quote(&path.to_string_lossy())
                    )),
                    Err(e) => {
                        eprintln!("[spawn] could not write claude agent settings for {id}: {e}")
                    }
                }
            } else if base == "codex" {
                if let Some(model) = def.model.as_deref().filter(|m| !m.is_empty()) {
                    launch.push_str(&format!(" --model {}", shell_quote(model)));
                }
                append_codex_context_window_config(&mut launch, def.model.as_deref());
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
                // Sandbox: same socket allowlist as claude, via per-spawn `-c`
                // overrides ([permissions.conclave] profile proven in Guetta's
                // research, test J). Never writes the user's ~/.codex/config.toml.
                if let Some(sock) = &socket_path {
                    for ov in runtime::sandbox_config::codex_socket_overrides(sock, proxy_port) {
                        launch.push_str(&format!(" -c {}", shell_quote(&ov)));
                    }
                }
                // rtk PreToolUse hook (Lane K): the codex analogue of claude's
                // per-instance settings hook. One `-c` flag carries the whole
                // hook table inline; `--dangerously-bypass-hook-trust` is
                // MANDATORY or the injected hook SILENTLY never fires (codex-cli
                // 0.144.1 — a `-c`-injected hook isn't in the persisted trust
                // store; verified live in Guetta's research). Same double
                // opt-in gate as claude (`rtk_hook` is Some only when
                // rtk_enabled and both bins resolve), so this too is fail-open.
                // Never writes the user's ~/.codex/config.toml.
                if let Some(rtk) = &rtk_hook {
                    launch.push_str(&format!(
                        " -c {}",
                        shell_quote(&runtime::sandbox_config::codex_rtk_hook_override(rtk))
                    ));
                    launch.push_str(" --dangerously-bypass-hook-trust");
                }
            }
            if let Some(extra) = def.custom_args.as_deref().filter(|s| !s.trim().is_empty()) {
                launch.push(' ');
                launch.push_str(extra.trim());
            }

            // Put `conclave` on the agent's PATH so the briefing's commands
            // resolve. The login+interactive shell sources its rc files BEFORE
            // running this `-c` command, so prepending the export here wins over
            // whatever PATH the rc files set. Best-effort: if the CLI binary isn't
            // found beside the app, skip it and launch without `conclave` (the
            // preamble then carries no PATH-fallback sentence either, since
            // there is no path to point at).
            if let Some(bin) = &conclave_bin {
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
            // Context-proxy base URL, pushed LAST so it wins over any
            // `custom_env` value of the same name (spec D8; the decision was
            // made above, beside the sandbox allowlist threading).
            if let Some(kv) = proxy_env_var {
                extra_env.push(kv);
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
            let Some(epoch) = state.runtime.register(&id, backend.handle) else {
                return serde_json::to_value(&session)
                    .map_err(|e| AppError::Internal(e.to_string()));
            };
            repo::session::set_launched_skill_ids(&state.db, &session.id, &skill_ids).await?;
            let started_at = DateTime::parse_from_rfc3339(&session.started_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            let transcript_ctx = TranscriptPollContext::new(
                &ws.folder_path,
                &cli_kind,
                started_at,
                resolve_session_context_limit(
                    &cli_kind,
                    def.model.as_deref(),
                    session.context_limit,
                ),
            );
            Some((backend.output_rx, epoch, Some(transcript_ctx)))
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
            let Some(epoch) = state.runtime.register(&id, backend.handle) else {
                return serde_json::to_value(&session)
                    .map_err(|e| AppError::Internal(e.to_string()));
            };
            Some((backend.output_rx, epoch, None))
        }
        _ => {
            // orchestrator / unknown: placeholder backend (fusion arrives in M4).
            if state
                .runtime
                .register(&id, runtime::LiveHandle::placeholder(&session.id))
                .is_none()
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

    // Detached forwarder: bridge output → bus, and mark the instance idle when
    // the child self-terminates (EOF closes output_rx).
    //
    // `chat` backends still use the byte-estimate path because we own the
    // provider loop. `cli` backends now use the transcript reader instead, so
    // the meter follows the harness transcripts rather than terminal redraws.
    if let Some((output_rx, epoch, transcript_ctx)) = output_rx {
        let track_context = def.r#type == "chat";
        tokio::spawn(forward_session_output(
            state.db.clone(),
            Arc::clone(&state.runtime),
            state.app().cloned(),
            id.clone(),
            session.id.clone(),
            def.cli_kind.clone(),
            def.model.clone(),
            output_rx,
            track_context,
            transcript_ctx,
            epoch,
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

    // D4b: the agent is being removed — flip its browser tab to `ended` BEFORE
    // its row is deleted. The tab stays as a read-only view of the final page
    // until the human closes it (browser.rs:463 contract). No-op for a tab-less
    // agent; the frontend's 2s `browser.status` poll repaints the badge.
    runtime::browser::mark_ended(&id);

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
/// `track_context` gates the live byte-estimate path and its auto-compact. It
/// is `true` only for `chat` backends, whose streamed assistant text is a
/// genuine proxy for the conversation. For CLI/PTY backends it is `false` and
/// `transcript_ctx` carries the transcript-backed meter reader instead: PTY
/// output still forwards as activity, but the context count comes from the
/// harness transcripts, not the redraw bytes.
///
/// `epoch` is this forwarder's registration generation (from `register`): the
/// EOF cleanup uses `unregister_epoch` so a LATE EOF — after a programmatic
/// kill → respawn reused the same instance id (see [`restart`]) — cannot tear
/// down the new generation's backend.
#[allow(clippy::too_many_arguments)]
async fn forward_session_output(
    db: sqlx::SqlitePool,
    runtime: Arc<runtime::Runtime>,
    app: Option<tauri::AppHandle>,
    instance_id: String,
    session_id: String,
    cli_kind: Option<String>,
    model: Option<String>,
    mut output_rx: tokio::sync::mpsc::Receiver<String>,
    track_context: bool,
    transcript_ctx: Option<TranscriptPollContext>,
    epoch: u64,
) {
    let session_row = repo::session::get(&db, &session_id).await.ok().flatten();
    // Same resolution as the spawn-time transcript_ctx fallback (R2b/R4): for
    // codex this seeds `TranscriptMeterState.limit` below with the per-model
    // table value, ignoring any stored session.context_limit, before the
    // transcript reader's first real poll lands.
    let limit = resolve_session_context_limit(
        cli_kind.as_deref().unwrap_or(""),
        model.as_deref(),
        session_row.as_ref().and_then(|s| s.context_limit),
    );
    if track_context {
        // Rolling ESTIMATE of context usage in characters. `last_flush_chars`
        // is the baseline at the previous persist, so we only write every
        // FLUSH_CHARS.
        let mut total_chars: usize = 0;
        let mut last_flush_chars: usize = 0;

        while let Some(chunk) = output_rx.recv().await {
            // Stamp activity (R-act-1 + bb plan:working-false-positive).
            // `chat` backends have no PTY, so there is no terminal echo to
            // suppress — every streamed delta is genuine assistant output,
            // stamped unconditionally. CLI/PTY backends go through the gated
            // variant: a chunk arriving inside the echo-suppression horizon
            // armed by our own `send_stdin`/`resize` (mount-jiggle repaint,
            // wheel-scroll arrow-key echo, keystroke echo — proven empirically
            // in `runtime::pty::tests::idle_claude_repaints_on_resize_jiggle`)
            // is dropped without extending the working window.
            runtime.mark_activity(&instance_id);
            let activity = true;

            // Count before moving `chunk` into the emit — avoids cloning every
            // chunk just to measure it. `chars().count()` (not `len()`) so
            // multi-byte UTF-8 output isn't over-counted in the
            // ≈4-chars/token estimate.
            total_chars += chunk.chars().count();

            if let Some(app) = &app {
                let _ = bus::session_output(
                    app,
                    bus::SessionOutput {
                        session_id: session_id.clone(),
                        chunk,
                        activity,
                    },
                );
            }

            // Flush the estimate roughly every ~100 tokens of new output.
            if total_chars - last_flush_chars >= FLUSH_CHARS {
                let compacted =
                    flush_context_estimate(&db, app.as_ref(), &session_id, total_chars, limit)
                        .await;
                last_flush_chars = total_chars;
                if compacted {
                    // Auto-compact boundary: model the post-compaction window
                    // by resetting the estimate baseline so the meter re-arms
                    // and won't re-fire until it fills again. This is an
                    // ESTIMATE-based compaction boundary — real summary
                    // carry-forward is deferred to M4.2 (the snapshot's
                    // carried_forward stays NULL).
                    total_chars = 0;
                    last_flush_chars = 0;
                }
            }
        }
    } else {
        let Some(transcript_ctx) = transcript_ctx else {
            // No meter path to run; just drain the output and clean up below.
            while let Some(chunk) = output_rx.recv().await {
                let activity = runtime.mark_activity_gated(&instance_id);
                if let Some(app) = &app {
                    let _ = bus::session_output(
                        app,
                        bus::SessionOutput {
                            session_id: session_id.clone(),
                            chunk,
                            activity,
                        },
                    );
                }
            }
            if runtime.unregister_epoch(&instance_id, epoch) {
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
                // D4b: the owning agent self-terminated (EOF). Inside the epoch
                // guard so a LATE EOF from a superseded (restarted) generation
                // returns false above and does NOT mark it. App-less + no emit:
                // the frontend's 2s `browser.status` poll repaints the badge.
                runtime::browser::mark_ended(&instance_id);
            }
            return;
        };

        let mut meter = TranscriptMeterState {
            tokens: session_row
                .as_ref()
                .and_then(|s| s.context_tokens)
                .unwrap_or(0),
            limit,
            last_poll: None,
        };
        let mut poll_timer = tokio::time::interval(TRANSCRIPT_POLL_INTERVAL);
        poll_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        poll_timer.tick().await;

        loop {
            tokio::select! {
                maybe_chunk = output_rx.recv() => {
                    let Some(chunk) = maybe_chunk else { break; };
                    let activity = runtime.mark_activity_gated(&instance_id);
                    if let Some(app) = &app {
                        let _ = bus::session_output(
                            app,
                            bus::SessionOutput {
                                session_id: session_id.clone(),
                                chunk,
                                activity,
                            },
                        );
                    }
                    poll_transcript_context(
                        &db,
                        app.as_ref(),
                        &instance_id,
                        &session_id,
                        &transcript_ctx,
                        &mut meter,
                        false,
                    )
                    .await;
                }
                _ = poll_timer.tick() => {
                    poll_transcript_context(
                        &db,
                        app.as_ref(),
                        &instance_id,
                        &session_id,
                        &transcript_ctx,
                        &mut meter,
                        false,
                    )
                    .await;
                }
            }
        }

        poll_transcript_context(
            &db,
            app.as_ref(),
            &instance_id,
            &session_id,
            &transcript_ctx,
            &mut meter,
            true,
        )
        .await;
    }

    // Child exited / EOF. Idempotent self-termination cleanup — epoch-guarded so
    // a late EOF after a restart respawn can't kill the new generation.
    if runtime.unregister_epoch(&instance_id, epoch) {
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
        // D4b: terminal EOF for the owning agent. This shared tail is reached by
        // BOTH the chat/`track_context` branch and the transcript sub-branch, so
        // it covers every non-drain EOF. Epoch-guarded (a superseded late EOF
        // returns false above). App-less; the frontend's 2s poll repaints.
        runtime::browser::mark_ended(&instance_id);
    }
}

/// Mutable meter state threaded through transcript polls: the last persisted
/// reading plus the poll-interval clock.
struct TranscriptMeterState {
    tokens: i64,
    limit: i64,
    last_poll: Option<std::time::Instant>,
}

/// Poll the transcript reader for the CLI transcript-backed meter and persist
/// any newer reading.
async fn poll_transcript_context(
    db: &sqlx::SqlitePool,
    app: Option<&tauri::AppHandle>,
    instance_id: &str,
    session_id: &str,
    transcript_ctx: &TranscriptPollContext,
    meter: &mut TranscriptMeterState,
    force: bool,
) {
    if !force {
        if let Some(last) = meter.last_poll.as_ref() {
            if last.elapsed() < TRANSCRIPT_POLL_INTERVAL {
                return;
            }
        }
    }

    // The reader walks the transcript tree and parses JSONL — synchronous,
    // file-bound work. Run it on the blocking pool so it never stalls the async
    // worker that also pumps PTY output and forwards keystrokes; blocking that
    // worker is what froze the terminal (no input, delayed output) every poll.
    let reader = transcript_ctx.reader.clone();
    let instance_id_owned = instance_id.to_owned();
    let workspace_folder = transcript_ctx.workspace_folder.clone();
    let cli_kind = transcript_ctx.cli_kind.clone();
    let started_at = transcript_ctx.started_at;
    let reading = tokio::task::spawn_blocking(move || {
        reader.poll(
            &instance_id_owned,
            Path::new(&workspace_folder),
            &cli_kind,
            started_at,
        )
    })
    .await
    .ok()
    .flatten();
    let Some(reading) = reading else {
        meter.last_poll = Some(std::time::Instant::now());
        return;
    };

    let changed = reading.tokens != meter.tokens || reading.limit != meter.limit;
    meter.tokens = reading.tokens;
    meter.limit = reading.limit;
    if !changed {
        meter.last_poll = Some(std::time::Instant::now());
        return;
    }

    let _ = repo::session::set_context_reading(db, session_id, reading.tokens, reading.limit).await;
    if let Some(app) = app {
        let _ = bus::session_context(
            app,
            bus::SessionContext {
                session_id: session_id.to_owned(),
                context_tokens: reading.tokens,
                context_limit: reading.limit,
                estimated: true,
            },
        );
    }

    #[cfg(debug_assertions)]
    eprintln!(
        "[transcript] {instance_id} -> {} tokens from {} at {}",
        reading.tokens, reading.source_kind, reading.observed_at
    );

    meter.last_poll = Some(std::time::Instant::now());
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

    // D4b: `stop` won the teardown race (unregister returned true) — flip the
    // agent's browser tab to `ended` (read-only until the human closes it). A
    // late EOF for this id finds a bumped epoch and skips its own mark, so this
    // is the sole marker. No-op for a tab-less agent; the 2s poll repaints.
    runtime::browser::mark_ended(&id);

    Ok(Value::Null)
}

/// Restart a CLI agent's process and resume it from a saved handoff.
///
/// Maps to `instance.restart` on the IPC bus. Two paths:
///
/// - **Live agent** (the normal case): arm a restart
///   ([`AppState::mark_restart_pending`]) and inject the "save your handoff"
///   prompt. The kill → respawn → resume tail fires from the `snapshot.save`
///   handler once the agent has actually persisted its handoff — the same
///   save-gated ordering as the compact loop, so a restart can never destroy
///   uncaptured context. An agent that ignores the prompt is simply never
///   restarted (the arm expires via TTL).
/// - **Not live**: nothing to save — respawn immediately and, if the session
///   has a handoff snapshot, inject the resume prompt once the CLI has booted.
///
/// Returns `{ status: "restarting", phase: "saving" | "respawning" }`.
/// CLI agents only: chat/orchestrator have no PTY process or handoff loop.
pub async fn restart(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: InstanceReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    let id = req.workspace_agent_id;

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
    if def.r#type != "cli" {
        return Err(AppError::Invalid(
            "restart · resume applies to CLI agents only".into(),
        ));
    }

    if state.runtime.is_live(&id) {
        // Save-gated: arm FIRST, then inject — a fast agent that saves the
        // instant it reads the prompt must find the arm set (mirrors compact).
        state.mark_restart_pending(&id);

        if req.self_triggered {
            // ADR 0006: the caller IS the agent, mid-turn — it already knows
            // a restart is coming (it triggered this itself), so injecting a
            // prompt into its own TUI would interleave with its own output.
            // Return the instruction as plain command output instead; the
            // save-gated tail (snapshot.rs::save → take_restart_pending →
            // run_respawn_resume) is unchanged and fires exactly as before
            // once the agent's `conclave snapshot save` lands.
            return Ok(serde_json::json!({
                "status": "restarting",
                "phase": "saving",
                "instanceId": id,
                "instruction": crate::engine::agentctx::self_restart_instruction(
                    state.restart_pending_ttl()
                ),
            }));
        }

        super::snapshot::submit_line(
            &state.runtime,
            &id,
            &crate::engine::agentctx::restart_save_prompt(),
        )
        .await;
        return Ok(serde_json::json!({
            "status": "restarting", "phase": "saving", "instanceId": id
        }));
    }

    // Not live: respawn now; the tail resolves the app-managed state itself.
    let Some(app) = state.app().cloned() else {
        return Err(AppError::Internal(
            "restart requires the app runtime (no AppHandle set)".into(),
        ));
    };
    tauri::async_runtime::spawn(run_respawn_resume(app, id.clone(), false));
    Ok(serde_json::json!({
        "status": "restarting", "phase": "respawning", "instanceId": id
    }))
}

/// The restart tail: (optionally) kill the live backend, respawn it, and — if
/// the session has a handoff to come back to — inject the resume prompt once
/// the CLI has booted. Runs as a detached task (spawned by [`restart`] for a
/// dead agent, or by the `snapshot.save` handler once a restart-armed agent has
/// persisted its handoff), so it resolves the shared [`AppState`] from the
/// `AppHandle` rather than borrowing it.
///
/// The kill → immediate respawn on the same instance id is safe because the old
/// forwarder's late EOF cleanup is epoch-guarded (`unregister_epoch`) — it can
/// no longer tear down the new generation's backend.
pub(crate) async fn run_respawn_resume(
    app: tauri::AppHandle,
    instance_id: String,
    kill_first: bool,
) {
    use tauri::Manager;
    let state = Arc::clone(app.state::<Arc<AppState>>().inner());

    if kill_first {
        // Let the agent render its "saved" confirmation, then kill its process.
        tokio::time::sleep(std::time::Duration::from_millis(RESTART_SETTLE_MS)).await;
        if state.runtime.unregister(&instance_id) {
            // Mirror `stop`: persist + emit idle so the UI sees the transition.
            let _ = repo::workspace_agent::set_status(&state.db, &instance_id, "idle").await;
            if let Ok(Some(session)) = repo::session::get_by_instance(&state.db, &instance_id).await
            {
                state.emit(
                    bus::SESSION_STATUS,
                    bus::SessionStatus {
                        session_id: session.id,
                        status: "idle".into(),
                    },
                );
            }
        }
    }

    // Decide the resume injection BEFORE respawning: does a handoff exist?
    let has_handoff = match repo::session::get_by_instance(&state.db, &instance_id).await {
        Ok(Some(s)) => repo::snapshot::latest_handoff_for_session(&state.db, &s.id)
            .await
            .ok()
            .flatten()
            .is_some(),
        _ => false,
    };

    if let Err(e) = spawn(
        &state,
        serde_json::json!({ "workspaceAgentId": instance_id }),
    )
    .await
    {
        eprintln!("restart: respawn failed for instance {instance_id}: {e}");
        return;
    }

    // Fresh start with nothing to resume from — done after the respawn.
    if !has_handoff {
        return;
    }

    // Give the CLI time to boot before typing the resume prompt into its TUI.
    tokio::time::sleep(std::time::Duration::from_millis(RESTART_BOOT_SETTLE_MS)).await;
    if !state.runtime.is_live(&instance_id) {
        return; // died during boot — nothing to resume.
    }
    super::snapshot::submit_line(
        &state.runtime,
        &instance_id,
        &crate::engine::agentctx::resume_restore_prompt(),
    )
    .await;
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
    use chrono::{DateTime, Utc};
    use serde_json::json;
    use std::path::PathBuf;
    use uuid::Uuid;

    #[test]
    fn codex_context_window_config_appends_table_override_for_known_model() {
        let mut launch = String::from("codex --model 'gpt-5.3-codex-spark'");
        append_codex_context_window_config(&mut launch, Some("gpt-5.3-codex-spark"));

        // 128_000 * 95 / 100 = 121_600.
        assert!(
            launch.contains(" -c 'model_context_window=128000'"),
            "{launch}"
        );
        assert!(
            launch.contains(" -c 'model_auto_compact_token_limit=121600'"),
            "{launch}"
        );
    }

    #[test]
    fn codex_context_window_config_emits_nothing_for_unknown_model() {
        for model in [Some("some-future-model"), Some(""), None] {
            let mut launch = String::from("codex");
            append_codex_context_window_config(&mut launch, model);
            assert!(
                !launch.contains("model_context_window"),
                "unknown/absent model must not become a Codex context override: {launch}"
            );
            assert!(
                !launch.contains("model_auto_compact_token_limit"),
                "unknown/absent model must not become a Codex auto-compact override: {launch}"
            );
        }
    }

    #[test]
    fn resolve_session_context_limit_codex_uses_table_and_ignores_stored() {
        // Known model: table value wins even when a stale stored value is
        // present (R4 — stored codex context_limit is ignored at launch).
        assert_eq!(
            resolve_session_context_limit("codex", Some("gpt-5.4"), Some(999)),
            1_050_000
        );
        // Unknown model: falls back to the conservative codex default, still
        // ignoring the stored value.
        assert_eq!(
            resolve_session_context_limit("codex", Some("some-future-model"), Some(999)),
            repo::session::default_context_limit_for("codex")
        );
        // No model at all: same conservative fallback.
        assert_eq!(
            resolve_session_context_limit("codex", None, Some(999)),
            repo::session::default_context_limit_for("codex")
        );
    }

    #[test]
    fn resolve_session_context_limit_non_codex_keeps_stored_value() {
        assert_eq!(
            resolve_session_context_limit("claude-code", Some("gpt-5.4"), Some(42)),
            42
        );
        assert_eq!(
            resolve_session_context_limit("claude-code", None, None),
            repo::session::default_context_limit_for("claude-code")
        );
    }

    #[test]
    fn codex_context_window_config_ignores_stored_context_window_entirely() {
        // R2/R4: the function no longer takes a stored `context_window` value
        // at all — only the model resolves the override, proving the old
        // stored numeric/sentinel value ("400000", "1m", "200k", ...) can no
        // longer influence codex's launch args.
        let mut launch = String::from("codex --model 'gpt-5.4'");
        append_codex_context_window_config(&mut launch, Some("gpt-5.4"));
        assert!(
            launch.contains(" -c 'model_context_window=1050000'"),
            "{launch}"
        );
    }

    /// Env injection for the context proxy: Claude agents default ON
    /// (rtk-parity), codex agents default OFF (Anthropic-only proxy would
    /// break the OpenAI-protocol harness) — either default overridable
    /// per-agent via `proxy_enabled`. Fail-open when the listener is down.
    #[test]
    fn proxy_env_defaults_on_for_claude_off_for_codex() {
        // Claude default ON: NULL proxy_enabled + default_on=true → injected.
        assert_eq!(
            proxy_env(None, true, Some(18787)),
            Some((
                "ANTHROPIC_BASE_URL".to_string(),
                "http://127.0.0.1:18787".to_string()
            ))
        );
        // Codex default OFF: NULL proxy_enabled + default_on=false → never.
        assert_eq!(proxy_env(None, false, Some(18787)), None);
        // Explicit opt-OUT wins for Claude even with default_on=true.
        assert_eq!(proxy_env(Some(false), true, Some(18787)), None);
        // Explicit opt-IN wins for codex even with default_on=false.
        assert_eq!(
            proxy_env(Some(true), false, Some(18787)),
            Some((
                "ANTHROPIC_BASE_URL".to_string(),
                "http://127.0.0.1:18787".to_string()
            ))
        );
        // Opted in but listener down → no injection (fail-open).
        assert_eq!(proxy_env(Some(true), true, None), None);
    }

    #[test]
    fn claude_context_window_suffix_stays_claude_only() {
        let eff = effective_claude_model("claude-sonnet-5", Some("1m"));
        let launch = format!("claude --model {}", shell_quote(&eff));

        assert!(launch.contains("'claude-sonnet-5[1m]'"), "{launch}");
        assert!(!launch.contains("model_context_window"), "{launch}");
        assert!(
            !launch.contains("model_auto_compact_token_limit"),
            "{launch}"
        );
    }

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

    fn temp_root(prefix: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("{}-{}", prefix, Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create temp root");
        root
    }

    fn write_jsonl(path: &std::path::Path, lines: &[serde_json::Value]) {
        let mut body = String::new();
        for (idx, line) in lines.iter().enumerate() {
            if idx > 0 {
                body.push('\n');
            }
            body.push_str(&line.to_string());
        }
        std::fs::write(path, body).expect("write jsonl");
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
        let epoch = state
            .runtime
            .register(&id, runtime::LiveHandle::placeholder(&session.id))
            .expect("register");
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
            None,
            None,
            rx,
            true, // chat backend — context tracking enabled.
            None,
            epoch,
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

        let epoch = state
            .runtime
            .register(&id, runtime::LiveHandle::placeholder(&session.id))
            .expect("register");
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
            None,
            None,
            rx,
            true, // chat backend — context tracking enabled.
            None,
            epoch,
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

        let epoch = state
            .runtime
            .register(&id, runtime::LiveHandle::placeholder(&session.id))
            .expect("register");
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
            None,
            None,
            rx,
            true, // chat backend — context tracking enabled.
            None,
            epoch,
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

        let epoch = state
            .runtime
            .register(&id, runtime::LiveHandle::placeholder(&session.id))
            .expect("register");
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
            None,
            None,
            rx,
            false, // CLI/PTY backend — context tracking disabled.
            None,
            epoch,
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

    /// CLI output must be able to trigger a transcript-backed context refresh
    /// through the forwarder, without using the byte-estimate path.
    #[tokio::test]
    async fn forwarder_updates_context_from_transcript_reader_for_cli() {
        let state = AppState::for_tests().await;
        let id = fixture_instance_typed(&state, "cli", Some("codex")).await;
        let session = repo::session::get_by_instance(&state.db, &id)
            .await
            .expect("get_by_instance failed")
            .expect("session exists");
        let ws = workspace_agent::get(&state.db, &id)
            .await
            .expect("get failed")
            .expect("workspace agent exists");
        let workspace_row = workspace::get(&state.db, &ws.workspace_id)
            .await
            .expect("workspace get failed")
            .expect("workspace exists");

        let claude_root = temp_root("transcript-forwarder-claude");
        let codex_root = temp_root("transcript-forwarder-codex");
        let codex_file = codex_root.join("2026/07/08/rollout.jsonl");
        std::fs::create_dir_all(codex_file.parent().expect("codex parent dir"))
            .expect("create codex dir");
        write_jsonl(
            &codex_file,
            &[
                serde_json::json!({
                    "type": "session_meta",
                    "timestamp": "2099-01-01T00:00:00Z",
                    "payload": {
                        "cwd": workspace_row.folder_path.clone(),
                        "id": "codex-session-id-not-conclave-instance-id",
                        "originator": "codex-tui"
                    }
                }),
                serde_json::json!({
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "developer",
                        "content": [
                            {
                                "type": "input_text",
                                "text": format!("You are a Conclave agent, and your own agent id is {id}.")
                            }
                        ]
                    }
                }),
                serde_json::json!({
                    "type": "event_msg",
                    "timestamp": "2099-01-01T00:00:01Z",
                    "payload": {
                        "type": "token_count",
                        "info": {
                            "last_token_usage": { "total_tokens": 321 },
                            "total_token_usage": { "total_tokens": 9_999 },
                            "model_context_window": 8_000
                        }
                    }
                }),
            ],
        );

        let started_at = DateTime::parse_from_rfc3339(&session.started_at)
            .expect("session started_at must parse")
            .with_timezone(&Utc);
        let transcript_ctx = TranscriptPollContext::with_reader(
            runtime::transcript_context::TranscriptContextReader::new(
                runtime::transcript_context::TranscriptContextConfig {
                    claude_projects_root: claude_root.clone(),
                    codex_sessions_root: codex_root.clone(),
                    fallback_limit: session
                        .context_limit
                        .unwrap_or_else(|| repo::session::default_context_limit_for("codex")),
                },
            ),
            &workspace_row.folder_path,
            "codex",
            started_at,
        );

        let epoch = state
            .runtime
            .register(&id, runtime::LiveHandle::placeholder(&session.id))
            .expect("register");
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
            Some("codex".into()),
            None,
            rx,
            false, // CLI/PTY backend — transcript-backed context enabled.
            Some(transcript_ctx),
            epoch,
        ));

        tx.send("prompt".to_string())
            .await
            .expect("send chunk failed");
        drop(tx);
        task.await.expect("forwarder task panicked");

        let after = repo::session::get(&state.db, &session.id)
            .await
            .expect("get failed")
            .expect("session exists");
        assert_eq!(
            after.context_tokens,
            Some(321),
            "CLI transcript reading must update the session context"
        );
        assert_eq!(
            after.context_limit,
            Some(8_000),
            "CLI transcript reading must persist the transcript context limit"
        );

        let _ = std::fs::remove_dir_all(&claude_root);
        let _ = std::fs::remove_dir_all(&codex_root);
    }

    /// Regression for the miss Mellow's review caught (challenge f00b5263,
    /// upheld 2026-07-11): `forward_session_output` must resolve its own
    /// pre-poll `limit` through [`resolve_session_context_limit`] — the same
    /// helper the spawn-time `TranscriptPollContext::new` call uses — instead
    /// of the plain stored-value-then-generic-default fallback. Exercised
    /// with a codex transcript file whose `token_count` event carries NO
    /// `model_context_window` field (the harness didn't report one, forcing
    /// the reader onto `fallback_limit`) and a deliberately WRONG stored
    /// `session.context_limit` (999) pre-seeded before the forwarder runs —
    /// proving the persisted limit comes from the gpt-5.4 table entry
    /// (1_050_000), not the stale stored value and not the generic
    /// [`repo::session::DEFAULT_CONTEXT_LIMIT`] (200_000).
    #[tokio::test]
    async fn forwarder_codex_known_model_seeds_table_limit_not_stored_or_default() {
        let state = AppState::for_tests().await;
        let id = fixture_instance_typed(&state, "cli", Some("codex")).await;
        let session = repo::session::get_by_instance(&state.db, &id)
            .await
            .expect("get_by_instance failed")
            .expect("session exists");
        let ws = workspace_agent::get(&state.db, &id)
            .await
            .expect("get failed")
            .expect("workspace agent exists");
        let workspace_row = workspace::get(&state.db, &ws.workspace_id)
            .await
            .expect("workspace get failed")
            .expect("workspace exists");

        // Deliberately wrong stored value — proves R4 (stored codex context
        // limit is ignored) rather than merely "some prior default".
        repo::session::set_context_reading(&state.db, &session.id, 0, 999)
            .await
            .expect("seed stale stored context_limit");

        let claude_root = temp_root("forwarder-model-seed-claude");
        let codex_root = temp_root("forwarder-model-seed-codex");
        let codex_file = codex_root.join("2026/07/11/rollout.jsonl");
        std::fs::create_dir_all(codex_file.parent().expect("codex parent dir"))
            .expect("create codex dir");
        write_jsonl(
            &codex_file,
            &[
                serde_json::json!({
                    "type": "session_meta",
                    "timestamp": "2099-01-01T00:00:00Z",
                    "payload": {
                        "cwd": workspace_row.folder_path.clone(),
                        "id": "codex-session-id-not-conclave-instance-id",
                        "originator": "codex-tui"
                    }
                }),
                serde_json::json!({
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "developer",
                        "content": [
                            {
                                "type": "input_text",
                                "text": format!("You are a Conclave agent, and your own agent id is {id}.")
                            }
                        ]
                    }
                }),
                // NO "model_context_window" field — the harness didn't
                // report one, so the reader must fall back to the
                // constructed TranscriptPollContext's fallback_limit.
                serde_json::json!({
                    "type": "event_msg",
                    "timestamp": "2099-01-01T00:00:01Z",
                    "payload": {
                        "type": "token_count",
                        "info": {
                            "last_token_usage": { "total_tokens": 42 },
                            "total_token_usage": { "total_tokens": 42 }
                        }
                    }
                }),
            ],
        );

        let started_at = DateTime::parse_from_rfc3339(&session.started_at)
            .expect("session started_at must parse")
            .with_timezone(&Utc);
        // `with_reader` (test-only) so the reader scans the temp roots above
        // instead of the real `~/.codex/sessions` — but `fallback_limit` is
        // resolved through the SAME helper the production spawn-time
        // `TranscriptPollContext::new` call site uses, proving the wiring.
        let transcript_ctx = TranscriptPollContext::with_reader(
            runtime::transcript_context::TranscriptContextReader::new(
                runtime::transcript_context::TranscriptContextConfig {
                    claude_projects_root: claude_root.clone(),
                    codex_sessions_root: codex_root.clone(),
                    fallback_limit: resolve_session_context_limit(
                        "codex",
                        Some("gpt-5.4"),
                        Some(999),
                    ),
                },
            ),
            &workspace_row.folder_path,
            "codex",
            started_at,
        );

        let epoch = state
            .runtime
            .register(&id, runtime::LiveHandle::placeholder(&session.id))
            .expect("register");
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
            Some("codex".into()),
            Some("gpt-5.4".into()),
            rx,
            false, // CLI/PTY backend — transcript-backed context enabled.
            Some(transcript_ctx),
            epoch,
        ));

        tx.send("prompt".to_string())
            .await
            .expect("send chunk failed");
        drop(tx);
        task.await.expect("forwarder task panicked");

        let after = repo::session::get(&state.db, &session.id)
            .await
            .expect("get failed")
            .expect("session exists");
        assert_eq!(
            after.context_limit,
            Some(1_050_000),
            "codex + known model (gpt-5.4) must seed the table's context limit, \
             not the stale stored value (999) or the generic default (200_000)"
        );

        let _ = std::fs::remove_dir_all(&claude_root);
        let _ = std::fs::remove_dir_all(&codex_root);
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

    /// ADR 0004: the sidecar + pointer are now UNCONDITIONAL, even when the
    /// agent def has no builtin or custom skills attached at all (no
    /// `fixture_skills_dir` override here, so `content_for_agent` returns
    /// truly empty content) — a later live reload needs the file to already
    /// exist so it can rewrite it in place.
    #[tokio::test]
    async fn apply_skills_to_preamble_writes_placeholder_when_nothing_attached() {
        let _fx = repo::skill::test_support::empty_skills_dir("cmd-inst-preamble-empty");
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
            "pointer must be appended even with nothing attached: {result}"
        );
        assert!(skill_ids.is_empty(), "nothing attached: {skill_ids:?}");

        let path = dirs::data_dir()
            .expect("data dir")
            .join("Conclave")
            .join("skills")
            .join(format!("{inst_id}.md"));
        let contents = std::fs::read_to_string(&path).expect("sidecar must exist");
        assert_eq!(contents, NO_SKILLS_PLACEHOLDER);
        let _ = std::fs::remove_file(&path);
    }

    /// A mandatory builtin fixture is still included and rendered into the
    /// sidecar body (not the placeholder) when at least one skill applies.
    #[tokio::test]
    async fn apply_skills_to_preamble_extends_when_builtin_mandatory_applies() {
        let _fx = repo::skill::test_support::fixture_skills_dir("cmd-inst-preamble-builtin");
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

    /// Shared setup for the `reload_skills_for_def` tests: a `cli` instance
    /// and its `id` + `agent_def_id` + expected sidecar path. Callers must
    /// hold a `fixture_skills_dir`/`empty_skills_dir` guard for the whole
    /// test — this fn only builds the DB rows, it doesn't own the builtin
    /// dir override.
    async fn reload_fixture(state: &AppState) -> (String, String, std::path::PathBuf) {
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
        let path = dirs::data_dir()
            .expect("data dir")
            .join("Conclave")
            .join("skills")
            .join(format!("{inst_id}.md"));
        (inst_id, def.id, path)
    }

    /// A dead instance's sidecar is rewritten with the current effective
    /// content, but `launched_skill_ids` is left untouched — spawn recomputes
    /// it fresh on the next launch (Task 3 risk ledger).
    #[tokio::test]
    async fn reload_skills_for_def_rewrites_dead_instance_sidecar_only() {
        let _fx = repo::skill::test_support::fixture_skills_dir("reload-dead");
        let state = AppState::for_tests().await;
        let (inst_id, def_id, path) = reload_fixture(&state).await;

        reload_skills_for_def(&state, &def_id)
            .await
            .expect("reload failed");

        let contents = std::fs::read_to_string(&path).expect("sidecar must exist");
        assert!(
            contents.contains("Mandatory fixture content."),
            "{contents}"
        );

        let session = repo::session::get_by_instance(&state.db, &inst_id)
            .await
            .expect("get_by_instance failed")
            .expect("session exists");
        assert!(
            session.launched_skill_ids.is_none(),
            "dead instance must not have launched_skill_ids touched: {:?}",
            session.launched_skill_ids
        );
        let _ = std::fs::remove_file(&path);
    }

    /// A LIVE instance gets the sidecar rewritten, the nudge injected
    /// (best-effort via a placeholder backend — not asserted directly, see
    /// risk ledger), and `launched_skill_ids` refreshed so the staleness
    /// badge clears.
    #[tokio::test]
    async fn reload_skills_for_def_refreshes_live_instance() {
        let _fx = repo::skill::test_support::fixture_skills_dir("reload-live");
        let state = AppState::for_tests().await;
        let (inst_id, def_id, path) = reload_fixture(&state).await;
        let session = repo::session::get_by_instance(&state.db, &inst_id)
            .await
            .expect("get_by_instance failed")
            .expect("session exists");
        state
            .runtime
            .register(&inst_id, runtime::LiveHandle::placeholder(&session.id))
            .expect("register");

        reload_skills_for_def(&state, &def_id)
            .await
            .expect("reload failed");

        let contents = std::fs::read_to_string(&path).expect("sidecar must exist");
        assert!(
            contents.contains("Mandatory fixture content."),
            "{contents}"
        );

        let refreshed = repo::session::get_by_instance(&state.db, &inst_id)
            .await
            .expect("get_by_instance failed")
            .expect("session exists");
        let ids: Vec<String> = refreshed
            .launched_skill_ids
            .as_deref()
            .and_then(|t| serde_json::from_str(t).ok())
            .unwrap_or_default();
        assert!(
            ids.contains(&"fix-mandatory".to_string()),
            "launched_skill_ids must refresh for a live instance: {ids:?}"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// When the sidecar's CURRENT content already matches what a fresh
    /// compute would produce (and the id set already matches what's
    /// recorded as launched), reload must be a true no-op — guards against
    /// nudging a live agent on every unrelated `agent.save` edit. Proven via
    /// a read-only file: if `reload` attempted a write anyway, it would
    /// error (proving the short-circuit fired, not just that a rewrite
    /// happened to produce identical bytes).
    #[tokio::test]
    async fn reload_skills_for_def_skips_when_unchanged() {
        let _fx = repo::skill::test_support::fixture_skills_dir("reload-unchanged");
        let state = AppState::for_tests().await;
        let (inst_id, def_id, path) = reload_fixture(&state).await;
        let session = repo::session::get_by_instance(&state.db, &inst_id)
            .await
            .expect("get_by_instance failed")
            .expect("session exists");

        // Simulate a prior successful spawn: the sidecar already holds the
        // CURRENT effective content and `launched_skill_ids` already
        // matches the CURRENT effective id set — nothing has drifted.
        let (body, ids) = repo::skill::content_for_agent(&state.db, &def_id)
            .await
            .expect("content_for_agent failed");
        crate::engine::agentctx::write_skill_sidecar(&inst_id, &body).expect("write failed");
        repo::session::set_launched_skill_ids(&state.db, &session.id, &ids)
            .await
            .expect("set_launched_skill_ids failed");

        let mut perms = std::fs::metadata(&path).expect("metadata").permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&path, perms).expect("set readonly failed");

        let result = reload_skills_for_def(&state, &def_id).await;

        let mut perms = std::fs::metadata(&path).expect("metadata").permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        std::fs::set_permissions(&path, perms).expect("restore perms failed");

        result.expect("reload must not attempt to rewrite an unchanged sidecar");
        let _ = std::fs::remove_file(&path);
    }

    /// PLAN BUG (found in lead integration review 2026-07-03): comparing the
    /// skill-ID SET alone is wrong — a `skill.save` content edit on an
    /// already-attached custom skill keeps the id set byte-for-byte
    /// identical, so an id-only guard silently skips the entire
    /// content-edit reload path, the feature's main use case. The guard must
    /// also compare the freshly computed body against the sidecar's CURRENT
    /// file content.
    #[tokio::test]
    async fn reload_skills_for_def_rewrites_on_content_only_edit() {
        let _fx = repo::skill::test_support::empty_skills_dir("reload-content-edit");
        let state = AppState::for_tests().await;
        let (inst_id, def_id, path) = reload_fixture(&state).await;
        let session = repo::session::get_by_instance(&state.db, &inst_id)
            .await
            .expect("get_by_instance failed")
            .expect("session exists");
        state
            .runtime
            .register(&inst_id, runtime::LiveHandle::placeholder(&session.id))
            .expect("register");

        let skill = repo::skill::create(&state.db, "Custom", None, "version one")
            .await
            .expect("create failed");
        repo::skill::set_custom_attachments(&state.db, &def_id, std::slice::from_ref(&skill.id))
            .await
            .expect("attach failed");

        // Baseline reload: writes "version one" and records the (unchanging)
        // id set.
        reload_skills_for_def(&state, &def_id)
            .await
            .expect("reload failed");
        let baseline = std::fs::read_to_string(&path).expect("sidecar must exist");
        assert!(baseline.contains("version one"), "{baseline}");

        // Content-only edit: SAME skill id, different content — the
        // effective id set `reload_skills_for_def` computes is identical
        // before and after this edit.
        repo::skill::update(&state.db, &skill.id, "Custom", None, "version two")
            .await
            .expect("update failed")
            .expect("row exists");

        reload_skills_for_def(&state, &def_id)
            .await
            .expect("reload failed");
        let updated = std::fs::read_to_string(&path).expect("sidecar must exist");
        assert!(
            updated.contains("version two") && !updated.contains("version one"),
            "content edit must rewrite the sidecar even though the id set didn't change: {updated}"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// Mellow's integration review, item L7: a per-instance failure must not
    /// deny reload to its siblings. The broken instance here has its
    /// `session` row deleted out from under it (an orphaned/corrupted row —
    /// the one thing `reload_skills_for_def` genuinely cannot proceed past
    /// for THAT instance) alongside a healthy sibling of the SAME def; the
    /// healthy instance must still get its sidecar rewritten, and the call
    /// overall must not error.
    #[tokio::test]
    async fn reload_skills_for_def_continues_past_a_broken_instance() {
        let _fx = repo::skill::test_support::fixture_skills_dir("reload-partial-failure");
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

        // Broken: instantiated (so it's a real workspace_agent of this def),
        // then its session row is deleted out from under it.
        let broken_id = workspace_agent::instantiate(&state.db, &ws.id, &def.id)
            .await
            .expect("instantiate failed")
            .id;
        sqlx::query("DELETE FROM session WHERE workspace_agent_id = ?")
            .bind(&broken_id)
            .execute(&state.db)
            .await
            .expect("delete session failed");

        // Healthy sibling of the SAME def, in a SECOND workspace —
        // `instantiate` is idempotent per (workspace_id, agent_def_id), so
        // reusing `ws` here would just return the broken instance again.
        // Created AFTER the broken one, so the old `?`-propagating code
        // (which aborts the whole loop on the first error) would never
        // reach it.
        let ws2 = workspace::create(&state.db, "WS2", "/tmp/ws2", None)
            .await
            .expect("create workspace failed");
        let healthy_id = workspace_agent::instantiate(&state.db, &ws2.id, &def.id)
            .await
            .expect("instantiate failed")
            .id;
        let healthy_path = dirs::data_dir()
            .expect("data dir")
            .join("Conclave")
            .join("skills")
            .join(format!("{healthy_id}.md"));

        reload_skills_for_def(&state, &def.id)
            .await
            .expect("one broken instance must not abort reload for its siblings");

        let contents = std::fs::read_to_string(&healthy_path).expect("healthy sidecar must exist");
        assert!(
            contents.contains("Mandatory fixture content."),
            "{contents}"
        );
        let _ = std::fs::remove_file(&healthy_path);
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

    /// restart on an unknown instance → NotFound (no arm, no tail).
    #[tokio::test]
    async fn restart_unknown_instance_not_found() {
        let state = AppState::for_tests().await;
        let err = restart(&state, json!({ "workspaceAgentId": "nope" }))
            .await
            .expect_err("restart must fail for an unknown instance");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    /// restart is CLI-only: an orchestrator instance → Invalid (no PTY process
    /// or handoff loop to restart).
    #[tokio::test]
    async fn restart_non_cli_invalid() {
        let state = AppState::for_tests().await;
        let id = fixture_instance(&state).await; // orchestrator type
        let err = restart(&state, json!({ "workspaceAgentId": id }))
            .await
            .expect_err("restart must reject non-cli agents");
        assert!(matches!(err, AppError::Invalid(_)));
    }

    /// restart on a LIVE cli agent is save-gated: it arms the restart, injects
    /// the save prompt (absorbed by the placeholder backend here), reports
    /// phase "saving" — and does NOT kill the process before the save lands.
    #[tokio::test]
    async fn restart_live_cli_arms_and_reports_saving() {
        let state = AppState::for_tests().await;
        let id = fixture_instance_typed(&state, "cli", Some("codex")).await;
        let session = repo::session::get_by_instance(&state.db, &id)
            .await
            .expect("get_by_instance failed")
            .expect("session exists");
        assert!(state
            .runtime
            .register(&id, runtime::LiveHandle::placeholder(&session.id))
            .is_some());

        let out = restart(&state, json!({ "workspaceAgentId": id }))
            .await
            .expect("restart failed");
        assert_eq!(out.get("phase").and_then(Value::as_str), Some("saving"));
        assert!(
            state.take_restart_pending(&id),
            "restart must be armed for the next save"
        );
        assert!(
            state.runtime.is_live(&id),
            "a live agent must NOT be killed before its handoff is saved"
        );
    }

    /// ADR 0006: a SELF-triggered restart on a live instance must NOT inject
    /// anything into the agent's own TUI (the caller IS the agent, mid-turn —
    /// injecting would interleave with its own output). It still arms the
    /// restart and reports phase "saving", but returns an `instruction` field
    /// instead. Proven via `LiveHandle::for_test`, whose receiver is NOT
    /// drained by a background task (unlike `placeholder`) — an empty
    /// `try_recv()` here proves nothing was ever sent, not just that it was
    /// silently consumed.
    #[tokio::test]
    async fn restart_self_true_live_does_not_write_to_pty() {
        let state = AppState::for_tests().await;
        let id = fixture_instance_typed(&state, "cli", Some("codex")).await;
        let session = repo::session::get_by_instance(&state.db, &id)
            .await
            .expect("get_by_instance failed")
            .expect("session exists");
        let (handle, mut rx) = runtime::LiveHandle::for_test(&session.id);
        assert!(state.runtime.register(&id, handle).is_some());

        let out = restart(&state, json!({ "workspaceAgentId": id, "self": true }))
            .await
            .expect("restart failed");

        assert_eq!(out.get("phase").and_then(Value::as_str), Some("saving"));
        assert!(
            state.take_restart_pending(&id),
            "self-triggered restart must still arm"
        );
        let instruction = out
            .get("instruction")
            .and_then(Value::as_str)
            .expect("self-triggered restart must return an instruction");
        assert!(
            instruction.contains("conclave snapshot save"),
            "{instruction}"
        );
        assert!(
            rx.try_recv().is_err(),
            "self-triggered restart must NOT write to the agent's own PTY"
        );
    }

    /// The returned instruction's TTL must match `AppState::restart_pending_ttl`
    /// exactly — computed here from the SAME accessor the production code
    /// uses, never a hardcoded literal, so this test can't silently drift
    /// from a future TTL change (Task 3 risk ledger).
    #[tokio::test]
    async fn restart_self_true_instruction_ttl_matches_restart_pending_ttl() {
        let state = AppState::for_tests().await;
        let id = fixture_instance_typed(&state, "cli", Some("codex")).await;
        let session = repo::session::get_by_instance(&state.db, &id)
            .await
            .expect("get_by_instance failed")
            .expect("session exists");
        let (handle, _rx) = runtime::LiveHandle::for_test(&session.id);
        assert!(state.runtime.register(&id, handle).is_some());

        let out = restart(&state, json!({ "workspaceAgentId": id, "self": true }))
            .await
            .expect("restart failed");
        let instruction = out.get("instruction").and_then(Value::as_str).unwrap();

        let expected_minutes = state.restart_pending_ttl().as_secs() / 60;
        assert!(
            instruction.contains(&expected_minutes.to_string()),
            "instruction must surface the real TTL ({expected_minutes} min): {instruction}"
        );
    }

    /// Double trigger (risk ledger): the agent runs `conclave restart` twice
    /// before saving. `mark_restart_pending` just overwrites the arm — the
    /// second call must succeed and return the SAME instruction, not an
    /// error.
    #[tokio::test]
    async fn restart_self_true_double_trigger_returns_same_instruction() {
        let state = AppState::for_tests().await;
        let id = fixture_instance_typed(&state, "cli", Some("codex")).await;
        let session = repo::session::get_by_instance(&state.db, &id)
            .await
            .expect("get_by_instance failed")
            .expect("session exists");
        let (handle, _rx) = runtime::LiveHandle::for_test(&session.id);
        assert!(state.runtime.register(&id, handle).is_some());

        let first = restart(&state, json!({ "workspaceAgentId": id, "self": true }))
            .await
            .expect("first restart failed");
        let second = restart(&state, json!({ "workspaceAgentId": id, "self": true }))
            .await
            .expect("second restart must not error");

        assert_eq!(first.get("instruction"), second.get("instruction"));
        assert!(
            state.take_restart_pending(&id),
            "must still be armed after the double trigger"
        );
    }

    /// Contrast case for `restart_self_true_live_does_not_write_to_pty`: the
    /// existing HUMAN-triggered path (no `self`) must still inject the save
    /// prompt into the PTY, proven with the same observing handle.
    #[tokio::test]
    async fn restart_non_self_live_still_injects_prompt() {
        let state = AppState::for_tests().await;
        let id = fixture_instance_typed(&state, "cli", Some("codex")).await;
        let session = repo::session::get_by_instance(&state.db, &id)
            .await
            .expect("get_by_instance failed")
            .expect("session exists");
        let (handle, mut rx) = runtime::LiveHandle::for_test(&session.id);
        assert!(state.runtime.register(&id, handle).is_some());

        let out = restart(&state, json!({ "workspaceAgentId": id }))
            .await
            .expect("restart failed");
        assert_eq!(out.get("phase").and_then(Value::as_str), Some("saving"));
        assert!(
            out.get("instruction").is_none(),
            "non-self path has no instruction field"
        );

        let sent = rx
            .try_recv()
            .expect("non-self restart must inject the save prompt");
        assert!(sent.contains("conclave snapshot save"), "{sent}");
    }

    /// restart on a dead cli agent needs the app runtime to drive the detached
    /// respawn tail; without an AppHandle (tests) it surfaces an honest
    /// Internal error instead of silently doing nothing.
    #[tokio::test]
    async fn restart_dead_cli_without_app_handle_is_internal() {
        let state = AppState::for_tests().await;
        let id = fixture_instance_typed(&state, "cli", Some("codex")).await;
        let err = restart(&state, json!({ "workspaceAgentId": id }))
            .await
            .expect_err("restart of a dead agent must fail without an AppHandle");
        assert!(matches!(err, AppError::Internal(_)));
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

    /// Helper: the `ended` flag of the browser tab owned by `id`, or `None` if
    /// no tab exists for it. Reads the process-global registry (fixture UUIDs
    /// keep each test's tab isolated from the others sharing that static).
    fn tab_ended(id: &str) -> Option<bool> {
        runtime::browser::state()
            .tabs
            .into_iter()
            .find(|t| t.tab_id == id)
            .map(|t| t.ended)
    }

    /// T2: a crash-death EOF (current epoch) flips the owning agent's browser
    /// tab to `ended`. Exercises the chat/`track_context` path, whose EOF falls
    /// through to the shared epoch-guarded tail.
    #[tokio::test]
    async fn forwarder_marks_tab_ended_on_eof() {
        let state = AppState::for_tests().await;
        let id = fixture_instance(&state).await;
        let session = repo::session::get_by_instance(&state.db, &id)
            .await
            .expect("get_by_instance failed")
            .expect("session exists");

        runtime::browser::test_seed_agent_tab(&id);

        let epoch = state
            .runtime
            .register(&id, runtime::LiveHandle::placeholder(&session.id))
            .expect("register");
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
            None,
            None,
            rx,
            true, // chat backend → shared epoch-guarded tail.
            None,
            epoch,
        ));

        drop(tx); // EOF → epoch-guarded cleanup runs.
        task.await.expect("forwarder task panicked");

        assert_eq!(
            tab_ended(&id),
            Some(true),
            "crash-death EOF must mark the owning tab ended"
        );
    }

    /// T2 (guard): a LATE EOF from a SUPERSEDED generation (its epoch no longer
    /// matches the live one after a restart reused the id) must NOT mark the tab
    /// ended — the same epoch guard that protects the idle transition.
    #[tokio::test]
    async fn forwarder_late_eof_does_not_mark_tab_ended() {
        let state = AppState::for_tests().await;
        let id = fixture_instance(&state).await;
        let session = repo::session::get_by_instance(&state.db, &id)
            .await
            .expect("get_by_instance failed")
            .expect("session exists");

        runtime::browser::test_seed_agent_tab(&id);

        // Generation 1's epoch — the one the stale forwarder will carry.
        let stale_epoch = state
            .runtime
            .register(&id, runtime::LiveHandle::placeholder(&session.id))
            .expect("register gen1");
        // Simulate a restart: drop gen1 and register gen2 (a strictly newer
        // epoch) on the same id. `next_epoch` is monotonic, so gen2 > gen1.
        assert!(state.runtime.unregister(&id), "gen1 was live");
        let _gen2 = state
            .runtime
            .register(&id, runtime::LiveHandle::placeholder(&session.id))
            .expect("register gen2");

        let (tx, rx) = tokio::sync::mpsc::channel::<String>(8);
        let task = tokio::spawn(forward_session_output(
            state.db.clone(),
            Arc::clone(&state.runtime),
            None,
            id.clone(),
            session.id.clone(),
            None,
            None,
            rx,
            true,
            None,
            stale_epoch, // superseded — unregister_epoch returns false.
        ));

        drop(tx);
        task.await.expect("forwarder task panicked");

        assert_eq!(
            tab_ended(&id),
            Some(false),
            "a superseded late EOF must NOT mark the tab ended"
        );
    }

    /// T3: `stop` flips the agent's browser tab to `ended` (it won the teardown
    /// race). Sibling to `stop_marks_idle`.
    #[tokio::test]
    async fn stop_marks_tab_ended() {
        let state = AppState::for_tests().await;
        let id = fixture_instance(&state).await;

        runtime::browser::test_seed_agent_tab(&id);

        spawn(&state, json!({ "workspaceAgentId": id }))
            .await
            .expect("spawn failed");
        assert!(state.runtime.is_live(&id));

        stop(&state, json!({ "workspaceAgentId": id }))
            .await
            .expect("stop failed");

        assert_eq!(
            tab_ended(&id),
            Some(true),
            "stop must mark the owning tab ended"
        );
    }

    /// T4: `restart` (D-1) does NOT mark the tab ended — the tab is reused by the
    /// respawned generation, so marking it would wrongly lock a live agent's tab.
    #[tokio::test]
    async fn restart_does_not_mark_tab_ended() {
        let state = AppState::for_tests().await;
        let id = fixture_instance_typed(&state, "cli", Some("codex")).await;
        let session = repo::session::get_by_instance(&state.db, &id)
            .await
            .expect("get_by_instance failed")
            .expect("session exists");

        runtime::browser::test_seed_agent_tab(&id);
        assert!(state
            .runtime
            .register(&id, runtime::LiveHandle::placeholder(&session.id))
            .is_some());

        let out = restart(&state, json!({ "workspaceAgentId": id }))
            .await
            .expect("restart failed");
        assert_eq!(out.get("phase").and_then(Value::as_str), Some("saving"));

        assert_eq!(
            tab_ended(&id),
            Some(false),
            "restart must NOT mark the tab ended (D-1)"
        );
    }
}
