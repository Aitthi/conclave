# Infinity-Turn Checkpoint (ctx-proxy Phase 2) — Design

**Status:** DRAFT for two-principal council review (Detoro chair, Aoki co-principal). In-loop authority granted by human 2026-07-11; human reviews the finished result.
**Supersedes nothing.** Distinct from Phase-1 dedup proxy, which is NO-GO/shelved (plan A9, commit 5ef698d).
**Predecessor evidence:** `docs/superpowers/plans/2026-07-10-agent-proxy-phase1.md` (A9 ruling), blackboard `measure:proxy-025-verdict` / `measure:proxy-025-ruling`.

## 1. Problem & goal

Agents degrade — slower, lower-quality output — once their live context grows past roughly **400–500k tokens**, even on a 1M-window model (fable-5). The harness's own self-compaction fires at ~70% (~700k on 1M), which is **already past** the degradation zone and lands as one disruptive event, not continuous management.

**Goal:** a proxy-managed **"infinity turn"** — the proxy keeps the **effective context** (what is actually sent upstream to the model) inside the high-quality zone at all times by compacting recoverable old tool-output in the background. The agent perceives an effectively unbounded context that never leaves its good operating band; the harness keeps running transparently.

**This is a QUALITY objective, not a cost objective.** Lower cost (smaller cached prefix) is a secondary benefit. The Phase-1 finding stands: "input reduction" is the wrong *primary* objective under prompt caching. Phase-2 is justified by keeping the model in its high-quality zone — a niche the harness does not cover.

## 2. Why Phase-2 can work where Phase-1 failed

Phase-1 elided only *duplicate* tool_results → tiny savings S (3.4% aggregate, 9.5% best) → cache-rebuild cost never amortized (break-even n ≥ (1.15R−1.25S)/(0.1S) → huge for small S). Phase-2 freezes a **large** block once (all recoverable old tool-output beyond the recent tail), so S is large and break-even collapses to ~2 turns — and the frozen block is monotonic and byte-stable, giving the long cache plateau Phase-1 never reached. The economics only close when S is large; **§7 gates the whole feature on empirically confirming S is large enough.**

## 3. Core mechanism (reuses the `ctxopt` crate; changes the policy)

The existing pipeline (`policy → analyze → apply → validate` in `src-tauri/crates/ctxopt/` driven by `src-tauri/src/engine/runtime/ctx_proxy.rs`) is retained. Phase-2 replaces the **dedup policy** with a **checkpoint policy**:

- **Trigger.** When estimated effective input crosses a configurable `ceiling` (default ~450k tokens), fire **one** checkpoint.
- **Freeze region.** Everything **before the recent tail** (tail = last N messages / ~80–100k tokens, kept verbatim so live work is untouched). Within that region, stub **only recoverable** tool_results.
- **Recoverability classifier (safety A).** By tool identity:
  - *Recoverable → elidable:* `Read`, `Grep`, `Glob`, `LS`, search, code-intel — idempotent, re-obtainable on demand.
  - *Non-recoverable → kept verbatim:* `Bash` (side effects), `WebFetch` (content drifts), `Write`/`Edit` (small anyway + confirmation), one-time computations. This bounds S; that is the accepted cost of safety.
- **Memory index (safety C).** At checkpoint, generate one compact frozen index block listing what was stubbed (tool, path/args, turn) so the agent has breadcrumbs. Each stub is actionable: `[ctxopt checkpoint: elided Read <path> @turn N — re-read to restore]`.
- **Escape hatch.** If the agent re-reads a stubbed file, that new Read lands in the live tail (not frozen) → content returns naturally. No special recovery path needed.

The effect is **lossless in capability**: elided content is always re-obtainable, so nothing the agent needs is unrecoverably gone — it is simply not resident until pulled back.

## 4. Cache stability (the load-bearing difference from Phase-1)

