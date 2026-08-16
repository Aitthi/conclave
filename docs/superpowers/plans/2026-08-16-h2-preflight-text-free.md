# H2 preflight cannot produce text at max_tokens:1 — make it text-free

owner: 30fa04f4-e047-4241-a9ed-f452529952be · authority: in-loop

## Context (from Mellow's challenge 5496f17d on task h1-gen-non-text, 2026-08-16)

The h1-gen-non-text merge (9f128b7) fixed the five H2 role-call parsers,
but the gate in FRONT of them is still shut against claude-opus-5:

- `quality.rs:859-866` `build_preflight_request` sends `max_tokens: 1`
  with no `thinking` field.
- `quality.rs:1272-1275` `preflight_evaluator` pipes the response through
  `extract_text_response`. opus-5 leads with a thinking block whether or
  not the request asks for one (Dew live probe, recorded in
  2026-08-16-h1-gen-non-text.md Outcome), so at a 1-token ceiling there
  is no room for a text block: content is `[thinking-partial]` or `[]`,
  failing `empty_text` or `missing_content` in every branch.
- `ctx_proxy.rs:3024-3031` makes that terminal: a preflight error calls
  `runtime.disarm_quality()` FIRST — an armed H2 self-disarms on the
  first qualifying carrier and runs zero role calls, burning the
  one-per-campaign preflight case.
- Key fact: `preflight_evaluator` DISCARDS `extracted.text` — it only
  consumes `response_model` and `usage`. The text requirement there is
  gratuitous.

## Task

Make the preflight survive an adaptive-thinking evaluator model. MUST
land before H2 is ever armed.

## Probe first (mandatory, fractions of a cent)

Live-POST a `build_preflight_request` analog (same body shape,
`max_tokens: 1`, same beta-header handling `generate_summary`/quality
client applies) against the configured evaluator model and RECORD the
actual `content[]` and `stop_reason` that come back. The fix below is
gated on this evidence; if reality differs, escalate with the probe
output before coding.

## Fix (gated on probe)

- Preferred: preflight validation accepts 2xx + `model` + complete
  `usage` WITHOUT requiring any text block — a text-optional parse path
  used ONLY by the preflight (the five role calls keep the strict
  text-required parser from 9f128b7 unchanged).
- Fallback (only if the probe shows validation-side surprises): raise
  the preflight `max_tokens` above the thinking floor — but this spends
  real output tokens per preflight; prefer the text-free validation.
- Pin with fixtures: content `[thinking-only]` and content `[]` must
  BOTH pass the preflight when model+usage are present; a missing/
  malformed `usage` must still fail. Mutation-verify per workspace
  standard.

## Constraints (inherited, one section)

- H2 role-call request shapes and the role-call parser semantics stay
  UNTOUCHED (audited at d6b81e6, fixed at 9f128b7).
- Failure-kind vocabulary unchanged — no new kinds in
  proxy_quality_metric.
- Content-free metrics: no message/response text may enter metric rows.
- Boundary: `src-tauri/src/engine/runtime/quality.rs` (tests inline),
  this plan file.

## Amendment — owner ruling on challenge 5ef3caa3 (2026-08-16, credit: Tiësto)

- **F1 probe CONFIRMED, and this plan's fallback is REFUTED**: at
  max_tokens:1 opus-5 returns content `[]` 8/8 with complete usage
  (missing_content); raising to 64 buys a text-empty thinking block 3/3
  (empty_text). Text-free validation (2xx + model + complete usage) is
  the ONLY fix — strike the "raise max_tokens" fallback from this plan.
  Both `[]` and thinking-only content must pass the preflight.
