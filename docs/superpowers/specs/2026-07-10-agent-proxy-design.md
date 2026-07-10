# Agent Context Proxy — Design Spec

**Date:** 2026-07-10 · **Owner:** Detoro (lead, 4fb2198c) · **Status:** approved by human ("Build-in ใน project นี้")

## Problem

Every request an agent CLI (Claude Code, Codex) sends to the provider carries the
full conversation history. Measurement (mattpocock's agent-proxy gist) shows the
bulk is redundant: repeated file reads, stale tool output, duplicated command
results. Harness-side compaction fires late (~95%) and is lossy. We want a
built-in Conclave HTTP proxy between spawned agents and `api.anthropic.com` that
trims provably-redundant context per request — extending effective session length
without any loss of agent capability.

## Decisions (with rejected alternatives)

| # | Decision | Rejected because |
|---|----------|------------------|
| D1 | Built into codeup (Conclave), not a standalone repo | Human ruling 2026-07-10. Standalone Node repo rejected: Conclave already owns agent spawning, env injection, sqlite, and context meters — integration wins |
| D2 | In-engine listener (task spawned in `lib.rs` beside the UDS server), loopback `127.0.0.1:$CONCLAVE_PROXY_PORT` (default 18787) | Sidecar process (design_host pattern) rejected: second lifecycle to supervise, no shared `AppState`, no benefit for an I/O task tokio already handles |
| D3 | Phase 1 is deterministic-only, **lossless by construction**: elide only tool_results that are byte-identical to a kept copy, or provably superseded (file later Edited/Written — re-readable from disk). No summarization, no LLM | Rule-of-thumb truncation rejected: violates the "zero intelligence loss" requirement |
| D4 | Rewrite = shrink `tool_result` content in place; never add/remove/reorder messages or blocks | Message deletion rejected: breaks `tool_use`/`tool_result` pairing and cache breakpoints |
| D5 | Cache-aware **hysteresis**: elision decisions are FROZEN per conversation and re-evaluated only when est. tokens cross high-water (70% of window) and grow ≥10% since last eval. Frozen set is monotone (grows, never shrinks) | Per-request re-analysis rejected: every new elision invalidates the provider cache prefix from that block onward; continuous rewriting = cache miss every turn, costing more than it saves |
| D6 | Operate on `serde_json::Value` with surgical edits | Full typed request model rejected: unknown fields would be silently dropped on re-serialize — corruption risk as the API evolves |
| D7 | Fail-open everywhere: parse error, validator rejection, upstream hiccup in the rewrite path → forward the ORIGINAL body untouched. Proxy may never break an agent | Fail-closed rejected: an optimizer must never be an outage source |
| D8 | Double opt-in rollout: global mode defaults to `log` (measure only; flip with `conclave proxy mode rewrite`), and each agent needs `proxy_enabled` (new nullable column, default off) before its spawn env gets `ANTHROPIC_BASE_URL` | Default-on rejected for v1: no fleet-wide blast radius before A/B evidence |
| D9 | v1 config lives on `ProxyRuntime` atomics (CLI-adjustable, reset on restart) + the per-agent column. No per-workspace bb keys yet | bb `config:*` keys rejected for v1: proxy is app-global; a request can't be attributed to a workspace cheaply. Revisit in Phase 2 |
| D10 | Conversation identity = longest prefix match over per-message hashes (client resends full history verbatim; its bytes are stable). In-memory LRU ledger, cap 64 conversations | Requiring a session header rejected: harness sends none we control |

## Architecture (Phase 1)

```
claude/codex CLI ──ANTHROPIC_BASE_URL──► engine listener (axum, loopback)
                                          │  POST /v1/messages only; everything
                                          │  else (incl. count_tokens) passthrough
                                          │
                                          ├─ ctxopt crate (pure, sync):
                                          │    ledger → policy → analyze → apply → validate
                                          │    (fail-open on any Err)
                                          ├─ reqwest streaming leg → api.anthropic.com
                                          │    response streamed back verbatim; SSE tee
                                          │    reads usage for metrics only
                                          └─ sqlite: proxy_request_metric (migration 0019)
```

- New workspace member crate `src-tauri/crates/ctxopt` — pure sync logic, serde_json
  only (mirrors `codeintel` purity). All intelligence-preserving guarantees live here,
  unit-tested.
- Engine module `src-tauri/src/engine/runtime/ctx_proxy.rs` hosts the axum service;
  `Arc<ProxyRuntime>` on `AppState` (both ctors, like `code_cache`).
- Spawn path: `instance.rs` env block (755-785) pushes `ANTHROPIC_BASE_URL` when the
  agent's `proxy_enabled` is true AND the listener is up. `sandbox_config.rs` network
  allowlists gain the loopback port (both Claude settings and Codex overrides).
- Auth headers are forwarded untouched and never logged or persisted. Request/response
  bodies are never persisted — metrics rows carry counts only.

### Phase 1 transforms (all provably lossless)

1. **Identical-read dedup** — N Reads of one `file_path` with byte-identical results:
   keep the LAST, stub earlier ones.
2. **Superseded-read stubbing** — a Read whose file is later touched by
   Edit/Write/MultiEdit/NotebookEdit: content is provably outdated and re-readable;
   stub with a pointer.
3. **Exact-duplicate tool-result dedup** — same tool, same input, byte-identical
   output: keep the LAST.

Guards: never elide within the most recent 10 messages; never elide results
< 600 bytes; never elide non-text content; `cache_control` on an elided block is
preserved. Validator proves post-state: identical message/block structure, all
`tool_use` untouched, tool_result id set unchanged, non-elided results byte-equal.

### Phase 1 acceptance

- Session past first threshold: input tokens per request reduced ≥25% on a real
  agent coding session (measured via `conclave proxy report`).
- Cache-hit tokens between rewrite rounds no worse than baseline (SSE tee metrics).
- 2–3 real bug-fix tasks run through `mode rewrite` complete equivalently to
  passthrough. This is the "no intelligence lost" gate.

## Phase 2 (deferred — sketch, do not build yet)

- **M3 manager agent:** after each turn, a cheap model (Haiku via
  `runtime/provider.rs::stream_anthropic`) reads only the history suffix + ledger and
  emits an *eviction plan* (per-block `drop`/`summarize-to`/`pin` commands). Code
  applies it through the same Phase-1 validator; the LLM never writes context bytes.
- **M4 side store:** evicted originals persist in sqlite keyed by recall id; stubs
  carry the id. Transcript becomes effectively unbounded.
- **M5 (optional) `context_recall` tool:** proxy injects a tool schema and services
  its own tool_use round-trips (mini agent loop) — true page-in. Gated on M4 evidence.
- Per-workspace bb config, workspace attribution, and context-meter integration
  revisit here.

## Risks

- `ANTHROPIC_BASE_URL` + OAuth: Claude Code sends its bearer token to whatever base
  URL is set; the gist proves the redirect works. Verify once on a live agent before
  flipping any agent's `proxy_enabled` (Task 11 gate).
- Engine restart while an agent runs → agent's base URL dead. Mitigated: opt-in only,
  and the listener outlives sessions (engine process = GUI app lifetime).
- Harness self-compaction shrinks history → ledger prefix mismatch → treated as a new
  conversation (correct, by design).
- SSE tee must not buffer the stream — TTFT must be unchanged (forward-first design).

## Pointers

Plan (Phase 1): `docs/superpowers/plans/2026-07-10-agent-proxy-phase1.md` ·
Wiring survey evidence: task ledger `agent-proxy-*` tasks · Inspiration:
https://gist.github.com/mattpocock/5b3d76ea21f5f698aefded47a9cea3b1
