# Infinity-Turn Checkpoint (ctx-proxy Phase 2) — Design v4

**Status:** v3 COUNCIL-REVIEWED (Detoro chair, Aoki co-principal); v4 hybrid-LLM-summary addendum is a **PROPOSAL UNDER REVIEW**, not approved for implementation. In-loop authority granted by human 2026-07-11; human reviews the finished result. M1 instrumentation is viable under the amended metric contract; M2 rejected the naive checkpoint and motivated the proposed hybrid path in §10–§11.
**Distinct from** Phase-1 dedup proxy, which is NO-GO/shelved (plan A9, commit 5ef698d).
**Predecessor evidence:** `docs/superpowers/plans/2026-07-10-agent-proxy-phase1.md` (A9), blackboard `measure:proxy-025-verdict` / `measure:proxy-025-ruling`.

## 1. Problem & goal

Agents degrade — slower, lower-quality output — once live context grows past roughly **400–500k tokens**, even on a 1M-window model. The harness self-compacts at ~70% (~700k on 1M), **already past** the degradation zone and as one disruptive event, not continuous management.

**Goal:** a proxy-managed **"infinity turn"** — keep the **effective context** (what is sent upstream) inside the high-quality band at all times by compacting recoverable old tool-output in the background, so the model always operates in its good zone.

**This is a QUALITY objective, not a cost objective.** Cost is secondary. Phase-1's lesson stands: "input reduction" is the wrong *primary* objective under prompt caching.

## 2. Economics — stated honestly (was overclaimed; challenge a06953a8)

Phase-1 elided only duplicates → tiny S → cache rebuild never amortized. Phase-2 freezes a large recoverable block once, so S is larger — **but "large absolute S" does not imply fast break-even.** The governing quantity is **q = S_net / R**, where:
- **R** = tokens in the invalidated cache suffix, starting at the **earliest changed block** (likely most of the prompt).
- **S_net** = tokens actually removed, **net of** stub text + any index overhead + all kept non-recoverable outputs.

From the recorded cache model, break-even future rounds `n = 11.5/q − 12.5`; `n ≤ 2` requires **q ≥ 0.793** — a high bar. **No "~2 turns" claim is made.** Whether Phase-2 pays is decided empirically by the Milestone-1 metric contract (§7.1), not asserted here. Note byte-S ≠ the token quantity that defines the quality ceiling; the gate measures tokens.

## 3. Core mechanism (reuses `ctxopt`; changes the policy)

Retains the `policy → analyze → apply → validate` pipeline (`src-tauri/crates/ctxopt/`, driven by `ctx_proxy.rs`). Phase-2 replaces the dedup policy with a **checkpoint policy**:

- **Freeze region.** Everything before the **recent tail** (last N messages / ~80–100k tokens, kept verbatim). Within it, stub **only recoverable** tool_results.
- **Recoverability — two properties, not one (challenge 22ec8a2b):**
  - *Capability-resident loss (ACCEPTED):* eliding a `Read`/`Grep`/`Glob`/`LS`/search/code-intel result removes it from context; the agent can re-obtain **current** state on demand. This is lossy w.r.t. the **historical** bytes the model saw — accepted, same tradeoff the harness's own compaction makes.
  - *Exact-output recovery (NOT provided in v1):* would require a content-addressed snapshot artifact persisting original bytes. Deferred (see §9).
  - *Non-recoverable → kept verbatim, tracked as a separate S bucket:* arbitrary `Bash` (side effects, mutable output — `cat`/`ls` are current-state, not historical recovery), `WebFetch` (drifts), `Write`/`Edit`. Only **content-addressed** reads (`git show <full SHA>`) are safely rerunnable and may later be reclassified recoverable.
- **Breadcrumbs, NOT an index block (challenge 7e325df7).** Each stub is self-describing in its own `tool_result` content: `[ctxopt checkpoint: elided Read <path> @turn N — re-read to restore]`. No separate message/block is added, so the strict validator (§5) still holds. A collective manifest, if ever needed, is deferred to the apply-path design (§9).
- **Escape hatch.** A re-read lands in the live tail (not frozen) → content returns naturally.

## 4. Checkpoint triggering & cache stability (thrash-guarded; challenge 6ea4c87f)

