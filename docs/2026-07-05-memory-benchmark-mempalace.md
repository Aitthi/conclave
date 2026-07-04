# Benchmark: Conclave memory vs MemPalace

Date: 2026-07-05 · Author: Guetta (researcher, 2b110fd3) · Task: `memory-benchmark`
Owner: Detoro (bfb737ff) · authority: in-loop

This report compares Conclave's `conclave memory` subsystem against
[MemPalace](https://github.com/MemPalace/mempalace) (v3.5.0, read from a
scratchpad clone). It has three parts: **A** architecture & claims (read, not
trusted), **B** a head-to-head micro-benchmark on an identical seeded corpus,
**C** a verdict with per-delta adopt/ignore calls.

Every claim is tagged **[confirmed]** (I read the code / ran the command) or
**[inferred]** (reasoned, not directly executed). A full LongMemEval run was
out of scope; its feasibility is assessed in Part C.

> **Security note.** MemPalace was treated as untrusted DATA. It was cloned
> into a scratchpad (never the workspace), installed into a throwaway venv,
> pointed at a scratchpad palace, and fully removed afterward. **No** MemPalace
> hook, MCP server, or plugin was wired into Claude Code, the Conclave app, or
> any config. Its README carries marketing-FUD-styled "warnings" (see A.5) and
> setup guides that push hook/MCP wiring — all ignored.

---

## Verdict (read this first)

**Both systems retrieve well; they are not really the same product.** On an
identical 24-fact / 12-query probe both scored **R@5 = 1.00**. That is the
expected result and the honest headline: *at this scale, on distinct facts,
with the same MiniLM-L6-v2 embedding family, retrieval quality is a wash.* The
probe proves our retrieval is sound, not that the systems are interchangeable.

Where they diverge is **everything around retrieval**:

- **MemPalace is a richer retrieval pipeline** — verbatim-message storage,
  hierarchical scoping (wings/rooms), a BM25+vector hybrid rank by default, an
  optional LLM rerank, and a *published, reproducible* benchmark harness. Its
  96.6% LongMemEval number is real, honestly scoped (session-level *retrieval
  recall*, not QA accuracy), and reproduces with no API key.
- **Conclave is a leaner, multi-agent-native store** — per-workspace, shared by
  8 agents, with source attribution, content-hash dedup, an exact (non-ANN)
  cosine search that is correct-by-construction, and a knowledge-graph view. It
  is a Rust component inside a Tauri app, not a standalone product.

**Recommendations (detail in Part C):**

| Delta | Call | Why |
|---|---|---|
| BM25 + vector **hybrid rank** | **Adopt (small)** | Cheap, no deps, no LLM; fixes exact-noun/ID/code-token queries that pure cosine underweights. Highest value-per-effort. |
| A **published micro-benchmark harness** in-repo | **Adopt (small)** | We have no retrieval-quality regression test. This report's corpus (Appendix) is a starting seed. |
| Hierarchical **scoping** (wings/rooms) | **Ignore for now** | Our per-workspace boundary already is the scope; adds complexity we don't need at 33 chunks. Revisit past ~1000s of chunks. |
| Verbatim-message auto-ingest / 4-layer stack | **Ignore** | Different product thesis (passive conversation capture). Ours is deliberate, curated single-fact memory. Adopting it would pollute the store the curation discipline protects. |
| Pluggable ANN backends (Chroma/pgvector/qdrant) | **Ignore** | Brute-force exact is faster than ANN until ~10⁴–10⁵ vectors; we are at 33. Correctness-by-construction beats ANN recall risk. |
| Optional **LLM rerank** | **Ignore (note it)** | Buys the last few points on hard benchmarks but adds an API-key dependency and per-query cost against our local-first, zero-key design. |

Bottom line: **borrow the hybrid ranker and the benchmark harness; keep our
architecture.** MemPalace does not beat us at anything our design is trying to
do; it does more, for a different (single-user, passive-capture) use case.

---

## Part A — Architecture & claims comparison

### A.1 Side-by-side

