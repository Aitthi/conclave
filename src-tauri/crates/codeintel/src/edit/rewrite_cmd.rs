//! `rewrite` verb — ported from astedit's `commands/rewrite.rs`. The CLI's
//! `clap` args and `print_json`/exit-code plumbing are replaced by direct
//! function parameters and a returned `(serde_json::Value, Vec<String>)`.
//! Unlike `rename`, `rewrite` never consumed the shared `Index` — it keeps
//! its own `walk_sources` call, matching the original.

use std::path::Path;

use anyhow::Result;
use serde_json::Value;

use crate::edit::error::AstEditError;
use crate::edit::rewrite::{rewrite_file, RewriteSite};
use crate::edit::serialize::{ErrorEntry, RewriteAppliedFile, RewriteData, RewriteEdit};
use crate::lang::Language as CgLang;
use crate::walk::walk_sources;

/// Structural rewrite of `pattern` → `template` (ast-grep pattern syntax)
/// across the project rooted at `root`. Dry-run unless `apply` is true.
/// `lang` restricts the walk to a single language; without it, every
/// supported extension is scanned and the pattern is compiled per language
/// on demand.
///
/// Returns `(data, written_files)` where `data` is astedit's `RewriteData`
/// shape verbatim and `written_files` lists the repo-relative paths actually
/// written to disk (empty unless `apply` is true). The former process exit
/// code 2 (pattern-compile failure) is now fully represented in the
/// `errors[]` lane of `data` — callers get `Ok`.
pub fn rewrite(
    root: &Path,
    pattern: &str,
    template: &str,
    apply: bool,
    lang: Option<&str>,
) -> Result<(Value, Vec<String>)> {
    let lang_filter = parse_lang_filter(lang)?;

    let mut applied: Vec<RewriteAppliedFile> = Vec::new();
    let mut errors: Vec<ErrorEntry> = Vec::new();
    let mut written_files: Vec<String> = Vec::new();

    let sources = walk_sources(root)?;

    // Pass 1: discover which languages we'd process. `Language` derives
    // `Hash`/`Eq` in this crate, but we still dedup against a `Vec` — the
    // list is tiny (one entry per language actually present) and this
    // preserves the order languages first appear in the walk.
    let mut langs_to_process: Vec<CgLang> = Vec::new();
    for src in &sources {
        if let Some(target) = lang_filter {
            if src.language != target {
                continue;
            }
        }
        if !langs_to_process.contains(&src.language) {
            langs_to_process.push(src.language);
        }
    }

    // Compile (pattern, rewrite) once per language. Empty source string is a
    // pure compile-only smoke test — ast-grep's `Pattern::try_new` and
    // `TemplateFix::try_new` run before any source is parsed.
    let mut had_compile_failure = false;
    for &l in &langs_to_process {
        if let Err(e) = rewrite_file("", pattern, template, l) {
            if matches!(e, AstEditError::PatternCompile { .. }) {
                had_compile_failure = true;
            }
            errors.push(ErrorEntry::from(&e));
        }
    }

    // Pass 2: only walk + match + apply when all languages compiled cleanly.
    if !had_compile_failure {
        for src in sources {
            if let Some(target) = lang_filter {
                if src.language != target {
                    continue;
                }
            }

            let rel = relative_path(&src.path, root);
            let source_text = match std::fs::read_to_string(&src.path) {
                Ok(s) => s,
                Err(e) => {
                    errors.push(ErrorEntry::from(&AstEditError::ParseError {
                        file: rel.clone(),
                        message: format!("read failed: {e}"),
                    }));
                    continue;
                }
            };

            let sites = match rewrite_file(&source_text, pattern, template, src.language) {
                Ok(sites) => sites,
                Err(e) => {
                    errors.push(ErrorEntry::from(&e));
                    continue;
                }
            };

            if sites.is_empty() {
                continue;
            }

            match apply_or_dry_run(&src.path, &rel, &source_text, &sites, apply) {
                Ok(entry) => {
                    if apply {
                        written_files.push(rel.clone());
                    }
                    applied.push(entry);
                }
                Err(e) => errors.push(ErrorEntry::from(&e)),
            }
        }
    }

    let data = RewriteData {
        subcommand: "rewrite",
        dry_run: !apply,
        applied: Some(applied),
        errors: Some(errors),
    };

    Ok((serde_json::to_value(data)?, written_files))
}

fn parse_lang_filter(lang: Option<&str>) -> Result<Option<CgLang>> {
    match lang {
        None => Ok(None),
        Some(s) => match s {
            "rust" => Ok(Some(CgLang::Rust)),
            "typescript" => Ok(Some(CgLang::TypeScript)),
            "tsx" => Ok(Some(CgLang::Tsx)),
            "javascript" => Ok(Some(CgLang::JavaScript)),
            "python" => Ok(Some(CgLang::Python)),
            other => Err(anyhow::anyhow!(
                "--lang {other:?} not supported; valid: rust, typescript, tsx, javascript, python"
            )),
        },
    }
}

fn relative_path(abs: &Path, root: &Path) -> String {
    abs.strip_prefix(root)
        .unwrap_or(abs)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Materialise `sites` into a `RewriteAppliedFile`. When `apply` is true,
/// also splice the bytes (reverse byte order), guard the race window, and
/// atomically write via `crate::edit::apply::write_atomic`.
fn apply_or_dry_run(
    abs: &Path,
    rel: &str,
    source: &str,
    sites: &[RewriteSite],
    apply: bool,
) -> Result<RewriteAppliedFile, AstEditError> {
    let mut edits: Vec<RewriteEdit> = sites
        .iter()
        .map(|s| RewriteEdit {
            line: s.line,
            col: s.col,
            start_byte: s.start_byte,
            end_byte: s.end_byte,
            old: s.old.clone(),
            new: s.new.clone(),
        })
        .collect();
    edits.sort_by_key(|e| std::cmp::Reverse(e.start_byte));

    let bytes_changed: i64 = edits
        .iter()
        .map(|e| e.new.len() as i64 - e.old.len() as i64)
        .sum();

    if apply {
        let original_len = source.len() as u64;
        let mut bytes = source.as_bytes().to_vec();
        for e in &edits {
            if e.end_byte > bytes.len() || &bytes[e.start_byte..e.end_byte] != e.old.as_bytes() {
                return Err(AstEditError::NodeKindMismatch {
                    file: rel.to_string(),
                    line: e.line,
                    col: e.col,
                });
            }
            bytes.splice(e.start_byte..e.end_byte, e.new.bytes());
        }

        let current = crate::edit::apply::current_len(abs, rel)?;
        if current != original_len {
            return Err(AstEditError::ConcurrentWrite {
                file: rel.to_string(),
            });
        }
        crate::edit::apply::write_atomic(abs, &bytes)?;
    }

    Ok(RewriteAppliedFile {
        file: rel.to_string(),
        bytes_changed,
        edits,
    })
}
