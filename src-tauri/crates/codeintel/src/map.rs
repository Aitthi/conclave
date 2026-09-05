//! The five codemap verbs (stats/files/tree/symbols/find), re-implemented as
//! library functions over the shared [`crate::index::Index`] instead of
//! codemap's own per-command file walk + `symbols::extract_file` pass.
//!
//! Every function here returns the *inner* `data` value (`anyhow::Result<Value>`,
//! or `(Value, bool)` where the bool means "results were truncated by `limit`").
//! Wrapping in the wire envelope (`output::envelope`) is the engine's job, not
//! this module's — these are library calls, not CLI commands.

use crate::index::{DefKind, Definition, Index};
use crate::walk::default_walker;
use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Maps a `DefKind` to the lowercase string codemap/consumers expect.
/// Mirrors `DefKind`'s own `#[serde(rename_all = "lowercase")]`, kept as an
/// explicit match (rather than round-tripping through `serde_json::to_value`)
/// so it doubles as the single source of truth for `parse_kind` below.
fn kind_str(kind: DefKind) -> &'static str {
    match kind {
        DefKind::Fn => "fn",
        DefKind::Struct => "struct",
        DefKind::Enum => "enum",
        DefKind::Trait => "trait",
        DefKind::Class => "class",
        DefKind::Interface => "interface",
        DefKind::Type => "type",
        DefKind::Const => "const",
        DefKind::Method => "method",
    }
}

/// Parses a `--kind` filter value. Accepts `DefKind`'s lowercase serializations
/// plus `function` as an alias of `fn` (codemap's `SymbolKind::parse` rule).
fn parse_kind(s: &str) -> Option<DefKind> {
    match s.to_ascii_lowercase().as_str() {
        "fn" | "function" => Some(DefKind::Fn),
        "struct" => Some(DefKind::Struct),
        "enum" => Some(DefKind::Enum),
        "trait" => Some(DefKind::Trait),
        "class" => Some(DefKind::Class),
        "interface" => Some(DefKind::Interface),
        "type" => Some(DefKind::Type),
        "const" => Some(DefKind::Const),
        "method" => Some(DefKind::Method),
        _ => None,
    }
}

/// Builds the `{file, name, kind, start_line, end_line, signature?}` element
/// shape shared by `symbols` and `find`. `signature` is omitted (not `null`)
/// when absent, matching codemap's `Symbol` struct.
fn symbol_json(d: &Definition) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("file".into(), json!(d.file));
    obj.insert("name".into(), json!(d.name));
    obj.insert("kind".into(), json!(kind_str(d.kind)));
    obj.insert("start_line".into(), json!(d.line));
    obj.insert("end_line".into(), json!(d.end_line));
    if let Some(sig) = &d.signature {
        obj.insert("signature".into(), json!(sig));
    }
    Value::Object(obj)
}

#[derive(Serialize, Default)]
struct LangStats {
    files: usize,
    lines: usize,
}

/// `{total_files, total_lines, languages: {name: {files, lines}}, symbols: {kind: count}}`.
/// All data comes from `idx` (already built over `root`); `root` is accepted for
/// interface parity with codemap's `stats` but isn't otherwise needed here.
pub fn stats(_root: &Path, idx: &Index) -> Result<Value> {
    let mut total_files = 0usize;
    let mut total_lines = 0usize;
    let mut languages: BTreeMap<&'static str, LangStats> = BTreeMap::new();

    for meta in idx.file_meta.values() {
        total_files += 1;
        total_lines += meta.lines;
        let entry = languages.entry(meta.language).or_default();
        entry.files += 1;
        entry.lines += meta.lines;
    }

    let mut symbols: BTreeMap<&'static str, usize> = BTreeMap::new();
    for d in &idx.definitions {
        *symbols.entry(kind_str(d.kind)).or_insert(0) += 1;
    }

    Ok(json!({
        "total_files": total_files,
        "total_lines": total_lines,
        "languages": languages,
        "symbols": symbols,
    }))
}

