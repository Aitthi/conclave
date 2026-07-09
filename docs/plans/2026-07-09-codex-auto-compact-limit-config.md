# Plan: Codex launch also configures auto-compact limit

Date: 2026-07-09
Owner: Aoki d1a70cab-1d34-490c-9fd7-ba4d4c940606
Authority: in-loop

Human evidence:

- Codex status line shows `400000` immediately after launch.
- After the first chat turn starts, the Codex status line switches back to about
  `258K`.

## Goal

When Conclave launches a Codex CLI agent with a numeric `context_window`, pass
both Codex context-management knobs:

- `model_context_window=<tokens>`
- `model_auto_compact_token_limit=<floor(tokens * 0.95)>`

The expected 400K launch should therefore use an auto-compact trigger of
`380000` instead of letting Codex fall back to its catalog default effective
limit of `258400`.

## Diagnosis

Confirmed current backend behavior:

- `src-tauri/src/engine/commands/instance.rs::append_codex_context_window_config`
  appends only `-c 'model_context_window=<tokens>'`.
- Live process args confirmed Conclave does pass `model_context_window=400000`.
- Codex CLI 0.143.0 local model metadata still reports `gpt-5.5`
  `context_window=272000`, and live transcripts report `model_context_window`
  as `258400`, exactly 95% of `272000`.
- Official Codex config docs list `model_auto_compact_token_limit` separately
  from `model_context_window`.

Hypothesis: Codex accepts the requested max window at boot, but the first live
turn reverts the status/effective working window to the default auto-compact
threshold unless `model_auto_compact_token_limit` is also set.

## Required Fix

1. In `src-tauri/src/engine/commands/instance.rs`, change the Codex context
   config helper so a positive numeric context window appends both `-c` values.
2. Compute auto-compact as integer floor of `tokens * 95 / 100`.
3. Keep invalid/sentinel values (`"1m"`, `"200k"`, zero, negative, nonnumeric)
   ignored exactly as today.
4. Do not change Builder UI, transcript parsing, token numerator formula, or
   Claude launch behavior.

## Tests

Update Rust launch construction tests:

- A Codex definition/helper call with `"400000"` must include both
  `model_context_window=400000` and `model_auto_compact_token_limit=380000`.
- Existing invalid/sentinel test must also assert no auto-compact override is
  appended for ignored values.
- Existing Claude test must still prove Claude launch does not include Codex
  context keys.

## Boundary

- `docs/plans/2026-07-09-codex-auto-compact-limit-config.md`
- `src-tauri/src/engine/commands/instance.rs`

No `src/` UI changes are expected; UI Pixel Gate does not apply unless this
boundary changes.

## Gates

- `cd src-tauri && cargo test codex_context_window_config`
- `cd src-tauri && cargo test`

## Acceptance

- New Codex launches with `contextWindow=400000` produce a command containing
  both `model_context_window=400000` and
  `model_auto_compact_token_limit=380000`.
- No magic display offset or transcript formula change is introduced.
- If a later live Codex status-line check still flips to 258K even with both
  keys present, record that as upstream Codex runtime behavior and do not claim
  this task fully fixes the runtime flip.
