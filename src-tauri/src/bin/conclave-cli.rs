#![cfg(unix)]
//! `conclave-cli` — thin JSON-RPC client for the Conclave UDS server.
//!
//! Parses argv into a `cli.exec` request, sends it over the Unix-domain
//! socket, and pretty-prints the result to stdout. All IPC goes through the
//! `cli.exec` allowlist in the server; this binary cannot bypass the router's
//! security boundary.
//!
//! On macOS, the socket lives at:
//!   `~/Library/Application Support/Conclave/conclave.sock`
//!
//! Usage is printed when the binary is run with no arguments, `help`,
//! `--help`, or `-h`.

use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// Connect retry budget: the app binds its UDS listener asynchronously during
/// startup (and briefly unlinks/rebinds the socket file on restart), so a
/// single-shot connect right after launch can race a listener that hasn't
/// bound yet and see ECONNREFUSED/ENOENT even though the app is genuinely
/// coming up. Spreading a few attempts over ~2s absorbs that race without
/// making a real "app isn't running" error wait any longer than necessary.
const CONNECT_RETRY_ATTEMPTS: u32 = 8;
const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(250);

/// Connect to the UDS socket at `path`, retrying on failure for a short,
/// bounded window (see [`CONNECT_RETRY_ATTEMPTS`]). Returns the last error if
/// every attempt fails.
async fn connect_with_retry(path: &std::path::Path) -> std::io::Result<UnixStream> {
    let mut last_err = None;
    for attempt in 0..CONNECT_RETRY_ATTEMPTS {
        match UnixStream::connect(path).await {
            Ok(stream) => return Ok(stream),
            Err(e) => {
                last_err = Some(e);
                if attempt + 1 < CONNECT_RETRY_ATTEMPTS {
                    tokio::time::sleep(CONNECT_RETRY_DELAY).await;
                }
            }
        }
    }
    Err(last_err.expect("loop runs at least once"))
}

/// Resolve the socket path (`~/Library/Application Support/Conclave/conclave.sock`).
///
/// Read-only: the client never creates the directory or adjusts permissions —
/// that is the server's responsibility (see `engine::uds::socket_path`).
///
/// Returns `None` if the user data directory cannot be resolved (e.g. `$HOME`
/// unset), so `main` can exit cleanly rather than panic with a backtrace.
fn socket_path() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("Conclave").join("conclave.sock"))
}

const USAGE: &str = "\
Usage: conclave <subcommand> [args...]

Subcommands:
  ws list
  ws use <workspaceId>
  agent list <workspaceId>
  send <sessionId> <text...>
  tell <agentId> <text...>          (agent→agent; inside a spawned agent)
  bb list <workspaceId>
  bb get <workspaceId> <key>
  bb set <workspaceId> <key> <value>
  bb delete <workspaceId> <key>     (alias: bb rm)
  snapshot list <sessionId>
  snapshot read <snapshotId>
  snapshot create <sessionId> <type> [label]
  snapshot save <text...>           (agent self-handoff; inside a spawned agent)
  snapshot last                     (read your latest handoff; inside a spawned agent)
  restart                           (self-triggered restart; inside a spawned agent)
  memory remember <workspaceId> <text...>
  memory search   <workspaceId> <query...> [--limit N]
  memory delete   <workspaceId> <chunkId>
  memory status   <workspaceId>
  lane start  <workspaceId> <slug>      (add lane worktree + claim task if present)
  lane finish <workspaceId> <slug>      (remove worktree + delete branch, after merge)
  lane guard install                    (install the shared-checkout commit-scope guard)
  run <orchestratorId> <prompt...>
  help

Requires the Conclave app to be running.
";

