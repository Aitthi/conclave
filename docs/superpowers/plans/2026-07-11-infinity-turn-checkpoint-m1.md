# Infinity-Turn Checkpoint — Milestone-1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

owner: 4fb2198c-e0d9-4e4b-af9e-d4e72542bace (Detoro) · authority: in-loop · spec: `docs/superpowers/specs/2026-07-11-infinity-turn-checkpoint-design.md` (v3, council-approved)

> **⚠ AMENDMENT R8 (2026-07-11, Detoro chair + Aoki co-principal; challenges 4f3aa72a + c08a3cf1).** This plan was implemented and shipped, then M1 measurement returned **0 interpretable samples**. Root cause: the `project()` M/L pre-gate runs in `bytes/4` space (overstates real tokens ~4.7× on cache-heavy traffic) and `checkpoint_gate` maps `Saturated → None` (no metric row). The **byte-space `Projection`/`project()` M/L pre-gate design and the `Saturated ⇒ None` mapping shown below are SUPERSEDED** — do not re-implement them as written. The authoritative corrected design (3-bucket `below_ceiling|eligible|saturated` classification decided **after** `count_tokens`, always persisting a row, `outcome` column added) lives in the fix plan **`docs/superpowers/plans/2026-07-11-infinity-turn-checkpoint-m1-fix.md`** and spec §4/§7.1 (R8). Everything else in this plan (ctxopt structure, recoverability classifier, count_tokens a/b/c client, async queue, migration pattern) stands.

**Goal:** Add a **log-mode-only** checkpoint projection to the ctx-proxy that measures, for real long-context `/v1/messages` traffic, whether freezing recoverable old tool_results *would* pull the effective context into a low-water band — recording the full `q = S_net/R` metric contract via Anthropic `count_tokens` — **without ever altering the bytes forwarded upstream**.

**Architecture:** Pure deterministic logic (recoverability classifier, checkpoint policy, projection + hysteresis pre-gate) lands in `src-tauri/crates/ctxopt` (serde_json only). All async/IO — the `count_tokens` client, the off-forwarding-path sampling queue, the new metric persistence, the global checkpoint toggle, and the ctx_proxy wiring — lives engine-side in `src-tauri/src/engine/`. The measurement path reads the request body by reference and produces metric rows; it is structurally incapable of changing `upstream_body`.

**Tech Stack:** Rust, tokio 1 (present), axum 0.8 (present), reqwest 0.12 rustls+stream (present), serde_json, sqlx/sqlite via the existing `db.rs` migration pattern and `chain_builder` query pattern.

## Global Constraints (every task inherits these)

- **Milestone-1 is LOG-MODE PROJECTION ONLY.** Measure whether a checkpoint *would* work. Do **not** implement the apply/rewrite path, per-agent isolation, or a snapshot store — those are explicitly deferred (spec §9).
- **NEVER alter forwarded bytes in Milestone-1.** The checkpoint measurement path must never influence `upstream_body`. This is asserted by a test (Task 7). The dedup rewrite path (`MODE_REWRITE`) is untouched and orthogonal.
- **Fail-open:** any error on the checkpoint/measurement path (parse, classify, project, count, persist) forwards the ORIGINAL request untouched and records the failure without failing the request. The proxy must never be able to break an agent.
- **`validate` stays strict** (validate.rs, unchanged): message count unchanged, every `tool_use` keeps a matching `tool_result`, block/key sets unchanged, only `tool_result.content` shrinks. Projected message-lists built for `count_tokens` (b) route through `validate` against the original before they are counted.
- **ctxopt crate purity:** `crates/ctxopt` depends on `serde_json` ONLY. No tokio, no axum, no reqwest, no engine types. All async/IO lives in the engine.
- **`count_tokens` is off the forwarding path:** sampling is asynchronous/queued and bounded; `count_tokens` RPM and latency can never delay or block forwarded traffic. A full sampling queue drops the sample (recorded), it never waits.
- **`count_tokens` returns a provider ESTIMATE.** Every persisted token value is labelled a provider estimate (`provider_estimate = 1`); `bytes/4` is kept only as a diagnostic column. Count failures are recorded (`count_failure = 1`), never fatal.
- **GLOBAL, not per-agent.** The toggle is a single app-global flag (`proxy checkpoint on|off`), default **off**. Per-agent control is blocked until an isolation design lands (spec §6).
- **Never log or persist auth headers** (`authorization`, `x-api-key`, `api-key`) **or request/response bodies.** Credential headers captured for `count_tokens` are used in-flight and dropped; metric rows carry counts/labels only.
- **Cargo.lock ships with every dep-changing commit.** No new deps are expected in this milestone (all crates already present); if any `Cargo.toml` changes, commit `src-tauri/Cargo.lock` in the same commit.
- **Pass criterion (the gate this milestone feeds):** post-context enters a defined low-water band on real long-context traffic **AND** q/plateau support the cost bound. Large S_net alone does **not** pass. **Plan prerequisite:** Task 4 ships a live-credential preflight proving `count_tokens` is authorized before any gate depends on it.
- Cargo gates run from `src-tauri/` (the worktree root has no Cargo.toml). Pure-crate tasks gate on `cargo test -p ctxopt`; engine tasks gate on the named `cargo test <filter>` plus full `cargo test` + `cargo build` before READY, recorded via `conclave task gate <ws> <slug> -- <cmd>`.
- No UI files are touched → the UI pixel gate does not apply.

## Constants (single source: `crates/ctxopt/src/lib.rs`, appended beside the existing five)

```rust
pub const DEFAULT_CEILING_TOKENS: usize = 450_000;  // C: evaluate a checkpoint above this effective-context estimate
pub const RECENT_TAIL_MSGS: usize = 15;             // keep the last ~10–20 messages verbatim (never frozen)
pub const MIN_NET_SAVING_TOKENS: usize = 40_000;    // M: floor on projected net token saving to proceed to sampling
pub const LOW_WATER_TOKENS: usize = 350_000;        // L: projected post-checkpoint tokens must land at/below this (< ceiling)
```

Engine runtime defaults (Task 7): global checkpoint flag default **off**; ceiling/M/L/tail seeded from the ctxopt consts above, runtime-settable via atomics that reset on restart. `count_tokens` method label: `CHECKPOINT_METHOD_VERSION = "m1-count_tokens-2023-06-01"`.

> **Task ordering note (boundary adjustment — code demanded it).** The spec's task sketch numbers plateau-tracking last (its T8) and ctx_proxy wiring earlier (its T6). Plateau state is *pure ledger state* that the engine wiring consumes, and the house rule forbids referencing a type before it is defined. So this plan orders the pure plateau-state work (Task 6) **before** the engine wiring (Task 7), and lands the CLI/commands surface (Task 8) last. Same eight pieces of work; dependency-correct order.

---

### Task 1: Recoverability classifier (ctxopt, pure)

**Files:**
- Create: `src-tauri/crates/ctxopt/src/checkpoint.rs` (module home for all checkpoint logic; classifier lands here first)
- Modify: `src-tauri/crates/ctxopt/src/lib.rs` (add the four consts above; add `pub mod checkpoint;`)
- Test path: `src-tauri/crates/ctxopt/src/checkpoint.rs` `#[cfg(test)] mod tests`

**Interfaces — Produces:** `ctxopt::checkpoint::is_recoverable(tool_name: &str) -> bool`.

Eliding a recoverable tool_result loses only *historical* bytes the model saw; the agent can re-obtain current state on demand (spec §3). Non-recoverable outputs (side-effecting, drifting, or mutating) are kept verbatim and tracked separately. Unknown tool names default to **non-recoverable** (fail-safe: keep). Code-intel/`Conclave` reads ride the single `Conclave` command tool on the wire (they are not distinct `tool_use` names in this app), so they conservatively fall into the non-recoverable-kept bucket in M1 — safe (M1 never alters bytes) and reclassifiable later once a content-addressed name is carried.

- [ ] **Step 1: write failing test** in `checkpoint.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_family_is_recoverable() {
        for t in ["Read", "Grep", "Glob", "LS", "WebSearch", "NotebookRead"] {
            assert!(is_recoverable(t), "{t} should be recoverable");
        }
    }

    #[test]
    fn side_effecting_and_drifting_tools_are_not_recoverable() {
        for t in ["Bash", "WebFetch", "Write", "Edit", "MultiEdit", "NotebookEdit", "Task"] {
            assert!(!is_recoverable(t), "{t} must not be recoverable");
        }
    }

    #[test]
    fn unknown_tool_defaults_to_not_recoverable() {
        assert!(!is_recoverable("Conclave"));
        assert!(!is_recoverable("mcp__whatever__do"));
        assert!(!is_recoverable(""));
    }
}
```

- [ ] **Step 2:** `cargo test -p ctxopt checkpoint` from `src-tauri/` → FAIL (`cannot find function is_recoverable` / `unresolved module checkpoint`)
- [ ] **Step 3: minimal impl.** In `lib.rs` add the four consts and `pub mod checkpoint;`. In `checkpoint.rs`:

