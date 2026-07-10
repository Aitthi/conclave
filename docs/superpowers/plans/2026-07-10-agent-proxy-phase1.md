# Agent Context Proxy — Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

owner: 4fb2198c-e0d9-4e4b-af9e-d4e72542bace (Detoro) · authority: in-loop

**Goal:** A built-in loopback HTTP proxy in the Conclave engine that deterministically elides provably-redundant tool_results from `/v1/messages` requests before forwarding to `api.anthropic.com` — losslessly, cache-aware, fail-open.

**Architecture:** Pure-logic crate `src-tauri/crates/ctxopt` (ledger → policy → analyze → apply → validate, serde_json only) + engine axum listener `src-tauri/src/engine/runtime/ctx_proxy.rs` spawned in `lib.rs` beside the UDS server + spawn-path env injection. Spec (decisions D1–D10, Phase 2 sketch): `docs/superpowers/specs/2026-07-10-agent-proxy-design.md`.

**Tech Stack:** Rust, tokio 1 (present), axum 0.8 (NEW dep), reqwest 0.12 rustls+stream (present), serde_json, sqlx/sqlite via existing db.rs pattern.

## Global Constraints (every task inherits these)

- **Cargo.lock ships with every dep-changing commit** (ruling 853e8270) — a commit that touches any `Cargo.toml` commits `src-tauri/Cargo.lock` in the same commit.
- **Fail-open:** any error in the rewrite path (parse, analyze, validate, panic) forwards the ORIGINAL request untouched. The proxy must never be able to break an agent. Every task's error paths follow this.
- **Never log or persist auth headers** (`authorization`, `x-api-key`, `api-key`) **or request/response bodies.** Metrics rows carry counts only.
- **Crate purity:** `crates/ctxopt` depends on `serde_json` ONLY. No tokio, no axum, no engine types. All async/IO lives in the engine.
- **Only tool_result `content` may change.** Never add/remove/reorder messages or blocks; never touch `tool_use`, `system`, `tools`, or any other request field. The validator (Task 5) enforces this; every transform routes through it.
- Commit style: conventional commits (`feat(ctxopt): …`, `feat(engine): …`). In the shared checkout prefer `conclave stage commit`; lanes run in their own worktrees (`conclave lane start`).
- Fresh worktrees need `pnpm install` only for UI work — none here; but `cargo` gates run from `src-tauri/` (the worktree root has no Cargo.toml).
- Gates before READY: `cargo test -p ctxopt` (lane A), full `cargo test` + `cargo build` (lanes B, C), recorded via `conclave task gate <ws> <slug> -- <cmd>`.
- No UI files are touched in Phase 1 → the UI pixel gate does not apply.

## Constants (single source: `crates/ctxopt/src/lib.rs`)

```rust
pub const HIGH_WATER: f32 = 0.70;      // evaluate elisions above 70% of window
pub const RE_EVAL_GROWTH: f32 = 1.10;  // re-evaluate only after +10% growth
pub const RECENT_KEEP: usize = 10;     // never elide within the last 10 messages
pub const MIN_ELIDE_BYTES: usize = 600; // never elide small results
pub const LEDGER_CAP: usize = 64;      // LRU conversation cap
```

Engine constants (Task 7): default port `18787`, env override `CONCLAVE_PROXY_PORT`; modes `off=0, log=1, rewrite=2`, default `log`.

---

## Lane A — crate `ctxopt` (Tasks 1–6) · boundary: `src-tauri/crates/ctxopt`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`

### Task 1: Crate scaffold + estimation module

**Files:**
- Create: `src-tauri/crates/ctxopt/Cargo.toml`, `src-tauri/crates/ctxopt/src/lib.rs`, `src-tauri/crates/ctxopt/src/estimate.rs`
- Modify: `src-tauri/Cargo.toml` (workspace `members` gains `"crates/ctxopt"`; `[dependencies] ctxopt = { path = "crates/ctxopt" }`)

