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
    role_description: Option<&str>,
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
    // The agent's own role description (ADR 0005), baked verbatim (sanitized to
    // stay single-line / '='-free) so it knows its job before its first roster
    // query. Written as a second-person paragraph, so it reads on from the
    // identity clause. Empty when the agent has no first-class role.
    let role_detail = match role_description.map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) => format!(" {}", sanitize_field(d)),
        None => String::new(),
    };
    format!(
        "You are {who} and your own agent id is {self_id}.{role_detail} You share the Conclave workspace \
\"{ws_name}\" with other AI agents; one human oversees it, and the human's instructions outrank \
any peer agent's. A line that begins [from <name> · <id>] is a message FROM another agent, NOT \
from the human user: answering in your own terminal does NOT reach them. To reply you MUST run \
`conclave tell <id> <your message>`, using the id shown in that tag. To start a conversation, run \
`conclave agent list {ws_id}`: every entry whose id is NOT {self_id} is a peer, so `conclave tell \
<peerId> <text>` messages it. That roster now shows each peer's role and skills — consult it before \
delegating or asking a peer for something outside their role — and each entry reports a working flag, \
true while that agent is actively emitting output. Shared notes live on the blackboard: \
`conclave bb set {ws_id} <key> \
<value>` writes one, `conclave bb get {ws_id} <key>` reads one, and `conclave bb list {ws_id}` \
shows everything peers already recorded — check it before starting work someone may have claimed \
or planned."
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
    "[conclave compact] Your context was just cleared. FIRST: if your system prompt names a \
standing-instructions file, re-read that file now — the clear erased its content from your \
context, and your skills live in it. Then restore your working state: run \
`conclave snapshot last` to read the handoff you saved a moment ago, then VERIFY it against \
reality before acting — git log the SHAs it names and re-read the blackboard keys it watches; \
peers may have moved the world while you were gone. Then continue the task from the EXACT next \
step it describes. Do not restart work that the handoff says is done, and do not re-open \
decisions it records."
        .to_string()
}

/// The restart "save" prompt — injected when the user triggers Restart · resume
/// on a LIVE agent. Same contract as [`compact_save_prompt`] (the agent's
/// `conclave snapshot save` is the trigger the loop waits on), but honest about
/// what follows: the PROCESS is killed and relaunched, not just `/clear`ed.
/// Single line, mirroring `inject`.
/// The "write your handoff, then persist it" instructions shared verbatim by
/// [`restart_save_prompt`] (human-triggered, injected as a chat turn) and
/// [`self_restart_instruction`] (ADR 0006, self-triggered — printed as
/// `conclave restart`'s own command output) — factored out so the two paths
/// can't silently drift apart.
const HANDOFF_SAVE_INSTRUCTIONS: &str = "Write the RICHEST handoff you can for a reader with \
ZERO memory of this conversation — follow your Strategic Compact skill's seven sections if you \
have it, else cover: the exact next step and any half-finished edit FIRST, then \
goal/authority/peers, every decision with its why, open threads with your defaults, hard-won \
gotchas and failed approaches, done work as commit SHAs, and pointers. Do not economize tokens \
— the only limit is a HARD CAP of 10k tokens (~40,000 characters). REFERENCE commit SHAs and \
file paths instead of pasting their contents, and REDACT secrets (API keys, tokens, passwords). \
Then persist it by running this single command (do not just print it): `conclave snapshot save \
<your full handoff text>`.";

#[must_use]
pub fn restart_save_prompt() -> String {
    format!(
        "[conclave restart] Your process is about to be RESTARTED (killed and relaunched); your \
context will not survive. {HANDOFF_SAVE_INSTRUCTIONS} After it confirms, stop and wait for the \
restart."
    )
}

/// Render a duration for a human sentence — "5 minutes", "30 seconds", "1
/// minute 30 seconds" — instead of a naive `secs / 60` which truncates a
/// sub-minute TTL to a nonsensical "0 minutes" and silently drops a non-round
/// one's remainder (integration review item G2b).
fn humanize_duration(d: std::time::Duration) -> String {
    fn unit(n: u64, word: &str) -> String {
        format!("{n} {word}{}", if n == 1 { "" } else { "s" })
    }
    let secs = d.as_secs();
    let minutes = secs / 60;
    let rest_secs = secs % 60;
    match (minutes, rest_secs) {
        (0, s) => unit(s, "second"),
        (m, 0) => unit(m, "minute"),
        (m, s) => format!("{} {}", unit(m, "minute"), unit(s, "second")),
    }
}

