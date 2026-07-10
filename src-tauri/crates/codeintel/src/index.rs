use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DefKind {
    Fn,
    Struct,
    Enum,
    Trait,
    Class,
    Interface,
    Type,
    Const,
    Method,
}

#[derive(Debug, Clone, Serialize)]
pub struct Definition {
    pub file: String,
    pub name: String,
    pub kind: DefKind,
    pub line: usize,
    pub column: usize,
    /// The byte range of the *body* of the definition. Used by `callers`/`callees`
    /// to decide whether a reference site sits inside this definition.
    pub body_start_byte: usize,
    pub body_end_byte: usize,
    /// True if the definition is module-public (Rust `pub`, TS `export`, Python module-level).
    /// Cross-file resolution only matches against exported definitions.
    pub exported: bool,
}

/// One concrete "this name in this file refers to that other module's symbol".
/// Glob imports leave `imported_name = "*"` and the resolver treats them as wildcards.
#[derive(Debug, Clone, Serialize)]
pub struct Import {
    pub file: String,
    pub line: usize,
    /// The local binding the rest of the file uses.
    pub local_name: String,
    /// The name as it exists in the source module (often equal to local_name).
    pub imported_name: String,
    /// The module path string as written in the source (e.g. `crate::auth`, `./util`).
    /// The resolver normalizes this against `file`'s location.
    pub module_path: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RefKind {
    Call,
    Reference,
}

#[derive(Debug, Clone, Serialize)]
pub struct Reference {
    pub file: String,
    pub name: String,
    pub kind: RefKind,
    pub line: usize,
    pub column: usize,
    /// Byte offset into the source — used to test "is this ref inside fn X's body?".
    pub byte_offset: usize,
    /// Line text trimmed to <= 200 chars — included in the output as `context`.
    pub context: String,
}

/// Snapshot of stat-time metadata for one source file. Populated during
/// `build_index`. Used by `astedit` for cheap drift detection between
/// index build and apply.
#[derive(Debug, Clone, Default, Serialize)]
#[non_exhaustive]
pub struct FileMeta {
    pub len: u64,
}

/// One `pub use foo::Bar as Baz;` style alias re-export. Records both names
/// so the rename pipeline can surface the site under `skip_reason:
/// "re-export-alias"` with a `via_alias` field.
#[derive(Debug, Clone, Default, Serialize)]
#[non_exhaustive]
pub struct AliasSite {
    pub file: String,
    pub line: usize,
    pub alias: String,
    pub original: String,
    pub module_path: String,
}

/// One `pub use foo::*;` style wildcard re-export. The set of symbols that
/// cross the boundary cannot be known without parsing the target module's
/// exports, so the rename pipeline surfaces these under `skip_reason:
/// "wildcard-reexport"` with a `via_module` field.
#[derive(Debug, Clone, Default, Serialize)]
#[non_exhaustive]
pub struct WildcardSite {
    pub file: String,
    pub line: usize,
    pub module_path: String,
}

#[derive(Debug, Default)]
#[non_exhaustive]
pub struct Index {
    pub definitions: Vec<Definition>,
    pub imports: Vec<Import>,
    pub references: Vec<Reference>,
    pub file_meta: std::collections::HashMap<String, FileMeta>,
    pub alias_reexports: std::collections::HashMap<String, Vec<AliasSite>>,
    pub wildcard_reexports: std::collections::HashMap<String, Vec<WildcardSite>>,
}

impl Index {
    /// Look up the definition that *contains* `byte_offset` in `file` — i.e. the function
    /// that the byte_offset's reference sits inside. Returns the innermost match.
    pub fn enclosing_definition(&self, file: &str, byte_offset: usize) -> Option<&Definition> {
        self.definitions
            .iter()
            .filter(|d| {
                d.file == file && d.body_start_byte <= byte_offset && byte_offset < d.body_end_byte
            })
            .min_by_key(|d| d.body_end_byte - d.body_start_byte)
    }
}

use crate::lang::{Language, QueryKind};
use crate::walk::walk_sources;
use anyhow::{Context, Result};
use rayon::prelude::*;
use std::fs;
use std::path::Path;
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

/// Compiled queries for every language present in a walk, shared by reference
/// across the parallel indexing pass.
type QueryCache = std::collections::HashMap<Language, LangQueries>;

impl DefKind {
    fn from_capture_suffix(s: &str) -> Option<Self> {
        match s.strip_prefix("def.")? {
            "fn" => Some(DefKind::Fn),
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
}

/// The three compiled tree-sitter queries for one language, plus the grammar
/// handle used to (re)configure the parser.
///
/// Compiling a tree-sitter `Query` from its `.scm` source is the dominant cost
/// of indexing — on a ~3.7k-file TS repo it was ~12ms per file and was being
/// paid once *per file* (defs+imports+refs ≈ 11k compilations). Compiling each
/// query once per language and reusing it across every file of that language
/// turns that O(files) cost into O(languages).
struct LangQueries {
    ts: tree_sitter::Language,
    defs: Option<Query>,
    imports: Option<Query>,
    refs: Option<Query>,
}

impl LangQueries {
    fn compile(lang: Language) -> Result<Self> {
        let ts = lang.ts_language();
        // Defs is load-bearing — a compile failure here is a hard error, matching
        // the previous per-file behaviour where the defs query used `?`.
        let defs = match lang.query_source(QueryKind::Defs) {
            Some(src) => Some(
                Query::new(&ts, src)
                    .with_context(|| format!("compile defs query for {}", lang.name()))?,
            ),
            None => None,
        };
        // Imports/refs are best-effort: node kinds drift across tree-sitter
        // releases, so a compile failure downgrades to "skip this query for this
        // language" rather than aborting the whole index.
        let imports =
            lang.query_source(QueryKind::Imports)
                .and_then(|src| match Query::new(&ts, src) {
                    Ok(q) => Some(q),
                    Err(e) => {
                        eprintln!(
                            "codegraph: imports query skipped for {}: {e:#}",
                            lang.name()
                        );
                        None
                    }
                });
        let refs = lang
            .query_source(QueryKind::Refs)
            .and_then(|src| match Query::new(&ts, src) {
                Ok(q) => Some(q),
                Err(e) => {
                    eprintln!("codegraph: refs query skipped for {}: {e}", lang.name());
                    None
                }
            });
        Ok(LangQueries {
            ts,
            defs,
            imports,
            refs,
        })
    }
}

pub fn build_index(root: &Path) -> Result<Index> {
    let files = walk_sources(root)?;

    // Compile each language's queries exactly once, up front. Doing this before the
    // parallel pass keeps the hot loop free of `Query::new` (the dominant cost) and
    // lets every worker thread share the compiled queries by reference — `Query` is
    // `Sync`. A defs-query compile failure is a hard error here, matching the
    // pre-refactor `?` behaviour.
    let mut qcache: std::collections::HashMap<Language, LangQueries> =
        std::collections::HashMap::new();
    for f in &files {
        if let std::collections::hash_map::Entry::Vacant(e) = qcache.entry(f.language) {
            e.insert(LangQueries::compile(f.language)?);
        }
    }

    // Index files in parallel. Each file is independent: its own `Parser`, its own
    // tree, its own partial `Index`. rayon's `collect` into a `Vec` preserves input
    // order, so merging the partials below reproduces exactly the sequential index —
    // definitions/references land in the same order, keeping output byte-identical.
    let partials: Vec<Index> = files
        .par_iter()
        .filter_map(|f| index_file(root, f, &qcache))
        .collect();

    // Merge partials in file order. Vecs concatenate; re-export maps merge per key.
    let mut idx = Index::default();
    for p in partials {
        idx.definitions.extend(p.definitions);
        idx.imports.extend(p.imports);
        idx.references.extend(p.references);
        idx.file_meta.extend(p.file_meta);
        for (k, v) in p.alias_reexports {
            idx.alias_reexports.entry(k).or_default().extend(v);
        }
        for (k, v) in p.wildcard_reexports {
            idx.wildcard_reexports.entry(k).or_default().extend(v);
        }
    }
    Ok(idx)
}

/// Parse one source file and run all three queries against it, returning a
/// single-file partial `Index`. Returns `None` for unreadable files, grammar
/// configuration failures, or parse failures — the caller drops them, matching
/// the sequential loop's `continue`.
fn index_file(root: &Path, f: &crate::walk::SourceFile, qcache: &QueryCache) -> Option<Index> {
    let source = fs::read_to_string(&f.path).ok()?;
    let rel = f
        .path
        .strip_prefix(root)
        .unwrap_or(&f.path)
        .to_string_lossy()
        .into_owned();
    let queries = qcache.get(&f.language)?;

    let mut parser = Parser::new();
    if parser.set_language(&queries.ts).is_err() {
        eprintln!("codegraph: set language failed for {rel}");
        return None;
    }
    let tree = parser.parse(&source, None)?;

    let mut local = Index::default();
    local.file_meta.insert(
        rel.clone(),
        FileMeta {
            len: source.len() as u64,
        },
    );
    if let Some(q) = &queries.defs {
        index_defs(&mut local, &source, &rel, f.language, &tree, q);
    }
    if let Some(q) = &queries.imports {
        index_imports(&mut local, &source, &rel, f.language, &tree, q);
    }
    if let Some(q) = &queries.refs {
        index_refs(&mut local, &source, &rel, &tree, q);
    }
    Some(local)
}

fn index_defs(
    idx: &mut Index,
    source: &str,
    rel: &str,
    lang: Language,
    tree: &tree_sitter::Tree,
    query: &Query,
) {
    let names = query.capture_names();
    let bytes = source.as_bytes();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), bytes);
    while let Some(m) = matches.next() {
        let mut def_node = None;
        let mut def_kind = None;
        let mut name = None;
        for cap in m.captures {
            let cname = names[cap.index as usize];
            if cname == "name" {
                name = Some(cap.node.utf8_text(bytes).unwrap_or("").to_string());
            } else if let Some(k) = DefKind::from_capture_suffix(cname) {
                def_node = Some(cap.node);
                def_kind = Some(k);
            }
        }
        let (Some(n), Some(node), Some(kind)) = (name, def_node, def_kind) else {
            continue;
        };
        let exported = is_exported(node, bytes, lang);
        idx.definitions.push(Definition {
            file: rel.to_string(),
            name: n,
            kind,
            line: node.start_position().row + 1,
            column: node.start_position().column + 1,
            body_start_byte: node.start_byte(),
            body_end_byte: node.end_byte(),
            exported,
        });
    }
}

fn index_imports(
    idx: &mut Index,
    source: &str,
    rel: &str,
    lang: Language,
    tree: &tree_sitter::Tree,
    query: &Query,
) {
    let names = query.capture_names();
    let bytes = source.as_bytes();

    // First pass: collect the byte ranges of every use_declaration node that was
    // matched by a reexport-specific pattern (@reexport_alias or @reexport_wildcard).
    // We use this in the second pass to skip the plain @import match for those exact
    // nodes, preventing double-recording.  Nodes matched ONLY by @import (e.g. plain
    // `pub use foo::Bar;` with no alias and no glob) are intentionally NOT in this set
    // and must still be recorded in the imports table per spec.
    let mut reexport_node_ranges: std::collections::HashSet<(usize, usize)> =
        std::collections::HashSet::new();
    {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, tree.root_node(), bytes);
        while let Some(m) = matches.next() {
            for cap in m.captures {
                let cname = names[cap.index as usize];
                if cname == "reexport_alias" || cname == "reexport_wildcard" {
                    reexport_node_ranges.insert((cap.node.start_byte(), cap.node.end_byte()));
                }
            }
        }
    }

