# H2 evaluator_request needs the first-party system block (OAuth 429 gate)

owner: 30fa04f4-e047-4241-a9ed-f452529952be · authority: in-loop

## Context (from Tiësto's challenge 5ef3caa3 on h2-preflight-text-free, 2026-08-16)

Live probes (recorded in 2026-08-16-h2-preflight-text-free.md Outcome/F2,
lane commit 651b66b): with an OAuth (Claude Code) carrier credential, any
body with NO top-level `system` is rejected HTTP 429 `rate_limit_error` —
a misleading label for a first-party identity gate. The byte-identical
body plus a first-party system block returns 200; "You are a helpful
assistant." also 429s, so the gate wants the first-party identity, not
just any system text. Interleaved A→429, G→200, A→429 control; holds at
max_tokens 1 and 64 and with role-call-shaped user content.

Affected: `evaluator_request` (quality.rs:758) builds bodies for H2 role
calls 1 (probe) and 2 (faithfulness) WITHOUT `system` — both would 429
the moment H2 is armed on an OAuth carrier. Calls 3/4/5 copy `system`
from the original request and are unaffected. The preflight's identical
defect is fixed in lane h2-preflight-text-free (fixed one-line
first-party system constant) — this task ports that same constant to
`evaluator_request`.

## Task

Add the SAME fixed first-party system constant (introduced by
h2-preflight-text-free — reuse the constant, do not duplicate the
string) to `evaluator_request` so role calls 1 and 2 carry it. Calls
3/4/5 stay untouched. Precondition for arming H2, alongside
h2-preflight-text-free.

## Chaining constraint

Same file as lane h2-preflight-text-free: this task MUST NOT be claimed
until that lane is merged; it builds on the constant that lane
introduces. Same implementer (Tiësto) carries the context.

## Fix

- `evaluator_request` sets `"system"` to the shared first-party constant
  on the bodies it builds for calls 1+2. No captured bytes; no other
  request-shape change; no new failure kinds.
- **One constant, no drift** (Mellow design note, e2d2ee0 review): both
  `build_preflight_request` and `evaluator_request` must read the SAME
  module const (`PREFLIGHT_SYSTEM`, introduced at e2d2ee0 — rename it if
  a preflight-specific name no longer fits its two callers). A test
  asserts the two builders emit an identical system block.
- **Narrow the preflight doc comment** (same note): `preflight_evaluator`
  still claims a green preflight predicts the role calls will work; after
  e2d2ee0 it validates auth/model/usage only. Reword to the narrow claim.
- **Breadcrumb at the disarm site** (same note): `ctx_proxy.rs` disarms
  H2 BEFORE recording the preflight failure, so if the identity gate ever
  changes, H2 disarms permanently under the misleading label
  `rate_limit_error`. Add a comment at the disarm-first call site
  (ctx_proxy.rs:3024 region) naming the trap: a preflight 429
  rate_limit_error may be the first-party identity gate, not quota — see
  2026-08-16-h2-preflight-text-free.md Probe outcome. Comment only, no
  behavior change.
- Pin with fixtures: bodies from `evaluator_request` must contain the
  system block; calls 3/4/5 bodies must carry the ORIGINAL request's
  system unchanged (regression: the port must not leak the constant into
  them). Mutation-verify per workspace standard.

## Constraints (inherited, one section)

- Role-call response parsing untouched (fixed at 9f128b7).
- Content-free metrics: nothing from message text enters metric rows.
- Behavior changes live in `quality.rs` ONLY; the `ctx_proxy.rs` entry
  in the boundary is for the breadcrumb comment, nothing else.
- Boundary: `src-tauri/src/engine/runtime/quality.rs` (tests inline),
  `src-tauri/src/engine/runtime/ctx_proxy.rs` (comment only),
  this plan file.

## Gates (record each via `conclave task gate`)

- `cd src-tauri && cargo test engine::runtime::quality`
- `cd src-tauri && cargo test engine::runtime::ctx_proxy`
- `git diff --check`

## Outcome (2026-08-16, lane `lane/h2-evaluator-first-party-system-v2`, Tiësto)

- `PREFLIGHT_SYSTEM` renamed **`FIRST_PARTY_SYSTEM`** — the name now covers
  its two callers — and moved next to `evaluator_request`, which is where a
  reader meets it first. `evaluator_request` sets `"system"` from it, so
  calls 1 and 2 carry the same line the preflight does; nothing else in
  those bodies moved.
- No new live probe was needed: the axis was already measured live with a
  role-call-shaped body (`max_tokens: 32`, long instruction content) —
  without `system` → 429 `rate_limit_error`, byte-identical body with the
  first-party line → 200. Full table in
  `2026-08-16-h2-preflight-text-free.md` "Probe outcome".
- Three tests pin the result: every fresh body (preflight + calls 1/2)
  emits an IDENTICAL system block; calls 1/2 carry the line; calls 3/4/5
  forward the ORIGINAL request's `system` and never the constant.
- `preflight_evaluator`'s doc comment now states what a green preflight
  does NOT predict (the role calls' text/JSON parse path, their size, and
  calls 3/4/5's model + replayed `system`).
- Breadcrumb added at the `ctx_proxy.rs` disarm-first site (comment only):
  a preflight `rate_limit_error` may be the identity gate, not quota.
- Mutation-verified, 6/6 caught: system line dropped from
  `evaluator_request`; a drifted second copy of the string; system dropped
  from the preflight; the constant emptied; the replay builder leaking the
  constant; the judge builder leaking the constant.
