# Context fallback limit: claude-code sessions default to 1M
<!-- conclave-plan:v1
{
"owner":"4fb2198c-e0d9-4e4b-af9e-d4e72542bace","authority":"in-loop",
"planPath":"docs/superpowers/plans/2026-07-10-context-fallback-limit.md","baseSha":"cb62a58e11a47a91d875ff2e7acd72462f6ddcc7","escalation":"4fb2198c-e0d9-4e4b-af9e-d4e72542bace",
"readingOrder":["docs/superpowers/plans/2026-07-10-context-fallback-limit.md","src-tauri/src/engine/repo/session.rs#DEFAULT_CONTEXT_LIMIT","src-tauri/src/engine/commands/instance.rs#fallback_limit","src-tauri/src/engine/runtime/transcript_context.rs#fallback_limit"],
"boundary":["src-tauri/src/engine/commands/instance.rs","src-tauri/src/engine/commands/snapshot.rs","src-tauri/src/engine/commands/workspace.rs","src-tauri/src/engine/db.rs","src-tauri/src/engine/repo/session.rs","src-tauri/src/engine/repo/workspace_agent.rs","src-tauri/src/engine/runtime/transcript_context.rs"],
"consumes":["src-tauri/src/engine/repo/session.rs#DEFAULT_CONTEXT_LIMIT","src-tauri/src/engine/runtime/transcript_context.rs#fallback_limit"],
"produces":["src-tauri/src/engine/repo/session.rs#default_context_limit_for"],"gates":["cd src-tauri && cargo fmt --check","cd src-tauri && cargo test","cd src-tauri && cargo clippy --all-targets --all-features -- -D warnings","git diff --check"]
} -->

## Goal

Stop false context warnings for claude-code agents: the pre-detection fallback
context limit for a claude-code session becomes 1,000,000 tokens (human
directive, 2026-07-10: no claude-code model in this environment runs a 200k
window). Codex and unknown cliKinds keep the conservative 200,000 fallback —
over-warning is recoverable, under-warning costs an agent its context.

## Background (evidence on task ledger context-meter-stale-model-limit)

Live incident: Tiësto received "[conclave context] You are at 70%" at a real
~12% (117k/1M). The 70% was computed from `DEFAULT_CONTEXT_LIMIT = 200_000`
(`src-tauri/src/engine/repo/session.rs:45`), which is stamped into
`session.context_limit` at spawn and used as `fallback_limit` by
`runtime/transcript_context.rs` (`last_model_window.unwrap_or(fallback_limit)`)
until the harness log yields a real `model_context_window`. In this
environment opus sessions were ALSO 1M before the Fable 5 switch, so 200k is
wrong data for every claude-code session, not merely stale.

## Non-goals

- Do not change the detected-limit path (`last_model_window` precedence stays).
- Do not change warning thresholds or the stall engine.
- Do not add config/UI for the value; a per-cliKind constant is enough for v1.
- Do not touch `runtime/task_timer.rs` — its tests pass explicit limits.

## Decisions

- Replace the single global default with a per-cliKind resolver in
  `repo/session.rs`: `pub fn default_context_limit_for(cli_kind: &str) -> i64`
  returning 1_000_000 for `"claude-code"` (and its `"claude"` spawn alias if
  the codebase uses one — grep before assuming) and `DEFAULT_CONTEXT_LIMIT`
  (200_000, kept as the conservative constant) for everything else.
- Every site that stamps or falls back to the default must go through the
  resolver WITH the session's cliKind: session row creation, the
  `unwrap_or(DEFAULT_CONTEXT_LIMIT)` sites in `commands/instance.rs` (:831,
  :988, :2029), `commands/snapshot.rs` (:170), and the seed/bind in
  `engine/db.rs` (:347). If a call site cannot know the cliKind, that is a
  finding — note it on the ledger rather than guessing.
- `commands/workspace.rs:249` asserts the stamped limit equals the old default;
  update the assertion to the resolver's value for the cliKind under test.
- `repo/workspace_agent.rs:711` comment names the old single source of truth —
  update the comment to point at the resolver.

## Ordered edits

1. `repo/session.rs`: add `default_context_limit_for`, keep
   `DEFAULT_CONTEXT_LIMIT` as the non-claude fallback, update the module doc
   (line ~14) that documents the 200k default.
2. Thread cliKind into the stamping/fallback sites listed above; replace each
   `DEFAULT_CONTEXT_LIMIT` use with the resolver.
3. `runtime/transcript_context.rs`: no production change (it receives
   `fallback_limit` from callers); update the two tests constructing configs
   with `fallback_limit: 200_000` (:821, :833) only if their assertions
   actually depend on the claude path — otherwise leave them pinning the
   codex/conservative path and add one claude-path test asserting a fresh
   claude-code session with no detected window reads limit 1_000_000.
4. Tests: resolver unit test (claude-code -> 1M, codex -> 200k, unknown ->
   200k); regression test for the incident shape — claude-code session,
   no detection, tokens 140_000 -> reading is 14%, NOT 70%.

## Verification

Run the four header gates; all must exit 0. `cargo test` must include the new
resolver + regression tests. After integration and app rebuild+restart, a
freshly spawned claude agent must show `contextLimit: 1000000` in
`conclave agent list` BEFORE its transcript reports a model window.

## Risks

- The 200k literal appears in many tests (task_timer.rs uses explicit args —
  out of scope); only change tests whose meaning is "the default", not tests
  that pass explicit limits.
- `db.rs:347` seeds sessions in migrations/fixtures — confirm which cliKind
  that path represents before wiring the resolver there.
- Under-warning risk for codex is deliberately NOT taken: codex keeps 200k.

## Rejected alternatives

- Flat 1M for all cliKinds: codex windows here are ~258k-353k; a 1M fallback
  would silence real warnings pre-detection (under-warn) — worse than a false
  nudge.
- Suppressing warnings while on fallback data: bigger behavior change to the
  stall engine; unnecessary once the claude fallback is truthful.
- Making the limit configurable: no second environment exists yet to need it.

## Escalation

Plan conflicts or surprises in call-site cliKind availability: challenge to
Detoro (4fb2198c-e0d9-4e4b-af9e-d4e72542bace), task owner. Implementation
judgment within this plan is the implementer's, recorded as task notes.
