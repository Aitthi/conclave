# Context meter: stale tokens shown on a fresh CLI generation

owner: 30fa04f4-e047-4241-a9ed-f452529952be · authority: in-loop

## Problem (reported 2026-08-16, human + screenshot)

A freshly (re)launched CLI agent shows a non-zero context meter (e.g. 15% =
149,355 tok) before it has consumed anything. The value is the PREVIOUS
process generation's reading.

## Root cause (code-confirmed)

- A CLI restart spawns a fresh process. Resume is a handoff-prompt injection
  (`run_respawn_resume`, src-tauri/src/engine/commands/instance.rs:1573), NOT
  `--continue` — so the old context is genuinely gone.
- `spawn` (instance.rs:541) reuses the persisted session row
  (`repo::session::get_by_instance`) and nothing resets `context_tokens`.
- The transcript meter seeds from the stale row: instance.rs:1206-1210
  (`TranscriptMeterState { tokens: session_row.context_tokens ... }`).
- The frontend seeds from the same stale `session.contextTokens`
  (src/components/ContextBars.tsx:614-618) and shows it until the NEW
  transcript produces a first reading. The reader correctly ignores old data
  (file mtime filter, transcript_context.rs:398; row filter `observed_at <
  started_at`, transcript_context.rs:500), so the stale value can persist for
  a long idle stretch.

## Fix (engine only; no frontend change)

In the CLI spawn success path (where `TranscriptPollContext::new` is built
with `started_at` and the resolved `limit`):

1. Persist a zero reading for the new generation:
   `repo::session::set_context_reading(db, session_id, 0, limit)` — pass the
   RESOLVED limit, never 0/NULL (set_context_reading writes both columns).
2. Emit `bus::session_context { session_id, context_tokens: 0, context_limit:
   limit, estimated: true }` so an already-open UI repaints immediately.
3. Seed `TranscriptMeterState.tokens = 0` (instance.rs:1206-1210) instead of
   the stale session value.

Scope rules:
- CLI/transcript branch ONLY. Chat sessions (`track_context` estimate branch)
  keep their persisted value — chat history genuinely persists across
  restarts.
- Do NOT reset on the idempotent already-live early return (instance.rs:559)
  — that would zero a running agent's meter.
- Verify BOTH entry points land at 0: cold `spawn` and the restart tail
  `run_respawn_resume` (confirm it routes through the same spawn path; if
  not, reset there too).
- Codex CLI kind gets the same reset (any CLI respawn is a fresh process).

## Tests (extend existing patterns in instance.rs tests, ~lines 2026-2200)

- New: session row pre-seeded with stale `context_tokens = 125_000`; spawn a
  CLI instance; assert the row reads 0 and the limit is the resolved one.
- Existing context-seeding tests must stay green.

## Verification / gates

- `cargo test --manifest-path src-tauri/Cargo.toml` green, recorded via
  `conclave task gate`.
- Known noise: conclave-cli has a flaky fixed-temp-path test under concurrent
  agents — unrelated red is noise, rerun it.

## Risk ledger

- `set_context_reading` writes tokens AND limit in one update — passing a
  wrong limit here would corrupt the denominator the UI divides by.
- The transcript reader already row-filters by `observed_at < started_at`; do
  not add extra filtering there.
- Fresh lane worktrees have no node_modules; this lane is Rust-only — don't
  run pnpm gates, they're out of scope.