**Interfaces — Produces:** `ctxopt::estimate::{est_tokens(bytes: usize) -> usize, context_window_for_model(model: &str) -> usize}`, the five `pub const`s above re-exported from `lib.rs`.

- [ ] **Step 1: failing test** in `estimate.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn est_is_bytes_over_four() { assert_eq!(est_tokens(4000), 1000); }
    #[test]
    fn window_defaults_and_1m() {
        assert_eq!(context_window_for_model("claude-3-5-haiku-20241022"), 200_000);
        assert_eq!(context_window_for_model("claude-sonnet-5[1m]"), 1_000_000);
        assert_eq!(context_window_for_model("claude-fable-5"), 1_000_000);
    }
}
```

- [ ] **Step 2:** `cargo test -p ctxopt` from `src-tauri/` → FAIL (crate/module missing)
- [ ] **Step 3: implement.** Crate manifest: `[package] name = "ctxopt" edition = "2021"` + `serde_json = "1"`. `lib.rs`: the 5 consts + `pub mod estimate;`. `estimate.rs`:

```rust
pub fn est_tokens(bytes: usize) -> usize { bytes / 4 }

/// Mirror of engine transcript_context::claude_model_context_window — kept
/// in sync by hand; crate purity forbids depending on the engine.
pub fn context_window_for_model(model: &str) -> usize {
    let m = model.to_ascii_lowercase();
    if m.contains("[1m]") || m.starts_with("claude-fable") || m.starts_with("claude-mythos") {
        return 1_000_000;
    }
    200_000
}
```

(Before committing, read `src-tauri/src/engine/runtime/transcript_context.rs:231` `claude_model_context_window` and make the two functions agree for every model id it names — that function is the source of truth.)

- [ ] **Step 4:** `cargo test -p ctxopt` → PASS
- [ ] **Step 5:** commit `feat(ctxopt): crate scaffold + token/window estimation` (include `Cargo.toml` + `Cargo.lock`)

### Task 2: Request indexer over `serde_json::Value`

**Files:** Create `src-tauri/crates/ctxopt/src/request.rs`; modify `lib.rs` (`pub mod request;`)

**Interfaces — Produces:**

```rust
pub struct ToolCall { pub id: String, pub name: String, pub input: serde_json::Value, pub msg_idx: usize }
pub struct ToolResultRef { pub tool_use_id: String, pub msg_idx: usize, pub block_idx: usize, pub text: Option<String> }
pub fn index_tools(messages: &Value) -> (Vec<ToolCall>, Vec<ToolResultRef>);
pub fn result_text(block: &Value) -> Option<String>; // Some iff content is a string or all-text blocks
```

- [ ] **Step 1: failing test** (fixture built with `serde_json::json!` — one assistant msg with a `tool_use` (name `Read`, input `{"file_path":"/a.rs"}`, id `tu_1`), one user msg with the paired `tool_result` whose content is `[{"type":"text","text":"fn main(){}"}]`, plus a `tool_result` with an image block):

```rust
#[test]
fn indexes_pairs_and_extracts_text() {
    let msgs = fixture();
    let (calls, results) = index_tools(&msgs);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "Read");
    assert_eq!(results[0].tool_use_id, "tu_1");
    assert_eq!(results[0].text.as_deref(), Some("fn main(){}"));
    assert_eq!(results[1].text, None); // image content → not elidable
}
```

- [ ] **Step 2:** run → FAIL. **Step 3: implement:**