/// Expand the agent-friendly subcommands that are keyed on the CALLER's own
/// instance into the wire form the server allowlist expects, filling the
/// instance id from the spawned agent's `CONCLAVE_INSTANCE_ID`:
///
/// - `tell <toInstanceId> <text...>` → `tell <selfId> <toInstanceId> <text...>`
/// - `snapshot save <text...>`       → `snapshot save <selfId> <text...>`
/// - `snapshot last`                 → `snapshot last <selfId>`
/// - `memory remember <workspaceId> <text...>` →
///   `memory remember <selfId-or-"-"> <workspaceId> <text...>` (ADR 0007:
///   optional author stamping — unlike the forms above, this one does NOT
///   require a known instance id; outside a spawned agent it injects the
///   sentinel `"-"` so the chunk saves as `manual`, same as today).
///
/// Everything else (including `snapshot list/read/create`, which take an explicit
/// id) passes through untouched.
///
/// Errors (as a user-facing string) when `tell`/`snapshot save`/`snapshot last`/
/// `restart` are used without a known instance id (i.e. run outside a spawned
/// agent), or when any of these forms is missing required arguments.
fn expand_self_args(argv: Vec<String>, self_instance: Option<&str>) -> Result<Vec<String>, String> {
    // Resolve the caller's own instance id, or a user-facing error naming `cmd`.
    let require_self = |cmd: &str| -> Result<&str, String> {
        self_instance.filter(|s| !s.is_empty()).ok_or_else(|| {
            format!(
                "conclave: `{cmd}` is only available inside a spawned agent (CONCLAVE_INSTANCE_ID unset)"
            )
        })
    };

    match argv.first().map(String::as_str) {
        Some("tell") => {
            let from = require_self("tell")?;
            if argv.len() < 3 {
                return Err("conclave: tell <agentId> <text...>".to_string());
            }
            let mut out = Vec::with_capacity(argv.len() + 1);
            out.push("tell".to_string()); // subcommand
            out.push(from.to_string()); // fromInstanceId (injected from env)
            out.extend_from_slice(&argv[1..]); // toInstanceId + text...
            Ok(out)
        }
        Some("snapshot") => match argv.get(1).map(String::as_str) {
            Some("save") => {
                let me = require_self("snapshot save")?;
                if argv.len() < 3 {
                    return Err("conclave: snapshot save <text...>".to_string());
                }
                let mut out = Vec::with_capacity(argv.len() + 1);
                out.push("snapshot".to_string());
                out.push("save".to_string());
                out.push(me.to_string()); // instanceId (injected from env)
                out.extend_from_slice(&argv[2..]); // text...
                Ok(out)
            }
            Some("last") => {
                let me = require_self("snapshot last")?;
                if argv.len() != 2 {
                    return Err("conclave: snapshot last".to_string());
                }
                Ok(vec![
                    "snapshot".to_string(),
                    "last".to_string(),
                    me.to_string(),
                ])
            }
            // snapshot list/read/create carry an explicit id — leave untouched.
            _ => Ok(argv),
        },
        // ADR 0006: self-triggered restart. Takes NO arguments — the target
        // is always the calling agent, resolved from `CONCLAVE_INSTANCE_ID`.
        Some("restart") => {
            let me = require_self("restart")?;
            if argv.len() != 1 {
                return Err("conclave: restart (no arguments)".to_string());
            }
            Ok(vec!["restart".to_string(), me.to_string()])
        }
        // ADR 0007: author stamping. Unlike `tell`/`snapshot save`/`restart`,
        // `memory remember` is valid both inside a spawned agent AND from a
        // plain terminal — stamping the author is optional enrichment, not a
        // requirement. When `CONCLAVE_INSTANCE_ID` is set, inject it as the
        // author slot; otherwise inject the sentinel "-" (never a real
        // instance id, which is always a UUID) so the server keeps the chunk
        // `manual` rather than fabricating an author.
        Some("memory") if argv.get(1).map(String::as_str) == Some("remember") => {
            if argv.len() < 4 {
                return Err("conclave: memory remember <workspaceId> <text...>".to_string());
            }
            let author = self_instance.filter(|s| !s.is_empty()).unwrap_or("-");
            let mut out = Vec::with_capacity(argv.len() + 1);
            out.push("memory".to_string());
            out.push("remember".to_string());
            out.push(author.to_string()); // author (injected from env, or "-")
            out.extend_from_slice(&argv[2..]); // workspaceId + text...
            Ok(out)
        }
        _ => Ok(argv),
    }
}

