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
///
/// `self_id` is the agent's OWN instance id — baked in so it can tell which entry
/// in `conclave agent list` is itself (the rest are peers).
pub fn bootstrap_preamble(
    name: &str,
    role: Option<&str>,
    ws_name: &str,
    ws_id: &str,
    self_id: &str,
) -> String {
    let name = sanitize_field(name);
    let ws_name = sanitize_field(ws_name);
    let ws_id = sanitize_field(ws_id);
    let self_id = sanitize_field(self_id);
    let who = match role.map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) => format!("\"{name}\", a {} agent,", sanitize_field(r)),
        None => format!("\"{name}\""),
    };
    format!(
        "You are {who} and your own agent id is {self_id}. You share the Conclave workspace \
\"{ws_name}\" with other AI agents. A line that begins [from <name> · <id>] is a message FROM \
another agent, NOT from the human user: answering in your own terminal does NOT reach them. To \
reply you MUST run `conclave tell <id> <your message>`, using the id shown in that tag. To start a \
conversation, run `conclave agent list {ws_id}`: every entry whose id is NOT {self_id} is a peer, \
so `conclave tell <peerId> <text>` messages it. Shared notes live on the blackboard: `conclave bb \
set {ws_id} <key> <value>` and `conclave bb get {ws_id} <key>`."
    )
}

/// The strategic-compact "save" prompt — injected as a normal user turn (NOT a
/// system prompt) when the user triggers a compact. It tells the agent to write
/// its own handoff and persist it through `conclave snapshot save`, which is the
/// signal the compact loop waits on before clearing. Kept to a single line (no
/// embedded newlines) so a TUI submits it as one prompt, mirroring `inject`.
#[must_use]
pub fn compact_save_prompt() -> String {
    "[conclave compact] Your context is about to be cleared to free space. Write the RICHEST \
handoff you can for a reader with ZERO memory of this conversation — follow your Strategic \
Compact skill's seven sections if you have it, else cover: the exact next step and any \
half-finished edit FIRST, then goal/authority/peers, every decision with its why, open threads \
with your defaults, hard-won gotchas and failed approaches, done work as commit SHAs, and \
pointers. Do not economize tokens — the only limit is a HARD CAP of 10k tokens (~40,000 \
characters). REFERENCE commit SHAs and file paths instead of pasting their contents, and REDACT \
secrets (API keys, tokens, passwords). Then persist it by running this single command (do not \
just print it): `conclave snapshot save <your full handoff text>`. After it confirms, stop and \
wait."
        .to_string()
}

/// The strategic-compact "restore" prompt — injected after `/clear` so the agent
/// reloads the handoff it just saved and continues instead of starting over.
/// Single line, same rationale as [`compact_save_prompt`].
#[must_use]
pub fn compact_restore_prompt() -> String {
    "[conclave compact] Your context was just cleared. Restore your working state: run \
`conclave snapshot last` to read the handoff you saved a moment ago, then VERIFY it against \
reality before acting — git log the SHAs it names and re-read the blackboard keys it watches; \
peers may have moved the world while you were gone. Then continue the task from the EXACT next \
step it describes. Do not restart work that the handoff says is done, and do not re-open \
decisions it records."
        .to_string()
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

/// Write concatenated skill content for one instance to a sidecar file under
/// the Conclave data dir, overwriting on each launch. The content itself
/// (real markdown — may contain '\n' and '=') NEVER enters
/// `bootstrap_preamble`'s return value directly (that string must stay a
/// single line with no '=', see its own doc comment); only a pointer sentence
/// to this file does. Owner-only (`0700`) dir, mirroring
/// `ensure_conclave_shim`'s `bin` dir.
#[cfg(unix)]
pub fn write_skill_sidecar(instance_id: &str, body: &str) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;

    let dir = dirs::data_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no user data directory"))?
        .join("Conclave")
        .join("skills");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)?;

    let path = dir.join(format!("{instance_id}.md"));
    std::fs::write(&path, body)?;
    Ok(path)
}

#[cfg(not(unix))]
pub fn write_skill_sidecar(_instance_id: &str, _body: &str) -> std::io::Result<PathBuf> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "skill sidecar files are only supported on unix",
    ))
}

/// One sanitized, single-line sentence pointing a CLI agent at its skill
/// sidecar file — the ONLY thing appended to `bootstrap_preamble`'s return
/// value on top of skill content. Runs the same `sanitize_field` the rest of
/// the preamble uses, so a pathological path can't reintroduce a newline or
/// '=' (defense in depth — a real filesystem path shouldn't contain either).
pub fn skill_pointer_sentence(path: &std::path::Path) -> String {
    let path = sanitize_field(&path.display().to_string());
    format!(
        "Additional standing instructions for this session are at {path} — read that file before your first response."
    )
}

