# Code Intelligence Built-in (`conclave code`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Absorb the four crates at `/Users/detoro/code/skills/crates` (codemap, codegraph, codegraph-core, astedit) into Conclave as one lib crate `src-tauri/crates/codeintel` exposed as the `code.*` engine command family / `conclave code <verb>` CLI, with a per-root index cache and six new language grammars.

**Architecture:** One new path-dep lib crate holds all tree-sitter logic (single lang registry, single walker, single index). The engine wraps it: `engine/commands/code.rs` handlers → router `code.*` methods → `cli.exec` allowlist arm → thin `run_code` dispatcher in conclave-cli that resolves the caller's cwd client-side (cwd does NOT travel on the UDS wire). A `CodeIntelCache` on `AppState` keeps per-root incremental indexes.

**Tech Stack:** Rust 2021 (toolchain 1.96.0), tree-sitter 0.25, ast-grep =0.38.7, rayon, serde/serde_json, ignore. Spec: `docs/superpowers/specs/2026-07-10-code-intel-builtin-design.md`.

## Global Constraints

- All work happens in the `codeup` repo (`/Users/detoro/code/codeup`) except Task 12 (files under `~/.claude/`). Source crates at `/Users/detoro/code/skills/crates` are READ-ONLY reference — never modify them.
- Rust: edition 2021, repo toolchain `1.96.0` (rust-toolchain.toml). `src-tauri/Cargo.toml` has `rust-version = "1.88"`; the new crate uses `rust-version = "1.88"` too (not the source's 1.74).
- Version pins copied verbatim from source: `tree-sitter = "0.25"`, `tree-sitter-rust = "0.24"`, `tree-sitter-typescript = "0.23"`, `tree-sitter-javascript = "0.23"`, `tree-sitter-python = "0.23"`, `ast-grep-core = "=0.38.7"`, `ast-grep-config = "=0.38.7"`, `ast-grep-language = "=0.38.7"`.
- Wire params (JSON-RPC `params`) are **camelCase**; the `data` payload keeps the source tools' **snake_case** field names verbatim (skill shims depend on them). Every verb's result is the envelope `{"schema_version": 1, "data": ...}` exactly like the source tools, plus optional top-level `"warnings"` and `"truncated"` keys.
- Every engine handler parses payload with `serde_json::from_value::<Req>(payload).map_err(|e| AppError::Invalid(e.to_string()))?` and runs tree-sitter work inside `tokio::task::spawn_blocking`.
- Any new `AppState` field MUST be initialized in BOTH `AppState::new()` (state.rs:83-99) and `AppState::for_tests()` (state.rs:198-214) — missing one breaks the build.
- List-shaped verbs (`files`, `symbols`, `find`, `refs`) take `--limit N` (default **200**); when they cut, set top-level `"truncated": true`.
- Commit after every task with a pathspec (`git commit -- <paths>`); never a bare `git commit` (shared checkout). Commit messages end with `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.
- A task that changes any `Cargo.toml` also regenerates `src-tauri/Cargo.lock` — include the lock in that task's commit pathspec. Standing ruling on challenge `853e8270` (task `codeintel-core-crate`): the lock rides with the dep change, even where a lane boundary listing omits it.
- Gates per task: `cargo test -p codeintel` (Tasks 1–8, 11), `cargo test` in `src-tauri` (Tasks 9–11), plus the task's own listed commands. Run gates from `src-tauri/`.
- A community grammar (Swift, Kotlin) that fails to compile against tree-sitter 0.25 is DROPPED from v1 (delete its lang entry + queries + fixtures + Cargo dep), noted in the task's commit message — per spec risk ledger. Do not fight it.
- UI copy, help text, and code comments are English only.

## File Structure (target)

```
src-tauri/Cargo.toml                         # + [workspace] members, + codeintel path dep
src-tauri/crates/codeintel/
  Cargo.toml
  src/lib.rs                                 # pub mod lang, walk, index, resolve, hash, error, map, graph, edit, cache; pub use output envelope helper
  src/{lang,walk,index,resolve,hash,error}.rs   # ported from codegraph-core
  src/queries/<lang>_{defs,imports,refs}.scm    # 12 ported + up to 18 new
  src/output.rs                              # {schema_version:1, data} envelope helper (deduped from the 3 source copies)
  src/map.rs                                 # stats/files/tree/symbols/find (from codemap, re-based onto core index)
  src/graph.rs                               # callers/callees/refs/impact (from codegraph)
  src/edit.rs + src/edit/{apply,rewrite,serialize}.rs   # from astedit
  src/cache.rs                               # NEW per-root incremental cache
  tests/…                                    # ported + new fixtures/tests
src-tauri/src/engine/state.rs                # + code_cache: Arc<codeintel::cache::CodeIntelCache>
src-tauri/src/engine/commands/code.rs        # NEW handlers (11 verbs)
src-tauri/src/engine/commands/mod.rs         # + pub mod code;
src-tauri/src/engine/router.rs               # + code.* arms
src-tauri/src/engine/commands/cli.rs         # + "code" arm (map_code_argv) + catch-all string
src-tauri/src/bin/conclave-cli.rs            # + run_code + USAGE lines
~/.claude/skills/ny-{codemap,codegraph,astedit}/scripts/*   # shims (Task 12)
~/.claude/CLAUDE.md                          # skills table update (Task 12)
```

---

### Task 1: Scaffold `codeintel` crate + workspace wiring (port codegraph-core verbatim)

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Create: `src-tauri/crates/codeintel/Cargo.toml`
- Create: `src-tauri/crates/codeintel/src/{lib,lang,walk,index,resolve,hash,error,output}.rs`
- Create: `src-tauri/crates/codeintel/src/queries/*.scm` (12 files)
- Test: `src-tauri/crates/codeintel/tests/{hash_test,file_meta_test,reexport_test}.rs` + `tests/fixtures/reexport_*/`

**Interfaces:**
- Produces: crate `codeintel` with `codeintel::index::build_index(root: &Path) -> anyhow::Result<Index>`, `codeintel::resolve::resolve_refs<'a>(idx: &'a Index, target: &str) -> Vec<Resolved<'a>>`, `codeintel::walk::walk_sources`, `codeintel::hash::compute_file_hash`, `codeintel::output::envelope(data: serde_json::Value) -> serde_json::Value`, all types (`Index`, `Definition`, `Import`, `Reference`, `FileMeta`, `AliasSite`, `WildcardSite`, `DefKind`, `RefKind`, `Language`, `QueryKind`, `Confidence`, `ResolveReason`, `CoreError`) with signatures identical to codegraph-core.

- [ ] **Step 1: Add workspace + path dep to `src-tauri/Cargo.toml`**

Append at the very bottom of `src-tauri/Cargo.toml`:

```toml
[workspace]
members = ["crates/codeintel"]
```

and under the existing `[dependencies]` section add:

```toml
codeintel = { path = "crates/codeintel" }
```

- [ ] **Step 2: Create `src-tauri/crates/codeintel/Cargo.toml`**

```toml
[package]
name = "codeintel"
description = "Code intelligence core for Conclave: survey, cross-reference, and AST-edit a codebase."
version = "0.1.0"
edition = "2021"
rust-version = "1.88"
license = "MIT"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
ignore = "0.4"
thiserror = "2"
sha2 = "0.10"
hex = "0.4"
rayon = "1"
tree-sitter = "0.25"
tree-sitter-rust = "0.24"
tree-sitter-typescript = "0.23"
tree-sitter-javascript = "0.23"
tree-sitter-python = "0.23"

[dev-dependencies]
tempfile = "3"
```

(Note: `thiserror = "2"` to match the host crate — the source used 1; the derive syntax is unchanged.)

- [ ] **Step 3: Copy codegraph-core sources verbatim**