```rust
//! Pure, deterministic checkpoint projection for the Conclave ctx-proxy.
//! Milestone-1: LOG MODE ONLY — it measures what a checkpoint *would* do and
//! never alters forwarded bytes. serde_json only (crate purity).

/// Read-only, re-runnable-for-current-state tools whose historical output may be
/// stubbed and re-obtained on demand. Everything else (side-effecting, drifting,
/// mutating) and every unknown name is kept verbatim (fail-safe).
pub fn is_recoverable(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "Read" | "Grep" | "Glob" | "LS" | "WebSearch" | "NotebookRead"
    )
}
```

- [ ] **Step 4:** `cargo test -p ctxopt checkpoint` → PASS
- [ ] **Step 5:** commit `git add src-tauri/crates/ctxopt/src/checkpoint.rs src-tauri/crates/ctxopt/src/lib.rs && git commit -m "feat(ctxopt): checkpoint recoverability classifier + M1 consts"`

### Task 2: Checkpoint policy — frozen region + candidate selection (ctxopt, pure)

**Files:**
- Modify: `src-tauri/crates/ctxopt/src/checkpoint.rs` (add plan types + `plan_checkpoint`)
- Test path: `src-tauri/crates/ctxopt/src/checkpoint.rs` tests

**Interfaces — Consumes:** `crate::request::{index_tools, ToolCall, ToolResultRef}`, `crate::checkpoint::is_recoverable`, `crate::estimate::est_tokens`. **Produces:**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointCandidate {
    pub tool_use_id: String,
    pub tool_name: String,
    pub msg_idx: usize,       // message index of the tool_result being stubbed
    pub gross_bytes: usize,   // serialized bytes of the original result content (reconstructed)
    pub stub: String,         // self-describing breadcrumb replacing the content
    pub stub_bytes: usize,    // serialized bytes of the stubbed content array
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointPlan {
    pub candidates: Vec<CheckpointCandidate>,
    pub earliest_changed_msg_index: usize, // min candidate msg_idx, or tail_start if none
    pub tail_start: usize,                  // first message index of the verbatim recent tail
    pub non_recoverable_kept_bytes: usize,  // gross bytes of frozen-region results kept verbatim
}

/// None when the estimate is at/below the ceiling (no checkpoint considered).
/// Some(plan) when above ceiling; `candidates` may be empty (Task 3 will rule it saturated).
pub fn plan_checkpoint(
    messages: &serde_json::Value,
    est_tokens: usize,   // estimator output for the whole request (injected; bytes/4 engine-side)
    ceiling_tokens: usize,
    tail_msgs: usize,
) -> Option<CheckpointPlan>;
```

Frozen region = messages `[0 .. tail_start)` where `tail_start = total_msgs.saturating_sub(tail_msgs)`. Pair each `tool_result` to its `tool_use` (same technique as `analyze.rs`: `index_tools` + a `HashMap<&str,&ToolCall>` by id) to recover the tool name. A candidate is any paired, text-bearing `tool_result` in the frozen region whose tool is `is_recoverable`. Non-recoverable text-bearing results in the frozen region accumulate into `non_recoverable_kept_bytes`. Breadcrumb per spec §3: `[ctxopt checkpoint: elided <Tool> <path?> @turn <msg_idx> — re-read to restore]` (path from the paired call's `file_path`/`notebook_path` when present, else omitted).

- [ ] **Step 1: write failing test** in `checkpoint.rs` tests (reuse the `tool_pair`/`filler`/`msgs` helpers pattern from `analyze.rs`; add local copies):

```rust
    use serde_json::{json, Value};

    fn tool_pair(id: &str, name: &str, input: Value, text: &str) -> [Value; 2] {
        [
            json!({"role":"assistant","content":[{"type":"tool_use","id":id,"name":name,"input":input}]}),
            json!({"role":"user","content":[{"type":"tool_result","tool_use_id":id,"content":[{"type":"text","text":text}]}]}),
        ]
    }
    fn filler(n: usize) -> Vec<Value> {
        (0..n).map(|i| json!({"role":"user","content":[{"type":"text","text":format!("f{i}")}]})).collect()
    }
    fn msgs(pairs: Vec<[Value; 2]>, fill: usize) -> Value {
        let mut out: Vec<Value> = pairs.into_iter().flatten().collect();
        out.extend(filler(fill));
        Value::Array(out)
    }

    #[test]
    fn below_ceiling_returns_none() {
        let m = msgs(vec![tool_pair("t1", "Read", json!({"file_path":"/a.rs"}), &"x".repeat(700))], 40);
        assert!(plan_checkpoint(&m, 100, 450_000, 15).is_none());
    }

    #[test]
    fn selects_recoverable_in_frozen_region_and_pairs_tool_name() {
        // 3 recoverable reads up front, then 40 filler messages push them out of the tail.
        let big = "x".repeat(2000);
        let m = msgs(
            vec![
                tool_pair("t1", "Read", json!({"file_path":"/a.rs"}), &big),
                tool_pair("t2", "Grep", json!({"pattern":"foo"}), &big),
                tool_pair("t3", "Bash", json!({"command":"ls"}), &big), // non-recoverable → kept bucket
            ],
            40,
        );
        let plan = plan_checkpoint(&m, 500_000, 450_000, 15).expect("above ceiling");
        let ids: Vec<&str> = plan.candidates.iter().map(|c| c.tool_use_id.as_str()).collect();
        assert_eq!(ids, ["t1", "t2"]); // Bash excluded
        assert_eq!(plan.candidates[0].tool_name, "Read");
        assert!(plan.candidates[0].stub.contains("Read"));
        assert!(plan.candidates[0].stub.contains("/a.rs"));
        assert_eq!(plan.earliest_changed_msg_index, 1); // t1 result is message #1
        assert!(plan.non_recoverable_kept_bytes > 0);   // the Bash result
        assert!(plan.candidates[0].gross_bytes > plan.candidates[0].stub_bytes);
    }

    #[test]
    fn recent_tail_is_never_a_candidate() {
        // Everything sits inside the last 15 messages → nothing frozen.
        let big = "x".repeat(2000);
        let m = msgs(vec![tool_pair("t1", "Read", json!({"file_path":"/a.rs"}), &big)], 2);
        let plan = plan_checkpoint(&m, 500_000, 450_000, 15).expect("above ceiling");
        assert!(plan.candidates.is_empty());
        assert_eq!(plan.earliest_changed_msg_index, plan.tail_start);
    }
```

- [ ] **Step 2:** `cargo test -p ctxopt checkpoint` → FAIL (`cannot find function plan_checkpoint` / missing types)
- [ ] **Step 3: minimal impl.** Add to `checkpoint.rs`:

```rust
use std::collections::HashMap;
use serde_json::{json, Value};

use crate::request::{index_tools, ToolCall};

fn candidate_path(call: &ToolCall) -> Option<String> {
    call.input
        .get("file_path")
        .or_else(|| call.input.get("notebook_path"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn breadcrumb(tool: &str, path: Option<&str>, turn: usize) -> String {
    match path {
        Some(p) => format!("[ctxopt checkpoint: elided {tool} {p} @turn {turn} — re-read to restore]"),
        None => format!("[ctxopt checkpoint: elided {tool} @turn {turn} — re-read to restore]"),
    }
}

fn content_bytes(text: &str) -> usize {
    json!([{ "type": "text", "text": text }]).to_string().len()
}

pub fn plan_checkpoint(
    messages: &Value,
    est_tokens: usize,
    ceiling_tokens: usize,
    tail_msgs: usize,
) -> Option<CheckpointPlan> {
    if est_tokens <= ceiling_tokens {
        return None;
    }
    let total_msgs = messages.as_array().map_or(0, Vec::len);
    let tail_start = total_msgs.saturating_sub(tail_msgs);

    let (calls, results) = index_tools(messages);
    let call_by_id: HashMap<&str, &ToolCall> =
        calls.iter().map(|c| (c.id.as_str(), c)).collect();

    let mut candidates: Vec<CheckpointCandidate> = Vec::new();
    let mut non_recoverable_kept_bytes = 0usize;

    for r in &results {
        if r.msg_idx >= tail_start {
            continue; // verbatim recent tail
        }
        let Some(text) = r.text.as_deref() else { continue };
        let Some(call) = call_by_id.get(r.tool_use_id.as_str()) else { continue };
        if !is_recoverable(&call.name) {
            non_recoverable_kept_bytes += content_bytes(text);
            continue;
        }
        let path = candidate_path(call);
        let stub = breadcrumb(&call.name, path.as_deref(), r.msg_idx);
        let stub_bytes = content_bytes(&stub);
        candidates.push(CheckpointCandidate {
            tool_use_id: r.tool_use_id.clone(),
            tool_name: call.name.clone(),
            msg_idx: r.msg_idx,
            gross_bytes: content_bytes(text),
            stub,
            stub_bytes,
        });
    }
    candidates.sort_by_key(|c| c.msg_idx);
    let earliest_changed_msg_index = candidates.first().map_or(tail_start, |c| c.msg_idx);

    Some(CheckpointPlan {
        candidates,
        earliest_changed_msg_index,
        tail_start,
        non_recoverable_kept_bytes,
    })
}
```

(Place the `CheckpointCandidate`/`CheckpointPlan` struct definitions from the Interfaces block above the fn.)

- [ ] **Step 4:** `cargo test -p ctxopt checkpoint` → PASS
- [ ] **Step 5:** commit `git add src-tauri/crates/ctxopt/src/checkpoint.rs && git commit -m "feat(ctxopt): checkpoint frozen-region + recoverable candidate selection"`

### Task 3: Projection + hysteresis pre-gate (ctxopt, pure) + reuse apply stubbing

**Files:**
- Modify: `src-tauri/crates/ctxopt/src/apply.rs` (extract `stub_tool_results` so both dedup-`apply` and checkpoint share the stubbing; existing `apply` delegates — existing tests must stay green)
- Modify: `src-tauri/crates/ctxopt/src/checkpoint.rs` (add `Projection`, `CheckpointOutcome`, `project`)
- Test path: both files' tests

**Interfaces — Consumes:** `crate::apply::stub_tool_results`, `crate::validate::validate`, `crate::estimate::est_tokens`, `crate::checkpoint::CheckpointPlan`. **Produces:**

```rust
// apply.rs
pub fn stub_tool_results(messages: &mut serde_json::Value, stubs: &std::collections::HashMap<&str, &str>) -> usize;

// checkpoint.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Projection {
    pub projected_messages: serde_json::Value,
    pub gross_candidate_bytes: usize,
    pub stub_overhead_bytes: usize,
    pub net_saved_bytes: usize,       // gross − overhead (saturating)
    pub net_saved_tokens: usize,      // est_tokens(net_saved_bytes)
    pub projected_post_tokens: usize, // est_whole_tokens − net_saved_tokens (saturating)
}

// ⚠ SUPERSEDED by R8 (see top banner + m1-fix plan): the byte-space M/L pre-gate
// below and the 2-outcome enum are replaced by a 3-bucket classification decided
// AFTER count_tokens (below_ceiling|eligible|saturated), always persisting a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointOutcome {
    Saturated,               // below M, above L, or nothing to freeze → no sampling, no change
    Eligible(Projection),
}

/// Build the projected message list and apply the min-net-saving (M) + low-water (L)
/// pre-gate. Bounds count_tokens calls: only Eligible outcomes get sampled.
pub fn project(
    messages: &serde_json::Value,
    plan: &CheckpointPlan,
    est_whole_tokens: usize,
    min_net_saving_tokens: usize, // M
    low_water_tokens: usize,      // L
) -> CheckpointOutcome;
```

- [ ] **Step 1a: write failing test** in `apply.rs` tests (new stubbing seam, existing behaviour preserved):

```rust
    #[test]
    fn stub_tool_results_replaces_only_named_ids() {
        use std::collections::HashMap;
        let mut m = fixture(); // tu_1 tool_result with 700-byte content
        let stubs: HashMap<&str, &str> = HashMap::from([("tu_1", "[ctxopt checkpoint: elided Read /a.rs @turn 1 — re-read to restore]")]);
        let saved = stub_tool_results(&mut m, &stubs);
        assert!(saved > 0);
        assert_eq!(m[1]["content"][0]["content"][0]["text"], stubs["tu_1"]);
        assert_eq!(m[1]["content"][0]["cache_control"]["type"], "ephemeral"); // sibling key preserved
    }
```

- [ ] **Step 1b: write failing test** in `checkpoint.rs` tests:

```rust
    #[test]
    fn eligible_when_net_saving_over_m_and_post_under_l() {
        // Two ~8KB recoverable reads → ~4000 net tokens saved from the estimate.
        let big = "x".repeat(8000);
        let m = msgs(
            vec![
                tool_pair("t1", "Read", json!({"file_path":"/a.rs"}), &big),
                tool_pair("t2", "Read", json!({"file_path":"/b.rs"}), &big),
            ],
            40,
        );
        let plan = plan_checkpoint(&m, 500_000, 450_000, 15).unwrap();
        // est_whole 500_000; M=1_000 tokens; L=499_000 → post ≈ 500_000 − net, must be ≤ L.
        match project(&m, &plan, 500_000, 1_000, 499_000) {
            CheckpointOutcome::Eligible(p) => {
                assert!(p.net_saved_tokens > 1_000);
                assert!(p.projected_post_tokens <= 499_000);
                assert!(p.gross_candidate_bytes > p.stub_overhead_bytes);
                // projected list differs from the original and is byte-shrunk: the
                // first stubbed tool_result now carries the breadcrumb, not the 8KB body.
                assert_ne!(p.projected_messages, m);
                assert!(p.projected_messages[1]["content"][0]["content"][0]["text"]
                    .as_str().unwrap().starts_with("[ctxopt checkpoint:"));
            }
            other => panic!("expected Eligible, got {other:?}"),
        }
    }

    #[test]
    fn saturated_when_net_saving_below_m() {
        let big = "x".repeat(8000);
        let m = msgs(vec![tool_pair("t1", "Read", json!({"file_path":"/a.rs"}), &big)], 40);
        let plan = plan_checkpoint(&m, 500_000, 450_000, 15).unwrap();
        // Demand an absurd M → cannot clear it.
        assert_eq!(project(&m, &plan, 500_000, 10_000_000, 499_000), CheckpointOutcome::Saturated);
    }

    #[test]
    fn saturated_when_post_stays_above_l() {
        let big = "x".repeat(8000);
        let m = msgs(vec![tool_pair("t1", "Read", json!({"file_path":"/a.rs"}), &big)], 40);
        let plan = plan_checkpoint(&m, 500_000, 450_000, 15).unwrap();
        // L below the projected post → cannot clear the low-water.
        assert_eq!(project(&m, &plan, 500_000, 1, 1_000), CheckpointOutcome::Saturated);
    }

    #[test]
    fn saturated_when_no_candidates() {
        let big = "x".repeat(8000);
        let m = msgs(vec![tool_pair("t1", "Read", json!({"file_path":"/a.rs"}), &big)], 2); // all in tail
        let plan = plan_checkpoint(&m, 500_000, 450_000, 15).unwrap();
        assert_eq!(project(&m, &plan, 500_000, 1, 499_000), CheckpointOutcome::Saturated);
    }
```

- [ ] **Step 2:** `cargo test -p ctxopt` → FAIL (`cannot find function stub_tool_results` / `project`)
- [ ] **Step 3: minimal impl.** In `apply.rs`, extract the stubbing loop and delegate:

```rust
use std::collections::HashMap;

/// Replace the `content` of every tool_result whose id is in `stubs` with a
/// text-only stub. Only `content` changes — every sibling key is preserved.
/// Returns serialized bytes saved (old − new, saturating, summed).
pub fn stub_tool_results(messages: &mut Value, stubs: &HashMap<&str, &str>) -> usize {
    let mut saved = 0usize;
    let Some(msgs) = messages.as_array_mut() else { return 0 };
    for msg in msgs {
        let Some(blocks) = msg.get_mut("content").and_then(Value::as_array_mut) else { continue };
        for b in blocks {
            if b.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let Some(stub) = b.get("tool_use_id").and_then(Value::as_str).and_then(|id| stubs.get(id)) else { continue };
            let replacement = json!([{ "type": "text", "text": stub }]);
            if let Some(obj) = b.as_object_mut() {
                let old_len = obj.get("content").map_or(0, |c| c.to_string().len());
                let new_len = replacement.to_string().len();
                obj.insert("content".into(), replacement);
                saved += old_len.saturating_sub(new_len);
            }
        }
    }
    saved
}

pub fn apply(messages: &mut Value, elisions: &[Elision]) -> usize {
    let stubs: HashMap<&str, &str> =
        elisions.iter().map(|e| (e.tool_use_id.as_str(), e.stub.as_str())).collect();
    stub_tool_results(messages, &stubs)
}
```

In `checkpoint.rs`, add the `Projection`/`CheckpointOutcome` types and:

```rust
use crate::estimate::est_tokens;

pub fn project(
    messages: &Value,
    plan: &CheckpointPlan,
    est_whole_tokens: usize,
    min_net_saving_tokens: usize,
    low_water_tokens: usize,
) -> CheckpointOutcome {
    if plan.candidates.is_empty() {
        return CheckpointOutcome::Saturated;
    }
    let stubs: HashMap<&str, &str> = plan
        .candidates
        .iter()
        .map(|c| (c.tool_use_id.as_str(), c.stub.as_str()))
        .collect();
    let mut projected = messages.clone();
    crate::apply::stub_tool_results(&mut projected, &stubs);

    let gross_candidate_bytes: usize = plan.candidates.iter().map(|c| c.gross_bytes).sum();
    let stub_overhead_bytes: usize = plan.candidates.iter().map(|c| c.stub_bytes).sum();
    let net_saved_bytes = gross_candidate_bytes.saturating_sub(stub_overhead_bytes);
    let net_saved_tokens = est_tokens(net_saved_bytes);
    let projected_post_tokens = est_whole_tokens.saturating_sub(net_saved_tokens);

    if net_saved_tokens > min_net_saving_tokens && projected_post_tokens <= low_water_tokens {
        CheckpointOutcome::Eligible(Projection {
            projected_messages: projected,
            gross_candidate_bytes,
            stub_overhead_bytes,
            net_saved_bytes,
            net_saved_tokens,
            projected_post_tokens,
        })
    } else {
        CheckpointOutcome::Saturated
    }
}
```

- [ ] **Step 4:** `cargo test -p ctxopt` → PASS (existing apply/validate tests included — the refactor must not regress them)
- [ ] **Step 5:** commit `git add src-tauri/crates/ctxopt/src/apply.rs src-tauri/crates/ctxopt/src/checkpoint.rs && git commit -m "feat(ctxopt): checkpoint projection + M/L hysteresis pre-gate (shared stubbing)"`

### Task 4: count_tokens client + async sampling queue (engine)

> **SECURITY CONTAINMENT (council challenge ea3df57c, Aoki — MANDATORY before this task is READY).** Incoming-request auth is the correct source, but it must be contained:
> - **Dedicated count client, no redirects:** build a separate `reqwest::Client` with `.redirect(reqwest::redirect::Policy::none())` and an explicit `.timeout(...)` (e.g. 20s); no retries. Rationale: reqwest 0.12 default follows up to 10 redirects and its `remove_sensitive_headers` strips `Authorization`/`Cookie` cross-host but **NOT `x-api-key`** — a redirect to another host would leak the client key. `Policy::none()` means a 3xx is returned as-is and no second, credential-bearing request is ever made.
> - **Sensitive credential type:** `CountCredential` keeps `#[derive(Clone)]` ONLY — never add `Debug`/`Serialize`/`Deserialize`. Store each auth value as a `reqwest::header::HeaderValue` with `.set_sensitive(true)`. Forward only an explicit allowlist: `authorization`, `x-api-key`, `anthropic-version` (and `anthropic-beta` only if count fidelity requires it) — never the whole header map.
> - **Missing auth → zero calls:** if neither `authorization` nor `x-api-key` is present, return a count-failure outcome and make **no** remote request.
> - **Required tests (in this task):** (a) a fake upstream returning a cross-host 3xx → the count client does NOT follow it and no credential reaches the redirect target; (b) a slow upstream → the explicit timeout fires, the permit is released and the credential dropped (no hang); (c) missing-auth input → zero remote calls, failure recorded.

**Files:**
- Create: `src-tauri/src/engine/runtime/count_tokens.rs`
- Modify: `src-tauri/src/engine/runtime/mod.rs` (add `pub mod count_tokens;`)
- Test path: `src-tauri/src/engine/runtime/count_tokens.rs` tests (async, `#[tokio::test]`, fake upstream in the same style as `ctx_proxy.rs` tests)

**Interfaces — Produces:**

```rust
pub const CHECKPOINT_METHOD_VERSION: &str = "m1-count_tokens-2023-06-01";

/// Auth headers lifted from the forwarded /v1/messages request. Used in-flight
/// for count_tokens and dropped; never logged or persisted (Global Constraints).
#[derive(Clone)]
pub struct CountCredential {
    pub api_key: Option<String>,        // x-api-key
    pub authorization: Option<String>,  // Authorization: Bearer …
    pub anthropic_version: String,      // anthropic-version header (default "2023-06-01")
}

/// Build a count_tokens body from a /v1/messages request: model + the given
/// messages + optional system/tools/tool_choice (max_tokens/stream/metadata dropped).
pub fn count_tokens_body(request: &serde_json::Value, messages: &serde_json::Value) -> serde_json::Value;

/// Structurally valid prefix ending at the message boundary BEFORE `earliest_changed_msg_index`.
pub fn prefix_messages(messages: &serde_json::Value, earliest_changed_msg_index: usize) -> serde_json::Value;

/// POST <upstream>/v1/messages/count_tokens; returns provider input_tokens estimate.
pub async fn count_tokens(
    client: &reqwest::Client,
    upstream: &str,
    cred: &CountCredential,
    body: &serde_json::Value,
) -> Result<u64, String>;

/// Credential preflight (plan prerequisite): a trivial count_tokens call proving
/// the live Claude credential is authorized for the endpoint. Ok(()) iff HTTP 200.
pub async fn preflight(
    client: &reqwest::Client,
    upstream: &str,
    cred: &CountCredential,
) -> Result<(), String>;
```

The sampling *queue* is a bounded `tokio::sync::Semaphore` (default 2 permits) held on `ProxyRuntime` (added in Task 7). Enqueue = `try_acquire_owned()`; on `Err` the sample is dropped and recorded as `count_failure`-adjacent (a `saturated`-style skip), it **never** blocks. Task 7 owns the enqueue; Task 4 owns the client + body builders + preflight so they are unit-testable in isolation.

- [ ] **Step 1: write failing test** in `count_tokens.rs` (fake upstream echoing `input_tokens` derived from body size, plus a 401 path for preflight):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, Response, StatusCode};
    use axum::routing::any;
    use axum::Router;
    use serde_json::json;

    async fn fake_upstream() -> (String, tokio::task::JoinHandle<()>) {
        async fn handler(request: Request<Body>) -> Response<Body> {
            let unauth = request.headers().get("x-api-key").map(|v| v == "bad").unwrap_or(false);
            if unauth {
                return Response::builder().status(StatusCode::UNAUTHORIZED).body(Body::from("no")).unwrap();
            }
            let body = to_bytes(request.into_body(), usize::MAX).await.unwrap();
            let count = body.len() as u64; // deterministic stand-in for a token count
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/json")
                .body(Body::from(json!({ "input_tokens": count }).to_string()))
                .unwrap()
        }
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move { axum::serve(listener, Router::new().fallback(any(handler))).await.unwrap() });
        (format!("http://{addr}"), handle)
    }

    fn cred(key: &str) -> CountCredential {
        CountCredential { api_key: Some(key.into()), authorization: None, anthropic_version: "2023-06-01".into() }
    }

    #[test]
    fn body_builder_keeps_model_and_swaps_messages_and_drops_max_tokens() {
        let req = json!({ "model": "claude-x", "max_tokens": 999, "stream": true,
            "system": "s", "tools": [{"name":"Read"}], "messages": [{"role":"user","content":"a"}] });
        let msgs = json!([{"role":"user","content":"b"}]);
        let body = count_tokens_body(&req, &msgs);
        assert_eq!(body["model"], "claude-x");
        assert_eq!(body["system"], "s");
        assert_eq!(body["tools"][0]["name"], "Read");
        assert_eq!(body["messages"], msgs);
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("stream").is_none());
    }

    #[test]
    fn prefix_truncates_at_message_boundary() {
        let msgs = json!([{"role":"user","content":"0"},{"role":"assistant","content":"1"},{"role":"user","content":"2"}]);
        let p = prefix_messages(&msgs, 2);
        assert_eq!(p.as_array().unwrap().len(), 2);
        assert_eq!(p[1]["content"], "1");
    }

    #[tokio::test]
    async fn count_tokens_returns_provider_estimate() {
        let (upstream, h) = fake_upstream().await;
        let client = reqwest::Client::new();
        let body = json!({ "model": "claude-x", "messages": [{"role":"user","content":"hello"}] });
        let n = count_tokens(&client, &upstream, &cred("good"), &body).await.unwrap();
        assert!(n > 0);
        h.abort();
    }

    #[tokio::test]
    async fn preflight_rejects_unauthorized_credential() {
        let (upstream, h) = fake_upstream().await;
        let client = reqwest::Client::new();
        assert!(preflight(&client, &upstream, &cred("good")).await.is_ok());
        assert!(preflight(&client, &upstream, &cred("bad")).await.is_err());
        h.abort();
    }
}
```

- [ ] **Step 2:** `cargo test count_tokens` from `src-tauri/` → FAIL (module/functions missing)
- [ ] **Step 3: minimal impl** in `count_tokens.rs`:

```rust
//! Anthropic count_tokens client + credential preflight for the ctx-proxy
//! checkpoint gate. Runs OFF the forwarding path; returns provider ESTIMATES.

