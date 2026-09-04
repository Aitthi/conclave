# Agent stop/resume discovery

owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop

## Goal

Map the existing workspace-agent lifecycle and propose the smallest complete change that lets a user stop an agent without removing its workspace membership, then resume it later.

## Scope

- Read-only inspection of persistence, runtime/session teardown and launch, CLI commands, IPC types, roster UI, fixtures, and tests.
- Record findings as a task note: current behavior, exact symbols/files, recommended state semantics, race/ownership risks, migration needs, and verification commands.
- Do not edit product code.

## Product decisions already settled

- “Stop” preserves the workspace-agent row, role, skills, supervisor graph, history, and settings.
- A stopped agent cannot receive new work/messages and has no live CLI/chat runtime.
- “Resume” reuses the same workspace-agent identity and starts a fresh runtime/session through the existing launch path.
- Removal remains a separate destructive action.
- The UI must make stopped state and the stop/resume actions explicit.

## Done

A bounded evidence note is attached to task `agent-stop-discovery`; the note is sufficient for a zero-context implementer plan.
