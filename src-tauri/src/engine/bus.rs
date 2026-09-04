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
pub const SESSION_CONTEXT: &str = "session:context";
pub const FUSION_STAGE: &str = "fusion:stage";
pub const MESSAGE_INJECTED: &str = "message:injected";
pub const SNAPSHOT_CREATED: &str = "snapshot:created";
pub const TASK_CHANGED: &str = "task:changed";
pub const MEMORY_CHANGED: &str = "memory:changed";
pub const ARTIFACT_CHANGED: &str = "artifact:changed";
pub const ROSTER_CHANGED: &str = "roster:changed";
pub const WORKSPACE_CHANGED: &str = "workspace:changed";

// ---------------------------------------------------------------------------
// Payload structs
// ---------------------------------------------------------------------------

/// Payload for `session:output` — a streamed output chunk from a session.
///
/// Serialises to `{ "sessionId": "...", "chunk": "...", "activity": bool }`
/// (camelCase) to match the `SessionOutputEvent` interface in
/// `src/ipc/events.ts`. `activity` (`bb plan:working-false-positive`) is
/// `false` for a chunk the engine judged to be the terminal's own echo of our
/// own input (a resize-provoked repaint, keystroke echo) rather than genuine
/// agent output — Roster's working indicator ignores those chunks; Terminal
/// still renders every chunk regardless, since the pane must show the echo
/// even though it isn't "activity".
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SessionOutput {
    pub session_id: String,
    pub chunk: String,
    pub activity: bool,
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

/// Payload for `session:context` — a live context-meter update for a session.
///
/// Serialises to `{ "sessionId", "contextTokens", "contextLimit", "estimated" }`
/// (camelCase) to match `SessionContextEvent` in `src/ipc/events.ts`.
///
/// HONESTY: `estimated` is always `true` for now — `context_tokens` is derived
/// from streamed output bytes (≈4 chars/token), NOT from real provider
/// token-usage telemetry. The UI labels the meter "estimate" accordingly.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SessionContext {
    pub session_id: String,
    pub context_tokens: i64,
    pub context_limit: i64,
    pub estimated: bool,
}

/// Payload for `snapshot:created` — a snapshot (manual or auto-compact) was
/// persisted for a session.
///
/// Serialises to `{ "sessionId", "snapshotId", "type", "tokens"?, "triggerPct"? }`
/// (camelCase) to match `SnapshotCreatedEvent` in `src/ipc/events.ts`. The kind
/// field is wired as `"type"` to align with the `Snapshot.type` domain field and
/// the `snapshot.create` payload (one name across the wire). `tokens` and
/// `trigger_pct` are `Option` (a session with no token estimate yet has neither);
/// `skip_serializing_if` omits the key entirely when `None` (matching the
/// optional `tokens?` / `triggerPct?` TS fields — never `null`).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotCreated {
    pub session_id: String,
    pub snapshot_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_pct: Option<i64>,
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

/// Payload for `task:changed` (ADR 0008) — a `task.*` mutating command ran.
/// Carries only identity + current `state`, not the mutation's details (an
/// event append leaves `state` unchanged but still fires this so the Lane
/// Board can refresh badges/counts).
///
/// Serialises to `{ "workspaceId", "taskId", "slug", "state" }` (camelCase) to
/// match `TaskChangedEvent` in `src/ipc/events.ts`.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TaskChanged {
    pub workspace_id: String,
    pub task_id: String,
    pub slug: String,
    pub state: String,
}

/// Payload for `memory:changed` — a workspace's `memory_chunk` table actually
/// changed (a chunk was written or removed by `remember`/`delete`/`clear`/
/// `approve`), so an open Memory Graph view should refetch. NEVER emitted for
/// a no-op — a deduped `remember`, a `delete` that found nothing, an empty
/// `clear`, or a deduped `approve` (see `commands::memory::emit_changed`).
///
/// Serialises to `{ "workspaceId" }` (camelCase) to match
/// `MemoryChangedEvent` in `src/ipc/events.ts`.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MemoryChanged {
    pub workspace_id: String,
}

/// Payload for `artifact:changed` (plan design-artifact-store, Lane A) — a
/// workspace's `artifact` table gained a row via `artifact.add`, so an open
/// Artifacts view should refetch. Carries only the workspace id (the view
/// re-lists by workspace); mirrors [`MemoryChanged`].
///
/// Serialises to `{ "workspaceId" }` (camelCase) to match
/// `ArtifactChangedEvent` in `src/ipc/events.ts`.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactChanged {
    pub workspace_id: String,
}

/// Payload for `roster:changed` (spec position-system §5.1) — a workspace's
/// roster membership/position changed (e.g. `instance.setPosition` wrote a
/// level or supervisor link), so an open Roster/org view should refetch.
/// Carries only the workspace id; mirrors [`MemoryChanged`].
///
/// Serialises to `{ "workspaceId" }` (camelCase) to match
/// `RosterChangedEvent` in `src/ipc/events.ts`.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RosterChanged {
    pub workspace_id: String,
}

/// Payload for persistent workspace lifecycle changes.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceChanged {
    pub workspace_id: String,
    pub run_state: String,
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

/// Emit a `session:context` event carrying a live context-meter estimate.
pub fn session_context(app: &AppHandle, payload: SessionContext) -> tauri::Result<()> {
    emit(app, SESSION_CONTEXT, payload)
}

/// Emit a `snapshot:created` event carrying a freshly-persisted snapshot.
pub fn snapshot_created(app: &AppHandle, payload: SnapshotCreated) -> tauri::Result<()> {
    emit(app, SNAPSHOT_CREATED, payload)
}

