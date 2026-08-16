# H1 summary generation non_text — diagnosis and fix

owner: 30fa04f4-e047-4241-a9ed-f452529952be · authority: in-loop

## What happened (2026-08-16, after the f9fc1b5 fix went live)

App rebuilt 15:44 local from main@d6b81e6 (contains the generation-400 fix
f9fc1b5). H1 re-armed as campaign `112082b3-86d4-4ca0-9422-bc7c9518194c`
(model `claude-opus-5`, price version `opus5-list-2026-08-16`, ceiling
200000). Test-Proxy-on's conversation was bulk-grown past the ceiling
(~338k real tokens, byte_est 220074) and the sampler admitted and generated.

Result: `proxy_summary_metric` **row id=5** (2026-08-16T08:53:11Z):
`outcome=generation_failure, failure_stage=non_text, error_type=NULL`.
The upstream call was **2xx** — the 400 family is fixed — but the response
`content` array contained at least one block whose `type != "text"`.
Plan facts: a_tokens=338347, tail_start_msg=29, runtime_prefix_messages=2,
source_count=10. gen_* usage columns NULL (parser bailed before usage), but
the upstream DID bill this call (~200k+ input tokens).

## Task

Find what non-text block type the generation response actually contains,
fix minimally so a valid summary is extracted, pin with regression tests.

## Where the failure fires

- Request built at `src-tauri/src/engine/runtime/ctx_proxy.rs:2056-2086`:
  `gen_body = {model, max_tokens: 8192, tool_choice: {"type":"none"},
  messages: gen_prefix + instruction}` plus `system` and `tools` copied
  verbatim from the original request. **No `thinking` field is set.**
- Response parsed at `src-tauri/src/engine/runtime/summary.rs:121-137`:
  the loop requires EVERY content block to be `type == "text"`; the first
  non-text block returns `failure("non_text_content")` → stage `non_text`.

## Hypotheses, most likely first — verify with a live probe, do not assume

1. **Thinking block.** claude-opus-5 may emit `thinking` (or
   `redacted_thinking`) blocks even when the request carries no `thinking`
   param (Claude 5 family adaptive/default-on thinking, or a beta header the
   credential forwards — `generate_summary` applies
   `credential.anthropic_beta` verbatim). Response = `[thinking, text]`
   fails at block 0.
2. **tool_use despite `tool_choice: none`.** Would contradict documented
   API behavior; if observed, the bug is in body construction (e.g.
   tool_choice dropped/overridden), not the API.
3. Any other block type the probe reveals.

## How to reproduce cheaply

- Same harness as h1-generation-400 (your capture replay): reconstruct a
  SMALL gen_body analog — a few messages with tool_use/tool_result history,
  original-style `system` + `tools`, `tool_choice: {"type":"none"}`, the
  SUMMARY_INSTRUCTION user turn, model `claude-opus-5`, no `thinking` —
  POST to the real `/v1/messages` with the same beta headers
  `generate_summary` would apply, and print `content[*].type`. A few cents.
- Vary ONE axis at a time if needed: with/without the credential beta
  header set; with/without `tools`. Two or three calls should pin the cause.

## Fix (gated on probe evidence)

- If thinking-family blocks: make the parser tolerant — skip
  `thinking`/`redacted_thinking` blocks, join the `text` blocks, keep
  `non_text_content` for any OTHER type, keep `empty_text` when no
  non-empty text survives. ALSO evaluate explicitly disabling thinking in
  `gen_body` if the API offers it for this model (cost: thinking output
  bills at $25/MTok and inflates g in the economics); if you disable it,
  the tolerant parser STAYS as defense. Escalate to me only if disabling
  changes cache identity of the copied `system`/`tools` (it must not touch
  them).
- If tool_use: fix the body so `tool_choice: none` actually reaches the
  wire; parser unchanged.
- Regression tests mutation-verified per workspace standard: a fixture
  response `[thinking, text]` must parse to the text; a fixture with a
  genuinely foreign block must still fail `non_text_content`; an all-
  thinking response must fail `empty_text` (or the failure you rule
  correct — note the choice).

## Constraints (inherited, one section)

- `prefix_messages` stays the C/cache authority; only parser and gen_body
  may change. Do not touch the walk-back (f9fc1b5) or H2 builders.
- Content-free metrics: no message text may enter proxy_summary_metric.
- Boundary: `src-tauri/src/engine/runtime/summary.rs`,
  `src-tauri/src/engine/runtime/ctx_proxy.rs` (tests inline in both).
- Lane worktree needs its own `pnpm install` only if you touch `src/` —
  you should not.

## Gates (record each via `conclave task gate`)

- `cd src-tauri && cargo test engine::runtime::summary`
- `cd src-tauri && cargo test engine::runtime::ctx_proxy`
- `git diff --check`

## Amendment — owner ruling on challenge 27913a07 (2026-08-16, credit: Dew)

- **Hypothesis 1 CONFIRMED** by Dew's live probe: claude-opus-5 returns
  `content = [thinking, text]` with NO `thinking` field in the request.
  The probe-first requirement of this plan is satisfied; the fix proceeds
  on that evidence.
- **Scope widened to `quality.rs`** (challenge accepted, option a): H2's
  `extract_text_response` (`quality.rs:654`, doc comment "mirroring the H1
  generation parser") has the identical defect and all five H2 role calls
  would die `non_text_content` against opus-5 the moment H2 is armed. Dew
  ports the same skip-thinking-family parser there, with the same
  mutation-verified test triplet.
- **Mechanics (task boundary is immutable):** the recorded boundary cannot
  change after create, so tooling that enforces it (`stage commit`, the
  integrator's boundary check) treats `quality.rs` as out-of-scope. Land
  the port as a SEPARATE commit in the lane worktree scoped to that one
  path (`git commit -- src-tauri/src/engine/runtime/quality.rs`, normal
  lane identity), so the integrator can attribute it to this ruling
  rather than to the original boundary (protocol from the
  boundary-immutable ruling).
- **H2 request shapes stay UNTOUCHED.** The fix in quality.rs is the
  response parser only. In particular do NOT disable thinking in the H2
  replay call: replay fidelity requires sending what the original CLI
  sent; thinking blocks in H2 responses are legitimate and the parser must
  tolerate them. Any request-shape change to H2 builders is out of scope
  (they were audited and merged at d6b81e6).
- **Added gate:** `cd src-tauri && cargo test engine::runtime::quality`.
- **Thinking stays OFF the gen_body — Dew's evaluation ACCEPTED.** His
  live cache probe (one axis varied, 10956-token cacheable prefix) shows
  `thinking:{"type":"disabled"}` misses the message-block prompt cache
  (cache_read 10956 → 0, full re-write) while omitting the field is
  cache-identical. At a ~200k prefix that trades ~$1.25 of cache-write for
  ~$0.008 of saved thinking output. Parser-only fix is the ruling; the
  probe table lives in this plan's lane-amended Outcome section.

## After merge (integrator + human)

Rebuild + relaunch (arming is in-memory), re-arm with the exact commands in
`docs/superpowers/plans/2026-08-16-h1-generation-400.md` After-merge
section, then drive Test-Proxy-on ONE small turn (its conversation is
already past the ceiling) and expect a `measured` row with non-zero q_h/n_h
in `conclave proxy summary-report`.
