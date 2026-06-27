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