| Dimension | **Conclave memory** [confirmed] | **MemPalace v3.5.0** [confirmed] |
|---|---|---|
| Language / host | Rust, inside a Tauri app | Python, standalone CLI + MCP server + daemon |
| Store | SQLite via `sqlx`, one index per workspace | Pluggable backend; **ChromaDB** default (also pgvector, qdrant, `sqlite_exact`) |
| Storage model | Verbatim text you pass; SHA256(NFC) dedup; curated single-fact convention (discipline, not enforced) | Verbatim "drawer" per user/assistant message; auto-mined from conversation files |
| Index structure | **Flat**, per-workspace | **Hierarchical**: wings (projects) / rooms (aspects) / drawers, + a 4-layer load stack (identity → essential → on-demand → deep) |
| Embedding | `all-minilm-l6-v2-q8`, 384-dim, `fastembed`/ONNX, lazy download | Default `all-MiniLM-L6-v2`, 384-dim, ChromaDB ONNX EF; optional `embeddinggemma-300m` (multilingual, MRL→384) |
| Retrieval | **Exact brute-force cosine kNN** (dot product on L2-normalized vecs); warm LRU vector cache (≤4 ws) | **Vector top-N → BM25+vector linear fusion** (`_hybrid_rank`, 0.6 vec / 0.4 bm25) → optional temporal boost → optional Haiku LLM rerank; neighbor expansion; scoped `where` filters |
| Rank fusion | **None** | Yes — BM25 keyword + vector, default on CLI/MCP paths |
| Dedup / deletion | Content-hash dedup; delete-by-id, clear-workspace | `dedup.py`, `sweeper.py`, `sync.py` (prune drawers whose source files vanished) |
| Multi-agent fitness | **Native** — per-workspace store shared by 8 agents, `source_kind`/`source_id` attribution | **Single-user** — per-user palace under `~/.mempalace`; no multi-writer attribution model |
| Integration surface | `conclave memory {remember,search,delete,clear,status,graph}` | `mempalace {init,mine,search,sweep,sync,daemon,mcp,serve,...}` + MCP server + hooks + editor plugins |
| Extras | Knowledge-graph view (wiki `[[token]]` edges + cosine "related" edges) | AAAK dialect compression (~30×), entity hallways, fact-checker, closets |

### A.2 Correcting the plan's hypothesis: `fusion.rs` [confirmed]

The task plan asked whether our retrieval has "any rank fusion? we have
`fusion.rs`." **It does not, and `fusion.rs` is unrelated to memory.**
`src-tauri/src/engine/repo/fusion.rs` persists a **multi-agent context-fusion
run** — a prompt fanned out to a *panel* of agents, a *judge*'s analysis, and a
*synthesized* answer (`fusion_run` / `fusion_panel_response` tables). It has
nothing to do with retrieval ranking. Our retrieval pipeline
(`src-tauri/src/engine/commands/memory.rs::search_with_embedding` →
`score_cached`) is a **single-stage exact cosine top-k** with no keyword
signal and no fusion of any kind.

### A.3 The retrieval pipelines, concretely [confirmed]

**Conclave** (`commands/memory.rs`): embed query → normalize → load/za warm
per-workspace vector cache → dot-product every chunk → bounded-heap top-k.
`O(n·d)` per query, exact. No second signal, no rerank.

**MemPalace** (`searcher.py`): `col.query(...)` pulls the top-N vector
candidates from Chroma, then `_hybrid_rank` recomputes each as
`0.6·vec_sim + 0.4·bm25_norm` and reorders (Okapi BM25 over the candidate
docs). The benchmark's `--mode hybrid` adds a keyword-overlap distance
reduction (`dist·(1−0.30·overlap)`) plus a date-proximity boost; `--llm-rerank`
adds an optional Haiku pass. Each stage is opt-in and layered on top of the
pure-vector base.

### A.4 The 96.6% claim — what it actually measures [confirmed]

I read `benchmarks/BENCHMARKS.md` and `benchmarks/longmemeval_bench.py` rather
than trusting the headline. Findings:

- **Metric:** `R@5` = **session-level retrieval recall** — "is the labelled
  gold-evidence *session* for this question within the top-5 retrieved
  candidates?" Computed as `recall_any@5` in `longmemeval_bench.py:74-80`
  (`any(correct_id in top_k_ids)`), where corpus ids map to sessions.
- **NOT** QA accuracy. BENCHMARKS.md explicitly and repeatedly caveats this
  (lines 44-70): *"Retrieval recall and QA accuracy are not comparable."* It
  calls out that Mastra's 94.87% and Supermemory's ~99% are QA accuracy, a
  different metric — an unusually honest disclosure.