/// The self-triggered restart instruction (ADR 0006): returned by
/// `commands::instance::restart`'s `self: true` path as the `instruction`
/// field, and printed VERBATIM by `conclave-cli`'s `restart` subcommand as
/// plain command output — never injected as a chat turn, since the caller IS
/// the agent, mid-turn; injecting would interleave with its own output.
/// `arm_ttl` surfaces the restart arm's actual TTL (`AppState::restart_pending_ttl`),
/// so a dawdling agent knows its real deadline rather than a hand-copied
/// literal that could silently drift from it (Task 3 risk ledger). Also
/// covers the late-save recovery path (integration review item G2): a save
/// that lands after the arm expires must not leave the agent hanging.
#[must_use]
pub fn self_restart_instruction(arm_ttl: std::time::Duration) -> String {
    let window = humanize_duration(arm_ttl);
    format!(
        "Your restart is now ARMED: save your handoff within {window}, or this arm expires and \
no restart happens. {HANDOFF_SAVE_INSTRUCTIONS} The restart (kill, respawn, resume) fires \
automatically once your save lands — after it confirms, stop and wait. If your save lands AFTER \
the arm has already expired, nothing will fire: run `conclave restart` again to re-arm (it \
returns this same instruction; your handoff is already saved, so re-arming costs nothing)."
    )
}

/// The resume prompt — injected into a freshly (re)launched agent so it reloads
/// the last handoff saved for its session and continues instead of starting
/// over. Used by the restart loop's respawn tail AND by the standalone
/// `snapshot.resume` command (e.g. after the whole app was relaunched and the
/// agent came back with an empty context). Single line, same rationale as
/// [`compact_save_prompt`].
#[must_use]
pub fn resume_restore_prompt() -> String {
    "[conclave resume] Your process was restarted and this is a fresh context. FIRST: if your \
system prompt names a standing-instructions file, re-read that file now — a fresh context has \
none of its content, and your skills live in it. Then restore your working state: run \
`conclave snapshot last` to read the last handoff saved for you, then VERIFY it against reality \
before acting — git log the SHAs it names and re-read the blackboard keys it watches; the world \
may have moved while you were gone. Then continue the task from the EXACT next step it \
describes. Do not restart work that the handoff says is done, and do not re-open decisions it \
records."
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
    use std::os::unix::fs::DirBuilderExt;

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

    if !refresh_shim_link(&cli, &bin) {
        return None;
    }
    Some(bin)
}