- Frozen bytes are **immutable once written** (Phase-1 D5 prefix-stability) and the checkpoint set is **monotonic** (only extends forward).
- **Re-checkpoint only** when effective input crosses `ceiling` again **and** ≥ΔN new messages have accrued since the last checkpoint — producing **few, large, long-lived** freezes instead of Phase-1's per-request elision ramp (which busted the prefix almost every turn).
- A cache breakpoint is placed immediately after the frozen region so the compacted prefix caches; large S → break-even ~2 turns.

## 5. Fail-open & equivalence guard

Any error on the checkpoint path (parse, classify, apply, validate, panic) → forward the **original request untouched**. The proxy must never be able to break an agent (Phase-1 invariant). `validate`: message count unchanged, every `tool_use` retains a matching `tool_result`, content only shrank.

## 6. Configuration ("infinity turn")

Per-agent opt-in, mirroring the existing proxy toggles (in-memory atomics, reset on relaunch unless persistence is added): `proxy checkpoint on|off`, `proxy ceiling <tokens>`, tail size. Default **off**. "Infinity turn" = this feature enabled for an agent.

## 7. VALIDATION GATE — measure before building the apply path

This is mandatory and ordered; do not implement the apply/rewrite path until 7.1 passes.

1. **Log-mode projection first.** Implement the checkpoint policy in **log mode only**: on real traffic, compute what a checkpoint *would* stub, the resulting effective-context-after, and the achieved S — **without** altering the bytes sent upstream. Measure over real fleet lanes whether structural (recoverable-only) S is large enough to pull effective context from >ceiling back into the target band. Recorded like Phase-1 into `proxy_request_metric` (or a sibling table).
2. **Spike the load-bearing assumption.** Determine whether the agent harness accounts context from **API-returned usage** (then a proxy that shrinks the upstream request also stops the harness self-compacting — full "infinity turn") or from **its own message list** (then the harness still self-compacts on its own view; the model-quality win still holds, but "never fills up" is only partial). This changes what we can claim; it does not block the quality win.
3. **Gate.** Only if 7.1 shows sufficient S and 7.2 is understood → flip apply on **one disposable agent**, measure before/after **quality** (the primary objective), effective-context band, and cache health. Fleet rollout is a separate later decision.

**Honest risks:** (a) if recoverable tool-output does *not* dominate large contexts, structural S is insufficient and we must escalate to a hybrid LLM-summary of the oldest block (bigger S, adds cost/latency/quality risk) — 7.1 tells us before we invest; (b) 7.2 is unverified; (c) quality equivalence of a background-compacted context is itself an empirical question the one-agent trial must answer, not assume.

## 8. Testing

Unit (`ctxopt`): checkpoint policy is deterministic (same input → same frozen set), recoverability classifier correctness, recent-tail always preserved, frozen set monotonic across turns; `apply` yields valid JSON with every tool_use/tool_result pair intact; `validate` guard rejects structure changes; **cache-stability test: identical input across turns → byte-identical output**. Plus the empirical S measurement from §7.1 in log mode. Fail-open tests: parse/classify/validate errors → original bytes returned.

## 9. Boundary (anticipated)

`src-tauri/crates/ctxopt/` (new checkpoint policy + recoverability classifier + index generator, alongside existing dedup), `src-tauri/src/engine/runtime/ctx_proxy.rs` (trigger/ceiling wiring, metric emission), `src-tauri/src/engine/commands/proxy.rs` (`checkpoint`/`ceiling` commands), CLI `proxy` argv mapping. Metric table migration if a sibling table is used. Exact paths finalized in the plan.

## Open questions for council (Aoki grill targets)

1. **§7.2 harness accounting** — is there a cheaper way to settle it than a live spike? Does either answer change the design, not just the claim?
2. **§3 recoverability set** — is Bash truly always non-recoverable for *context* purposes (we are not re-running it, only noting the output is gone)? Could large idempotent Bash reads (e.g. `cat`, `ls`) be safely reclassified to raise S?
3. **§4 re-checkpoint policy** — is monotonic-extend + ΔN hysteresis enough to guarantee a long plateau, or is there a traffic shape that still thrashes?
4. **§2 economics** — does the large-S break-even still hold once index-block + kept non-recoverable outputs are counted against S?
