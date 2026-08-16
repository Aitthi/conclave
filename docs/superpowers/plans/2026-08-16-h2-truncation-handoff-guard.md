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

## Outcome (2026-08-16, implementer record — lane `lane/h2-truncation-handoff-guard`)

Both fixes landed at `75887d1`; Mellow's two review one-liners closed in
the same lane. Decisions of record, so they do not live only in commit
messages:

- **Counted refusal REJECTED, in-Rust step-11 check taken** (the plan's
  preferred shape). Three reasons, checked rather than assumed:
  1. It is strictly MORE code, not "no more code" as the plan's escape
     clause required — `QualityCandidate` carries no `stop_reason`, so
     the counted variant needs a new field, a new `AtomicU64`, a new
     `QualityStatus` field, and the check.
  2. It BREACHES the boundary. `QualityStatus` is declared in
     `ctx_proxy.rs:185`, but `commands/proxy.rs` constructs it
     exhaustively (~`:702`) and hand-maps every counter into JSON
     (~`:512`) — it is not serde-derived, so a new counter is always a
     two-file change, and `commands/proxy.rs` is not in this boundary.
  3. It is REDUNDANT. The count is already observable as `truncated` in
     `proxy summary-report` (added in h1-stop-reason-guard, 22946e7).
  Reusing `quality_h1_blocked` was also rejected: its documented meaning
  is "linked H1 campaign not `Pass`/armed", which this is not.
- **Mock faithfulness fix (found by mutation testing, pre-existing gap
  outside the diff).** `start_h2_e2e_upstream`'s generation branch
  emitted NO `stop_reason`, so `Option::is_some_and` over `None` is
  always false and the CLEAN path was unpinned — inverting the gate
  broke only the new test. The mock now emits `stop_reason` on both
  paths (`end_turn` / `max_tokens`), which also matches the real API;
  the inversion now fails three tests. General rule: when a guard keys
  off a response field, the happy-path mock must populate that field, or
  the positive branch is untested.
- **Review one-liner (a)**: step 11 uses `summary::END_TURN_STOP_REASON`
  instead of a hardcoded literal. The const had exactly one reference —
  its own declaration. Verified non-cosmetic: renaming its value now
  fails three tests.
- **Review one-liner (b)**: `the_local_stop_reason_allowlist_matches_the_runtime_vocabulary`
  pins the repo-local `KNOWN_STOP_REASONS` against
  `summary::KNOWN_STOP_REASONS` + `UNKNOWN_STOP_REASON`. The `error_type`
  sibling cannot be pinned this way (`count_tokens::KNOWN_ERROR_TYPES` is
  private); this pair is public on both sides. Drift here fails quietly
  and backwards — upstream adds a reason, the writer emits it,
  `insert_terminal` rejects it, and the evidence row is silently LOST.
  The test also pins `END_TURN_STOP_REASON == "end_turn"`, the one value
  `clean_stop!()`/`truncated_stop!()` must hard-code as SQL text.
- **Acceptance fixture** is Mellow's verbatim, and non-vacuous by
  construction: H2 armed, linked, preflight-verified, and the campaign
  `H1Gate` asserted `Pass` inside the test, so the zero rows / zero role
  calls can only come from the new gate. Note for future editors:
  `seed_passing_h1_campaign` inserts 31 measured rows with NULL
  `stop_reason`, so `wait_for_summary_measured` returns BEFORE the live
  row lands — the fixture waits on `measured AND stop_reason IS NOT NULL`,
  which only the live row can satisfy.
- Unchanged per recorded decisions: truncated rows stay
  `outcome=measured` (spend visibility) and `plateau_turns` stays
  unfiltered.

## Gates (record each via `conclave task gate`)

- `cd src-tauri && cargo test engine::runtime::ctx_proxy`
- `cd src-tauri && cargo test proxy_summary_metric`
- `git diff --check`
