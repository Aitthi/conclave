# M1 count_tokens contract + closed-prefix fix

owner: 1b074885-4035-46f0-a449-b77f2be610c8 · authority: in-loop

## Goal

Restore interpretable M1 checkpoint samples in one rebuild by fixing the proven
malformed `c` prefix, matching Anthropic's stable/beta count-token routing
contract, and making future `a`/`b`/`c` failures distinguishable without
persisting response bodies or error messages.

Authoritative ruling: task `infinity-turn-checkpoint`, event `f4a78a94`.
Root cause credit: Aoki challenge `e440a1a2`; Mellow DIAG `a7bc616f`.

## Global constraints

- M1 remains log-only. Never alter bytes forwarded to `/v1/messages`.
- Never persist or log credentials, request bodies, response bodies, or
  upstream `error.message`. The existing status + allowlisted `error.type`
  invariant remains mandatory.
- Count sampling remains off-path, bounded, no-redirect, timeout-limited, and
  tied to the immutable captured upstream.
- No raw-body debug capture. Stage labels are content-free literals only.
- Preserve stable count behavior for requests with no `anthropic-beta` header.
- Do not touch the user's untracked `design/screens/welcome.tsx`.

## Decisions

### 1. Structurally closed `c` prefix

`earliest_changed_msg_index` names the user message containing the earliest
candidate `tool_result`. A prefix ending at `messages[..result_index]` is
invalid because it retains the paired assistant `tool_use` and drops its
required result.

Change `prefix_messages` so it derives the `tool_result` IDs at and after the
candidate boundary, finds their paired `tool_use` message(s), and moves the cut
before the earliest assistant message needed to make the retained prefix
globally closed. Do not implement this as a plain `idx - 1`: one assistant turn
may contain parallel `tool_use` blocks whose results span multiple following
user messages. After choosing the cut, validate the entire retained prefix has
zero unmatched `tool_use` IDs and zero retained `tool_result` IDs without their
use. If it is not closed, continue backing up to the preceding implicated
assistant turn; never emit a malformed C body. Excluding the whole affected
tool cycle is intentionally conservative: `R = a - c` remains greater than or
equal to the true changed suffix, as required by spec §7.1. No off-by-one shift
onto a multi-block or multi-message result sequence is allowed.

The helper must have a deterministic safe fallback for malformed input and
must never panic. The normal accepted-request path must produce a non-empty,
schema-valid prefix for the existing long-context fixtures.

### 2. Stable vs beta count route

Mirror the current Anthropic generated SDK contract:

- `CountCredential.anthropic_beta == None`: POST
  `/v1/messages/count_tokens` and do not invent a beta header.
- `Some(original_betas)`: POST `/v1/messages/count_tokens?beta=true`, preserve
  the captured beta values, and idempotently ensure
  `token-counting-2024-11-01` is present exactly once.

Do not split/reorder away caller beta values. Header construction must remain
bounded to the existing allowlist and mark credential-bearing values sensitive.

Primary contract source:
`anthropics/anthropic-sdk-typescript/src/resources/beta/messages/messages.ts`,
generated `Messages.countTokens`.

### 3. Safe failure-stage telemetry

At the `sample_checkpoint` call chain, wrap failures with exactly one
content-free stage label: `a`, `b`, or `c`. Keep the existing HTTP status and
allowlisted error type after the label. Do not broaden persisted content.

## Files

- Modify `src-tauri/src/engine/runtime/count_tokens.rs`
- Modify `src-tauri/src/engine/runtime/ctx_proxy.rs`
- Modify `docs/superpowers/specs/2026-07-11-infinity-turn-checkpoint-design.md`
- This plan: `docs/superpowers/plans/2026-07-12-proxy-m1-counttokens-contract-fix.md`

## Test-first implementation

1. Add a schema-aware fake count upstream which rejects an assistant
   `tool_use` not immediately closed by the following user `tool_result`.
   Reproduce the current `tool_use@0/tool_result@1` prefix failure first.
2. Add unit coverage for a multi-tool assistant message whose parallel results
   span both a multi-block user message and more than one following user
   message. Assert the prefix cuts before the whole implicated tool cycle,
   retains earlier closed turns, has zero unmatched use/result IDs, and is
   accepted by the schema-aware fake. This test is the guard against a naive
   `idx - 1` implementation (Tiësto review finding, 2026-07-12).
3. Add route/header tests:
   - no beta => stable URL, no invented token-counting beta;
   - beta => `?beta=true`, original betas preserved, token-counting beta added;
   - already-present token-counting beta => exactly one copy.
4. Add stage tests that force each call to fail independently and assert the
   stored diagnostic identifies `a`, `b`, or `c` while containing no request,
   header, raw-body, `error.message`, or leak marker.
5. Add an end-to-end sampled fixture where all `a`/`b`/`c` requests pass the
   schema-aware fake and a metric row is persisted.
6. Amend spec §7.1 with the closed-boundary rule and conditional beta-route
   contract, citing ruling `f4a78a94`.

## Gates

From `src-tauri/`:

```sh
$(rustup which rustfmt) --check --edition 2021 src/engine/runtime/count_tokens.rs src/engine/runtime/ctx_proxy.rs
cargo test -p ctxopt -p conclave
cargo clippy -p ctxopt -p conclave --all-targets -- -D warnings
```

If repository-wide clippy has pre-existing out-of-boundary warnings, record
the exact warnings and additionally run clippy with warnings allowed to prove
there are no new errors. Workspace `cargo fmt --check` is intentionally not a
gate for this lane: on the base SHA it reports pre-existing diffs across clean
out-of-boundary files and can only be satisfied by violating the immutable
boundary. Never rewrite those files (Dabin challenge `5110f07a`, accepted).
No UI files are touched, so the UI Pixel Gate does not apply.

## READY note contract

Report the commit SHA, changed files, each test/gate result, the recorded gate
event ID, and explicit red→green proof that the original
`tool_use@0/tool_result@1` repro fails before the fix and passes after it. Also
provide explicit proof
for: closed C boundary; stable/beta route split; token-counting idempotence;
safe stage labels; schema-aware `a`/`b`/`c` success; invariant leak tests. Move
the task to `review`. Tiësto is the named reviewer; Detoro owns integration.
