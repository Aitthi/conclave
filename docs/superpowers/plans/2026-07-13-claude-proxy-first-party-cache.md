# Preserve Claude Code first-party cache semantics through ctx-proxy

owner: 1b074885-4035-46f0-a449-b77f2be610c8 · authority: in-loop

## Goal

Make a Claude Code process routed through Conclave's trusted loopback ctx-proxy
behave like a direct `api.anthropic.com` process for prompt-cache and other
first-party eligibility decisions. The fix must eliminate the proxy-only uncached
input bursts without changing request bodies, the proxy rewrite policy, or any UI.

## Evidence and ruling

Task `claude-proxy-token-overhead-diagnosis`, note `074b9dc2`, reproduced the
problem from existing data without paid calls:

- Four proxied Claude transcripts each contained periodic uncached bursts with a
  maximum `input_tokens` of 15,098–15,236 and 6–14 requests above 100 tokens.
- Two direct transcripts stayed at 1–2 uncached input tokens across 157 and 26
  unique requests.
- All 5,459 historical log-mode rows had identical request byte counts in/out;
  `ctx_proxy.rs` forwards `body.to_vec()` in log mode. The forwarding transform is
  not the differentiator.
- `instance.rs::proxy_env` injects `ANTHROPIC_BASE_URL` only. Claude Code 2.1.207
  contains the explicit `_CLAUDE_CODE_ASSUME_FIRST_PARTY_BASE_URL` escape hatch
  and matching non-first-party/prompt-cache diagnostics.

Ruling: the built-in proxy is a trusted reverse proxy whose production upstream is
fixed to `https://api.anthropic.com`. Whenever Conclave injects its loopback
`ANTHROPIC_BASE_URL`, it must atomically inject
`_CLAUDE_CODE_ASSUME_FIRST_PARTY_BASE_URL=1` as well.

Do not inject `ENABLE_PROMPT_CACHING_1H`: that would force a TTL policy and can
change cache-write economics for API-key users. The first-party assertion preserves
Claude Code's direct-path/provider-default policy. Do not switch to `HTTPS_PROXY`:
the current product must inspect `/v1/messages`, while an opaque CONNECT tunnel
cannot. Do not change proxy default-on/off behavior in this task.

## Reading order

1. `docs/superpowers/specs/2026-07-10-agent-proxy-design.md` (D2, D7, D8, risks)
2. `docs/superpowers/plans/2026-07-13-claude-proxy-token-overhead-diagnosis.md`
3. Task note `074b9dc2` on `claude-proxy-token-overhead-diagnosis`
4. `src-tauri/src/engine/commands/instance.rs` (`proxy_env`, spawn selection,
   `extra_env` ordering, and `proxy_env_defaults_on_for_claude_off_for_codex`)
5. `src-tauri/src/engine/runtime/ctx_proxy.rs` (`DEFAULT_UPSTREAM` and log-mode
   forwarding) — read-only, not in this task boundary

## Implementation

### 1. Make the proxy env contract atomic

In `src-tauri/src/engine/commands/instance.rs`, change `proxy_env` so an enabled,
live proxy returns both environment variables as one all-or-none value:

- `ANTHROPIC_BASE_URL=http://127.0.0.1:<active_port>`
- `_CLAUDE_CODE_ASSUME_FIRST_PARTY_BASE_URL=1`

An array or small typed value is preferred over two independent `Option`s so future
call sites cannot inject the route without the semantic assertion. Preserve every
existing selection rule exactly: Claude NULL defaults on, Codex NULL defaults off,
explicit false wins, explicit true wins, and a missing listener returns no proxy env.

Thread the atomic pair through `proxy_port` selection and append/extend it after
custom and secret env values. Both values must win over same-name user env entries,
matching the existing D8 rule for the base URL. Keep credentials untouched.

Update nearby comments to explain why the underscore-prefixed variable is required:
the loopback host is not itself an Anthropic hostname, but the built-in proxy's
production upstream is Anthropic first-party. Add a maintenance warning: if runtime
upstream ever becomes user-configurable, this assertion must be conditional on an
allowlisted first-party upstream.

### 2. Pin the spawn contract with tests

Update `proxy_env_defaults_on_for_claude_off_for_codex` (or split it into focused
tests) to assert the exact two-variable result for every enabled case and no result
for disabled/listener-down cases. The exact key and value `1` are load-bearing.

Add coverage proving the returned set contains no TTL-forcing variable such as
`ENABLE_PROMPT_CACHING_1H`; policy preservation is part of the fix. If the helper
shape alone makes last-write ordering hard to see, add a small pure helper/test for
appending the pair after custom env, rather than testing PTY process launch.

No test may start Claude Code or make a network/model call.

### 3. Amend the durable proxy decision record

In `docs/superpowers/specs/2026-07-10-agent-proxy-design.md`, add D11 recording the
atomic first-party assertion, the 2026-07-13 transcript evidence, and why forced
1-hour TTL / opaque corporate-proxy routing were rejected. Update the architecture
spawn-path bullet and OAuth risk to match the new contract.

## Boundary

- `src-tauri/src/engine/commands/instance.rs`
- `docs/superpowers/specs/2026-07-10-agent-proxy-design.md`
- `docs/superpowers/plans/2026-07-13-claude-proxy-first-party-cache.md`

No `src/` UI files are touched, so the UI Pixel Gate does not apply.

## Acceptance gates

Run and record each gate on the final commit:

1. `cd src-tauri && cargo test proxy_env_defaults_on_for_claude_off_for_codex`
2. `cd src-tauri && cargo test -p conclave`
3. `cd src-tauri && cargo clippy -p conclave --all-targets -- -D warnings`
4. `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
5. `git diff --check`

READY must name the immutable commit SHA, changed files, and gate ids. Live
before/after token verification requires rebuilding/relaunching Conclave and fresh
Claude traffic, so it is a post-merge operational verification rather than a lane
gate: proxied sessions should remain at direct-like 1–2 uncached input tokens during
steady-state turns and must not reproduce periodic >100-token bursts under equivalent
workload.

## Risk ledger

- `_CLAUDE_CODE_ASSUME_FIRST_PARTY_BASE_URL` is an internal Claude Code contract.
  Its installed 2.1.207 binary presence is verified, but a future Claude release can
  rename it. The exact-key unit test prevents our wiring from drifting; the live
  post-relaunch token check detects upstream client drift.
- The assertion would be unsafe for a user-configurable third-party upstream. Today
  production `ctx_proxy` fixes upstream to `https://api.anthropic.com`; preserve that
  invariant or make the assertion conditional in the same change that relaxes it.
- Existing running Claude processes keep their spawn environment. The fix affects
  only agents spawned after the rebuilt Conclave app is relaunched.
