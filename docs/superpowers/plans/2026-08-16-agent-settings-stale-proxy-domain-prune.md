# Prune stale proxy loopback domain from agent-settings

owner: 30fa04f4-e047-4241-a9ed-f452529952be (Detoro) · authority: in-loop
priority: LOW hygiene follow-up · origin: Mellow review finding on task
`ctx-proxy-removal` (merged 4b76626)

## What

Files under `~/Library/Application Support/Conclave/agent-settings/*.json`
(5 observed at review time) still carry
`sandbox.network.allowedDomains: ["127.0.0.1"]`, written by the removed
ctx-proxy union block (deleted from `claude_sandbox_settings` in 4b76626).
`write_claude_settings` merges the existing file and nothing strips the key,
so those instances keep a loopback TCP hole for a listener that no longer
exists.

Severity is LOW: loopback-only, and the same profile already grants a UDS
socket hole. Nothing blocks on this.

## Recommended approach (settle before claiming)

Add a strip step in the `claude_sandbox_settings` merge path
(`src-tauri/src/engine/runtime/sandbox_config.rs`): when merging an existing
file, remove `"127.0.0.1"` from `network.allowedDomains` and drop the key if
it empties. Self-healing on every spawn; no one-shot migration machinery.

Rejected alternative: a one-shot startup prune over the agent-settings dir —
more code, runs once, misses files restored from backup later.

Risk ledger: the only known writer of `"127.0.0.1"` was the deleted proxy
block, but a user could in principle have added it by hand for a local
service; stripping it anyway is the accepted tradeoff (they can use any
other loopback spelling, e.g. `localhost`, if they truly need one — note
this in the strip's doc comment).

## Boundary

`src-tauri/src/engine/runtime/sandbox_config.rs` (+ its tests) only.

## Gates

From `src-tauri/`: `cargo check`, `cargo test` — green, recorded via
`conclave task gate`. Add a test: merging an existing file carrying the stale
entry yields settings without it, and a user-added `localhost` entry survives.