```bash
SRC=/Users/detoro/code/skills/crates/codegraph-core/src
DST=src-tauri/crates/codeintel/src
cp $SRC/{lang.rs,walk.rs,index.rs,resolve.rs,hash.rs,error.rs} $DST/
cp -R $SRC/queries $DST/queries
```

Create `$DST/lib.rs`:

```rust
//! Code intelligence core for Conclave: one language registry, one walker,
//! one index, shared by the map/graph/edit command families.
pub mod error;
pub mod hash;
pub mod index;
pub mod lang;
pub mod output;
pub mod resolve;
pub mod walk;

pub use error::CoreError;
```

Create `$DST/output.rs` (the deduped envelope helper — same shape all three source tools used):

```rust
use serde_json::{json, Value};

/// Wrap a command payload in the stable wire envelope.
pub fn envelope(data: Value) -> Value {
    json!({ "schema_version": 1, "data": data })
}
```

- [ ] **Step 4: Copy the codegraph-core test suite + fixtures**

```bash
cp -R /Users/detoro/code/skills/crates/codegraph-core/tests src-tauri/crates/codeintel/tests
```

(brings `hash_test.rs`, `file_meta_test.rs`, `reexport_test.rs`, `fixtures/reexport_{rust,ts,js,py}/`). Replace every `codegraph_core::` path in the test files with `codeintel::` (`grep -rl codegraph_core tests/ | xargs sed -i '' 's/codegraph_core/codeintel/g'`).

- [ ] **Step 5: Run the tests, expect green**

Run from `src-tauri/`: `cargo test -p codeintel`
Expected: 17 tests pass (6 hash + 1 file_meta + 10 reexport), 0 failures.

- [ ] **Step 6: Full host build still green**

Run: `cargo build --bin conclave-cli`
Expected: compiles (the dep is linked but unused — allow it for now; `cargo build` not `clippy -D warnings` is the gate here).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/crates/codeintel
git commit -m "feat(codeintel): scaffold crate — port codegraph-core (lang/walk/index/resolve/hash/error + 12 queries + tests)

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>" -- src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/crates/codeintel
```

---

### Task 2: Extend the index for map verbs (`end_line`, `signature`, per-file lines/language)

The map family (Task 3) derives codemap's output from the core index instead of codemap's duplicate parser stack. codemap's `Symbol` carries `start_line`/`end_line`/`signature`; core's `Definition` lacks them. codemap's `stats`/`files` need per-file line counts and language names; core's `FileMeta` only has `len`.

**Files:**
- Modify: `src-tauri/crates/codeintel/src/index.rs`
- Test: `src-tauri/crates/codeintel/tests/index_ext_test.rs` (new)

**Interfaces:**
- Produces: `Definition` gains `pub end_line: usize` and `pub signature: Option<String>` (first source line of the def node, trimmed, truncated to 120 chars — codemap's rule). `FileMeta` gains `pub lines: usize` and `pub language: &'static str` (from `Language::name()`). `Index` gains `pub warnings: Vec<String>` — one entry `"failed to parse <rel_path>"` per source file the indexer skipped (today `index_file` returns `None` silently on read/parse failure). All populated by `build_index`; all existing fields unchanged.

- [ ] **Step 1: Write the failing test** — `tests/index_ext_test.rs`:

```rust
use codeintel::index::build_index;
use std::fs;

#[test]
fn definitions_carry_end_line_and_signature() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("lib.rs"),
        "pub fn greet(name: &str) -> String {\n    format!(\"hi {name}\")\n}\n",
    )
    .unwrap();
    let idx = build_index(dir.path()).unwrap();
    let def = idx.definitions.iter().find(|d| d.name == "greet").unwrap();
    assert_eq!(def.line, 1);
    assert_eq!(def.end_line, 3);
    assert_eq!(def.signature.as_deref(), Some("pub fn greet(name: &str) -> String {"));
}

#[test]
fn file_meta_carries_lines_and_language() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("lib.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    let idx = build_index(dir.path()).unwrap();
    let meta = idx.file_meta.get("lib.rs").unwrap();
    assert_eq!(meta.lines, 2);
    assert_eq!(meta.language, "rust");
}

#[test]
fn unparseable_file_lands_in_warnings() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("ok.rs"), "fn a() {}\n").unwrap();
    fs::write(dir.path().join("bad.rs"), [0xFF, 0xFE, 0x00, 0xD8]).unwrap(); // invalid UTF-8
    let idx = build_index(dir.path()).unwrap();
    assert!(idx.definitions.iter().any(|d| d.name == "a"));
    assert!(idx.warnings.iter().any(|w| w.contains("bad.rs")), "warnings: {:?}", idx.warnings);
}

#[test]
fn signature_is_truncated_to_120_chars() {
    let dir = tempfile::tempdir().unwrap();
    let long = format!("fn f(a: {}) {{}}\n", "u8, ".repeat(60));
    fs::write(dir.path().join("lib.rs"), long).unwrap();
    let idx = build_index(dir.path()).unwrap();
    let def = idx.definitions.iter().find(|d| d.name == "f").unwrap();
    assert_eq!(def.signature.as_ref().unwrap().chars().count(), 120);
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p codeintel --test index_ext_test`. Expected: compile error (`end_line` unknown field).

- [ ] **Step 3: Implement** — in `index.rs`: add the two fields to `Definition` and the two to `FileMeta` (give `FileMeta` `language: &'static str` — update its `Default` impl usage accordingly; if `#[derive(Default)]` breaks on `&'static str`, implement `Default` by hand with `language: ""`). In the def-extraction path (`index_defs`), the captured def node is in hand: set `end_line = node.end_position().row + 1`; build `signature` from the node's first source line: `src[node.start_byte()..].lines().next()` trimmed, `.chars().take(120).collect()`, `Some(...)` always for defs. In the per-file entry point where `FileMeta { len }` is built, also count `lines = src.lines().count()` and set `language = language.name()`. In `build_index`'s assembly loop, a file whose `index_file` returned `None` appends `format!("failed to parse {rel_path}")` to `idx.warnings` instead of vanishing (change the rayon closure to return `Result`-like `(rel_path, Option<partial>)` pairs so the miss is observable).

- [ ] **Step 4: Run all crate tests** — `cargo test -p codeintel`. Expected: PASS (old tests untouched — new fields are additive; `Definition` is constructed in one place).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/codeintel
git commit -m "feat(codeintel): index carries end_line/signature per definition, lines/language per file

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>" -- src-tauri/crates/codeintel
```

---

### Task 3: `map.rs` — stats / files / tree / symbols / find on the shared index

Re-implement codemap's five verbs as library functions over the core index. codemap's own `lang.rs`/`walk.rs`/`symbols.rs` and its 4 combined `.scm` files are NOT ported — that duplication dies here. Behavior delta (intentional, an improvement): `symbols`/`find`/`stats` now also see `method` definitions (core indexes impl methods; codemap didn't).

**Files:**
- Create: `src-tauri/crates/codeintel/src/map.rs` (+ `pub mod map;` in lib.rs)
- Test: `src-tauri/crates/codeintel/tests/map_test.rs`, fixture `tests/fixtures/sample_project/` (copy from `/Users/detoro/code/skills/crates/codemap/tests/fixtures/sample_project/`)

**Interfaces:**
- Consumes: `Index` (Task 2 fields), `walk::{default_walker, walk_sources}`, `output::envelope`.
- Produces (all return `anyhow::Result<serde_json::Value>`, the **inner `data` value**, not the envelope — the engine wraps):
  - `pub fn stats(root: &Path, idx: &Index) -> Result<Value>` — `{total_files, total_lines, languages: {name: {files, lines}}, symbols: {kind: count}}` (BTreeMaps, sorted).
  - `pub fn files(idx: &Index, limit: usize) -> Result<(Value, bool)>` — array of `{path, language, lines, size_bytes}`, sorted by path; bool = truncated.
  - `pub fn tree(root: &Path) -> Result<Value>` — recursive `{name, children?, is_dir}` via `default_walker` (index-free).
  - `pub fn symbols(idx: &Index, target: Option<&str>, all: bool, kinds: &[String], limit: usize) -> Result<(Value, bool)>` — array of `{file, name, kind, start_line, end_line, signature?}`.
  - `pub fn find(idx: &Index, name: &str, exact: bool, limit: usize) -> Result<(Value, bool)>` — same element shape as `symbols`, sorted by (file, start_line).
- Kind mapping: `DefKind` serializes lowercase already (`fn|struct|enum|trait|class|interface|type|const|method`); the `kinds` filter accepts those strings plus `function` as alias of `fn` (codemap's `SymbolKind::parse` rule).

- [ ] **Step 1: Copy the fixture, write the failing tests** — port the 13 assertions from codemap's `files_test.rs`/`find_test.rs`/`stats_test.rs`/`symbols_test.rs` as **library calls** instead of binary spawns. Representative shape (write all of them in this style; one test fn per original test):

```rust
use codeintel::{index::build_index, map};
use std::path::Path;

