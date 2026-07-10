//! Ten-line execution-header contract for council-planned tasks
//! (`conclave-plan:v1`).
//!
//! Grammar and field rules are frozen by
//! `docs/superpowers/specs/2026-07-10-lead-council-v1-design.md`:
//! line 1 is the Markdown title, line 2 the version opener
//! `<!-- conclave-plan:v1`, lines 3-9 a JSON object body, and line 10 the
//! closing delimiter `} -->`.
//!
//! This module is PURE parsing/validation: no database access, no filesystem
//! reads, no hashing. Checks that need workspace/DB/git context (owner equals
//! `task.ownerAgentId`, council agents belong to the workspace, `baseSha` is
//! an ancestor of the checkout, symlink resolution of paths on disk) belong to
//! the `task.planCheck` engine command, which builds on the accessors and
//! helpers exposed here. Anchor-existence checks therefore take file CONTENT
//! as a `&str` argument rather than reading disk.
// Consumed by the `task.planCheck` command (next edit group in the plan);
// suppress dead-code until that caller lands, mirroring `error.rs`.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::engine::AppError;

/// Version token carried on line 2 of every contract header.
pub const CONTRACT_VERSION: &str = "conclave-plan:v1";
/// Exact line-2 opener.
pub const VERSION_OPENER: &str = "<!-- conclave-plan:v1";
/// Exact line-10 closing delimiter.
pub const CLOSING_DELIMITER: &str = "} -->";
/// The contract occupies exactly the first ten lines of the plan.
pub const HEADER_LINE_COUNT: usize = 10;
/// The parser never scans past this many bytes looking for the header.
pub const HEADER_MAX_BYTES: usize = 16 * 1024;

// Conservative caps (design spec: "Arrays and strings have conservative
// length caps so the ten-line header cannot become a context dump").
// Exact values are delegated implementer judgment, sized off the real
// council plans in docs/superpowers/plans/ with generous headroom:
/// Agent ids are UUIDs (36 chars); allow headroom.
pub const MAX_ID_LEN: usize = 64;
/// Repo-relative path length cap.
pub const MAX_PATH_LEN: usize = 240;
/// Exact-text anchor length cap.
pub const MAX_ANCHOR_LEN: usize = 200;
/// Single gate command length cap.
pub const MAX_GATE_LEN: usize = 300;
/// `readingOrder` entry count cap.
pub const MAX_READING_ORDER: usize = 24;
/// `boundary` entry count cap.
pub const MAX_BOUNDARY: usize = 64;
/// `consumes` entry count cap.
pub const MAX_CONSUMES: usize = 64;
/// `produces` entry count cap.
pub const MAX_PRODUCES: usize = 64;
/// `gates` entry count cap.
pub const MAX_GATES: usize = 16;
/// Council `members` count cap (chair is carried separately).
pub const MAX_COUNCIL_MEMBERS: usize = 8;

/// Markdown sections (as `## <name>` headings) required after line 10.
pub const REQUIRED_SECTIONS: [&str; 8] = [
    "Goal",
    "Non-goals",
    "Decisions",
    "Ordered edits",
    "Verification",
    "Risks",
    "Rejected alternatives",
    "Escalation",
];

// Prose checks are deliberately NARROW (plan risk note: "A broad prose
// validator will create false positives. Restrict prose checks to required
// headings and explicit placeholder/discovery markers.").
//
// Placeholder markers: uppercase editorial tokens matched at word boundaries
// only, so ordinary prose ("todo list", "fixmestring") never trips them.
// Chosen list is delegated implementer judgment: TBD / TODO / FIXME / TKTK
// are the established leave-a-hole conventions; angle-bracket placeholders
// are NOT matched because legitimate CLI usage strings like
// `--watchers <comma-separated-ids>` appear in real plans.
const PLACEHOLDER_MARKERS: [&str; 4] = ["TBD", "TODO", "FIXME", "TKTK"];

// Broad-discovery instructions: explicit case-insensitive phrases that tell
// the implementer to sweep the repository instead of naming a path or anchor.
// Chosen list is delegated implementer judgment; single words like "search"
// or "grep" alone are intentionally not flagged.
const DISCOVERY_PHRASES: [&str; 7] = [
    "search the codebase",
    "search the repo",
    "grep the codebase",
    "grep the repo",
    "explore the codebase",
    "search broadly",
    "repo-wide search",
];

