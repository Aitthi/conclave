# H1 measured rows must not admit truncated summaries — stop_reason guard

owner: 30fa04f4-e047-4241-a9ed-f452529952be · authority: in-loop

## Context (from Mellow's review of h1-gen-non-text @ cecd199, F2, 2026-08-16)

Nothing in `src-tauri/src/engine/runtime/` reads `stop_reason` (grep:
zero hits), and the 9f128b7 parser fix makes truncation REACHABLE for
the first time: before it every opus-5 response died at `non_text`;
after it, a `stop_reason=max_tokens` response with partial text parses
as a valid summary and persists as `measured`.

Why this must land BEFORE economics rows are read as evidence: gen_body
caps output at 8192 (`summary.rs:14` → `ctx_proxy.rs:2075`) and adaptive
thinking consumes that budget (Dew measured 272/788 output tokens as
thinking on a SMALL prompt; H1 prefixes are ~200k over 10 sources). A
truncated (shorter) summary makes B_h smaller → s_h = A − B_h larger →
q_h higher → n_h = 11.5/q_h − 12.5 + 10g/q_h lower — i.e. a truncated
non-summary passes `meets_two_turn` MORE easily than a real one. The GO
bar would be biased optimistically.

## Task

Record `stop_reason` on every generation that returns usage, and keep
non-`end_turn` rows out of the economics aggregates. Owner ruling on the
shape (recording beats fail-closed — it preserves the evidence and needs
no failure-vocabulary migration):

1. Additive migration `src-tauri/src/engine/migrations/
   0026_proxy_summary_stop_reason.sql`:
   `ALTER TABLE proxy_summary_metric ADD COLUMN stop_reason TEXT;`
   (nullable; historical rows stay NULL).
2. `GeneratedSummary` (`summary.rs`) carries `stop_reason` from the
   response body; `generate_summary` extracts it (missing field → NULL,
   not an error).
3. Persist it on the metric row (`ctx_proxy.rs` insert path +
   `repo/proxy_summary_metric.rs`).
4. `summary-report` (`repo/proxy_summary_metric.rs` +
   `commands/proxy.rs`): rows with `stop_reason` present and !=
   'end_turn' are EXCLUDED from measured/q_h/n_h/meets_* aggregates and
   surfaced as a new `truncated` count in the report JSON. NULL
   stop_reason (historical/absent) keeps today's behavior. Acceptance:
   a max_tokens-stopped generation must be IMPOSSIBLE to enter the
   q_h/n_h aggregates, and the report must show it happened.
5. Nit from the same review (one line, no behavior change): the
   summary.rs doc text claims the parser fails `empty_text` "when no
   text survives", but the code aborts when ANY text block is blank
   (`summary.rs:143-144`), which differs from quality.rs's trim-at-end.
   Correct the doc/comment to state the strict per-block behavior and
   that the two parsers intentionally differ on that shape. Do NOT
   change parser behavior in this lane.

## Constraints (inherited, one section)

- Parser text semantics unchanged (only the new stop_reason extraction).
- Content-free metrics: `stop_reason` is an enum-like short string from
  the API, never message text.
- Failure-kind vocabulary unchanged; outcome of a truncated row stays
  `measured` — the guard lives in the AGGREGATION, so the evidence is
  kept.
- Migration is additive-only; do not rewrite the CREATE TABLE or its
  CHECK constraints.
- Boundary: `src-tauri/src/engine/runtime/summary.rs`,
  `src-tauri/src/engine/runtime/ctx_proxy.rs`,
  `src-tauri/src/engine/repo/proxy_summary_metric.rs`,
  `src-tauri/src/engine/commands/proxy.rs`,
  `src-tauri/src/engine/migrations/0026_proxy_summary_stop_reason.sql`,
  this plan file.
- Tests mutation-verified per workspace standard: an end_turn row enters
  aggregates; a max_tokens row is excluded and counted as truncated; a
  NULL row keeps legacy behavior.

## Gates (record each via `conclave task gate`)

- `cd src-tauri && cargo test engine::runtime::summary`
- `cd src-tauri && cargo test engine::runtime::ctx_proxy`
- `cd src-tauri && cargo test proxy_summary_metric`
- `git diff --check`