```rust
use serde_json::Value;

pub fn index_tools(messages: &Value) -> (Vec<ToolCall>, Vec<ToolResultRef>) {
    let (mut calls, mut results) = (Vec::new(), Vec::new());
    let Some(msgs) = messages.as_array() else { return (calls, results) };
    for (mi, msg) in msgs.iter().enumerate() {
        let Some(blocks) = msg.get("content").and_then(Value::as_array) else { continue };
        for (bi, b) in blocks.iter().enumerate() {
            match b.get("type").and_then(Value::as_str) {
                Some("tool_use") => {
                    let (Some(id), Some(name)) = (b.get("id").and_then(Value::as_str), b.get("name").and_then(Value::as_str)) else { continue };
                    calls.push(ToolCall { id: id.into(), name: name.into(), input: b.get("input").cloned().unwrap_or(Value::Null), msg_idx: mi });
                }
                Some("tool_result") => {
                    let Some(id) = b.get("tool_use_id").and_then(Value::as_str) else { continue };
                    results.push(ToolResultRef { tool_use_id: id.into(), msg_idx: mi, block_idx: bi, text: result_text(b) });
                }
                _ => {}
            }
        }
    }
    (calls, results)
}

pub fn result_text(block: &Value) -> Option<String> {
    match block.get("content") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(parts)) => {
            let mut out = String::new();
            for p in parts {
                if p.get("type").and_then(Value::as_str) != Some("text") { return None; }
                out.push_str(p.get("text")?.as_str()?);
            }
            Some(out)
        }
        _ => None,
    }
}
```

- [ ] **Step 4:** PASS · **Step 5:** commit `feat(ctxopt): tool_use/tool_result indexer`

### Task 3: Analysis — identical-read + exact-duplicate dedup

**Files:** Create `src-tauri/crates/ctxopt/src/analyze.rs`; modify `lib.rs`

**Interfaces — Produces:**

```rust
pub enum ElisionReason {
    IdenticalRead { path: String, kept_msg: usize },
    SupersededRead { path: String, edit_msg: usize },   // implemented in Task 4
    DuplicateResult { tool: String, kept_msg: usize },
}
pub struct Elision { pub tool_use_id: String, pub stub: String, pub reason: ElisionReason }
pub fn analyze(messages: &Value) -> Vec<Elision>;
```

Rules (all in this task except superseded): group `Read` results by `input.file_path`; among byte-identical texts keep the LAST (highest `msg_idx`), elide earlier ones. For non-Read tools: key = `(name, input.to_string(), text)`, keep the LAST. Guards: skip results with `text == None`, `text.len() < MIN_ELIDE_BYTES`, or `msg_idx >= total_msgs - RECENT_KEEP`. Stub texts exactly:

```rust
fn stub_identical(path: &str, kept: usize) -> String {
    format!("[ctxopt] elided: byte-identical to the later Read of {path} (message #{kept}); nothing lost.")
}
fn stub_duplicate(tool: &str, kept: usize) -> String {
    format!("[ctxopt] elided: identical output of the same {tool} call kept at message #{kept}.")
}
```

- [ ] **Step 1: failing tests:** (a) two identical Reads of `/a.rs` (each >600 bytes, both older than RECENT_KEEP — build fixture with 12 trailing filler messages) → exactly one Elision, targeting the EARLIER `tool_use_id`, reason `IdenticalRead{kept_msg == later}`; (b) identical Reads but different bytes → no elision; (c) identical result inside the last 10 messages → no elision; (d) two identical `Bash` results (same input) → one `DuplicateResult` elision; (e) 500-byte identical reads → no elision.
- [ ] **Step 2:** FAIL · **Step 3:** implement with a `HashMap<String, Vec<(&ToolResultRef, &ToolCall)>>` per rule, using `index_tools`. Dedup output by `tool_use_id` (a result gets at most one elision; first rule wins). **Step 4:** PASS · **Step 5:** commit `feat(ctxopt): identical-read and duplicate-result analysis`

### Task 4: Analysis — superseded reads

**Files:** Modify `src-tauri/crates/ctxopt/src/analyze.rs`

**Interfaces — Consumes:** Task 3's `analyze` internals. **Produces:** same `analyze` signature, now also emitting `SupersededRead`.

