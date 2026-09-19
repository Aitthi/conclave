use crate::engine::runtime::outbox::{HeldItem, Outbox, Push};
use crate::engine::{bus, repo, runtime::StdinError, AppError, AppState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Instant;

/// Gaps (ms) before each submit-Enter sent after an injected message's text.
/// Escalating so at least one CR arrives after the receiver drained the text
/// out of the PTY — a CR read in the same burst as the text is treated as
/// paste content, not a keystroke, leaving the message stuck in the composer.
const SUBMIT_CR_DELAYS_MS: [u64; 3] = [40, 120, 300];

/// Payload for `message.send` — a line of user input destined for a live session.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendReq {
    session_id: String,
    text: String,
    /// `true` when `text` is a composed message (StdinBar) rather than raw
    /// keystrokes (Terminal pane, the submit `\r`): it is then delivered as ONE
    /// bracketed paste on PTY backends so a body longer than one kernel read
    /// (macOS: 1022 bytes) is not head-truncated by the receiving TUI. Absent
    /// = raw, the byte-exact path xterm's `onData` relies on.
    #[serde(default)]
    paste: bool,
}

/// Ack returned by `message.send`. Shape mirrors the TS `Message` interface in
/// `src/ipc/types.ts` (camelCase). NOT persisted yet — the `message` table is M3.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MessageAck {
    id: String,
    session_id: String,
    role: &'static str,
    text: String,
    created_at: String,
}

async fn require_delivery_eligible(
    state: &AppState,
    instance_id: &str,
) -> Result<repo::workspace_agent::RuntimeEligibility, AppError> {
    let eligibility = repo::workspace_agent::runtime_eligibility(&state.db, instance_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("workspace_agent id={instance_id} not found")))?;
    super::workspace::require_not_archived(
        &eligibility.workspace_id,
        eligibility.archived_at.as_deref(),
    )?;
    if eligibility.run_state != "started" {
        return Err(AppError::Invalid(format!(
            "workspace {} is stopped — start it before sending messages",
            eligibility.workspace_id
        )));
    }
    if eligibility.availability != "active" {
        return Err(AppError::Invalid(format!(
            "workspace_agent id={instance_id} is stopped — resume it before sending messages"
        )));
    }
    Ok(eligibility)
}

/// Send a line of user input to a running CLI agent's live PTY.
///
/// Resolves the session by id, routes `text` verbatim to the live backend's
/// stdin (the FRONTEND appends the newline), and returns a `user` message ack.
///
/// Errors:
/// - session id unknown → [`AppError::NotFound`]
/// - session not running (no live backend) → [`AppError::NotFound`]
/// - backend stdin channel closed (server-side fault) → [`AppError::Internal`]
pub async fn send(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let SendReq {
        session_id,
        text,
        paste,
    } = serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    let session = repo::session::get(&state.db, &session_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("session id={session_id} not found")))?;

    let eligibility = require_delivery_eligible(state, &session.workspace_agent_id).await?;
    let workspace_lock = state.workspace_lifecycle_lock(&eligibility.workspace_id);
    let _workspace_guard = workspace_lock.read().await;
    let agent_lock = state.agent_lifecycle_lock(&session.workspace_agent_id);
    let _agent_guard = agent_lock.lock().await;
    require_delivery_eligible(state, &session.workspace_agent_id).await?;

    // Route the text to the live PTY keyed by the owning workspace_agent. The
    // ack below still owns `text` — the runtime borrows it.
    let routed = if paste {
        state
            .runtime
            .send_stdin_paste(&session.workspace_agent_id, &text)
    } else {
        state.runtime.send_stdin(&session.workspace_agent_id, &text)
    };
    routed.map_err(|e| match e {
        // No live backend = the session simply isn't running yet.
        StdinError::NotLive => AppError::NotFound(format!(
            "session {session_id} is not running — spawn it first"
        )),
        // Registered-but-closed channel is a backend fault, not a bad request.
        StdinError::Closed => {
            AppError::Internal(format!("session {session_id} backend stdin channel closed"))
        }
    })?;

    // TODO(M3): persist to message table
    let ack = MessageAck {
        id: uuid::Uuid::new_v4().to_string(),
        session_id,
        role: "user",
        text,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    serde_json::to_value(ack).map_err(|e| AppError::Internal(e.to_string()))
}

/// Payload for `message.inject` — inter-agent input injection.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InjectReq {
    from_instance_id: String,
    to_instance_id: String,
    text: String,
    /// `true` = deliver NOW as a single-item flush, bypassing the target's
    /// outbox stack and leaving it untouched. Set ONLY by the human's routed
    /// send in the UI (ChatView / StdinBar) — spec ruling 5. CLI `tell` and
    /// system notifications never set it.
    #[serde(default)]
    immediate: bool,
}