- Frozen bytes are **immutable once written** (Phase-1 D5); the checkpoint set is **monotonic**.
- **High/low-water hysteresis, not ΔN-only.** Fire a checkpoint **only if** projected **net saving > M** AND projected **post-context ≤ low-water L (< ceiling)**. Otherwise emit a **`saturated`/`unmanageable`** metric and make **no wire, state, or cache change** — a traffic shape dominated by non-recoverable growth or live-tail growth must not thrash the cache.
- **AMENDMENT R8 (challenge 4f3aa72a + c08a3cf1, Detoro chair + Aoki, 2026-07-11).** The `M`/`L` decision above is evaluated on **real `count_tokens` values, never on `bytes/4`.** The original M1 implementation applied M/L in `bytes/4` space (`est_whole_tokens = body.len()/4`); empirically `bytes/4` overstates real tokens ~4.7× (18.7–19.0 bytes/real-token on real cache-heavy traffic), so real ~395k-token conversations read as ~1.85M, fail `post ≤ L`, and are dropped as `saturated` **without a metric row** → 0 interpretable samples. Corrected M1 contract in §7.1.
- The checkpoint boundary is represented explicitly; prior stub bytes never mutate.
- **Do not invent a cache breakpoint.** Milestone-1 only **observes** the actual incoming `cache_control` positions the client already sets (validator forbids changing them — validate.rs:63-70). Any deliberate breakpoint placement is an apply-path concern, deferred.

## 5. Fail-open & equivalence guard (unchanged, strict)

Any error on the checkpoint path → forward the **original request untouched** (Phase-1 invariant). `validate` stays strict: message count unchanged, every `tool_use` keeps a matching `tool_result`, block/key sets unchanged, only `tool_result.content` shrinks. Per-result stubs (§3) are compatible with this by construction; that is why breadcrumbs live inside stubs, not in a new block.

## 6. Configuration — GLOBAL in v1 (per-agent is impossible today; challenge d5452139)

The proxy runtime is **app-global** (`ctx_proxy.rs:42-50`, one atomic mutated by `commands/proxy.rs`; `instance.rs` routes every opted-in agent to the same port with **no agent identity**). Therefore v1 = a **global** toggle: `proxy checkpoint on|off`, `proxy ceiling <tokens>`, tail size, default **off**. Fleet-wide **log projection** is fine globally. **Per-agent control and any single-agent apply trial are BLOCKED** until an isolation design lands: agent/conversation identity carried to the proxy (or a per-agent endpoint/port), CLI targeting an agent id, and a test that enabling one agent cannot rewrite another's traffic.

## 7. VALIDATION GATE (mandatory, ordered)

**Milestone-1 — log-mode projection (the only work greenlit now).** Implement the checkpoint policy in **log mode only**: compute what a checkpoint *would* stub and record the full metric contract per candidate, **without altering upstream bytes**. Metric contract (challenge a06953a8): earliest-changed byte/message, **R** (invalidated-suffix tokens), gross candidate tokens, stub+breadcrumb overhead, **S_net** (tokens), **q = S_net/R**, projected cache break-even, projected post-checkpoint tokens, and observed expected plateau turns — plus a separate non-recoverable-kept bucket. Persist into `proxy_request_metric` or a sibling table. **Pass criterion:** post-context enters a defined low-water band on real long-context traffic **AND** q/plateau support the cost bound. S alone does not pass.

**Token counting — the gate is token-level but `ctxopt/estimate.rs` is only `bytes/4` and SSE usage is whole-request (challenge da69a2b7).** The metric contract's tokens are obtained via Anthropic's `count_tokens` endpoint (accepts the full structured request, free, separately rate-limited, returns a provider *estimate*). For sampled checkpoint candidates, call `count_tokens` on **(a)** the original full request, **(b)** the projected (post-checkpoint) full request, **(c)** a structurally valid prefix ending at the message boundary **before** the earliest changed block. Then **S_net = a − b**, **R = a − c** (message-boundary rounding makes R conservatively ≥ the true changed suffix), **q = S_net/R**, projected-post = b. Label every value a provider estimate; record count failures, model, and method/version; keep `bytes/4` only as a diagnostic. **Plateau** is *observed*, not an instantaneous field: the number of subsequent requests that hold the same projected frozen boundary until the next eligible checkpoint or harness compaction. Sampling is **asynchronous/queued** so `count_tokens` RPM and latency can never touch forwarded traffic. **Plan prerequisite:** verify the live Claude credential supports `count_tokens` before the gate depends on it.

