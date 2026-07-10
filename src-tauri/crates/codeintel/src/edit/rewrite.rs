//! ast-grep helper used by `commands/rewrite.rs`. Centralises pattern
//! compilation, per-language dispatch, and match-to-edit materialisation
//! so the command handler can focus on file IO + the JSON envelope.

use ast_grep_core::matcher::Pattern;
use ast_grep_core::replacer::{Replacer, TemplateFix};
use ast_grep_core::tree_sitter::{LanguageExt, StrDoc, Visitor};
use ast_grep_language::{JavaScript, Python, Rust, Tsx, TypeScript};
use crate::lang::Language as CgLang;

use crate::edit::error::AstEditError;

/// A structural match site materialised into the byte/line/col coordinates
/// the JSON envelope needs. Produced by `rewrite_file`.
#[derive(Debug, Clone)]
pub struct RewriteSite {
    pub start_byte: usize,
    pub end_byte: usize,
    pub line: usize,
    pub col: usize,
    pub old: String,
    pub new: String,
}

/// Compile (pattern, rewrite) for `lang` and return every match site as a
/// `RewriteSite` carrying both byte coordinates and the materialised
/// replacement text.
pub fn rewrite_file(
    source: &str,
    pattern: &str,
    rewrite: &str,
    lang: CgLang,
) -> Result<Vec<RewriteSite>, AstEditError> {
    match lang {
        CgLang::Rust => collect_sites(source, pattern, rewrite, Rust, lang.name()),
        CgLang::TypeScript => collect_sites(source, pattern, rewrite, TypeScript, lang.name()),
        CgLang::Tsx => collect_sites(source, pattern, rewrite, Tsx, lang.name()),
        CgLang::JavaScript => collect_sites(source, pattern, rewrite, JavaScript, lang.name()),
        CgLang::Python => collect_sites(source, pattern, rewrite, Python, lang.name()),
    }
}

/// Per-language workhorse. Generic over the ast-grep language unit struct so
/// the per-arm `match` above stays a straight dispatch table — pattern compile,
/// template compile, parse + visit, then materialise each match into a
/// `RewriteSite`.
fn collect_sites<L>(
    source: &str,
    pattern: &str,
    rewrite: &str,
    lang: L,
    lang_name: &str,
) -> Result<Vec<RewriteSite>, AstEditError>
where
    L: LanguageExt + Clone + 'static,
{
    let compiled =
        Pattern::try_new(pattern, lang.clone()).map_err(|e| AstEditError::PatternCompile {
            lang: lang_name.to_string(),
            message: e.to_string(),
        })?;
    // Tree-sitter is lenient and may parse a broken pattern into a tree
    // containing ERROR nodes; treat those as a compile failure too so we
    // never run an obviously-malformed pattern against real source.
    if compiled.has_error() {
        return Err(AstEditError::PatternCompile {
            lang: lang_name.to_string(),
            message: format!("pattern `{pattern}` contains syntax errors"),
        });
    }

    let fixer = TemplateFix::try_new(rewrite, &lang).map_err(|e| AstEditError::PatternCompile {
        lang: lang_name.to_string(),
        message: format!("invalid rewrite template: {e}"),
    })?;

    let root = lang.ast_grep(source);
    let mut out = Vec::new();
    for m in Visitor::new(&compiled).visit(root.root()) {
        let range = m.range();
        let pos = m.start_pos();
        let (row, byte_col) = pos.byte_point();
        let new_bytes = <TemplateFix as Replacer<StrDoc<L>>>::generate_replacement(&fixer, &m);
        let new_text = String::from_utf8(new_bytes).map_err(|e| AstEditError::ParseError {
            file: lang_name.to_string(),
            message: format!("rewrite produced non-utf8 bytes: {e}"),
        })?;
        out.push(RewriteSite {
            start_byte: range.start,
            end_byte: range.end,
            // ast-grep yields zero-based row/col; the JSON envelope is
            // documented as 1-based, so bump both here.
            line: row.saturating_add(1),
            col: byte_col.saturating_add(1),
            old: m.text().into_owned(),
            new: new_text,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_file_rust_simple_pattern() {
        let source = "fn make() { println!(\"hi\"); }";
        let sites = rewrite_file(source, "println!($A)", "eprintln!($A)", CgLang::Rust)
            .expect("compile + match");
        assert_eq!(sites.len(), 1, "one match expected; got {sites:?}");
        let s = &sites[0];
        assert_eq!(s.old, "println!(\"hi\")");
        assert_eq!(s.new, "eprintln!(\"hi\")");
        assert!(s.start_byte < s.end_byte);
        assert_eq!(s.line, 1);
    }

    #[test]
    fn rewrite_file_no_match_returns_empty_vec() {
        let source = "fn main() {}";
        let sites = rewrite_file(source, "println!($A)", "eprintln!($A)", CgLang::Rust)
            .expect("compile ok, no matches");
        assert!(sites.is_empty(), "expected no matches; got {sites:?}");
    }

    #[test]
    fn rewrite_file_invalid_pattern_returns_pattern_compile() {
        // Use a syntactically broken pattern. Replace this input if `(((`
        // happens to compile cleanly on the resolved ast-grep version (try
        // `"$"` or `""` if so).
        let source = "fn main() {}";
        let err = rewrite_file(source, "(((", "fn main() {}", CgLang::Rust)
            .expect_err("expected pattern compile failure");
        assert_eq!(err.kind(), "pattern-compile", "got: {err:?}");
        if let AstEditError::PatternCompile { lang, .. } = err {
            assert_eq!(lang, "rust");
        }
    }
}