fn fixture() -> &'static Path { Path::new("tests/fixtures/sample_project") }

#[test]
fn files_lists_all_supported_extensions() {
    let idx = build_index(fixture()).unwrap();
    let (data, truncated) = map::files(&idx, 200).unwrap();
    assert!(!truncated);
    let paths: Vec<&str> = data.as_array().unwrap().iter()
        .map(|f| f["path"].as_str().unwrap()).collect();
    for p in ["src/lib.rs", "src/component.tsx", "src/types.ts", "src/util.js", "app.py"] {
        assert!(paths.contains(&p), "missing {p}");
    }
    let rs = data.as_array().unwrap().iter().find(|f| f["path"] == "src/lib.rs").unwrap();
    assert!(rs["lines"].as_u64().unwrap() > 0);
    assert_eq!(rs["language"], "rust");
}

#[test]
fn find_exact_returns_only_exact_name() {
    let idx = build_index(fixture()).unwrap();
    let (data, _) = map::find(&idx, "greet", true, 200).unwrap();
    assert!(data.as_array().unwrap().iter().all(|s| s["name"] == "greet"));
}

#[test]
fn symbols_kind_filter_and_limit_truncate() {
    let idx = build_index(fixture()).unwrap();
    let (all, _) = map::symbols(&idx, None, true, &[], 200).unwrap();
    let n = all.as_array().unwrap().len();
    assert!(n > 2);
    let (cut, truncated) = map::symbols(&idx, None, true, &[], 2).unwrap();
    assert_eq!(cut.as_array().unwrap().len(), 2);
    assert!(truncated);
    let (fns, _) = map::symbols(&idx, None, true, &["fn".into()], 200).unwrap();
    assert!(fns.as_array().unwrap().iter().all(|s| s["kind"] == "fn" || s["kind"] == "method"));
}
```

(plus the tree test asserting the recursive `{name, children, is_dir}` shape, the stats test asserting per-language/per-kind maps, and the remaining ports.)

- [ ] **Step 2: Run to verify failure** — `cargo test -p codeintel --test map_test`. Expected: compile error (`map` module missing).

- [ ] **Step 3: Implement `map.rs`** — reference implementations live in `/Users/detoro/code/skills/crates/codemap/src/commands/{stats,files,tree,symbols,find}.rs`; port their logic with these substitutions: iterate `idx.definitions` (mapping `Definition{name, kind, line, end_line, signature, file}` to the symbol element shape with `start_line: line`) instead of `symbols::extract_file`; take `lines`/`size_bytes`/`language` from `idx.file_meta` (`size_bytes` = `meta.len`) instead of re-reading files; `tree` ports as-is (walker only). `symbols` target rule (from codemap): `target == Some(".")` or `all` ⇒ whole project; otherwise filter `idx.definitions` to `file == target`. Apply `limit` after sorting; return `(value, len_before > limit)`.

- [ ] **Step 4: Run tests** — `cargo test -p codeintel`. Expected: PASS.

- [ ] **Step 5: Commit** (pathspec `src-tauri/crates/codeintel`, message `feat(codeintel): map verbs (stats/files/tree/symbols/find) on the shared index`).

---

### Task 4: `graph.rs` — callers / callees / refs / impact

**Files:**
- Create: `src-tauri/crates/codeintel/src/graph.rs` (+ `pub mod graph;`)
- Test: `src-tauri/crates/codeintel/tests/graph_test.rs`, fixture `tests/fixtures/multi_lang/` (copy from `/Users/detoro/code/skills/crates/codegraph/tests/fixtures/multi_lang/`)

**Interfaces:**
- Consumes: `Index`, `resolve::resolve_refs`, `Index::enclosing_definition`.
- Produces (inner `data` values; element shapes byte-compatible with codegraph's JSON):
  - `pub fn find_refs(idx: &Index, name: &str, limit: usize) -> Result<(Value, bool)>` — `{file,line,column,kind,name,context,confidence,reason}`, kind ∈ `definition|call|reference`.
  - `pub fn callers(idx: &Index, name: &str, depth: usize) -> Result<Value>` — `{file,line,column,name,kind,distance,confidence,reason,sites:[{file,line,column,context}]}`, depth capped at 8.
  - `pub fn callees(idx: &Index, name: &str, depth: usize) -> Result<Value>` — `{name,kind,def_file,def_line,distance,confidence,reason,sites:[…]}` (`def_file`/`def_line` nullable, keys always present), depth capped at 8.
  - `pub fn impact(idx: &Index, name: &str) -> Result<Value>` — `{name,kind,file,line,distance,confidence,reason}`, depth capped at 6.

- [ ] **Step 1: Copy fixture + write failing tests** — port codegraph's 13 behavioral tests (`find_refs_test.rs` 6, `callers_test.rs` 3, `callees_test.rs` 2, `impact_test.rs` 2) as library calls against `tests/fixtures/multi_lang/...` subdirs, in the same style as Task 3 Step 1 (e.g. `let (data,_) = graph::find_refs(&idx, "authenticate", 200)` then assert on `confidence == "high"` / entry sets). Skip the two binary-CLI tests (`cli_help_lists_all_four_subcommands`, help text) — there is no binary anymore. Port `index_test.rs`'s 4 index-behavior tests too (they exercise core paths not covered by Task 1's suite).

- [ ] **Step 2: Verify failure** — `cargo test -p codeintel --test graph_test` → compile error.

- [ ] **Step 3: Implement** — port `/Users/detoro/code/skills/crates/codegraph/src/commands/{find_refs,callers,callees,impact}.rs` bodies into `graph.rs` functions, replacing clap arg structs with the parameters above and `print_json(envelope(...))` with returning the inner value. Keep `HARD_CAP = 8` and `MAX_DEPTH = 6` constants. `find_refs` applies `limit` after the (file,line) sort; the other three return unlimited (bounded by depth caps).

- [ ] **Step 4: Run** — `cargo test -p codeintel` → PASS.

- [ ] **Step 5: Commit** (pathspec, `feat(codeintel): graph verbs (callers/callees/refs/impact)`).

---

### Task 5: `edit.rs` — rename / rewrite (astedit port)

**Files:**
- Modify: `src-tauri/crates/codeintel/Cargo.toml` (add the three `=0.38.7` ast-grep deps)
- Create: `src-tauri/crates/codeintel/src/edit.rs`, `src/edit/{apply,rewrite,serialize,rename_cmd,rewrite_cmd}.rs` (+ `pub mod edit;`)
- Test: `src-tauri/crates/codeintel/tests/edit_test.rs`, fixtures `tests/fixtures/{same_file,cross_file_import,glob_import,name_only,alias_reexport,wildcard_reexport,multi_def,apply_write,rewrite_rust,rewrite_typescript,rewrite_tsx,rewrite_javascript,rewrite_python,rewrite_metavar,rewrite_multimatch,rewrite_no_match,rewrite_lang_filter,rewrite_apply}/` (copy all from `/Users/detoro/code/skills/crates/astedit/tests/fixtures/`)

**Interfaces:**
- Consumes: `Index`, `resolve_refs`, `hash::compute_file_hash`, `walk::walk_sources`.
- Produces:
  - `pub fn rename(root: &Path, idx: &Index, old: &str, new: &str, apply: bool, lang: Option<&str>, anchor: Option<&str>) -> Result<(Value, Vec<String>)>` — inner `data` = astedit's `RenameData` shape verbatim (`{subcommand:"rename", dry_run, needs_anchor?, candidates?, applied?, skipped?, errors?}`); second element = repo-relative paths of files actually written (empty on dry-run) — Task 11 feeds these to cache invalidation.
  - `pub fn rewrite(root: &Path, pattern: &str, template: &str, apply: bool, lang: Option<&str>) -> Result<(Value, Vec<String>)>` — `RewriteData` shape verbatim.
  - The former process exit code 2 (needs_anchor / pattern-compile) is NOT an error here — it is fully represented in the payload; callers get `Ok`.

- [ ] **Step 1: Copy sources + fixtures**

```bash
SRC=/Users/detoro/code/skills/crates/astedit/src
DST=src-tauri/crates/codeintel/src/edit
mkdir -p $DST
cp $SRC/apply.rs $SRC/rewrite.rs $SRC/serialize.rs $DST/
cp $SRC/commands/rename.rs $DST/rename_cmd.rs
cp $SRC/commands/rewrite.rs $DST/rewrite_cmd.rs
cp -R /Users/detoro/code/skills/crates/astedit/tests/fixtures/* src-tauri/crates/codeintel/tests/fixtures/
```

Create `src/edit.rs` as the module root: `pub mod apply; pub mod rewrite; pub mod serialize; mod rename_cmd; mod rewrite_cmd;` re-exporting `pub use rename_cmd::rename; pub use rewrite_cmd::rewrite;`. Merge astedit's `AstEditError` enum into the ported files (bring `error.rs` content into `edit.rs` or a `edit/error.rs` — keep `kind()` strings identical: `parse-error|hash-mismatch|concurrent-write|node-kind-mismatch|write-failed|pattern-compile`).

- [ ] **Step 2: Write failing tests** — port astedit's 10 rename + 12 rewrite integration tests as library calls: replace `run_astedit_json(args)` with direct `edit::rename(...)`/`edit::rewrite(...)` on a `copy_fixture` tempdir (port `tests/common/mod.rs::copy_fixture` as a helper fn inside `edit_test.rs`). Exit-code assertions translate: old `exit 2` + needs_anchor ⇒ assert `data["needs_anchor"] == true`; old pattern-compile `exit 2` ⇒ assert `data["errors"][0]["error_kind"] == "pattern-compile"`. Also port the 13 inline unit tests from `apply.rs`/`rewrite.rs`/`serialize.rs`/`error.rs` (they come along in the copied files — just make their `use` paths compile).

- [ ] **Step 3: Verify failure** — `cargo test -p codeintel --test edit_test` → compile errors.

- [ ] **Step 4: Make it compile and pass** — in the ported files replace `codegraph_core::` with `crate::`, replace clap arg structs with the fn parameters, replace `print_json`/exit-code plumbing with returning `(serde_json::to_value(RenameData{...})?, written_files)`. `rename_cmd` internally calls `build_index` today — change it to take `idx: &Index` (the engine provides the cached index; `rewrite` keeps its own `walk_sources` since it never used the index). Add the ast-grep deps to Cargo.toml exactly as pinned in Global Constraints.

- [ ] **Step 5: Run** — `cargo test -p codeintel` → PASS (expect ~35 new tests green).

- [ ] **Step 6: Commit** (pathspec, `feat(codeintel): edit verbs (rename/rewrite) — astedit port, exit codes folded into payload`).

---

### Task 6: `cache.rs` — per-root incremental index cache

**Files:**
- Modify: `src-tauri/crates/codeintel/src/index.rs` (expose per-file indexing)
- Create: `src-tauri/crates/codeintel/src/cache.rs` (+ `pub mod cache;`)
- Test: `src-tauri/crates/codeintel/tests/cache_test.rs`

**Interfaces:**
- Consumes: `index_file` internals, `walk_sources`, `hash::compute_file_hash`.
- Produces:
  - In `index.rs`: `pub(crate) struct FilePartial { pub defs: Vec<Definition>, pub imports: Vec<Import>, pub refs: Vec<Reference>, pub meta: FileMeta, pub alias_sites: Vec<AliasSite>, pub wildcard_sites: Vec<WildcardSite> }` and `pub(crate) fn index_one_file(...) -> Option<FilePartial>` refactored out of the existing rayon closure (`build_index` now assembles from partials — behavior unchanged).
  - In `cache.rs`:
    ```rust
    pub struct CodeIntelCache { /* Mutex<lru list + HashMap<PathBuf, CachedRoot>> */ }
    impl CodeIntelCache {
        pub fn new() -> Self;                          // capacity 8 roots
        pub fn get_index(&self, root: &Path) -> anyhow::Result<Arc<Index>>;
        pub fn invalidate_files(&self, root: &Path, rel_paths: &[String]);
        pub fn invalidate_root(&self, root: &Path);
    }
    ```
  - Refresh algorithm inside `get_index`: `walk_sources(root)` → for each file `stat` (mtime+size); unchanged entry ⇒ reuse partial; changed/new ⇒ `compute_file_hash`; if hash equals stored hash ⇒ refresh stat only; else re-parse via `index_one_file` (rayon over the changed set); files gone from the walk are dropped. Any change (or first build) re-assembles and stores `Arc<Index>`; no change returns the stored `Arc` untouched. `CodeIntelCache` must be `Send + Sync` (`Mutex` held only around the map, not during parsing of a cold root is NOT required for v1 — holding it for the whole refresh is acceptable and simpler; callers are already on `spawn_blocking`).

- [ ] **Step 1: Write failing tests** — `tests/cache_test.rs`:

```rust
use codeintel::cache::CodeIntelCache;
use std::fs;

