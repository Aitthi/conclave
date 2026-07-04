# Plan: Workspace Memory v1 (native AI memory system)

Date: 2026-07-04 · Lead: Detoro (`bfb737ff-486d-4581-b407-95711d5e07ab`)
Basis: `docs/2026-07-04-mempalace-rust-port-scope.md` (Marty) + human scope ruling.

## Scope (final, human-ruled 2026-07-04)

An **explicit AI memory system** for Conclave: agents/users store memories, search
them semantically, and delete them — per workspace, fully local, zero extra
installs for the end user.

**Cut from scope by the human:** structured chat-message persistence and
automatic indexing of chat turns. There is NO auto-capture in v1. Memory enters
the system only through the explicit `memory.remember` command. The scope
report's "Task 2 / prerequisite" section is superseded by this ruling.

Everything else from the scope report's rulings stands:

- R1: Rust floor 1.77 → **1.88** (required by fastembed 5.17.2 / ort rc.12).
- R2: v1 model = **all-MiniLM-L6-v2 q8 only** (`AllMiniLML6V2Q`, 384-dim, ~23 MB,
  Apache-2.0). EmbeddingGemma deferred entirely (Gemma terms).
- R3: **exact cosine** search first; `sqlite-vec` deferred behind the
  50k-rows / p95<100ms gate.
- R4: never embed `session:output` / PTY text.

## Global constraints (every task inherits these)

1. Language: repo code, comments, docs in **English**.
2. DB access follows the existing repo-layer convention (see
   `src-tauri/src/engine/repo/blackboard.rs` module doc): chain-builder for
   single-table SELECTs, raw `sqlx` for UPSERT/JOIN with a doc-comment stating
   the fallback rationale.
3. Vectors: **L2-normalized at write time**, encoded as **little-endian f32
   BLOB**; cosine ≡ dot product at query time. Dimension is checked against
   `memory_index.dimension` on every write and every query; mismatch →
   `AppError::Invalid`, never a silent wrong result.
4. All memory operations are **workspace-scoped**; a query must never return
   another workspace's chunks. Enforce in SQL (`WHERE workspace_id = ?`), not
   in post-filtering.
5. Model inference and full-table vector scans run via `tokio::task::spawn_blocking`
   (or a dedicated worker) — never on the async executor threads.
6. One embedding model per workspace index: `memory_index.model_id` is recorded
   on first write; a different model id on a later write is an error (re-embed
   flow is out of scope for v1).
7. No network at query time. The model downloads once (first embed), lives under
   the app-support dir, then everything is offline.
8. Branch discipline: one branch per lane, lead merges. Commit with a pathspec
   (shared tree): `git commit -- <your files>`.
