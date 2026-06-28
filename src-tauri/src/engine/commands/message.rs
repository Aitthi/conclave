use crate::engine::{bus, repo, runtime::StdinError, AppError, AppState};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Payload for `message.send` — a line of user input destined for a live session.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendReq {
    session_id: String,
    text: String,
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
    let SendReq { session_id, text } =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    let session = repo::session::get(&state.db, &session_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("session id={session_id} not found")))?;

    // Route the text to the live PTY keyed by the owning workspace_agent. The
    // ack below still owns `text` — `send_stdin` borrows it.
    state
        .runtime
        .send_stdin(&session.workspace_agent_id, &text)
        .map_err(|e| match e {
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
/// **Auto-submit:** a trailing newline is appended to `text` before routing to
/// stdin — this mirrors how the frontend StdinBar submits (the chat backend
/// trims it, the CLI PTY submits on it).
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

    // Validate BOTH instances exist — name which one is missing for clarity.
    if !repo::workspace_agent::exists(&state.db, &from_instance_id).await? {
        return Err(AppError::NotFound(format!(
            "sender instance id={from_instance_id} not found"
        )));
    }
    if !repo::workspace_agent::exists(&state.db, &to_instance_id).await? {
        return Err(AppError::NotFound(format!(
            "target instance id={to_instance_id} not found"
        )));
    }

    // Auto-submit = append a newline, then route to the target's live backend.
    let line = format!("{text}\n");
    let status = match state.runtime.send_stdin(&to_instance_id, &line) {
        // Delivered to a live backend.
        Ok(()) => "delivered",
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
    let ListReq { instance_id, limit } =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    if !repo::workspace_agent::exists(&state.db, &instance_id).await? {
        return Err(AppError::NotFound(format!(
            "instance id={instance_id} not found"
        )));
    }

    let limit = limit.unwrap_or(DEFAULT_LIST_LIMIT).clamp(1, MAX_LIST_LIMIT);
    let rows = repo::inter_agent_message::list_for_instance(&state.db, &instance_id, limit).await?;
    serde_json::to_value(rows).map_err(|e| AppError::Internal(e.to_string()))
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
            .register(&to, LiveHandle::placeholder(&session_id)));

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
}