// ── Lane manager + commit guard (ADR 0008, Lane C) ─────────────────────────
//
// Unlike every other subcommand — thin `cli.exec` clients — the `lane` verbs
// are mostly LOCAL git operations (worktree lifecycle, hook install). Only
// `lane start`'s optional task wiring touches the UDS server, and it soft-fails
// so the worktree is usable even before Lane A's `task.*` commands are merged.

/// The pre-commit guard script, embedded so `lane guard install` is
/// self-contained and the integration test can pin the exact same bytes.
const GUARD_HOOK: &str = include_str!("pre_commit_guard.sh");

/// A stable substring of [`GUARD_HOOK`] used to recognise a hook this tool
/// installed, so re-installs are idempotent and a foreign pre-commit hook is
/// never silently clobbered.
const GUARD_MARKER: &str = "conclave lane commit guard";

const LANE_USAGE: &str = "\
Usage: conclave lane <start|finish|guard> ...
  lane start  <workspaceId> <slug>   add .claude/worktrees/<slug> on branch lane/<slug>
  lane finish <workspaceId> <slug>   remove the worktree + delete branch (after merge)
  lane guard install                 install the shared-checkout commit-scope pre-commit hook
";

/// Validate a lane slug. It becomes both a git branch component (`lane/<slug>`)
/// and a worktree directory (`.claude/worktrees/<slug>`), so keep it to a
/// conservative charset that cannot escape either — no `/`, no `..`, no
/// leading `-` (which git would read as a flag) or `.`.
fn validate_slug(slug: &str) -> Result<(), String> {
    if slug.is_empty() {
        return Err("conclave: lane slug must not be empty".to_string());
    }
    if slug.starts_with('-') || slug.starts_with('.') {
        return Err(format!(
            "conclave: lane slug '{slug}' must not start with '-' or '.'"
        ));
    }
    if !slug
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "conclave: lane slug '{slug}' may contain only letters, digits, '-' and '_'"
        ));
    }
    Ok(())
}

/// Run one `cli.exec` round-trip for the optional task wiring, returning the
/// server's `result` value or a user-facing error string. Kept separate from
/// `main`'s inline flow because lane wiring must treat a server error as a soft
/// warning, not a fatal exit.
async fn uds_task_call(argv: Vec<String>, self_instance: Option<&str>) -> Result<Value, String> {
    let argv = expand_self_args(argv, self_instance)?;
    let path = socket_path().ok_or("could not resolve user data directory")?;
    let stream = connect_with_retry(&path)
        .await
        .map_err(|e| format!("cannot connect to the app ({e}); is Conclave running?"))?;

    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "cli.exec",
        "params": { "argv": argv },
    });
    let mut request_line = serde_json::to_string(&request).expect("serialize request cannot fail");
    request_line.push('\n');

    let (read, mut write) = stream.into_split();
    write
        .write_all(request_line.as_bytes())
        .await
        .map_err(|e| format!("failed to send request: {e}"))?;

    let mut reader = BufReader::new(read);
    let mut line = String::new();
    match reader.read_line(&mut line).await {
        Ok(0) => return Err("connection closed before a response".to_string()),
        Err(e) => return Err(format!("failed to read response: {e}")),
        Ok(_) => {}
    }

    let response: Value =
        serde_json::from_str(line.trim()).map_err(|e| format!("could not parse response: {e}"))?;
    if let Some(error) = response.get("error") {
        return Err(error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error")
            .to_string());
    }
    response
        .get("result")
        .cloned()
        .ok_or_else(|| "malformed response (no result)".to_string())
}