/// Emit a `fusion:stage` event carrying stage-progress data.
pub fn fusion_stage(app: &AppHandle, payload: FusionStage) -> tauri::Result<()> {
    emit(app, FUSION_STAGE, payload)
}

/// Emit a `message:injected` event carrying a delivered inter-agent injection.
pub fn message_injected(app: &AppHandle, payload: MessageInjected) -> tauri::Result<()> {
    emit(app, MESSAGE_INJECTED, payload)
}

/// Emit a `task:changed` event carrying a task's post-mutation identity+state.
pub fn task_changed(app: &AppHandle, payload: TaskChanged) -> tauri::Result<()> {
    emit(app, TASK_CHANGED, payload)
}

/// Emit a `memory:changed` event carrying the workspace whose memory store
/// just changed.
pub fn memory_changed(app: &AppHandle, payload: MemoryChanged) -> tauri::Result<()> {
    emit(app, MEMORY_CHANGED, payload)
}

/// Emit an `artifact:changed` event carrying the workspace whose artifact
/// store just gained a row.
pub fn artifact_changed(app: &AppHandle, payload: ArtifactChanged) -> tauri::Result<()> {
    emit(app, ARTIFACT_CHANGED, payload)
}

/// Emit a `roster:changed` event carrying the workspace whose roster
/// membership/position just changed.
pub fn roster_changed(app: &AppHandle, payload: RosterChanged) -> tauri::Result<()> {
    emit(app, ROSTER_CHANGED, payload)
}

pub fn workspace_changed(app: &AppHandle, payload: WorkspaceChanged) -> tauri::Result<()> {
    emit(app, WORKSPACE_CHANGED, payload)
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
            activity: true,
        })
        .unwrap();
        assert_eq!(
            val,
            json!({ "sessionId": "s1", "chunk": "hi", "activity": true })
        );
    }

    #[test]
    fn session_output_activity_false_for_suppressed_echo() {
        let val = serde_json::to_value(SessionOutput {
            session_id: "s1".into(),
            chunk: "\x1b[2J".into(),
            activity: false,
        })
        .unwrap();
        assert_eq!(
            val,
            json!({ "sessionId": "s1", "chunk": "\x1b[2J", "activity": false })
        );
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
    fn session_context_camel_case() {
        let val = serde_json::to_value(SessionContext {
            session_id: "s1".into(),
            context_tokens: 50_000,
            context_limit: 200_000,
            estimated: true,
        })
        .unwrap();
        assert_eq!(
            val,
            json!({
                "sessionId": "s1",
                "contextTokens": 50_000,
                "contextLimit": 200_000,
                "estimated": true
            })
        );
    }

    #[test]
    fn snapshot_created_camel_case() {
        let val = serde_json::to_value(SnapshotCreated {
            session_id: "s1".into(),
            snapshot_id: "snap1".into(),
            kind: "manual".into(),
            tokens: Some(50_000),
            trigger_pct: Some(25),
        })
        .unwrap();
        assert_eq!(
            val,
            json!({
                "sessionId": "s1",
                "snapshotId": "snap1",
                "type": "manual",
                "tokens": 50_000,
                "triggerPct": 25
            })
        );
    }

    #[test]
    fn snapshot_created_omits_tokens_when_none() {
        // `tokens: None` must omit the key (TS `tokens?: number`), NOT serialise
        // `"tokens": null` — null is unassignable under strict null checks.
        let val = serde_json::to_value(SnapshotCreated {
            session_id: "s1".into(),
            snapshot_id: "snap1".into(),
            kind: "auto".into(),
            tokens: None,
            trigger_pct: None,
        })
        .unwrap();
        assert_eq!(
            val,
            json!({ "sessionId": "s1", "snapshotId": "snap1", "type": "auto" })
        );
        assert!(val.get("tokens").is_none(), "tokens key must be absent");
        assert!(
            val.get("triggerPct").is_none(),
            "triggerPct key must be absent"
        );
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
    fn task_changed_camel_case() {
        let val = serde_json::to_value(TaskChanged {
            workspace_id: "ws1".into(),
            task_id: "t1".into(),
            slug: "memory-graph".into(),
            state: "claimed".into(),
        })
        .unwrap();
        assert_eq!(
            val,
            json!({
                "workspaceId": "ws1",
                "taskId": "t1",
                "slug": "memory-graph",
                "state": "claimed"
            })
        );
    }

    #[test]
    fn memory_changed_camel_case() {
        let val = serde_json::to_value(MemoryChanged {
            workspace_id: "ws1".into(),
        })
        .unwrap();
        assert_eq!(val, json!({ "workspaceId": "ws1" }));
    }

    #[test]
    fn artifact_changed_camel_case() {
        let val = serde_json::to_value(ArtifactChanged {
            workspace_id: "ws1".into(),
        })
        .unwrap();
        assert_eq!(val, json!({ "workspaceId": "ws1" }));
    }

    #[test]
    fn roster_changed_camel_case() {
        let val = serde_json::to_value(RosterChanged {
            workspace_id: "ws1".into(),
        })
        .unwrap();
        assert_eq!(val, json!({ "workspaceId": "ws1" }));
    }

    #[test]
    fn workspace_changed_camel_case() {
        let val = serde_json::to_value(WorkspaceChanged {
            workspace_id: "ws1".into(),
            run_state: "stopped".into(),
        })
        .unwrap();
        assert_eq!(val, json!({ "workspaceId": "ws1", "runState": "stopped" }));
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
