# Plan: fix transcript context limit persistence

Date: 2026-07-09
Owner: Aoki d1a70cab-1d34-490c-9fd7-ba4d4c940606
Authority: in-loop

Human bug report:

- "Context meter มันเกินจริงไป 4% ตอนแรก 2%"

## Goal

The Conclave context meter for CLI agents must use the same denominator as the
harness transcript reading that supplied the token count. A Codex transcript
reading with `model_context_window = 258400` must not be persisted or later
listed as `contextTokens / 200000`.

This task fixes the overcount path caused by stale persisted `session.context_limit`.
It does not change transcript ownership matching and must not add a magic token
offset.

## Diagnosis Record

Confirmed code path:

- `src-tauri/src/engine/runtime/transcript_context.rs` returns both
  `TranscriptContextReading.tokens` and `TranscriptContextReading.limit`.
- `src-tauri/src/engine/commands/instance.rs::poll_transcript_context` emits
  `session:context` with the transcript `reading.limit`, but persists only
  `reading.tokens` via `repo::session::set_context_tokens`.
- `src-tauri/src/engine/repo/session.rs` seeds every new session with
  `DEFAULT_CONTEXT_LIMIT = 200_000`.
- `conclave agent list` and initial UI session state read
  `session.context_tokens` and `session.context_limit` from the database through
  `workspace_agent::list_by_workspace_with_launched_skills`.

Observed local evidence on 2026-07-09:

- Real Codex transcript for Aoki reported `model_context_window = 258400`.
- The roster path reported `contextLimit = 200000`.
- Therefore any persisted/list-derived percentage can overstate usage even when
  transcript attribution and live-event emission are correct.

## Ranked Hypotheses

1. Primary: transcript-backed updates persist the token numerator but not the
   transcript denominator, so roster and initial UI state use stale 200K limits.
   Prediction: after persisting both tokens and limit, `conclave agent list` and
   initial `instance.list` state match the transcript denominator.
2. Secondary: the Codex token numerator should not be
   `last_token_usage.total_tokens`.
   Prediction: even after denominator persistence, a direct harness-visible
   percent comparison remains off by several points. Do not change the numerator
   in this task without a captured transcript field proving the replacement.
3. Secondary: transcript ownership regressed and the wrong file is selected.
   Prediction: owner/workspace tests fail or a non-owning transcript supplies the
   reading. Existing owner-marker tests should continue to pass.

## Required Fix

1. Add a repository helper in `src-tauri/src/engine/repo/session.rs` that updates
   both `context_tokens` and `context_limit` and stamps `last_active_at`.
   Suggested name: `set_context_reading(pool, session_id, tokens, limit)`.
2. Change transcript-backed updates in
   `src-tauri/src/engine/commands/instance.rs::poll_transcript_context` to call
   the new helper.
3. Leave the chat byte-estimate path on the existing token-only helper unless
   the implementer finds a concrete reason it should also update limit.
4. Add regression coverage:
   - session repo test: the new helper updates both tokens and limit.
   - forwarder test: `forwarder_updates_context_from_transcript_reader_for_cli`
     must assert `context_tokens == 321` and `context_limit == 8000`.
   - roster/list test: a workspace agent with a session updated to
     `tokens=321, limit=8000` must surface both values through
     `list_by_workspace_with_launched_skills` / `conclave agent list` data path.

## Non-Goals

- Do not change `last_token_usage.total_tokens` to another numerator.
- Do not subtract 2%, 4%, or any fixed fudge factor.
- Do not change official Codex model max/default mapping in `Builder.tsx`.
- Do not touch `src/` UI unless tests prove a separate UI-only bug. If `src/`
  UI is touched, the UI Pixel Gate applies.

## Boundary

- `docs/plans/2026-07-09-fix-context-meter-limit-persistence.md`
- `src-tauri/src/engine/repo/session.rs`
- `src-tauri/src/engine/commands/instance.rs`
- `src-tauri/src/engine/repo/workspace_agent.rs` only for roster regression tests

## Gates

Run and record:

- `cd src-tauri && cargo test set_context`
- `cd src-tauri && cargo test forwarder_updates_context_from_transcript_reader_for_cli`
- `cd src-tauri && cargo test list_by_workspace_with_launched_skills`
- `cd src-tauri && cargo test`

No UI pixel gate is required if no `src/` UI file changes.

## Acceptance

- A transcript-backed Codex reading with `tokens=321, limit=8000` persists both
  values to the session row.
- The roster/agent-list data path returns the updated limit, not the default
  `200000`.
- Existing transcript ownership tests continue to pass.
- No magic percentage offset is introduced.
