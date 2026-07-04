# MemPalace → Conclave native Rust port: scope recommendation

Date: 2026-07-04  
Researcher: Marty (`5524d1c5-50c0-47d3-bfbd-7bb45f9d38ef`)  
Source snapshot: MemPalace `da5a48caf5d8a843df7568a00e44c714bd91ab11` (read-only)

## Decision summary

Build a Conclave-native memory module, not a general MemPalace port.

The minimal useful cut is:

1. Persist structured completed conversation messages.
2. Turn selected messages into workspace-scoped memory chunks.
3. Embed them locally with one pinned model.
4. Store normalized `float32` vectors as SQLite BLOBs in Conclave's existing database.
5. Search one workspace with exact cosine and return top-k source-linked chunks.
6. Expose explicit remember/search/delete commands; automatically index completed **chat-agent** turns only.

Use `fastembed` rather than raw `ort` if raising Conclave's Rust floor from 1.77 to 1.88 is acceptable. Ship quantized all-MiniLM-L6-v2 first. Defer EmbeddingGemma to an opt-in multilingual model because it adds a ~309 MB download and Gemma-specific distribution terms. Defer `sqlite-vec` until measured workspace corpora make exact search miss a defined latency gate.

Do **not** port miners, source adapters, entity extraction, closets, knowledge graph, multi-backend abstractions, filesystem “palaces,” repair/sync/daemon machinery, or raw PTY transcript mining.

## Important prerequisite found in Conclave

