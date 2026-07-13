# Claude Code proxy token-overhead diagnosis

owner: 1b074885-4035-46f0-a449-b77f2be610c8 · authority: in-loop

## Goal

Explain and reproduce the reported condition that otherwise-equivalent Claude Code
sessions consume materially more tokens when Conclave's context proxy is enabled,
then produce an evidence-backed implementation brief. This task is diagnostic only:
it must not change runtime behavior and must not make new paid model calls.

## Known state

- `conclave proxy status` on 2026-07-13 reports `mode=log`, `checkpoint=false`,
  `summaryShadow=false`, and `qualityShadow=false`.
- The proxy report for the last 48 hours contains 3,979 requests and
  1,059,104,771 cache-read tokens, so prompt caching is not globally absent.
- Claude definitions default to proxy-on unless `proxy_enabled=false`; direct and
  proxied historical sessions should therefore be separable from stored definition,
  session, transcript, and proxy-metric records.
- Phase-1 rewrite is shelved; current log mode is expected to forward request bytes
  unchanged and run no paid generation sidecars.

## Reading order

1. `docs/superpowers/specs/2026-07-10-agent-proxy-design.md`
2. `docs/superpowers/plans/2026-07-11-proxy-default-on-claude.md`
3. `src-tauri/src/engine/commands/instance.rs` (`proxy_env` and spawn call site)
4. `src-tauri/src/engine/runtime/ctx_proxy.rs` (`proxy`, `forward`, metrics, and
   post-response sidecar admission)
5. `src-tauri/src/engine/runtime/transcript_context.rs` and the session/agent repos
   that populate `contextTokens`
6. Existing local app database, Claude transcripts, and runtime logs; redact all
   credentials and prompt bodies from notes.

## Required procedure

1. Reproduce from existing evidence only. Build a table of comparable proxied and
   direct Claude sessions, including model, duration/turn count where recoverable,
   input tokens, cache-creation tokens, cache-read tokens, output tokens, request
   count, retry/error count, and Conclave `contextTokens`. State comparability limits.
2. Trace the fail path end-to-end: spawn environment → Claude Code request → proxy
   buffering/forwarding → Anthropic response → transcript usage parsing → Conclave
   context meter. Enumerate every toggle and retry path that changes token accounting.
3. Test, in rank order, these hypotheses and any stronger evidence-derived ones:
   (a) token-meter double counting or wrong usage-field selection; (b) proxy-induced
   retries/duplicate requests; (c) header/query/body mutation that changes cache
   behavior; (d) measurement sidecars producing additional counted traffic;
   (e) the compared sessions differ in model, prompt, work, or lifetime.
4. For each hypothesis, record the cleanest disproof and its result. A surviving
   root cause must explain every row in the experiment ledger.
5. Post `READY diagnosis:` to the task with the root cause, supporting file/line and
   data pointers, exact minimal fix surface, regression-test design, and commands.
   If existing evidence cannot produce a controlled comparison, post `BLOCKED` with
   the single smallest paid experiment required; do not run it.

## Acceptance gates

- No source or runtime behavior changes.
- No new network/model spend.
- No raw prompts, credentials, tokens, or response bodies in task notes.
- Root cause is supported by a repeatable local query/script or the task explicitly
  records why no repro exists.
- Every observation is entered in the task ledger as: experiment, result, ruled
  in/out.

## Boundary

This task may write only this diagnosis plan and task-ledger notes. Runtime files
are read-only until the lead creates a separate implementation task.