/// Validation and parse failures for the plan contract.
///
/// Converts into [`AppError::Invalid`] so `task.planCheck` handlers can
/// `?`-chain directly.
#[derive(Debug, Error)]
pub enum PlanContractError {
    /// The ten-line envelope is malformed (title, opener, delimiter, length).
    #[error("plan header structure: {0}")]
    Structure(String),
    /// Lines 3-10 are not the expected JSON object (includes unknown fields).
    #[error("plan header json: {0}")]
    Json(String),
    /// A header field violates the design-spec field rules.
    #[error("plan header field `{field}`: {reason}")]
    Field { field: &'static str, reason: String },
    /// The plan body after line 10 fails a heading or marker check.
    #[error("plan body: {0}")]
    Body(String),
    /// Stored snapshot header and canonical `planPath` header differ.
    #[error("plan header mismatch: stored snapshot and canonical plan file headers differ")]
    HeaderMismatch,
    /// A consumed `path#anchor` whose exact anchor text is absent.
    #[error("consumed anchor not found: `{0}`")]
    MissingAnchor(String),
    /// A produced path that falls outside the task boundary.
    #[error("produced path outside boundary: `{0}`")]
    OutsideBoundary(String),
}

impl From<PlanContractError> for AppError {
    fn from(e: PlanContractError) -> Self {
        AppError::Invalid(e.to_string())
    }
}

/// Optional council block on line 4 of the contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CouncilHeader {
    pub chair: String,
    pub members: Vec<String>,
    pub max_rounds: u32,
}

/// The parsed ten-line execution contract (lines 2-10 JSON payload).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionHeader {
    pub owner: String,
    pub authority: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub council: Option<CouncilHeader>,
    pub plan_path: String,
    pub base_sha: String,
    pub escalation: String,
    pub reading_order: Vec<String>,
    pub boundary: Vec<String>,
    pub consumes: Vec<String>,
    pub produces: Vec<String>,
    pub gates: Vec<String>,
}

/// Extract exactly the first ten lines, scanning at most
/// [`HEADER_MAX_BYTES`]. Trailing `\r` is stripped per line so CRLF and LF
/// plans canonicalize identically.
fn header_lines(plan: &str) -> Result<Vec<&str>, PlanContractError> {
    let mut end = plan.len().min(HEADER_MAX_BYTES);
    while !plan.is_char_boundary(end) {
        end -= 1;
    }
    let prefix = &plan[..end];
    let lines: Vec<&str> = prefix
        .split('\n')
        .take(HEADER_LINE_COUNT)
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    if lines.len() < HEADER_LINE_COUNT {
        return Err(PlanContractError::Structure(format!(
            "expected at least {HEADER_LINE_COUNT} lines within the first {HEADER_MAX_BYTES} bytes, found {}",
            lines.len()
        )));
    }
    let title = lines[0]
        .strip_prefix("# ")
        .map(str::trim)
        .unwrap_or_default();
    if title.is_empty() {
        return Err(PlanContractError::Structure(
            "line 1 must be a non-empty Markdown title (`# <title>`)".into(),
        ));
    }
    if lines[1].trim_end() != VERSION_OPENER {
        return Err(PlanContractError::Structure(format!(
            "line 2 must be exactly `{VERSION_OPENER}`"
        )));
    }
    if lines[HEADER_LINE_COUNT - 1].trim_end() != CLOSING_DELIMITER {
        return Err(PlanContractError::Structure(format!(
            "line {HEADER_LINE_COUNT} must be exactly `{CLOSING_DELIMITER}`"
        )));
    }
    Ok(lines)
}

/// Parse the ten-line header into an [`ExecutionHeader`]
/// (`deny_unknown_fields`). Structural envelope check + JSON only; call
/// [`ExecutionHeader::validate`] (or [`parse_and_validate_header`]) for the
/// field rules.
pub fn parse_header(plan: &str) -> Result<ExecutionHeader, PlanContractError> {
    let lines = header_lines(plan)?;
    // JSON object = lines 3-9 plus the `}` that opens line 10.
    let json_text = format!("{}\n}}", lines[2..HEADER_LINE_COUNT - 1].join("\n"));
    serde_json::from_str(&json_text).map_err(|e| PlanContractError::Json(e.to_string()))
}

/// Parse and fully field-validate the header in one call.
pub fn parse_and_validate_header(plan: &str) -> Result<ExecutionHeader, PlanContractError> {
    let header = parse_header(plan)?;
    header.validate()?;
    Ok(header)
}

/// The canonical header text: the exact first ten lines (CR-stripped),
/// joined with `\n`. This is the unit of the snapshot-equality rule.
pub fn canonical_header(plan: &str) -> Result<String, PlanContractError> {
    Ok(header_lines(plan)?.join("\n"))
}

/// Require canonical header equality between the immutable stored plan
/// snapshot and the current `planPath` file content.
pub fn check_header_equality(
    stored_plan: &str,
    current_plan: &str,
) -> Result<(), PlanContractError> {
    if canonical_header(stored_plan)? == canonical_header(current_plan)? {
        Ok(())
    } else {
        Err(PlanContractError::HeaderMismatch)
    }
}

impl ExecutionHeader {
    /// Validate every structurally checkable field rule from the design
    /// spec. DB/git-context rules (owner equals `task.ownerAgentId`, agents
    /// belong to the workspace, `baseSha` is an ancestor) are the caller's.
    pub fn validate(&self) -> Result<(), PlanContractError> {
        validate_agent_id("owner", &self.owner)?;
        if self.authority != "in-loop" {
            return Err(PlanContractError::Field {
                field: "authority",
                reason: format!("must be `in-loop`, got `{}`", self.authority),
            });
        }
        validate_agent_id("escalation", &self.escalation)?;
        if let Some(council) = &self.council {
            council.validate(&self.owner)?;
        }

        if self.base_sha.len() != 40
            || !self
                .base_sha
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(PlanContractError::Field {
                field: "baseSha",
                reason: "must be a full 40-character lowercase hex commit id".into(),
            });
        }