**AMENDMENT — globally closed C prefix + conditional beta count route (ruling `f4a78a94`, Detoro chair, 2026-07-12).** The C boundary MUST retain zero unmatched `tool_use` IDs and zero `tool_result` IDs without their use. Because one assistant turn may issue parallel tools whose results span multiple blocks or following user messages, this is not an `idx - 1` rule: starting from the earliest changed result boundary, back up across the whole implicated tool cycle until the entire retained prefix is closed. Excluding that cycle keeps **R = a − c** conservatively greater than or equal to the true changed suffix. For the count request itself, a credential with no `anthropic-beta` uses stable `POST /v1/messages/count_tokens` and invents no beta header; a captured `anthropic-beta` uses `POST /v1/messages/count_tokens?beta=true`, preserves the caller's beta values and order, and idempotently appends `token-counting-2024-11-01` exactly once. A count failure is tagged with exactly one content-free stage literal `a`, `b`, or `c` before the existing status + allowlisted `error.type`; response bodies and `error.message` remain forbidden.

**AMENDMENT — credential lifted-set includes `anthropic-beta` (Detoro in-loop, 2026-07-11; reconciles containment `ea3df57c`).** The count-sampler lifts ONLY these headers off the forwarded request and re-applies them to the `count_tokens` call against the *same captured upstream* (`job.upstream`, never re-read from global state): `x-api-key`, `authorization`, `anthropic-version`, **and `anthropic-beta`**. `ea3df57c` originally restricted the lifted set to auth headers to stop a retargeted global upstream from receiving the credential; `anthropic-beta` is added because it is **not a secret**, it is **required for the lifted OAuth Bearer auth to be accepted** (and for `[1m]` 1M-context), and forwarding it to the same captured upstream does not weaken retargeting containment. The captured comma-separated values retain their original text and order; under ruling `f4a78a94`, the beta count route additionally ensures `token-counting-2024-11-01` is appended exactly once. Without the lifted values every OAuth-authenticated sample returned `count_failure` (blind instrument). Because the GUI app's stderr → `/dev/null`, the `count_tokens` error is now persisted durably in `proxy_checkpoint_metric.error_snippet` as **HTTP status + an allowlisted content-free `error.type` enum** (status-only `(unknown type)`/`(unparsed)` otherwise; NEVER the raw body or `error.message` — rulings `e376fec5` + `f8651210`, challenge `fda10918`), `count_failure` rows only; NOT in `checkpoint-report`. Fix task: `proxy-m1-counttokens-beta`.

**M1 measurement mechanic — corrected (R8; challenge 4f3aa72a + c08a3cf1).** `bytes/4` is a cheap **sample-TRIGGER only** (decide whether to spend a `count_tokens` sample: byte-est > ceiling **and** ≥1 recoverable candidate outside the tail) and a recorded **diagnostic** — it is **never** the M/L authority. Classification is decided **after** `count_tokens`, on real tokens, into **three buckets**, and a metric row is **always persisted** (add an `outcome` column to `proxy_checkpoint_metric`):
  - **`below_ceiling`** — real `a ≤ ceiling` (the byte-trigger was a false positive); record `a` + byte diagnostics, no q claim.
  - **`eligible`** — `a > ceiling` **and** `S_net = a−b > M` **and** `projected_post = b ≤ L`; record the full a/b/c contract (R, S_net, q, projected-post, plateau).
  - **`saturated`** — `a > ceiling` but M or L not met; record the full a/b/c contract so the *distribution* of near-misses is visible.
  Two thresholds must not be conflated: the **byte-space trigger** (generous, gates count_tokens spend) and the **real-token ceiling** compared to `a`. Async queue-drop stays a separate counter (`checkpointSamplesDropped`), never a metric row. The old bytes/4 M/L pre-gate is **removed**.

**§7.2 — accounting is a DESIGN GATE before apply (challenge c19ef7b0).** Because the ledger identifies conversations by first-message + prefix hash (`ledger.rs:47-52`), harness self-compaction (which rewrites/drops the prefix) **resets** checkpoint state and the cache plateau. So harness accounting is not a mere "claim adjustment": settle it via a **controlled mock-upstream experiment** (hold response content constant, vary only returned `usage`) if Claude Code permits, else a live spike. **If own-list accounting**, narrow the product to pre-compaction quality shaping or harness integration; **do not claim infinity-turn.**

