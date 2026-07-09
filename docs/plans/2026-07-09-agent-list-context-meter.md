# Plan: agent list exposes session context meter

Date: 2026-07-09
Owner: Aoki d1a70cab-1d34-490c-9fd7-ba4d4c940606
Authority: in-loop
Human request: "conclave cli list agent มี context meter ด้วยไหมว่า agent ไหนให้ context ไปเท่าไหร่" then "เพิ่มใน cli ให้หน่อย"

## Goal

`conclave agent list <workspaceId>` should show the same per-agent session context
numbers the UI already uses, so leads can see which live agent is consuming how
much context from the terminal without opening the UI.

## Decisions

1. Add fields to the existing `instance.list` / roster JSON shape rather than
   inventing a new CLI verb.
   - `conclave agent list` already maps to `instance.list`.
   - UI and CLI callers receive one additive JSON shape.

2. Source of truth is the `session` row paired to each `workspace_agent`.
   - `session.context_tokens` and `session.context_limit` are already persisted
     by the chat byte estimate and CLI transcript-backed meter.
   - No transcript parsing belongs in this lane.

3. Field names follow the existing `Session` TS shape and bus payload:
   - `contextTokens`
   - `contextLimit`
   - Optional computed `contextPercent` is allowed only if tests prove it is
     derived and does not replace the raw fields. Prefer raw fields first.

4. Keep output backwards-compatible.
   - These are additive optional fields on roster rows.
   - Agents and UI code that ignore unknown JSON keys must continue to work.

## Boundary

- `src-tauri/src/engine/repo/workspace_agent.rs`
- `src-tauri/src/engine/commands/instance.rs` only if handler-level population is cleaner
- `src-tauri/src/bin/conclave-cli.rs` only if a human-readable renderer exists for this path
- `src-tauri/src/engine/commands/cli.rs` only for tests or mapping updates if required
- `src/ipc/types.ts` to keep frontend type definitions accurate

Do not touch `src/components/*`; the UI already consumes session context via
`session:context` and `Session`.

## Required Behavior

- `conclave agent list <workspaceId>` returns each roster row with
  `contextTokens` and `contextLimit` when that workspace agent has a session row.
- Rows without a session omit those fields, matching existing optional field style.
- Values match the current `session` table row for that agent.
- `lastActivityAt`/`working` behavior stays unchanged.
- `org <workspaceId>` may ignore the new fields; no renderer change required
  unless a test already covers it.

## Tests

- Add/extend Rust repo test proving `list_by_workspace_with_launched_skills`
  includes `contextTokens` and `contextLimit` from the joined session.
- Add serialization assertion that camelCase keys are present when values exist
  and absent when no session exists.
- If CLI-specific rendering changes, add/extend `conclave-cli.rs` tests.

## Gates

- `cd src-tauri && cargo test`
- No UI Pixel Gate required unless the implementation touches `src/components/*`
  or other render-path UI files.

## Risk Ledger

- Do not use transcript files directly here. The meter pipeline already persists
  sanitized numeric values into `session`; this lane only exposes them.
- Do not make the roster query inner-join `session`; agents that have never been
  launched must remain visible.
- Avoid deriving trust from current local values: the present DB may show several
  agents with identical counts because transcript attribution is a separate
  known concern. This lane exposes the stored values; it does not fix attribution.
