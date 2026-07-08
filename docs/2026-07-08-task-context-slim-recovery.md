# Recovery Plan: context slim task reads

Date: 2026-07-08
Owner: Aoki d1a70cab-1d34-490c-9fd7-ba4d4c940606
Authority: in-loop
Supersedes stalled lane: context-slim-task-reads
Reviewer: Armin f06992c7-74b4-4ab2-8cf4-e6674481af35

## Goal

Finish Task A from `docs/2026-07-08-plan-context-economy-transcript-meter.md`
without re-reading the full board:

- `task.list` keeps its default wire shape with `plan` present unless callers
  explicitly request slim output.
- `conclave task list <ws> [--state s]` uses slim mode by default.
- `conclave task list <ws> --full` restores old plan-bearing output.
- `conclave task brief <ws> <slug> [--limit N]` returns a bounded,
  readable resume packet with source pointers.

## Existing Partial Work

The abandoned lane left an uncommitted edit in
`src-tauri/src/engine/repo/task.rs`: `list(...)` already has an `include_plan`
parameter and substitutes an empty plan when slim. Treat this as partial work
to inspect and either keep, correct, or replace. Do not revert it blindly.

## Required Files

- `src-tauri/src/engine/commands/task.rs`
- `src-tauri/src/engine/repo/task.rs`
- `src-tauri/src/engine/commands/cli.rs`
- `src-tauri/src/engine/router.rs`
- `src-tauri/src/bin/conclave-cli.rs`
- `src-tauri/skills/tool-map/SKILL.md`
- `src-tauri/skills/leadership/SKILL.md`
- `src-tauri/skills/collaboration/SKILL.md`
- `src-tauri/skills/implementer/SKILL.md`
- `docs/2026-07-08-plan-context-economy-transcript-meter.md`
- `docs/2026-07-08-task-context-slim-recovery.md`

## Acceptance

- Backend command tests prove `task.list` default remains full, while slim mode
  omits or blanks `plan` only when requested.
- CLI mapping tests prove `task list` sends slim and `task list --full` sends
  full.
- `task brief` tests prove output is capped, readable, and record-backed.
- Gate: `cd src-tauri && cargo test`.

## Notes

No `src/` UI files are in scope. If implementation touches UI anyway, follow
the UI Pixel Gate in `AGENTS.md` before READY.