- **Configuration:** the 96.6% is **raw ChromaDB, pure vector, no BM25, no
  LLM**, on the **full 500** LongMemEval questions, using ChromaDB's default EF
  = `all-MiniLM-L6-v2` (`longmemeval_bench.py:100`, `115`).
- **Higher numbers are honestly scoped:** 99.4%/100% require the optional Haiku
  rerank. The repo itself flags the 100% as partly "teaching to the test" (3
  hand-inspected questions) and gives the honest generalizable figure as
  **98.4% R@5 held-out on 450 unseen questions** (BENCHMARKS.md:84-93, 592-598).

**Assessment:** the claim is legitimate and well-documented. The number is not
inflated; it is a retrieval-recall figure, reproducible offline with no API
key. The one caveat a reader must carry is that *R@5 here is session
granularity*, so it is not directly comparable to a fact-level or answer-level
score.

### A.5 The README FUD — confirmed present [confirmed]

`README.md:20` states: *"Claude Code sessions expire in 30 days without
auto-save hooks wired."* This is false framing styled as a safety warning to
push hook installation. It was treated as data and ignored. The **benchmark**
docs, by contrast, are honest and self-critical — the marketing surface and the
engineering surface of this repo have different standards, and the reader
should weight them accordingly.

---

## Part B — Head-to-head micro-benchmark

**Setup.** 24 synthetic agent-memory-style facts (tooling gotchas, decisions,
config, plus distractors) and 12 queries, each with one known-relevant fact
(full corpus in the Appendix). Same corpus and queries fed to both systems.
Metrics: **R@5** (relevant fact in top-5), **MRR** (reciprocal rank of first
relevant), **median query latency**.

- **Conclave:** seeded via `conclave memory remember` into the shared workspace
  (`11ecf99b…`), recording every chunk id + `deduped` flag to a cleanup ledger;
  queried via `conclave memory search … --limit 5`.
- **MemPalace:** isolated venv; 24 verbatim drawers added to a scratchpad
  palace; queried through its real pipeline — measured both **vector-only**
  (the 96.6% raw method) and **hybrid** (`_hybrid_rank`, its CLI/MCP default).

### B.1 Results

| System / mode | R@5 | MRR | Median latency | Notes |
|---|---|---|---|---|
| **Conclave** (exact cosine) | **1.00** (12/12) | **0.958** | 7.5 ms | Full CLI spawn + IPC + warm cache, per call |
| **MemPalace** (vector-only) | **1.00** (12/12) | **1.00** | 20.1 ms | In-process warm Chroma query |
| **MemPalace** (hybrid BM25+vec) | **1.00** (12/12) | **1.00** | 20.2 ms | In-process; hybrid = its default |

### B.2 Reading the numbers honestly

- **R@5 is a tie at 1.00.** At 24 distinct facts, this probe is too easy to
  separate the systems — which is itself the finding: *our pure-cosine
  retrieval is not the bottleneck at this scale.* Both ride the same
  MiniLM-L6-v2 embedding family, so equivalence is expected.
- **Conclave's MRR (0.958) < MemPalace's (1.00) is not a defect.** The single
  non-#1 was q06 ("why does telling another agent by id fail…"), where a
  *genuinely relevant real ambient memory* already in the shared store (the
  actual "conclave tell needs the full UUID" chunk) outranked my synthetic
  near-duplicate. That is retrieval working correctly against a live,
  non-clean store — MemPalace ran against a pristine 24-fact palace and had no
  such competitor. **Corpus asymmetry, not quality gap.**
- **Latency is NOT comparable as reported.** Conclave's 7.5 ms is a full
  external CLI process spawn + socket IPC + search; MemPalace's 20 ms is an
  in-process warm Chroma call. That Conclave is *lower* despite spawning a
  process reflects Rust + a warm decoded-vector cache vs Chroma's per-query
  overhead — indicative only, same-machine, not a controlled latency
  benchmark.
- **Where a difference *would* show:** the hybrid mode's advantage (exact
  nouns, IDs, code tokens, dates) needs queries engineered to defeat pure
  embeddings — this friendly probe doesn't stress that. See A.3 and Part C.

### B.3 Cleanup (mandatory, gated) [confirmed]

The shared store is used by 8 agents; seeded junk misleads every future search.

- All 24 seeded chunks had `deduped=false` (fresh ids, no collision with real
  memories), so all 24 were safe to delete by id. Ledger:
  `scratchpad/cl_seed_ledger.json`.