**§7.3 — apply trial (BLOCKED until isolation §6 + accounting §7.2 settle).** Predefine the quality evaluation *before* any apply: replay matched long-context checkpoints, **baseline vs projected** context, with **blinded** next-action / task-outcome scoring; the isolated live agent is a **safety** validation only, not the primary quality measure.

**Honest risks:** (a) M2 confirmed structural recoverable S is insufficient for the naive checkpoint; §10 records the NO-GO and §11 proposes hybrid LLM-summary, which may produce a bigger S but adds generation cost and a materially harder quality proof; (b) accounting (§7.2) is unverified; (c) quality equivalence is measured, never assumed.

## 8. Testing

Unit (`ctxopt`): deterministic checkpoint (same input → same frozen set), recoverability classifier, recent-tail preserved, monotonic frozen set; `apply` → valid JSON, tool_use/tool_result pairs intact; `validate` rejects structure changes; **byte-stable test: identical input across turns → identical output**. **Adversarial (challenge 6ea4c87f):** zero-new-eligible growth and non-recoverable-heavy growth must emit `saturated` and change nothing. Metric-contract tests: R, S_net, q computed correctly incl. overhead. **Token-counting (R7):** a live-credential preflight that `count_tokens` is authorized (plan prerequisite); the async sampling queue never blocks or delays forwarded traffic even under count_tokens rate-limit/latency; a/b/c derivation (S_net=a−b, R=a−c) computed on a fixture; count-failure paths recorded, not fatal. Fail-open: parse/classify/validate errors → original bytes.

## 9. Boundary (anticipated) & deferred

Milestone-1 boundary: `src-tauri/crates/ctxopt/` (checkpoint policy + recoverability classifier, alongside dedup), `ctx_proxy.rs` (trigger/ceiling wiring, metric emission), `commands/proxy.rs` (`checkpoint`/`ceiling`), CLI argv mapping, metric migration if a sibling table is used. Exact paths finalized in the plan. **Deferred (apply-path, not Milestone-1):** per-agent isolation design (§6), accounting resolution (§7.2), a content-addressed snapshot store for exact-output recovery (§3), and any collective manifest/breakpoint representation with its own narrowly-proven validator.

## 10. M2 verdict — naive prefix-checkpoint NO-GO (2026-07-12)

M2 measured 25 successful `count_tokens` samples across four conversations at a 100k ceiling (`count_failure=0`). `q` ranged **0.146–0.526**, average **0.273**; zero samples reached the **0.793** threshold required for cache break-even within two subsequent turns. The implied break-even was 9–66 turns. Evidence is on parent task `infinity-turn-checkpoint` and blackboard `measure:proxy-m2-q`.

This is structural, not an instrumentation miss: observed proxy traffic was approximately **99.8% `cache_read`**. Most tokens belong to a stable cached prefix; the narrow rerunnable-tool classifier exposes only a small fraction as safely droppable. Rewriting that small fraction invalidates a much larger cache suffix, and dropping more bytes without preserving their meaning would increase quality risk. The naive apply path is therefore shelved. The sampler remains useful as the token-level measurement substrate for the hybrid hypothesis below.

The 100k acquisition ceiling is a caveat, not a reason to reverse the verdict: M2 did not sample the 400–500k degradation band. Hybrid may proceed to shadow measurement only if its final GO decision includes real samples in that band; 100k samples may calibrate the instrument but cannot establish the product claim.

## 11. Hybrid LLM-summary proposal (REVIEW-GATED; no build authorized)

### 11.1 Hypothesis and safety boundary

The hybrid path replaces **semantic content**, not just rerunnable bytes: an LLM condenses an old, closed span of tool results into a structured summary that remains on the wire, while deterministic tombstones replace the other covered result bodies. This can address the low-q failure only if old tool results contain substantial semantic redundancy that a summary can preserve much more compactly.

The first proposal deliberately stays inside the existing structural proof:

- **Eligible source:** textual `tool_result.content` in complete tool-use/result cycles before the recent tail. Unlike the naive policy, results from non-rerunnable tools (`Bash`, `WebFetch`, etc.) may be summarized because their material facts are retained, not discarded. The summarizer receives the paired tool name/input and enough surrounding user/assistant text to interpret each result, but that surrounding text is reference context, not removable content.
- **Kept verbatim:** system prompt, tool definitions, every user block, every assistant block and `tool_use` input, the 80–100k-token recent tail, non-text/image/document result blocks, malformed or unmatched tool cycles, and any result for which the accepted summary would not be strictly smaller.
- **Summary placement:** one target `tool_result.content` near the end of the frozen span carries the structured aggregate summary; the other covered result contents become short deterministic `[ctxopt summary <checkpoint-id>: covered by aggregate @turn N]` tombstones. The candidate is rejected unless the carrier and every tombstoned result individually shrink. Message count, roles, block types, key sets, tool IDs, and all sibling fields remain unchanged, so `ctxopt::validate` can continue to prove that only selected `tool_result.content` shrank.
- **No arbitrary message splice in v1:** deleting user/assistant turns or inserting a synthetic summary message would weaken the current equivalence guard and erase instructions/decisions that M2 did not prove safe to remove. Whole-message compaction is a separate future design requiring a new validator and its own quality evidence.

The summary contract is factual and reference-oriented, not narrative. It must retain: user constraints and acceptance criteria mentioned by a result; decisions and rejected alternatives; exact identifiers, paths, commands, versions, errors, and numeric findings needed later; mutations already performed; unresolved questions/blockers; negative results; and provenance as covered `tool_use_id` references. It must say `unknown` rather than infer. Tool output is delimited as untrusted data so prompt-like text inside a result cannot override the summarization instruction.

An accepted summary is **immutable**. The first accepted bytes, source-boundary hash, prompt version, and summarizer model version form a checkpoint record; later turns reuse those exact bytes. The same frozen span is never regenerated, because a regenerated paraphrase would thrash the cache. A later checkpoint may summarize only newly frozen material or create a new aggregate that includes the prior summary under an explicit generation bump. Apply remains blocked on the identity/isolation and harness-accounting gates in §6/§7.2; an in-memory ledger is sufficient for shadow measurement but not for an infinity-turn product claim.

### 11.2 Why this might beat low q — and why 99.8% cache-read may still defeat it

Naive `q` was capped by the bytes labeled rerunnable. Hybrid expands the candidate pool to old textual tool results whose **meaning** may be compressible even when the original output is not reproducible. Large logs, repeated file reads, search results, build output, and superseded investigation trails can therefore contribute to saving. If those dominate the invalidated suffix, hybrid `q_h` can be much larger than M2's 0.146–0.526.

However, 99.8% `cache_read` is not evidence that the prefix is irrelevant. It says the prefix is stable and currently cheap to reuse; the agent may still depend on it. Hybrid deliberately exchanges byte-level recoverability for a semantic-sufficiency claim, so it raises rather than removes the quality burden. It also invalidates that cheap prefix once, just like the naive rewrite. If the retained user/assistant/tool-use bytes dominate, if the summary must preserve most tool-result facts, or if behavioral replay shows that omitted detail changes the next action, hybrid is also a NO-GO. No compression ratio can overrule a quality failure.

### 11.3 Token and dollar economics

For one candidate, measured with the existing provider estimator:

- `A` = tokens in the original request.
- `C` = tokens in the globally closed prefix before the earliest changed result (reuse `count_tokens::prefix_messages`).
- `R = A − C` = invalidated cache suffix.
- `B_h` = tokens in the projected request containing the aggregate summary and tombstones.
- `S_h = A − B_h` = net tokens saved, already net of the aggregate summary and tombstone overhead.
- `q_h = S_h / R` = physical rewrite efficiency. It is the analog of q, but is **not** sufficient for a GO because it excludes the generation call and semantic loss.

Let `p_w` and `p_r` be the actual per-token cache-write and cache-read prices for the forwarded model, and let `C_gen` be the measured dollar cost of the one-off summary call, including uncached input, cache creation/read, and output tokens at the summarizer model's rates. The incremental cost of the rewritten turn is:

`Δ0 = p_w × (R − S_h) − p_r × R + C_gen`

Each later stable turn saves `p_r × S_h`, so the number of **subsequent** turns required to amortize the checkpoint is:

`n_h = max(0, Δ0 / (p_r × S_h))`

Under the prior normalized Anthropic cache ratios (`p_w = 1.25 p_i`, `p_r = 0.10 p_i`) and `g = C_gen / (p_i × R)`, this becomes:

`n_h = max(0, 11.5/q_h − 12.5 + 10g/q_h)`