/// Inject a line of text into a TARGET instance's live input, auto-submit it,
/// tag the origin (sender), persist an `InterAgentMessage`, and emit a bus event
/// so the UI can render the injection ("injected from X · auto-submitted").
///
/// This is the inter-agent messaging backbone (M3.1).
///
/// **Outbox:** the row is persisted as `"held"` and pushed onto the target's
/// outbox stack (`runtime::outbox`). Delivery happens in [`flush_stack`] —
/// from the sweeper when the stack's deadline passes, or synchronously here
/// when the push hits `MAX_ITEMS` (the ack then already reads
/// `"delivered"`/`"queued"`). Human input (`send`) never touches the outbox,
/// and a human ROUTED send passes `immediate: true` to be delivered on its own
/// right away, leaving the target's pending stack untouched (spec ruling 5).
///
/// **Origin tag:** the origin is carried as `from_instance_id` on the persisted
/// row AND on the `message:injected` event. We deliberately inject the RAW
/// `text` into the target's stdin (no marker pollution of the agent's actual
/// input); the UI renders the "injected from X" chrome from the event/row. The
/// visible origin-tagged bubble/line is the M3.2 UI task.
///
/// Errors:
/// - malformed payload → [`AppError::Invalid`]
/// - unknown sender OR target instance → [`AppError::NotFound`]
pub async fn inject(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let InjectReq {
        from_instance_id,
        to_instance_id,
        text,
        immediate,
    } = serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    // Resolve both endpoints before locking so cross-workspace injection can
    // acquire every workspace READ guard in one deterministic order. Reverse
    // injections (A→B and B→A) therefore cannot deadlock. Same-workspace and
    // self-injection dedupe down to one workspace/agent lock respectively.
    let source_eligibility = require_delivery_eligible(state, &from_instance_id).await?;
    let target_eligibility = require_delivery_eligible(state, &to_instance_id).await?;
    let mut workspace_ids = vec![
        source_eligibility.workspace_id,
        target_eligibility.workspace_id,
    ];
    workspace_ids.sort_unstable();
    workspace_ids.dedup();
    let mut _workspace_guards = Vec::with_capacity(workspace_ids.len());
    for workspace_id in workspace_ids {
        _workspace_guards.push(
            state
                .workspace_lifecycle_lock(&workspace_id)
                .read_owned()
                .await,
        );
    }

    // Lock agents only after all workspace guards, again in deterministic
    // order. These owned guards stay live through stdin delivery and durable
    // persistence, making either endpoint's Stop linearizable with inject.
    let mut instance_ids = vec![from_instance_id.as_str(), to_instance_id.as_str()];
    instance_ids.sort_unstable();
    instance_ids.dedup();
    let mut _agent_guards = Vec::with_capacity(instance_ids.len());
    for instance_id in instance_ids {
        _agent_guards.push(state.agent_lifecycle_lock(instance_id).lock_owned().await);
    }

    // State may have changed between the optimistic lookup and lock
    // acquisition. Re-read both halves while holding every lifecycle guard;
    // rejection occurs before any stdin write or queued-row insert.
    require_delivery_eligible(state, &from_instance_id).await?;
    require_delivery_eligible(state, &to_instance_id).await?;

    // Resolve the sender row for the display name used by the `[from …]` tag.
    // Existence and lifecycle eligibility were already validated above while
    // holding both endpoint guards.
    let sender = match repo::workspace_agent::get(&state.db, &from_instance_id).await? {
        Some(inst) => repo::agent_definition::get(&state.db, &inst.agent_def_id)
            .await?
            .map(|d| d.name)
            .unwrap_or_else(|| "another agent".to_string()),
        None => {
            return Err(AppError::NotFound(format!(
                "sender instance id={from_instance_id} not found"
            )))
        }
    };
    // Persist FIRST as `held`: the row exists before anything can flush it, so
    // a flush that races this call can never see a missing id. `text` stays
    // RAW on the row (the persisted row + UI carry the origin separately);
    // the `[from {name} · {id}] ` tag is applied at flush, in `Outbox::body`.
    let mut row = repo::inter_agent_message::create(
        &state.db,
        &from_instance_id,
        &to_instance_id,
        &text,
        "held",
        true,
    )
    .await?;

    let item = HeldItem {
        row_id: row.id.clone(),
        from_instance_id: from_instance_id.clone(),
        sender_name: sender,
        text,
    };
    // `immediate` (human routed send from the composer — spec ruling 5) skips
    // the stack entirely: deliver THIS message now and leave whatever is
    // pending for the target untouched. Everything else joins the stack.
    let push = if immediate {
        Push::Flush(vec![item])
    } else {
        state.outbox.push(&to_instance_id, item, Instant::now())
    };

    // Release BOTH lifecycle guards before a synchronous flush:
    // `flush_stack` takes the target's guards itself and the agent mutex is
    // not re-entrant — holding them here would deadlock.
    drop(_agent_guards);
    drop(_workspace_guards);

    if let Push::Flush(items) = push {
        row.status = flush_stack(state, &to_instance_id, items).await.to_owned();
    }

    serde_json::to_value(row).map_err(|e| AppError::Internal(e.to_string()))
}

/// Deliver one target's flushed stack as ONE bracketed paste + the escalating
/// submit CRs, then settle every row's status. Called by the outbox sweeper
/// (`runtime::outbox::run`) and by `inject` on a cap flush.
///
/// The body goes out as ONE bracketed paste (PTY backends): a body longer than
/// the kernel's PTY input queue (macOS: 1022 bytes) reaches the TUI as several
/// reads, and Claude Code's un-bracketed burst handling keeps only the LAST
/// read — the receiver then submits just the tail (a 1023-byte tell arrived as
/// "."; docs/superpowers/plans/2026-09-04-inject-bracketed-paste.md). Inside
/// the envelope the TUI reassembles the whole body regardless of read
/// boundaries, exactly as it does for a human's terminal paste. A chat backend
/// has no terminal and receives the body raw.
///
/// A TUI's Enter is CR (`\r`), not LF; and a CR inside the paste envelope (or,
/// on a non-bracketed burst, in the SAME write as the text) is literal paste
/// content. Worse, a SINGLE spaced CR still races the receiver's PTY drain, so
/// Enter is pressed AFTER the paste at escalating gaps
/// ([`SUBMIT_CR_DELAYS_MS`]) — at least one lands as an isolated keystroke;
/// extra Enters on an empty composer are no-ops.
///
/// Returns the final row status: `"delivered"` (PTY accepted; one
/// `message:injected` event per item) or `"queued"` (target not live, backend
/// channel closed, or no longer delivery-eligible; no PTY write, no event).
/// Never errors — a flush has no caller to hand an error to. A `Closed`
/// channel or a lost eligibility race degrades the rows (and a synchronous
/// caller's ack) to `queued` rather than erroring; `inject`'s cap/immediate
/// path shares that contract.
pub async fn flush_stack(
    state: &AppState,
    to_instance_id: &str,
    items: Vec<HeldItem>,
) -> &'static str {
    if items.is_empty() {
        return "delivered";
    }
    let ids: Vec<String> = items.iter().map(|i| i.row_id.clone()).collect();

    // Serialize deliveries per target BEFORE the eligibility round-trip (see
    // Outbox::flush_lock), so batches land in lock-acquisition order (tokio
    // FIFO): otherwise the order is settled only after each flush's DB
    // round-trip, and a cap/immediate flush can overtake a stack the sweeper
    // already took.
    let flush_lock = state.outbox.flush_lock(to_instance_id);
    let _flush_guard = flush_lock.lock().await;

    // Same guard order as `inject`: workspace READ, then the agent mutex, then
    // re-check eligibility under the guards so a Stop that raced us wins.
    let eligibility = match require_delivery_eligible(state, to_instance_id).await {
        Ok(e) => e,
        Err(_) => return settle_rows(state, &ids, "queued").await,
    };
    let workspace_lock = state.workspace_lifecycle_lock(&eligibility.workspace_id);
    let _workspace_guard = workspace_lock.read().await;
    let agent_lock = state.agent_lifecycle_lock(to_instance_id);
    let _agent_guard = agent_lock.lock().await;
    if require_delivery_eligible(state, to_instance_id)
        .await
        .is_err()
    {
        return settle_rows(state, &ids, "queued").await;
    }

    let body = Outbox::body(&items);
    match state.runtime.send_stdin_paste(to_instance_id, &body) {
        Ok(()) => {
            for delay_ms in SUBMIT_CR_DELAYS_MS {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                let _ = state.runtime.send_stdin(to_instance_id, "\r");
            }
        }
        // Target isn't running: the rows stay recorded as queued.
        // TODO(M3.x): deliver-on-spawn — drain queued messages when the target
        // becomes live. No queue drain yet.
        Err(StdinError::NotLive) => return settle_rows(state, &ids, "queued").await,
        Err(StdinError::Closed) => {
            eprintln!(
                "[outbox] target {to_instance_id} backend stdin channel closed; {} rows queued",
                items.len()
            );
            return settle_rows(state, &ids, "queued").await;
        }
    }

    let status = settle_rows(state, &ids, "delivered").await;
    // Persist-before-emit, one event per item so the UI keeps one bubble per
    // message ("injected from X · auto-submitted").
    let to_session_id = state.runtime.session_id(to_instance_id);
    for item in items {
        state.emit(
            bus::MESSAGE_INJECTED,
            bus::MessageInjected {
                to_instance_id: to_instance_id.to_owned(),
                to_session_id: to_session_id.clone(),
                from_instance_id: item.from_instance_id,
                text: item.text,
                auto_submitted: true,
            },
        );
    }
    status
}