#[test]
fn warm_hit_returns_same_arc_when_nothing_changed() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("lib.rs"), "fn a() {}\n").unwrap();
    let cache = CodeIntelCache::new();
    let one = cache.get_index(dir.path()).unwrap();
    let two = cache.get_index(dir.path()).unwrap();
    assert!(std::sync::Arc::ptr_eq(&one, &two));
}

#[test]
fn edited_file_is_reindexed() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("lib.rs");
    fs::write(&f, "fn a() {}\n").unwrap();
    let cache = CodeIntelCache::new();
    assert!(cache.get_index(dir.path()).unwrap().definitions.iter().any(|d| d.name == "a"));
    fs::write(&f, "fn b() {}\n").unwrap();
    let idx = cache.get_index(dir.path()).unwrap();
    assert!(idx.definitions.iter().any(|d| d.name == "b"));
    assert!(!idx.definitions.iter().any(|d| d.name == "a"));
}

#[test]
fn deleted_file_leaves_the_index() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("lib.rs");
    fs::write(&f, "fn a() {}\n").unwrap();
    fs::write(dir.path().join("keep.rs"), "fn k() {}\n").unwrap();
    let cache = CodeIntelCache::new();
    cache.get_index(dir.path()).unwrap();
    fs::remove_file(&f).unwrap();
    let idx = cache.get_index(dir.path()).unwrap();
    assert!(!idx.definitions.iter().any(|d| d.name == "a"));
    assert!(idx.definitions.iter().any(|d| d.name == "k"));
}