Rule: a `Read` result of path `p` at `msg_idx i` is elided when any tool_use with name in `["Edit","Write","MultiEdit","NotebookEdit"]` and `input.file_path == p` (or `input.notebook_path == p` for NotebookEdit) exists at `msg_idx j > i`. Same guards as Task 3. Precedence: if a result already has an `IdenticalRead` elision, keep that one (both are lossless; identical points at surviving bytes). Stub:

```rust
fn stub_superseded(path: &str, edit_msg: usize) -> String {
    format!("[ctxopt] elided: this Read of {path} was superseded by a later Edit/Write (message #{edit_msg}); re-read the file for current content.")
}
```

- [ ] **Step 1: failing tests:** (a) Read `/a.rs` then Edit `/a.rs` later, both old → `SupersededRead` elision on the read; (b) Edit BEFORE the read → no elision; (c) Edit on a different path → no elision; (d) read also duplicated-identical → single elision, reason `IdenticalRead`.
- [ ] **Steps 2–4:** FAIL → implement → PASS · **Step 5:** commit `feat(ctxopt): superseded-read analysis`

### Task 5: Apply + structural validator

**Files:** Create `src-tauri/crates/ctxopt/src/apply.rs`, `src-tauri/crates/ctxopt/src/validate.rs`; modify `lib.rs`

**Interfaces — Produces:**

```rust
pub fn apply(messages: &mut Value, elisions: &[Elision]) -> usize; // bytes saved
pub fn validate(before: &Value, after: &Value, elisions: &[Elision]) -> Result<(), String>;
```

`apply`: for each tool_result block whose `tool_use_id` is elided, replace ONLY its `content` with `json!([{ "type": "text", "text": e.stub }])`. All sibling keys (`cache_control`, `is_error`, anything unknown) stay untouched.

`validate` checks, in order, returning `Err(reason)` on the first failure:
1. both are arrays of equal length; per-message `role` equal;
2. per-message block count equal and block `type` sequence equal;
3. every non-`tool_result` block byte-equal (`Value` equality);
4. every tool_result NOT in the elision set byte-equal;
5. every elided tool_result: `content` serialized length strictly smaller, and every key other than `content` equal (this covers `cache_control`).

- [ ] **Step 1: failing tests:** (a) apply one elision → content replaced, `cache_control` still present, bytes saved > 0, `validate` returns `Ok`; (b) hand-corrupt `after` (drop a message) → `Err` containing `"length"`; (c) hand-mutate a tool_use in `after` → `Err`; (d) grow an elided content → `Err`; (e) empty elision list → `apply` returns 0 and `validate(before, before, &[])` is `Ok`.
- [ ] **Steps 2–4:** FAIL → implement → PASS · **Step 5:** commit `feat(ctxopt): elision apply + structural validator`

### Task 6: Conversation ledger + hysteresis policy

**Files:** Create `src-tauri/crates/ctxopt/src/ledger.rs`, `src-tauri/crates/ctxopt/src/policy.rs`; modify `lib.rs`

**Interfaces — Produces:**

```rust
// ledger.rs
pub struct ConvState { pub msg_hashes: Vec<u64>, pub frozen: Vec<Elision>,
                       pub last_eval_est: usize, pub last_input_tokens: Option<u64> }
pub struct Ledger { /* Vec<ConvState> LRU, cap */ }
impl Ledger {
    pub fn new(cap: usize) -> Self;
    /// Match by (first-message hash equal AND shorter list is a prefix of longer);
    /// on match update stored hashes to incoming and move to LRU back; else insert
    /// (evicting front past cap). Returns index for conv_mut.
    pub fn observe(&mut self, messages: &Value) -> usize;
    pub fn conv_mut(&mut self, idx: usize) -> &mut ConvState;
}
pub fn hash_messages(messages: &Value) -> Vec<u64>; // DefaultHasher over each message's .to_string()

// policy.rs
pub enum Decision { Passthrough, ApplyFrozen, Reevaluate }
pub fn decide(est_tokens: usize, window: usize, conv: &ConvState) -> Decision;
```