/// After the worktree exists, best-effort claim the matching task and move it
/// to `in_progress`. Any failure (Lane A not merged, task absent, app down) is
/// a note, not an error — the worktree is already usable.
async fn lane_task_wiring(ws: &str, slug: &str, self_instance: Option<&str>) {
    let claim = vec![
        "task".to_string(),
        "claim".to_string(),
        ws.to_string(),
        slug.to_string(),
    ];
    match uds_task_call(claim, self_instance).await {
        Ok(_) => {
            println!("lane start: task '{slug}' claimed");
            let state = vec![
                "task".to_string(),
                "state".to_string(),
                ws.to_string(),
                slug.to_string(),
                "in_progress".to_string(),
            ];
            match uds_task_call(state, self_instance).await {
                Ok(_) => println!("lane start: task '{slug}' -> in_progress"),
                Err(e) => eprintln!("lane start: warning — could not set task state ({e})"),
            }
        }
        Err(e) => {
            eprintln!(
                "lane start: note — skipping task claim/state ({e}). \
                 The worktree is ready; wire the task once `conclave task` is available."
            );
        }
    }
}

/// `conclave lane start <ws> <slug>`: add a lane worktree off `main`, then
/// best-effort claim + start the matching task.
async fn lane_start(ws: &str, slug: &str, self_instance: Option<&str>) -> ExitCode {
    if let Err(e) = validate_slug(slug) {
        eprintln!("{e}");
        return ExitCode::from(2);
    }
    let worktree = format!(".claude/worktrees/{slug}");
    let branch = format!("lane/{slug}");

    // This exact form (`-b <branch> <path> main`) is required: a plain
    // `git worktree add <path> main` fails "main is already used by worktree"
    // because the shared checkout has `main` checked out.
    match Command::new("git")
        .args(["worktree", "add", "-b", &branch, &worktree, "main"])
        .status()
    {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!(
                "conclave: `git worktree add` failed (exit {})",
                s.code().unwrap_or(-1)
            );
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("conclave: could not run git: {e}");
            return ExitCode::from(2);
        }
    }
    println!("lane start: worktree {worktree} on branch {branch}");

    lane_task_wiring(ws, slug, self_instance).await;
    ExitCode::SUCCESS
}

/// `conclave lane finish <ws> <slug>`: refuse a dirty worktree, then remove it
/// and delete the branch. `-d` (not `-D`) is the safety — it refuses to delete
/// a branch not yet merged, so a premature finish before the lead merges fails
/// loudly instead of discarding work.
fn lane_finish(_ws: &str, slug: &str) -> ExitCode {
    if let Err(e) = validate_slug(slug) {
        eprintln!("{e}");
        return ExitCode::from(2);
    }
    let worktree = format!(".claude/worktrees/{slug}");
    let branch = format!("lane/{slug}");

    if !std::path::Path::new(&worktree).exists() {
        eprintln!("conclave: worktree {worktree} does not exist");
        return ExitCode::FAILURE;
    }

    // Dirty check first, for a clear message (`git worktree remove` also
    // refuses a dirty tree, but with a terser error).
    match Command::new("git")
        .args(["-C", &worktree, "status", "--porcelain"])
        .output()
    {
        Ok(o) if o.status.success() => {
            if !o.stdout.is_empty() {
                eprintln!("conclave: refusing to finish — {worktree} has uncommitted changes:");
                eprint!("{}", String::from_utf8_lossy(&o.stdout));
                eprintln!(
                    "The lead merges before finish; commit or discard these, then retry."
                );
                return ExitCode::FAILURE;
            }
        }
        Ok(o) => {
            eprintln!(
                "conclave: `git status` failed: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            );
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("conclave: could not run git: {e}");
            return ExitCode::from(2);
        }
    }

    match Command::new("git")
        .args(["worktree", "remove", &worktree])
        .status()
    {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!(
                "conclave: `git worktree remove` failed (exit {})",
                s.code().unwrap_or(-1)
            );
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("conclave: could not run git: {e}");
            return ExitCode::from(2);
        }
    }

    // `-d` refuses an unmerged branch. If it refuses, the worktree is already
    // gone but the branch (and its commits) survive — report it as a warning,
    // not a hard failure, so the user can merge then delete manually.
    match Command::new("git").args(["branch", "-d", &branch]).status() {
        Ok(s) if s.success() => {
            println!("lane finish: removed worktree {worktree} and branch {branch}");
        }
        Ok(_) => {
            eprintln!(
                "lane finish: removed worktree {worktree}, but branch {branch} is not fully \
                 merged — kept it. Merge it, then `git branch -d {branch}`."
            );
        }
        Err(e) => {
            eprintln!("conclave: could not run git: {e}");
            return ExitCode::from(2);
        }
    }
    ExitCode::SUCCESS
}

