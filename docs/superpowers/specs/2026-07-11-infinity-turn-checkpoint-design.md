# Infinity-Turn Checkpoint (ctx-proxy Phase 2) — Design v2

**Status:** COUNCIL-REVIEWED (Detoro chair, Aoki co-principal). Six challenges filed and ACCEPTED; all folded below (decision log at tail). In-loop authority granted by human 2026-07-11; human reviews the finished result. Verdict: **rework-before-plan done; measurement-only Milestone-1 is viable under the amended metric contract.**
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
- The checkpoint boundary is represented explicitly; prior stub bytes never mutate.
- **Do not invent a cache breakpoint.** Milestone-1 only **observes** the actual incoming `cache_control` positions the client already sets (validator forbids changing them — validate.rs:63-70). Any deliberate breakpoint placement is an apply-path concern, deferred.

## 5. Fail-open & equivalence guard (unchanged, strict)

Any error on the checkpoint path → forward the **original request untouched** (Phase-1 invariant). `validate` stays strict: message count unchanged, every `tool_use` keeps a matching `tool_result`, block/key sets unchanged, only `tool_result.content` shrinks. Per-result stubs (§3) are compatible with this by construction; that is why breadcrumbs live inside stubs, not in a new block.

## 6. Configuration — GLOBAL in v1 (per-agent is impossible today; challenge d5452139)

The proxy runtime is **app-global** (`ctx_proxy.rs:42-50`, one atomic mutated by `commands/proxy.rs`; `instance.rs` routes every opted-in agent to the same port with **no agent identity**). Therefore v1 = a **global** toggle: `proxy checkpoint on|off`, `proxy ceiling <tokens>`, tail size, default **off**. Fleet-wide **log projection** is fine globally. **Per-agent control and any single-agent apply trial are BLOCKED** until an isolation design lands: agent/conversation identity carried to the proxy (or a per-agent endpoint/port), CLI targeting an agent id, and a test that enabling one agent cannot rewrite another's traffic.

## 7. VALIDATION GATE (mandatory, ordered)

**Milestone-1 — log-mode projection (the only work greenlit now).** Implement the checkpoint policy in **log mode only**: compute what a checkpoint *would* stub and record the full metric contract per candidate, **without altering upstream bytes**. Metric contract (challenge a06953a8): earliest-changed byte/message, **R** (invalidated-suffix tokens), gross candidate tokens, stub+breadcrumb overhead, **S_net** (tokens), **q = S_net/R**, projected cache break-even, projected post-checkpoint tokens, and observed expected plateau turns — plus a separate non-recoverable-kept bucket. Persist into `proxy_request_metric` or a sibling table. **Pass criterion:** post-context enters a defined low-water band on real long-context traffic **AND** q/plateau support the cost bound. S alone does not pass.

**§7.2 — accounting is a DESIGN GATE before apply (challenge c19ef7b0).** Because the ledger identifies conversations by first-message + prefix hash (`ledger.rs:47-52`), harness self-compaction (which rewrites/drops the prefix) **resets** checkpoint state and the cache plateau. So harness accounting is not a mere "claim adjustment": settle it via a **controlled mock-upstream experiment** (hold response content constant, vary only returned `usage`) if Claude Code permits, else a live spike. **If own-list accounting**, narrow the product to pre-compaction quality shaping or harness integration; **do not claim infinity-turn.**

**§7.3 — apply trial (BLOCKED until isolation §6 + accounting §7.2 settle).** Predefine the quality evaluation *before* any apply: replay matched long-context checkpoints, **baseline vs projected** context, with **blinded** next-action / task-outcome scoring; the isolated live agent is a **safety** validation only, not the primary quality measure.

**Honest risks:** (a) structural recoverable S may be insufficient → escalate to hybrid LLM-summary (bigger S, adds cost/latency/quality risk) — Milestone-1 tells us first; (b) accounting (§7.2) unverified; (c) quality equivalence is measured, never assumed.

## 8. Testing

Unit (`ctxopt`): deterministic checkpoint (same input → same frozen set), recoverability classifier, recent-tail preserved, monotonic frozen set; `apply` → valid JSON, tool_use/tool_result pairs intact; `validate` rejects structure changes; **byte-stable test: identical input across turns → identical output**. **Adversarial (challenge 6ea4c87f):** zero-new-eligible growth and non-recoverable-heavy growth must emit `saturated` and change nothing. Metric-contract tests: R, S_net, q computed correctly incl. overhead. Fail-open: parse/classify/validate errors → original bytes.

## 9. Boundary (anticipated) & deferred

Milestone-1 boundary: `src-tauri/crates/ctxopt/` (checkpoint policy + recoverability classifier, alongside dedup), `ctx_proxy.rs` (trigger/ceiling wiring, metric emission), `commands/proxy.rs` (`checkpoint`/`ceiling`), CLI argv mapping, metric migration if a sibling table is used. Exact paths finalized in the plan. **Deferred (apply-path, not Milestone-1):** per-agent isolation design (§6), accounting resolution (§7.2), a content-addressed snapshot store for exact-output recovery (§3), and any collective manifest/breakpoint representation with its own narrowly-proven validator.

## Decision log — council rulings 2026-07-11 (Detoro chair)

All six of Aoki's challenges ACCEPTED after evidence verification; credit Aoki.
- **R1 (22ec8a2b) recoverability:** dropped "lossless"; split capability-resident (accepted lossy) vs exact-output (needs snapshot). §3.
- **R2 (d5452139) isolation:** v1 global toggle; per-agent + apply trial blocked until isolation design. §6.
- **R3 (7e325df7) index/breakpoint vs validator:** no index block, no invented breakpoint; breadcrumbs inside per-result stubs; validator stays strict. §3/§4.
- **R4 (a06953a8) economics:** deleted "~2 turns"; gate on q=S_net/R with full metric contract. §2/§7.1.
- **R5 (6ea4c87f) thrash:** high/low-water + min-net-saving M + low-water L; saturated→no-op; adversarial tests. §4/§8.
- **R6 (c19ef7b0) accounting+quality:** §7.2 upgraded to a design gate (mock-upstream accounting experiment); predefined blinded quality eval. §7.
