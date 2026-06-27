//! Typed event-bus helpers for emitting Tauri events to the UI.
//!
//! Every event constant and payload struct here must stay in sync with
//! `src/ipc/events.ts`. The serialisation tests in `#[cfg(test)]` below
//! enforce the camelCase field-name contract at compile time.
//!
//! Foundation module: the emit primitives are wired but not yet called — the
//! agent runtime starts emitting in M2. Allow dead_code until then.
#![allow(dead_code)]

use serde::Serialize;
use tauri::{AppHandle, Emitter};

// ---------------------------------------------------------------------------
// Event-name constants — must match EVENT_NAMES in src/ipc/events.ts
// ---------------------------------------------------------------------------

pub const SESSION_OUTPUT: &str = "session:output";
pub const SESSION_STATUS: &str = "session:status";
pub const FUSION_STAGE: &str = "fusion:stage";
pub const MESSAGE_INJECTED: &str = "message:injected";

// ---------------------------------------------------------------------------
// Payload structs
// ---------------------------------------------------------------------------

/// Payload for `session:output` — a streamed output chunk from a session.
///
/// Serialises to `{ "sessionId": "...", "chunk": "..." }` (camelCase) to
/// match the `SessionOutputEvent` interface in `src/ipc/events.ts`.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SessionOutput {
    pub session_id: String,
    pub chunk: String,
}

/// Payload for `session:status` — lifecycle status change for a session.
///
/// `status` must be one of `"running"`, `"idle"`, or `"waiting"` to match
/// the `SessionStatusEvent` union type in `src/ipc/events.ts`.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatus {
    pub session_id: String,
    pub status: String,
}

/// Payload for `fusion:stage` — stage progress in a multi-agent fusion run.
///
/// `stage` must be one of `"panel"`, `"judge"`, or `"synthesize"`.
/// `data` is optional; it serialises as `null` when absent, which is safe
/// for the `data?` field in the TypeScript `FusionStageEvent` interface.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FusionStage {
    pub run_id: String,
    pub stage: String,
    pub data: Option<serde_json::Value>,
}

/// Payload for `message:injected` — an inter-agent injection delivered to a
/// target instance's live input (M3.1 message router).
///
/// Emitted only when the injection is actually DELIVERED (the target is live),
/// so `to_session_id` is normally `Some`. It is `Option` because the live
/// session id is resolved from the runtime registry; `skip_serializing_if`
/// omits the key entirely when `None` (matching the optional `toSessionId?`
/// field in the `MessageInjectedEvent` TS interface — `string | undefined`, not
/// `null`). The origin is carried as `from_instance_id` — the UI renders the
/// "injected from X" chrome from this event (the raw `text` was injected into
/// stdin with no marker pollution).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MessageInjected {
    pub to_instance_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_session_id: Option<String>,
    pub from_instance_id: String,
    pub text: String,
    pub auto_submitted: bool,
}

// ---------------------------------------------------------------------------
// Generic emit helper
// ---------------------------------------------------------------------------

/// Emit any serialisable payload to every window/webview via the Tauri event
/// system.  Wraps `AppHandle::emit` from the `Emitter` trait (Tauri v2).
pub fn emit<T: Serialize + Clone>(app: &AppHandle, event: &str, payload: T) -> tauri::Result<()> {
    app.emit(event, payload)
}

// ---------------------------------------------------------------------------
// Typed convenience emitters
// ---------------------------------------------------------------------------

/// Emit a `session:output` event carrying a streamed chunk.
pub fn session_output(app: &AppHandle, payload: SessionOutput) -> tauri::Result<()> {
    emit(app, SESSION_OUTPUT, payload)
}

/// Emit a `session:status` event carrying a lifecycle status change.
pub fn session_status(app: &AppHandle, payload: SessionStatus) -> tauri::Result<()> {
    emit(app, SESSION_STATUS, payload)
}

/// Emit a `fusion:stage` event carrying stage-progress data.
pub fn fusion_stage(app: &AppHandle, payload: FusionStage) -> tauri::Result<()> {
    emit(app, FUSION_STAGE, payload)
}

/// Emit a `message:injected` event carrying a delivered inter-agent injection.
pub fn message_injected(app: &AppHandle, payload: MessageInjected) -> tauri::Result<()> {
    emit(app, MESSAGE_INJECTED, payload)
}

// ---------------------------------------------------------------------------
// Contract tests
//
// These tests do NOT require a running Tauri application — they verify only
// the JSON serialisation shape, which is the Rust↔TypeScript wire contract.
// Run with: cargo test --lib engine::bus
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn session_output_camel_case() {
        let val = serde_json::to_value(SessionOutput {
            session_id: "s1".into(),
            chunk: "hi".into(),
        })
        .unwrap();
        assert_eq!(val, json!({ "sessionId": "s1", "chunk": "hi" }));
    }

    #[test]
    fn session_status_camel_case() {
        let val = serde_json::to_value(SessionStatus {
            session_id: "s2".into(),
            status: "running".into(),
        })
        .unwrap();
        assert_eq!(val, json!({ "sessionId": "s2", "status": "running" }));
    }

    #[test]
    fn fusion_stage_camel_case_with_data() {
        let val = serde_json::to_value(FusionStage {
            run_id: "r1".into(),
            stage: "panel".into(),
            data: Some(json!({ "count": 3 })),
        })
        .unwrap();
        assert_eq!(
            val,
            json!({ "runId": "r1", "stage": "panel", "data": { "count": 3 } })
        );
    }

    #[test]
    fn message_injected_camel_case() {
        let val = serde_json::to_value(MessageInjected {
            to_instance_id: "i2".into(),
            to_session_id: Some("s2".into()),
            from_instance_id: "i1".into(),
            text: "hi".into(),
            auto_submitted: true,
        })
        .unwrap();
        assert_eq!(
            val,
            json!({
                "toInstanceId": "i2",
                "toSessionId": "s2",
                "fromInstanceId": "i1",
                "text": "hi",
                "autoSubmitted": true
            })
        );
    }

    #[test]
    fn message_injected_omits_session_id_when_none() {
        // `to_session_id: None` must omit the key (TS `toSessionId?: string`),
        // NOT serialise `"toSessionId": null` — null is unassignable under
        // strict null checks.
        let val = serde_json::to_value(MessageInjected {
            to_instance_id: "i2".into(),
            to_session_id: None,
            from_instance_id: "i1".into(),
            text: "hi".into(),
            auto_submitted: true,
        })
        .unwrap();
        assert_eq!(
            val,
            json!({
                "toInstanceId": "i2",
                "fromInstanceId": "i1",
                "text": "hi",
                "autoSubmitted": true
            })
        );
        assert!(val.get("toSessionId").is_none(), "key must be absent");
    }

    #[test]
    fn fusion_stage_camel_case_no_data() {
        let val = serde_json::to_value(FusionStage {
            run_id: "r2".into(),
            stage: "judge".into(),
            data: None,
        })
        .unwrap();
        assert_eq!(
            val,
            json!({ "runId": "r2", "stage": "judge", "data": null })
        );
    }
}
