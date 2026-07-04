//! Retrieval regression bench for workspace memory search.
//!
//! A FIXED, labelled corpus (24 facts, 12 queries with known-relevant ids)
//! scored end-to-end through the real [`search_with_embedder`] path under the
//! deterministic [`FakeEmbedder`] — no ONNX model, no network. It exists so a
//! change to the ranker can be judged by a NUMBER (R@5, MRR) instead of prose.
//!
//! ## Why the corpus looks the way it does
//!
//! [`FakeEmbedder`] seeds each vector from a hash of the WHOLE text string
//! (fixed-key `DefaultHasher`, so it is deterministic across machines and runs).
//! Only *identical* text yields cosine ≈ 1.0; any token change gives an
//! uncorrelated, noisy vector. The corpus exploits exactly that:
//!
//! - **Semantic queries** (`q01`–`q06`): the query text equals its gold fact's
//!   text, so pure cosine already ranks the gold fact #1. These guard against
//!   regression — a ranker change must not knock them off the top.
//! - **Exact-token queries** (`q07`–`q12`): the query shares a rare token
//!   (`sqlsafestr`, `rfc3339`, `argv`, `q8`, `nfc`, an error phrase) with its
//!   gold fact but is NOT the full fact text, so their vectors are uncorrelated
//!   and pure cosine ranks the gold fact by noise. These are the cases a
//!   BM25+vector hybrid ranker must WIN — its keyword term lifts the gold fact.
//! - **Distractors** (`f13`–`f24`): unrelated facts that deliberately avoid the
//!   rare tokens above, so they fill the candidate pool without competing.
//!
//! The pinned metrics below are a REGRESSION FLOOR: a change may raise them and
//! re-pin higher; it may not silently lower them.

#![cfg(test)]

use super::memory::{remember_with_embedder, search_with_embedder, MemorySearchCache};
use crate::engine::{repo::workspace, runtime::embedder::FakeEmbedder, AppState};
use serde_json::json;
use std::{collections::HashMap, sync::Arc};

/// Fake-embedder dimension for the bench. Larger than the 8 used by the unit
/// tests so noise cosines cluster near zero and the semantic exact matches sit
/// cleanly at 1.0 — keeps the baseline stable and the delta legible.
const DIMENSION: usize = 32;
/// Depth searched per query. R@5 reads the first five; MRR is MRR@`EVAL_DEPTH`.
const EVAL_DEPTH: usize = 10;

/// `(fid, text)` — the fixed corpus. `f01`–`f06` are semantic gold facts (a
/// query equals one verbatim), `f07`–`f12` carry a rare token a query shares,
/// `f13`–`f24` are distractors.
const FACTS: &[(&str, &str)] = &[
    // --- Semantic gold facts (query == this text) ---
    (
        "f01",
        "The Conclave data layer uses sqlx with sqlite and a chain builder pattern rather than rusqlite.",
    ),
    (
        "f02",
        "Conclave application UI copy is written in English while replies to the user stay in Thai.",
    ),
    (
        "f03",
        "Do not ship speculative defensive fixes for a bug you have not reproduced with a live repro.",
    ),
    (
        "f04",
        "Act as lead by checking the agent roster and the blackboard before starting any solo work.",
    ),
    (
        "f05",
        "A shared tree commit needs an explicit pathspec so it does not sweep the whole shared index.",
    ),
    (
        "f06",
        "The memory search cache invalidates the whole workspace generation on every successful write.",
    ),
    // --- Exact-token gold facts (share a rare token with a query) ---
    (
        "f07",
        "sqlx 0.9 query_as and query require SqlSafeStr which is only implemented for a static str and not for a owned heap value.",
    ),
    (
        "f08",
        "Inter agent messaging needs the full agent identifier and a short prefix fails with target instance id not found.",
    ),
    (
        "f09",
        "Never sort RFC3339 timestamp strings lexicographically because chrono emits variable fractional second digit widths.",
    ),
    (
        "f10",
        "The task gate command currently misparses argv words that contain spaces which is a known quoting defect.",
    ),
    (
        "f11",
        "The production embedder is fastembed with the quantized minilm q8 model at 384 dimensions.",
    ),
    (
        "f12",
        "Content hash dedup normalizes text to nfc so composed and decomposed unicode forms collapse to one chunk.",
    ),
    // --- Distractors ---
    (
        "f13",
        "The knowledge graph view mounts as an absolute inset zero center pane with a floating blurred header.",
    ),
    (
        "f14",
        "Every full pane view carries an icon tile and a title and a subtitle and a right side close button.",
    ),
    (
        "f15",
        "The stall engine pages a task owner when a claimed lane goes quiet for thirty minutes.",
    ),
    (
        "f16",
        "A challenge has four parts the claim the evidence a proposed resolution and a default.",
    ),
    (
        "f17",
        "Lane worktrees live under a hidden claude worktrees folder and use a branch named per lane.",
    ),
    (
        "f18",
        "The blackboard is for durable facts and is not a chat log or a work tracker.",
    ),
    (
        "f19",
        "Related edges in the graph use a cosine similarity threshold of point four five per node.",
    ),
    (
        "f20",
        "Memory recall should happen before research so a paid fact is not derived a second time.",
    ),
    (
        "f21",
        "The human instructions always outrank a peer agent request inside the shared workspace.",
    ),
    (
        "f22",
        "One worktree and one branch per implementer and the integrator owns the final merge.",
    ),
    (
        "f23",
        "A gate event records the exit code and the head commit and the output tail.",
    ),
    (
        "f24",
        "Subagents that you dispatch report to you and you are their escalation target.",
    ),
];