`decide`: below `HIGH_WATER * window` → `Passthrough` if `frozen` empty else `ApplyFrozen` (frozen decisions keep applying forever — prefix stability). At/above high water → `Reevaluate` iff `last_eval_est == 0` or `est >= last_eval_est * RE_EVAL_GROWTH`, else `ApplyFrozen`. Caller contract (engine): on `Reevaluate`, run `analyze`, EXTEND `frozen` with new elisions (monotone — never remove), set `last_eval_est = est_tokens`.

- [ ] **Step 1: failing tests:** (a) same conversation grown by 2 messages → `observe` returns the same index, hashes updated; (b) different first message → new index; (c) cap 2 + third conversation → oldest evicted; (d) `decide(69k, 100k*… )` below water, frozen empty → `Passthrough`; frozen non-empty → `ApplyFrozen`; (e) first crossing of 70% → `Reevaluate`; (f) 5% growth after eval → `ApplyFrozen`; 12% growth → `Reevaluate`.
- [ ] **Steps 2–4:** FAIL → implement → PASS · **Step 5:** commit `feat(ctxopt): conversation ledger + hysteresis policy`. Lane gate: `conclave task gate <ws> agent-proxy-ctxopt -- cargo test -p ctxopt`.

---

## Lane B — engine service (Tasks 7–10) · boundary: `src-tauri/src/engine/runtime/ctx_proxy.rs`, `src-tauri/src/engine/state.rs`, `src-tauri/src/lib.rs`, `src-tauri/src/engine/router.rs`, `src-tauri/src/engine/commands/proxy.rs`, `src-tauri/src/engine/commands/cli.rs`, `src-tauri/src/bin/conclave-cli.rs`, `src-tauri/src/engine/db.rs`, `src-tauri/src/engine/migrations/0019_proxy_metric.sql`, `src-tauri/src/engine/repo/proxy_metric.rs`, `src-tauri/src/engine/repo/mod.rs`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` · **starts after lane A merges**

### Task 7: Listener + streaming passthrough

**Files:**
- Create: `src-tauri/src/engine/runtime/ctx_proxy.rs`
- Modify: `src-tauri/Cargo.toml` (`axum = "0.8"`; commit Cargo.lock), `src-tauri/src/engine/runtime/mod.rs` (`pub mod ctx_proxy;`), `src-tauri/src/engine/state.rs` (`pub ctx_proxy: std::sync::Arc<ProxyRuntime>` on `AppState`, BOTH ctors — `new()` ~:100 and `for_tests()` ~:216, mirroring `code_cache` at `state.rs:63`), `src-tauri/src/lib.rs` (spawn beside the UDS server, `lib.rs:52` idiom)

**Interfaces — Produces:**

```rust
pub struct ProxyRuntime {
    pub port: u16,                                  // CONCLAVE_PROXY_PORT or 18787
    pub mode: std::sync::atomic::AtomicU8,          // 0 off / 1 log (default) / 2 rewrite
    pub upstream: std::sync::RwLock<String>,        // "https://api.anthropic.com"; tests override
    pub ledger: std::sync::Mutex<ctxopt::ledger::Ledger>,
    pub active: std::sync::atomic::AtomicBool,      // true once bound
}
impl ProxyRuntime { pub fn active_port(&self) -> Option<u16>; }
pub async fn serve(state: std::sync::Arc<AppState>);   // never returns; sets active=true after bind
```

Behavior this task: EVERY request (all methods/paths — including `/v1/messages/count_tokens`) is forwarded verbatim: same method/path/query, all headers except `host`/`content-length`/`connection`/hop-by-hop (recomputed by reqwest), body passed through, response streamed back chunk-by-chunk via `axum::body::Body::from_stream(resp.bytes_stream())` with upstream status + headers (minus hop-by-hop). No buffering of the response. Auth headers forwarded, never logged. Upstream connection error → `502` with a short plain-text body.

