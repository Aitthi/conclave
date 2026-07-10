use serde::Serialize;

/// The wire-format payload for `astedit rename`. Wrapped by `output::print_json`
/// in `{schema_version: 1, data: <RenameData>}`.
#[derive(Debug, Serialize)]
#[allow(dead_code)]
pub struct RenameData {
    pub subcommand: &'static str, // always "rename"
    pub dry_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needs_anchor: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidates: Option<Vec<Candidate>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied: Option<Vec<AppliedFile>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<Vec<SkippedSite>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<ErrorEntry>>,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
pub struct Candidate {
    pub file: String,
    pub line: usize,
    pub kind: String, // serialized DefKind ("fn", "struct", ...)
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
pub struct AppliedFile {
    pub file: String,
    pub bytes_changed: i64,
    pub edits: Vec<AppliedEdit>,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
pub struct AppliedEdit {
    pub line: usize,
    pub col: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    pub old: String,
    pub new: String,
    pub confidence: &'static str, // "high" | "medium"
    pub reason: &'static str,     // ResolveReason::as_str()
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
pub struct SkippedSite {
    pub file: String,
    pub line: usize,
    pub col: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    pub name: String,
    pub confidence: &'static str,
    pub reason: &'static str,
    pub skip_reason: &'static str, // "low-confidence" | "re-export-alias" | "wildcard-reexport"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via_alias: Option<String>, // only for skip_reason == "re-export-alias"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via_module: Option<String>, // only for skip_reason == "wildcard-reexport"
}

#[derive(Debug, Serialize)]
#[allow(dead_code)]
pub struct ErrorEntry {
    pub error_kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub col: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
}

impl From<&crate::edit::error::AstEditError> for ErrorEntry {
    fn from(e: &crate::edit::error::AstEditError) -> Self {
        use crate::edit::error::AstEditError as E;
        let mut entry = ErrorEntry {
            error_kind: e.kind(),
            file: e.file().map(|s| s.to_string()),
            message: None,
            os_code: None,
            line: None,
            col: None,
            lang: None,
        };
        match e {
            E::ParseError { message, .. } => entry.message = Some(message.clone()),
            E::HashMismatch { .. } => {}
            E::ConcurrentWrite { .. } => {}
            E::NodeKindMismatch { line, col, .. } => {
                entry.line = Some(*line);
                entry.col = Some(*col);
            }
            E::WriteFailed {
                os_code, message, ..
            } => {
                entry.os_code = *os_code;
                entry.message = Some(message.clone());
            }
            E::PatternCompile { lang, message } => {
                entry.lang = Some(lang.clone());
                entry.message = Some(message.clone());
            }
        }
        entry
    }
}

/// The wire-format payload for `astedit rewrite`. Wrapped by `output::print_json`
/// in `{schema_version: 1, data: <RewriteData>}`. Parallel to `RenameData` —
/// kept separate (rather than a generic envelope) so the JSON shape is
/// auditable per subcommand at a glance.
#[derive(Debug, Serialize)]
pub struct RewriteData {
    pub subcommand: &'static str, // always "rewrite"
    pub dry_run: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied: Option<Vec<RewriteAppliedFile>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<ErrorEntry>>,
}

/// One file's worth of rewrite edits. Mirrors `AppliedFile` but uses `RewriteEdit`
/// (no confidence/reason fields per spec § Output schema).
#[derive(Debug, Serialize)]
pub struct RewriteAppliedFile {
    pub file: String,
    pub bytes_changed: i64,
    pub edits: Vec<RewriteEdit>,
}

/// A single structural rewrite edit. Confidence and reason are deliberately
/// omitted — structural matches are AST-shape exact (implicit "high").
#[derive(Debug, Serialize)]
pub struct RewriteEdit {
    pub line: usize,
    pub col: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    pub old: String,
    pub new: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn rename_data_serializes_omitting_none_fields() {
        let data = RenameData {
            subcommand: "rename",
            dry_run: true,
            needs_anchor: None,
            candidates: None,
            applied: Some(vec![]),
            skipped: Some(vec![]),
            errors: Some(vec![]),
        };
        let v: Value = serde_json::to_value(&data).unwrap();
        assert_eq!(v["subcommand"], "rename");
        assert_eq!(v["dry_run"], true);
        assert!(v.get("needs_anchor").is_none());
        assert!(v.get("candidates").is_none());
        assert!(v["applied"].is_array());
        assert!(v["skipped"].is_array());
        assert!(v["errors"].is_array());
    }

