//! Agent awareness bootstrap — how a freshly-spawned CLI agent learns who it is,
//! who its peers are, and how to reach them.
//!
//! Two pieces:
//! 1. [`bootstrap_preamble`] — a one-line briefing injected via the harness's
//!    *system-prompt* layer (`claude --append-system-prompt`, `codex -c
//!    developer_instructions=…`), NOT as a chat turn. That layer is reconstructed
//!    every turn from the launch args, so it survives `/clear` (which only wipes
//!    conversation history) — the agent never forgets its identity.
//! 2. [`ensure_conclave_shim`] — puts a `conclave` executable on a directory we
//!    can prepend to the agent's PATH, so the briefing's `conclave …` commands
//!    actually resolve.
//!
//! The preamble is deliberately tiny and STATIC: it bakes only the identity +
//! the workspace id, then points the agent at `conclave agent list` for the LIVE
//! roster. That way a peer added/removed after spawn doesn't make the briefing
//! stale — the dynamic part is always queried fresh.

use std::path::PathBuf;

/// Strip the two characters that would break the briefing's wire form out of a
/// user-controlled field: `=` (Codex splits `-c key=value` on the first one) and
/// newlines (the value must stay a single line). Collapses each to a space.
fn sanitize_field(s: &str) -> String {
    s.replace(['=', '\n', '\r'], " ")
}

/// One-line awareness briefing for a CLI agent. Kept to a single line with no
/// `=` so it survives Codex's `-c key=value` parsing as a literal string. Every
/// interpolated user field (name, role, workspace name/id) is sanitized so a
/// crafted name like `"x=y"` or one with a newline can't break that invariant.
pub fn bootstrap_preamble(name: &str, role: Option<&str>, ws_name: &str, ws_id: &str) -> String {
    let name = sanitize_field(name);
    let ws_name = sanitize_field(ws_name);
    let ws_id = sanitize_field(ws_id);
    let who = match role.map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) => format!("\"{name}\", a {} agent,", sanitize_field(r)),
        None => format!("\"{name}\""),
    };
    format!(
        "You are {who} in the Conclave workspace \"{ws_name}\". Other AI agents share this \
workspace with you. Run `conclave agent list {ws_id}` to see them (each entry has an id). \
Use `conclave tell <id> <text>` to message an agent — they receive it tagged with your name. \
Use `conclave bb set {ws_id} <key> <value>` / `conclave bb get {ws_id} <key>` for the shared \
blackboard. Messages from other agents arrive in your input prefixed with [from <name>]."
    )
}

/// Ensure a `conclave` command exists on a dedicated shim directory and return
/// that directory (to be prepended to the spawned agent's PATH). The shim is a
/// symlink to the sibling `conclave-cli` binary, refreshed each call so it always
/// points at the current build.
///
/// Returns `None` when the `conclave-cli` binary can't be found next to the app
/// (e.g. a dev run that only built the app bin) — the caller then simply skips
/// PATH injection and the agent launches without `conclave` available, rather
/// than failing the spawn.
#[cfg(unix)]
pub fn ensure_conclave_shim() -> Option<PathBuf> {
    use std::os::unix::fs::{symlink, DirBuilderExt};

    let exe = std::env::current_exe().ok()?;
    // `conclave-cli` (src/bin/conclave-cli.rs) is built as a sibling of the app
    // binary and ships beside it in the bundle's MacOS dir.
    let cli = exe.parent()?.join("conclave-cli");
    if !cli.exists() {
        return None;
    }

    // Owner-only shim dir under the Conclave data dir. Force 0700 explicitly
    // rather than rely on the parent's mode or the process umask.
    let bin = dirs::data_dir()?.join("Conclave").join("bin");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&bin)
        .ok()?;

    let link = bin.join("conclave");
    // Refresh: a stale symlink (old build path) is replaced atomically enough for
    // our purposes — remove then recreate.
    let _ = std::fs::remove_file(&link);
    symlink(&cli, &link).ok()?;
    Some(bin)
}

#[cfg(not(unix))]
pub fn ensure_conclave_shim() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::bootstrap_preamble;

    #[test]
    fn preamble_is_single_line_with_no_equals() {
        let p = bootstrap_preamble("Atlas", Some("builder"), "My Repo", "ws_123");
        assert!(!p.contains('\n'), "must be one line (Codex -c literal)");
        assert!(!p.contains('='), "no '=' so Codex doesn't split it");
    }

    #[test]
    fn preamble_includes_identity_and_workspace_id() {
        let p = bootstrap_preamble("Atlas", Some("builder"), "My Repo", "ws_123");
        assert!(p.contains("Atlas"));
        assert!(p.contains("builder"));
        assert!(p.contains("My Repo"));
        // The workspace id must appear so `conclave agent list <id>` is runnable.
        assert!(p.contains("ws_123"));
    }

    #[test]
    fn preamble_handles_missing_role() {
        let p = bootstrap_preamble("Vega", None, "Repo", "ws_9");
        assert!(p.contains("Vega"));
        assert!(!p.contains("a  agent")); // no empty-role artifact
        assert!(p.contains("ws_9"));
    }

    #[test]
    fn preamble_stays_single_line_and_equals_free_with_hostile_input() {
        // A crafted name/role/workspace must not be able to introduce `=` or a
        // newline that would break Codex's `-c developer_instructions=…` parsing.
        let p = bootstrap_preamble("a=b\nc", Some("r=1"), "ws=prod\nx", "id\n1");
        assert!(!p.contains('\n'), "no newline: {p}");
        assert!(!p.contains('='), "no '=': {p}");
    }

    #[test]
    fn preamble_trims_blank_role() {
        let p = bootstrap_preamble("Sol", Some("   "), "Repo", "ws_1");
        assert!(
            !p.contains("agent,"),
            "blank role collapses to bare name form"
        );
    }
}