#[test]
fn invalidate_files_forces_reparse_even_with_same_stat() {
    // invalidate_files must drop the entry outright: same-second same-size
    // edits are exactly the case mtime+size cannot see.
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("lib.rs");
    fs::write(&f, "fn aa() {}\n").unwrap();
    let cache = CodeIntelCache::new();
    cache.get_index(dir.path()).unwrap();
    fs::write(&f, "fn bb() {}\n").unwrap(); // same length; mtime may collide
    cache.invalidate_files(dir.path(), &["lib.rs".to_string()]);
    let idx = cache.get_index(dir.path()).unwrap();
    assert!(idx.definitions.iter().any(|d| d.name == "bb"));
}

#[test]
fn lru_evicts_past_eight_roots() {
    let cache = CodeIntelCache::new();
    let dirs: Vec<_> = (0..9).map(|_| tempfile::tempdir().unwrap()).collect();
    for d in &dirs { fs::write(d.path().join("a.rs"), "fn a() {}\n").unwrap(); cache.get_index(d.path()).unwrap(); }
    let first = cache.get_index(dirs[0].path()).unwrap(); // evicted → rebuilt, still correct
    assert!(first.definitions.iter().any(|d| d.name == "a"));
}
```

- [ ] **Step 2: Verify failure** — `cargo test -p codeintel --test cache_test` → compile error.

- [ ] **Step 3: Refactor `index.rs`** — extract the body of the existing `par_iter().filter_map(|f| index_file(...))` closure into `pub(crate) fn index_one_file(file: &SourceFile, root: &Path, queries: &LangQueries) -> Option<FilePartial>` (make `LangQueries` construction reusable: `pub(crate) fn compile_queries() -> LangQueries`). `build_index` keeps its exact public signature and behavior, now `walk → par_iter → index_one_file → assemble`. Run `cargo test -p codeintel` — everything from Tasks 1–5 must stay green before proceeding.

- [ ] **Step 4: Implement `cache.rs`** per the interface block (store per-file `(mtime, size, FileHash, FilePartial)` keyed by rel path inside `CachedRoot`; a file whose re-parse returns `None` is stored as a parse-failure marker so the assembled `Index.warnings` still names it; assemble `Index` by concatenating partials in sorted-rel-path order so output ordering matches `build_index`; keep an `Arc<Index>` alongside, rebuilt only when any entry changed; LRU = `Vec<PathBuf>` touch order, evict front past 8).

- [ ] **Step 5: Run** — `cargo test -p codeintel` → PASS.

- [ ] **Step 6: Commit** (pathspec, `feat(codeintel): per-root incremental index cache (mtime/size fast path, hash confirm, LRU 8)`).

---

### Task 7: New grammars, batch A — Go, C, C++, Java (upstream, low risk)

**Files:**
- Modify: `src-tauri/crates/codeintel/Cargo.toml`, `src/lang.rs`
- Create: `src/queries/{go,c,cpp,java}_{defs,imports,refs}.scm` (12 files)
- Test: `src-tauri/crates/codeintel/tests/lang_new_test.rs`, fixtures `tests/fixtures/lang_new/{go,c,cpp,java}/`

**Interfaces:**
- Consumes: the `lang.rs` four-arm wiring pattern (name / from_extension / ts_language / query_source) and the `.scm` capture-name contract: defs `@name` + `@def.<kind>`; refs `@name` + `@ref.call`/`@ref.reference`; imports `@path @name @alias @original @group @import` (+ `@reexport_alias` / `@reexport_wildcard` / `@vis` where the language has re-exports — Go/C/C++/Java do not; their `_imports.scm` may use only the basic captures).
- Produces: `Language::{Go, C, Cpp, Java}` variants; extensions `go` / `c`,`h` / `cc`,`cpp`,`cxx`,`hpp`,`hh` / `java`.

- [ ] **Step 1: Add deps**

```toml
tree-sitter-go = "0.23"
tree-sitter-c = "0.23"
tree-sitter-cpp = "0.23"
tree-sitter-java = "0.23"
```

Run `cargo build -p codeintel`. If a pin is rejected or its API mismatches tree-sitter 0.25, take the nearest release that exposes `LANGUAGE: LanguageFn` and compiles (`cargo add tree-sitter-go` shows the latest); record the final pins in the commit message.

- [ ] **Step 2: Write failing fixture tests** — one fixture file + test per language asserting symbol extraction AND cross-file call resolution. Fixture content (create exactly these):

`tests/fixtures/lang_new/go/main.go`:
```go
package main

func Greet(name string) string { return "hi " + name }

func main() { println(Greet("x")) }
```
`tests/fixtures/lang_new/c/main.c`:
```c
int add(int a, int b) { return a + b; }
int main(void) { return add(1, 2); }
```
`tests/fixtures/lang_new/cpp/shape.cpp`:
```cpp
class Shape { public: int area(); };
int Shape::area() { return 0; }
int use() { Shape s; return s.area(); }
```
`tests/fixtures/lang_new/java/App.java`:
```java
public class App {
    static int add(int a, int b) { return a + b; }
    public static void main(String[] args) { add(1, 2); }
}
```

`tests/lang_new_test.rs` (same pattern for every language — write all four):
```rust
use codeintel::{graph, index::build_index};
use std::path::Path;

