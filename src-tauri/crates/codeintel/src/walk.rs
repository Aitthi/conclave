use crate::lang::Language;
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};

pub const IGNORED_DIRS: &[&str] = &["target", "node_modules", ".git", "dist", "build"];

pub struct SourceFile {
    pub path: PathBuf,
    pub language: Language,
}

/// Pre-configured walker honouring `.gitignore` and skipping `IGNORED_DIRS`.
/// Other modules in this crate build on top of it so the exclusion list lives in one place.
pub fn default_walker(root: &Path) -> WalkBuilder {
    let mut b = WalkBuilder::new(root);
    b.hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !IGNORED_DIRS.iter().any(|d| *d == name.as_ref())
        });
    b
}

/// Walk `root`, returning files whose extension maps to a recognised `Language`.
pub fn walk_sources(root: &Path) -> anyhow::Result<Vec<SourceFile>> {
    let mut out = Vec::new();
    for entry in default_walker(root).build() {
        let entry = entry?;
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.into_path();
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if let Some(language) = Language::from_extension(ext) {
            out.push(SourceFile { path, language });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}