/// Write the final status onto every row of a flushed stack. Logs and moves
/// on if the UPDATE fails — the PTY write (if any) already happened and the
/// sweeper must never stall on a DB hiccup.
async fn settle_rows(state: &AppState, ids: &[String], status: &'static str) -> &'static str {
    if let Err(e) = repo::inter_agent_message::update_status(&state.db, ids, status).await {
        eprintln!(
            "[outbox] update_status({status}) for {} row(s) failed: {e}",
            ids.len()
        );
    }
    status
}

/// Payload for `message.list` — the inbox/outbox query for one instance.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListReq {
    instance_id: String,
    limit: Option<i64>,
    /// When `Some(true)`, attach `fromName`/`toName` (instance-id → agent
    /// definition name) to each emitted object. Absent/false = the plain row
    /// serialization the UI's typed feed depends on, byte-for-byte.
    #[serde(default)]
    with_names: Option<bool>,
}

/// Default + max number of messages returned by `message.list`.
const DEFAULT_LIST_LIMIT: i64 = 50;
const MAX_LIST_LIMIT: i64 = 200;

/// List an instance's inbox + outbox (inter-agent messages where it is sender
/// OR recipient), newest-first, as a JSON array of `InterAgentMessage`.
///
/// `limit` defaults to [`DEFAULT_LIST_LIMIT`] and is clamped to
/// `1..=MAX_LIST_LIMIT` so a hostile/garbage value can't ask for an unbounded
/// scan or a non-positive LIMIT.
///
/// Errors:
/// - malformed payload → [`AppError::Invalid`]
/// - unknown instance → [`AppError::NotFound`]
pub async fn list(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let ListReq {
        instance_id,
        limit,
        with_names,
    } = serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    if !repo::workspace_agent::exists(&state.db, &instance_id).await? {
        return Err(AppError::NotFound(format!(
            "instance id={instance_id} not found"
        )));
    }

    let limit = limit.unwrap_or(DEFAULT_LIST_LIMIT).clamp(1, MAX_LIST_LIMIT);
    let rows = repo::inter_agent_message::list_for_instance(&state.db, &instance_id, limit).await?;
    rows_to_value(state, rows, with_names == Some(true)).await
}

/// Serialize list rows to a JSON array. When `with_names` is true, attach
/// `fromName`/`toName` (resolved once per distinct instance id); otherwise emit
/// the plain row serialization UNCHANGED so the UI's typed feed is untouched.
async fn rows_to_value(
    state: &AppState,
    rows: Vec<repo::inter_agent_message::InterAgentMessageRow>,
    with_names: bool,
) -> Result<Value, AppError> {
    if !with_names {
        return serde_json::to_value(rows).map_err(|e| AppError::Internal(e.to_string()));
    }

    // Resolve every DISTINCT id once (up to 2×MAX rows would otherwise re-query
    // the same senders): id → workspace_agent → agent_definition.name, the same
    // chain the `inject` handler uses for the `[from …]` tag.
    let ids: std::collections::HashSet<&str> = rows
        .iter()
        .flat_map(|r| [r.from_instance_id.as_str(), r.to_instance_id.as_str()])
        .collect();
    let mut names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for id in ids {
        if let Some(inst) = repo::workspace_agent::get(&state.db, id).await? {
            if let Some(def) = repo::agent_definition::get(&state.db, &inst.agent_def_id).await? {
                names.insert(id.to_string(), def.name);
            }
        }
    }

    // Enrich additively: start from the exact row JSON, then insert the two name
    // keys when resolvable (absent otherwise — the renderer falls back to a
    // short id).
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let mut v = serde_json::to_value(row).map_err(|e| AppError::Internal(e.to_string()))?;
        if let Some(obj) = v.as_object_mut() {
            if let Some(name) = names.get(&row.from_instance_id) {
                obj.insert("fromName".into(), Value::String(name.clone()));
            }
            if let Some(name) = names.get(&row.to_instance_id) {
                obj.insert("toName".into(), Value::String(name.clone()));
            }
        }
        out.push(v);
    }
    Ok(Value::Array(out))
}

/// Payload for `message.listForWorkspace` — the Chat Hub's workspace-wide query.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListForWorkspaceReq {
    workspace_id: String,
    limit: Option<i64>,
    /// See [`ListReq::with_names`] — same opt-in enrichment for the Chat Hub
    /// feed. The UI never sends it, so its output stays unchanged.
    #[serde(default)]
    with_names: Option<bool>,
}