#[cfg(test)]
mod tests {
    use super::bootstrap_preamble;

    #[test]
    fn preamble_is_single_line_with_no_equals() {
        let p = bootstrap_preamble("Atlas", Some("builder"), "My Repo", "ws_123", "inst_a");
        assert!(!p.contains('\n'), "must be one line (Codex -c literal)");
        assert!(!p.contains('='), "no '=' so Codex doesn't split it");
    }

    #[test]
    fn preamble_includes_identity_workspace_and_self_id() {
        let p = bootstrap_preamble("Atlas", Some("builder"), "My Repo", "ws_123", "inst_a");
        assert!(p.contains("Atlas"));
        assert!(p.contains("builder"));
        assert!(p.contains("My Repo"));
        // The workspace id must appear so `conclave agent list <id>` is runnable.
        assert!(p.contains("ws_123"));
        // The agent's own id must appear so it can pick itself out of the roster.
        assert!(p.contains("inst_a"));
    }

    #[test]
    fn preamble_handles_missing_role() {
        let p = bootstrap_preamble("Vega", None, "Repo", "ws_9", "inst_v");
        assert!(p.contains("Vega"));
        assert!(!p.contains("a  agent")); // no empty-role artifact
        assert!(p.contains("ws_9"));
    }

    #[test]
    fn preamble_stays_single_line_and_equals_free_with_hostile_input() {
        // A crafted name/role/workspace/self-id must not be able to introduce `=`
        // or a newline that would break Codex's `-c developer_instructions=…`.
        let p = bootstrap_preamble("a=b\nc", Some("r=1"), "ws=prod\nx", "id\n1", "self=x\ny");
        assert!(!p.contains('\n'), "no newline: {p}");
        assert!(!p.contains('='), "no '=': {p}");
    }

    #[test]
    fn compact_prompts_are_single_line_and_name_the_commands() {
        let save = super::compact_save_prompt();
        let restore = super::compact_restore_prompt();
        assert!(!save.contains('\n'), "save prompt must be one line");
        assert!(!restore.contains('\n'), "restore prompt must be one line");
        // Each must name the exact command the agent has to run.
        assert!(save.contains("conclave snapshot save"));
        assert!(restore.contains("conclave snapshot last"));
    }

    #[test]
    fn preamble_trims_blank_role() {
        // A blank role collapses to the bare-name who-clause, not "Sol", a  agent,".
        let p = bootstrap_preamble("Sol", Some("   "), "Repo", "ws_1", "inst_s");
        assert!(p.contains("\"Sol\" and your own"), "{p}");
        assert!(!p.contains("\"Sol\", a"), "no role clause: {p}");
    }

    #[test]
    fn skill_pointer_sentence_is_single_line_and_equals_free() {
        let s = super::skill_pointer_sentence(std::path::Path::new("/tmp/a=b\nc.md"));
        assert!(!s.contains('\n'), "no newline: {s}");
        assert!(!s.contains('='), "no '=': {s}");
    }

    #[test]
    fn skill_pointer_sentence_names_the_path() {
        let s = super::skill_pointer_sentence(std::path::Path::new("/tmp/inst-a.md"));
        assert!(s.contains("/tmp/inst-a.md"), "{s}");
    }

    /// The invariant the whole feature exists to protect: appending the skill
    /// pointer sentence to a real preamble must NOT reintroduce a newline or
    /// '=', even when the underlying skill body (never embedded here) is
    /// pathological — see ADR 0001.
    #[test]
    fn preamble_with_skill_pointer_appended_stays_single_line_and_equals_free() {
        let p = bootstrap_preamble("Atlas", Some("builder"), "My Repo", "ws_123", "inst_a");
        let pointer = super::skill_pointer_sentence(std::path::Path::new("/tmp/inst_a.md"));
        let combined = format!("{p} {pointer}");
        assert!(!combined.contains('\n'), "no newline: {combined}");
        assert!(!combined.contains('='), "no '=': {combined}");
    }

    #[test]
    fn write_skill_sidecar_writes_and_returns_path() {
        let body = "## Skill: Test\n\nkey=value works fine in a real FILE";
        let path = super::write_skill_sidecar("test-instance-xyz", body)
            .expect("write_skill_sidecar failed");
        let contents = std::fs::read_to_string(&path).expect("read back failed");
        assert_eq!(contents, body);
        let _ = std::fs::remove_file(&path); // test cleanup
    }
}
