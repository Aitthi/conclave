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
- Pin with fixtures: bodies from `evaluator_request` must contain the
  system block; calls 3/4/5 bodies must carry the ORIGINAL request's
  system unchanged (regression: the port must not leak the constant into
  them). Mutation-verify per workspace standard.

## Constraints (inherited, one section)

- Role-call response parsing untouched (fixed at 9f128b7).
- Content-free metrics: nothing from message text enters metric rows.
- Boundary: `src-tauri/src/engine/runtime/quality.rs` (tests inline),
  this plan file.

## Gates (record each via `conclave task gate`)

- `cd src-tauri && cargo test engine::runtime::quality`
- `git diff --check`