/// `conclave lane guard install`: write the pre-commit guard into the SHARED
/// checkout's hooks dir (via `--git-common-dir`, so it is installed once and
/// shared across every worktree — where the hook self-skips).
fn lane_guard_install() -> ExitCode {
    let common = match Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Ok(o) => {
            eprintln!(
                "conclave: not inside a git repository ({})",
                String::from_utf8_lossy(&o.stderr).trim()
            );
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("conclave: could not run git: {e}");
            return ExitCode::from(2);
        }
    };

    let hooks_dir = std::path::Path::new(&common).join("hooks");
    if let Err(e) = std::fs::create_dir_all(&hooks_dir) {
        eprintln!("conclave: could not create {}: {e}", hooks_dir.display());
        return ExitCode::from(2);
    }
    let hook_path = hooks_dir.join("pre-commit");

    // Never clobber a foreign pre-commit hook; re-installing our own is fine.
    if hook_path.exists() {
        let existing = std::fs::read_to_string(&hook_path).unwrap_or_default();
        if !existing.contains(GUARD_MARKER) {
            eprintln!(
                "conclave: an existing pre-commit hook at {} is not a conclave guard.",
                hook_path.display()
            );
            eprintln!("Back it up or remove it, then re-run `conclave lane guard install`.");
            return ExitCode::FAILURE;
        }
    }

    if let Err(e) = std::fs::write(&hook_path, GUARD_HOOK) {
        eprintln!("conclave: could not write {}: {e}", hook_path.display());
        return ExitCode::from(2);
    }
    use std::os::unix::fs::PermissionsExt;
    if let Err(e) = std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755)) {
        eprintln!("conclave: could not chmod {}: {e}", hook_path.display());
        return ExitCode::from(2);
    }
    println!(
        "lane guard: installed pre-commit hook at {}",
        hook_path.display()
    );
    println!("Set CONCLAVE_COMMIT_SCOPE to your lane's path prefixes before committing here.");
    ExitCode::SUCCESS
}

/// Dispatch the `lane` subcommand. Handled entirely here (before `main`'s
/// socket machinery) because these verbs render their own output and mostly
/// run git locally rather than issuing a single `cli.exec`.
async fn run_lane(argv: &[String], self_instance: Option<&str>) -> ExitCode {
    match argv.get(1).map(String::as_str) {
        Some("start") => match (argv.get(2), argv.get(3)) {
            (Some(ws), Some(slug)) => lane_start(ws, slug, self_instance).await,
            _ => {
                eprintln!("conclave: lane start <workspaceId> <slug>");
                ExitCode::from(2)
            }
        },
        Some("finish") => match (argv.get(2), argv.get(3)) {
            (Some(ws), Some(slug)) => lane_finish(ws, slug),
            _ => {
                eprintln!("conclave: lane finish <workspaceId> <slug>");
                ExitCode::from(2)
            }
        },
        Some("guard") => match argv.get(2).map(String::as_str) {
            Some("install") => lane_guard_install(),
            _ => {
                eprintln!("conclave: lane guard install");
                ExitCode::from(2)
            }
        },
        _ => {
            eprint!("{LANE_USAGE}");
            ExitCode::from(2)
        }
    }
}