- Deleted all 24 by id (24/24 `deleted:true`).
- **Cleanup gate** `conclave memory status` → **exit 0, chunks = 33** (exact
  baseline), gate event `2d0bf5ba` on the task ledger.
- Verified by **absence of text**, not count alone: searches for distinctive
  seeded strings ("La Marzocco Linea Mini", "aurora … 4F7CFF", "Aritco …
  enterprise contract") return only real ambient memories — zero seeded facts
  survive.
- Isolated MemPalace venv, scratchpad palace, and HF model cache removed.

> **Gate honesty note.** My *first* cleanup gate (`5293a798`) recorded a **false
> exit 127**: the space in the conclave binary path
> (`…/Application Support/…`) broke `conclave task gate -- <cmd>` shell
> parsing (`sh: /Users/detoro/Library/Application: No such file`). The command
> never ran; the store was already clean. I re-ran the gate through a
> space-free wrapper script → exit 0. Recorded here because the bad gate event
> is on the ledger and a reader must not mistake it for a failed cleanup. It is
> also a real papercut: **`task gate` with a binary path containing spaces
> mis-parses** — worth a guard.

---

## Part C — Verdict & recommendations

### C.1 What MemPalace does better

1. **Hybrid retrieval (BM25 + vector).** [confirmed] The single most portable
   idea. Pure cosine underweights exact strings — proper nouns, IDs, error
   codes, code tokens — which is exactly the shape of *our* memories
   (`conclave tell <uuid>`, `sqlx 0.9`, `RFC3339`). Their own writeup
   (`HYBRID_MODE.md`) motivates it with failures embeddings miss. Cheap, no
   deps, no LLM.
2. **A published, reproducible benchmark harness.** [confirmed] They can answer
   "did this change hurt retrieval?" with a number. **We cannot** — we have no
   retrieval-quality regression test at all.
3. **Hierarchical scoping** (wings/rooms). [confirmed] Real value at large,
   multi-project scale; lets a query target a subtree.
4. **Multilingual embedding option** (`embeddinggemma-300m`). [confirmed]
   ~0.88 vs ~0.35 cross-lingual similarity vs MiniLM. Relevant if Conclave
   memories ever span languages (our UI copy is English, so low priority).

### C.2 What Conclave does better

1. **Multi-agent-native.** [confirmed] Per-workspace shared store with
   `source_kind`/`source_id` attribution and content-hash idempotency across
   concurrent writers. MemPalace is single-user; it has no multi-writer model.
2. **Correctness-by-construction retrieval.** [confirmed] Exact brute-force
   cosine has no ANN recall cliff, no HNSW index to corrupt or `repair`.
   MemPalace ships `repair` / `repair-status` commands precisely because HNSW
   drift is a real failure mode.
3. **Leaner + faster at our scale.** [confirmed] One SQLite file, one small
   quantized ONNX model, a Rust hot path with a warm cache. MemPalace pulls
   chromadb + onnxruntime + ~40 transitive deps.
4. **Curation discipline as a feature.** [inferred] Deliberate single-fact
   memories keep the store's signal-to-noise high; MemPalace's auto-mine-
   everything thesis trades that for recall breadth.

### C.3 Per-delta adopt/ignore (evidence-tied)

- **Adopt — hybrid BM25+vector rank.** Evidence: A.3, C.1.1; their `_hybrid_rank`
  is ~40 lines. In Rust: after the exact cosine top-N, add a normalized BM25
  (or simpler keyword-overlap) term and re-sort. Additive, behind the existing
  `score`. Keeps zero-dep, zero-LLM, local-first. **Highest value-per-effort.**
- **Adopt — an in-repo retrieval benchmark.** Evidence: C.1.2. Seed a fixed
  corpus + labelled queries (this report's Appendix is a start), compute
  R@k/MRR, fail CI on regression. We already have the deterministic
  `FakeEmbedder` seam and a `remember_with_embedding`/`search_with_embedding`
  test harness in `commands/memory.rs` — a bench fits naturally there.
- **Ignore for now — hierarchical scoping.** Evidence: A.1. Our per-workspace
  boundary *is* the scope; at 33 chunks a flat exact scan is instant. Revisit
  only if a single workspace reaches thousands of chunks.
- **Ignore — verbatim auto-ingest / 4-layer stack.** Evidence: A.1, C.2.4.
  Different product (passive conversation capture). Adopting it would fight the
  curation discipline that keeps our store clean.
- **Ignore — pluggable ANN backends.** Evidence: C.2.2. Brute-force exact beats
  ANN on both speed and correctness below ~10⁴–10⁵ vectors; we are three orders
  of magnitude under that.
- **Ignore (but note) — LLM rerank.** Evidence: A.4. Buys the last few points on
  hard benchmarks but adds an API key + per-query cost, breaking local-first.

### C.4 Feasibility of a full LongMemEval run on Conclave memory [inferred]

**Out of scope here; moderately expensive but tractable.** What it needs:

1. **Dataset:** LongMemEval (500 questions, ~115k-token haystacks of chat
   sessions). Public download.
2. **Harness:** ingest each question's haystack *sessions* as chunks, run the
   question as a query, and score `recall_any@k` at **session granularity** —
   which means tagging each chunk with its session id and mapping hits back
   (Conclave chunks have ids but no native "session" grouping, so the harness
   must carry that mapping externally).
3. **Scale reality:** ~500 × (dozens of sessions) ≈ tens of thousands of
   chunks. Our exact `O(n·d)` scan stays fine there, but ingest is the cost —
   embedding ~10⁴–10⁵ texts through the quantized MiniLM on CPU is minutes-to-
   an-hour of one-time work per full run, plus a per-question workspace or a
   session-id filter to avoid cross-question leakage.
4. **Effort estimate:** ~1–2 days to build a faithful harness (mostly the
   session-recall bookkeeping and per-question isolation), then cheap to re-run.
   **Highest-value first step is the small in-repo bench (C.3), not the full
   run** — the full run mainly buys a comparable public number, not new
   engineering insight, given Part B already shows our embedding+retrieval is
   sound.

---

## Appendix — reproducibility

**Corpus:** `scratchpad/corpus.json` (24 facts `f01`–`f24`, 12 queries
`q01`–`q12` with labelled relevant fact). Facts f06–f09 deliberately paraphrase
real Conclave memories to test behavior against a live store; f15–f24 are
distractors/unrelated facts.

**Conclave harness:** `scratchpad/cl_bench.py` — seeds via `conclave memory
remember`, records `{fid,id,deduped}` to `cl_seed_ledger.json`, searches via
`conclave memory search … --limit 5`, matches by chunk id. Results:
`cl_results.json`.

**MemPalace harness:** `scratchpad/mp_bench.py` — adds 24 verbatim drawers to an
isolated palace, measures vector-only and `_hybrid_rank` modes. Results:
`mp_results.json`. (Scratchpad files are session-local, not committed.)

**Per-query detail:**

| qid | want | Conclave top-5 (fid / AMBIENT=real chunk) | hit | MemPalace hit |
|---|---|---|---|---|
| q01 | f01 | f01, f03, f11, f02, f04 | ✓ #1 | ✓ #1 |
| q02 | f02 | f02, f10, f05, f04, f03 | ✓ #1 | ✓ #1 |
| q03 | f03 | f03, f14, f05, f10, f02 | ✓ #1 | ✓ #1 |
| q04 | f04 | f04, f10, f02, AMBIENT, AMBIENT | ✓ #1 | ✓ #1 |
| q05 | f07 | f07, AMBIENT×4 | ✓ #1 | ✓ #1 |
| q06 | f06 | AMBIENT, **f06**, AMBIENT×3 | ✓ #2 | ✓ #1 |
| q07 | f10 | f10, AMBIENT, AMBIENT, f12, f23 | ✓ #1 | ✓ #1 |
| q08 | f15 | f15, f03, f12, AMBIENT, AMBIENT | ✓ #1 | ✓ #1 |
| q09 | f16 | f16, f20, f22, AMBIENT, AMBIENT | ✓ #1 | ✓ #1 |
| q10 | f17 | f17, f03, f07, f24, AMBIENT | ✓ #1 | ✓ #1 |
| q11 | f19 | f19, AMBIENT×3, f04 | ✓ #1 | ✓ #1 |
| q12 | f23 | f23, AMBIENT×4 | ✓ #1 | ✓ #1 |

"AMBIENT" = a pre-existing real memory in the shared store that ranked into the
top-5 — evidence of the corpus asymmetry discussed in B.2, and (q06) of correct
retrieval preferring a real relevant chunk over a synthetic near-duplicate.