- [ ] **Step 1: failing test** (in-module `#[tokio::test]`): bind a fake upstream axum server on an ephemeral port that (a) echoes method+path+body length in a JSON body for `POST /v1/messages/count_tokens`, and (b) for `POST /v1/messages` returns `text/event-stream` with 3 chunks written with delays. Build `AppState::for_tests()`, point `upstream` at the fake, run `serve` on an ephemeral port (make port pickable: read from `ProxyRuntime.port`; test constructs the runtime directly), then assert with reqwest through the proxy: bytes identical, status preserved, SSE arrives as multiple chunks.
- [ ] **Steps 2–4:** FAIL → implement → PASS (`cargo test ctx_proxy` from `src-tauri/`)
- [ ] **Step 5:** commit `feat(engine): loopback context-proxy listener with streaming passthrough`

### Task 8: Rewrite pipeline + SSE usage tee

**Files:** Modify `src-tauri/src/engine/runtime/ctx_proxy.rs`

**Interfaces — Consumes:** all lane-A functions. **Produces:** internal `fn rewrite_body(rt: &ProxyRuntime, body: &[u8]) -> RewriteOutcome { body: Vec<u8>, elisions: usize, bytes_saved: usize, model: String, decision: &'static str }` and `struct UsageTotals { input_tokens, cache_read, cache_creation, output_tokens: Option<u64> }` extracted by the tee.

Pipeline for `POST /v1/messages` (exact path only) when mode != off:
1. Parse body → `Value`; on error: forward original (fail-open), `decision="parse-error"`.
2. `ledger.observe(&v["messages"])` → conv; `est = last_input_tokens.unwrap_or(est_tokens(body.len()))`; `window = context_window_for_model(model)`.
3. `policy::decide` → on `Reevaluate`: `analyze`, extend `frozen`, stamp `last_eval_est`.
4. Filter `frozen` to elisions whose `tool_use_id` still exists in this request; `apply` on a CLONE; `validate(original, rewritten, &applied)`; `Err` → forward original (log the reason via `log::warn!`, no body content).
5. mode `log` → forward ORIGINAL but still compute + report what WOULD be saved; mode `rewrite` → forward rewritten bytes.
6. Response tee: wrap the upstream bytes stream; accumulate line fragments; on `data: ` JSON lines read `message_start.message.usage` (input/cache_read/cache_creation) and final `message_delta.usage.output_tokens`; when the stream ends, write `conv.last_input_tokens = Some(input + cache_read + cache_creation)` back through the ledger and hand `UsageTotals` to the metrics hook (Task 9 fills it in; this task leaves a `fn on_request_complete(...)` stub that only updates the ledger). The tee must forward each chunk BEFORE parsing it.

- [ ] **Step 1: failing tests:** (a) unit-test `rewrite_body` with a >70%-of-window fixture containing one identical-read pair → `decision="rewrite"`, `elisions==1`, output smaller, output parses as JSON with the stub in place; (b) small request → `decision="passthrough"`, byte-identical output; (c) garbage body → original bytes back; (d) tee unit test over a canned SSE byte-stream split at awkward boundaries → correct `UsageTotals`.
- [ ] **Steps 2–4:** FAIL → implement → PASS · **Step 5:** commit `feat(engine): ctx proxy rewrite pipeline + SSE usage tee`

### Task 9: Metrics — migration 0019 + repo + wiring

**Files:**
- Create: `src-tauri/src/engine/migrations/0019_proxy_metric.sql`, `src-tauri/src/engine/repo/proxy_metric.rs`
- Modify: `src-tauri/src/engine/db.rs` (add `if version < 19 { … }` gate after the `< 18` block, same transaction pattern `db.rs:71-96`), `src-tauri/src/engine/repo/mod.rs`, `ctx_proxy.rs` (`on_request_complete` inserts a row via `state.pool`)

**Interfaces — Produces:**