use serde_json::{json, Value};

pub const CHECKPOINT_METHOD_VERSION: &str = "m1-count_tokens-2023-06-01";

#[derive(Clone)]
pub struct CountCredential {
    pub api_key: Option<String>,
    pub authorization: Option<String>,
    pub anthropic_version: String,
}

pub fn count_tokens_body(request: &Value, messages: &Value) -> Value {
    let mut body = json!({
        "model": request.get("model").cloned().unwrap_or(Value::Null),
        "messages": messages.clone(),
    });
    for key in ["system", "tools", "tool_choice"] {
        if let Some(v) = request.get(key) {
            body[key] = v.clone();
        }
    }
    body
}

pub fn prefix_messages(messages: &Value, earliest_changed_msg_index: usize) -> Value {
    match messages.as_array() {
        Some(arr) => {
            let end = earliest_changed_msg_index.min(arr.len());
            Value::Array(arr[..end].to_vec())
        }
        None => Value::Array(Vec::new()),
    }
}

fn apply_cred(mut req: reqwest::RequestBuilder, cred: &CountCredential) -> reqwest::RequestBuilder {
    if let Some(key) = &cred.api_key {
        req = req.header("x-api-key", key);
    }
    if let Some(auth) = &cred.authorization {
        req = req.header("authorization", auth);
    }
    req.header("anthropic-version", cred.anthropic_version.as_str())
        .header("content-type", "application/json")
}