    // Whether this language uses quoted string nodes for module paths (TS/JS) or bare
    // scoped-identifier paths (Rust). TS/JS `(string)` nodes include the surrounding
    // quote characters in their text, so we strip them before storing.
    let path_uses_quotes = matches!(
        lang,
        Language::TypeScript | Language::Tsx | Language::JavaScript
    );

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), bytes);
    while let Some(m) = matches.next() {
        let mut path_text: Option<String> = None;
        let mut single_name: Option<String> = None;
        let mut alias: Option<String> = None;
        let mut original_name: Option<String> = None;
        let mut group_node: Option<tree_sitter::Node<'_>> = None;
        let mut import_node: Option<tree_sitter::Node<'_>> = None;
        let mut is_reexport_alias = false;
        let mut is_reexport_wildcard = false;
        for cap in m.captures {
            let cname = names[cap.index as usize];
            let text = cap.node.utf8_text(bytes).unwrap_or("").to_string();
            match cname {
                "path" => path_text = Some(text),
                "name" => single_name = Some(text),
                "alias" => alias = Some(text),
                "original" => original_name = Some(text),
                "group" => group_node = Some(cap.node),
                "import" => import_node = Some(cap.node),
                "reexport_alias" => {
                    is_reexport_alias = true;
                    import_node = Some(cap.node);
                }
                "reexport_wildcard" => {
                    is_reexport_wildcard = true;
                    import_node = Some(cap.node);
                }
                _ => {}
            }
        }
        let line = import_node.map(|n| n.start_position().row + 1).unwrap_or(0);
        // Strip surrounding quote characters for TS/JS string nodes.
        let module_path = {
            let raw = path_text.unwrap_or_default();
            if path_uses_quotes {
                raw.trim_matches('"').trim_matches('\'').to_string()
            } else {
                raw
            }
        };

        // If this is a plain @import match for a use_declaration that was ALSO matched by
        // a reexport-specific pattern, skip it — the reexport branch below will handle it.
        // Crucially, plain `pub use foo::Bar;` (no alias, no glob) only matches @import,
        // so its byte range is NOT in reexport_node_ranges and it passes through correctly.
        if !is_reexport_alias && !is_reexport_wildcard {
            if let Some(node) = import_node {
                let range = (node.start_byte(), node.end_byte());
                if reexport_node_ranges.contains(&range) {
                    continue;
                }
            }
        }

        // Re-export patterns fire when `pub` is present (Rust) or on export_statement
        // (TS/JS) — route them to their own tables.
        if is_reexport_alias {
            if let Some(a) = alias {
                // TS/JS: `original` comes from the explicit @original capture (the `name`
                // field of export_specifier). Rust: derive from the last `::` segment of
                // the module path since `use foo::Bar as Baz` encodes the original name
                // as the final path component.
                let original = if let Some(orig) = original_name {
                    orig
                } else {
                    module_path
                        .rsplit("::")
                        .next()
                        .unwrap_or(&module_path)
                        .to_string()
                };
                idx.alias_reexports
                    .entry(a.clone())
                    .or_default()
                    .push(AliasSite {
                        file: rel.to_string(),
                        line,
                        alias: a,
                        original,
                        module_path,
                    });
            }
            continue;
        }
        if is_reexport_wildcard {
            // TS/JS paths are slash-separated (e.g. "./widgets"), Rust paths use `::`.
            // Key on the last segment regardless of separator.
            let key = module_path
                .rsplit(['/', ':'])
                .find(|s| !s.is_empty())
                .unwrap_or(&module_path)
                .to_string();
            idx.wildcard_reexports
                .entry(key)
                .or_default()
                .push(WildcardSite {
                    file: rel.to_string(),
                    line,
                    module_path,
                });
            continue;
        }

        match (single_name, alias, group_node) {
            (Some(n), _, _) => {
                idx.imports.push(Import {
                    file: rel.to_string(),
                    line,
                    local_name: n.clone(),
                    imported_name: n,
                    module_path: module_path.clone(),
                });
            }
            (_, Some(a), _) => {
                // `use foo::bar as a;` — alias is the local name, the last segment of path is imported_name.
                let imported = module_path
                    .rsplit("::")
                    .next()
                    .unwrap_or(&module_path)
                    .to_string();
                idx.imports.push(Import {
                    file: rel.to_string(),
                    line,
                    local_name: a,
                    imported_name: imported,
                    module_path,
                });
            }
            (_, _, Some(group)) => {
                // Walk the group node's `(identifier)` and `(use_as_clause)` children.
                let mut cur = group.walk();
                for child in group.children(&mut cur) {
                    match child.kind() {
                        "identifier" => {
                            let nm = child.utf8_text(bytes).unwrap_or("").to_string();
                            idx.imports.push(Import {
                                file: rel.to_string(),
                                line,
                                local_name: nm.clone(),
                                imported_name: nm,
                                module_path: module_path.clone(),
                            });
                        }
                        "use_as_clause" => {
                            // First identifier child = imported, alias child = local.
                            let mut sub = child.walk();
                            let mut kids = child
                                .children(&mut sub)
                                .filter(|n| n.kind() == "identifier");
                            let imp = kids
                                .next()
                                .map(|n| n.utf8_text(bytes).unwrap_or("").to_string())
                                .unwrap_or_default();
                            let als = kids
                                .next()
                                .map(|n| n.utf8_text(bytes).unwrap_or("").to_string())
                                .unwrap_or_else(|| imp.clone());
                            idx.imports.push(Import {
                                file: rel.to_string(),
                                line,
                                local_name: als,
                                imported_name: imp,
                                module_path: module_path.clone(),
                            });
                        }
                        _ => {}
                    }
                }
            }
            _ => {
                // Glob `use foo::*;` — record one wildcard entry.
                idx.imports.push(Import {
                    file: rel.to_string(),
                    line,
                    local_name: "*".to_string(),
                    imported_name: "*".to_string(),
                    module_path,
                });
            }
        }
    }
}