9. Migrations are NOT auto-discovered: `db::migrate`
   (`src-tauri/src/engine/db.rs`) registers each one explicitly with an
   `if version < N { include_str!(...); PRAGMA user_version = N; }` block.
   **Any task that adds a migration also owns adding its version block in
   `db.rs`** — a migration file without its `db.rs` block never runs.
   (Guard added 2026-07-04 after Dabin caught this gap in T2's file boundary.)

   **Ordering rule (2026-07-04, flagged by Dew):** a `version < N` block may
   only merge to main when block `N-1` already exists on main — otherwise a
   fresh DB migrated in the gap jumps past the missing version and skips it
   forever (its later block never fires). The lead enforces this at merge.
   Structural guard (T5 scope): a unit test that lists
   `src/engine/migrations/` and asserts (a) file numbers are contiguous from
   0001 with no gaps, and (b) a fresh `migrate()` lands `user_version` at the
   max file number — so a skipped-version state fails the gate instead of
   shipping.

## Interface contract (fixed — escalate to lead before deviating)

### Schema — migration `src-tauri/src/engine/migrations/0009_memory_system.sql`

```sql
CREATE TABLE memory_index (
  workspace_id TEXT PRIMARY KEY REFERENCES workspace(id) ON DELETE CASCADE,
  model_id     TEXT NOT NULL,          -- e.g. "all-minilm-l6-v2-q8"
  dimension    INTEGER NOT NULL,       -- 384
  created_at   TEXT NOT NULL,
  updated_at   TEXT NOT NULL
);

CREATE TABLE memory_chunk (
  id           TEXT PRIMARY KEY,       -- uuid v4
  workspace_id TEXT NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
  source_kind  TEXT NOT NULL,          -- 'manual' | 'agent' (v1 values; TEXT not enum)
  source_id    TEXT,                   -- agent instance id when source_kind='agent'
  text         TEXT NOT NULL,
  embedding    BLOB NOT NULL,          -- normalized f32 LE, dimension floats
  dimension    INTEGER NOT NULL,
  content_hash TEXT NOT NULL,          -- sha256 hex of NFC-normalized text
  created_at   TEXT NOT NULL,
  updated_at   TEXT NOT NULL,
  UNIQUE (workspace_id, content_hash)  -- idempotency: same text stored twice = upsert
);
CREATE INDEX idx_memory_chunk_ws_created ON memory_chunk(workspace_id, created_at);
```

**Amendment (ruling on Dabin's T4 performance escalation, 2026-07-04):** keyset
paging over `(workspace_id, id)` planned a TEMP B-TREE sort on every page
through `idx_memory_chunk_ws_created` (measured: 50k warm p95 = 4589 ms).
A composite index fixes the scan shape (50k → 183 ms):

```sql
-- migration 0010_memory_search_index.sql (owned by T4, incl. its db.rs
-- `if version < 10` block per global constraint 9)
CREATE INDEX idx_memory_chunk_ws_id ON memory_chunk(workspace_id, id);
```

This lands as a NEW migration, not an in-place edit of 0009: 0009 is already
merged and any dev DB at `user_version = 9` would silently skip an amended
file. T5's seed migration shifts to `0011`.

### Embedder trait — `src-tauri/src/engine/runtime/embedder.rs`

```rust
pub trait Embedder: Send + Sync {
    fn model_id(&self) -> &'static str;
    fn dimension(&self) -> usize;
    /// Cheap, side-effect free; true iff embed() would succeed right now
    /// without network. Backs memory.status.modelReady. (Added 2026-07-04,
    /// ruling on Dabin's second T4 escalation.)
    fn is_ready(&self) -> bool;
    /// Blocking; caller wraps in spawn_blocking. Returns one normalized
    /// f32 vector per input text, in order.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;
}
```

`FastembedEmbedder` implements it for production; tests and the T2/T4 lanes use
`FakeEmbedder` (deterministic: seeded hash → pseudo-vector, normalized) so no
lane blocks on the model gate.

**Seam ownership (ruling on Dabin's T4 escalation, 2026-07-04):** the trait +
`FakeEmbedder` were landed on main by the lead (`5eb4640`) so T3 and T4 build
against the same seam without cross-lane dependencies. From that commit on:

- T3 owns all further additions to `runtime/embedder.rs`
  (`FastembedEmbedder`) AND the production wiring surface: an
  `AppState.memory_embedder: Arc<dyn Embedder>` field in `engine/state.rs`
  plus its construction.
- T4 implements command handlers generic over an injected
  `Arc<dyn Embedder>` (tests/benchmarks use `FakeEmbedder`). The
  `router.rs` entries + production wrappers reading
  `state.memory_embedder` are T4's final, small integration delta AFTER
  T3 merges — never fake/`NotImplemented` production routing.

### Commands — `src-tauri/src/engine/commands/memory.rs`, routed in `router.rs`

| Command | Payload | Result |
|---|---|---|
| `memory.remember` | `{ workspaceId, text, sourceKind?, sourceId? }` | `{ id, deduped: bool }` |
| `memory.search`   | `{ workspaceId, query, limit? (default 8) }` | `{ hits: [{ id, text, score, sourceKind, sourceId, createdAt }] }` |
| `memory.delete`   | `{ workspaceId, id }` | `{ deleted: bool }` |
| `memory.clear`    | `{ workspaceId }` | `{ deleted: number }` |
| `memory.status`   | `{ workspaceId }` | `{ chunks, modelId?, dimension?, modelReady: bool }` |

Wire shapes camelCase, mirroring existing commands. `memory.remember` embeds
BEFORE opening the write transaction (constraint 3 of the scope report).

### CLI — `src-tauri/src/bin/conclave-cli.rs`

```
conclave memory remember <workspaceId> <text>
conclave memory search   <workspaceId> <query> [--limit N]
conclave memory delete   <workspaceId> <chunkId>
conclave memory status   <workspaceId>
```

## Tasks and lanes

### T1 — Prototype gate (lane: Dew, branch `spike/fastembed-gate`)

Prove the load-bearing assumptions before anything merges:

1. Bump `rust-version = "1.88"` in `src-tauri/Cargo.toml`; full workspace
   `cargo build --release` must pass.
2. Add `fastembed = "5.17"` (disable default features not needed; document which).
3. Spike (behind `#[cfg(test)]` or an ignored test, not shipped code): embed 3
   strings with `AllMiniLML6V2Q`, print dimension + first values; verify the
   cache lands where `fastembed`'s cache-dir override points (target: Conclave
   app-support dir), and that a second run is offline.
4. Measure: release binary size before/after fastembed link.
5. Observe which execution provider actually runs (CPU expected; note CoreML
   assignment if any).

**Output:** findings comment on `progress:memory-v1/t1` + the spike branch.
GO/NO-GO ruling by lead. Estimate: 1–2 days.

### T2 — Migration + repo (lane: Dabin, branch `feat/memory-repo`)

Files: `migrations/0009_memory_system.sql`, `repo/memory.rs` (+ `repo/mod.rs`
line), BLOB codec module `engine/runtime/vec_codec.rs`, and the
`if version < 9` registration block in `engine/db.rs` (see global
constraint 9; T4's `0010` and T5's `0011` migrations likewise own their
version blocks).

- Codec: `encode(&[f32]) -> Vec<u8>` / `decode(&[u8], dim) -> Result<Vec<f32>>`,
  little-endian, with round-trip + wrong-length tests.
- Repo fns: `upsert_chunk`, `get_index`, `ensure_index(model_id, dim)`,
  `list_embeddings(workspace_id)` (streamed/chunked read for search),
  `delete_chunk`, `clear_workspace`, `count`.
- Idempotency via `UNIQUE(workspace_id, content_hash)` upsert — follow the
  blackboard `set` raw-sqlx UPSERT pattern.
- Tests use `FakeEmbedder` vectors; no fastembed dependency in this lane.

Estimate: 2–3 days. Independent of T1.

### T3 — Embedder service (Dew after T1 GO, same branch chain)

`runtime/embedder.rs`: trait above + `FastembedEmbedder` (lazy `OnceCell`
init, batching, cache-dir under app support, download-state surfaced in
`memory.status.modelReady`, error mapping to `AppError`) + `FakeEmbedder`.
Estimate: 2–3 days.

### T4 — Search + commands + router (Dabin after T2, branch `feat/memory-commands`)

`commands/memory.rs` + `router.rs` entries + top-k search:
dot-product over decoded BLOBs with a bounded `BinaryHeap` (never full sort).
**Every BLOB goes through `vec_codec::decode(bytes, index.dimension)` before
scoring — never score raw bytes.** (Mellow's T2 review: the read-path
corruption guard of global constraint 3 is carried by this decode call;
`list_embeddings` returns BLOBs un-decoded by design.) Scan scoped by
workspace, `spawn_blocking`. Benchmark fixture at 10k and 50k
rows × 384 dims; record warm p95. Estimate: 2 days.

**Search cache (ruling on Dabin's T4 performance escalation, 2026-07-04):**
measured at 50k×384, raw BLOB materialization from SQLite alone is ~158–183 ms
(76.8 MB per scan) — the fixed 100 ms gate is unreachable by any per-query
scan, with metadata/page-size/scorer hypotheses falsified by component timing.
Therefore T4 adds a `MemorySearchCache` of decoded normalized vectors:

- keyed by `workspace_id`; populated lazily on first search; scoring runs on
  cached vectors (~33–37 ms measured CPU at 50k);
- **correctness property (fixed):** the cache must never serve results that
  contradict a completed `remember`/`delete`/`clear` — invalidate or update
  on every successful write to that workspace; mechanism (drop-on-write vs
  incremental) is implementer judgment;
- **bound (fixed):** small fixed LRU, at most 4 workspaces resident
  (~77 MB each at 50k); eviction must be safe under concurrent search;
- cache rebuilds run off the async executor (global constraint 5);
- T4 owns the cache type + generic handler injection now; the production
  `AppState` field joins the existing post-T3 wiring delta.

**Gate (amended):** warm/cached p95 **< 100 ms @ 50k** stands. Cold path
(first search after launch or invalidation, composite index in place) is
recorded, target **< 300 ms @ 50k**, non-blocking but must be reported.

### T5 — CLI + agent tool surface (whoever frees first)

CLI subcommands (above) + seed a `memory` core tool for agents following the
`0002_seed_core_tools.sql` pattern (new migration `0011_seed_memory_tool.sql`
— `0010` is taken by T4's search index) so workspace agents can call
remember/search via their existing tool channel. Estimate: 1.5–2.5 days.

### T6 — Packaging, notices, validation (lead + Dew)

- `THIRD-PARTY-NOTICES` (or existing notices file): MemPalace MIT (copyright
  2026 MemPalace Contributors + upstream URL + commit
  `da5a48caf5d8a843df7568a00e44c714bd91ab11`), fastembed Apache-2.0,
  ort MIT/Apache-2.0, MiniLM Apache-2.0.
- Validation: fresh install → first `memory.remember` downloads model once →
  kill network → search/remember still work; corrupt cache file → recoverable
  error; notarized build; record app-size delta.
Estimate: 1.5–2.5 days.

**Total: ~10–15 engineer-days, 2 lanes parallel after the contract above.**

## Risk ledger

- ~~ort rc12 native-lib packaging (.dylib)~~ **RESOLVED by T1 (Dew, 2026-07-04):**
  on macOS aarch64, ort links onnxruntime **statically** (70 MB `.a` at build
  time from pyke's CDN) — no separate runtime dylib exists; the single Mach-O
  goes through the existing sign/notarize path, proven with a real
  `cargo tauri build` (notarization Accepted, stapled, spctl pass). Residual:
  that bundle had fastembed dead-code-eliminated (+48 KB only), so **T3 and T6
  must rerun codesign/spctl once the embedder is wired into product code**.
  **CLOSED by T3 (Dew):** rerun with the embedder live — codesign/spctl still
  pass, notarized. Real size delta **+28.6 MB** (binary 17.7→46.3 MB, .app
  18→45 MB, DMG 7.5→16.4 MB) — about 2× the earlier ~14 MB estimate; T6 uses
  these real numbers.
- **fastembed cache-dir override** — verify the env/API override actually
  redirects (default is `.fastembed_cache` in cwd; unacceptable). If not
  overridable, escalate before T3.
- **hf.co reachability at first use** — download failure must surface as a
  clear `memory.status.modelReady=false` + retriable error, never a hang.
- **Windows/Linux** — out of scope for v1 validation (macOS only), note in T6.
- **50k gate machine** — benchmarks run on the lead's machine; the "oldest
  supported Apple Silicon" gate from the scope report is approximated, note
  actual hardware in the benchmark record.

## Acceptance gates (v1 done when)

1. Explicit memory stored, searched, deleted via IPC and CLI, workspace-scoped.
2. Repeated `remember` of identical text is idempotent (`deduped: true`).
3. Model/dimension mismatch fails loudly before any result is returned.
4. Model downloads once, then fully offline.
5. 50k × 384 warm p95 < 100 ms (recorded number, not assertion).
6. Notarized build passes; size delta recorded; notices shipped.
7. No auto-capture code paths exist (human scope ruling).
