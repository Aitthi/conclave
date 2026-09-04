use crate::engine::{bus, repo, runtime::StdinError, AppError, AppState};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
}

/// Inject a line of text into a TARGET instance's live input, auto-submit it,
/// tag the origin (sender), persist an `InterAgentMessage`, and emit a bus event
/// so the UI can render the injection ("injected from X · auto-submitted").
///
/// This is the inter-agent messaging backbone (M3.1).
///
/// **Auto-submit:** the tagged body is written as one bracketed paste, then
/// submitted with separate spaced `\r` keystrokes (see [`SUBMIT_CR_DELAYS_MS`]);
/// a chat backend receives the body raw and needs no Enter.
///
/// **Origin tag:** the origin is carried as `from_instance_id` on the persisted
/// row AND on the `message:injected` event. We deliberately inject the RAW
/// `text` into the target's stdin (no marker pollution of the agent's actual
/// input); the UI renders the "injected from X" chrome from the event/row. The
/// visible origin-tagged bubble/line is the M3.2 UI task.
///
/// Status resolution:
/// - target live, stdin accepted → `"delivered"` (+ emit `message:injected`)
/// - target not running ([`StdinError::NotLive`]) → `"queued"` (recorded, not
///   delivered; no error, no event)
/// - backend stdin channel closed ([`StdinError::Closed`]) → [`AppError::Internal`]
///
/// Errors:
/// - malformed payload → [`AppError::Invalid`]
/// - unknown sender OR target instance → [`AppError::NotFound`]
pub async fn inject(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let InjectReq {
        from_instance_id,
        to_instance_id,
        text,
    } = serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    let eligibility = require_delivery_eligible(state, &to_instance_id).await?;
    let workspace_lock = state.workspace_lifecycle_lock(&eligibility.workspace_id);
    let _workspace_guard = workspace_lock.read().await;
    let agent_lock = state.agent_lifecycle_lock(&to_instance_id);
    let _agent_guard = agent_lock.lock().await;
    require_delivery_eligible(state, &to_instance_id).await?;

    // Validate BOTH instances exist — name which one is missing for clarity.
    // Resolving the sender row here doubles as its existence check AND yields the
    // display name for the `[from …]` tag, so we don't query the sender twice.
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
    // Prefix the delivered input with the sender's name AND id so the receiving
    // agent knows who it's from and can reply directly with `conclave tell <id>`
    // (no roster lookup needed). The persisted row + UI carry origin separately,
    // so `text` stays RAW for those — only the stdin line is tagged.
    // The body goes out as ONE bracketed paste (PTY backends): a body longer
    // than the kernel's PTY input queue (macOS: 1022 bytes) reaches the TUI as
    // several reads, and Claude Code's un-bracketed burst handling keeps only
    // the LAST read — the receiver then submits just the tail (a 1023-byte
    // tell arrived as "."; docs/superpowers/plans/2026-09-04-inject-bracketed-paste.md).
    // Inside the envelope the TUI reassembles the whole body regardless of
    // read boundaries, exactly as it does for a human's terminal paste.
    let body = format!("[from {sender} · {from_instance_id}] {text}");
    let status = match state.runtime.send_stdin_paste(&to_instance_id, &body) {
        // Delivered to a live backend — now SUBMIT it. A TUI's Enter is CR (\r),
        // not LF; and a CR inside the paste envelope (or, on a non-bracketed
        // burst, in the SAME write as the text) is literal paste content
        // (cursor drops to a new line, nothing submits). Worse, a SINGLE spaced
        // CR still races the receiver's PTY drain: under load the 40ms beat
        // elapses before the text is read, the CR coalesces into the same read
        // burst, and the message sits unsubmitted in the composer. So press
        // Enter at escalating gaps — at least one CR lands as an isolated
        // keystroke, and extra Enters on an already-empty composer are no-ops.
        Ok(()) => {
            for delay_ms in SUBMIT_CR_DELAYS_MS {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                let _ = state.runtime.send_stdin(&to_instance_id, "\r");
            }
            "delivered"
        }
        // Target isn't running: RECORD the message as queued but do NOT error and
        // do NOT emit the delivered event.
        // TODO(M3.x): deliver-on-spawn — drain queued messages when the target
        // becomes live. No queue drain yet.
        Err(StdinError::NotLive) => "queued",
        // Registered-but-closed channel is a backend fault, not a bad request.
        // We return early WITHOUT persisting — nothing was delivered or recorded.
        Err(StdinError::Closed) => {
            return Err(AppError::Internal(format!(
                "target instance {to_instance_id} backend stdin channel closed"
            )));
        }
    };

    // Persist the injection FIRST (auto_submitted always true for an injection),
    // then emit. Persist-before-emit means the UI never receives a `delivered`
    // event for a message that failed to record — "delivered" implies both sent
    // AND recorded. `create` borrows its args, so they're still owned for the emit.
    let row = repo::inter_agent_message::create(
        &state.db,
        &from_instance_id,
        &to_instance_id,
        &text,
        status,
        true,
    )
    .await?;

    // Emit only once the delivered row is durably persisted, so the UI can render
    // the injection ("injected from X · auto-submitted").
    if status == "delivered" {
        state.emit(
            bus::MESSAGE_INJECTED,
            bus::MessageInjected {
                to_instance_id,
                to_session_id: state.runtime.session_id(&row.to_instance_id),
                from_instance_id,
                text,
                auto_submitted: true,
            },
        );
    }

    serde_json::to_value(row).map_err(|e| AppError::Internal(e.to_string()))
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
    async fn inject_offline_target_queues() {
        let state = AppState::for_tests().await;
        let from = fixture_instance_id(&state, "Sender").await;
        let to = fixture_instance_id(&state, "Target").await;

        // Target exists but is NOT registered in the runtime → NotLive → queued.
        let val = inject(
            &state,
            json!({ "fromInstanceId": from, "toInstanceId": to, "text": "hi" }),
        )
        .await
        .expect("inject should record (not error) for an offline target");

        assert_eq!(val.get("status").and_then(Value::as_str), Some("queued"));
        // A row was persisted (id present).
        assert!(val.get("id").and_then(Value::as_str).is_some());
        assert_eq!(
            val.get("autoSubmitted").and_then(Value::as_bool),
            Some(true)
        );
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
    async fn inject_live_target_delivers() {
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
        .expect("inject should deliver to a live target");

        assert_eq!(val.get("status").and_then(Value::as_str), Some("delivered"));
        assert_eq!(
            val.get("autoSubmitted").and_then(Value::as_bool),
            Some(true)
        );
        assert!(val.get("id").and_then(Value::as_str).is_some());
    }

    /// The submit CR must be RETRIED: a single CR races the receiver's PTY
    /// drain — if it coalesces into the same read burst as the text, the TUI
    /// treats it as paste content and the message sits unsubmitted in the
    /// composer. Three spaced CRs make at least one land as an isolated
    /// keystroke; extra Enters on an already-empty composer are no-ops.
    #[tokio::test]
    async fn inject_live_target_retries_submit_cr() {
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

        inject(
            &state,
            json!({ "fromInstanceId": from, "toInstanceId": to, "text": "hi" }),
        )
        .await
        .expect("inject should deliver to a live target");

        let mut writes = Vec::new();
        while let Ok(w) = rx.try_recv() {
            writes.push(w);
        }
        assert_eq!(
            writes.len(),
            1 + SUBMIT_CR_DELAYS_MS.len(),
            "expected tagged body + one CR per retry slot, got {writes:?}"
        );
        // The body is ONE bracketed paste: the receiver accumulates it across
        // PTY reads (macOS hands a TUI at most 1022 bytes per read) instead of
        // keeping only the last burst chunk — the head-truncation bug of
        // 2026-09-04 (docs/superpowers/plans/2026-09-04-inject-bracketed-paste.md).
        assert!(
            writes[0].starts_with("\x1b[200~[from ") && writes[0].ends_with("] hi\x1b[201~"),
            "first write is the tagged body inside a bracketed-paste envelope, got {:?}",
            writes[0]
        );
        for cr in &writes[1..] {
            assert_eq!(cr, "\r", "every follow-up write is a bare Enter");
        }
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
}