This is the new break-even metric. The earlier `q ≥ 0.793` rule is recovered only when `g = 0`; summary generation makes the required compression stricter. The summary request should preserve the original byte-identical prefix/cache markers where the API contract permits and append a no-tools summarization instruction after a closed cycle, because it may reuse the cache that dominates live traffic. That reuse is **not assumed**: the summary response's actual `input_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens`, and `output_tokens` determine `C_gen`. If the summary call cannot reuse the prefix, or if its output price dominates, the economics may be impossible even with excellent compression.

Prices are versioned inputs to the report, not constants in code. Store token usage, model IDs, prompt version, and price-schedule version separately so historical measurements can be recomputed. The quality objective may justify a bounded cost premium, but that exception must be an explicit product ruling; the measurement report must not relabel a negative `n_h` result as a cost win.

### 11.4 Quality preservation and validation

There are three independent gates, all fail-closed for the candidate and fail-open for forwarding:

1. **Structural gate:** closed tool cycles; recent tail and protected bytes identical; every changed result strictly smaller; current `ctxopt::validate` passes; projected request passes provider `count_tokens`. Any failure discards the candidate and forwards the original request.
2. **Faithfulness gate:** a verifier sees the source span and summary, checks every summary claim against cited `tool_use_id` evidence, and scores a source-derived probe set covering constraints, decisions, mutations, exact identifiers/errors, negative findings, and open work. A critical hallucination or omission rejects the candidate. Verifier model/version and rubric are pinned and recorded; the summarizer cannot grade itself.
3. **Behavioral gate:** on a no-side-effect replay, call the task model with the original and projected contexts, tools disabled, to produce the next-action plan. Blind randomized judging compares correctness, constraint adherence, and selected next action; neither tools nor mutations are executed. This tests what matters—the agent's behavior—not prose similarity. A stratified human audit of accepted, rejected, and near-threshold cases checks judge drift.

Raw requests, raw summaries, credentials, and verifier prompts must not enter the metric database or logs. A shadow job holds them only in memory for the duration of generation/count/verification, then persists bounded numeric scores, content hashes, model/prompt versions, token usage, failure stage, and allowlisted error types. The summarizer and verifier use the exact upstream captured for the forwarded request, a no-redirect client, lifted sensitive headers with the same containment rules as `count_tokens`, no tools, explicit timeouts, no retries on forwarded latency, and a global semaphore/cooldown. They are off the forwarding path and cannot delay or alter the real request.

### 11.5 Integration seams (anticipated, not authorized)

- `src-tauri/crates/ctxopt/`: add a pure summary-span planner beside `checkpoint.rs`; identify globally closed cycles, protected blocks, carrier/tombstone placement, byte-stable checkpoint IDs, and projection diagnostics. Reuse `apply::stub_tool_results` and the existing strict `validate`; do not add network/model code to this crate.
- `src-tauri/src/engine/runtime/ctx_proxy.rs`: retain the cheap byte trigger and global off-path queue. Extend `CheckpointJob` (or add a distinct `SummaryJob`) with the source boundary/hash and captured upstream, invoke a dedicated summarizer/verifier client asynchronously, build the projection, run a/b/c counts, and persist one terminal outcome for every admitted job. Forwarded bytes remain untouched throughout shadow milestones.
- `src-tauri/src/engine/runtime/count_tokens.rs`: reuse `CountCredential`, no-redirect client policy, `count_tokens_body`, the beta route/header handling, safe error taxonomy, and the globally closed `prefix_messages` C boundary. The hybrid measurement still computes `R=A−C` and `S_h=A−B_h`; it must not regress to `bytes/4` authority.
- Runtime generation code belongs in a separate `summary.rs` so count-only security and error handling remain narrow. It constructs the cache-preserving, no-tools request; records actual usage needed for `C_gen`; and returns content or a bounded content-free error stage.
- Persist hybrid rows in a sibling `proxy_summary_metric` table (or a versioned schema with an unambiguous method discriminator), not as naive M2 rows. Required fields: A/B/C/R/S_h/q_h; summary/tombstone tokens; generation usage by cache bucket and output; `C_gen`, price version, `n_h`; projected post tokens; plateau; faithfulness/probe/behavior scores; prompt/summarizer/verifier versions; outcome/failure stage; source-boundary and summary hashes. Never persist bodies.

### 11.6 Ordered live measurement plan and GO/NO-GO bars

**H0 — deterministic fixtures, no model traffic.** Prove closed-cycle selection, protected-byte identity, carrier/tombstone shrink rules, structural validation, byte-stable reuse, prompt-injection fixtures, count-prefix closure, and fail-open forwarding. A single structural mismatch is a stop.

