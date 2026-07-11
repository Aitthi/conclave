//! Pure, deterministic checkpoint projection for the Conclave ctx-proxy.
//! Milestone-1: LOG MODE ONLY — it measures what a checkpoint *would* do and
//! never alters forwarded bytes. serde_json only (crate purity).

/// Read-only, re-runnable-for-current-state tools whose historical output may be
/// stubbed and re-obtained on demand. Everything else (side-effecting, drifting,
/// mutating) and every unknown name is kept verbatim (fail-safe).
pub fn is_recoverable(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "Read" | "Grep" | "Glob" | "LS" | "WebSearch" | "NotebookRead"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_family_is_recoverable() {
        for t in ["Read", "Grep", "Glob", "LS", "WebSearch", "NotebookRead"] {
            assert!(is_recoverable(t), "{t} should be recoverable");
        }
    }

    #[test]
    fn side_effecting_and_drifting_tools_are_not_recoverable() {
        for t in ["Bash", "WebFetch", "Write", "Edit", "MultiEdit", "NotebookEdit", "Task"] {
            assert!(!is_recoverable(t), "{t} must not be recoverable");
        }
    }

    #[test]
    fn unknown_tool_defaults_to_not_recoverable() {
        assert!(!is_recoverable("Conclave"));
        assert!(!is_recoverable("mcp__whatever__do"));
        assert!(!is_recoverable(""));
    }
}