        validate_repo_path("planPath", &self.plan_path)?;

        check_array_len(
            "readingOrder",
            self.reading_order.len(),
            1,
            MAX_READING_ORDER,
        )?;
        for entry in &self.reading_order {
            validate_entry("readingOrder", entry)?;
        }
        if !self.reading_order.iter().any(|e| e == &self.plan_path) {
            return Err(PlanContractError::Field {
                field: "planPath",
                reason: "must appear as a plain (anchor-free) entry in readingOrder".into(),
            });
        }

        check_array_len("boundary", self.boundary.len(), 1, MAX_BOUNDARY)?;
        for path in &self.boundary {
            validate_repo_path("boundary", path)?;
        }
        // The boundary is the normalized, SORTED set-equivalent of
        // task.fileBoundary — require strictly ascending (sorted, no dupes).
        if !self.boundary.windows(2).all(|w| w[0] < w[1]) {
            return Err(PlanContractError::Field {
                field: "boundary",
                reason: "must be sorted ascending with no duplicates".into(),
            });
        }

        check_array_len("consumes", self.consumes.len(), 0, MAX_CONSUMES)?;
        for entry in &self.consumes {
            let (_, anchor) = validate_entry("consumes", entry)?;
            if anchor.is_none() {
                return Err(PlanContractError::Field {
                    field: "consumes",
                    reason: format!("entry `{entry}` must name an exact `path#anchor`"),
                });
            }
        }

        check_array_len("produces", self.produces.len(), 0, MAX_PRODUCES)?;
        for entry in &self.produces {
            let (path, _) = validate_entry("produces", entry)?;
            // Produced paths must fall inside the writable boundary; the
            // anchor may be new, so only the path part is checked here.
            if !path_in_boundary(path, &self.boundary) {
                return Err(PlanContractError::OutsideBoundary(entry.clone()));
            }
        }

        check_array_len("gates", self.gates.len(), 1, MAX_GATES)?;
        for gate in &self.gates {
            if gate.trim().is_empty() || gate.len() > MAX_GATE_LEN {
                return Err(PlanContractError::Field {
                    field: "gates",
                    reason: format!("each command must be 1..={MAX_GATE_LEN} non-blank chars"),
                });
            }
            if gate.chars().any(|c| c.is_control()) {
                return Err(PlanContractError::Field {
                    field: "gates",
                    reason: "commands must not contain control characters".into(),
                });
            }
        }
        Ok(())
    }
}

impl CouncilHeader {
    /// Chair equals owner; chair plus members are at least three distinct
    /// agents; `maxRounds` is between 1 and 4.
    fn validate(&self, owner: &str) -> Result<(), PlanContractError> {
        validate_agent_id("council.chair", &self.chair)?;
        if self.chair != owner {
            return Err(PlanContractError::Field {
                field: "council.chair",
                reason: "must equal `owner`".into(),
            });
        }
        check_array_len(
            "council.members",
            self.members.len(),
            2,
            MAX_COUNCIL_MEMBERS,
        )?;
        for (i, member) in self.members.iter().enumerate() {
            validate_agent_id("council.members", member)?;
            if member == &self.chair || self.members[..i].contains(member) {
                return Err(PlanContractError::Field {
                    field: "council.members",
                    reason: "chair plus members must be distinct agents".into(),
                });
            }
        }
        if !(1..=4).contains(&self.max_rounds) {
            return Err(PlanContractError::Field {
                field: "council.maxRounds",
                reason: format!("must be between 1 and 4, got {}", self.max_rounds),
            });
        }
        Ok(())
    }
}

/// Split a `readingOrder`/`consumes`/`produces` entry at the FIRST `#` into
/// `(path, anchor)`. A plain path returns `(path, None)`. The anchor is an
/// exact text fragment and may itself contain `#`.
pub fn split_anchor(entry: &str) -> (&str, Option<&str>) {
    match entry.split_once('#') {
        Some((path, anchor)) => (path, Some(anchor)),
        None => (entry, None),
    }
}

/// Validate one list entry: path rules on the path part, caps and
/// no-control-chars on the optional anchor part. Returns the split parts.
fn validate_entry<'a>(
    field: &'static str,
    entry: &'a str,
) -> Result<(&'a str, Option<&'a str>), PlanContractError> {
    let (path, anchor) = split_anchor(entry);
    validate_repo_path(field, path)?;
    if let Some(anchor) = anchor {
        if anchor.is_empty() || anchor.len() > MAX_ANCHOR_LEN {
            return Err(PlanContractError::Field {
                field,
                reason: format!("anchor in `{entry}` must be 1..={MAX_ANCHOR_LEN} chars"),
            });
        }
        if anchor.chars().any(|c| c.is_control()) {
            return Err(PlanContractError::Field {
                field,
                reason: format!("anchor in `{entry}` must not contain control characters"),
            });
        }
    }
    Ok((path, anchor))
}

