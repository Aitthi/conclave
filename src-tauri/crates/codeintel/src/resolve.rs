use crate::index::{Definition, Import, Index, Reference};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    High,
    Medium,
    Low,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::High => "high",
            Confidence::Medium => "medium",
            Confidence::Low => "low",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveReason {
    SameFileScope,
    ImportResolved,
    NameOnly,
}

impl ResolveReason {
    pub fn as_str(self) -> &'static str {
        match self {
            ResolveReason::SameFileScope => "same-file-scope",
            ResolveReason::ImportResolved => "import-resolved",
            ResolveReason::NameOnly => "name-only",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Resolved<'a> {
    pub reference: &'a Reference,
    /// Matched definition (used by callers/callees commands; not yet consumed by find-refs).
    pub definition: Option<&'a Definition>,
    pub confidence: Confidence,
    pub reason: ResolveReason,
}

/// Resolve every reference whose `name == target` against the index.
/// Definitions are matched on `name` only (no signature equality, no generics).
pub fn resolve_refs<'a>(idx: &'a Index, target: &str) -> Vec<Resolved<'a>> {
    let defs_by_name: Vec<&Definition> = idx
        .definitions
        .iter()
        .filter(|d| d.name == target)
        .collect();
    let mut out = Vec::new();
    for r in &idx.references {
        if r.name != target {
            continue;
        }

        // Rule 1: same-file definition?
        if let Some(d) = defs_by_name.iter().find(|d| d.file == r.file) {
            out.push(Resolved {
                reference: r,
                definition: Some(d),
                confidence: Confidence::High,
                reason: ResolveReason::SameFileScope,
            });
            continue;
        }

        // Rule 2: named import in r.file whose local_name == target pointing at a defining file?
        let imports_in_file: Vec<&Import> =
            idx.imports.iter().filter(|i| i.file == r.file).collect();
        let named_import = imports_in_file
            .iter()
            .find(|i| i.local_name == target && i.imported_name != "*");
        if let Some(imp) = named_import {
            if let Some(d) = defs_by_name.iter().find(|d| module_matches(d, imp)) {
                out.push(Resolved {
                    reference: r,
                    definition: Some(d),
                    confidence: Confidence::High,
                    reason: ResolveReason::ImportResolved,
                });
                continue;
            }
        }

        // Rule 3: glob import from a defining file?
        let glob_resolution = imports_in_file
            .iter()
            .filter(|i| i.imported_name == "*")
            .find_map(|imp| {
                defs_by_name
                    .iter()
                    .find(|d| module_matches(d, imp))
                    .copied()
            });
        if let Some(d) = glob_resolution {
            out.push(Resolved {
                reference: r,
                definition: Some(d),
                confidence: Confidence::Medium,
                reason: ResolveReason::ImportResolved,
            });
            continue;
        }

        // Rule 4/5: name-only.
        let conf = if defs_by_name.len() == 1 {
            Confidence::Medium
        } else {
            Confidence::Low
        };
        out.push(Resolved {
            reference: r,
            definition: defs_by_name.first().copied(),
            confidence: conf,
            reason: ResolveReason::NameOnly,
        });
    }
    out
}

/// Heuristic: does `imp.module_path` (e.g. "crate::auth") plausibly point at the file
/// containing `def` (e.g. "src/auth.rs")?
fn module_matches(def: &Definition, imp: &Import) -> bool {
    if imp.module_path.is_empty() {
        return false;
    }
    let raw = imp.module_path.trim_matches('"').trim_matches('\'');
    let def_file = def.file.replace('\\', "/");

    // Rust-style first.
    if raw.starts_with("crate::") || raw == "crate" || raw.starts_with("self::") {
        let path = raw
            .trim_start_matches("crate::")
            .trim_start_matches("self::")
            .replace("::", "/");
        if path.is_empty() {
            return def_file == "src/lib.rs" || def_file == "src/main.rs" || def_file == "lib.rs";
        }
        return [
            format!("src/{path}.rs"),
            format!("src/{path}/mod.rs"),
            format!("{path}.rs"),
            format!("{path}/mod.rs"),
        ]
        .iter()
        .any(|s| s == &def_file);
    }

    // JS/TS-style relative paths.
    if raw.starts_with("./") || raw.starts_with("../") || raw.starts_with('/') {
        // We don't know the importing file's location here — match against def file's stem.
        let trimmed = raw.trim_start_matches("./");
        let trimmed = trimmed
            .trim_end_matches(".ts")
            .trim_end_matches(".tsx")
            .trim_end_matches(".js")
            .trim_end_matches(".mjs")
            .trim_end_matches(".cjs");
        let import_last = trimmed.rsplit('/').next().unwrap_or(trimmed);
        let def_stem = std::path::Path::new(&def_file)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        return import_last == def_stem;
    }

    // Python dotted paths. Examples:
    //   `.user`              → relative; match against any file with stem == "user".
    //   `package.user`       → match against `package/user.py`.
    //   `package.user.sub`   → match against `package/user/sub.py`.
    if raw.starts_with('.') || raw.chars().any(|c| c == '.') {
        let normalized = raw.trim_start_matches('.').replace('.', "/");
        let candidates = [
            format!("{normalized}.py"),
            format!("{normalized}/__init__.py"),
        ];
        let def_stem = std::path::Path::new(&def_file)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let last = normalized.rsplit('/').next().unwrap_or(&normalized);
        return candidates.iter().any(|c| def_file.ends_with(c)) || def_stem == last;
    }
    false
}