#[test]
fn go_symbols_and_call_refs() {
    let idx = build_index(Path::new("tests/fixtures/lang_new/go")).unwrap();
    assert!(idx.definitions.iter().any(|d| d.name == "Greet" && d.file == "main.go"));
    let (data, _) = graph::find_refs(&idx, "Greet", 200).unwrap();
    let kinds: Vec<&str> = data.as_array().unwrap().iter().map(|h| h["kind"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"definition"));
    assert!(kinds.contains(&"call"));
}
// + c_symbols_and_call_refs (add), cpp_class_method_and_call (Shape/area: expect
//   kinds struct-or-class + method captured), java_class_method_and_call (App/add)
```

- [ ] **Step 3: Verify failure** — `cargo test -p codeintel --test lang_new_test` → fails (no defs found / unknown extension).

- [ ] **Step 4: Wire `lang.rs` + write the queries** — add the four variants to every match (name, from_extension, ts_language via `tree_sitter_go::LANGUAGE.into()` etc., query_source `include_str!`). First-draft `.scm` (adjust node names against the real grammar until tests pass — run `cargo test` and use tree-sitter parse errors/empty results as the signal; the TEST is the contract, the draft below is a starting point):

`go_defs.scm`:
```scheme
(function_declaration name: (identifier) @name) @def.fn
(method_declaration name: (field_identifier) @name) @def.method
(type_declaration (type_spec name: (type_identifier) @name type: (struct_type))) @def.struct
(type_declaration (type_spec name: (type_identifier) @name type: (interface_type))) @def.interface
(type_declaration (type_spec name: (type_identifier) @name)) @def.type
(const_declaration (const_spec name: (identifier) @name)) @def.const
```
`go_refs.scm`:
```scheme
(call_expression function: (identifier) @name) @ref.call
(call_expression function: (selector_expression field: (field_identifier) @name)) @ref.call
(type_identifier) @name @ref.reference
```
`go_imports.scm`:
```scheme
(import_spec path: (interpreted_string_literal) @path) @import
(import_spec name: (package_identifier) @alias path: (interpreted_string_literal) @path) @import
```
`c_defs.scm`:
```scheme
(function_definition declarator: (function_declarator declarator: (identifier) @name)) @def.fn
(struct_specifier name: (type_identifier) @name body: (field_declaration_list)) @def.struct
(enum_specifier name: (type_identifier) @name body: (enumerator_list)) @def.enum
(type_definition declarator: (type_identifier) @name) @def.type
```
`c_refs.scm`:
```scheme
(call_expression function: (identifier) @name) @ref.call
(type_identifier) @name @ref.reference
```
`c_imports.scm`:
```scheme
(preproc_include path: (string_literal) @path) @import
(preproc_include path: (system_lib_string) @path) @import
```
`cpp_defs.scm`: c_defs plus:
```scheme
(class_specifier name: (type_identifier) @name body: (field_declaration_list)) @def.class
(function_definition declarator: (function_declarator declarator: (qualified_identifier name: (identifier) @name))) @def.method
(function_definition declarator: (function_declarator declarator: (field_identifier) @name)) @def.method
```
`cpp_refs.scm`: c_refs plus:
```scheme
(call_expression function: (field_expression field: (field_identifier) @name)) @ref.call
```
`cpp_imports.scm`: same as `c_imports.scm`.
`java_defs.scm`:
```scheme
(class_declaration name: (identifier) @name) @def.class
(interface_declaration name: (identifier) @name) @def.interface
(enum_declaration name: (identifier) @name) @def.enum
(method_declaration name: (identifier) @name) @def.method
(constant_declaration declarator: (variable_declarator name: (identifier) @name)) @def.const
```
`java_refs.scm`:
```scheme
(method_invocation name: (identifier) @name) @ref.call
(object_creation_expression type: (type_identifier) @name) @ref.call
(type_identifier) @name @ref.reference
```
`java_imports.scm`:
```scheme
(import_declaration (scoped_identifier) @path) @import
```

- [ ] **Step 5: Iterate until green** — `cargo test -p codeintel --test lang_new_test`; on empty results, dump the parse tree of the fixture (`tree_sitter::Parser` in a scratch test or `println!` the root node's s-expression) and fix node names. Then full `cargo test -p codeintel` (existing languages must be unaffected).

- [ ] **Step 6: Commit** (pathspec, `feat(codeintel): Go/C/C++/Java grammars + queries + fixtures`).

---

### Task 8: New grammars, batch B — Swift, Kotlin (community; droppable per spec)

**Files:** same shape as Task 7 — `Cargo.toml`, `lang.rs`, `src/queries/{swift,kotlin}_{defs,imports,refs}.scm`, fixtures `tests/fixtures/lang_new/{swift,kotlin}/`, tests appended to `tests/lang_new_test.rs`.

**Interfaces:** `Language::{Swift, Kotlin}`; extensions `swift` / `kt`,`kts`.

- [ ] **Step 1: Probe the deps** — try, in order, until one compiles against tree-sitter 0.25: Swift: `tree-sitter-swift = "0.7"`; Kotlin: `tree-sitter-kotlin-ng = "1.1"`, then `tree-sitter-kotlin = "0.3"`. `cargo build -p codeintel` after each. **If no candidate for a language compiles within two attempts, DROP that language** (revert its dep + entries), note it in the commit message, and finish the task with the survivor(s) — this is the pre-authorized spec ruling, not a failure.
- [ ] **Step 2: Fixtures + failing tests** — `swift/main.swift`: `func greet(name: String) -> String { return "hi " + name }\nfunc caller() { _ = greet(name: "x") }`; `kotlin/App.kt`: `fun add(a: Int, b: Int): Int = a + b\nfun main() { add(1, 2) }`. Tests mirror Task 7 Step 2 (defs found + call ref resolves).
- [ ] **Step 3: Wire + queries** — first drafts: `swift_defs.scm`: `(function_declaration name: (simple_identifier) @name) @def.fn`, `(class_declaration name: (type_identifier) @name) @def.class`, `(protocol_declaration name: (type_identifier) @name) @def.interface`; `swift_refs.scm`: `(call_expression (simple_identifier) @name) @ref.call`, `(type_identifier) @name @ref.reference`; `swift_imports.scm`: `(import_declaration (identifier) @path) @import`. `kotlin_defs.scm`: `(function_declaration (simple_identifier) @name) @def.fn`, `(class_declaration (type_identifier) @name) @def.class`, `(object_declaration (type_identifier) @name) @def.class`; `kotlin_refs.scm`: `(call_expression (simple_identifier) @name) @ref.call`, `(type_identifier) @name @ref.reference`; `kotlin_imports.scm`: `(import_header (identifier) @path) @import`. Iterate node names against the real grammar exactly as in Task 7 Step 5.
- [ ] **Step 4: Run** — `cargo test -p codeintel` → PASS (with any drop recorded).
- [ ] **Step 5: Commit** (pathspec, `feat(codeintel): Swift/Kotlin grammars` — or the survivor + drop note).

---

### Task 9: Engine command family — `AppState` cache + `code.rs` handlers + router

**Files:**
- Modify: `src-tauri/src/engine/state.rs`, `src-tauri/src/engine/commands/mod.rs`, `src-tauri/src/engine/router.rs`
- Create: `src-tauri/src/engine/commands/code.rs`
- Test: in-module `#[cfg(test)]` in `code.rs`

**Interfaces:**
- Consumes: `codeintel::{cache::CodeIntelCache, map, graph, edit, output::envelope}`.
- Produces: router methods `code.stats|files|tree|symbols|find|callers|callees|refs|impact|rename|rewrite`. Every request struct carries `path: String` (absolute root, injected by the CLI — Task 10). Handler results: `envelope(data)` with optional `"truncated"`/`"warnings"` merged at top level.

- [ ] **Step 1: `AppState` field** — add `pub code_cache: std::sync::Arc<codeintel::cache::CodeIntelCache>,` to the struct; add `code_cache: std::sync::Arc::new(codeintel::cache::CodeIntelCache::new()),` to BOTH `new()` and `for_tests()`.

- [ ] **Step 2: Write failing handler tests** (in `code.rs`, bottom):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn stats_rejects_missing_path() {
        let state = AppState::for_tests().await;
        let err = stats(&state, json!({})).await.unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
    }

    #[tokio::test]
    async fn stats_rejects_nonexistent_root() {
        let state = AppState::for_tests().await;
        let err = stats(&state, json!({"path": "/nonexistent/xyz"})).await.unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
    }

    #[tokio::test]
    async fn find_returns_envelope_with_truncation_flag() {
        let state = AppState::for_tests().await;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "fn alpha() {}\nfn alpha_two() {}\n").unwrap();
        let out = find(&state, json!({"path": dir.path(), "name": "alpha", "limit": 1})).await.unwrap();
        assert_eq!(out["schema_version"], 1);
        assert_eq!(out["data"].as_array().unwrap().len(), 1);
        assert_eq!(out["truncated"], true);
    }

    #[tokio::test]
    async fn rename_dry_run_touches_no_files() {
        let state = AppState::for_tests().await;
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("lib.rs");
        std::fs::write(&f, "fn old_name() {}\nfn c() { old_name(); }\n").unwrap();
        let out = rename(&state, json!({"path": dir.path(), "old": "old_name", "new": "new_name"})).await.unwrap();
        assert_eq!(out["data"]["dry_run"], true);
        assert!(std::fs::read_to_string(&f).unwrap().contains("old_name"));
    }
}
```

(`tempfile` goes into `src-tauri/Cargo.toml` `[dev-dependencies]` if not already there — check first.)

- [ ] **Step 3: Verify failure** — `cargo test --lib code` in src-tauri → compile error.

- [ ] **Step 4: Implement `code.rs`** — the skeleton (repeat the pattern for all 11 verbs; blocking work always via `spawn_blocking` with cloned `Arc`s):

```rust
use crate::engine::{AppError, AppState};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RootReq {
    path: String,
}