**H1 — shadow token/economics measurement, no forwarded-byte change.** Re-arm the existing sampler after app relaunch, generate candidate summaries off-path, and reuse M1's a/b/c instrument. Acquire at least **30 successful candidates across at least 10 conversations**, including at least **10 candidates whose real A is in the 400–500k degradation band**; 100k samples are calibration only. Report distributions by conversation, never only pooled averages. Advance to H2 only if: count/generation failure ≤5%; at least 90% of candidates land at or below the defined low-water L; at least 80% have `n_h ≤ 2` using measured `C_gen`; and no conversation has all candidates miss. These retain M2's two-turn cost standard while charging the new call honestly.

**H2 — shadow quality validation, still no forwarded-byte change.** Evaluate at least **100 stratified cases** across live in-memory candidates plus adversarial/replayable fixtures; include side-effecting outputs, long logs, exact error diagnosis, rejected alternatives, parallel tool cycles, and prompt-like text inside tool output. GO requires: zero critical hallucinations; zero critical omissions; source-probe recall ≥98%; no structural failures; and blinded next-action non-inferiority whose 95% confidence lower bound is no worse than **−5 percentage points** versus original context. Any critical loss is a design failure, not something compression economics can average away.

**H3 — isolated apply safety trial.** Still BLOCKED until §6 identity/isolation and §7.2 harness accounting are settled. Only after H1+H2 pass may one isolated agent reuse the exact accepted summary bytes. Monitor task outcome, re-read/recovery attempts, latency, cache usage, `n_h` versus observed plateau, and emergency fail-open. The live trial is a safety check, not the primary quality proof.

The hybrid decision replaces the single q threshold with a vector: **`q_h` for physical compression, `n_h` for cost-adjusted amortization, and faithfulness/behavioral non-inferiority for quality.** H1 or H2 failure is NO-GO for apply. Passing them authorizes an apply-path design review, not implementation by implication.

## Decision log — council rulings (Detoro chair)

All seven of Aoki's challenges ACCEPTED after evidence verification; credit Aoki.
- **R1 (22ec8a2b) recoverability:** dropped "lossless"; split capability-resident (accepted lossy) vs exact-output (needs snapshot). §3.
- **R2 (d5452139) isolation:** v1 global toggle; per-agent + apply trial blocked until isolation design. §6.
- **R3 (7e325df7) index/breakpoint vs validator:** no index block, no invented breakpoint; breadcrumbs inside per-result stubs; validator stays strict. §3/§4.
- **R4 (a06953a8) economics:** deleted "~2 turns"; gate on q=S_net/R with full metric contract. §2/§7.1.
- **R5 (6ea4c87f) thrash:** high/low-water + min-net-saving M + low-water L; saturated→no-op; adversarial tests. §4/§8.
- **R6 (c19ef7b0) accounting+quality:** §7.2 upgraded to a design gate (mock-upstream accounting experiment); predefined blinded quality eval. §7.
- **R7 (da69a2b7) token counting:** gate is token-level but telemetry is `bytes/4` + whole-request usage; defined a `count_tokens` a/b/c sampling algorithm (S_net=a−b, R=a−c, q=S_net/R), async/queued, credential verified as a plan prerequisite; plateau redefined as observed. §7.1/§8.
- **R8 (4f3aa72a + c08a3cf1) M1 0-sample defect (Detoro chair + Aoki co-principal, post-implementation):** the shipped M1 gate applied M/L in `bytes/4` space and dropped `Saturated` with no metric row → 0 interpretable samples on real (~4.7× over-estimated) cache-heavy traffic. Ruling: `bytes/4` is a sample-trigger + diagnostic only; classify **after** `count_tokens` into `below_ceiling|eligible|saturated`; **always** persist a row (`outcome` column added); remove the bytes/4 M/L pre-gate. §4/§7.1. Fix task: `infinity-turn-checkpoint-m1-fix`.
- **R9 (74bbaf61, 2026-07-12; credit Aoki) first hybrid safety boundary:** preserve the proven strict validator and summarize only old, closed-cycle `tool_result.content`; keep user/assistant/message structure verbatim. Expanding candidates to all old tool results is a hypothesis, not proof of value, so measured summarizer cost is binding in `n_h`. Whole-message splicing is deferred pending a new validator and separate quality proof. §11.1/§11.3.