pub async fn count_tokens(
    client: &reqwest::Client,
    upstream: &str,
    cred: &CountCredential,
    body: &Value,
) -> Result<u64, String> {
    let url = format!("{}/v1/messages/count_tokens", upstream.trim_end_matches('/'));
    let resp = apply_cred(client.post(url), cred)
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("count_tokens HTTP {}", resp.status()));
    }
    let value: Value = resp.json().await.map_err(|e| e.to_string())?;
    value
        .get("input_tokens")
        .and_then(Value::as_u64)
        .ok_or_else(|| "count_tokens: missing input_tokens".to_string())
}

pub async fn preflight(
    client: &reqwest::Client,
    upstream: &str,
    cred: &CountCredential,
) -> Result<(), String> {
    let body = json!({ "model": "claude-3-5-haiku-20241022",
        "messages": [{ "role": "user", "content": "ok" }] });
    count_tokens(client, upstream, cred, &body).await.map(|_| ())
}
```

- [ ] **Step 4:** `cargo test count_tokens` → PASS
- [ ] **Step 5:** commit `git add src-tauri/src/engine/runtime/count_tokens.rs src-tauri/src/engine/runtime/mod.rs && git commit -m "feat(engine): anthropic count_tokens client + credential preflight"`

### Task 5: Checkpoint metric contract persistence (engine: migration 0021 + sibling repo)

**Files:**
- Create: `src-tauri/src/engine/migrations/0021_proxy_checkpoint_metric.sql`
- Create: `src-tauri/src/engine/repo/proxy_checkpoint_metric.rs`
- Modify: `src-tauri/src/engine/db.rs` (add `if version < 21 { … 0021 … PRAGMA user_version = 21 }` block; add `include_str!("migrations/0021_proxy_checkpoint_metric.sql")` to the `connect_at_v13`-style chain list if present)
- Modify: `src-tauri/src/engine/repo/mod.rs` (add `pub mod proxy_checkpoint_metric;`)
- Test path: `proxy_checkpoint_metric.rs` tests (`connect_in_memory`)

**Interfaces — Produces:**

```rust
pub struct CheckpointMetricInsert {
    pub created_at: String,
    pub model: String,
    pub earliest_changed_byte: i64,
    pub earliest_changed_msg: i64,
    pub r_tokens: i64,                 // a − c
    pub gross_candidate_tokens: i64,
    pub stub_overhead_tokens: i64,
    pub s_net_tokens: i64,             // a − b
    pub q: f64,                        // s_net / r
    pub projected_break_even: f64,     // 11.5/q − 12.5 (spec §2)
    pub projected_post_tokens: i64,    // b
    pub plateau_turns: i64,            // observed (Task 6); 0 on first sample
    pub non_recoverable_kept_tokens: i64,
    pub provider_estimate: i64,        // 1 = count_tokens estimate
    pub count_failure: i64,            // 1 = a/b/c count failed; token cols may be 0
    pub method_version: String,
    pub bytes_est_tokens: i64,         // bytes/4 diagnostic only
}
pub async fn insert(pool: &sqlx::SqlitePool, m: CheckpointMetricInsert) -> sqlx::Result<()>;