/// Whether a query is expected to already rank #1 under pure cosine
/// (`Semantic`) or to depend on the keyword term to rank well (`ExactToken`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum QueryKind {
    Semantic,
    ExactToken,
}

/// `(qid, query text, gold fid, kind)`.
const QUERIES: &[(&str, &str, &str, QueryKind)] = &[
    // Semantic: query text is the gold fact verbatim.
    (
        "q01",
        "The Conclave data layer uses sqlx with sqlite and a chain builder pattern rather than rusqlite.",
        "f01",
        QueryKind::Semantic,
    ),
    (
        "q02",
        "Conclave application UI copy is written in English while replies to the user stay in Thai.",
        "f02",
        QueryKind::Semantic,
    ),
    (
        "q03",
        "Do not ship speculative defensive fixes for a bug you have not reproduced with a live repro.",
        "f03",
        QueryKind::Semantic,
    ),
    (
        "q04",
        "Act as lead by checking the agent roster and the blackboard before starting any solo work.",
        "f04",
        QueryKind::Semantic,
    ),
    (
        "q05",
        "A shared tree commit needs an explicit pathspec so it does not sweep the whole shared index.",
        "f05",
        QueryKind::Semantic,
    ),
    (
        "q06",
        "The memory search cache invalidates the whole workspace generation on every successful write.",
        "f06",
        QueryKind::Semantic,
    ),
    // Exact-token: a rare shared token, but not the full fact text.
    ("q07", "SqlSafeStr static str not an owned value", "f07", QueryKind::ExactToken),
    ("q08", "target instance id not found for full agent identifier", "f08", QueryKind::ExactToken),
    ("q09", "RFC3339 chrono fractional second digit widths", "f09", QueryKind::ExactToken),
    ("q10", "task gate argv quoting defect with spaces", "f10", QueryKind::ExactToken),
    ("q11", "fastembed quantized minilm q8 model", "f11", QueryKind::ExactToken),
    ("q12", "nfc dedup composed decomposed unicode", "f12", QueryKind::ExactToken),
];

struct QueryOutcome {
    qid: &'static str,
    kind: QueryKind,
    /// Zero-based rank of the gold fact in the returned hits, or `None` if it
    /// fell outside `EVAL_DEPTH`.
    rank: Option<usize>,
}

struct BenchResult {
    r_at_5: f64,
    mrr: f64,
    per_query: Vec<QueryOutcome>,
}

