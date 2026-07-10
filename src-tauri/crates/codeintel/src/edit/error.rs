use thiserror::Error;

/// Closed enum of every condition the edit verbs (rename/rewrite) report
/// through the `errors[]` lane of the JSON payload. Variant names map 1:1 to
/// the spec-locked `error_kind` strings via `kind()` — used by
/// `crate::edit::serialize::ErrorEntry` to emit the JSON. We deliberately do
/// not derive `Serialize`: the public JSON shape lives in
/// `crate::edit::serialize`, which builds it explicitly so the wire schema
/// cannot drift from variant renaming.
///
/// Ported from `astedit::error::AstEditError` (src-tauri/crates/codeintel is
/// the astedit port target; see codeintel Task 5 brief).
#[derive(Debug, Error)]
pub enum AstEditError {
    #[error("parse error in {file}: {message}")]
    ParseError { file: String, message: String },

    #[error("file changed between index and apply: {file}")]
    HashMismatch { file: String },

    #[error("concurrent write detected on {file}")]
    ConcurrentWrite { file: String },

    #[error("node kind mismatch at {file}:{line}:{col}")]
    NodeKindMismatch {
        file: String,
        line: usize,
        col: usize,
    },

    #[error("write failed on {file}: {message}")]
    WriteFailed {
        file: String,
        os_code: Option<i32>,
        message: String,
    },

    #[error("pattern failed to compile for {lang}: {message}")]
    PatternCompile { lang: String, message: String },
}

impl AstEditError {
    /// The kebab-case string emitted as `error_kind` in the JSON payload.
    /// Used by `crate::edit::serialize::ErrorEntry` when building the JSON response.
    pub fn kind(&self) -> &'static str {
        match self {
            AstEditError::ParseError { .. } => "parse-error",
            AstEditError::HashMismatch { .. } => "hash-mismatch",
            AstEditError::ConcurrentWrite { .. } => "concurrent-write",
            AstEditError::NodeKindMismatch { .. } => "node-kind-mismatch",
            AstEditError::WriteFailed { .. } => "write-failed",
            AstEditError::PatternCompile { .. } => "pattern-compile",
        }
    }

    /// The repo-relative file path the error is attributed to, if any.
    /// `PatternCompile` is not file-scoped (it errors before any file is
    /// touched), so returns `None` for that variant.
    /// Used by `crate::edit::serialize::ErrorEntry` when building the JSON response.
    pub fn file(&self) -> Option<&str> {
        match self {
            AstEditError::ParseError { file, .. }
            | AstEditError::HashMismatch { file }
            | AstEditError::ConcurrentWrite { file }
            | AstEditError::NodeKindMismatch { file, .. }
            | AstEditError::WriteFailed { file, .. } => Some(file),
            AstEditError::PatternCompile { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_kind_strings_match_spec() {
        assert_eq!(
            AstEditError::ParseError {
                file: "x".into(),
                message: "y".into()
            }
            .kind(),
            "parse-error"
        );
        assert_eq!(
            AstEditError::HashMismatch { file: "x".into() }.kind(),
            "hash-mismatch"
        );
        assert_eq!(
            AstEditError::ConcurrentWrite { file: "x".into() }.kind(),
            "concurrent-write"
        );
        assert_eq!(
            AstEditError::NodeKindMismatch {
                file: "x".into(),
                line: 1,
                col: 1
            }
            .kind(),
            "node-kind-mismatch"
        );
        assert_eq!(
            AstEditError::WriteFailed {
                file: "x".into(),
                os_code: None,
                message: "y".into()
            }
            .kind(),
            "write-failed"
        );
        assert_eq!(
            AstEditError::PatternCompile {
                lang: "rust".into(),
                message: "y".into()
            }
            .kind(),
            "pattern-compile"
        );
    }
}