/// How `main` renders a successful result row.
enum OutMode {
    /// One-line confirmation (`tell` / `snapshot save`) — never echo the payload.
    Terse,
    /// The carried-forward handoff text (`snapshot last`) — the agent reads this.
    Handoff,
    /// The engine's `instruction` field (`restart`, ADR 0006) — the agent's next
    /// step (write + save the handoff), printed verbatim so it reads as plain
    /// command output rather than an injected chat turn.
    Instruction,
    /// Pretty-printed JSON (everything else).
    Json,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();

    // Empty invocation or explicit help request — print usage and exit cleanly.
    if argv.is_empty() || argv[0] == "help" || argv[0] == "--help" || argv[0] == "-h" {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    // `lane` is handled locally (git worktree lifecycle + hook install), not as
    // a single `cli.exec` — dispatch it before the socket machinery below.
    if argv[0] == "lane" {
        return run_lane(&argv, std::env::var("CONCLAVE_INSTANCE_ID").ok().as_deref()).await;
    }

    // Expand the self-keyed forms (`tell`, `snapshot save`, `snapshot last`) to
    // their wire form, filling the instance id from CONCLAVE_INSTANCE_ID (set on
    // spawned agents).
    let argv = match expand_self_args(argv, std::env::var("CONCLAVE_INSTANCE_ID").ok().as_deref()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };

    // Pick how to render a successful result:
    // - `tell` / `snapshot save` echo a full row; printing it into the agent's
    //   context would waste tokens, so collapse to a one-line confirmation.
    // - `snapshot last` exists precisely SO the agent reads its handoff — print
    //   the carried-forward content, not the JSON row.
    let is_snapshot = argv.first().map(String::as_str) == Some("snapshot");
    let sub = argv.get(1).map(String::as_str);
    let out_mode = if argv.first().map(String::as_str) == Some("tell")
        || (is_snapshot && sub == Some("save"))
    {
        OutMode::Terse
    } else if is_snapshot && sub == Some("last") {
        OutMode::Handoff
    } else if argv.first().map(String::as_str) == Some("restart") {
        OutMode::Instruction
    } else {
        OutMode::Json
    };

    let path = match socket_path() {
        Some(p) => p,
        None => {
            eprintln!("conclave: could not resolve user data directory");
            return ExitCode::from(2);
        }
    };

    // Connect to the running Conclave app via its Unix-domain socket, with a
    // short retry window to absorb the app's async startup/restart race.
    let stream = match connect_with_retry(&path).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "conclave: cannot connect to the app at {} ({}). Is Conclave running?",
                path.display(),
                e
            );
            return ExitCode::from(2);
        }
    };

    // Build the JSON-RPC 2.0 request envelope. The server-side `cli.exec`
    // allowlist validates the subcommand; this client is intentionally dumb.
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "cli.exec",
        "params": { "argv": argv },
    });

    let mut request_line = serde_json::to_string(&request).expect("serialize request cannot fail");
    request_line.push('\n');

    let (read, mut write) = stream.into_split();

    if let Err(e) = write.write_all(request_line.as_bytes()).await {
        eprintln!("conclave: failed to send request: {e}");
        return ExitCode::from(2);
    }

    // Read exactly one response line.
    let mut reader = BufReader::new(read);
    let mut line = String::new();

    match reader.read_line(&mut line).await {
        Ok(0) => {
            eprintln!("conclave: connection closed before a response was received");
            return ExitCode::from(2);
        }
        Err(e) => {
            eprintln!("conclave: failed to read response: {e}");
            return ExitCode::from(2);
        }
        Ok(_) => {}
    }

    let response: Value = match serde_json::from_str(line.trim()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("conclave: could not parse server response: {e}");
            return ExitCode::from(2);
        }
    };

    // JSON-RPC 2.0: exactly one of `result` or `error` is present.
    if let Some(error) = response.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        eprintln!("conclave: {message}");
        return ExitCode::FAILURE;
    }

    if let Some(result) = response.get("result") {
        match out_mode {
            // Terse: `tell` → "delivered -> <to>"; `snapshot save` → "saved <id>".
            // Never echo the payload text back into the agent's context.
            OutMode::Terse => {
                if let Some(to) = result.get("toInstanceId").and_then(Value::as_str) {
                    let status = result
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("sent");
                    println!("{status} -> {to}");
                } else {
                    let id = result.get("id").and_then(Value::as_str).unwrap_or("");
                    println!("saved snapshot {id}");
                }
            }
            // The carried-forward handoff — the whole point of `snapshot last` is
            // for the agent to READ this, so print the content, not the JSON row.
            // If the latest snapshot somehow carries no handoff text, fail LOUDLY
            // (non-zero) rather than print a placeholder the agent would silently
            // "restore" from.
            OutMode::Handoff => match result.get("carriedForward").and_then(Value::as_str) {
                Some(content) => println!("{content}"),
                None => {
                    eprintln!(
                        "conclave: latest snapshot has no handoff content (type: {})",
                        result.get("type").and_then(Value::as_str).unwrap_or("?")
                    );
                    return ExitCode::FAILURE;
                }
            },
            // `restart`'s whole point is telling the agent what to do next —
            // print that instruction, not the status/phase JSON row.
            OutMode::Instruction => match result.get("instruction").and_then(Value::as_str) {
                Some(text) => println!("{text}"),
                None => {
                    eprintln!(
                        "conclave: restart response has no instruction field (status: {})",
                        result.get("status").and_then(Value::as_str).unwrap_or("?")
                    );
                    return ExitCode::FAILURE;
                }
            },
            OutMode::Json => {
                let pretty =
                    serde_json::to_string_pretty(result).expect("serialize result cannot fail");
                println!("{pretty}");
            }
        }
        return ExitCode::SUCCESS;
    }

    // Neither `result` nor `error` — malformed response from the server.
    eprintln!("conclave: malformed response (no result or error field)");
    ExitCode::from(2)
}