fn root_of(raw: &str) -> Result<PathBuf, AppError> {
    let p = PathBuf::from(raw);
    if !p.is_absolute() {
        return Err(AppError::Invalid(format!("code: path must be absolute, got {raw}")));
    }
    if !p.is_dir() {
        return Err(AppError::Invalid(format!("code: not a directory: {raw}")));
    }
    Ok(p)
}

fn wrap(data: Value, truncated: bool, warnings: &[String]) -> Value {
    let mut v = codeintel::output::envelope(data);
    if truncated {
        v["truncated"] = json!(true);
    }
    if !warnings.is_empty() {
        v["warnings"] = json!(warnings);
    }
    v
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> Result<T, AppError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| AppError::Internal(format!("code: task join: {e}")))?
        .map_err(|e| AppError::Invalid(format!("code: {e:#}")))
}

pub async fn stats(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req = serde_json::from_value::<RootReq>(payload)
        .map_err(|e| AppError::Invalid(e.to_string()))?;
    let root = root_of(&req.path)?;
    let cache = state.code_cache.clone();
    let (data, warnings) = blocking(move || {
        let idx = cache.get_index(&root)?;
        Ok((codeintel::map::stats(&root, &idx)?, idx.warnings.clone()))
    })
    .await?;
    Ok(wrap(data, false, &warnings))
}
```

Verb-specific request structs (all `camelCase`, all with `path`): `files{limit: Option<usize>}`, `tree{}`, `symbols{target: Option<String>, all: Option<bool>, kind: Option<Vec<String>>, limit: Option<usize>}`, `find{name: String, exact: Option<bool>, limit: Option<usize>}`, `callers/callees{name: String, depth: Option<usize>}`, `refs/impact{name: String, limit: Option<usize> /* refs only */}`, `rename{old: String, new: String, apply: Option<bool>, lang: Option<String>, anchor: Option<String>}`, `rewrite{pattern: String, rewrite: String, apply: Option<bool>, lang: Option<String>}`. `limit` defaults to 200 in the handler (`req.limit.unwrap_or(200)`). `rename` passes the cached index (`cache.get_index`), and both `rename`/`rewrite` call `cache.invalidate_files(&root, &written)` with the written-files list before returning (Task 11 asserts this).

- [ ] **Step 5: Register** — `mod.rs`: `pub mod code;` (after `pub mod cli;`). `router.rs`: add `code` to the `use` list and a `// ── code (code intelligence) ──` block of 11 arms above the `other =>` catch-all, exactly in the browser.* style: `"code.stats" => code::stats(state, payload).await,` … `"code.rewrite" => code::rewrite(state, payload).await,` (fn for `code.refs` is `refs`, for `code.find` is `find`; avoid shadowing `type`-style keywords — none of the 11 collide).

- [ ] **Step 6: Run** — `cargo test --lib` in src-tauri → PASS.

- [ ] **Step 7: Commit** (pathspec `src-tauri/src/engine src-tauri/Cargo.toml src-tauri/Cargo.lock`, `feat(engine): code.* command family backed by codeintel cache`).

---

### Task 10: CLI surface — allowlist arm, `run_code` cwd injection, USAGE, socket round-trip

**Files:**
- Modify: `src-tauri/src/engine/commands/cli.rs` (map_argv + catch-all string)
- Modify: `src-tauri/src/bin/conclave-cli.rs` (early dispatch + `run_code` + USAGE)
- Test: cli.rs in-module tests; `run_code` argv tests in conclave-cli.rs `mod tests`; round-trip appended to `src-tauri/src/engine/uds.rs` tests

