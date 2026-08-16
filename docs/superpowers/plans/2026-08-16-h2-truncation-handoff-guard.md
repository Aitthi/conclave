# Truncated summaries must not reach the H2 candidate handoff

owner: 30fa04f4-e047-4241-a9ed-f452529952be · authority: in-loop

## Context (Mellow's challenge 3a2afb6f on h1-stop-reason-guard, 2026-08-16)

h1-stop-reason-guard (merged 22946e7) keeps a truncated generation as
`outcome=measured` (so its spend stays visible) and filters it out of
every economics aggregate. But step 11 — the H2 candidate handoff at
`ctx_proxy.rs:2277-2300` — runs right after the measured insert with the
only precondition `snapshot_quality_campaign().is_none()`, so a truncated
row still builds a `QualityCandidate` carrying the cut-off
`generated.text`. `admit_live_quality_case` (ctx_proxy.rs:2703-2745)
re-checks only the CAMPAIGN-level `H1Gate::Pass`, which is now computed
from clean rows only — a healthy campaign passes while this one candidate
is truncated. Nothing on the path reads `stop_reason`. Latent, not live
(H2 is OFF), but armed H2 would spend five role calls judging a summary
H1 itself refuses to count.

## Task

Gate the handoff on the same clean-stop notion the aggregates use, plus
one write-boundary hardening from the same review. Precondition for
arming H2 (joins h2-preflight-text-free, h2-evaluator-first-party-system).

## Fix

- **Handoff gate**: skip step 11 when `generated.stop_reason` is
  `Some(x)` with `x != "end_turn"` (same semantics as the SQL
  `clean_stop!()`: None/legacy passes, unknown shapes are truncated).
  Preferred shape: the in-Rust check at step 11 — cheap and local. If you
  find during implementation that a counted refusal in
  `admit_live_quality_case` (like the existing `quality_h1_blocked`
  path) is no more code, prefer THAT for observability; either shape
  passes review as long as the acceptance fixture holds.
- **Acceptance fixture (Mellow's, verbatim)**: a TruncatedAtMaxTokens
  response with H2 armed must produce ZERO quality rows and ZERO role
  calls. Mutation-verify the gate.
- **Hardening (Mellow finding (a), owner-accepted)**:
  `insert_terminal` in `repo/proxy_summary_metric.rs` allowlists
  `error_type` at the write boundary but not `stop_reason` — the
  privacy guarantee currently rests on one call site's
  `Option<&'static str>` type. Add the same write-boundary allowlist
  for `stop_reason` (the eight known literals + reject anything else),
  pinned by a test, matching the sibling column's pattern.

## Chaining constraint

`ctx_proxy.rs` is also in lane h2-evaluator-first-party-system-v2's
boundary (comment-only breadcrumb). Do NOT claim this task until that
lane is MERGED — two lanes must never share a file.

## Constraints (inherited, one section)

- Truncated rows STAY `outcome=measured` (spend visibility — recorded
  decision, h1-stop-reason-guard Outcome); only the handoff/admission
  changes.
- Failure-kind vocabulary unchanged; content-free metrics.
- `plateau_turns` stays unfiltered (recorded owner decision — not a
  GO-bar input); do not "fix" it in passing.
- Boundary: `src-tauri/src/engine/runtime/ctx_proxy.rs`,
  `src-tauri/src/engine/repo/proxy_summary_metric.rs`, this plan file.

## Gates (record each via `conclave task gate`)

- `cd src-tauri && cargo test engine::runtime::ctx_proxy`
- `cd src-tauri && cargo test proxy_summary_metric`
- `git diff --check`
