use crate::engine::{repo, runtime::StdinError, AppError, AppState};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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

// TODO(M3): real impl — inject a system/tool message into an instance conversation
#[allow(dead_code)]
pub async fn inject(_state: &AppState, _payload: Value) -> Result<Value, AppError> {
    Ok(json!({ "stub": "message.inject", "todo": true }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::repo::{
        agent_definition::{self, AgentDefinitionInput},
        session, workspace, workspace_agent,
    };

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
}