#[derive(Debug, Clone, PartialEq, sqlx::FromRow, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointReport {
    pub samples: i64,
    pub eligible: i64,             // rows with count_failure = 0
    pub avg_q: f64,
    pub avg_projected_post_tokens: f64,
    pub max_plateau_turns: i64,
    pub count_failures: i64,
}
pub async fn report(pool: &sqlx::SqlitePool, since_hours: i64) -> sqlx::Result<CheckpointReport>;
```

- [ ] **Step 1: write failing test** in `proxy_checkpoint_metric.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::db::connect_in_memory;

    fn row(q: f64, post: i64, plateau: i64, fail: i64) -> CheckpointMetricInsert {
        CheckpointMetricInsert {
            created_at: chrono::Utc::now().to_rfc3339(),
            model: "claude-x".into(),
            earliest_changed_byte: 1_000,
            earliest_changed_msg: 1,
            r_tokens: 400_000,
            gross_candidate_tokens: 90_000,
            stub_overhead_tokens: 200,
            s_net_tokens: (q * 400_000.0) as i64,
            q,
            projected_break_even: if q > 0.0 { 11.5 / q - 12.5 } else { f64::INFINITY },
            projected_post_tokens: post,
            plateau_turns: plateau,
            non_recoverable_kept_tokens: 5_000,
            provider_estimate: 1,
            count_failure: fail,
            method_version: "m1-count_tokens-2023-06-01".into(),
            bytes_est_tokens: 500_000,
        }
    }

    #[tokio::test]
    async fn report_aggregates_and_excludes_failures_from_eligible() {
        let pool = connect_in_memory().await;
        insert(&pool, row(0.8, 340_000, 3, 0)).await.unwrap();
        insert(&pool, row(0.2, 360_000, 0, 1)).await.unwrap(); // count failure
        let r = report(&pool, 24).await.unwrap();
        assert_eq!(r.samples, 2);
        assert_eq!(r.eligible, 1);
        assert_eq!(r.count_failures, 1);
        assert_eq!(r.max_plateau_turns, 3);
    }
}
```

- [ ] **Step 2:** `cargo test proxy_checkpoint_metric` from `src-tauri/` → FAIL (`no such table` / module missing)
- [ ] **Step 3: minimal impl.**

`0021_proxy_checkpoint_metric.sql`:

```sql
CREATE TABLE IF NOT EXISTS proxy_checkpoint_metric (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at TEXT NOT NULL,
    model TEXT NOT NULL,
    earliest_changed_byte INTEGER NOT NULL,
    earliest_changed_msg INTEGER NOT NULL,
    r_tokens INTEGER NOT NULL,
    gross_candidate_tokens INTEGER NOT NULL,
    stub_overhead_tokens INTEGER NOT NULL,
    s_net_tokens INTEGER NOT NULL,
    q REAL NOT NULL,
    projected_break_even REAL NOT NULL,
    projected_post_tokens INTEGER NOT NULL,
    plateau_turns INTEGER NOT NULL,
    non_recoverable_kept_tokens INTEGER NOT NULL,
    provider_estimate INTEGER NOT NULL,
    count_failure INTEGER NOT NULL,
    method_version TEXT NOT NULL,
    bytes_est_tokens INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_proxy_checkpoint_metric_created_at
    ON proxy_checkpoint_metric(created_at);
```

`db.rs` (append after the `version < 20` block, before `tx.commit()`):

```rust
    if version < 21 {
        sqlx::raw_sql(include_str!("migrations/0021_proxy_checkpoint_metric.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 21;")
            .execute(&mut *tx)
            .await?;
    }
```

`proxy_checkpoint_metric.rs` (mirror `proxy_metric.rs`: raw `sqlx::query` INSERT with `?1..?17` — the table has 17 non-`id` columns, bind count MUST equal the migration column count; `report` via `QueryBuilder::<Sqlite>` with `COUNT(*) AS samples`, `SUM(CASE WHEN count_failure = 0 THEN 1 ELSE 0 END) AS eligible`, and every scalar aggregate wrapped in `COALESCE(..., 0)` so an empty table returns zeros instead of NULL (an un-COALESCE'd `AVG`/`MAX`/`SUM` returns NULL on zero rows and panics the `fetch_one` deserialize — regression-tested): `COALESCE(AVG(q),0) AS avg_q`, `COALESCE(AVG(projected_post_tokens),0) AS avg_projected_post_tokens`, `COALESCE(MAX(plateau_turns),0) AS max_plateau_turns`, `COALESCE(SUM(count_failure),0) AS count_failures`, `where_gte("created_at", cutoff)`, `fetch_one::<CheckpointReport, _>`). Use `super::cb_err` for the chain-builder error map, exactly as `proxy_metric::report` does. Add `pub mod proxy_checkpoint_metric;` to `repo/mod.rs`.

- [ ] **Step 4:** `cargo test proxy_checkpoint_metric` → PASS
- [ ] **Step 5:** commit `git add src-tauri/src/engine/migrations/0021_proxy_checkpoint_metric.sql src-tauri/src/engine/repo/proxy_checkpoint_metric.rs src-tauri/src/engine/db.rs src-tauri/src/engine/repo/mod.rs && git commit -m "feat(engine): checkpoint metric contract table + repo (migration 0021)"`

### Task 6: Plateau state in the conversation ledger (ctxopt, pure)

**Files:**
- Modify: `src-tauri/crates/ctxopt/src/ledger.rs` (add two `ConvState` fields + a pure `record_plateau` helper)
- Test path: `ledger.rs` tests

Plateau is *observed across requests*, not an instantaneous field (spec §7.1): the number of subsequent requests that hold the same projected frozen boundary until the next eligible checkpoint or harness compaction. It is pure per-conversation state, so it lives on `ConvState`; the engine wiring (Task 7) records it and reads it back for persistence.

**Interfaces — Produces:** new `ConvState` fields `pub checkpoint_boundary: Option<usize>` and `pub plateau_turns: u32`; and

```rust
/// Fold a newly-observed projected boundary into a conversation's plateau count.
/// Same boundary as last eligible sample → +1; a new/changed boundary → reset to 0.
/// Returns the plateau count AFTER this observation.
pub fn record_plateau(conv: &mut ConvState, boundary: usize) -> u32;
```

- [ ] **Step 1: write failing test** in `ledger.rs` tests:

```rust
    #[test]
    fn plateau_increments_while_boundary_holds_and_resets_on_change() {
        let mut led = Ledger::new(4);
        let idx = led.observe(&conv(&["a"]));
        let c = led.conv_mut(idx);
        assert_eq!(record_plateau(c, 7), 0); // first sight of boundary 7
        assert_eq!(record_plateau(c, 7), 1); // held
        assert_eq!(record_plateau(c, 7), 2); // held
        assert_eq!(record_plateau(c, 9), 0); // boundary moved → reset
        assert_eq!(c.checkpoint_boundary, Some(9));
        assert_eq!(c.plateau_turns, 0);
    }
```

- [ ] **Step 2:** `cargo test -p ctxopt ledger` from `src-tauri/` → FAIL (unknown field `checkpoint_boundary` / missing `record_plateau`)
- [ ] **Step 3: minimal impl.** Add the two fields to `ConvState` and initialize them (`checkpoint_boundary: None, plateau_turns: 0`) at BOTH construction sites: the `None =>` insert arm in `Ledger::observe` and the `conv(..)` test helper in `policy.rs` tests (that helper constructs `ConvState` literally — update it or it will not compile). Then:

```rust
pub fn record_plateau(conv: &mut ConvState, boundary: usize) -> u32 {
    if conv.checkpoint_boundary == Some(boundary) {
        conv.plateau_turns = conv.plateau_turns.saturating_add(1);
    } else {
        conv.checkpoint_boundary = Some(boundary);
        conv.plateau_turns = 0;
    }
    conv.plateau_turns
}
```

- [ ] **Step 4:** `cargo test -p ctxopt` → PASS (whole crate — the `policy.rs` test helper must be updated so it compiles)
- [ ] **Step 5:** commit `git add src-tauri/crates/ctxopt/src/ledger.rs src-tauri/crates/ctxopt/src/policy.rs && git commit -m "feat(ctxopt): observed checkpoint plateau state on ConvState"`

### Task 7: Wire the checkpoint measurement path into ctx_proxy (engine)

> **SECURITY CONTAINMENT (council challenge ea3df57c, Aoki — MANDATORY before this task is READY).**
> - **Immutable upstream capture (no async TOCTOU):** `CheckpointJob` MUST carry the exact `upstream` base string captured for the ORIGINAL forward. `sample_checkpoint` uses `job.upstream` and MUST NOT re-read `state.ctx_proxy.upstream` — a change between forward and sampling could otherwise send credential A to host B.
> - **Schedule after success:** enqueue `sample_checkpoint` ONLY after the original upstream response returns a success status. A failed/aborted forward samples nothing.
> - **Rate/cadence beyond the semaphore:** the 2-permit semaphore bounds concurrency, not rate or credential lifetime. Add a global cooldown/rate-limit (or per-conversation-boundary sampling cadence) so sustained eligible traffic cannot fan out unbounded (each eligible request otherwise triggers 3 count calls). Dropped/rate-limited samples are recorded as a metric, never silently lost.
> - **Required tests (in this task):** (a) mutating `state.ctx_proxy.upstream` between forward and the spawned sample cannot retarget the in-flight credential (the job holds the original upstream); (b) sustained eligible requests respect the cap/cooldown and dropped samples are recorded; (c) sampling is scheduled only after a successful original response.

**Files:**
- Modify: `src-tauri/src/engine/runtime/ctx_proxy.rs` (add global checkpoint atomics to `ProxyRuntime`; add the sampling semaphore; add `credential_from_headers`, the sync pre-gate `checkpoint_gate`, and the async `sample_checkpoint`; call the gate from `forward_inner` after the upstream body is fixed)
- Test path: `ctx_proxy.rs` tests

**Interfaces — Consumes:** `ctxopt::checkpoint::{plan_checkpoint, project, CheckpointOutcome}`, `ctxopt::estimate::est_tokens`, `ctxopt::{DEFAULT_CEILING_TOKENS, RECENT_TAIL_MSGS, MIN_NET_SAVING_TOKENS, LOW_WATER_TOKENS}`, `ctxopt::ledger::record_plateau`, `crate::engine::runtime::count_tokens::{CountCredential, count_tokens, count_tokens_body, prefix_messages, CHECKPOINT_METHOD_VERSION}`, `crate::engine::repo::proxy_checkpoint_metric`. **Produces (new on `ProxyRuntime`):**

```rust
pub checkpoint: AtomicBool,     // global toggle, default false
pub ceiling: AtomicU32,         // C tokens, default DEFAULT_CEILING_TOKENS
pub min_net_saving: AtomicU32,  // M tokens, default MIN_NET_SAVING_TOKENS
pub low_water: AtomicU32,       // L tokens, default LOW_WATER_TOKENS
pub tail_msgs: AtomicU32,       // default RECENT_TAIL_MSGS
sample_permits: Arc<tokio::sync::Semaphore>, // bounded off-path sampling (2 permits)
```

and a sync gate + async sampler:

```rust
/// Pure-ish pre-gate: parse body, plan + project (ctxopt), return an eligible job
/// or None. Reads `body` by reference; produces NO forwardable bytes → cannot alter
/// the forwarded request (Global Constraint: NEVER alter forwarded bytes in M1).
struct CheckpointJob {
    model: String,
    original: Value,
    projected_messages: Value,
    earliest_changed_msg_index: usize,
    earliest_changed_byte: usize,
    gross_candidate_bytes: usize,
    stub_overhead_bytes: usize,
    non_recoverable_kept_bytes: usize,
    projected_post_tokens: usize,
    est_whole_tokens: usize,
}
fn checkpoint_gate(rt: &ProxyRuntime, body: &[u8]) -> Option<CheckpointJob>;
fn credential_from_headers(headers: &HeaderMap) -> CountCredential;
async fn sample_checkpoint(state: Arc<AppState>, cred: CountCredential, job: CheckpointJob);
```

Wiring in `forward_inner`: after `upstream_body` is computed and BEFORE (or after) the upstream send, if `rt.checkpoint.load(Acquire)` and it is a POST `/v1/messages`, call `checkpoint_gate(&state.ctx_proxy, &body)`; on `Some(job)`, capture `credential_from_headers(&parts.headers)`, `try_acquire_owned()` a permit, and `tokio::spawn(sample_checkpoint(...))` (drop-on-full, never await). `upstream_body` is **never** derived from the job. `sample_checkpoint` computes a = count(original), b = count(projected), c = count(prefix) via the T4 client; `S_net = a − b`, `R = a − c`, `q = S_net/R` (guard R==0 → q=0, count_failure semantics); records `record_plateau` against the ledger conversation using `earliest_changed_msg_index` as the boundary; persists via `proxy_checkpoint_metric::insert`. Any error → record `count_failure = 1` with the bytes/4 diagnostic and return (fail-open).

- [ ] **Step 1: write failing tests** in `ctx_proxy.rs` tests (reuse `high_water_request`, `start_fake_upstream`, `start_proxy`; the fake upstream already answers `/v1/messages/count_tokens`):

```rust
    #[test]
    fn checkpoint_gate_off_by_default_and_never_yields_a_forward_body() {
        let rt = ProxyRuntime::with_port(0);
        // Default: checkpoint disabled → no job regardless of size.
        let body = serde_json::to_vec(&high_water_request(560_000)).unwrap();
        assert!(checkpoint_gate(&rt, &body).is_none());
    }

    #[test]
    fn checkpoint_gate_when_enabled_projects_without_touching_the_body() {
        let rt = ProxyRuntime::with_port(0);
        rt.checkpoint.store(true, Ordering::Release);
        rt.ceiling.store(1, Ordering::Release);            // force above-ceiling
        rt.min_net_saving.store(1, Ordering::Release);     // trivial M
        rt.low_water.store(u32::MAX, Ordering::Release);   // trivial L
        rt.tail_msgs.store(2, Ordering::Release);          // push the reads out of the tail
        let body = serde_json::to_vec(&high_water_request(1_000)).unwrap();
        let job = checkpoint_gate(&rt, &body).expect("eligible job");
        // The job carries the ORIGINAL request unchanged for count (a); the
        // projected list is a SEPARATE value — the forwarded body is `body` itself.
        assert_eq!(serde_json::to_vec(&job.original).unwrap().len(), body.len());
        assert!(job.projected_messages != job.original["messages"]);
        assert!(job.earliest_changed_msg_index <= job.original["messages"].as_array().unwrap().len());
    }

    #[tokio::test]
    async fn sample_checkpoint_persists_a_metric_row() {
        let (upstream, up) = start_fake_upstream().await;
        let mut state = AppState::for_tests().await;
        let rt = Arc::new(ProxyRuntime::with_port(0));
        *rt.upstream.write().unwrap() = upstream.clone();
        rt.checkpoint.store(true, Ordering::Release);
        rt.ceiling.store(1, Ordering::Release);
        rt.min_net_saving.store(1, Ordering::Release);
        rt.low_water.store(u32::MAX, Ordering::Release);
        rt.tail_msgs.store(2, Ordering::Release);
        state.ctx_proxy = rt.clone();
        let state = Arc::new(state);
        let body = serde_json::to_vec(&high_water_request(1_000)).unwrap();
        let job = checkpoint_gate(&rt, &body).unwrap();
        let cred = CountCredential { api_key: Some("k".into()), authorization: None, anthropic_version: "2023-06-01".into() };
        sample_checkpoint(state.clone(), cred, job).await;
        let report = crate::engine::repo::proxy_checkpoint_metric::report(&state.db, 24).await.unwrap();
        assert_eq!(report.samples, 1);
        up.abort();
    }
```

- [ ] **Step 2:** `cargo test ctx_proxy` from `src-tauri/` → FAIL (fields/functions missing)
- [ ] **Step 3: minimal impl.** Add the atomics to `ProxyRuntime` and seed them in `with_port` (`checkpoint: AtomicBool::new(false)`, `ceiling: AtomicU32::new(ctxopt::DEFAULT_CEILING_TOKENS as u32)`, `min_net_saving: AtomicU32::new(ctxopt::MIN_NET_SAVING_TOKENS as u32)`, `low_water: AtomicU32::new(ctxopt::LOW_WATER_TOKENS as u32)`, `tail_msgs: AtomicU32::new(ctxopt::RECENT_TAIL_MSGS as u32)`, `sample_permits: Arc::new(tokio::sync::Semaphore::new(2))`). Then:

```rust
fn credential_from_headers(headers: &HeaderMap) -> CountCredential {
    let get = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
    CountCredential {
        api_key: get("x-api-key"),
        authorization: get("authorization"),
        anthropic_version: get("anthropic-version").unwrap_or_else(|| "2023-06-01".to_owned()),
    }
}

fn checkpoint_gate(rt: &ProxyRuntime, body: &[u8]) -> Option<CheckpointJob> {
    if !rt.checkpoint.load(Ordering::Acquire) {
        return None;
    }
    let original: Value = serde_json::from_slice(body).ok()?;
    let messages = original.get("messages")?.clone();
    let model = original.get("model").and_then(Value::as_str).unwrap_or_default().to_owned();
    let est_whole_tokens = ctxopt::estimate::est_tokens(body.len());
    let ceiling = rt.ceiling.load(Ordering::Acquire) as usize;
    let tail = rt.tail_msgs.load(Ordering::Acquire) as usize;
    let plan = ctxopt::checkpoint::plan_checkpoint(&messages, est_whole_tokens, ceiling, tail)?;
    let m = rt.min_net_saving.load(Ordering::Acquire) as usize;
    let l = rt.low_water.load(Ordering::Acquire) as usize;
    // ⚠ SUPERSEDED by R8: `Saturated => None` drops the row and hides the sample.
    // Corrected gate demotes bytes/4 to a trigger and classifies post-count_tokens,
    // always persisting a row. See m1-fix plan.
    match ctxopt::checkpoint::project(&messages, &plan, est_whole_tokens, m, l) {
        ctxopt::checkpoint::CheckpointOutcome::Saturated => None,
        ctxopt::checkpoint::CheckpointOutcome::Eligible(p) => Some(CheckpointJob {
            model,
            original,
            projected_messages: p.projected_messages,
            earliest_changed_msg_index: plan.earliest_changed_msg_index,
            earliest_changed_byte: plan.candidates.first().map_or(0, |c| c.gross_bytes),
            gross_candidate_bytes: p.gross_candidate_bytes,
            stub_overhead_bytes: p.stub_overhead_bytes,
            non_recoverable_kept_bytes: plan.non_recoverable_kept_bytes,
            projected_post_tokens: p.projected_post_tokens,
            est_whole_tokens,
        }),
    }
}
```

`sample_checkpoint` (fail-open; every early return still inserts a `count_failure = 1` row with the bytes/4 diagnostic so a failed count is *recorded*, not silent):

```rust
async fn sample_checkpoint(state: Arc<AppState>, cred: CountCredential, job: CheckpointJob) {
    use crate::engine::runtime::count_tokens as ct;
    let upstream = state.ctx_proxy.upstream.read().unwrap_or_else(|e| e.into_inner()).trim_end_matches('/').to_owned();
    let client = &state.ctx_proxy.client_for_count(); // reuse the reqwest::Client
    let body_a = ct::count_tokens_body(&job.original, &job.original["messages"]);
    let body_b = ct::count_tokens_body(&job.original, &job.projected_messages);
    let prefix = ct::prefix_messages(&job.original["messages"], job.earliest_changed_msg_index);
    let body_c = ct::count_tokens_body(&job.original, &prefix);

    let counts = async {
        let a = ct::count_tokens(client, &upstream, &cred, &body_a).await?;
        let b = ct::count_tokens(client, &upstream, &cred, &body_b).await?;
        let c = ct::count_tokens(client, &upstream, &cred, &body_c).await?;
        Ok::<_, String>((a, b, c))
    }.await;

    let plateau = {
        let mut ledger = state.ctx_proxy.ledger.lock().unwrap_or_else(|e| e.into_inner());
        let idx = ledger.observe(&job.original["messages"]);
        ctxopt::ledger::record_plateau(ledger.conv_mut(idx), job.earliest_changed_msg_index)
    };

    let bytes_est = ctxopt::estimate::est_tokens(job.gross_candidate_bytes.saturating_sub(job.stub_overhead_bytes));
    let mut row = crate::engine::repo::proxy_checkpoint_metric::CheckpointMetricInsert {
        created_at: chrono::Utc::now().to_rfc3339(),
        model: job.model.clone(),
        earliest_changed_byte: saturating_i64(job.earliest_changed_byte as u64),
        earliest_changed_msg: saturating_i64(job.earliest_changed_msg_index as u64),
        r_tokens: 0, gross_candidate_tokens: saturating_i64(ctxopt::estimate::est_tokens(job.gross_candidate_bytes) as u64),
        stub_overhead_tokens: saturating_i64(ctxopt::estimate::est_tokens(job.stub_overhead_bytes) as u64),
        s_net_tokens: 0, q: 0.0, projected_break_even: f64::INFINITY,
        projected_post_tokens: saturating_i64(job.projected_post_tokens as u64),
        plateau_turns: i64::from(plateau),
        non_recoverable_kept_tokens: saturating_i64(ctxopt::estimate::est_tokens(job.non_recoverable_kept_bytes) as u64),
        provider_estimate: 1, count_failure: 0,
        method_version: ct::CHECKPOINT_METHOD_VERSION.to_owned(),
        bytes_est_tokens: saturating_i64(bytes_est as u64),
    };
    match counts {
        Ok((a, b, c)) => {
            let r = a.saturating_sub(c);
            let s_net = a.saturating_sub(b);
            let q = if r == 0 { 0.0 } else { s_net as f64 / r as f64 };
            row.r_tokens = saturating_i64(r);
            row.s_net_tokens = saturating_i64(s_net);
            row.q = q;
            row.projected_break_even = if q > 0.0 { 11.5 / q - 12.5 } else { f64::INFINITY };
            row.projected_post_tokens = saturating_i64(b);
        }
        Err(error) => {
            eprintln!("[ctx-proxy] checkpoint count_tokens failed: {error}");
            row.count_failure = 1;
        }
    }
    if let Err(error) = crate::engine::repo::proxy_checkpoint_metric::insert(&state.db, row).await {
        eprintln!("[ctx-proxy] failed to record checkpoint metric: {error}");
    }
}
```

Add a small accessor `fn client_for_count(&self) -> reqwest::Client { self.client.clone() }` on `ProxyRuntime` (the `reqwest::Client` is an `Arc` internally — cheap to clone). Wire into `forward_inner` after `upstream_body` is set:

```rust
    if parts.method == axum::http::Method::POST && parts.uri.path() == "/v1/messages" {
        if let Some(job) = checkpoint_gate(&state.ctx_proxy, &body) {
            let cred = credential_from_headers(&parts.headers);
            if let Ok(permit) = state.ctx_proxy.sample_permits.clone().try_acquire_owned() {
                let sample_state = state.clone();
                tokio::spawn(async move {
                    let _permit = permit; // released on completion
                    sample_checkpoint(sample_state, cred, job).await;
                });
            } else {
                eprintln!("[ctx-proxy] checkpoint sampler saturated; dropping sample");
            }
        }
    }
```

This block reads `body`/`parts.headers` only and spawns; `upstream_body` (already computed above) is untouched → forwarded bytes are byte-identical.

- [ ] **Step 4:** `cargo test ctx_proxy` → PASS
- [ ] **Step 5:** commit `git add src-tauri/src/engine/runtime/ctx_proxy.rs && git commit -m "feat(engine): log-mode checkpoint measurement wired into ctx_proxy (never alters forwarded bytes)"`

### Task 8: CLI + commands + report surface (engine)

**Files:**
- Modify: `src-tauri/src/engine/commands/proxy.rs` (add `set_checkpoint`, `set_ceiling`, `checkpoint_report`; surface checkpoint state in `status_value`)
- Modify: `src-tauri/src/engine/router.rs` (dispatch `proxy.checkpoint`, `proxy.ceiling`, `proxy.checkpointReport`)
- Modify: `src-tauri/src/engine/commands/cli.rs` (extend `map_proxy_argv`: `checkpoint on|off`, `ceiling <tokens>`, `checkpoint-report [--since-hours N]`; extend the usage string and `src-tauri/src/bin/conclave-cli.rs:131` help line)
- Test path: `proxy.rs` tests + `cli.rs` tests

**Interfaces — Produces (router methods):** `proxy.checkpoint {enabled: bool}`, `proxy.ceiling {tokens: u64}`, `proxy.checkpointReport {sinceHours?: i64}` → `CheckpointReport`. **CLI:** `proxy checkpoint on|off`, `proxy ceiling <tokens>`, `proxy checkpoint-report [--since-hours N]`.

- [ ] **Step 1: write failing tests.** In `proxy.rs` tests:

```rust
    #[tokio::test]
    async fn checkpoint_toggle_and_ceiling_reflected_in_status() {
        let state = AppState::for_tests().await;
        let status = router::dispatch(&state, "proxy.checkpoint", json!({ "enabled": true })).await.unwrap();
        assert_eq!(status["checkpoint"], true);
        assert_eq!(state.ctx_proxy.checkpoint.load(Ordering::Acquire), true);

        let status = router::dispatch(&state, "proxy.ceiling", json!({ "tokens": 400_000 })).await.unwrap();
        assert_eq!(status["ceiling"], 400_000);
        assert_eq!(state.ctx_proxy.ceiling.load(Ordering::Acquire), 400_000);

        let report = router::dispatch(&state, "proxy.checkpointReport", Value::Null).await.unwrap();
        assert_eq!(report["samples"], 0);
    }
```

In `cli.rs` tests (beside the existing `proxy status` assertions near line 3537):

```rust
        assert_eq!(ok_method(&["proxy", "checkpoint", "on"]), "proxy.checkpoint");
        assert_eq!(ok_params(&["proxy", "checkpoint", "on"]), json!({ "enabled": true }));
        assert_eq!(ok_method(&["proxy", "ceiling", "400000"]), "proxy.ceiling");
        assert_eq!(ok_params(&["proxy", "ceiling", "400000"]), json!({ "tokens": 400_000 }));
        assert_eq!(ok_method(&["proxy", "checkpoint-report"]), "proxy.checkpointReport");
```

- [ ] **Step 2:** `cargo test proxy` from `src-tauri/` → FAIL (unknown command / no argv arm)
- [ ] **Step 3: minimal impl.** In `proxy.rs`:

```rust
#[derive(Deserialize)]
struct CheckpointReq { enabled: bool }
#[derive(Deserialize)]
struct CeilingReq { tokens: u64 }

pub async fn set_checkpoint(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: CheckpointReq = serde_json::from_value(payload)
        .map_err(|e| AppError::Invalid(format!("proxy.checkpoint: bad payload: {e}")))?;
    state.ctx_proxy.checkpoint.store(req.enabled, Ordering::Release);
    Ok(status_value(state))
}

pub async fn set_ceiling(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: CeilingReq = serde_json::from_value(payload)
        .map_err(|e| AppError::Invalid(format!("proxy.ceiling: bad payload: {e}")))?;
    let tokens = u32::try_from(req.tokens)
        .map_err(|_| AppError::Invalid("proxy.ceiling: tokens out of range".into()))?;
    state.ctx_proxy.ceiling.store(tokens, Ordering::Release);
    Ok(status_value(state))
}

pub async fn checkpoint_report(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req = if payload.is_null() { ReportReq::default() }
        else { serde_json::from_value::<ReportReq>(payload)
            .map_err(|e| AppError::Invalid(format!("proxy.checkpointReport: bad payload: {e}")))? };
    let since_hours = req.since_hours.unwrap_or(24);
    if since_hours < 0 {
        return Err(AppError::Invalid("proxy.checkpointReport: sinceHours must be non-negative".into()));
    }
    Ok(serde_json::to_value(
        crate::engine::repo::proxy_checkpoint_metric::report(&state.db, since_hours).await?,
    ).expect("CheckpointReport serialization cannot fail"))
}
```

Add to `status_value`'s `json!` object: `"checkpoint": runtime.checkpoint.load(Ordering::Acquire),` and `"ceiling": runtime.ceiling.load(Ordering::Acquire),`. In `router.rs`, beside the existing `proxy.*` arms:

```rust
        "proxy.checkpoint" => proxy::set_checkpoint(state, payload).await,
        "proxy.ceiling" => proxy::set_ceiling(state, payload).await,
        "proxy.checkpointReport" => proxy::checkpoint_report(state, payload).await,
```

In `cli.rs` `map_proxy_argv`, extend the usage string and add arms (mirroring the `report` arm's `take_flag` handling):

```rust
        Some("checkpoint") if argv.len() == 3 => match argv[2].as_str() {
            "on" => Ok(("proxy.checkpoint", json!({ "enabled": true }))),
            "off" => Ok(("proxy.checkpoint", json!({ "enabled": false }))),
            _ => Err(AppError::Invalid(usage.into())),
        },
        Some("ceiling") if argv.len() == 3 => {
            let tokens = argv[2].parse::<u64>().map_err(|_| AppError::Invalid(usage.into()))?;
            Ok(("proxy.ceiling", json!({ "tokens": tokens })))
        }
        Some("checkpoint-report") => {
            let (since_hours, rest) = take_flag(&argv[2..], "--since-hours");
            if !rest.is_empty() {
                return Err(AppError::Invalid(usage.into()));
            }
            let mut params = json!({});
            if let Some(raw) = since_hours {
                let value = raw.parse::<i64>().map_err(|_| AppError::Invalid(
                    "cli: proxy checkpoint-report: --since-hours expects a non-negative integer".into()))?;
                if value < 0 {
                    return Err(AppError::Invalid(
                        "cli: proxy checkpoint-report: --since-hours expects a non-negative integer".into()));
                }
                params["sinceHours"] = json!(value);
            }
            Ok(("proxy.checkpointReport", params))
        }
```

Update the usage literal to `"cli: proxy <status|mode <off|log|rewrite>|threshold <ratio>|checkpoint <on|off>|ceiling <tokens>|report [--since-hours N]|checkpoint-report [--since-hours N]>"` and the help line at `conclave-cli.rs:131` to match.

- [ ] **Step 4:** `cargo test proxy` → PASS; then full `cargo test` + `cargo build` from `src-tauri/` before READY.
- [ ] **Step 5:** commit `git add src-tauri/src/engine/commands/proxy.rs src-tauri/src/engine/router.rs src-tauri/src/engine/commands/cli.rs src-tauri/src/bin/conclave-cli.rs && git commit -m "feat(cli): proxy checkpoint on|off, ceiling, checkpoint-report"`

Note: the live-credential `count_tokens` preflight (Task 4 `preflight`) is the plan prerequisite for the pass-criterion gate — run it against the real Claude credential once before trusting any recorded `q`/plateau; it is exercised in-code by `sample_checkpoint`'s count path but must be confirmed live before the gate is read.