impl RefKind {
    fn from_capture_suffix(s: &str) -> Option<Self> {
        match s.strip_prefix("ref.")? {
            "call" => Some(RefKind::Call),
            "reference" => Some(RefKind::Reference),
            _ => None,
        }
    }
}

fn index_refs(idx: &mut Index, source: &str, rel: &str, tree: &tree_sitter::Tree, query: &Query) {
    let names = query.capture_names();
    let bytes = source.as_bytes();
    let mut cursor = QueryCursor::new();

    // Dedupe key: (byte_offset, name). Calls and Reference captures often overlap at the same byte.
    let mut seen: std::collections::HashSet<(usize, String)> = std::collections::HashSet::new();

    let mut matches = cursor.matches(query, tree.root_node(), bytes);
    while let Some(m) = matches.next() {
        let mut name_node: Option<tree_sitter::Node<'_>> = None;
        let mut ref_kind: Option<RefKind> = None;
        for cap in m.captures {
            let cname = names[cap.index as usize];
            if cname == "name" {
                name_node = Some(cap.node);
            } else if let Some(k) = RefKind::from_capture_suffix(cname) {
                ref_kind = Some(k);
            }
        }
        let (Some(node), Some(kind)) = (name_node, ref_kind) else {
            continue;
        };
        let name = node.utf8_text(bytes).unwrap_or("").to_string();
        let byte_offset = node.start_byte();
        if !seen.insert((byte_offset, name.clone())) {
            // Same site already recorded — keep the first (Call wins over Reference because the query lists calls first).
            continue;
        }
        let line = node.start_position().row + 1;
        let column = node.start_position().column + 1;
        let context = line_at(source, line);
        idx.references.push(Reference {
            file: rel.to_string(),
            name,
            kind,
            line,
            column,
            byte_offset,
            context,
        });
    }
}

fn line_at(source: &str, line: usize) -> String {
    let raw = source.lines().nth(line.saturating_sub(1)).unwrap_or("");
    let trimmed = raw.trim();
    if trimmed.chars().count() > 200 {
        trimmed.chars().take(200).collect::<String>() + "…"
    } else {
        trimmed.to_string()
    }
}

fn is_exported(node: tree_sitter::Node<'_>, bytes: &[u8], lang: Language) -> bool {
    match lang {
        Language::Rust => {
            // Look for a `visibility_modifier` child whose text starts with `pub`.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "visibility_modifier" {
                    let text = child.utf8_text(bytes).unwrap_or("");
                    if text.starts_with("pub") {
                        return true;
                    }
                }
            }
            false
        }
        Language::TypeScript | Language::Tsx | Language::JavaScript => {
            // Walk parents looking for an export_statement.
            let mut p = node.parent();
            while let Some(node) = p {
                if node.kind() == "export_statement" || node.kind() == "export_clause" {
                    return true;
                }
                p = node.parent();
            }
            false
        }
        Language::Python => {
            // Module-level definitions are conventionally "public" — we leave this true for now
            // and refine in Task 15 if necessary.
            true
        }
    }
}
