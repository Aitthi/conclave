//! Pure, deterministic checkpoint projection for the Conclave ctx-proxy.
//! Milestone-1: LOG MODE ONLY — it measures what a checkpoint *would* do and
//! never alters forwarded bytes. serde_json only (crate purity).

use serde_json::{json, Value};
use std::collections::HashMap;

use crate::estimate::est_tokens;
use crate::request::{index_tools, ToolCall};

/// Read-only, re-runnable-for-current-state tools whose historical output may be
/// stubbed and re-obtained on demand. Everything else (side-effecting, drifting,
/// mutating) and every unknown name is kept verbatim (fail-safe).
pub fn is_recoverable(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "Read" | "Grep" | "Glob" | "LS" | "WebSearch" | "NotebookRead"
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointCandidate {
    pub tool_use_id: String,
    pub tool_name: String,
    pub msg_idx: usize,
    pub gross_bytes: usize,
    pub stub: String,
    pub stub_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointPlan {
    pub candidates: Vec<CheckpointCandidate>,
    pub earliest_changed_msg_index: usize,
    pub tail_start: usize,
    pub non_recoverable_kept_bytes: usize,
}

fn candidate_path(call: &ToolCall) -> Option<String> {
    call.input
        .get("file_path")
        .or_else(|| call.input.get("notebook_path"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn breadcrumb(tool: &str, path: Option<&str>, turn: usize) -> String {
    match path {
        Some(p) => {
            format!("[ctxopt checkpoint: elided {tool} {p} @turn {turn} — re-read to restore]")
        }
        None => format!("[ctxopt checkpoint: elided {tool} @turn {turn} — re-read to restore]"),
    }
}

fn content_bytes(text: &str) -> usize {
    json!([{ "type": "text", "text": text }]).to_string().len()
}

/// None when the estimate is at/below the ceiling (no checkpoint considered).
/// Some(plan) when above ceiling; `candidates` may be empty (Task 3 rules it saturated).
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
    let call_by_id: HashMap<&str, &ToolCall> = calls.iter().map(|c| (c.id.as_str(), c)).collect();

    let mut candidates: Vec<CheckpointCandidate> = Vec::new();
    let mut non_recoverable_kept_bytes = 0usize;

    for r in &results {
        if r.msg_idx >= tail_start {
            continue; // verbatim recent tail
        }
        let Some(text) = r.text.as_deref() else {
            continue;
        };
        let Some(call) = call_by_id.get(r.tool_use_id.as_str()) else {
            continue;
        };
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Projection {
    pub projected_messages: Value,
    pub gross_candidate_bytes: usize,
    pub stub_overhead_bytes: usize,
    pub net_saved_bytes: usize,
    pub net_saved_tokens: usize,
    pub projected_post_tokens: usize,
}

/// Real-token checkpoint classification, decided AFTER `count_tokens` (R8,
/// design §4/§7.1). `bytes/4` is only a sample trigger + recorded diagnostic —
/// it must never gate M/L, because it overstates real tokens ~4.7× on
/// cache-heavy traffic and would drop every real conversation as saturated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointClass {
    BelowCeiling,
    Eligible,
    Saturated,
}

/// Classify a sampled checkpoint on REAL token counts. `a` = count(original),
/// `b` = count(projected); `ceiling`/`m`/`l` are all real-token thresholds.
/// - `BelowCeiling` when the byte trigger was a false positive (`a <= ceiling`).
/// - `Eligible` when net saving `a - b > m` AND projected post `b <= l`.
/// - `Saturated` otherwise (near-miss; still recorded so its distribution shows).
pub fn classify(a: usize, b: usize, ceiling: usize, m: usize, l: usize) -> CheckpointClass {
    if a <= ceiling {
        return CheckpointClass::BelowCeiling;
    }
    if a.saturating_sub(b) > m && b <= l {
        CheckpointClass::Eligible
    } else {
        CheckpointClass::Saturated
    }
}

/// Build the projected message list + byte-space diagnostics for a plan.
/// `Some(Projection)` when the plan has candidates, `None` otherwise. Carries NO
/// M/L gate — that is `classify`'s job, on real tokens (R8). `est_whole_tokens`
/// is the bytes/4 diagnostic used only for the `projected_post_tokens` estimate.
pub fn build_projection(
    messages: &Value,
    plan: &CheckpointPlan,
    est_whole_tokens: usize,
) -> Option<Projection> {
    if plan.candidates.is_empty() {
        return None;
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

    Some(Projection {
        projected_messages: projected,
        gross_candidate_bytes,
        stub_overhead_bytes,
        net_saved_bytes,
        net_saved_tokens,
        projected_post_tokens,
    })
}

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
        for t in [
            "Bash",
            "WebFetch",
            "Write",
            "Edit",
            "MultiEdit",
            "NotebookEdit",
            "Task",
        ] {
            assert!(!is_recoverable(t), "{t} must not be recoverable");
        }
    }

    #[test]
    fn unknown_tool_defaults_to_not_recoverable() {
        assert!(!is_recoverable("Conclave"));
        assert!(!is_recoverable("mcp__whatever__do"));
        assert!(!is_recoverable(""));
    }

    use serde_json::{json, Value};

    fn tool_pair(id: &str, name: &str, input: Value, text: &str) -> [Value; 2] {
        [
            json!({"role":"assistant","content":[{"type":"tool_use","id":id,"name":name,"input":input}]}),
            json!({"role":"user","content":[{"type":"tool_result","tool_use_id":id,"content":[{"type":"text","text":text}]}]}),
        ]
    }
    fn filler(n: usize) -> Vec<Value> {
        (0..n)
            .map(|i| json!({"role":"user","content":[{"type":"text","text":format!("f{i}")}]}))
            .collect()
    }
    fn msgs(pairs: Vec<[Value; 2]>, fill: usize) -> Value {
        let mut out: Vec<Value> = pairs.into_iter().flatten().collect();
        out.extend(filler(fill));
        Value::Array(out)
    }

    #[test]
    fn below_ceiling_returns_none() {
        let m = msgs(
            vec![tool_pair(
                "t1",
                "Read",
                json!({"file_path":"/a.rs"}),
                &"x".repeat(700),
            )],
            40,
        );
        assert!(plan_checkpoint(&m, 100, 450_000, 15).is_none());
    }

    #[test]
    fn selects_recoverable_in_frozen_region_and_pairs_tool_name() {
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
        let ids: Vec<&str> = plan
            .candidates
            .iter()
            .map(|c| c.tool_use_id.as_str())
            .collect();
        assert_eq!(ids, ["t1", "t2"]); // Bash excluded
        assert_eq!(plan.candidates[0].tool_name, "Read");
        assert!(plan.candidates[0].stub.contains("Read"));
        assert!(plan.candidates[0].stub.contains("/a.rs"));
        assert_eq!(plan.earliest_changed_msg_index, 1); // t1 result is message #1
        assert!(plan.non_recoverable_kept_bytes > 0); // the Bash result
        assert!(plan.candidates[0].gross_bytes > plan.candidates[0].stub_bytes);
    }

    // --- classify (R8): M/L decided on REAL tokens, three buckets ---

    #[test]
    fn classify_below_ceiling_when_real_tokens_at_or_under_ceiling() {
        // Byte trigger fired (est > ceiling) but real a <= ceiling → false positive.
        assert_eq!(
            classify(100_000, 0, 100_000, 40_000, 350_000),
            CheckpointClass::BelowCeiling
        ); // a == ceiling boundary
        assert_eq!(
            classify(50_000, 10_000, 100_000, 40_000, 350_000),
            CheckpointClass::BelowCeiling
        );
    }

    #[test]
    fn classify_eligible_when_net_over_m_and_post_at_or_under_l() {
        // a=500k, b=300k → net=200k > M=40k; post b=300k <= L=350k.
        assert_eq!(
            classify(500_000, 300_000, 100_000, 40_000, 350_000),
            CheckpointClass::Eligible
        );
        // b == L is the eligible boundary (<=).
        assert_eq!(
            classify(500_000, 350_000, 100_000, 40_000, 350_000),
            CheckpointClass::Eligible
        );
    }

    #[test]
    fn classify_saturated_when_net_at_or_below_m() {
        // a-b == M exactly is NOT > M → saturated (b<=L isolates the M failure).
        assert_eq!(
            classify(390_000, 350_000, 100_000, 40_000, 350_000),
            CheckpointClass::Saturated
        );
    }

    #[test]
    fn classify_saturated_when_post_over_l() {
        // Net big enough (a-b=140k > M) but projected post b > L.
        assert_eq!(
            classify(500_000, 360_000, 100_000, 40_000, 350_000),
            CheckpointClass::Saturated
        );
        // b just one token over L.
        assert_eq!(
            classify(500_000, 350_001, 100_000, 40_000, 350_000),
            CheckpointClass::Saturated
        );
    }

    // --- build_projection (R8): projection + byte diagnostics, no M/L gate ---

    #[test]
    fn build_projection_some_when_candidates_present() {
        let big = "x".repeat(8000);
        let m = msgs(
            vec![
                tool_pair("t1", "Read", json!({"file_path":"/a.rs"}), &big),
                tool_pair("t2", "Read", json!({"file_path":"/b.rs"}), &big),
            ],
            40,
        );
        let plan = plan_checkpoint(&m, 500_000, 450_000, 15).unwrap();
        let p = build_projection(&m, &plan, 500_000).expect("candidates present");
        assert!(p.net_saved_tokens > 0);
        assert!(p.gross_candidate_bytes > p.stub_overhead_bytes);
        assert_ne!(p.projected_messages, m);
        assert!(p.projected_messages[1]["content"][0]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("[ctxopt checkpoint:"));
        // Byte-space diagnostic: post = whole − net_saved (all in bytes/4 space).
        assert_eq!(p.projected_post_tokens, 500_000 - p.net_saved_tokens);
    }

    #[test]
    fn build_projection_none_when_no_candidates() {
        let big = "x".repeat(8000);
        let m = msgs(
            vec![tool_pair("t1", "Read", json!({"file_path":"/a.rs"}), &big)],
            2,
        ); // the single result sits in the verbatim tail → no candidates
        let plan = plan_checkpoint(&m, 500_000, 450_000, 15).unwrap();
        assert!(plan.candidates.is_empty());
        assert!(build_projection(&m, &plan, 500_000).is_none());
    }

    #[test]
    fn build_projection_is_deterministic() {
        let big = "x".repeat(4000);
        let m = msgs(
            vec![
                tool_pair("t1", "Read", json!({"file_path":"/a.rs"}), &big),
                tool_pair("t2", "Grep", json!({"pattern":"foo"}), &big),
            ],
            40,
        );
        let plan = plan_checkpoint(&m, 500_000, 450_000, 15).unwrap();
        assert_eq!(
            build_projection(&m, &plan, 500_000),
            build_projection(&m, &plan, 500_000)
        );
    }

    #[test]
    fn recent_tail_is_never_a_candidate() {
        let big = "x".repeat(2000);
        let m = msgs(
            vec![tool_pair("t1", "Read", json!({"file_path":"/a.rs"}), &big)],
            2,
        );
        let plan = plan_checkpoint(&m, 500_000, 450_000, 15).expect("above ceiling");
        assert!(plan.candidates.is_empty());
        assert_eq!(plan.earliest_changed_msg_index, plan.tail_start);
    }
}