```sql
CREATE TABLE IF NOT EXISTS proxy_request_metric (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  created_at TEXT NOT NULL,
  model TEXT NOT NULL,
  mode TEXT NOT NULL,              -- 'log' | 'rewrite'
  decision TEXT NOT NULL,          -- 'passthrough' | 'apply-frozen' | 'reevaluate' | 'parse-error' | 'validate-reject'
  request_bytes_in INTEGER NOT NULL,
  request_bytes_out INTEGER NOT NULL,
  elisions INTEGER NOT NULL,
  bytes_saved INTEGER NOT NULL,
  input_tokens INTEGER, cache_read_tokens INTEGER,
  cache_creation_tokens INTEGER, output_tokens INTEGER
);
```

```rust
// repo/proxy_metric.rs — model on repo/blackboard.rs (FromRow + camelCase Serialize,
// raw sqlx INSERT like blackboard::set, chain-builder for the single-table SELECT)
pub struct MetricInsert { /* one field per column above except id */ }
pub async fn insert(pool: &SqlitePool, m: MetricInsert) -> Result<(), Error>;
#[derive(sqlx::FromRow, serde::Serialize)] #[serde(rename_all = "camelCase")]
pub struct ProxyReport { pub requests: i64, pub rewritten: i64, pub bytes_saved: i64,
    pub input_tokens: i64, pub cache_read_tokens: i64 }
pub async fn report(pool: &SqlitePool, since_hours: i64) -> Result<ProxyReport, Error>;
```

- [ ] **Step 1: failing test:** repo test against an in-memory pool (mimic an existing `repo::blackboard` test): insert 2 rows, `report(pool, 24)` aggregates both; row with NULL usage aggregates as 0.
- [ ] **Steps 2–4:** FAIL → implement → PASS (`cargo test proxy_metric`) · **Step 5:** commit `feat(engine): proxy request metrics (migration 0019 + repo)`

### Task 10: Router commands + CLI verbs

**Files:**
- Create: `src-tauri/src/engine/commands/proxy.rs`
- Modify: `src-tauri/src/engine/router.rs` (arms after `code.*` at `router.rs:140-150`), `src-tauri/src/engine/commands/mod.rs`, `src-tauri/src/engine/commands/cli.rs` (`map_argv` `:60` gains a `"proxy"` arm → `map_proxy_argv`, modeled on `map_code_argv`), `src-tauri/src/bin/conclave-cli.rs` (special-case `proxy` in `main` `:3735` pattern → plain `cli.exec` round-trip; no local path injection needed)

**Interfaces — Produces:** engine commands `proxy.status` (→ `{active, port, mode, conversations}`), `proxy.mode` (payload `{mode: "off"|"log"|"rewrite"}` → sets the atomic, returns new status), `proxy.report` (payload `{sinceHours?: i64}` default 24 → `ProxyReport`). CLI: `conclave proxy status | mode <off|log|rewrite> | report [--since-hours N]`.

- [ ] **Step 1: failing tests:** handler tests via `router::dispatch` on `AppState::for_tests()` — `proxy.status` returns `mode:"log"`; `proxy.mode {"mode":"rewrite"}` flips it; `proxy.report` returns zeros on empty db; `map_proxy_argv` allowlist test: `proxy status` maps, `proxy nuke` errors.
- [ ] **Steps 2–4:** FAIL → implement → PASS · **Step 5:** commit `feat(cli): conclave proxy status/mode/report`. Lane gates: `cargo test` (full) + `cargo build` from `src-tauri/`.

---

## Lane C — spawn integration (Task 11) · boundary: `src-tauri/src/engine/commands/instance.rs`, `src-tauri/src/engine/runtime/sandbox_config.rs`, `src-tauri/src/engine/repo/agent_definition.rs`, `src-tauri/src/engine/migrations/0020_agent_proxy_enabled.sql`, `src-tauri/src/engine/db.rs` · **starts after lane B merges**

### Task 11: Per-agent opt-in + env injection + sandbox allowlist

