# Plan: memory retrieval — regression bench first, then hybrid BM25+vector rank

Date: 2026-07-05 · Owner: Detoro bfb737ff · authority: in-loop
Task: `memory-hybrid-bench` · Implementer: Tiësto fd0dec79 · Reviewer: Mellow (LAND, blocking)

## Why

Human approved both adopt recommendations from
`docs/2026-07-05-memory-benchmark-mempalace.md` (task `memory-benchmark`,
merged @ 7b6c3a9): (1) an in-repo retrieval regression bench — today we cannot
answer "did this change hurt retrieval?" with a number; (2) a BM25+vector
hybrid ranker — pure cosine underweights exact tokens (UUIDs, error codes,
crate names), which is exactly the shape of our memories.

ORDERING IS THE POINT: the bench lands FIRST and scores the baseline; the
ranker lands second and must show its improvement as a bench delta, not prose.

## Current retrieval (read before changing)

`src-tauri/src/engine/commands/memory.rs` — `search_with_embedding` (:638) →
`score_cached` (:578): exact brute-force cosine (dot product on L2-normalized
384-dim vectors), bounded-heap top-k, warm LRU per-workspace cache. Test seam:
`remember_with_embedder` / `search_with_embedder` + `FakeEmbedder` (imported
at :921 from `engine::embedder`). Existing tests: `mod tests` at :911.

## Task 1 — retrieval regression bench (commit before Task 2)

- New test module (either inside `commands/memory.rs::tests` or a sibling
  `commands/memory_bench.rs` wired into `mod` — implementer's call) holding a
  FIXED labelled corpus: ~24 facts + ~12 queries with known-relevant ids.
  Seed from the appendix of `docs/2026-07-05-memory-benchmark-mempalace.md`;
  adapt texts so relevance is decidable under `FakeEmbedder`'s deterministic
  vectors AND include exact-token queries (UUID fragments, error codes,
  flag/crate names) whose gold fact shares tokens but not fake-vector
  proximity — those are the cases the hybrid ranker must win.
- Compute R@5 and MRR over the query set; assert them against pinned
  baselines with a comment stating the values are a REGRESSION FLOOR (a
  change may raise them and re-pin; it may not silently lower them).
- Runs under plain `cargo test` (no ONNX model, no network — FakeEmbedder
  only). A real-model bench is OUT of scope.
- Commit Task 1 alone, THEN gate it (evidence pins to HEAD at run time —
  gate an uncommitted tree and the ledger blames the parent SHA).

## Task 2 — hybrid BM25+vector ranker, default on

- In `search_with_embedding`: widen the exact-cosine candidate pull to
  top-N (N = max(50, 4·k), capped at corpus size), then re-score each
  candidate as `0.6·cosine + 0.4·bm25_norm` and return the top-k of that.
  BM25 (Okapi, standard k1=1.2 b=0.75, or a simpler normalized keyword-
  overlap if it demonstrably passes the same bench) computed IN-MODULE over
  the candidate set only — no global keyword index, no new crate deps.
  Tokenization: lowercase + split on non-alphanumerics; keep it dumb and
  deterministic.
- Deterministic: same store + same query ⇒ same ranking, ties broken by a
  stable key (e.g. chunk id) so tests never flake.
- The returned `score` stays a single f32 (the fused value) — CLI/JSON
  surface (`conclave memory search` output shape) must not change.
- The bench from Task 1 must show the exact-token queries improving (rank of
  gold fact rises) and the semantic queries not regressing; re-pin baselines
  upward in the same commit with the delta stated in the commit message.
- Update the module doc comment at the top of `commands/memory.rs` (it
  currently describes pure cosine) to describe the two-stage rank.

## Boundary

`src-tauri/src/engine/commands/memory.rs` and (optionally) one new sibling
module file under `src-tauri/src/engine/commands/`. NOTHING else — no
`repo/memory.rs` schema change, no CLI surface change, no TS/UI.

## Gates (commit first, then gate; via wrapper script — see risk ledger)

- `cargo test` (full, from src-tauri) after EACH task's commit.
- `cargo clippy --all-targets -- -D warnings` after Task 2.
- Mellow LAND review before merge (blocking): bench actually discriminates
  (delete the hybrid term → bench fails), determinism, no dep added,
  baseline re-pin justified by per-query deltas.

## Risk ledger

- `task gate` currently mis-parses argv words containing spaces (defect,
  being fixed in parallel lane `gate-argv-quoting`): wrap any gate command
  whose path has spaces in a script and gate the script path. Plain `cargo
  test` etc. are unaffected. Run cargo gates from a dir inside the cargo
  project (worktree root has no Cargo.toml → exit 101 red herring).
- FakeEmbedder vectors are deterministic but NOT semantic — corpus design
  must make vector-relevance explicit (near-identical texts) rather than
  assuming meaning. If a probe case can't be expressed under FakeEmbedder,
  drop it and note it; do not add the ONNX model to tests.
- 0.6/0.4 weights are MemPalace's defaults, adopted as a starting point —
  if the bench argues for different weights, change them and record why in
  the task notes; the weights are constants, not config surface, for now.
- Shared checkout rules if not using a lane worktree: commit with explicit
  pathspec only (convention:shared-tree-git).