**Interfaces:**
- Consumes: router methods from Task 9; `uds_task_call` (conclave-cli.rs:491); `take_flag`/`take_switch` (cli.rs:896-917).
- Produces: `conclave code <verb> …` end-to-end. Argv contract (what `map_code_argv` accepts — after `run_code` has injected an absolute `--path`):
  ```
  code stats|tree --path <ABS>
  code files [--limit N] --path <ABS>
  code symbols [TARGET] [--all] [--kind k1,k2] [--limit N] --path <ABS>
  code find <NAME> [--exact] [--limit N] --path <ABS>
  code callers|callees <NAME> [--depth N] --path <ABS>
  code refs <NAME> [--limit N] --path <ABS>
  code impact <NAME> --path <ABS>
  code rename <OLD> <NEW> [--apply] [--lang L] [--anchor FILE:LINE] --path <ABS>
  code rewrite --pattern P --rewrite R [--apply] [--lang L] --path <ABS>
  ```
  `--json` is accepted and ignored anywhere (output is always JSON; kept so existing skill invocations don't break).

- [ ] **Step 1: Failing map_argv tests** (append to cli.rs `#[cfg(test)]`; the module already tests `map_argv` patterns — follow the local style):

```rust
#[test]
fn code_find_maps_to_router_method() {
    let (m, p) = map_argv(&argv(&["code", "find", "greet", "--exact", "--path", "/tmp/x"])).unwrap();
    assert_eq!(m, "code.find");
    assert_eq!(p, serde_json::json!({"name": "greet", "exact": true, "path": "/tmp/x"}));
}

#[test]
fn code_rename_requires_old_and_new() {
    let err = map_argv(&argv(&["code", "rename", "onlyone", "--path", "/tmp/x"])).unwrap_err();
    assert!(matches!(err, AppError::Invalid(_)));
}

#[test]
fn code_rejects_unknown_verb() {
    let err = map_argv(&argv(&["code", "dance", "--path", "/tmp/x"])).unwrap_err();
    assert!(matches!(err, AppError::Invalid(_)));
}
```

- [ ] **Step 2: Verify failure** — `cargo test --lib cli` → fails (unknown subcommand 'code').

- [ ] **Step 3: Implement `map_code_argv`** — new `"code" => map_code_argv(argv),` arm in `map_argv`; the helper lives next to `map_task_argv` and handles the 11 verbs: parse flags with `take_flag`/`take_switch`, build camelCase params (`--kind a,b` → `"kind": ["a","b"]`; numeric flags parsed with the same `parse::<i64>` + Invalid-error style as `browser snapshot --max-text`), require `--path`, strip a stray `--json` switch silently, and error `AppError::Invalid("cli: code <stats|files|tree|symbols|find|callers|callees|refs|impact|rename|rewrite> …")` on anything else. Update the catch-all allowed-list string (cli.rs:886-888) to end `…, design, browser, code`.

- [ ] **Step 4: `run_code` in conclave-cli** — early dispatch in `main()` after the `stage` block: `if argv[0] == "code" { return run_code(&argv, self_instance.as_deref()).await; }`. Implementation: if `--path` present, make it absolute (`std::env::current_dir()?.join(p)` unless already absolute, then `canonicalize()` with a readable error); if absent, append `--path <cwd>`. Then `uds_task_call(argv, self_instance)` and print via the same pretty-JSON + `emit_capped` path main uses (`OutMode::Json` equivalent), exit 0 on Ok / print error to stderr and exit 1 on Err. Add unit tests for the path-injection helper (pure fn `inject_code_path(argv: Vec<String>, cwd: &Path) -> Vec<String>` so it's testable without a socket): absent → appended; relative → joined; absolute → untouched.

- [ ] **Step 5: USAGE** — add to `const USAGE` after the browser lines:

```
  code stats|files|tree|symbols|find <args>   survey a codebase (tree-sitter)
  code callers|callees|refs|impact <name>     semantic cross-references
  code rename|rewrite [--apply]               AST-validated edits (dry-run default)
```

- [ ] **Step 6: Socket round-trip test** — append to `uds.rs` tests, modeled on `task_verbs_round_trip_over_a_real_socket`'s `call()` helper but using `handle_line` directly (simpler, no socket): build a tempdir with one `lib.rs` fixture (`fn greet() {}\nfn c() { greet(); }`), then `handle_line(&state, r#"{"jsonrpc":"2.0","id":1,"method":"cli.exec","params":{"argv":["code","find","greet","--path","<dir>"]}}"#)` and assert the response's `result.schema_version == 1` and `result.data[0].name == "greet"`; a second call `code refs greet --path <dir>` asserts a `"call"` kind hit.

- [ ] **Step 7: Run everything** — `cargo test` in src-tauri → PASS. Then the live smoke (needs the dev app running — if no engine socket exists, note it and rely on the uds test): `cargo run --bin conclave-cli -- code stats --path /Users/detoro/code/codeup/src-tauri/crates/codeintel` → pretty JSON with `total_files > 0`.

- [ ] **Step 8: Commit** (pathspec `src-tauri/src`, `feat(cli): conclave code family — allowlist, cwd injection, usage, round-trip test`).

---

### Task 11: Write-path cache invalidation + full-stack verification

**Files:**
- Test: `src-tauri/src/engine/commands/code.rs` (in-module), `src-tauri/tests/` untouched
- Modify: only what the failing test demands (the invalidation call was specified in Task 9 Step 4 — this task PROVES it)

- [ ] **Step 1: Failing test** (in code.rs tests):

```rust
#[tokio::test]
async fn apply_rename_invalidates_cache_for_next_query() {
    let state = AppState::for_tests().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"), "fn old_name() {}\nfn c() { old_name(); }\n").unwrap();
    let p = dir.path().to_str().unwrap();
    // warm the cache
    let _ = find(&state, serde_json::json!({"path": p, "name": "old_name"})).await.unwrap();
    // apply the rename
    let out = rename(&state, serde_json::json!({"path": p, "old": "old_name", "new": "new_name", "apply": true})).await.unwrap();
    assert_eq!(out["data"]["dry_run"], false);
    // the very next query must see the new world, even if mtime granularity hid the write
    let after = find(&state, serde_json::json!({"path": p, "name": "new_name"})).await.unwrap();
    assert!(!after["data"].as_array().unwrap().is_empty(), "cache served a stale index after --apply");
    let stale = find(&state, serde_json::json!({"path": p, "name": "old_name"})).await.unwrap();
    assert!(stale["data"].as_array().unwrap().is_empty());
}
```

- [ ] **Step 2: Run** — if Task 9's implementation already wired `invalidate_files` correctly, this passes immediately (fine — the test still guards the regression). If it fails, fix the handler until green.
- [ ] **Step 3: Full gates** — `cargo test -p codeintel && cargo test` (src-tauri) → all green. `cargo build` (full app incl. GUI) → compiles.
- [ ] **Step 4: Commit** (pathspec, `test(engine): apply-path invalidates code index cache; full-stack gates green`).

---

### Task 12: Skill shims + docs (outside the repo)

**Files:**
- Modify: `~/.claude/skills/ny-codemap/scripts/codemap`, `~/.claude/skills/ny-codegraph/scripts/codegraph`, `~/.claude/skills/ny-astedit/scripts/astedit` (each: rename the existing binary to `<name>-bin`, install a shim in its place)
- Modify: `~/.claude/CLAUDE.md` (skills table)

**Interfaces:**
- Consumes: `conclave code <verb>` (Task 10) with data shapes identical to the old tools — consumers reading `.data` keep working.
- Produces: old skill entry points transparently prefer the engine, fall back to the standalone binary when Conclave isn't running.

- [ ] **Step 1: Inspect before overwriting** — `file ~/.claude/skills/ny-codemap/scripts/codemap` (expect Mach-O binary). `mv` each binary to `<name>-bin` in the same dir. If any is already a script, STOP and re-read it before proceeding (do not clobber unknown logic).

- [ ] **Step 2: Install the shims** — `~/.claude/skills/ny-codemap/scripts/codemap` (`chmod +x`):

```bash
#!/bin/bash
# codemap shim: prefer the Conclave engine (cached index), fall back to the
# standalone binary when the app is not running.
set -euo pipefail
CONCLAVE="$HOME/Library/Application Support/Conclave/bin/conclave"
SOCK="$HOME/Library/Application Support/Conclave/conclave.sock"
if [[ -S "$SOCK" && -x "$CONCLAVE" ]]; then
  exec "$CONCLAVE" code "$@"
fi
exec "$(dirname "$0")/codemap-bin" "$@"
```

`ny-codegraph/scripts/codegraph`: identical except the fallback line (`codegraph-bin "$@"`) and the verb translation — old `find-refs` must become `refs` on the conclave path:

```bash
if [[ -S "$SOCK" && -x "$CONCLAVE" ]]; then
  args=("$@"); [[ "${args[0]:-}" == "find-refs" ]] && args[0]="refs"
  exec "$CONCLAVE" code "${args[@]}"
fi
exec "$(dirname "$0")/codegraph-bin" "$@"
```

`ny-astedit/scripts/astedit`: identical pattern, fallback `astedit-bin "$@"` (verbs `rename`/`rewrite` map 1:1). Note in the shim comment: exit-code semantics differ on the conclave path (always 0 on success; `needs_anchor`/`pattern-compile` live in the JSON) — the SKILL.md files already tell agents to read the JSON.

- [ ] **Step 3: Smoke both paths** — with the app running: `~/.claude/skills/ny-codemap/scripts/codemap stats --path /Users/detoro/code/codeup --json` → JSON with `schema_version: 1`. Then rename the socket check temporarily (`SOCK=/nonexistent bash -x .../codemap stats --json --path .`) or quit the app to verify the fallback binary path also answers.

- [ ] **Step 4: Update `~/.claude/CLAUDE.md`** — in the Skills table, keep the three skill rows but change the Binary column to `conclave code … (shim: ~/.claude/skills/ny-*/scripts/*)` and update the example block's first lines to `conclave code stats / symbols . / find <NAME> --exact` etc. Do not restructure the file.

- [ ] **Step 5: No git commit** (files live outside any repo) — instead record completion as a conclave task note per the lane protocol.

---

## Amendments (post-planning defects, owned by the plan)

- **Rename def-site defect (challenge 4f99e542 by Dabin, ruling 77b4ae3d):** inherited from astedit upstream — `rename --apply` edits references only; a `fn` definition's own identifier is never edited (structs pass by accident: `rust_refs.scm` captures every `type_identifier`, the def's name token included). Task 11's fn-based test correctly exposes it. Fix = task `codeintel-rename-def-site` (def-site edit + additive `Definition.name_start_byte`/`name_end_byte` + fn-based regression), landing after the grammars lane merges.
- **Task 7/8 test-template gap (challenge 18c27878 by Mellow, ruling 5359d396):** the plan's own Step 2 fixtures contained no import statements, leaving all six new `*_imports.scm` files untested while the heading promised cross-file coverage — a plan defect. Coverage ruling: every new language asserts `idx.imports` extraction; Go+Java get true cross-file fixtures (quoted-path + dotted-path).
- **Risk ledger addition:** cross-file call RESOLUTION (module_matches conventions) for Swift/C/C++/Kotlin — and possibly Go/Java, pending Dew's findings — is not exercised in v1; `resolve.rs` generalization beyond rust/TS/JS/py conventions is a recorded follow-up, deliberately out of every current lane's scope.

## Verification (whole feature)

1. `cargo test -p codeintel && cargo test` (src-tauri) — all green.
2. `cargo build` — full app builds.
3. Live: `conclave code stats --path /Users/detoro/code/codeup` twice — second call visibly faster (warm cache); `conclave code refs build_index --path src-tauri/crates/codeintel` returns definition + call hits; `conclave code rename … --apply` on a scratch dir changes the file and the follow-up `find` sees it.
4. Skill shim smoke (Task 12 Step 3) on both the engine path and the fallback path.