/// Array of `{path, language, lines, size_bytes}`, sorted by path.
pub fn files(idx: &Index, limit: usize) -> Result<(Value, bool)> {
    let mut entries: Vec<(&String, &crate::index::FileMeta)> = idx.file_meta.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));

    let total = entries.len();
    let truncated = total > limit;
    entries.truncate(limit);

    let arr: Vec<Value> = entries
        .into_iter()
        .map(|(path, meta)| {
            json!({
                "path": path,
                "language": meta.language,
                "lines": meta.lines,
                "size_bytes": meta.len,
            })
        })
        .collect();
    Ok((Value::Array(arr), truncated))
}

#[derive(Serialize)]
struct TreeNode {
    name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<TreeNode>,
    is_dir: bool,
}

/// Recursive `{name, children?, is_dir}` tree, walked directly off disk via
/// `default_walker` — no index needed.
pub fn tree(root: &Path) -> Result<Value> {
    let canon_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in default_walker(&canon_root).build() {
        let entry = entry?;
        let path = entry.into_path();
        if path == canon_root {
            continue;
        }
        paths.push(path);
    }
    paths.sort();

    let tree = build_tree(&canon_root, &paths);
    Ok(serde_json::to_value(tree)?)
}

fn build_tree(root: &Path, paths: &[PathBuf]) -> TreeNode {
    let mut root_node = TreeNode {
        name: root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".".into()),
        children: Vec::new(),
        is_dir: true,
    };
    for path in paths {
        let rel = path.strip_prefix(root).unwrap();
        let parts: Vec<&str> = rel.iter().filter_map(|c| c.to_str()).collect();
        insert_tree(&mut root_node, &parts, path.is_dir());
    }
    root_node
}

fn insert_tree(node: &mut TreeNode, parts: &[&str], is_dir: bool) {
    if parts.is_empty() {
        return;
    }
    let head = parts[0];
    let existing = node.children.iter_mut().position(|c| c.name == head);
    let idx = match existing {
        Some(i) => i,
        None => {
            node.children.push(TreeNode {
                name: head.to_string(),
                children: Vec::new(),
                is_dir: parts.len() > 1 || is_dir,
            });
            node.children.len() - 1
        }
    };
    insert_tree(&mut node.children[idx], &parts[1..], is_dir);
}

/// Array of `{file, name, kind, start_line, end_line, signature?}`.
/// `target == Some(".")` (or `None`, matching codemap's CLI default) or `all`
/// means "whole project"; otherwise filters `idx.definitions` to `file == target`.
pub fn symbols(
    idx: &Index,
    target: Option<&str>,
    all: bool,
    kinds: &[String],
    limit: usize,
) -> Result<(Value, bool)> {
    let mut filter_kinds: Vec<DefKind> = Vec::new();
    for k in kinds {
        filter_kinds.push(parse_kind(k).ok_or_else(|| anyhow!("unknown kind value: {k}"))?);
    }

    let target = target.unwrap_or(".");
    let whole_project = all || target == ".";

    let defs: Vec<&Definition> = idx
        .definitions
        .iter()
        .filter(|d| whole_project || d.file == target)
        .filter(|d| filter_kinds.is_empty() || filter_kinds.contains(&d.kind))
        .collect();

    let total = defs.len();
    let truncated = total > limit;
    let arr: Vec<Value> = defs.into_iter().take(limit).map(symbol_json).collect();
    Ok((Value::Array(arr), truncated))
}

/// Array of `{file, name, kind, start_line, end_line, signature?}`, sorted by
/// `(file, start_line)`.
pub fn find(idx: &Index, name: &str, exact: bool, limit: usize) -> Result<(Value, bool)> {
    let mut hits: Vec<&Definition> = idx
        .definitions
        .iter()
        .filter(|d| {
            if exact {
                d.name == name
            } else {
                d.name.contains(name)
            }
        })
        .collect();
    hits.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));

    let total = hits.len();
    let truncated = total > limit;
    let arr: Vec<Value> = hits.into_iter().take(limit).map(symbol_json).collect();
    Ok((Value::Array(arr), truncated))
}