/// `message.listForWorkspace` — the whole workspace's inter-agent traffic,
/// newest first (Chat Hub). `limit` defaults to [`MAX_LIST_LIMIT`] (the hub
/// wants the full recent window) and is clamped to `1..=MAX_LIST_LIMIT`,
/// same rationale as [`list`].
pub async fn list_for_workspace(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let ListForWorkspaceReq {
        workspace_id,
        limit,
        with_names,
    } = serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    if !repo::workspace::exists(&state.db, &workspace_id).await? {
        return Err(AppError::NotFound(format!(
            "workspace id={workspace_id} not found"
        )));
    }

    let limit = limit.unwrap_or(MAX_LIST_LIMIT).clamp(1, MAX_LIST_LIMIT);
    let rows =
        repo::inter_agent_message::list_for_workspace(&state.db, &workspace_id, limit).await?;
    rows_to_value(state, rows, with_names == Some(true)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::repo::{
        agent_definition::{self, AgentDefinitionInput},
        session, workspace, workspace_agent,
    };
    use crate::engine::runtime::LiveHandle;
    use serde_json::json;

    /// Create a workspace + agent_definition, instantiate an instance (with its
    /// session), and return the session id. Does NOT spawn — no live backend.
    async fn fixture_session_id(state: &AppState) -> String {
        let ws = workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        workspace::set_run_state(&state.db, &ws.id, "started")
            .await
            .expect("start fixture workspace");
        let def = agent_definition::create(
            &state.db,
            &AgentDefinitionInput {
                name: "MessageTestAgent".into(),
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
            },
        )
        .await
        .expect("create agent_def failed");
        let instance = workspace_agent::instantiate(&state.db, &ws.id, &def.id)
            .await
            .expect("instantiate failed");
        session::get_by_instance(&state.db, &instance.id)
            .await
            .expect("get_by_instance failed")
            .expect("session exists")
            .id
    }

    /// Create a workspace + agent_definition, instantiate, and return the
    /// resulting `workspace_agent` (instance) id. `name` keeps the agent_def
    /// names distinct so two instances can coexist in one workspace.
    async fn fixture_instance_id(state: &AppState, name: &str) -> String {
        let ws = workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        workspace::set_run_state(&state.db, &ws.id, "started")
            .await
            .expect("start fixture workspace");
        let def = agent_definition::create(
            &state.db,
            &AgentDefinitionInput {
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
            },
        )
        .await
        .expect("create agent_def failed");
        workspace_agent::instantiate(&state.db, &ws.id, &def.id)
            .await
            .expect("instantiate failed")
            .id
    }

    /// Helper: a workspace with TWO instances in it — returns (ws_id, a, b).
    /// `fixture_instance_id` makes a fresh workspace per call, which is
    /// exactly what listForWorkspace tests must NOT do.
    async fn fixture_workspace_pair(state: &AppState) -> (String, String, String) {
        let ws = workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        workspace::set_run_state(&state.db, &ws.id, "started")
            .await
            .expect("start fixture workspace");
        let mut ids = Vec::new();
        for name in ["Alpha", "Bravo"] {
            let def = agent_definition::create(
                &state.db,
                &AgentDefinitionInput {
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
                },
            )
            .await
            .expect("create agent_def failed");
            ids.push(
                workspace_agent::instantiate(&state.db, &ws.id, &def.id)
                    .await
                    .expect("instantiate failed")
                    .id,
            );
        }
        let b = ids.pop().expect("b");
        let a = ids.pop().expect("a");
        (ws.id, a, b)
    }

    /// listForWorkspace: unknown workspace → NotFound.
    #[tokio::test]
    async fn list_for_workspace_unknown_workspace_not_found() {
        let state = AppState::for_tests().await;
        let err = list_for_workspace(&state, json!({ "workspaceId": "nope" }))
            .await
            .expect_err("should fail for unknown workspace");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    /// listForWorkspace returns the workspace's rows (camelCase), and another
    /// workspace's traffic never leaks in.
    #[tokio::test]
    async fn list_for_workspace_returns_scoped_rows() {
        let state = AppState::for_tests().await;
        let (ws_id, a, b) = fixture_workspace_pair(&state).await;
        // Traffic in a DIFFERENT workspace (fixture makes its own ws per call).
        let x = fixture_instance_id(&state, "Other").await;
        repo::inter_agent_message::create(&state.db, &a, &b, "hello", "delivered", true)
            .await
            .expect("msg in ws");
        repo::inter_agent_message::create(&state.db, &x, &x, "elsewhere", "delivered", true)
            .await
            .expect("msg elsewhere");

        let val = list_for_workspace(&state, json!({ "workspaceId": ws_id }))
            .await
            .expect("list failed");
        let arr = val.as_array().expect("array");
        assert_eq!(arr.len(), 1, "only the workspace's own message");
        assert_eq!(arr[0].get("text").and_then(Value::as_str), Some("hello"));
        assert_eq!(
            arr[0].get("fromInstanceId").and_then(Value::as_str),
            Some(a.as_str())
        );
    }

    /// `withNames: true` attaches `fromName`/`toName` resolved through the
    /// instance-id → agent_definition.name chain.
    #[tokio::test]
    async fn list_with_names_attaches_resolved_names() {
        let state = AppState::for_tests().await;
        let (_ws, a, b) = fixture_workspace_pair(&state).await; // Alpha, Bravo
        repo::inter_agent_message::create(&state.db, &a, &b, "hi", "delivered", true)
            .await
            .expect("persist row");

        let val = list(&state, json!({ "instanceId": a, "withNames": true }))
            .await
            .expect("list failed");
        let arr = val.as_array().expect("array");
        assert_eq!(arr.len(), 1);
        assert_eq!(
            arr[0].get("fromName").and_then(Value::as_str),
            Some("Alpha")
        );
        assert_eq!(arr[0].get("toName").and_then(Value::as_str), Some("Bravo"));
        // Raw ids still present — enrichment is additive.
        assert!(arr[0].get("fromInstanceId").is_some());
    }

    /// UI-path guard: WITHOUT `withNames`, `list` output is byte-for-byte the
    /// plain row serialization — no `fromName`/`toName` keys leak in.
    #[tokio::test]
    async fn list_without_names_has_no_name_keys() {
        let state = AppState::for_tests().await;
        let (_ws, a, b) = fixture_workspace_pair(&state).await;
        repo::inter_agent_message::create(&state.db, &a, &b, "hi", "delivered", true)
            .await
            .expect("persist row");

        let val = list(&state, json!({ "instanceId": a }))
            .await
            .expect("list failed");
        let arr = val.as_array().expect("array");
        assert!(arr[0].get("fromName").is_none(), "no fromName without flag");
        assert!(arr[0].get("toName").is_none(), "no toName without flag");
    }

    /// Same enrichment for the workspace-wide feed.
    #[tokio::test]
    async fn list_for_workspace_with_names_attaches_resolved_names() {
        let state = AppState::for_tests().await;
        let (ws_id, a, b) = fixture_workspace_pair(&state).await; // Alpha, Bravo
        repo::inter_agent_message::create(&state.db, &a, &b, "hi", "delivered", true)
            .await
            .expect("persist row");

        let val = list_for_workspace(&state, json!({ "workspaceId": ws_id, "withNames": true }))
            .await
            .expect("list failed");
        let arr = val.as_array().expect("array");
        assert_eq!(arr.len(), 1);
        assert_eq!(
            arr[0].get("fromName").and_then(Value::as_str),
            Some("Alpha")
        );
        assert_eq!(arr[0].get("toName").and_then(Value::as_str), Some("Bravo"));
    }

    /// UI-path guard for the Chat Hub feed: WITHOUT `withNames`, no name keys.
    #[tokio::test]
    async fn list_for_workspace_without_names_has_no_name_keys() {
        let state = AppState::for_tests().await;
        let (ws_id, a, b) = fixture_workspace_pair(&state).await;
        repo::inter_agent_message::create(&state.db, &a, &b, "hi", "delivered", true)
            .await
            .expect("persist row");

        let val = list_for_workspace(&state, json!({ "workspaceId": ws_id }))
            .await
            .expect("list failed");
        let arr = val.as_array().expect("array");
        assert!(arr[0].get("fromName").is_none(), "no fromName without flag");
        assert!(arr[0].get("toName").is_none(), "no toName without flag");
    }

    #[tokio::test]
    async fn send_to_unknown_session_not_found() {
        let state = AppState::for_tests().await;
        let err = send(&state, json!({ "sessionId": "nope", "text": "hi\n" }))
            .await
            .expect_err("send should fail for unknown session");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn send_to_not_running_session_not_found() {
        let state = AppState::for_tests().await;
        let session_id = fixture_session_id(&state).await;

        // Never spawned → no live backend → NotLive maps to NotFound.
        let err = send(&state, json!({ "sessionId": session_id, "text": "hi\n" }))
            .await
            .expect_err("send should fail for a session with no live backend");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn inject_unknown_target_not_found() {
        let state = AppState::for_tests().await;
        let from = fixture_instance_id(&state, "Sender").await;

        let err = inject(
            &state,
            json!({ "fromInstanceId": from, "toInstanceId": "nope", "text": "hi" }),
        )
        .await
        .expect_err("inject should fail for unknown target");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn inject_unknown_sender_not_found() {
        let state = AppState::for_tests().await;
        let to = fixture_instance_id(&state, "Target").await;

        let err = inject(
            &state,
            json!({ "fromInstanceId": "nope", "toInstanceId": to, "text": "hi" }),
        )
        .await
        .expect_err("inject should fail for unknown sender");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn inject_offline_target_is_held() {
        let state = AppState::for_tests().await;
        let from = fixture_instance_id(&state, "Sender").await;
        let to = fixture_instance_id(&state, "Target").await;

        // `inject` no longer touches the PTY: the row is persisted `held` and
        // the outbox decides when to flush. The NotLive → `queued` outcome now
        // belongs to `flush_against_not_live_target_marks_rows_queued`.
        let val = inject(
            &state,
            json!({ "fromInstanceId": from, "toInstanceId": to, "text": "hi" }),
        )
        .await
        .expect("inject should record (not error) for an offline target");

        assert_eq!(val.get("status").and_then(Value::as_str), Some("held"));
        // A row was persisted (id present).
        assert!(val.get("id").and_then(Value::as_str).is_some());
        assert_eq!(
            val.get("autoSubmitted").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[tokio::test]
    async fn self_inject_deduplicates_endpoint_locks() {
        let state = AppState::for_tests().await;
        let instance_id = fixture_instance_id(&state, "SelfSender").await;

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            inject(
                &state,
                json!({
                    "fromInstanceId": instance_id,
                    "toInstanceId": instance_id,
                    "text": "note to self"
                }),
            ),
        )
        .await
        .expect("self-inject must not deadlock on the same endpoint")
        .expect("eligible self-inject should be held");

        assert_eq!(result.get("status").and_then(Value::as_str), Some("held"));
    }

    #[tokio::test]
    async fn list_unknown_instance_not_found() {
        let state = AppState::for_tests().await;
        let err = list(&state, json!({ "instanceId": "nope" }))
            .await
            .expect_err("list should fail for unknown instance");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn list_returns_in_and_out_newest_first() {
        let state = AppState::for_tests().await;
        let a = fixture_instance_id(&state, "Alpha").await;
        let b = fixture_instance_id(&state, "Bravo").await;

        // a → b (outbox for a), then b → a (inbox for a). Offline targets queue,
        // which is fine — we only care that both rows persist and come back.
        inject(
            &state,
            json!({ "fromInstanceId": a, "toInstanceId": b, "text": "out" }),
        )
        .await
        .expect("first inject failed");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        inject(
            &state,
            json!({ "fromInstanceId": b, "toInstanceId": a, "text": "in" }),
        )
        .await
        .expect("second inject failed");

        let val = list(&state, json!({ "instanceId": a }))
            .await
            .expect("list failed");
        let arr = val.as_array().expect("list returns an array");
        assert_eq!(arr.len(), 2, "both in+out rows for a");
        // Newest first: the b→a inbox row.
        assert_eq!(arr[0].get("text").and_then(Value::as_str), Some("in"));
        assert_eq!(arr[1].get("text").and_then(Value::as_str), Some("out"));
        // camelCase contract surfaces through the command boundary.
        assert!(arr[0].get("fromInstanceId").is_some());
        assert!(arr[0].get("toInstanceId").is_some());
    }

    #[tokio::test]
    async fn inject_live_target_holds_without_writing() {
        let state = AppState::for_tests().await;
        let from = fixture_instance_id(&state, "Sender").await;
        let to = fixture_instance_id(&state, "Target").await;

        // Resolve the target's session id and register a live placeholder so
        // send_stdin succeeds.
        let session_id = session::get_by_instance(&state.db, &to)
            .await
            .expect("get_by_instance failed")
            .expect("session exists")
            .id;
        assert!(state
            .runtime
            .register(&to, LiveHandle::placeholder(&session_id))
            .is_some());

        let val = inject(
            &state,
            json!({ "fromInstanceId": from, "toInstanceId": to, "text": "hi" }),
        )
        .await
        .expect("inject should record for a live target");

        assert_eq!(val.get("status").and_then(Value::as_str), Some("held"));
        assert_eq!(
            val.get("autoSubmitted").and_then(Value::as_bool),
            Some(true)
        );
        assert!(val.get("id").and_then(Value::as_str).is_some());
        assert!(
            state.outbox.deadline(&to).is_some(),
            "the message waits in the target's outbox stack"
        );
    }

    /// Two injects inside the outbox window reach the PTY as ONE bracketed
    /// paste (both tagged lines, FIFO, blank-line separated) followed by the
    /// same escalating CR retries a single message got before — the receiver
    /// handles the whole stack in one turn.
    ///
    /// The submit CR must be RETRIED: a single CR races the receiver's PTY
    /// drain — if it coalesces into the same read burst as the text, the TUI
    /// treats it as paste content and the message sits unsubmitted in the
    /// composer. Three spaced CRs make at least one land as an isolated
    /// keystroke; extra Enters on an already-empty composer are no-ops.
    #[tokio::test]
    async fn stack_flushes_as_one_paste_then_retries_submit_cr() {
        let state = AppState::for_tests().await;
        let from = fixture_instance_id(&state, "Sender").await;
        let to = fixture_instance_id(&state, "Target").await;
        let session_id = session::get_by_instance(&state.db, &to)
            .await
            .expect("get_by_instance failed")
            .expect("session exists")
            .id;
        let (handle, mut rx) = LiveHandle::for_test_pty(&session_id);
        assert!(state.runtime.register(&to, handle).is_some());

        let first = inject(
            &state,
            json!({ "fromInstanceId": from, "toInstanceId": to, "text": "hi" }),
        )
        .await
        .expect("inject accepted");
        let second = inject(
            &state,
            json!({ "fromInstanceId": from, "toInstanceId": to, "text": "again" }),
        )
        .await
        .expect("inject accepted");
        assert_eq!(first["status"], "held");
        assert_eq!(second["status"], "held");
        assert!(rx.try_recv().is_err(), "nothing reaches the PTY while held");

        let items = state.outbox.take_all(&to);
        assert_eq!(items.len(), 2);
        let status = flush_stack(&state, &to, items).await;
        assert_eq!(status, "delivered");

        let mut writes = Vec::new();
        while let Ok(w) = rx.try_recv() {
            writes.push(w);
        }
        assert_eq!(
            writes.len(),
            1 + SUBMIT_CR_DELAYS_MS.len(),
            "expected ONE body paste + one CR per retry slot, got {writes:?}"
        );
        // The body is ONE bracketed paste: the receiver accumulates it across
        // PTY reads (macOS hands a TUI at most 1022 bytes per read) instead of
        // keeping only the last burst chunk — the head-truncation bug of
        // 2026-09-04 (docs/superpowers/plans/2026-09-04-inject-bracketed-paste.md).
        let body = &writes[0];
        assert!(
            body.starts_with("\x1b[200~[from Sender · "),
            "bracketed paste opens with the first tag, got {body:?}"
        );
        assert!(
            body.ends_with("] again\x1b[201~"),
            "paste closes after the last message, got {body:?}"
        );
        assert!(
            body.contains("] hi\n\n[from Sender · "),
            "messages are FIFO and blank-line separated, got {body:?}"
        );
        for cr in &writes[1..] {
            assert_eq!(cr, "\r", "every follow-up write is a bare Enter");
        }

        let rows = repo::inter_agent_message::list_for_instance(&state.db, &to, 10)
            .await
            .unwrap();
        assert!(
            rows.iter().all(|r| r.status == "delivered"),
            "flush marks every row delivered: {rows:?}"
        );
    }

    /// Reaching MAX_ITEMS flushes synchronously inside `inject`: the 10th ack
    /// already reads `delivered` and the PTY has exactly one paste.
    #[tokio::test]
    async fn tenth_inject_flushes_synchronously() {
        let state = AppState::for_tests().await;
        let from = fixture_instance_id(&state, "Sender").await;
        let to = fixture_instance_id(&state, "Target").await;
        let session_id = session::get_by_instance(&state.db, &to)
            .await
            .unwrap()
            .unwrap()
            .id;
        let (handle, mut rx) = LiveHandle::for_test_pty(&session_id);
        assert!(state.runtime.register(&to, handle).is_some());

        let mut last = Value::Null;
        for n in 1..=crate::engine::runtime::outbox::MAX_ITEMS {
            last = inject(
                &state,
                json!({ "fromInstanceId": from, "toInstanceId": to, "text": format!("m{n}") }),
            )
            .await
            .unwrap();
        }
        assert_eq!(last["status"], "delivered");
        let mut writes = Vec::new();
        while let Ok(w) = rx.try_recv() {
            writes.push(w);
        }
        assert_eq!(writes.len(), 1 + SUBMIT_CR_DELAYS_MS.len());
        assert!(writes[0].contains("] m1\n\n") && writes[0].ends_with("] m10\x1b[201~"));
        assert!(
            state.outbox.take_all(&to).is_empty(),
            "cap flush emptied the stack"
        );
    }

    /// `flush_stack` waits on the target's flush lock BEFORE it touches the
    /// DB or the PTY. Holding that lock from outside therefore freezes a
    /// delivery mid-flight — which is exactly the window the sweeper-vs-cap
    /// overtake needed. This is the test that actually discriminates: comment
    /// out the two `flush_lock` lines in `flush_stack` and it fails, because
    /// the paste lands while the guard is still held.
    ///
    /// (The non-interleaving property alone does NOT discriminate: the target's
    /// agent lifecycle mutex is already held across the paste and the whole CR
    /// loop, so writes never interleave even without the flush lock.)
    #[tokio::test]
    async fn flush_stack_waits_for_the_targets_flush_lock() {
        let state = std::sync::Arc::new(AppState::for_tests().await);
        let from = fixture_instance_id(&state, "Sender").await;
        let to = fixture_instance_id(&state, "Target").await;
        let session_id = session::get_by_instance(&state.db, &to)
            .await
            .unwrap()
            .unwrap()
            .id;
        let (handle, mut rx) = LiveHandle::for_test_pty(&session_id);
        assert!(state.runtime.register(&to, handle).is_some());

        inject(
            &state,
            json!({ "fromInstanceId": from, "toInstanceId": to, "text": "held by the lock" }),
        )
        .await
        .unwrap();
        let items = state.outbox.take_all(&to);

        // Take the target's flush lock the way an in-flight flush would.
        let held = state.outbox.flush_lock(&to);
        let guard = held.lock().await;

        let flushing = tokio::spawn({
            let state = std::sync::Arc::clone(&state);
            let to = to.clone();
            async move { flush_stack(&state, &to, items).await }
        });

        // Give the spawned flush every chance to run ahead of us.
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            rx.try_recv().is_err(),
            "nothing may reach the PTY while the target's flush lock is held"
        );

        drop(guard);
        assert_eq!(flushing.await.expect("flush task panicked"), "delivered");
        let first = rx
            .try_recv()
            .expect("the paste lands once the lock is free");
        assert!(first.starts_with("\x1b[200~") && first.ends_with("\x1b[201~"));
    }

    /// Two flushes racing for ONE target never interleave their writes: each
    /// stack's bracketed paste and its submit CRs come out as one contiguous
    /// run. NOTE this invariant is carried by the target's agent lifecycle
    /// mutex (held across the paste AND the CR loop), so it holds with or
    /// without the flush lock — the lock's own guard is
    /// `flush_stack_waits_for_the_targets_flush_lock` above.
    #[tokio::test]
    async fn concurrent_flushes_for_one_target_do_not_interleave() {
        let state = AppState::for_tests().await;
        let from = fixture_instance_id(&state, "Sender").await;
        let to = fixture_instance_id(&state, "Target").await;
        let session_id = session::get_by_instance(&state.db, &to)
            .await
            .unwrap()
            .unwrap()
            .id;
        let (handle, mut rx) = LiveHandle::for_test_pty(&session_id);
        assert!(state.runtime.register(&to, handle).is_some());

        inject(
            &state,
            json!({ "fromInstanceId": from, "toInstanceId": to, "text": "a" }),
        )
        .await
        .unwrap();
        let stack_a = state.outbox.take_all(&to);
        inject(
            &state,
            json!({ "fromInstanceId": from, "toInstanceId": to, "text": "b" }),
        )
        .await
        .unwrap();
        let stack_b = state.outbox.take_all(&to);
        assert_eq!(stack_a.len(), 1);
        assert_eq!(stack_b.len(), 1);

        tokio::join!(
            flush_stack(&state, &to, stack_a),
            flush_stack(&state, &to, stack_b)
        );

        let mut writes = Vec::new();
        while let Ok(w) = rx.try_recv() {
            writes.push(w);
        }
        let run = 1 + SUBMIT_CR_DELAYS_MS.len();
        assert_eq!(
            writes.len(),
            2 * run,
            "each stack contributes one paste + its CRs, got {writes:?}"
        );
        for start in [0, run] {
            assert!(
                writes[start].starts_with("\x1b[200~") && writes[start].ends_with("\x1b[201~"),
                "write {start} must be a whole bracketed paste, got {:?}",
                writes[start]
            );
            for cr in &writes[start + 1..start + run] {
                assert_eq!(
                    cr, "\r",
                    "a paste's submit CRs must not be split by the other flush: {writes:?}"
                );
            }
        }
    }

    /// A human routed send (`immediate: true`) is delivered on its own, right
    /// now — and the target's pending stack stays exactly as it was.
    #[tokio::test]
    async fn immediate_inject_delivers_now_and_leaves_the_stack_alone() {
        let state = AppState::for_tests().await;
        let from = fixture_instance_id(&state, "Sender").await;
        let to = fixture_instance_id(&state, "Target").await;
        let session_id = session::get_by_instance(&state.db, &to)
            .await
            .unwrap()
            .unwrap()
            .id;
        let (handle, mut rx) = LiveHandle::for_test_pty(&session_id);
        assert!(state.runtime.register(&to, handle).is_some());

        inject(
            &state,
            json!({ "fromInstanceId": from, "toInstanceId": to, "text": "pending" }),
        )
        .await
        .unwrap();
        let ack = inject(
            &state,
            json!({ "fromInstanceId": from, "toInstanceId": to, "text": "from the human", "immediate": true }),
        )
        .await
        .unwrap();
        assert_eq!(ack["status"], "delivered");

        let mut writes = Vec::new();
        while let Ok(w) = rx.try_recv() {
            writes.push(w);
        }
        assert_eq!(
            writes.len(),
            1 + SUBMIT_CR_DELAYS_MS.len(),
            "one paste for the immediate message only"
        );
        assert!(writes[0].ends_with("] from the human\x1b[201~"));
        assert!(
            !writes[0].contains("pending"),
            "the held message was NOT flushed along"
        );

        let still_held = state.outbox.take_all(&to);
        assert_eq!(still_held.len(), 1);
        assert_eq!(still_held[0].text, "pending");
    }

    /// A target that is not live at flush time gets `queued` rows, no PTY
    /// write, and no event — the same outcome an offline target had before.
    #[tokio::test]
    async fn flush_against_not_live_target_marks_rows_queued() {
        let state = AppState::for_tests().await;
        let from = fixture_instance_id(&state, "Sender").await;
        let to = fixture_instance_id(&state, "Target").await; // never registered → NotLive
        inject(
            &state,
            json!({ "fromInstanceId": from, "toInstanceId": to, "text": "hi" }),
        )
        .await
        .unwrap();
        let items = state.outbox.take_all(&to);
        assert_eq!(flush_stack(&state, &to, items).await, "queued");
        let rows = repo::inter_agent_message::list_for_instance(&state.db, &to, 10)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, "queued");
    }

    /// A chat backend has no terminal: the paste envelope is a PTY-only
    /// concern, so a non-PTY handle must receive the tagged body raw.
    #[tokio::test]
    async fn inject_non_pty_target_gets_raw_body() {
        let state = AppState::for_tests().await;
        let from = fixture_instance_id(&state, "Sender").await;
        let to = fixture_instance_id(&state, "Target").await;

        let session_id = session::get_by_instance(&state.db, &to)
            .await
            .expect("get_by_instance failed")
            .expect("session exists")
            .id;
        let (handle, mut rx) = LiveHandle::for_test(&session_id);
        assert!(state.runtime.register(&to, handle).is_some());

        inject(
            &state,
            json!({ "fromInstanceId": from, "toInstanceId": to, "text": "hi" }),
        )
        .await
        .expect("inject should deliver to a live target");
        flush_stack(&state, &to, state.outbox.take_all(&to)).await;

        let first = rx.try_recv().expect("tagged body write");
        assert!(
            first.starts_with("[from ") && first.ends_with("] hi"),
            "non-PTY body is raw (no ESC[200~ envelope), got {first:?}"
        );
    }

    /// `message.send` is the raw keystroke path (Terminal pane, submit CR).
    /// Only an explicit `paste: true` opts a text write into the envelope, and
    /// only on a PTY handle.
    #[tokio::test]
    async fn send_paste_flag_wraps_text_on_pty_only() {
        let state = AppState::for_tests().await;
        let to = fixture_instance_id(&state, "Target").await;
        let session_id = session::get_by_instance(&state.db, &to)
            .await
            .expect("get_by_instance failed")
            .expect("session exists")
            .id;
        let (handle, mut rx) = LiveHandle::for_test_pty(&session_id);
        assert!(state.runtime.register(&to, handle).is_some());

        send(&state, json!({ "sessionId": session_id, "text": "hello" }))
            .await
            .expect("raw send");
        assert_eq!(rx.try_recv().expect("raw write"), "hello");

        send(
            &state,
            json!({ "sessionId": session_id, "text": "hello", "paste": true }),
        )
        .await
        .expect("paste send");
        assert_eq!(
            rx.try_recv().expect("paste write"),
            "\x1b[200~hello\x1b[201~"
        );

        send(&state, json!({ "sessionId": session_id, "text": "\r" }))
            .await
            .expect("cr send");
        assert_eq!(
            rx.try_recv().expect("cr write"),
            "\r",
            "Enter stays a keystroke"
        );
    }

    #[tokio::test]
    async fn stopped_workspace_or_agent_rejects_inject_without_delivery_or_queue() {
        let state = AppState::for_tests().await;
        let (workspace_id, from, to) = fixture_workspace_pair(&state).await;
        let session_id = session::get_by_instance(&state.db, &to)
            .await
            .unwrap()
            .unwrap()
            .id;
        let (handle, mut rx) = LiveHandle::for_test(&session_id);
        assert!(state.runtime.register(&to, handle).is_some());

        workspace::set_run_state(&state.db, &workspace_id, "stopped")
            .await
            .unwrap();
        assert!(matches!(
            inject(
                &state,
                json!({ "fromInstanceId": from, "toInstanceId": to, "text": "blocked" }),
            )
            .await,
            Err(AppError::Invalid(_))
        ));
        assert!(
            rx.try_recv().is_err(),
            "no stdin write after workspace stop"
        );
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM inter_agent_message WHERE to_instance_id=?")
                .bind(&to)
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(count, 0, "rejection must not create a queued row");

        workspace::set_run_state(&state.db, &workspace_id, "started")
            .await
            .unwrap();
        workspace_agent::set_availability(&state.db, &to, "stopped")
            .await
            .unwrap();
        assert!(matches!(
            inject(
                &state,
                json!({ "fromInstanceId": from, "toInstanceId": to, "text": "blocked" }),
            )
            .await,
            Err(AppError::Invalid(_))
        ));
        assert!(rx.try_recv().is_err(), "no stdin write after agent stop");
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM inter_agent_message WHERE to_instance_id=?")
                .bind(&to)
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn stopped_source_agent_rejects_inject_without_delivery_or_queue() {
        let state = AppState::for_tests().await;
        let (_workspace_id, from, to) = fixture_workspace_pair(&state).await;
        let session_id = session::get_by_instance(&state.db, &to)
            .await
            .unwrap()
            .unwrap()
            .id;
        let (handle, mut rx) = LiveHandle::for_test(&session_id);
        assert!(state.runtime.register(&to, handle).is_some());

        workspace_agent::set_availability(&state.db, &from, "stopped")
            .await
            .unwrap();
        assert!(matches!(
            inject(
                &state,
                json!({ "fromInstanceId": from, "toInstanceId": to, "text": "blocked-source" }),
            )
            .await,
            Err(AppError::Invalid(_))
        ));
        assert!(rx.try_recv().is_err(), "stopped source must not deliver");
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM inter_agent_message \
             WHERE from_instance_id=? AND text='blocked-source'",
        )
        .bind(&from)
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(count, 0, "rejection must not create a queued row");
    }

    #[tokio::test]
    async fn stopped_source_workspace_rejects_cross_workspace_inject_without_queue() {
        let state = AppState::for_tests().await;
        let from = fixture_instance_id(&state, "Sender").await;
        let to = fixture_instance_id(&state, "Target").await;
        let source = workspace_agent::runtime_eligibility(&state.db, &from)
            .await
            .unwrap()
            .unwrap();
        let session_id = session::get_by_instance(&state.db, &to)
            .await
            .unwrap()
            .unwrap()
            .id;
        let (handle, mut rx) = LiveHandle::for_test(&session_id);
        assert!(state.runtime.register(&to, handle).is_some());

        workspace::set_run_state(&state.db, &source.workspace_id, "stopped")
            .await
            .unwrap();
        assert!(matches!(
            inject(
                &state,
                json!({ "fromInstanceId": from, "toInstanceId": to, "text": "blocked-workspace" }),
            )
            .await,
            Err(AppError::Invalid(_))
        ));
        assert!(
            rx.try_recv().is_err(),
            "stopped source workspace must not deliver"
        );
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM inter_agent_message \
             WHERE from_instance_id=? AND text='blocked-workspace'",
        )
        .bind(&from)
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(count, 0, "rejection must not create a queued row");
    }

    #[tokio::test]
    async fn stopped_workspace_rejects_direct_send_without_stdin_write() {
        let state = AppState::for_tests().await;
        let session_id = fixture_session_id(&state).await;
        let session = session::get(&state.db, &session_id).await.unwrap().unwrap();
        let eligibility =
            workspace_agent::runtime_eligibility(&state.db, &session.workspace_agent_id)
                .await
                .unwrap()
                .unwrap();
        let (handle, mut rx) = LiveHandle::for_test(&session_id);
        assert!(state
            .runtime
            .register(&session.workspace_agent_id, handle)
            .is_some());
        workspace::set_run_state(&state.db, &eligibility.workspace_id, "stopped")
            .await
            .unwrap();
        assert!(matches!(
            send(
                &state,
                json!({ "sessionId": session_id, "text": "blocked" })
            )
            .await,
            Err(AppError::Invalid(_))
        ));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn agent_stop_race_either_commits_delivery_before_stop_or_rejects_without_row() {
        let state = std::sync::Arc::new(AppState::for_tests().await);
        let (_workspace_id, from, to) = fixture_workspace_pair(&state).await;
        let session_id = session::get_by_instance(&state.db, &to)
            .await
            .unwrap()
            .unwrap()
            .id;
        let (handle, _rx) = LiveHandle::for_test(&session_id);
        assert!(state.runtime.register(&to, handle).is_some());

        let (delivery, stopped) = tokio::join!(
            inject(
                &state,
                json!({ "fromInstanceId": from, "toInstanceId": to, "text": "race" }),
            ),
            super::super::instance::stop(&state, json!({ "workspaceAgentId": to })),
        );
        stopped.unwrap();
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM inter_agent_message WHERE to_instance_id=? AND text='race'",
        )
        .bind(&to)
        .fetch_one(&state.db)
        .await
        .unwrap();
        match delivery {
            Ok(_) => assert_eq!(count, 1, "delivery that won must be durable"),
            Err(AppError::Invalid(_)) => {
                assert_eq!(count, 0, "delivery that lost must not queue")
            }
            other => panic!("unexpected race outcome: {other:?}"),
        }
        assert!(!state.runtime.is_live(&to));
        assert_eq!(
            workspace_agent::get(&state.db, &to)
                .await
                .unwrap()
                .unwrap()
                .availability,
            "stopped"
        );
    }

    #[tokio::test]
    async fn source_agent_stop_race_rejects_after_stop_wins_without_row() {
        let state = std::sync::Arc::new(AppState::for_tests().await);
        let (_workspace_id, from, to) = fixture_workspace_pair(&state).await;
        let session_id = session::get_by_instance(&state.db, &to)
            .await
            .unwrap()
            .unwrap()
            .id;
        let (handle, mut rx) = LiveHandle::for_test(&session_id);
        assert!(state.runtime.register(&to, handle).is_some());

        // Hold the source mutex while queueing Stop first, then inject. Tokio's
        // mutex is FIFO, so releasing this guard makes Stop the lifecycle
        // winner. A correct inject waits behind it and rejects before delivery.
        let source_lock = state.agent_lifecycle_lock(&from);
        let source_guard = source_lock.lock().await;
        let stop_state = std::sync::Arc::clone(&state);
        let stop_from = from.clone();
        let stop_task = tokio::spawn(async move {
            super::super::instance::stop(&stop_state, json!({ "workspaceAgentId": stop_from }))
                .await
        });
        tokio::task::yield_now().await;

        let inject_state = std::sync::Arc::clone(&state);
        let inject_from = from.clone();
        let inject_to = to.clone();
        let inject_task = tokio::spawn(async move {
            inject(
                &inject_state,
                json!({
                    "fromInstanceId": inject_from,
                    "toInstanceId": inject_to,
                    "text": "source-race"
                }),
            )
            .await
        });
        tokio::task::yield_now().await;
        drop(source_guard);

        stop_task.await.unwrap().unwrap();
        assert!(matches!(
            inject_task.await.unwrap(),
            Err(AppError::Invalid(_))
        ));
        assert!(rx.try_recv().is_err(), "losing inject must not deliver");
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM inter_agent_message \
             WHERE from_instance_id=? AND text='source-race'",
        )
        .bind(&from)
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(count, 0, "losing inject must not persist a row");
    }

    #[tokio::test]
    async fn workspace_stop_race_either_commits_delivery_before_stop_or_rejects_without_row() {
        let state = std::sync::Arc::new(AppState::for_tests().await);
        let (workspace_id, from, to) = fixture_workspace_pair(&state).await;
        let session_id = session::get_by_instance(&state.db, &to)
            .await
            .unwrap()
            .unwrap()
            .id;
        let (handle, _rx) = LiveHandle::for_test(&session_id);
        assert!(state.runtime.register(&to, handle).is_some());

        let (delivery, stopped) = tokio::join!(
            inject(
                &state,
                json!({ "fromInstanceId": from, "toInstanceId": to, "text": "workspace-race" }),
            ),
            super::super::workspace::stop(&state, json!({ "workspaceId": workspace_id }),),
        );
        stopped.unwrap();
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM inter_agent_message \
             WHERE to_instance_id=? AND text='workspace-race'",
        )
        .bind(&to)
        .fetch_one(&state.db)
        .await
        .unwrap();
        match delivery {
            Ok(_) => assert_eq!(count, 1, "delivery that won must be durable"),
            Err(AppError::Invalid(_)) => {
                assert_eq!(count, 0, "delivery that lost must not queue")
            }
            other => panic!("unexpected race outcome: {other:?}"),
        }
        assert_eq!(
            workspace::get(&state.db, &workspace_id)
                .await
                .unwrap()
                .unwrap()
                .run_state,
            "stopped"
        );
        assert!(!state.runtime.is_live(&to));
    }
}