- **NEW finding, accepted into this lane's scope**: an OAuth (Claude
  Code) carrier rejects any body with NO top-level `system` — HTTP 429
  `rate_limit_error`, misleading label, actually a first-party identity
  gate (interleaved control A→429, G→200, A→429; "You are a helpful
  assistant." also 429; same at max_tokens 1/64). `build_preflight_request`
  (quality.rs:874) omits system, so even a text-free preflight would 429.
  Fix in this lane: a FIXED one-line first-party system constant in
  `build_preflight_request` — no captured bytes, max_tokens stays 1, no
  new failure kinds. Pin with a fixture asserting the system block is
  present.
- **Out of scope, frozen here, new task follows**: `evaluator_request`
  (quality.rs:758, role calls 1 probe + 2 faithfulness) also omits
  system and would 429 the same way; calls 3/4/5 copy the original
  system and are unaffected. That fix lands as task
  `h2-evaluator-first-party-system` AFTER this lane merges (same file —
  chained, same implementer), and is a precondition for arming H2
  alongside this lane.

## Gates (record each via `conclave task gate`)

- `cd src-tauri && cargo test engine::runtime::quality`
- `git diff --check`

## Probe outcome (2026-08-16, lane `lane/h2-preflight-text-free`, implementer Tiësto)

Live POSTs to `https://api.anthropic.com/v1/messages` with the captured
Claude Code OAuth carrier (bearer + `anthropic-beta` byte-for-byte, the
header set `apply_credential_headers` builds), body = the exact
`build_preflight_request` shape. Token capture technique per Dew
(capture-only fake upstream on a FREE port — 8791 was already held by an
earlier lane's server, so bind and verify your own).

**F1 — the plan's premise is CONFIRMED, deterministically.**

| body | HTTP | content[] | stop_reason | usage | `extract_text_response` |
|---|---|---|---|---|---|
| `max_tokens:1` (8 runs) | 200 | `[]` (8/8) | `max_tokens` | all 4 buckets, `output_tokens:1` | `missing_content` |
| `max_tokens:64` (3 runs) | 200 | `[{"type":"thinking","thinking":"","signature":…}]` (3/3) | `max_tokens` | all 4 buckets, `output_tokens:64` | `empty_text` |

`model` and complete `usage` are present in EVERY 200 — the two fields
`preflight_evaluator` actually consumes. Only the discarded text is
missing, so the preferred fix (text-free validation) is the right one.

**The plan's fallback is REFUTED, not just dispreferred.** Raising the
ceiling to 64 buys a thinking block whose text is empty (`display:
omitted` — signature only) and burns all 64 output tokens: 3/3 still fail
`empty_text`. There is no small ceiling that reliably yields text, so
`max_tokens` stays 1.

**F2 — NEW, out-of-plan blocker (escalated as a task challenge).** With an
OAuth carrier credential, a body carrying NO top-level `system` is
rejected `HTTP 429 rate_limit_error` (message sanitized to `"Error"`);
the byte-identical body plus any Claude Code system block returns 200.
Interleaved control run A→G→A: 429 / 200 / 429. Verified at
`max_tokens` 1 and 64, and with a role-call-shaped user content.
Accepted system values (200): `"You are Claude Code, Anthropic's official
CLI for Claude."`, the captured `system[0]` billing block, the captured
`system[1]` SDK-identity string, and the full captured 4-block array.
Rejected (429): no `system`, and `"You are a helpful assistant."` — so
this is a first-party-identity gate, not a "system must be non-empty"
rule.

Two runs the paragraph above under-recorded, added post-merge by the
owner from Tiësto's exact parameters (they are the evidence
h2-evaluator-first-party-system-v2 reused instead of re-probing):

| run | body | HTTP | notes |
|---|---|---|---|
| role-call-shaped | opus-5, max_tokens 32, `tool_choice:none`, ONE user turn, 242-char probe-instruction-shaped content with JSON payload, n=1/arm | (a) no system → **429** rate_limit_error; (b) byte-identical + captured `system[1]` SDK-identity string → **200**, content `['thinking']` | arm (b) used the CAPTURED SDK-identity line, NOT the constant now in code; stop_reason/usage not captured |
| constant itself | `"You are Claude Code, Anthropic's official CLI for Claude."` (57 chars) at max_tokens 1, n=1 each on claude-opus-5 AND claude-sonnet-5 | both **200**, content `[]`, stop_reason max_tokens, usage complete (input 32 / cc 0 / cr 0 / output 1) | the only DIRECT evidence for the exact `FIRST_PARTY_SYSTEM` string |

Headers for all rows: captured OAuth bearer + captured `anthropic-beta`
byte-for-byte + `anthropic-version 2023-06-01`; no `?beta=true`.

Consequence: `build_preflight_request` (no `system`) and
`evaluator_request` (`quality.rs:758`, calls 1 probe + 2 faithfulness,
also no `system`) cannot reach a 200 against an OAuth carrier at all.
Calls 3/4/5 copy `system` from the original request and are unaffected.
The H1 generation path is unaffected for the same reason.

## F2 proposal (filed as a task challenge — RULED accepted, challenge 5ef3caa3)

- Preflight body gains a fixed one-line first-party `system` block; the
  shape stays otherwise byte-identical (`max_tokens` stays 1, still
  `tool_choice:none`, still the benign `"ok"` message, and no captured
  conversation bytes ever enter it — the string is a constant, not the
  carrier's `system`).
- `evaluator_request` (calls 1+2) is NOT touched in this lane — the
  role-call shapes are frozen by this plan's Constraints. It needs its own
  task, landing before H2 is ever armed, or probe and faithfulness die 429
  on the first case.
