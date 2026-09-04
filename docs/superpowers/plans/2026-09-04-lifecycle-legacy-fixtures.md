# Lifecycle legacy fixture opt-in

owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop

## Goal

Update legacy Rust test fixtures that intentionally exercise live runtime/message/task behavior so they explicitly start their workspace after the lifecycle migration changed the correct default to stopped.

## Scope

- Test-only edits in `src-tauri/src/engine/commands/orient.rs`, `src-tauri/src/engine/runtime/task_timer.rs`, and `src-tauri/src/engine/uds.rs`.
- Add the narrow explicit `set_run_state(..., "started")` setup required by the affected tests.
- Do not weaken production lifecycle guards, change assertions unrelated to setup, or edit production behavior.

## Verification

Run affected focused tests, commit only this boundary, then run the full Rust suite together with `workspace-agent-lifecycle-backend` before READY/integration.
