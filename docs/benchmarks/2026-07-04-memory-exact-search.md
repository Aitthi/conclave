# Workspace Memory v1 exact-search benchmark

Date: 2026-07-04

## Environment

- Apple M1 Pro
- 16 GiB RAM
- macOS 26.5.1 (25F80)
- Rust release profile
- In-memory SQLite fixture using the production schema and repository path
- Deterministic `FakeEmbedder` vectors, 384 dimensions
- Exact dot-product top-k (`k = 8`)

Command:

```sh
cargo test --release --manifest-path src-tauri/Cargo.toml \
  benchmark_exact_search_10k_50k -- --ignored --nocapture
```

## Result

| Rows | Cold cache build | Warm p95 (20 samples) |
|---:|---:|---:|
| 10,000 | 38.121 ms | 3.022 ms |
| 50,000 | 182.565 ms | 14.799 ms |

The 50k warm p95 passes the required `< 100 ms` gate. The 50k cold path also
meets the non-blocking `< 300 ms` target.

The benchmark covers database BLOB loading, mandatory `vec_codec::decode`,
cache construction, and exact cached scoring. It excludes model inference so
the measurement isolates the exact-search subsystem.

## Diagnostic record

The initial DB-backed scan missed the gate:

- 10k warm p95: 416.125 ms
- 50k warm p95: 4,589.564 ms

`EXPLAIN QUERY PLAN` showed a temporary B-tree sort on every keyset page.
Adding `idx_memory_chunk_ws_id(workspace_id, id)` reduced 50k to roughly
183 ms, but component timing showed SQLite BLOB materialization alone at
158–183 ms for 76.8 MB; page size, metadata columns, and dot-product scoring
were falsified as the remaining bottleneck. The bounded four-workspace decoded
vector cache removes that per-query BLOB materialization while preserving
workspace-scoped exact search.
