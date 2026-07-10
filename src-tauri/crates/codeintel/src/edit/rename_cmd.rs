//! `rename` verb — ported from astedit's `commands/rename.rs`. The CLI's
//! `clap` args, `build_index` call, and `print_json`/exit-code plumbing are
//! replaced by direct function parameters and a returned
//! `(serde_json::Value, Vec<String>)`: the caller supplies the cached
//! `Index` (Task 5 brief — the engine builds it once and shares it across
//! commands), and every condition the old CLI reported through a process
//! exit code (needs_anchor, in particular) is now fully represented in the
//! JSON payload with `Ok` returned.

use std::path::Path;

use anyhow::Result;
use serde_json::Value;

use crate::edit::error::AstEditError;
use crate::edit::serialize::{AppliedEdit, AppliedFile, RenameData, SkippedSite};
use crate::index::{DefKind, Definition, Index};
use crate::resolve::{resolve_refs, Confidence, Resolved};

/// Rename `old` to `new` across the project rooted at `root`, using the
/// pre-built `idx`. Dry-run unless `apply` is true. `lang` is accepted for
/// interface parity with `rewrite` but — matching the original astedit CLI,
/// which declared `--lang` on `rename` yet never consumed it — is not used
/// to filter anything here; rename resolution is driven entirely by the
/// index's cross-file reference graph. `anchor` disambiguates `FILE:LINE`
/// when `old` names more than one definition across different files.
///
/// Returns `(data, written_files)` where `data` is astedit's `RenameData`
/// shape verbatim and `written_files` lists the repo-relative paths actually
/// written to disk (empty unless `apply` is true).
pub fn rename(
    root: &Path,
    idx: &Index,
    old: &str,
    new: &str,
    apply: bool,
    lang: Option<&str>,
    anchor: Option<&str>,
) -> Result<(Value, Vec<String>)> {
    let _ = lang; // accepted for interface parity; unused (see doc comment above)

    let resolved = resolve_refs(idx, old);

    // Step 2: find defs by name. The resolver doesn't expose this — query the
    // index directly.
    let defs: Vec<&Definition> = idx.definitions.iter().filter(|d| d.name == old).collect();

    // Count distinct files; same-file multiple defs (e.g. nested modules) are
    // handled by the resolver — only cross-file ambiguity requires --anchor.
    let def_files: std::collections::HashSet<&str> = defs.iter().map(|d| d.file.as_str()).collect();

    let mut written_files: Vec<String> = Vec::new();

    // Multi-def + no anchor → needs_anchor payload; former exit code 2 folded
    // into the JSON, callers get Ok.
    if def_files.len() > 1 && anchor.is_none() {
        let candidates: Vec<crate::edit::serialize::Candidate> = defs
            .iter()
            .map(|d| crate::edit::serialize::Candidate {
                file: d.file.clone(),
                line: d.line,
                kind: def_kind_str(d.kind).to_string(),
            })
            .collect();
        let data = RenameData {
            subcommand: "rename",
            dry_run: !apply,
            needs_anchor: Some(true),
            candidates: Some(candidates),
            applied: None,
            skipped: None,
            errors: None,
        };
        return Ok((serde_json::to_value(data)?, written_files));
    }

    // Pick the chosen definition. With --anchor present (or single def), this
    // narrows the resolved set to refs that resolve to that specific def.
    // Same-file multi-defs (def_files.len() == 1, defs.len() > 1) have no
    // cross-file ambiguity, so we don't require an anchor — treat as unfiltered.
    let chosen_def: Option<&Definition> = match (defs.len(), anchor) {
        (0, _) => None,
        (1, _) => Some(defs[0]),
        (_, Some(anchor)) => {
            let (file, line) = parse_anchor(anchor)?;
            Some(
                defs.iter()
                    .find(|d| d.file == file && d.line == line)
                    .copied()
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "anchor {file}:{line} did not match any definition of {old}",
                        )
                    })?,
            )
        }
        // Same-file multi-def (def_files.len() == 1) with no anchor: no
        // cross-file filtering needed; the resolver handles same-file scope.
        (_, None) => None,
    };

    // When chosen_def is set, filter resolved to refs that pin to that def.
    // Low-confidence refs (definition is None) stay in regardless of anchor —
    // they aren't tied to any specific def and we still want to surface them
    // as skipped[low-confidence].
    let resolved: Vec<Resolved<'_>> = if let Some(def) = chosen_def {
        resolved
            .into_iter()
            .filter(|r| match r.confidence {
                Confidence::Low => true,
                _ => match r.definition {
                    Some(d) => d.file == def.file && d.line == def.line && d.kind == def.kind,
                    None => r.reference.file == def.file,
                },
            })
            .collect()
    } else {
        resolved
    };

    let mut applied: Vec<AppliedFile> = Vec::new();
    let mut skipped: Vec<SkippedSite> = Vec::new();
    let mut errors: Vec<crate::edit::serialize::ErrorEntry> = Vec::new();

    // Low-confidence refs go to skipped[low-confidence].
    for r in &resolved {
        if matches!(r.confidence, Confidence::Low) {
            skipped.push(SkippedSite {
                file: r.reference.file.clone(),
                line: r.reference.line,
                col: r.reference.column,
                start_byte: r.reference.byte_offset,
                end_byte: r.reference.byte_offset + old.len(),
                name: r.reference.name.clone(),
                confidence: r.confidence.as_str(),
                reason: r.reason.as_str(),
                skip_reason: "low-confidence",
                via_alias: None,
                via_module: None,
            });
        }
    }

    // Alias re-export sites → skipped[re-export-alias].
    if let Some(sites) = idx.alias_reexports.get(old) {
        for site in sites {
            skipped.push(SkippedSite {
                file: site.file.clone(),
                line: site.line,
                col: 0,
                start_byte: 0,
                end_byte: 0,
                name: old.to_string(),
                confidence: "high",
                reason: "same-file-scope",
                skip_reason: "re-export-alias",
                via_alias: Some(site.original.clone()),
                via_module: None,
            });
        }
    }

    // Wildcard re-exports: surface every wildcard whose target module defines
    // a symbol named OLD. We use a lossy match (file stem == module path's
    // last segment) — same heuristic the resolver uses for wildcard imports.
    let defines_old: std::collections::HashSet<String> = idx
        .definitions
        .iter()
        .filter(|d| d.name == old)
        .map(|d| d.file.clone())
        .collect();

    for sites in idx.wildcard_reexports.values() {
        for site in sites {
            // Cheap match: the module path's tail segment appears in some
            // definition-file's path. e.g. module_path "crate::inner" tail
            // "inner" matches "src/inner.rs" in `defines_old`.
            let tail = site
                .module_path
                .rsplit("::")
                .next()
                .unwrap_or(&site.module_path);
            let related = defines_old.iter().any(|f| {
                let stem = std::path::Path::new(f)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                stem == tail
            });
            if !related {
                continue;
            }

            let already = skipped.iter().any(|s| {
                s.file == site.file && s.line == site.line && s.skip_reason == "wildcard-reexport"
            });
            if already {
                continue;
            }

            skipped.push(SkippedSite {
                file: site.file.clone(),
                line: site.line,
                col: 0,
                start_byte: 0,
                end_byte: 0,
                name: old.to_string(),
                confidence: "medium",
                reason: "import-resolved",
                skip_reason: "wildcard-reexport",
                via_alias: None,
                via_module: Some(site.module_path.clone()),
            });
        }
    }

    // High/Medium → queued for edit, minus any collision with an alias site.
    let alias_keys: std::collections::HashSet<(String, usize)> = idx
        .alias_reexports
        .get(old)
        .map(|sites| sites.iter().map(|s| (s.file.clone(), s.line)).collect())
        .unwrap_or_default();

    let mut by_file: std::collections::BTreeMap<String, Vec<&Resolved>> = Default::default();
    for r in &resolved {
        if !matches!(r.confidence, Confidence::High | Confidence::Medium) {
            continue;
        }
        if alias_keys.contains(&(r.reference.file.clone(), r.reference.line)) {
            continue;
        }
        by_file.entry(r.reference.file.clone()).or_default().push(r);
    }

    // Definition-site edits (ruling 77b4ae3d): the resolver yields references
    // only, so a definition whose name token is not itself captured as a
    // reference (fn/method defs; struct defs DO arrive as type_identifier
    // references) would survive an --apply untouched. Rename the chosen
    // definition's name token too — or every definition in the same-file
    // multi-def case, which runs unanchored. build_edits dedupes by
    // start_byte, so struct-style defs already present as references are not
    // edited twice.
    let defs_to_edit: Vec<&Definition> = match chosen_def {
        Some(d) => vec![d],
        None => defs.clone(),
    };
    let mut defs_by_file: std::collections::BTreeMap<String, Vec<&Definition>> =
        Default::default();
    for d in defs_to_edit {
        defs_by_file.entry(d.file.clone()).or_default().push(d);
    }

    // Union of files touched by reference edits and definition-site edits — a
    // definition in a file with no resolvable references must still be edited.
    let mut files: std::collections::BTreeSet<String> = by_file.keys().cloned().collect();
    files.extend(defs_by_file.keys().cloned());

    for file in files {
        let refs = by_file.get(&file).map(|v| v.as_slice()).unwrap_or(&[]);
        let def_sites = defs_by_file.get(&file).map(|v| v.as_slice()).unwrap_or(&[]);
        match build_edits(&file, refs, def_sites, old, new, apply, root, idx) {
            Ok(entry) => {
                if apply {
                    written_files.push(file.clone());
                }
                applied.push(entry);
            }
            Err(e) => errors.push(crate::edit::serialize::ErrorEntry::from(&e)),
        }
    }

    let data = RenameData {
        subcommand: "rename",
        dry_run: !apply,
        needs_anchor: None,
        candidates: None,
        applied: Some(applied),
        skipped: Some(skipped),
        errors: Some(errors),
    };

    Ok((serde_json::to_value(data)?, written_files))
}