    #[test]
    fn error_entry_from_write_failed_carries_os_code() {
        let err = crate::edit::error::AstEditError::WriteFailed {
            file: "src/x.rs".into(),
            os_code: Some(13),
            message: "permission denied".into(),
        };
        let entry: ErrorEntry = (&err).into();
        let v = serde_json::to_value(&entry).unwrap();
        assert_eq!(v["error_kind"], "write-failed");
        assert_eq!(v["file"], "src/x.rs");
        assert_eq!(v["os_code"], 13);
        assert_eq!(v["message"], "permission denied");
    }

    #[test]
    fn skipped_site_alias_carries_via_alias_only() {
        let s = SkippedSite {
            file: "src/lib.rs".into(),
            line: 4,
            col: 0,
            start_byte: 10,
            end_byte: 14,
            name: "User".into(),
            confidence: "high",
            reason: "same-file-scope",
            skip_reason: "re-export-alias",
            via_alias: Some("Account".into()),
            via_module: None,
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["skip_reason"], "re-export-alias");
        assert_eq!(v["via_alias"], "Account");
        assert!(v.get("via_module").is_none());
    }

    #[test]
    fn rewrite_data_serializes_omitting_none_fields() {
        let data = RewriteData {
            subcommand: "rewrite",
            dry_run: true,
            applied: Some(vec![]),
            errors: Some(vec![]),
        };
        let v: Value = serde_json::to_value(&data).unwrap();
        assert_eq!(v["subcommand"], "rewrite");
        assert_eq!(v["dry_run"], true);
        assert!(v["applied"].is_array());
        assert!(v["errors"].is_array());
        assert!(v.get("needs_anchor").is_none());
        assert!(v.get("candidates").is_none());
    }

    #[test]
    fn rewrite_edit_serializes_without_confidence_or_reason() {
        let edit = RewriteEdit {
            line: 4,
            col: 7,
            start_byte: 100,
            end_byte: 108,
            old: "console.log".into(),
            new: "console.error".into(),
        };
        let v: Value = serde_json::to_value(&edit).unwrap();
        assert_eq!(v["line"], 4);
        assert_eq!(v["col"], 7);
        assert_eq!(v["start_byte"], 100);
        assert_eq!(v["end_byte"], 108);
        assert_eq!(v["old"], "console.log");
        assert_eq!(v["new"], "console.error");
        assert!(
            v.get("confidence").is_none(),
            "rewrite edit must not carry confidence: {v:?}"
        );
        assert!(
            v.get("reason").is_none(),
            "rewrite edit must not carry reason: {v:?}"
        );
    }

    #[test]
    fn rewrite_applied_file_shape_matches_spec() {
        let file = RewriteAppliedFile {
            file: "src/lib.rs".into(),
            bytes_changed: 12,
            edits: vec![RewriteEdit {
                line: 1,
                col: 0,
                start_byte: 0,
                end_byte: 4,
                old: "User".into(),
                new: "Account".into(),
            }],
        };
        let v: Value = serde_json::to_value(&file).unwrap();
        assert_eq!(v["file"], "src/lib.rs");
        assert_eq!(v["bytes_changed"], 12);
        assert!(v["edits"].is_array());
        assert_eq!(v["edits"][0]["old"], "User");
        assert_eq!(v["edits"][0]["new"], "Account");
    }
}