#[cfg(test)]
mod tests {
    use super::{expand_self_args, validate_slug, GUARD_HOOK, GUARD_MARKER};

    fn v(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn tell_injects_sender_from_env() {
        let out = expand_self_args(v(&["tell", "to1", "hello", "there"]), Some("self1")).unwrap();
        assert_eq!(out, v(&["tell", "self1", "to1", "hello", "there"]));
    }

    #[test]
    fn tell_without_instance_id_errors() {
        assert!(expand_self_args(v(&["tell", "to1", "hi"]), None).is_err());
        assert!(expand_self_args(v(&["tell", "to1", "hi"]), Some("")).is_err());
    }

    #[test]
    fn tell_without_text_errors() {
        assert!(expand_self_args(v(&["tell", "to1"]), Some("self1")).is_err());
    }

    #[test]
    fn snapshot_save_injects_instance_from_env() {
        let out =
            expand_self_args(v(&["snapshot", "save", "my", "handoff"]), Some("self1")).unwrap();
        assert_eq!(out, v(&["snapshot", "save", "self1", "my", "handoff"]));
    }

    #[test]
    fn snapshot_save_without_instance_id_errors() {
        assert!(expand_self_args(v(&["snapshot", "save", "x"]), None).is_err());
    }

    #[test]
    fn snapshot_save_without_text_errors() {
        assert!(expand_self_args(v(&["snapshot", "save"]), Some("self1")).is_err());
    }

    #[test]
    fn snapshot_last_injects_instance_from_env() {
        let out = expand_self_args(v(&["snapshot", "last"]), Some("self1")).unwrap();
        assert_eq!(out, v(&["snapshot", "last", "self1"]));
    }

    #[test]
    fn snapshot_last_without_instance_id_errors() {
        assert!(expand_self_args(v(&["snapshot", "last"]), None).is_err());
    }

    // ── restart (ADR 0006: self-triggered restart) ─────────────────────────

    #[test]
    fn restart_injects_instance_from_env() {
        let out = expand_self_args(v(&["restart"]), Some("self1")).unwrap();
        assert_eq!(out, v(&["restart", "self1"]));
    }

    #[test]
    fn restart_without_instance_id_errors() {
        assert!(expand_self_args(v(&["restart"]), None).is_err());
        assert!(expand_self_args(v(&["restart"]), Some("")).is_err());
    }

    #[test]
    fn restart_takes_no_arguments() {
        assert!(expand_self_args(v(&["restart", "extra"]), Some("self1")).is_err());
    }

    #[test]
    fn snapshot_list_passes_through_untouched() {
        // `snapshot list/read/create` carry an explicit id — must NOT be expanded.
        let list = v(&["snapshot", "list", "sess1"]);
        assert_eq!(expand_self_args(list.clone(), Some("self1")).unwrap(), list);
        let create = v(&["snapshot", "create", "sess1", "manual"]);
        assert_eq!(
            expand_self_args(create.clone(), Some("self1")).unwrap(),
            create
        );
    }

    // ── memory remember (ADR 0007: optional author stamping) ───────────────

    #[test]
    fn memory_remember_injects_instance_from_env() {
        let out = expand_self_args(v(&["memory", "remember", "ws1", "hi", "there"]), Some("self1"))
            .unwrap();
        assert_eq!(
            out,
            v(&["memory", "remember", "self1", "ws1", "hi", "there"])
        );
    }

    #[test]
    fn memory_remember_injects_sentinel_without_instance_id() {
        let out = expand_self_args(v(&["memory", "remember", "ws1", "hi"]), None).unwrap();
        assert_eq!(out, v(&["memory", "remember", "-", "ws1", "hi"]));
    }

    #[test]
    fn memory_remember_injects_sentinel_with_empty_instance_id() {
        let out = expand_self_args(v(&["memory", "remember", "ws1", "hi"]), Some("")).unwrap();
        assert_eq!(out, v(&["memory", "remember", "-", "ws1", "hi"]));
    }

    #[test]
    fn memory_remember_missing_text_errors_even_without_instance_id() {
        assert!(expand_self_args(v(&["memory", "remember", "ws1"]), None).is_err());
    }

    #[test]
    fn memory_search_delete_status_pass_through_untouched() {
        let search = v(&["memory", "search", "ws1", "query"]);
        assert_eq!(
            expand_self_args(search.clone(), Some("self1")).unwrap(),
            search
        );
        let status = v(&["memory", "status", "ws1"]);
        assert_eq!(expand_self_args(status.clone(), None).unwrap(), status);
    }

    #[test]
    fn non_self_args_pass_through_untouched() {
        let args = v(&["send", "sess1", "hello"]);
        assert_eq!(expand_self_args(args.clone(), Some("self1")).unwrap(), args);
        let list = v(&["agent", "list", "ws1"]);
        assert_eq!(expand_self_args(list.clone(), None).unwrap(), list);
    }

    // ── lane (ADR 0008, Lane C) ────────────────────────────────────────────

    #[test]
    fn validate_slug_accepts_conventional_lane_names() {
        for ok in ["aws-c", "memory_graph", "lane1", "a", "aws-c-2"] {
            assert!(validate_slug(ok).is_ok(), "expected '{ok}' to be valid");
        }
    }

    #[test]
    fn validate_slug_rejects_path_and_flag_escapes() {
        // empty, path separators, traversal, leading dash/dot, whitespace.
        for bad in ["", "a/b", "..", "../x", "-x", ".hidden", "a b", "a.b", "a\tb"] {
            assert!(
                validate_slug(bad).is_err(),
                "expected '{bad}' to be rejected"
            );
        }
    }

    #[test]
    fn guard_hook_is_wired_and_recognisable() {
        // The embedded hook must be non-empty POSIX sh carrying the marker used
        // for idempotent re-install / foreign-hook detection.
        assert!(GUARD_HOOK.starts_with("#!/bin/sh"));
        assert!(GUARD_HOOK.contains(GUARD_MARKER));
        // The load-bearing self-skip must reference both git-dir queries.
        assert!(GUARD_HOOK.contains("--git-dir"));
        assert!(GUARD_HOOK.contains("--git-common-dir"));
        assert!(GUARD_HOOK.contains("CONCLAVE_COMMIT_SCOPE"));
    }
}