#[allow(clippy::too_many_arguments)]
fn build_edits(
    file: &str,
    refs: &[&Resolved<'_>],
    def_sites: &[&Definition],
    old: &str,
    new: &str,
    apply: bool,
    root: &Path,
    idx: &Index,
) -> Result<AppliedFile, AstEditError> {
    let old_len = old.len();
    let new_len = new.len();
    let mut edits: Vec<AppliedEdit> = Vec::new();
    for r in refs {
        edits.push(AppliedEdit {
            line: r.reference.line,
            col: r.reference.column,
            start_byte: r.reference.byte_offset,
            end_byte: r.reference.byte_offset + old_len,
            old: old.to_string(),
            new: new.to_string(),
            confidence: r.confidence.as_str(),
            reason: r.reason.as_str(),
        });
    }
    // Definition name tokens, deduped by start_byte against the reference
    // edits: struct-style defs arrive as type_identifier references at the
    // exact same byte offset, and editing the span twice would corrupt the
    // splice.
    for d in def_sites {
        if edits.iter().any(|e| e.start_byte == d.name_start_byte) {
            continue;
        }
        edits.push(AppliedEdit {
            line: d.line,
            col: d.column,
            start_byte: d.name_start_byte,
            end_byte: d.name_end_byte,
            old: old.to_string(),
            new: new.to_string(),
            confidence: "high",
            reason: "definition",
        });
    }
    edits.sort_by_key(|e| std::cmp::Reverse(e.start_byte));
    let bytes_changed = (new_len as i64 - old_len as i64) * edits.len() as i64;

    if apply {
        let abs = root.join(file);

        // Step 5a: drift check. Skip the file if it changed since indexing.
        let meta = idx
            .file_meta
            .get(file)
            .ok_or_else(|| AstEditError::HashMismatch {
                file: file.to_string(),
            })?;
        crate::edit::apply::check_drift(&abs, file, meta, None)?;

        // Read after drift passes.
        let source = std::fs::read(&abs).map_err(|e| AstEditError::WriteFailed {
            file: file.to_string(),
            os_code: e.raw_os_error(),
            message: e.to_string(),
        })?;
        let original_len = source.len() as u64;
        let mut bytes = source;

        // Defensive node-kind check + splice in descending byte order.
        for e in &edits {
            if e.end_byte > bytes.len() || &bytes[e.start_byte..e.end_byte] != old.as_bytes() {
                return Err(AstEditError::NodeKindMismatch {
                    file: file.to_string(),
                    line: e.line,
                    col: e.col,
                });
            }
            bytes.splice(e.start_byte..e.end_byte, new.bytes());
        }

        // Step 5e: race-window guard. Re-stat just before the write and
        // compare against the length we read into memory. Same-length
        // concurrent writes slip through — accepted trade-off (spec).
        let current = crate::edit::apply::current_len(&abs, file)?;
        if current != original_len {
            return Err(AstEditError::ConcurrentWrite {
                file: file.to_string(),
            });
        }

        // Step 5f: atomic write.
        crate::edit::apply::write_atomic(&abs, &bytes)?;
    }

    Ok(AppliedFile {
        file: file.to_string(),
        bytes_changed,
        edits,
    })
}

fn def_kind_str(k: DefKind) -> &'static str {
    use crate::index::DefKind::*;
    match k {
        Fn => "fn",
        Struct => "struct",
        Enum => "enum",
        Trait => "trait",
        Class => "class",
        Interface => "interface",
        Type => "type",
        Const => "const",
        Method => "method",
    }
}

/// Parse a `--anchor FILE:LINE` value into `(file, line)`. The file is the
/// repo-relative, forward-slash-normalized form the index uses; the line is
/// 1-based.
fn parse_anchor(s: &str) -> Result<(String, usize)> {
    let (file, line) = s
        .rsplit_once(':')
        .ok_or_else(|| anyhow::anyhow!("--anchor expected FILE:LINE, got {s:?}"))?;
    let line: usize = line
        .parse()
        .map_err(|_| anyhow::anyhow!("--anchor line must be a positive integer, got {line:?}"))?;
    if line == 0 {
        anyhow::bail!("--anchor line must be 1-based, got 0");
    }
    Ok((file.to_string(), line))
}