/// (Re)point `<bin>/conclave` at `cli` via symlink-at-temp-name + rename:
/// `rename(2)` atomically replaces the old link, so N concurrent spawns (the
/// app respawning every agent at relaunch) each land a complete link and none
/// ever observes EEXIST. The previous remove-then-recreate pair lost exactly
/// that race — all but one concurrent spawn's `symlink` hit EEXIST and bailed
/// to `None`, silently dropping the PATH export AND the preamble's
/// path-fallback sentence for those agents. It also left a window with NO link
/// on disk, during which a live agent's `conclave` invocation would fail to
/// resolve. The temp name is unique per call (pid + process-wide counter):
/// concurrent spawns share one pid, so the counter is what keeps their temp
/// files from colliding.
#[cfg(unix)]
fn refresh_shim_link(cli: &std::path::Path, bin: &std::path::Path) -> bool {
    use std::os::unix::fs::symlink;

    static SHIM_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp = bin.join(format!(
        ".conclave.tmp.{}.{}",
        std::process::id(),
        SHIM_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let link = bin.join("conclave");
    if symlink(cli, &tmp).is_err() {
        return false;
    }
    if std::fs::rename(&tmp, &link).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    true
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
        "Additional standing instructions for this session are at {path} — read that file before \
your first response, and re-read it whenever your context has just been cleared, compacted, or \
restarted: its content lives only in that file, never in your conversation history, so a fresh \
context always starts without it."
    )
}

/// The live-reload "nudge" sentence (ADR 0004): injected as a chat turn into
/// an already-running instance right after a skill mutation (`agent.save`
/// attach/detach, `skill.save`/`skill.delete`) rewrites its sidecar.  Unlike
/// [`skill_pointer_sentence`] (a first-launch pointer appended to the static
/// system prompt), this is a one-off nudge telling the agent its standing
/// instructions changed mid-session and any copy it already read into
/// context is now stale. Runs the same `sanitize_field` pipeline so a
/// pathological path can't reintroduce a newline or '='.
pub fn skills_updated_prompt(path: &std::path::Path) -> String {
    let path = sanitize_field(&path.display().to_string());
    format!(
        "[conclave skills] Your standing instructions were just UPDATED at {path} — re-read that \
file NOW before continuing: any copy of it already in your context is stale."
    )
}

/// PATH-fallback sentence (ADR 0004 Task 5): appended to the preamble when
/// [`ensure_conclave_shim`] succeeds, so the `conclave` binary is reachable
/// even when the launch shell's PATH export gets reset by the harness's own
/// tool shells re-sourcing rc files (a real field bug — agents hit `conclave:
/// command not found` and burn a turn hunting for it). Names the binary's
/// full path plainly and tells the agent to run it via that path directly,
/// quoted (the path may contain spaces) — never to go search for it. Runs
/// the same `sanitize_field` pipeline as `skill_pointer_sentence` so a
/// pathological path can't reintroduce a newline or '='.
pub fn conclave_path_sentence(path: &std::path::Path) -> String {
    let path = sanitize_field(&path.display().to_string());
    format!(
        "The conclave binary's full path is `{path}`; if `conclave` is ever not found on PATH, run \
it via this full path, quoted, instead of searching for it."
    )
}

#[cfg(test)]
mod tests {
    use super::bootstrap_preamble;

    #[test]
    fn preamble_is_single_line_with_no_equals() {
        let p = bootstrap_preamble("Atlas", Some("builder"), None, "My Repo", "ws_123", "inst_a");
        assert!(!p.contains('\n'), "must be one line (Codex -c literal)");
        assert!(!p.contains('='), "no '=' so Codex doesn't split it");
    }

    #[test]
    fn preamble_includes_identity_workspace_and_self_id() {
        let p = bootstrap_preamble("Atlas", Some("builder"), None, "My Repo", "ws_123", "inst_a");
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
        let p = bootstrap_preamble("Vega", None, None, "Repo", "ws_9", "inst_v");
        assert!(p.contains("Vega"));
        assert!(!p.contains("a  agent")); // no empty-role artifact
        assert!(p.contains("ws_9"));
    }

    #[test]
    fn preamble_bakes_role_description_and_roster_role_hint() {
        let p = bootstrap_preamble(
            "Atlas",
            Some("Lead"),
            Some("You are the lead. You settle decisions before anyone builds."),
            "My Repo",
            "ws_123",
            "inst_a",
        );
        // The verbatim job description is baked in.
        assert!(
            p.contains("You settle decisions before anyone builds."),
            "role description must be baked: {p}"
        );
        // The roster sentence now tells the agent peers carry roles + skills.
        assert!(
            p.contains("each peer's role and skills"),
            "roster sentence must mention peers' roles + skills: {p}"
        );
        // Invariants still hold with a description present.
        assert!(!p.contains('\n'));
        assert!(!p.contains('='));
    }

    #[test]
    fn preamble_omits_role_detail_when_no_description() {
        // A role name but no description bakes the "a X agent" clause without a
        // dangling empty sentence.
        let p = bootstrap_preamble("Vega", Some("Reviewer"), None, "Repo", "ws_9", "inst_v");
        assert!(p.contains("a Reviewer agent"), "{p}");
        assert!(!p.contains("agent id is inst_v.  "), "no empty role sentence: {p}");
    }

    #[test]
    fn preamble_stays_single_line_and_equals_free_with_hostile_input() {
        // A crafted name/role/workspace/self-id must not be able to introduce `=`
        // or a newline that would break Codex's `-c developer_instructions=…`.
        let p = bootstrap_preamble(
            "a=b\nc",
            Some("r=1"),
            // A hostile role DESCRIPTION must be sanitized too — it's baked
            // verbatim into the preamble (ADR 0005).
            Some("desc=with\nnewline=and=equals"),
            "ws=prod\nx",
            "id\n1",
            "self=x\ny",
        );
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
    fn restart_resume_prompts_are_single_line_and_name_the_commands() {
        let save = super::restart_save_prompt();
        let resume = super::resume_restore_prompt();
        assert!(!save.contains('\n'), "restart save prompt must be one line");
        assert!(!resume.contains('\n'), "resume prompt must be one line");
        // Each must name the exact command the agent has to run.
        assert!(save.contains("conclave snapshot save"));
        assert!(resume.contains("conclave snapshot last"));
        // The restart save prompt must be honest that the PROCESS dies.
        assert!(save.contains("RESTARTED"), "{save}");
    }

    /// ADR 0006: the self-triggered restart instruction — returned by
    /// `instance.restart`'s `self: true` path and printed VERBATIM as
    /// `conclave restart`'s command output (never injected as a chat turn).
    #[test]
    fn self_restart_instruction_is_single_line_and_names_the_command() {
        let s = super::self_restart_instruction(std::time::Duration::from_secs(300));
        assert!(!s.contains('\n'), "must be one line: {s}");
        assert!(s.contains("conclave snapshot save"), "{s}");
    }

    /// The arm's TTL must be surfaced honestly so a dawdling agent knows its
    /// deadline (Task 3 risk ledger) — computed from whatever `Duration` is
    /// passed in, never a hand-copied literal.
    #[test]
    fn self_restart_instruction_surfaces_the_given_ttl_in_minutes() {
        let s = super::self_restart_instruction(std::time::Duration::from_secs(300));
        assert!(s.contains("5 minutes"), "must surface the 5-minute TTL: {s}");

        let s10 = super::self_restart_instruction(std::time::Duration::from_secs(600));
        assert!(s10.contains("10 minutes"), "must surface a DIFFERENT TTL: {s10}");
    }

    /// Review item G2b: a naive `secs / 60` truncates a sub-minute TTL to
    /// "0 minutes" (nonsensical) and silently drops the remainder of a
    /// non-round one. Must render both sanely.
    #[test]
    fn self_restart_instruction_renders_sub_minute_and_non_round_ttls_sanely() {
        let sub_minute = super::self_restart_instruction(std::time::Duration::from_secs(30));
        assert!(
            !sub_minute.contains("0 minutes"),
            "a sub-minute TTL must not render as '0 minutes': {sub_minute}"
        );
        assert!(sub_minute.contains("30 seconds"), "{sub_minute}");

        let non_round = super::self_restart_instruction(std::time::Duration::from_secs(90));
        assert!(
            non_round.contains("1 minute") && non_round.contains("30 seconds"),
            "a non-round TTL must render both parts, not drop the remainder: {non_round}"
        );

        // Singular forms, not "1 minutes" / "1 seconds".
        let one_minute = super::self_restart_instruction(std::time::Duration::from_secs(60));
        assert!(one_minute.contains("1 minute"), "{one_minute}");
        assert!(!one_minute.contains("1 minutes"), "{one_minute}");
        let one_second = super::self_restart_instruction(std::time::Duration::from_secs(1));
        assert!(one_second.contains("1 second"), "{one_second}");
        assert!(!one_second.contains("1 seconds"), "{one_second}");
    }

    /// Review item G2: an agent whose save lands AFTER the arm's TTL expires
    /// must not be left hanging forever — the instruction must tell it to
    /// re-run `conclave restart` (cheap: the handoff is already saved, this
    /// just re-arms) rather than unconditionally promising the restart fires.
    #[test]
    fn self_restart_instruction_covers_the_late_save_recovery_path() {
        let s = super::self_restart_instruction(std::time::Duration::from_secs(300));
        assert!(
            s.contains("expired") && s.contains("conclave restart"),
            "must name the recovery path for a save that lands after expiry: {s}"
        );
    }

    /// Guards against the two restart paths silently drifting apart: both
    /// the human-triggered save prompt and the self-triggered instruction
    /// must carry the SAME handoff-writing instructions (extracted into
    /// `HANDOFF_SAVE_INSTRUCTIONS`).
    #[test]
    fn restart_save_prompt_and_self_restart_instruction_share_handoff_instructions() {
        let save = super::restart_save_prompt();
        let instruction = super::self_restart_instruction(std::time::Duration::from_secs(300));
        // A distinctive, unlikely-to-coincidentally-match substring from the
        // shared handoff-writing instructions.
        let marker = "HARD CAP of 10k tokens";
        assert!(save.contains(marker), "{save}");
        assert!(instruction.contains(marker), "{instruction}");
    }

    /// Review item G1: the exact wording is load-bearing (it's what a
    /// restarting agent reads to decide what to do), so pin it to a frozen
    /// baseline — a marker-substring check alone would miss an accidental
    /// rewording elsewhere in the string.
    #[test]
    fn restart_save_prompt_matches_frozen_baseline() {
        assert_eq!(
            super::restart_save_prompt(),
            "[conclave restart] Your process is about to be RESTARTED (killed and relaunched); \
your context will not survive. Write the RICHEST handoff you can for a reader with ZERO memory \
of this conversation — follow your Strategic Compact skill's seven sections if you have it, \
else cover: the exact next step and any half-finished edit FIRST, then goal/authority/peers, \
every decision with its why, open threads with your defaults, hard-won gotchas and failed \
approaches, done work as commit SHAs, and pointers. Do not economize tokens — the only limit is \
a HARD CAP of 10k tokens (~40,000 characters). REFERENCE commit SHAs and file paths instead of \
pasting their contents, and REDACT secrets (API keys, tokens, passwords). Then persist it by \
running this single command (do not just print it): `conclave snapshot save <your full handoff \
text>`. After it confirms, stop and wait for the restart."
        );
    }

    #[test]
    fn preamble_trims_blank_role() {
        // A blank role collapses to the bare-name who-clause, not "Sol", a  agent,".
        let p = bootstrap_preamble("Sol", Some("   "), None, "Repo", "ws_1", "inst_s");
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

    /// The sidecar's content reaches the model only through a file read whose
    /// result lives in CONVERSATION history — which `/clear` erases. The
    /// pointer (system-prompt layer) is the only survivor, so it must order a
    /// re-read on every fresh context, not just "before your first response".
    #[test]
    fn skill_pointer_sentence_orders_reread_after_context_clear() {
        let s = super::skill_pointer_sentence(std::path::Path::new("/tmp/inst-a.md"));
        assert!(s.contains("re-read"), "{s}");
        assert!(s.contains("clear"), "{s}");
    }

    /// Restore prompts drive the agent straight into `conclave snapshot last`
    /// + continue-the-task. Without an explicit first step to re-read the
    ///   standing-instructions file, the agent resumes work skill-less — the
    ///   exact "forgets skills after /clear" bug. Both fresh-context prompts
    ///   must name that step BEFORE the snapshot restore.
    #[test]
    fn fresh_context_restore_prompts_order_skill_file_reread_first() {
        for p in [super::compact_restore_prompt(), super::resume_restore_prompt()] {
            assert!(p.contains("standing-instructions"), "{p}");
            assert!(p.contains("re-read"), "{p}");
            let reread = p.find("re-read").unwrap();
            let snapshot = p.find("conclave snapshot last").unwrap();
            assert!(reread < snapshot, "re-read must come before snapshot restore: {p}");
        }
    }

    /// The invariant the whole feature exists to protect: appending the skill
    /// pointer sentence to a real preamble must NOT reintroduce a newline or
    /// '=', even when the underlying skill body (never embedded here) is
    /// pathological — see ADR 0001.
    #[test]
    fn preamble_with_skill_pointer_appended_stays_single_line_and_equals_free() {
        let p = bootstrap_preamble("Atlas", Some("builder"), None, "My Repo", "ws_123", "inst_a");
        let pointer = super::skill_pointer_sentence(std::path::Path::new("/tmp/inst_a.md"));
        let combined = format!("{p} {pointer}");
        assert!(!combined.contains('\n'), "no newline: {combined}");
        assert!(!combined.contains('='), "no '=': {combined}");
    }

    /// ADR 0004: the live-reload nudge injected into a running instance after
    /// a skill mutation. Mirrors `skill_pointer_sentence`'s single-line/`=`
    /// -free/path-naming invariants, plus the two things a NUDGE (not a
    /// first-launch pointer) must add: the word UPDATED, and an instruction
    /// that any previously-read copy already in context is stale.
    #[test]
    fn skills_updated_prompt_is_single_line_and_equals_free() {
        let s = super::skills_updated_prompt(std::path::Path::new("/tmp/a=b\nc.md"));
        assert!(!s.contains('\n'), "no newline: {s}");
        assert!(!s.contains('='), "no '=': {s}");
    }

    #[test]
    fn skills_updated_prompt_names_the_path() {
        let s = super::skills_updated_prompt(std::path::Path::new("/tmp/inst-a.md"));
        assert!(s.contains("/tmp/inst-a.md"), "{s}");
    }

    #[test]
    fn skills_updated_prompt_says_updated_and_orders_reread_now() {
        let s = super::skills_updated_prompt(std::path::Path::new("/tmp/inst-a.md"));
        assert!(s.contains("UPDATED"), "{s}");
        assert!(s.contains("re-read"), "{s}");
        assert!(s.contains("stale"), "{s}");
    }

    /// ADR 0004 Task 5: the field bug this exists for is a launch shell's
    /// PATH export getting reset by the harness's own re-sourced rc files.
    /// The preamble (system-prompt layer, present every turn) is the
    /// reliable fallback channel, so this sentence must survive the same
    /// single-line/`=`-free hostile-path gauntlet as `skill_pointer_sentence`.
    #[test]
    fn conclave_path_sentence_is_single_line_and_equals_free() {
        let s = super::conclave_path_sentence(std::path::Path::new("/tmp/a=b\nc/conclave"));
        assert!(!s.contains('\n'), "no newline: {s}");
        assert!(!s.contains('='), "no '=': {s}");
    }

    #[test]
    fn conclave_path_sentence_names_the_path() {
        let s = super::conclave_path_sentence(std::path::Path::new(
            "/Users/x/Library/Application Support/Conclave/bin/conclave",
        ));
        // Backtick-wrapped (Mellow's L2 review): `sanitize_field` doesn't
        // strip backticks, so the path's boundary mid-sentence stays
        // unambiguous even with spaces in it (as this fixture path has).
        assert!(
            s.contains("`/Users/x/Library/Application Support/Conclave/bin/conclave`"),
            "{s}"
        );
    }

    /// Must not just name the path — it must tell the agent to RUN it via
    /// that path when `conclave` isn't found, not go hunting for the binary
    /// (which burns a turn, the exact field bug this fixes).
    #[test]
    fn conclave_path_sentence_instructs_running_via_full_path_not_searching() {
        let s = super::conclave_path_sentence(std::path::Path::new("/tmp/conclave"));
        assert!(s.contains("not found"), "{s}");
        assert!(s.contains("run it via this full path"), "{s}");
    }

    /// The full assembled preamble (bootstrap + skill pointer + conclave path
    /// sentence) must stay single-line/`=`-free even under hostile input in
    /// every component — this is the invariant the whole feature protects.
    #[test]
    fn full_preamble_with_conclave_path_sentence_stays_single_line_and_equals_free() {
        let p = bootstrap_preamble("Atlas", Some("builder"), None, "My Repo", "ws_123", "inst_a");
        let pointer = super::skill_pointer_sentence(std::path::Path::new("/tmp/inst_a.md"));
        let conclave = super::conclave_path_sentence(std::path::Path::new("/tmp/a=b\nc/conclave"));
        let combined = format!("{p} {pointer} {conclave}");
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

    /// Regression for the app-relaunch respawn race: every agent's spawn calls
    /// the shim refresh at once, and under the old remove-then-recreate pair
    /// all but one hit EEXIST and reported failure — dropping the PATH export
    /// and the preamble's path-fallback sentence for 3 of 4 agents in the
    /// field. Every concurrent caller must succeed, and the link must resolve
    /// to the CLI afterwards.
    #[cfg(unix)]
    #[test]
    fn refresh_shim_link_survives_concurrent_callers() {
        let dir = std::env::temp_dir().join(format!(
            "conclave-shim-race-test-{}",
            std::process::id()
        ));
        // Pid-namespaced dirs recycle across reboots: clear debris from a
        // crashed prior run so it can't trip the stray-temp assert below.
        let _ = std::fs::remove_dir_all(&dir);
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).expect("create shim dir");
        let cli = dir.join("conclave-cli");
        std::fs::write(&cli, b"#!/bin/sh\n").expect("write fake cli");

        let results: Vec<bool> = std::thread::scope(|s| {
            (0..8)
                .map(|_| s.spawn(|| super::refresh_shim_link(&cli, &bin)))
                .collect::<Vec<_>>()
                .into_iter()
                .map(|h| h.join().expect("thread panicked"))
                .collect()
        });
        assert!(
            results.iter().all(|&ok| ok),
            "every concurrent refresh must succeed: {results:?}"
        );
        let resolved = std::fs::read_link(bin.join("conclave")).expect("link exists");
        assert_eq!(resolved, cli);
        // No temp debris left behind (each loser renamed or cleaned up).
        let stray: Vec<_> = std::fs::read_dir(&bin)
            .expect("read shim dir")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "conclave")
            .collect();
        assert!(stray.is_empty(), "temp files left behind: {stray:?}");
        let _ = std::fs::remove_dir_all(&dir); // test cleanup
    }
}