/// Seed the fixed corpus into a fresh workspace and score every query through
/// the real search path. Deterministic: same corpus + same embedder ⇒ same
/// ranks, so the metrics below are safe to pin.
async fn run_bench() -> BenchResult {
    let state = AppState::for_tests().await;
    let cache = Arc::new(MemorySearchCache::new());
    let workspace_id = workspace::create(&state.db, "memory-bench", "/tmp/memory-bench", None)
        .await
        .expect("create bench workspace")
        .id;

    let mut fid_to_id: HashMap<&'static str, String> = HashMap::new();
    for (fid, text) in FACTS {
        let remembered = remember_with_embedder(
            &state,
            json!({ "workspaceId": workspace_id, "text": text }),
            Arc::new(FakeEmbedder::new(DIMENSION)),
            Arc::clone(&cache),
        )
        .await
        .expect("seed fact");
        assert_eq!(
            remembered["deduped"], false,
            "corpus fact {fid} collided with an earlier fact — texts must be unique"
        );
        fid_to_id.insert(fid, remembered["id"].as_str().expect("fact id").to_owned());
    }

    let mut per_query = Vec::with_capacity(QUERIES.len());
    let mut recall_hits = 0usize;
    let mut reciprocal_rank_sum = 0.0f64;
    for (qid, query, gold_fid, kind) in QUERIES {
        let gold_id = fid_to_id.get(gold_fid).expect("gold fact seeded");
        let result = search_with_embedder(
            &state,
            json!({ "workspaceId": workspace_id, "query": query, "limit": EVAL_DEPTH }),
            Arc::new(FakeEmbedder::new(DIMENSION)),
            Arc::clone(&cache),
        )
        .await
        .expect("search query");
        let hits = result["hits"].as_array().expect("hits array");
        let rank = hits
            .iter()
            .position(|hit| hit["id"].as_str() == Some(gold_id.as_str()));
        if let Some(rank) = rank {
            if rank < 5 {
                recall_hits += 1;
            }
            reciprocal_rank_sum += 1.0 / (rank as f64 + 1.0);
        }
        per_query.push(QueryOutcome {
            qid,
            kind: *kind,
            rank,
        });
    }

    BenchResult {
        r_at_5: recall_hits as f64 / QUERIES.len() as f64,
        mrr: reciprocal_rank_sum / QUERIES.len() as f64,
        per_query,
    }
}

// --- Pinned regression floors -------------------------------------------------
//
// Hybrid BM25+vector floor (Task 2). The keyword stage lifts every exact-token
// gold fact to rank #1 — R@5 0.50 → 1.00, MRR 0.5104 → 1.00 vs the pure-cosine
// baseline (Task 1). Pinned at the ceiling: any ranker that drops an
// exact-token query back below the top five (e.g. deleting the BM25 term)
// fails this floor. Do not lower without a per-query justification recorded in
// the task notes.
const BASELINE_R_AT_5: f64 = 1.0;
const BASELINE_MRR: f64 = 1.0;

#[tokio::test]
async fn retrieval_regression_floor() {
    let result = run_bench().await;

    // Print the discovered numbers so the baseline can be pinned / a delta read.
    eprintln!(
        "[memory-bench] R@5={:.4} MRR={:.4}",
        result.r_at_5, result.mrr
    );
    for outcome in &result.per_query {
        eprintln!(
            "[memory-bench]   {} {:?} rank={:?}",
            outcome.qid, outcome.kind, outcome.rank
        );
    }

    // Structural sanity: every semantic query must rank its gold fact #1 under
    // any correct ranker (cosine == 1.0 for an identical-text match).
    for outcome in &result.per_query {
        if outcome.kind == QueryKind::Semantic {
            assert_eq!(
                outcome.rank,
                Some(0),
                "semantic query {} must rank its gold fact first",
                outcome.qid
            );
        }
    }

    // Discrimination: the hybrid contract is that every exact-token query
    // retrieves its gold fact into the top five. Under pure cosine these rank
    // by noise (five of six fall outside the top ten) — so deleting the BM25
    // term fails this assertion. This is what makes the bench a real guard and
    // not a rubber stamp.
    for outcome in &result.per_query {
        if outcome.kind == QueryKind::ExactToken {
            assert!(
                matches!(outcome.rank, Some(rank) if rank < 5),
                "exact-token query {} must retrieve its gold fact into the top five \
                 (hybrid keyword stage); got rank {:?}",
                outcome.qid,
                outcome.rank
            );
        }
    }

    assert!(
        result.r_at_5 >= BASELINE_R_AT_5,
        "R@5 {:.4} regressed below floor {:.4}",
        result.r_at_5,
        BASELINE_R_AT_5
    );
    assert!(
        result.mrr >= BASELINE_MRR,
        "MRR {:.4} regressed below floor {:.4}",
        result.mrr,
        BASELINE_MRR
    );
}