**Files:**
- Create: `src-tauri/src/engine/migrations/0020_agent_proxy_enabled.sql` — `ALTER TABLE agent_definition ADD COLUMN proxy_enabled INTEGER;`
- Modify: `src-tauri/src/engine/db.rs` (`if version < 20` gate), `src-tauri/src/engine/repo/agent_definition.rs` (`pub proxy_enabled: Option<bool>` beside `rtk_enabled` `:121`, plus the column in its SELECT list and update path — mirror `rtk_enabled` everywhere it appears), `src-tauri/src/engine/commands/instance.rs` (env block 755-785), `src-tauri/src/engine/runtime/sandbox_config.rs` (network allowlists)

**Interfaces — Consumes:** `state.ctx_proxy.active_port() -> Option<u16>` (Task 7), `proxy_enabled` column (this task).

Env injection, appended at the END of the `extra_env` construction (`instance.rs` ~:785) so it wins over `custom_env`:

```rust
if def.proxy_enabled.unwrap_or(false) {
    if let Some(port) = state.ctx_proxy.active_port() {
        extra_env.push(("ANTHROPIC_BASE_URL".into(), format!("http://127.0.0.1:{port}")));
    }
}
```

(default OFF — note the deliberate asymmetry with `rtk_enabled.unwrap_or(true)`; spec D8.)

Sandbox: in `sandbox_config.rs`, wherever the Claude settings (`claude_sandbox_settings` `:60`) and Codex overrides (`codex_socket_overrides` `:41`) enumerate allowed network destinations, add `127.0.0.1:<port>` when (and only when) the base-URL injection fired — thread the decision through the existing call sites in `instance.rs:629-733` rather than allowlisting unconditionally.

- [ ] **Step 1: failing tests:** (a) repo test: `proxy_enabled` round-trips through agent_definition read/update; (b) unit test on the env-construction helper (extract the injection into a testable `fn proxy_env(def, active_port) -> Option<(String,String)>` if the block isn't already testable): enabled+active → Some, enabled+inactive → None, disabled+active → None; (c) sandbox settings JSON test: with injection on, rendered settings contain the loopback destination; off → unchanged (byte-equal with pre-task snapshot fixture).
- [ ] **Steps 2–4:** FAIL → implement → PASS (full `cargo test`) · **Step 5:** commit `feat(engine): per-agent proxy opt-in wired into spawn env + sandbox`
- [ ] **Step 6 — live verification gate (before READY):** with the rebuilt app running: `conclave proxy status` shows `active`, flip ONE test agent's `proxy_enabled`, spawn it, send one message, then `conclave proxy report --since-hours 1` shows ≥1 request row and the agent's reply arrived normally. Record via `conclave task gate <ws> agent-proxy-spawn -- conclave proxy report --since-hours 1`. This is also the OAuth+base-URL risk check from the spec.

---

## Verification (Phase 1 exit, run by the lead after all merges)

1. `mode log` on a real session → `conclave proxy report` shows plausible would-save numbers.
2. `mode rewrite` + 2–3 real bug-fix tasks → outcomes equivalent to passthrough; report shows ≥25% input reduction after first threshold crossing; cache_read tokens between rewrite rounds not degraded.
3. Kill test: stop the engine mid-session with proxy off for all agents → nothing breaks (opt-in isolation).

## Risk ledger

- axum 0.8 API drift vs plan snippets — trust compiler over plan; the SHAPE (stream-through, no response buffering) is the contract.
- `transcript_context.rs` window fn and `ctxopt::estimate` must agree — Task 1 step 3 note is the guard.
- `instance.rs` is 2768 lines and hot — lane C touches ONLY the env block + the two launch-branch call sites; anything wider escalates to the lead.
- SSE tee buffering would silently ruin TTFT — Task 7/8 tests assert multi-chunk arrival.
- Mode resets to `log` on engine restart BY DESIGN (D9) — do not "fix" this in Phase 1.

## Amendments

(none yet)