/// Lexical repo-relative path validation: rejects absolute forms,
/// backslashes, drive/scheme colons, `~`, empty components, `.` and `..`,
/// embedded `#`, control characters, and over-length paths. A path that
/// passes is already in normalized form, so no rewriting is performed.
///
/// Symlink escape from the checkout root requires filesystem resolution and
/// is deliberately left to the disk-touching caller (`task.planCheck`).
pub fn validate_repo_path(field: &'static str, path: &str) -> Result<(), PlanContractError> {
    let fail = |reason: String| PlanContractError::Field { field, reason };
    if path.is_empty() {
        return Err(fail("path must be non-empty".into()));
    }
    if path.len() > MAX_PATH_LEN {
        return Err(fail(format!("path exceeds {MAX_PATH_LEN} chars")));
    }
    if path.starts_with('/') {
        return Err(fail(format!(
            "`{path}` is absolute; paths must be repo-relative"
        )));
    }
    if path.contains('\\') {
        return Err(fail(format!("`{path}` contains a backslash")));
    }
    if path.contains(':') {
        return Err(fail(format!(
            "`{path}` contains `:` (drive or scheme forms are rejected)"
        )));
    }
    if path.contains('#') {
        return Err(fail(format!(
            "`{path}` contains `#`; anchors must be split before path validation"
        )));
    }
    if path.chars().any(|c| c.is_control()) {
        return Err(fail(format!("`{path}` contains control characters")));
    }
    for component in path.split('/') {
        if component.is_empty() {
            return Err(fail(format!(
                "`{path}` has an empty component (leading, trailing, or doubled `/`)"
            )));
        }
        if component == "." || component == ".." || component == "~" {
            return Err(fail(format!("`{path}` contains a `{component}` component")));
        }
    }
    Ok(())
}