Conclave has a `message` table, but its normal user-message path does not persist to it. `message.send` explicitly says “NOT persisted yet” and has a `TODO(M3): persist to message table` ([local source](../src-tauri/src/engine/commands/message.rs#L13), lines 13–15 and 58–66). Chat history is also process-local inside `spawn_chat`; assistant output leaves as streamed deltas rather than a completed message record ([local source](../src-tauri/src/engine/runtime/chat.rs#L47), lines 47–74).

The shared output forwarder cannot be treated as a generic transcript source. Its own contract says CLI/PTY output is terminal redraw noise, while only chat output is meaningful conversation text ([local source](../src-tauri/src/engine/commands/instance.rs#L588), lines 588–609 and 648–664).

Therefore:

- Structured message persistence is a prerequisite for automatic conversation memory.
- For chat agents, add a completed-turn boundary from the provider loop and persist user/assistant messages before indexing.
- Do not embed `session:output` chunks.
- Automatic CLI-agent memory needs harness-native transcript adapters or explicit agent calls later; parsing PTY output is outside the minimal core.

## 1. What maps cleanly from MemPalace

### Concept map

| MemPalace concept | Source behavior | Conclave mapping |
|---|---|---|
| Palace | Filesystem directory plus selected backend | Drop. Use Conclave's existing `conclave.db`. |
| Collection | Named vector namespace with one recorded dimension/model | One workspace-scoped memory index. Keep model/dimension identity. |
| Drawer | Verbatim text plus metadata and embedding | `memory_chunk`, linked to its source message/session. |
| Wing | Broad project grouping | `workspace_id`. |
| Room | Subtopic grouping | Drop for v1; optional `source_kind`/session filter is sufficient. |
| Closet | Derived topic/entity document used as a ranking signal | Drop. This depends on mining/entity logic and is not required for semantic search. |

MemPalace's `palace.get_collection` adds two valuable invariants around an otherwise simple backend: explicit-vector backends are wrapped by a local embedder, and collection opens enforce embedder identity ([source](https://github.com/MemPalace/mempalace/blob/da5a48caf5d8a843df7568a00e44c714bd91ab11/mempalace/palace.py#L79-L200)). Port those invariants, not the backend registry or palace-path selection.

### SQLite schema to preserve in spirit

`sqlite_exact.py` uses:

- `collections(name, dimension, created_at)`;
- `documents(collection_id, id, document, metadata_json, embedding BLOB, dim, created_at, updated_at)`;
- a unique document key per collection;
- a metadata key recording the embedder model;
- optional FTS5; and
- WAL mode ([source](https://github.com/MemPalace/mempalace/blob/da5a48caf5d8a843df7568a00e44c714bd91ab11/mempalace/backends/sqlite_exact.py#L896-L951)).

For Conclave, use the existing workspace and migration model rather than reproducing `collections`:

- `memory_index(workspace_id PK/FK, model_id, dimension, created_at, updated_at)`
- `memory_chunk(id PK, workspace_id FK, message_id nullable FK, source_kind, source_id, chunk_index, text, embedding BLOB, dimension, content_hash, created_at, updated_at)`
- uniqueness on `(workspace_id, source_kind, source_id, chunk_index)`
- index on `(workspace_id, created_at)`

Typed scope/source columns are preferable to putting core query fields in JSON. A small optional metadata JSON column can remain for forward compatibility.

Preserve these correctness rules:

- one embedding model and dimension per workspace index;
- a model change requires re-embedding, never mixed vector spaces;
- batch embedding completes before the SQLite transaction writes chunks;
- normalized vectors use a documented canonical little-endian `float32` encoding;
- store/delete/search are workspace-scoped;
- delete/clear is part of v1 because the data is conversation content.

MemPalace's backend currently checks dimensions on every write and query ([source](https://github.com/MemPalace/mempalace/blob/da5a48caf5d8a843df7568a00e44c714bd91ab11/mempalace/backends/sqlite_exact.py#L299-L319), [query check](https://github.com/MemPalace/mempalace/blob/da5a48caf5d8a843df7568a00e44c714bd91ab11/mempalace/backends/sqlite_exact.py#L568-L591)). Keep that behavior.

### What to port versus drop

Port/adapt:

- model/dimension identity checks;
- explicit-vector embedding boundary;
- batch store/upsert, get, delete, count;
- deterministic content hash/idempotency;
- bounded chunking for model input limits;
- exact top-k cosine;
- simple filters: workspace required, session/source optional;
- source linkage in search results.

Implement independently rather than copying line-for-line:

- pre-normalize each stored vector so cosine is a dot product;
- use a bounded top-k heap rather than sorting every scored row;
- run synchronous model inference and large scans off Tokio's async worker threads;
- make model download/cache state explicit.

Drop from the minimal core:

- Chroma, pgvector, Qdrant, backend plugins and backend detection;
- file/project/conversation miners and source adapters;
- normalization pipelines for third-party transcript formats;
- entity detection/registry, closets, hallways, layers and knowledge graph;
- LLM extraction/refinement/fact checking;
- BM25 candidate union, closet boosts and neighbor hydration;
- daemon, hooks, sync, export, backup, migration and repair CLIs;
- multi-palace filesystem management;
- automatic CLI/PTY transcript capture.

The dropped search logic is substantial: `search_memories` over-fetches vectors, queries a second “closets” collection, applies rank boosts, hydrates neighboring chunks, and optionally unions lexical candidates ([source](https://github.com/MemPalace/mempalace/blob/da5a48caf5d8a843df7568a00e44c714bd91ab11/mempalace/searcher.py#L1049-L1321)). None of that is necessary to prove local semantic memory.

## 2. Rust embedding: fastembed versus raw ort

### Confirmed package support

As of 2026-07-04, `fastembed 5.17.2` directly lists:

- `AllMiniLML6V2` and `AllMiniLML6V2Q`;
- `EmbeddingGemma300M`, `EmbeddingGemma300MQ4`, and `EmbeddingGemma300MQ`.

Its model table points the q8 Gemma variant at `onnx-community/embeddinggemma-300m-ONNX`, `onnx/model_quantized.onnx`, plus the external data file ([fastembed model enum/source](https://docs.rs/fastembed/5.17.2/fastembed/enum.EmbeddingModel.html), [model table](https://docs.rs/fastembed/latest/src/fastembed/models/text_embedding.rs.html#417-443)). It also handles model download/cache, tokenization, batching, pooling and normalization. This is the main reason to prefer it over raw `ort`.

The compatibility cost is real:

- Conclave declares `rust-version = "1.77"`.
- `fastembed 5.17.2` pins `ort = 2.0.0-rc.12`.
- `ort rc.12` declares Rust 1.88 ([crate metadata](https://docs.rs/crate/ort/2.0.0-rc.12)).

`fastembed 4.9.1` uses `ort rc.9` (Rust 1.70) and supports MiniLM, but its model enum does not contain EmbeddingGemma. It is a possible MiniLM-only compatibility pin, although the crate itself declares no MSRV and still needs a CI proof on Rust 1.77.

### Recommendation

Preferred:

- raise the project Rust floor to 1.88;
- use `fastembed 5.17.2` with unnecessary default features disabled;
- keep one lazily initialized model worker;
- place the cache under Conclave's application-support directory;
- pin model repo revision and verify expected files/hashes;
- call inference through `spawn_blocking` or a dedicated worker queue.

Fallback if the Rust floor cannot move:

- use `fastembed 4.9.1` with quantized MiniLM only;
- defer Gemma.

Do not choose raw `ort` merely to avoid the version bump. Raw `ort` requires Conclave to own Hugging Face retrieval, tokenizer files, input tensor construction, pooling/output selection, query/document prompting, external initializer handling, batching, MRL truncation and normalization. That is several additional failure surfaces. Raw `ort rc.9` plus a custom Gemma pipeline is technically plausible but unverified against this model export and should be treated as a separate spike.

### Model size, dimension and license

| Model | Fastembed variant | Vector dimension | Download | License/product effect |
|---|---|---:|---:|---|
| all-MiniLM-L6-v2 q8 | `AllMiniLML6V2Q` | 384 | 23 MB ONNX | Apache-2.0; simplest v1 choice ([model file](https://huggingface.co/Xenova/all-MiniLM-L6-v2/blob/main/onnx/model_quantized.onnx)). |
| EmbeddingGemma q8 | `EmbeddingGemma300MQ` | 768 from fastembed | 568 KB graph + 309 MB external data | Gemma terms, not a permissive OSS model license ([files](https://huggingface.co/onnx-community/embeddinggemma-300m-ONNX/tree/8cd82455fa7fae53af154775fd1494f5577ef4b0/onnx), [terms](https://ai.google.dev/gemma/terms)). |

The EmbeddingGemma model card says the native output is 768 and officially describes MRL truncation to 512, 256 or 128 dimensions ([model card](https://huggingface.co/onnx-community/embeddinggemma-300m-ONNX)). MemPalace instead truncates to 384 and re-normalizes ([source](https://github.com/MemPalace/mempalace/blob/da5a48caf5d8a843df7568a00e44c714bd91ab11/mempalace/embedding.py#L203-L220), [inference](https://github.com/MemPalace/mempalace/blob/da5a48caf5d8a843df7568a00e44c714bd91ab11/mempalace/embedding.py#L321-L352)). Do not copy that 384 choice without an evaluation; use 768 or an officially documented MRL width.

For retrieval, also use the model card's distinct prompts:

- query: `task: search result | query: ...`
- document: `title: none | text: ...`

MemPalace's current embedding wrapper applies one sentence-similarity prefix to every input. That is not the documented retrieval configuration.

### Binary size measurement

A local Darwin arm64 release probe on `rustc 1.96.0` measured:

- raw `ort rc.12` + CoreML registration: 23,867,200 bytes;
- `fastembed 5.17.2` + `ort` + tokenizers/HF client: 28,833,584 bytes;
- current standalone Conclave release binary: 17,698,144 bytes.

These are isolated statically linked probes, so they are not arithmetically additive to the final Tauri binary. They show the relevant order of magnitude: the native runtime/wrapper is tens of MB, not hundreds. Models should remain lazy downloads and therefore not inflate the notarized app bundle.

### CPU and CoreML

CPU execution is supported for both models. `ort` exposes the CoreML execution provider and compute-unit controls ([ort CoreML API](https://docs.rs/ort/2.0.0-rc.12/ort/ep/coreml/struct.CoreML.html)). The probe linked CoreML successfully on arm64 macOS.

Do not claim CoreML acceleration for these exact graphs yet. ONNX Runtime assigns only supported subgraphs to an execution provider and falls back to CPU; its CoreML operator support is a subset ([ORT provider model](https://onnxruntime.ai/docs/execution-providers/), [CoreML operators](https://onnxruntime.ai/docs/execution-providers/CoreML-ExecutionProvider.html)). Quantized Gemma in particular needs an empirical provider-assignment and latency test. Treat CPU as the guaranteed baseline and CoreML as an optimization gate.

## 3. Exact cosine versus sqlite-vec

MemPalace's exact backend loads every matching row, decodes every BLOB, computes cosine for every vector, sorts all scores and slices top-k ([source](https://github.com/MemPalace/mempalace/blob/da5a48caf5d8a843df7568a00e44c714bd91ab11/mempalace/backends/sqlite_exact.py#L568-L606)). Complexity is `O(N × dimensions)` plus a full sort.

`sqlite-vec` is also brute-force exact search today, not ANN. Its advantages are optimized C loops, vector-oriented storage and top-k query execution; it does not change the asymptotic scale limit. The project is still pre-v1 and explicitly warns to expect breaking changes ([project](https://github.com/asg017/sqlite-vec)). Its official Rust integration documents `rusqlite` plus unsafe `sqlite3_auto_extension` registration, not `sqlx` ([Rust guide](https://alexgarcia.xyz/sqlite-vec/rust.html)), so adopting it now also adds an integration seam.

Published M1/8 GB benchmarks report:

- 100,000 on-disk float vectors at 384 dimensions: below 75 ms with `vec0`;
- 1,000,000 float vectors: no tested dimension met the 100 ms target; even 192 dimensions took 192 ms;
- 1,000,000 × 128 dimensions: 33–35 ms in a different in-memory benchmark setup.

The author explicitly warns that results are hardware/workload dependent ([benchmark source](https://alexgarcia.xyz/blog/2024/sqlite-vec-stable-release/index.html#benchmarks)).

Storage alone is material:

- 10k × 384 float32 vectors: ~14.6 MiB;
- 50k: ~73.2 MiB;
- 100k: ~146.5 MiB;
- 1M: ~1.43 GiB;

excluding SQLite row/index overhead and text.

Recommendation:

- ship optimized Rust exact search first;
- normalize on write and use dot product plus a bounded top-k heap;
- scope every scan by `workspace_id`;
- add benchmark fixtures at 10k, 50k and 100k 384-dimensional rows;
- acceptance target: warm-cache p95 search under 100 ms at 50k rows on the oldest supported Apple Silicon machine;
- begin `sqlite-vec` adoption when a real workspace exceeds 50k chunks or the 50k p95 gate fails;
- require `sqlite-vec` by 100k only if the product still expects interactive search at that scale;
- ANN is a later architectural decision if corpora approach one million rows.

There is no universal row count where exact search “breaks.” For this product, 50k is the conservative migration trigger and 100 ms is the actual decision metric.

## 4. Rough implementation partition and effort

Assumption: one experienced Rust engineer, existing Conclave test patterns, no UI redesign, fastembed path, MiniLM q8 only.

| Task | Output | Estimate |
|---|---|---:|
| 1. Decision/prototype gate | Confirm Rust 1.88 bump, integrated app-size delta, MiniLM inference, CPU/CoreML observation, model cache path | 1–2 days |
| 2. Structured message persistence | Message repo; persist user and completed assistant turns; explicit turn-completion channel; deletion behavior | 2–3 days |
| 3. Memory schema/repository | Migration, model identity, chunk CRUD, canonical BLOB codec, transaction/idempotency tests | 2–3 days |
| 4. Embedder service | Lazy worker, q8 MiniLM, cache/download states, batching, normalization, error mapping | 2–3 days |
| 5. Exact search service | Workspace filters, top-k heap, source linkage, delete/clear, 10k/50k/100k benchmarks | 1.5–2.5 days |
| 6. Product/agent integration | `memory.remember/search/delete` IPC and CLI/tool surface; auto-index completed chat turns | 1.5–2.5 days |
| 7. Packaging/compliance/validation | Offline restart, corrupt/missing model, notarized build, third-party notices, privacy/deletion tests | 1.5–2.5 days |

Total: roughly **12–18 engineer-days** including validation.

Parallelism after the schema/API contract:

- embedder prototype and message-persistence work are independent;
- repository and exact-search work can proceed with deterministic fake embeddings;
- integration follows once both are stable.

Additions:

- raw `ort` custom pipeline: +4–7 days plus more model-specific risk;
- EmbeddingGemma option/settings/license flow: +2–4 days;
- automatic CLI transcript adapters: +5–10 days per harness family and should be separately scoped;
- BM25/FTS hybrid ranking: +2–3 days;
- `sqlite-vec`: +2–4 days after a focused `sqlx` integration spike.

## 5. MIT attribution obligations for ported MemPalace logic

MemPalace's license says the copyright and permission notice must be included in all copies or substantial portions of the software ([repository license](https://github.com/MemPalace/mempalace/blob/da5a48caf5d8a843df7568a00e44c714bd91ab11/LICENSE#L1-L20)).

For a logic port, take the conservative compliance path:

1. Add the full MemPalace MIT license to Conclave's bundled third-party notices.
2. Preserve `Copyright (c) 2026 MemPalace Contributors`.
3. Record the upstream URL and source commit used for the port.
4. Add a short attribution comment to files that materially adapt upstream algorithms or schema.
5. State that the Rust implementation was modified/ported by Conclave.

MIT does not require publishing Conclave's source or using the MIT license for the whole app. Attribution need not appear in the primary UI; it can live in the shipped licenses/notices view or file.

Dependencies/models have separate obligations:

- `fastembed`: Apache-2.0;
- `ort`: MIT OR Apache-2.0;
- `sqlite-vec` if later used: MIT OR Apache-2.0;
- MiniLM model: Apache-2.0;
- EmbeddingGemma: Gemma Terms, including a required Notice file and downstream-use-restriction notice for distribution.

The Gemma terms are not an MIT-style attribution-only license. Before Conclave automatically downloads or exposes Gemma functionality, product/legal should confirm the distribution flow and notices. The safest implementation is an explicit opt-in installer that shows the terms and ships the required Notice text.

## Recommended minimal-core acceptance gates

The port is complete when:

- a completed chat turn is durably persisted before memory indexing;
- an explicit memory can also be stored without a chat turn;
- repeated store calls are idempotent;
- search never crosses workspaces;
- model/dimension mismatch fails before returning misleading results;
- delete/clear removes source-linked chunks;
- a model downloads once, works offline afterward, and reports recoverable cache errors;
- 50k × 384 exact search meets warm-cache p95 <100 ms on the supported baseline Mac;
- the release app is built/notarized with measured size delta recorded;
- third-party/model notices ship with the app;
- no raw PTY output is embedded.

## Confirmed versus inferred

Confirmed:

- MemPalace's exact data model is plain SQLite plus float32 BLOBs.
- Its vector query is a full exact cosine scan.
- Current fastembed supports both requested model families and q8 variants.
- Current fastembed implies Rust 1.88 through ort rc.12.
- MiniLM q8 is ~23 MB; EmbeddingGemma q8 weights are ~309 MB.
- Conclave currently does not persist ordinary chat/user turns.
- sqlite-vec is exact brute force and remains pre-v1.

Inferred/recommended:

- 50k is the right Conclave migration trigger; it must be validated by the proposed benchmark.
- Fastembed's integrated app delta will be similar to, but not exactly equal to, the standalone probe.
- CoreML may accelerate part or all of either model; model-specific verification is still required.
- The safest Gemma compliance posture is explicit opt-in plus bundled terms/notice; this is not legal advice.
