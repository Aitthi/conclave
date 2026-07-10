# Code Intelligence Built-in (`conclave code`) — Design

- **Date:** 2026-07-10
- **Owner:** Detoro (lead, `4fb2198c-e0d9-4e4b-af9e-d4e72542bace`) · authority: in-loop
- **Status:** Approved design, pending implementation plan
- **Source material:** `/Users/detoro/code/skills/crates` — `codemap`, `codegraph`, `codegraph-core`, `astedit` (~6,000 LOC, tree-sitter based)

## Goal

Absorb the standalone code-exploration/refactoring tools (`codemap`, `codegraph`,
`astedit`) into Conclave as a first-class engine command family (`code.*`) with a
CLI surface (`conclave code …`), and improve them: one shared core, a per-root
index cache, six additional language grammars, and updated agent skills.

## Decisions (settled with the human, 2026-07-10)

1. **Integration shape: engine command family.** New `code.*` methods in
   `engine/router.rs`, implemented in `engine/commands/codeintel.rs`, mirroring
   the `browser.*` pattern. `conclave-cli` stays a thin client forwarding argv +
   caller `cwd` over UDS. Rejected: CLI-local execution (breaks the thin-client
   invariant, no central cache); sidecar binaries (repack only, no improvement).
2. **Improvements in scope:** unified core (kill `codemap`'s duplicated
   `lang.rs`/`walk.rs`/parser stack), per-workspace in-memory index cache,
   skill-file updates pointing at `conclave code`, and new languages.
3. **New languages:** Go, Swift, C, C++, Java, Kotlin — added to the existing
   Rust, TypeScript, TSX, JavaScript, Python (11 grammars total).

## Architecture

**Code location: one new lib crate** at `src-tauri/crates/codeintel`, consumed
by the `conclave` package as a path dependency. Rejected: vendoring as modules
under `engine/` (couples 6k+ LOC to the tauri lib compile unit); keeping the
original four-crate split (the three CLI bins become dead weight).

```
src-tauri/crates/codeintel/
  src/lang.rs    # single language registry, 11 grammars, one entry per language
  src/walk.rs    # single ignore-aware walker (codemap's duplicate is deleted)
  src/index.rs   # symbol/call index (from codegraph-core), now also backing map verbs
  src/cache.rs   # NEW: in-memory per-root index cache (see Cache)
  src/map.rs     # stats / files / tree / symbols / find   (from codemap)
  src/graph.rs   # callers / callees / refs / impact        (from codegraph)
  src/edit.rs    # rename / rewrite                         (from astedit)
src-tauri/src/engine/commands/codeintel.rs   # thin router wrapper: code.* → crate
```

- tree-sitter parsing is CPU-bound → engine runs it under
  `tokio::task::spawn_blocking`.
- Each grammar is one self-contained entry in `lang.rs`; a broken community
  grammar (see Risk ledger) is dropped from v1 without touching the others.

## Command surface

Flat verbs, one level under `code`:

```
conclave code stats|files|tree|symbols|find      # from codemap
conclave code callers|callees|refs|impact        # from codegraph
conclave code rename|rewrite [--apply]           # from astedit
```

Router methods `code.stats` … `code.rewrite` (11 verbs). Every verb keeps the
original tools' flag shape: `--json`, `--path <DIR>`, plus verb-specific flags
(`--exact`, `--lang`, `--pattern`, `--rewrite`, `--apply`). Root resolution:
`--path` made absolute against the caller's `cwd`; default root = caller `cwd`.

## Data flow

Example `conclave code callers foo`:

1. CLI forwards argv + `cwd` over UDS → router `code.callers`.
2. Wrapper resolves the root, requests the index from the cache.
3. Cache walks the file list (fast, ignore-aware); unchanged files
   (mtime+size) reuse their entries; changed files re-parse in
   `spawn_blocking`, confirmed by content hash (existing `hash.rs`).
4. Query answers as one JSON payload over UDS. Stay under the 4 MiB line cap:
   list-shaped verbs (`files`, `symbols`, `find`, `refs`) get `--limit`
   (default 200 entries) and set `truncated: true` when they cut.
5. LRU holds ≤ 8 roots. No disk persistence in v1.

## Writes (`rename` / `rewrite`)

- Dry-run remains the default; output is a diff summary. `--apply` writes.
- Writes are atomic per file (temp + rename) and **immediately invalidate the
  cache entries of touched files** — the standalone tools had no cache, so this
  hazard is new here.

## Error handling

- Nonexistent/non-dir root → explicit error with the resolved path.
- A file that fails to parse is skipped and reported in `warnings[]`; the
  command does not fail (existing core behavior).
- Engine not running → CLI reports the standard socket-unavailable error.

## Testing

- Port the source crates' existing test suites into `codeintel`.
- New fixture repos (small, per language) for the six new grammars.
- New tests: cache invalidation (edit file → query result changes), `--apply`
  leaves no stale cache, `--limit`/truncation behavior.
- Gates: `cargo test -p codeintel`, full `cargo build`, and an integration
  smoke run of `conclave code stats` against a real checkout.

## Skills update

- `ny-codemap` / `ny-codegraph` / `ny-astedit` scripts become shims: try
  `conclave code …` first; if the UDS socket is absent (app not running), fall
  back to the existing standalone binaries, which stay in place. Claude Code
  sessions outside Conclave keep working.
- Update the skills table in `~/.claude/CLAUDE.md` to the new commands.

## Risk ledger

- **Community grammars (Swift, Kotlin):** maintenance quality varies; may not
  compile against tree-sitter 0.25. Mitigation: per-language isolation in
  `lang.rs`; drop from v1 if broken, note in the outcome report.
- **Grammar version skew:** source crates pin tree-sitter 0.25 with 0.23/0.24
  grammar crates; adding six more multiplies the version matrix. Resolve at
  implementation time inside the one `codeintel` crate.
- **UDS payload cap (4 MiB/line):** enforced via default `--limit` +
  `truncated` flag rather than hoping outputs stay small.
- **Cache staleness after external writes:** mtime+size fast path can miss
  same-second same-size edits; content hash confirmation covers correctness
  where it matters (changed-file re-parse decision).
- **App binary growth:** 11 static grammars enlarge `conclave`. Accepted.

## Out of scope for v1 (recorded to prevent re-proposal)

- UI view for code intelligence (enabled later by living engine-side).
- Disk-persisted index (cold rebuild is seconds at this repo scale).
- Watch mode / background re-indexing.