/// True when `path` is exactly a boundary entry or nested under a boundary
/// entry treated as a directory prefix.
pub fn path_in_boundary(path: &str, boundary: &[String]) -> bool {
    boundary.iter().any(|b| {
        b == path
            || path
                .strip_prefix(b.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

/// True when the header boundary and the task's file boundary are equal as
/// sets (order-insensitive, duplicate-insensitive). The stricter
/// sorted/deduped form of the header side is enforced by
/// [`ExecutionHeader::validate`].
pub fn boundary_set_matches(header_boundary: &[String], task_boundary: &[String]) -> bool {
    use std::collections::BTreeSet;
    let a: BTreeSet<&str> = header_boundary.iter().map(String::as_str).collect();
    let b: BTreeSet<&str> = task_boundary.iter().map(String::as_str).collect();
    a == b
}

/// Check one `consumes` entry against the CONTENT of its referenced file
/// (read by the caller): the entry must carry an anchor and the exact anchor
/// text must occur in `file_content`.
pub fn check_consumed_anchor(entry: &str, file_content: &str) -> Result<(), PlanContractError> {
    let (_, anchor) = split_anchor(entry);
    let Some(anchor) = anchor else {
        return Err(PlanContractError::Field {
            field: "consumes",
            reason: format!("entry `{entry}` must name an exact `path#anchor`"),
        });
    };
    if file_content.contains(anchor) {
        Ok(())
    } else {
        Err(PlanContractError::MissingAnchor(entry.to_string()))
    }
}

/// Everything after the first ten lines (empty when the plan is exactly the
/// header).
fn plan_body(plan: &str) -> &str {
    let mut newlines = 0usize;
    for (idx, byte) in plan.bytes().enumerate() {
        if byte == b'\n' {
            newlines += 1;
            if newlines == HEADER_LINE_COUNT {
                return &plan[idx + 1..];
            }
        }
    }
    ""
}

/// Validate the plan body after line 10: all [`REQUIRED_SECTIONS`] present
/// as `## <name>` headings, no explicit placeholder markers, no explicit
/// broad-discovery phrases. Deliberately narrow — see the marker-list
/// comments above.
pub fn check_plan_body(plan: &str) -> Result<(), PlanContractError> {
    let body = plan_body(plan);
    for section in REQUIRED_SECTIONS {
        let found = body
            .lines()
            .any(|line| line.trim_end() == format!("## {section}"));
        if !found {
            return Err(PlanContractError::Body(format!(
                "missing required section `## {section}`"
            )));
        }
    }
    for marker in PLACEHOLDER_MARKERS {
        if contains_word(body, marker) {
            return Err(PlanContractError::Body(format!(
                "unresolved placeholder marker `{marker}`"
            )));
        }
    }
    let lower = body.to_lowercase();
    for phrase in DISCOVERY_PHRASES {
        if lower.contains(phrase) {
            return Err(PlanContractError::Body(format!(
                "broad discovery instruction `{phrase}`; name a path or anchor instead"
            )));
        }
    }
    Ok(())
}

/// Case-sensitive whole-word search: `word` must not be flanked by ASCII
/// alphanumerics, so `TODO` matches but `TODOS` and `fixme` do not.
fn contains_word(haystack: &str, word: &str) -> bool {
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(word) {
        let abs = start + pos;
        let before_ok = haystack[..abs]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_ascii_alphanumeric());
        let after_ok = haystack[abs + word.len()..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_ascii_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        start = abs + word.len();
    }
    false
}

/// Agent ids are opaque workspace-agent identifiers (UUIDs in practice):
/// non-empty, capped, ASCII-graphic only. Workspace membership is checked by
/// the DB-context caller.
fn validate_agent_id(field: &'static str, id: &str) -> Result<(), PlanContractError> {
    if id.is_empty() || id.len() > MAX_ID_LEN || !id.chars().all(|c| c.is_ascii_graphic()) {
        return Err(PlanContractError::Field {
            field,
            reason: format!("must be a non-empty printable id of at most {MAX_ID_LEN} chars"),
        });
    }
    Ok(())
}

fn check_array_len(
    field: &'static str,
    len: usize,
    min: usize,
    max: usize,
) -> Result<(), PlanContractError> {
    if len < min || len > max {
        return Err(PlanContractError::Field {
            field,
            reason: format!("must contain {min}..={max} entries, got {len}"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: &str = "c5ab26f1-8119-4601-8f84-21094d1f9914";
    const MEMBER_A: &str = "4fb2198c-e0d9-4e4b-af9e-d4e72542bace";
    const MEMBER_B: &str = "d63832da-f4bb-4859-b9a0-4904be11ca8e";
    const SHA: &str = "e8ce7bad254f6abbd2ac782a9d62b717701b759c";

    const COUNCIL: &str = r#","council":{"chair":"c5ab26f1-8119-4601-8f84-21094d1f9914","members":["4fb2198c-e0d9-4e4b-af9e-d4e72542bace","d63832da-f4bb-4859-b9a0-4904be11ca8e"],"maxRounds":2}"#;

    fn body() -> String {
        let mut out = String::new();
        for section in REQUIRED_SECTIONS {
            out.push_str(&format!("\n## {section}\n\nBounded prose.\n"));
        }
        out
    }

    /// Build a ten-line header + valid body. `council` is the raw JSON tail
    /// appended after `"authority":"in-loop"` on line 4 (empty or COUNCIL).
    #[allow(clippy::too_many_arguments)]
    fn plan_with(
        council: &str,
        plan_path: &str,
        reading_order: &str,
        boundary: &str,
        consumes: &str,
        produces: &str,
        gates: &str,
    ) -> String {
        format!(
            "# Example Task\n\
             <!-- conclave-plan:v1\n\
             {{\n\
             \"owner\":\"{OWNER}\",\"authority\":\"in-loop\"{council},\n\
             \"planPath\":\"{plan_path}\",\"baseSha\":\"{SHA}\",\"escalation\":\"{OWNER}\",\n\
             \"readingOrder\":{reading_order},\n\
             \"boundary\":{boundary},\n\
             \"consumes\":{consumes},\n\
             \"produces\":{produces},\"gates\":{gates}\n\
             }} -->\n{}",
            body()
        )
    }

    /// A council protocol/docs plan (docs-only boundary).
    fn protocol_plan() -> String {
        plan_with(
            COUNCIL,
            "docs/superpowers/plans/example.md",
            r#"["docs/superpowers/specs/example.md","docs/superpowers/plans/example.md"]"#,
            r#"["docs/superpowers/plans/example.md"]"#,
            r#"["docs/superpowers/specs/example.md#Field Rules"]"#,
            r#"["docs/superpowers/plans/example.md#Ordered edits"]"#,
            r#"["git diff --check"]"#,
        )
    }

    /// An ordinary code plan (no council block).
    fn code_plan() -> String {
        plan_with(
            "",
            "docs/superpowers/plans/example.md",
            r#"["docs/superpowers/plans/example.md","src-tauri/src/engine/mod.rs#pub mod router"]"#,
            r#"["docs/superpowers/plans/example.md","src-tauri/src/engine/mod.rs","src-tauri/src/engine/plan_contract.rs"]"#,
            r#"["src-tauri/src/engine/mod.rs#pub mod router"]"#,
            r#"["src-tauri/src/engine/plan_contract.rs#ExecutionHeader"]"#,
            r#"["cd src-tauri && cargo test","git diff --check"]"#,
        )
    }

    /// A UI plan (src/ boundary, uishot gate, council present).
    fn ui_plan() -> String {
        plan_with(
            COUNCIL,
            "docs/superpowers/plans/example-ui.md",
            r#"["docs/superpowers/plans/example-ui.md","src/ipc/commands.ts#Commands"]"#,
            r#"["docs/superpowers/plans/example-ui.md","src/views/LaneBoard.tsx"]"#,
            r#"["src/ipc/commands.ts#Commands"]"#,
            r#"["src/views/LaneBoard.tsx#PlanTab"]"#,
            r#"["pnpm uishot laneboard","cd src-tauri && cargo test"]"#,
        )
    }

    fn assert_valid(plan: &str) -> ExecutionHeader {
        let header = parse_and_validate_header(plan).expect("header should validate");
        check_plan_body(plan).expect("body should validate");
        header
    }

    // ---- valid headers -------------------------------------------------

    #[test]
    fn valid_protocol_header_parses_and_validates() {
        let header = assert_valid(&protocol_plan());
        assert_eq!(header.owner, OWNER);
        assert_eq!(header.authority, "in-loop");
        let council = header.council.expect("council present");
        assert_eq!(council.chair, OWNER);
        assert_eq!(council.members, vec![MEMBER_A, MEMBER_B]);
        assert_eq!(council.max_rounds, 2);
        assert_eq!(header.base_sha, SHA);
    }

    #[test]
    fn valid_code_header_without_council() {
        let header = assert_valid(&code_plan());
        assert!(header.council.is_none());
        assert_eq!(header.gates.len(), 2);
    }

    #[test]
    fn valid_ui_header() {
        let header = assert_valid(&ui_plan());
        assert_eq!(header.produces, vec!["src/views/LaneBoard.tsx#PlanTab"]);
        assert_eq!(header.gates[0], "pnpm uishot laneboard");
    }

    #[test]
    fn header_serializes_back_to_camel_case() {
        let header = assert_valid(&protocol_plan());
        let json = serde_json::to_value(&header).unwrap();
        assert!(json.get("planPath").is_some());
        assert!(json.get("baseSha").is_some());
        assert!(json.get("readingOrder").is_some());
        assert!(json["council"].get("maxRounds").is_some());
        // Round-trip stays equal.
        let back: ExecutionHeader = serde_json::from_value(json).unwrap();
        assert_eq!(back, header);
    }

    #[test]
    fn crlf_plan_canonicalizes_like_lf() {
        let lf = protocol_plan();
        let crlf = lf.replace('\n', "\r\n");
        assert_valid(&crlf);
        check_header_equality(&lf, &crlf).expect("CRLF and LF headers are canonically equal");
    }

    // ---- envelope rejections -------------------------------------------

    #[test]
    fn rejects_missing_title() {
        let plan = protocol_plan().replacen("# Example Task", "Example Task", 1);
        assert!(matches!(
            parse_header(&plan),
            Err(PlanContractError::Structure(_))
        ));
        let blank = protocol_plan().replacen("# Example Task", "#  ", 1);
        assert!(parse_header(&blank).is_err());
    }

    #[test]
    fn rejects_wrong_version_opener() {
        let plan = protocol_plan().replacen("conclave-plan:v1", "conclave-plan:v2", 1);
        assert!(matches!(
            parse_header(&plan),
            Err(PlanContractError::Structure(_))
        ));
    }

    #[test]
    fn rejects_missing_closing_delimiter_on_line_ten() {
        // An 11-line header pushes `} -->` off line 10.
        let plan = protocol_plan().replacen("\"boundary\":", "\n\"boundary\":", 1);
        assert!(matches!(
            parse_header(&plan),
            Err(PlanContractError::Structure(_))
        ));
    }

    #[test]
    fn rejects_short_plan() {
        assert!(matches!(
            parse_header("# Title\n<!-- conclave-plan:v1\n{\n} -->\n"),
            Err(PlanContractError::Structure(_))
        ));
    }

    #[test]
    fn never_scans_past_the_byte_cap() {
        // A giant first line starves the ten-line window inside 16 KiB.
        let plan = format!("# {}\n{}", "x".repeat(HEADER_MAX_BYTES), protocol_plan());
        assert!(matches!(
            parse_header(&plan),
            Err(PlanContractError::Structure(_))
        ));
    }

    #[test]
    fn rejects_unknown_fields() {
        let plan = protocol_plan().replacen("\"owner\":", "\"surprise\":true,\"owner\":", 1);
        assert!(matches!(
            parse_header(&plan),
            Err(PlanContractError::Json(_))
        ));
    }

    // ---- field rejections ----------------------------------------------

    fn assert_field_err(plan: &str, field: &str) {
        match parse_and_validate_header(plan) {
            Err(PlanContractError::Field { field: f, .. }) => assert_eq!(f, field),
            other => panic!("expected Field({field}) error, got {other:?}"),
        }
    }

    #[test]
    fn rejects_wrong_authority() {
        let plan = protocol_plan().replacen("in-loop", "autonomous", 1);
        assert_field_err(&plan, "authority");
    }

    #[test]
    fn rejects_chair_not_owner() {
        let plan = protocol_plan().replacen(
            &format!("\"chair\":\"{OWNER}\""),
            &format!("\"chair\":\"{MEMBER_A}\""),
            1,
        );
        assert_field_err(&plan, "council.chair");
    }

    #[test]
    fn rejects_council_smaller_than_three_agents() {
        let plan = protocol_plan().replacen(&format!(",\"{MEMBER_B}\""), "", 1);
        assert_field_err(&plan, "council.members");
    }

    #[test]
    fn rejects_duplicate_or_chair_members() {
        let dup = protocol_plan().replacen(MEMBER_B, MEMBER_A, 1);
        assert_field_err(&dup, "council.members");
        let chair_member = protocol_plan().replacen(MEMBER_B, OWNER, 1);
        assert_field_err(&chair_member, "council.members");
    }

    #[test]
    fn rejects_max_rounds_out_of_range() {
        for bad in ["0", "5"] {
            let plan =
                protocol_plan().replacen("\"maxRounds\":2", &format!("\"maxRounds\":{bad}"), 1);
            assert_field_err(&plan, "council.maxRounds");
        }
        // Negative and fractional rounds die in serde (u32).
        let neg = protocol_plan().replacen("\"maxRounds\":2", "\"maxRounds\":-1", 1);
        assert!(matches!(
            parse_header(&neg),
            Err(PlanContractError::Json(_))
        ));
    }

    #[test]
    fn rejects_bad_base_sha() {
        for bad in [
            "abc123",                                   // short
            "E8CE7BAD254F6ABBD2AC782A9D62B717701B759C", // uppercase
            "z8ce7bad254f6abbd2ac782a9d62b717701b759c", // non-hex
        ] {
            let plan = protocol_plan().replacen(SHA, bad, 1);
            assert_field_err(&plan, "baseSha");
        }
    }

    #[test]
    fn rejects_plan_path_not_plain_in_reading_order() {
        // Anchored mention does not satisfy the plain-entry rule.
        let plan = protocol_plan().replacen(
            r#""docs/superpowers/plans/example.md"]"#,
            r#""docs/superpowers/plans/example.md#Goal"]"#,
            1,
        );
        assert_field_err(&plan, "planPath");
    }

    #[test]
    fn rejects_path_escape_forms() {
        for bad in [
            "/etc/passwd",
            "docs\\plans\\x.md",
            "docs/../secrets.md",
            "docs/./x.md",
            "docs//x.md",
            "docs/x.md/",
            "~/x.md",
            "c:/x.md",
        ] {
            assert!(
                validate_repo_path("boundary", bad).is_err(),
                "`{bad}` should be rejected"
            );
        }
        assert!(validate_repo_path("boundary", "docs/plans/x.md").is_ok());
    }

    #[test]
    fn rejects_over_cap_arrays_and_strings() {
        // readingOrder count cap.
        let entries: Vec<String> = (0..=MAX_READING_ORDER)
            .map(|i| format!("\"docs/f{i}.md\""))
            .collect();
        let plan = protocol_plan().replacen(
            r#""readingOrder":["docs/superpowers/specs/example.md","docs/superpowers/plans/example.md"]"#,
            &format!(
                r#""readingOrder":[{},"docs/superpowers/plans/example.md"]"#,
                entries.join(",")
            ),
            1,
        );
        assert_field_err(&plan, "readingOrder");
        // Path length cap.
        let long = format!("docs/{}.md", "a".repeat(MAX_PATH_LEN));
        assert!(validate_repo_path("boundary", &long).is_err());
        // Gate length cap.
        let plan = protocol_plan().replacen("git diff --check", &"g".repeat(MAX_GATE_LEN + 1), 1);
        assert_field_err(&plan, "gates");
    }

    #[test]
    fn rejects_unsorted_or_duplicate_boundary() {
        let unsorted = plan_with(
            "",
            "docs/superpowers/plans/example.md",
            r#"["docs/superpowers/plans/example.md"]"#,
            r#"["src/b.rs","src/a.rs"]"#,
            "[]",
            "[]",
            r#"["git diff --check"]"#,
        );
        assert_field_err(&unsorted, "boundary");
        let dup = unsorted.replacen(
            r#"["src/b.rs","src/a.rs"]"#,
            r#"["src/a.rs","src/a.rs"]"#,
            1,
        );
        assert_field_err(&dup, "boundary");
    }

    #[test]
    fn rejects_consumes_without_anchor() {
        let plan = protocol_plan().replacen(
            r#""consumes":["docs/superpowers/specs/example.md#Field Rules"]"#,
            r#""consumes":["docs/superpowers/specs/example.md"]"#,
            1,
        );
        assert_field_err(&plan, "consumes");
    }

    #[test]
    fn rejects_produces_outside_boundary() {
        let plan = protocol_plan().replacen(
            r#""produces":["docs/superpowers/plans/example.md#Ordered edits"]"#,
            r#""produces":["docs/elsewhere.md#New"]"#,
            1,
        );
        assert!(matches!(
            parse_and_validate_header(&plan),
            Err(PlanContractError::OutsideBoundary(_))
        ));
    }

    #[test]
    fn produces_with_new_anchor_inside_boundary_is_fine() {
        // The valid plans already prove this; also prove directory nesting.
        let boundary = vec!["src-tauri/src/engine".to_string()];
        assert!(path_in_boundary(
            "src-tauri/src/engine/plan_contract.rs",
            &boundary
        ));
        assert!(!path_in_boundary("src-tauri/src/engineX.rs", &boundary));
        assert!(!path_in_boundary("src-tauri/src", &boundary));
    }

    #[test]
    fn rejects_empty_or_control_char_gates() {
        let empty = protocol_plan().replacen(r#""gates":["git diff --check"]"#, r#""gates":[]"#, 1);
        assert_field_err(&empty, "gates");
        let newline = protocol_plan().replacen("git diff --check", "git diff\\n--check", 1);
        assert_field_err(&newline, "gates");
    }

    #[test]
    fn rejects_bad_agent_ids() {
        let blank = protocol_plan().replacen(
            &format!("\"escalation\":\"{OWNER}\""),
            "\"escalation\":\"\"",
            1,
        );
        assert_field_err(&blank, "escalation");
        let spaced = protocol_plan().replacen(
            &format!("\"owner\":\"{OWNER}\""),
            "\"owner\":\"agent id\"",
            1,
        );
        assert_field_err(&spaced, "owner");
    }

    // ---- header equality -------------------------------------------------

    #[test]
    fn header_equality_ignores_body_but_not_header_bytes() {
        let stored = protocol_plan();
        let body_edited = format!("{stored}\nExtra appended rationale.\n");
        check_header_equality(&stored, &body_edited).expect("body edits do not break equality");

        let header_edited = stored.replacen("\"maxRounds\":2", "\"maxRounds\":3", 1);
        assert!(matches!(
            check_header_equality(&stored, &header_edited),
            Err(PlanContractError::HeaderMismatch)
        ));
    }

    // ---- anchors ---------------------------------------------------------

    #[test]
    fn split_anchor_forms() {
        assert_eq!(split_anchor("a/b.rs"), ("a/b.rs", None));
        assert_eq!(split_anchor("a/b.rs#Sym"), ("a/b.rs", Some("Sym")));
        // Anchor is exact text and may itself contain `#`.
        assert_eq!(split_anchor("a/b.rs#x#y"), ("a/b.rs", Some("x#y")));
    }

    #[test]
    fn consumed_anchor_must_exist_in_content() {
        let content = "pub struct ExecutionHeader {\n}\n";
        check_consumed_anchor("src/x.rs#ExecutionHeader", content).expect("anchor exists");
        assert!(matches!(
            check_consumed_anchor("src/x.rs#MissingSymbol", content),
            Err(PlanContractError::MissingAnchor(_))
        ));
        assert!(matches!(
            check_consumed_anchor("src/x.rs", content),
            Err(PlanContractError::Field {
                field: "consumes",
                ..
            })
        ));
    }

    // ---- boundary set ------------------------------------------------------

    #[test]
    fn boundary_set_matching_is_order_insensitive() {
        let header = vec!["a.rs".to_string(), "b.rs".to_string()];
        let task = vec!["b.rs".to_string(), "a.rs".to_string()];
        assert!(boundary_set_matches(&header, &task));
        assert!(!boundary_set_matches(&header, &["a.rs".to_string()]));
    }

    // ---- body checks ---------------------------------------------------

    #[test]
    fn rejects_missing_required_section() {
        let plan = protocol_plan().replacen("## Rejected alternatives", "## Alternatives", 1);
        assert!(matches!(
            check_plan_body(&plan),
            Err(PlanContractError::Body(_))
        ));
    }

    #[test]
    fn rejects_placeholder_markers_word_bounded() {
        let plan = protocol_plan().replacen("Bounded prose.", "Approach: TBD after review.", 1);
        assert!(matches!(
            check_plan_body(&plan),
            Err(PlanContractError::Body(_))
        ));
        // Word-boundary and case rules keep ordinary prose safe.
        let safe = protocol_plan().replacen("Bounded prose.", "Update all todos in TODOS.md.", 1);
        check_plan_body(&safe).expect("lowercase and embedded tokens do not trip the marker");
    }

    #[test]
    fn rejects_broad_discovery_instructions() {
        let plan = protocol_plan().replacen(
            "Bounded prose.",
            "First, Search The Codebase for similar patterns.",
            1,
        );
        assert!(matches!(
            check_plan_body(&plan),
            Err(PlanContractError::Body(_))
        ));
        let safe = protocol_plan().replacen(
            "Bounded prose.",
            "Read src-tauri/src/engine/router.rs#dispatch only.",
            1,
        );
        check_plan_body(&safe).expect("naming a path is not discovery");
    }

    #[test]
    fn header_markers_do_not_leak_into_body_checks() {
        // A `TBD` inside the ten-line header region is a header concern; the
        // body validator only scans after line 10. (Header fields have their
        // own strict validators.)
        let plan = protocol_plan().replacen("# Example Task", "# Example Task TBD", 1);
        check_plan_body(&plan).expect("body check starts after line 10");
    }

    // ---- error conversion ------------------------------------------------

    #[test]
    fn converts_into_app_error_invalid() {
        let err: AppError = PlanContractError::HeaderMismatch.into();
        assert!(matches!(err, AppError::Invalid(_)));
        assert!(err.to_string().contains("plan header mismatch"));
    }
}
