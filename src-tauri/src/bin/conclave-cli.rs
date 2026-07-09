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
  msg list [--limit N]              (read YOUR own inter-agent inbox+outbox, newest-first; inside a spawned agent)
  msg all  <workspaceId> [--limit N] (read the whole workspace's inter-agent traffic, newest-first)
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
  memory propose  <workspaceId> <text...> [--source-note NOTE]   (distilled candidate; inside a spawned agent)
  memory queue    <workspaceId> [--state pending|approved|rejected]
  memory approve  <workspaceId> <proposalId> [--reason TEXT...]  (reviewer ≠ proposer; inside a spawned agent)
  memory reject   <workspaceId> <proposalId> [--reason TEXT...]  (inside a spawned agent)
  lane start  <workspaceId> <slug>      (add lane worktree + claim task if present)
  lane finish <workspaceId> <slug>      (remove worktree + delete branch, after merge)
  lane guard install                    (install the shared-checkout commit-scope guard)
  stage status  <workspaceId> <slug>                    (git status, partitioned by task boundary)
  stage diff    <workspaceId> <slug>                    (git diff HEAD, scoped to boundary)
  stage commit  <workspaceId> <slug> -m <msg>            (private-index commit of boundary paths only; inside a spawned agent)
  stage snap    <workspaceId> <slug> [-m <label>]        (explicit snapshot onto the op log; inside a spawned agent)
  stage log     <workspaceId> <slug>                    (list snapshots, newest first)
  stage restore <workspaceId> <slug> <snapSha>           (restore boundary paths from a snapshot; auto-snaps first; inside a spawned agent)
  stage clear   <workspaceId> <slug>                    (delete the snapshot ref)
  task create   <workspaceId> <slug> <title...> [--boundary p1,p2] [--canon txt] [--plan-file path]
  task list     <workspaceId> [--state s] [--full]
  task get      <workspaceId> <slug>
  task brief    <workspaceId> <slug> [--limit N]
  task claim    <workspaceId> <slug>          (inside a spawned agent)
  task state    <workspaceId> <slug> <state>  (inside a spawned agent)
  task note     <workspaceId> <slug> <text...>          (inside a spawned agent)
  task gate     <workspaceId> <slug> -- <cmd...>        (inside a spawned agent; runs <cmd> here, exits with <cmd>'s exit code; each word after -- is passed verbatim, not re-parsed by a shell — for shell syntax use: -- sh -c \"…\")
  task challenge <workspaceId> <slug> --claim t --evidence t --proposal t --default t [--deadline-min N]
  task rule     <workspaceId> <slug> <challengeEventId> <text...>  (inside a spawned agent)
  task close    <workspaceId> <slug>          (inside a spawned agent)
  task watch    <workspaceId> <slug>          (inside a spawned agent)
  task unwatch  <workspaceId> <slug>          (inside a spawned agent)
  uishot        [--task <slug>] <args...>     (runs the workspace's package.json \"uishot\" script here; with --task also records it as a task gate)
  artifact add  <workspaceId> --title <t> --kind <k> (--file <path> | --content <text>)  (kinds: markdown|code|html|svg|mermaid|react|text)
  artifact list <workspaceId>
  artifact get  <id>
  position set  <workspaceId> <agentId> [--level <junior|mid|senior|principal>|none] [--supervisor <agentId>|none]  (at least one flag; \"none\" clears)
  org           <workspaceId>          (indented supervisor tree)
  design review <workspaceId> [--json]  (deterministic design QA; gate with the plain form — --json is for data retrieval, it always exits 0)
  browser open|goto <url>              (in-app browser; missing scheme → https://)
  browser status | snapshot [--max-text N] | close   (status/snapshot print JSON)
  browser click|type <selector> [text...] | eval <js...>   (selectors come from snapshot; eval is local-only)
  browser screenshot [path] [--width N] [--height N]   (path defaults to ./browser-screenshot.png, resolved to an absolute path in this shell's cwd)
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
        // ADR 0008: task verbs. `gate` arrives here ALREADY fully expanded by
        // `run_task_gate` (it needs self too, but resolves it itself before
        // running the shell command) — pass it through untouched. `create`'s
        // owner is optional enrichment (mirrors `memory remember`): default it
        // to self when available and not already given via `--owner`, but
        // never require a spawned-agent context. Every other verb inherently
        // mutates task state/history and so REQUIRES self, same as `tell`.
        Some("task") if argv.get(1).map(String::as_str) == Some("gate") => Ok(argv),
        Some("task") if argv.get(1).map(String::as_str) == Some("create") => {
            if argv.iter().any(|w| w == "--owner") {
                return Ok(argv);
            }
            match self_instance.filter(|s| !s.is_empty()) {
                Some(me) => {
                    let mut out = argv;
                    out.push("--owner".to_string());
                    out.push(me.to_string());
                    Ok(out)
                }
                None => Ok(argv),
            }
        }
        Some("task")
            if matches!(
                argv.get(1).map(String::as_str),
                Some(
                    "claim"
                        | "state"
                        | "note"
                        | "challenge"
                        | "rule"
                        | "close"
                        | "watch"
                        | "unwatch"
                )
            ) =>
        {
            let verb = argv[1].clone();
            let me = require_self(&format!("task {verb}"))?;
            let mut out = Vec::with_capacity(argv.len() + 1);
            out.push("task".to_string());
            out.push(verb);
            out.push(me.to_string());
            out.extend_from_slice(&argv[2..]);
            Ok(out)
        }
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
        // plan memory-distill-queue: the review-queue verbs stamp the caller's
        // own agent id as proposer (`propose`) or reviewer (`approve`/`reject`).
        // Unlike `memory remember`, these REQUIRE a spawned-agent context — the
        // gate compares proposer vs reviewer by real agent id, and the "-"
        // sentinel is never a valid agent. `memory queue` is a read-only list
        // and passes through untouched.
        Some("memory")
            if matches!(
                argv.get(1).map(String::as_str),
                Some("propose" | "approve" | "reject")
            ) =>
        {
            let verb = argv[1].clone();
            let me = require_self(&format!("memory {verb}"))?;
            if argv.len() < 3 {
                return Err(format!("conclave: memory {verb} <workspaceId> ..."));
            }
            let mut out = Vec::with_capacity(argv.len() + 1);
            out.push("memory".to_string());
            out.push(verb);
            out.push(me.to_string()); // proposer/reviewer (injected from env)
            out.extend_from_slice(&argv[2..]); // workspaceId + rest...
            Ok(out)
        }
        // plan design-artifact-store: `artifact add` stamps the caller's own
        // agent id as the optional creator (free text — an instance OR def id).
        // Like `memory remember`, valid both inside a spawned agent AND from a
        // plain terminal: inject the sentinel "-" when CONCLAVE_INSTANCE_ID is
        // unset so the server keeps `agentId` NULL rather than fabricating one.
        // `artifact list`/`get` are read-only and pass through untouched.
        Some("artifact") if argv.get(1).map(String::as_str) == Some("add") => {
            if argv.len() < 3 {
                return Err(
                    "conclave: artifact add <workspaceId> --title <t> --kind <k> (--file <path> | --content <text>)"
                        .to_string(),
                );
            }
            let agent = self_instance.filter(|s| !s.is_empty()).unwrap_or("-");
            let mut out = Vec::with_capacity(argv.len() + 1);
            out.push("artifact".to_string());
            out.push("add".to_string());
            out.push(agent.to_string()); // creator agent (injected from env, or "-")
            out.extend_from_slice(&argv[2..]); // workspaceId + flags...
            Ok(out)
        }
        // `msg list` reads the CALLER's own inbox+outbox — inject the instance
        // id from CONCLAVE_INSTANCE_ID at argv[2], BEFORE any `--limit` tail,
        // exactly like `snapshot last`. Requires a spawned-agent context (there
        // is no "self" to read outside one). `msg all <ws>` takes an explicit
        // workspace id and needs no self — pass it (and any unknown sub) through.
        Some("msg") if argv.get(1).map(String::as_str) == Some("list") => {
            let me = require_self("msg list")?;
            let mut out = Vec::with_capacity(argv.len() + 1);
            out.push("msg".to_string());
            out.push("list".to_string());
            out.push(me.to_string()); // instanceId (injected from env)
            out.extend_from_slice(&argv[2..]); // any --limit N tail
            Ok(out)
        }
        Some("browser") if argv.get(1).map(String::as_str) == Some("screenshot") => {
            // Resolve the output path (default ./browser-screenshot.png) to an
            // absolute path in the AGENT's cwd, so the app process — which has a
            // different cwd — writes the PNG where the agent can read it.
            let mut out = argv.clone();
            // The path is the first non-flag token after "screenshot"; flags are
            // --width/--height with a following value. Find it, or inject default.
            let flags = ["--width", "--height"];
            let mut i = 2;
            let mut path_idx: Option<usize> = None;
            while i < out.len() {
                if flags.contains(&out[i].as_str()) {
                    // Only skip the value slot if there IS one and it doesn't
                    // itself look like a flag; otherwise treat this as a
                    // malformed/missing-value flag and advance by one so we
                    // don't misread the next flag as this one's value.
                    if i + 1 < out.len() && !out[i + 1].starts_with("--") {
                        i += 2; // skip flag + its value
                    } else {
                        i += 1;
                    }
                    continue;
                }
                path_idx = Some(i);
                break;
            }
            let cwd = std::env::current_dir().map_err(|e| format!("cwd: {e}"))?;
            match path_idx {
                Some(idx) => {
                    let p = std::path::Path::new(&out[idx]);
                    if p.is_relative() {
                        out[idx] = cwd.join(p).to_string_lossy().into_owned();
                    }
                }
                None => {
                    out.push(cwd.join("browser-screenshot.png").to_string_lossy().into_owned());
                }
            }
            Ok(out)
        }
        _ => Ok(argv),
    }
}

/// `artifact add ... --file <path>`: read the file at the CLIENT's cwd and
/// rewrite the flag into `--content <text>` (plus a derived `--filename
/// <basename>` when none was given), so the server's pure `map_argv` never does
/// file I/O — the same client-side resolution `task create --plan-file` uses
/// (see [`resolve_task_create_plan_file`]). Rejects supplying both `--file` and
/// `--content`. A no-op for anything but `artifact add` and for the inline
/// `--content` form.
fn resolve_artifact_add_file(argv: Vec<String>) -> Result<Vec<String>, String> {
    if argv.first().map(String::as_str) != Some("artifact")
        || argv.get(1).map(String::as_str) != Some("add")
    {
        return Ok(argv);
    }
    let Some(pos) = argv.iter().position(|w| w == "--file") else {
        return Ok(argv);
    };
    if argv.iter().any(|w| w == "--content") {
        return Err(
            "conclave: artifact add: give exactly one of --file or --content, not both".to_string(),
        );
    }
    let path = argv
        .get(pos + 1)
        .ok_or("conclave: artifact add: --file requires a path")?
        .clone();
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("conclave: artifact add: cannot read --file '{path}': {e}"))?;
    let basename = std::path::Path::new(&path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned());
    let mut out = argv;
    out[pos] = "--content".to_string();
    out[pos + 1] = content;
    if let Some(name) = basename {
        if !out.iter().any(|w| w == "--filename") {
            out.push("--filename".to_string());
            out.push(name);
        }
    }
    Ok(out)
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
                eprintln!("The lead merges before finish; commit or discard these, then retry.");
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
    let hook_path = hooks_dir.join("pre-commit");
    match Command::new("git")
        .args(["config", "--get", "core.hooksPath"])
        .output()
    {
        Ok(output) if output.status.success() => {
            let configured = String::from_utf8_lossy(&output.stdout);
            let configured = configured.trim_end_matches(['\r', '\n']);
            let configured = if configured.is_empty() {
                "(empty)"
            } else {
                configured
            };
            eprintln!(
                "conclave: warning: core.hooksPath is set to '{configured}'; the installed hook at \
                 {} will not fire unless core.hooksPath is unset or points to {}.",
                hook_path.display(),
                hooks_dir.display()
            );
        }
        _ => {}
    }

    if let Err(e) = std::fs::create_dir_all(&hooks_dir) {
        eprintln!("conclave: could not create {}: {e}", hooks_dir.display());
        return ExitCode::from(2);
    }

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

// ── Stage: private-index commit + attribution + snapshot op log (plan stage-v1) ──
//
// jj (Jujutsu)-inspired shared-checkout collaboration: a PRIVATE `GIT_INDEX_FILE`
// means `stage commit`/`stage snap` never read or write the shared `.git/index`,
// so two agents committing concurrently cannot interfere (the b9ab709 accident
// class). Attribution uses native git authorship (`GIT_AUTHOR_NAME`/`_EMAIL`)
// instead of the shared human identity (the c3d8fcb ambiguity) — the committer
// stays the repo default. Snapshots are ordinary git commits chained onto a
// local-only ref `refs/conclave/stage/<slug>` — a content-addressed op log that
// survives everything git survives and is never pushed.

const STAGE_USAGE: &str = "\
Usage: conclave stage <status|diff|commit|snap|log|restore|clear> ...
  stage status  <workspaceId> <slug>              git status, partitioned by task boundary
  stage diff    <workspaceId> <slug>              git diff HEAD, scoped to boundary
  stage commit  <workspaceId> <slug> -m <msg>     private-index commit of boundary paths only
  stage snap    <workspaceId> <slug> [-m <label>] explicit snapshot onto the op log
  stage log     <workspaceId> <slug>              list snapshots, newest first
  stage restore <workspaceId> <slug> <snapSha>    restore boundary paths from a snapshot (auto-snaps first)
  stage clear   <workspaceId> <slug>              delete the snapshot ref
";

/// The local-only op-log ref a task's snapshots chain onto. Never added to a
/// push refspec (risk ledger) — it lives only in this repo's `.git`.
fn stage_snapshot_ref(slug: &str) -> String {
    format!("refs/conclave/stage/{slug}")
}

/// Mirrors `pre_commit_guard.sh`'s `in_scope`: `path` is in boundary if it
/// equals a boundary entry or lives under it (`entry/...`), so a boundary of
/// "src" never matches "srcfoo". A trailing slash on an entry is stripped
/// first so "docs/" and "docs" behave identically.
fn path_in_boundary(path: &str, boundary: &[String]) -> bool {
    boundary.iter().any(|entry| {
        let e = entry.strip_suffix('/').unwrap_or(entry.as_str());
        !e.is_empty() && (path == e || path.starts_with(&format!("{e}/")))
    })
}

/// A task without a boundary has nothing for `stage` to scope to — the
/// boundary IS the contract (decision 3); a wrong boundary gets fixed by
/// amending the plan, never by an ad-hoc path override here.
fn require_boundary(slug: &str, boundary: Vec<String>) -> Result<Vec<String>, String> {
    if boundary.is_empty() {
        return Err(format!(
            "conclave: task '{slug}' has no fileBoundary — stage refuses to operate without \
             one (the boundary IS the contract; amend the plan if it's wrong)"
        ));
    }
    Ok(boundary)
}

/// Extract `fileBoundary` from a `task get` response and enforce
/// `require_boundary` on it. Split out from `stage_boundary` so the envelope
/// shape can be pinned in a unit test without a live UDS socket.
///
/// `task get` returns the envelope `{"task": {...}, "events": [...]}`, not a
/// flat task object, so `fileBoundary` lives under `.task` — read it there
/// only (envelope-only, no flat fallback: the engine has one response shape,
/// and a fallback would mask future envelope drift rather than surface it).
fn parse_task_boundary(result: &Value, slug: &str) -> Result<Vec<String>, String> {
    let boundary: Vec<String> = result
        .get("task")
        .and_then(|task| task.get("fileBoundary"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    require_boundary(slug, boundary)
}

/// Read a task's `fileBoundary` over the existing `task get` client path (no
/// new engine route — the plan's boundary is the CLI-only feature's only
/// server dependency).
async fn stage_boundary(
    ws: &str,
    slug: &str,
    self_instance: Option<&str>,
) -> Result<Vec<String>, String> {
    let argv = vec![
        "task".to_string(),
        "get".to_string(),
        ws.to_string(),
        slug.to_string(),
    ];
    let result = uds_task_call(argv, self_instance).await?;
    parse_task_boundary(&result, slug)
}

/// Resolve the calling agent's display name from the workspace roster (over
/// the existing `agent list` client path) — native git authorship (decision
/// 4) needs a human-readable name, not just the instance id.
async fn agent_identity(ws: &str, self_instance: &str) -> Result<(String, String), String> {
    let argv = vec!["agent".to_string(), "list".to_string(), ws.to_string()];
    let result = uds_task_call(argv, Some(self_instance)).await?;
    let rows = result
        .as_array()
        .ok_or_else(|| "conclave: agent list: malformed response".to_string())?;
    rows.iter()
        .find(|row| row.get("id").and_then(Value::as_str) == Some(self_instance))
        .map(|row| {
            let name = row
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(self_instance)
                .to_string();
            (name, self_instance.to_string())
        })
        .ok_or_else(|| {
            format!("conclave: agent id '{self_instance}' not found in workspace roster")
        })
}

/// Resolve the caller's own instance id for a `stage` verb that mutates
/// state (commit/snap/restore all end up authoring a commit) — same
/// requirement as `tell`/`task claim` etc in `expand_self_args`.
fn require_stage_self<'a>(cmd: &str, self_instance: Option<&'a str>) -> Result<&'a str, String> {
    self_instance.filter(|s| !s.is_empty()).ok_or_else(|| {
        format!(
            "conclave: `stage {cmd}` is only available inside a spawned agent (CONCLAVE_INSTANCE_ID unset)"
        )
    })
}

/// Run `git <args>` in `repo`, returning trimmed stdout or a user-facing
/// error including stderr. Threading an explicit `repo` (rather than relying
/// on the process cwd) is what makes the git-plumbing helpers below
/// unit-testable against a throwaway repo.
fn git_output(repo: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .map_err(|e| format!("could not run git {}: {e}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Same as [`git_output`] but with `GIT_INDEX_FILE` pointed at a private
/// index — the shared `.git/index` is never named on this path, so it is
/// never read or written (decision 2).
fn git_with_index(
    repo: &std::path::Path,
    index_path: &std::path::Path,
    args: &[&str],
) -> Result<String, String> {
    let output = Command::new("git")
        .current_dir(repo)
        .env("GIT_INDEX_FILE", index_path)
        .args(args)
        .output()
        .map_err(|e| format!("could not run git {}: {e}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// A path for a throwaway private index, unique per call (a UUIDv4 suffix —
/// two concurrent `stage` invocations must never collide on the same file).
fn tmp_index_path() -> PathBuf {
    std::env::temp_dir().join(format!("conclave-stage-index-{}", uuid::Uuid::new_v4()))
}

/// Build a tree object representing `base` (any commit-ish) with the CURRENT
/// working-tree content of `boundary` paths layered on top, via a private
/// index (decision 2) — the shared index is never touched. `add -A` (not
/// plain `add`) so a boundary deletion is captured, not just
/// modifications/additions (test 7).
fn build_boundary_tree(
    repo: &std::path::Path,
    base: &str,
    boundary: &[String],
) -> Result<String, String> {
    let index_path = tmp_index_path();
    let result = (|| {
        git_with_index(repo, &index_path, &["read-tree", base])?;
        let mut args: Vec<&str> = vec!["add", "-A", "--"];
        args.extend(boundary.iter().map(String::as_str));
        git_with_index(repo, &index_path, &args)?;
        git_with_index(repo, &index_path, &["write-tree"])
    })();
    let _ = std::fs::remove_file(&index_path);
    result
}

fn current_branch(repo: &std::path::Path) -> Result<String, String> {
    git_output(repo, &["symbolic-ref", "--short", "HEAD"])
        .map_err(|_| "stage commit requires a branch checkout (HEAD is detached)".to_string())
}

fn head_sha(repo: &std::path::Path) -> Result<String, String> {
    git_output(repo, &["rev-parse", "HEAD"])
}

fn tree_of(repo: &std::path::Path, commitish: &str) -> Result<String, String> {
    git_output(repo, &["rev-parse", &format!("{commitish}^{{tree}}")])
}

/// `git commit-tree` with agent authorship (decision 4). The COMMITTER is
/// deliberately left at the repo default (no `GIT_COMMITTER_*` env) — only
/// the author identifies the agent.
fn commit_tree(
    repo: &std::path::Path,
    tree: &str,
    parents: &[&str],
    message: &str,
    author_name: &str,
    author_email: &str,
) -> Result<String, String> {
    let mut args = vec!["commit-tree", tree];
    for p in parents {
        args.push("-p");
        args.push(p);
    }
    args.push("-m");
    args.push(message);
    let output = Command::new("git")
        .current_dir(repo)
        .env("GIT_AUTHOR_NAME", author_name)
        .env("GIT_AUTHOR_EMAIL", author_email)
        .args(&args)
        .output()
        .map_err(|e| format!("could not run git commit-tree: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git commit-tree failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Compare-and-swap ref update: `git update-ref <ref> <new> <old>` fails if
/// `<ref>`'s current value isn't exactly `<old>` — this is what makes two
/// concurrent `stage commit`s unable to clobber each other (decision 2).
fn update_ref_cas(
    repo: &std::path::Path,
    ref_name: &str,
    new_sha: &str,
    expected_old: &str,
) -> Result<(), String> {
    let output = Command::new("git")
        .current_dir(repo)
        .args(["update-ref", ref_name, new_sha, expected_old])
        .output()
        .map_err(|e| format!("could not run git update-ref: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Core commit mechanics (decision 2): build a boundary-only tree via a
/// private index, commit it with agent attribution + `Conclave-Task`/
/// `Conclave-Agent` trailers (decision 4), then CAS the branch ref forward —
/// on a stale CAS, refresh HEAD and rebuild the tree rather than failing
/// outright, since a peer's unrelated commit landing between our read and
/// write is expected in a shared checkout, not an error condition. Returns
/// the new commit sha and the number of changed boundary files.
///
/// Synchronous and UDS-free (boundary/identity are resolved by the caller)
/// so it is unit-testable against a throwaway repo — see
/// [`stage_commit_core_with_hook`] for the CAS-retry test seam.
fn stage_commit_core(
    repo: &std::path::Path,
    branch: &str,
    slug: &str,
    boundary: &[String],
    message: &str,
    author_name: &str,
    author_id: &str,
) -> Result<(String, usize), String> {
    stage_commit_core_with_hook(
        repo,
        branch,
        slug,
        boundary,
        message,
        author_name,
        author_id,
        |_attempt, _repo| {},
    )
}

/// [`stage_commit_core`] with a race-injection seam: `race_hook(attempt,
/// repo)` runs right after an attempt captures its view of HEAD (and before
/// it tries the CAS), so a test can move the branch ref out from under
/// attempt 1 and deterministically exercise the CAS-failure-then-
/// refresh-and-retry path (test 3) without a timing-dependent race.
/// Production always passes a no-op hook via [`stage_commit_core`].
#[allow(clippy::too_many_arguments)]
fn stage_commit_core_with_hook(
    repo: &std::path::Path,
    branch: &str,
    slug: &str,
    boundary: &[String],
    message: &str,
    author_name: &str,
    author_id: &str,
    race_hook: impl Fn(u32, &std::path::Path),
) -> Result<(String, usize), String> {
    let branch_ref = format!("refs/heads/{branch}");
    let author_email = format!("{author_id}@agents.conclave.local");
    let full_message = format!("{message}\n\nConclave-Task: {slug}\nConclave-Agent: {author_id}");

    for attempt in 1..=3u32 {
        let old_head = head_sha(repo)?;
        race_hook(attempt, repo);
        let head_tree = tree_of(repo, &old_head)?;
        let new_tree = build_boundary_tree(repo, &old_head, boundary)?;
        if new_tree == head_tree {
            return Err(format!("nothing to commit in boundary for '{slug}'"));
        }
        let new_commit = commit_tree(
            repo,
            &new_tree,
            &[&old_head],
            &full_message,
            author_name,
            &author_email,
        )?;
        match update_ref_cas(repo, &branch_ref, &new_commit, &old_head) {
            Ok(()) => {
                let n_files = git_output(repo, &["diff", "--name-only", &old_head, &new_commit])
                    .map(|s| s.lines().filter(|l| !l.is_empty()).count())
                    .unwrap_or(0);
                return Ok((new_commit, n_files));
            }
            Err(e) if attempt < 3 => {
                let _ = e; // refresh-and-retry: the loop re-reads HEAD from the top
                continue;
            }
            Err(e) => return Err(format!("branch moving too fast, retry ({e})")),
        }
    }
    unreachable!("loop always returns within 3 attempts")
}

/// Snapshot `boundary`'s CURRENT working-tree content onto
/// `refs/conclave/stage/<slug>` (decision 5), chained onto the ref's current
/// tip (or an orphan first snapshot if the ref doesn't exist yet). Skips the
/// ref update entirely when the new tree is identical to the previous
/// snapshot's — the op log stays noise-free (test 9). `reason` becomes the
/// `<label|auto-...>` segment of the commit message.
fn snapshot(
    repo: &std::path::Path,
    slug: &str,
    boundary: &[String],
    reason: &str,
    author_name: &str,
    author_email: &str,
) -> Result<Option<String>, String> {
    let snap_ref = stage_snapshot_ref(slug);
    let prev = git_output(repo, &["rev-parse", "--verify", "-q", &snap_ref]).ok();

    let base = prev.clone().unwrap_or_else(|| "HEAD".to_string());
    let new_tree = build_boundary_tree(repo, &base, boundary)?;

    if let Some(prev_sha) = &prev {
        let prev_tree = tree_of(repo, prev_sha)?;
        if new_tree == prev_tree {
            return Ok(None);
        }
    }

    let now = chrono::Utc::now().to_rfc3339();
    let message = format!("snap({slug}): {reason} @ {now}");
    let parent_refs: Vec<&str> = prev.iter().map(String::as_str).collect();
    let new_commit = commit_tree(
        repo,
        &new_tree,
        &parent_refs,
        &message,
        author_name,
        author_email,
    )?;

    let expected = prev.unwrap_or_else(|| "0".repeat(40));
    update_ref_cas(repo, &snap_ref, &new_commit, &expected)?;
    Ok(Some(new_commit))
}

fn stage_status_entries(
    repo: &std::path::Path,
    boundary: &[String],
) -> Result<(Vec<String>, Vec<String>), String> {
    fn tracked(
        repo: &std::path::Path,
        index_path: &std::path::Path,
        pathspec: &[String],
    ) -> Result<Vec<String>, String> {
        let mut args = vec!["diff", "--name-status", "HEAD", "--"];
        args.extend(pathspec.iter().map(String::as_str));
        Ok(git_with_index(repo, index_path, &args)?
            .lines()
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect())
    }

    fn untracked(
        repo: &std::path::Path,
        index_path: &std::path::Path,
        pathspec: &[String],
    ) -> Result<Vec<String>, String> {
        let mut args = vec!["ls-files", "--others", "--exclude-standard", "--"];
        args.extend(pathspec.iter().map(String::as_str));
        Ok(git_with_index(repo, index_path, &args)?
            .lines()
            .filter(|line| !line.is_empty())
            .map(|path| format!("??\t{path}"))
            .collect())
    }

    fn entries(
        repo: &std::path::Path,
        index_path: &std::path::Path,
        pathspec: &[String],
    ) -> Result<Vec<String>, String> {
        let mut result = tracked(repo, index_path, pathspec)?;
        result.extend(untracked(repo, index_path, pathspec)?);
        Ok(result)
    }

    let index_path = tmp_index_path();
    let result = (|| {
        git_with_index(repo, &index_path, &["read-tree", "HEAD"])?;
        let in_boundary = entries(repo, &index_path, boundary)?;
        let out_of_boundary = entries(repo, &index_path, &[])?
            .into_iter()
            .filter(|line| {
                line.rsplit('\t')
                    .next()
                    .is_none_or(|path| !path_in_boundary(path, boundary))
            })
            .collect();
        Ok((in_boundary, out_of_boundary))
    })();
    let _ = std::fs::remove_file(&index_path);
    result
}

/// `stage status <ws> <slug>`: HEAD-vs-worktree changes, partitioned into
/// IN-BOUNDARY vs OUT-OF-BOUNDARY sections through a private HEAD-seeded
/// index, never the shared index.
async fn stage_status(ws: &str, slug: &str, self_instance: Option<&str>) -> ExitCode {
    let boundary = match stage_boundary(ws, slug, self_instance).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let (in_boundary, out_of_boundary) =
        match stage_status_entries(std::path::Path::new("."), &boundary) {
            Ok(entries) => entries,
            Err(e) => {
                eprintln!("conclave: {e}");
                return ExitCode::FAILURE;
            }
        };
    println!("IN-BOUNDARY ({}):", in_boundary.len());
    for l in &in_boundary {
        println!("  {l}");
    }
    println!("OUT-OF-BOUNDARY ({}):", out_of_boundary.len());
    for l in &out_of_boundary {
        println!("  {l}");
    }
    ExitCode::SUCCESS
}

/// `stage diff <ws> <slug>`: `git diff HEAD -- <boundary paths>`.
async fn stage_diff(ws: &str, slug: &str, self_instance: Option<&str>) -> ExitCode {
    let boundary = match stage_boundary(ws, slug, self_instance).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let mut args = vec!["diff", "HEAD", "--"];
    args.extend(boundary.iter().map(String::as_str));
    match Command::new("git").args(&args).status() {
        Ok(s) => ExitCode::from(s.code().unwrap_or(1).rem_euclid(256) as u8),
        Err(e) => {
            eprintln!("conclave: could not run git: {e}");
            ExitCode::from(2)
        }
    }
}

/// `stage commit <ws> <slug> -m <msg>`: decision 2/4/8 mechanics. Auto-snaps
/// first (decision 6, once — not per CAS retry), then runs
/// [`stage_commit_core`] and, on success, posts the ledger stamp (decision
/// 8) over the existing `task note` client path. A ledger-note failure
/// (engine down) warns but does not roll back the already-landed commit.
async fn stage_commit(
    ws: &str,
    slug: &str,
    message: &str,
    self_instance: Option<&str>,
) -> ExitCode {
    let me = match require_stage_self("commit", self_instance) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let boundary = match stage_boundary(ws, slug, self_instance).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let (author_name, author_id) = match agent_identity(ws, me).await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let repo = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("conclave: stage commit: cannot resolve cwd: {e}");
            return ExitCode::from(2);
        }
    };
    let branch = match current_branch(&repo) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("conclave: stage commit: {e}");
            return ExitCode::FAILURE;
        }
    };

    let author_email = format!("{author_id}@agents.conclave.local");
    if let Err(e) = snapshot(
        &repo,
        slug,
        &boundary,
        "auto-pre-commit",
        &author_name,
        &author_email,
    ) {
        eprintln!("conclave: stage commit: auto-snapshot failed: {e}");
        return ExitCode::FAILURE;
    }

    match stage_commit_core(
        &repo,
        &branch,
        slug,
        &boundary,
        message,
        &author_name,
        &author_id,
    ) {
        Ok((sha, n_files)) => {
            let short = &sha[..12.min(sha.len())];
            println!("stage commit: {short} — {message} ({n_files} files)");
            let note_text = format!("stage commit {short} — {message} ({n_files} files)");
            let note_argv = vec![
                "task".to_string(),
                "note".to_string(),
                ws.to_string(),
                slug.to_string(),
                note_text,
            ];
            if let Err(e) = uds_task_call(note_argv, self_instance).await {
                eprintln!("conclave: stage commit: warning — ledger note failed ({e})");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("conclave: stage commit: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `stage snap <ws> <slug> [-m <label>]`: an explicit snapshot (default
/// label "manual" when `-m` is omitted).
async fn stage_snap(
    ws: &str,
    slug: &str,
    label: Option<&str>,
    self_instance: Option<&str>,
) -> ExitCode {
    let me = match require_stage_self("snap", self_instance) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let boundary = match stage_boundary(ws, slug, self_instance).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let (author_name, author_id) = match agent_identity(ws, me).await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let repo = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("conclave: stage snap: cannot resolve cwd: {e}");
            return ExitCode::from(2);
        }
    };
    let reason = label.unwrap_or("manual");
    let author_email = format!("{author_id}@agents.conclave.local");
    match snapshot(&repo, slug, &boundary, reason, &author_name, &author_email) {
        Ok(Some(sha)) => {
            println!("stage snap: {} ({reason})", &sha[..12.min(sha.len())]);
            ExitCode::SUCCESS
        }
        Ok(None) => {
            println!("stage snap: no change since the last snapshot — skipped");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("conclave: stage snap: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `stage log <ws> <slug>`: `git log` over the snapshot ref, newest first.
/// Doesn't need the task boundary (only the slug, to name the ref).
fn stage_log(slug: &str) -> ExitCode {
    let repo = std::path::Path::new(".");
    let snap_ref = stage_snapshot_ref(slug);
    match git_output(
        repo,
        &[
            "log",
            "--format=%h  %ad  %s",
            "--date=iso-strict",
            &snap_ref,
        ],
    ) {
        Ok(out) if !out.is_empty() => {
            println!("{out}");
            ExitCode::SUCCESS
        }
        Ok(_) => {
            println!("stage log: no snapshots for '{slug}' yet");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("conclave: stage log: {e}");
            ExitCode::FAILURE
        }
    }
}

fn validate_stage_restore_source(
    repo: &std::path::Path,
    ws: &str,
    slug: &str,
    snap_sha: &str,
) -> Result<(), String> {
    let snap_ref = stage_snapshot_ref(slug);
    let ref_exists = Command::new("git")
        .current_dir(repo)
        .args(["rev-parse", "--verify", "--quiet", &snap_ref])
        .output()
        .map_err(|e| format!("could not run git rev-parse: {e}"))?;
    if !ref_exists.status.success() {
        return Err(format!(
            "snapshot ref '{snap_ref}' does not exist; run `conclave stage log {ws} {slug}`"
        ));
    }

    let reachable = Command::new("git")
        .current_dir(repo)
        .args(["merge-base", "--is-ancestor", snap_sha, &snap_ref])
        .output()
        .map_err(|e| format!("could not run git merge-base: {e}"))?;
    if !reachable.status.success() {
        return Err(format!(
            "snapshot '{snap_sha}' is not reachable from '{snap_ref}'; \
             run `conclave stage log {ws} {slug}`"
        ));
    }
    Ok(())
}

/// `stage restore <ws> <slug> <snapSha>`: auto-snaps the current state first
/// (decision 6 — so the restore itself is undoable), then `git restore
/// --worktree` from `snapSha`. Never touches the index (no `--staged`).
async fn stage_restore(
    ws: &str,
    slug: &str,
    snap_sha: &str,
    self_instance: Option<&str>,
) -> ExitCode {
    let me = match require_stage_self("restore", self_instance) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let boundary = match stage_boundary(ws, slug, self_instance).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let (author_name, author_id) = match agent_identity(ws, me).await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let repo = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("conclave: stage restore: cannot resolve cwd: {e}");
            return ExitCode::from(2);
        }
    };
    if let Err(e) = validate_stage_restore_source(&repo, ws, slug, snap_sha) {
        eprintln!("conclave: stage restore: {e}");
        return ExitCode::FAILURE;
    }
    let author_email = format!("{author_id}@agents.conclave.local");
    match snapshot(
        &repo,
        slug,
        &boundary,
        "auto-pre-restore",
        &author_name,
        &author_email,
    ) {
        Ok(Some(sha)) => println!(
            "stage restore: auto-snapshotted current state as {}",
            &sha[..12.min(sha.len())]
        ),
        Ok(None) => {}
        Err(e) => {
            eprintln!("conclave: stage restore: auto-snapshot failed: {e}");
            return ExitCode::FAILURE;
        }
    }

    println!(
        "stage restore: restoring {} path(s) from {snap_sha}:",
        boundary.len()
    );
    for p in &boundary {
        println!("  {p}");
    }

    let mut args = vec!["restore", "--worktree", "--source", snap_sha, "--"];
    args.extend(boundary.iter().map(String::as_str));
    match Command::new("git").args(&args).status() {
        Ok(s) if s.success() => {
            println!("stage restore: done");
            ExitCode::SUCCESS
        }
        Ok(s) => {
            eprintln!(
                "conclave: git restore failed (exit {})",
                s.code().unwrap_or(-1)
            );
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("conclave: could not run git: {e}");
            ExitCode::from(2)
        }
    }
}

/// `stage clear <ws> <slug>`: delete the snapshot ref (decision 7 — never
/// auto-deleted by `lane finish`/task close; only this explicit verb).
/// Doesn't need the task boundary, only the slug.
fn stage_clear(slug: &str) -> ExitCode {
    let snap_ref = stage_snapshot_ref(slug);
    match Command::new("git")
        .args(["update-ref", "-d", &snap_ref])
        .output()
    {
        Ok(o) if o.status.success() => {
            println!("stage clear: deleted {snap_ref}");
            ExitCode::SUCCESS
        }
        Ok(o) => {
            eprintln!(
                "conclave: git update-ref -d failed: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            );
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("conclave: could not run git: {e}");
            ExitCode::from(2)
        }
    }
}

/// Dispatch the `stage` subcommand.
async fn run_stage(argv: &[String], self_instance: Option<&str>) -> ExitCode {
    match argv.get(1).map(String::as_str) {
        Some("status") => match (argv.get(2), argv.get(3)) {
            (Some(ws), Some(slug)) => stage_status(ws, slug, self_instance).await,
            _ => {
                eprintln!("conclave: stage status <workspaceId> <slug>");
                ExitCode::from(2)
            }
        },
        Some("diff") => match (argv.get(2), argv.get(3)) {
            (Some(ws), Some(slug)) => stage_diff(ws, slug, self_instance).await,
            _ => {
                eprintln!("conclave: stage diff <workspaceId> <slug>");
                ExitCode::from(2)
            }
        },
        Some("commit") => {
            let m_pos = argv.iter().position(|w| w == "-m");
            let message = m_pos.and_then(|p| argv.get(p + 1));
            match (argv.get(2), argv.get(3), message) {
                (Some(ws), Some(slug), Some(msg)) => {
                    stage_commit(ws, slug, msg, self_instance).await
                }
                _ => {
                    eprintln!("conclave: stage commit <workspaceId> <slug> -m <msg>");
                    ExitCode::from(2)
                }
            }
        }
        Some("snap") => {
            let m_pos = argv.iter().position(|w| w == "-m");
            let label = m_pos.and_then(|p| argv.get(p + 1).map(String::as_str));
            match (argv.get(2), argv.get(3)) {
                (Some(ws), Some(slug)) => stage_snap(ws, slug, label, self_instance).await,
                _ => {
                    eprintln!("conclave: stage snap <workspaceId> <slug> [-m <label>]");
                    ExitCode::from(2)
                }
            }
        }
        Some("log") => match argv.get(3) {
            Some(slug) => stage_log(slug),
            None => {
                eprintln!("conclave: stage log <workspaceId> <slug>");
                ExitCode::from(2)
            }
        },
        Some("restore") => match (argv.get(2), argv.get(3), argv.get(4)) {
            (Some(ws), Some(slug), Some(sha)) => stage_restore(ws, slug, sha, self_instance).await,
            _ => {
                eprintln!("conclave: stage restore <workspaceId> <slug> <snapSha>");
                ExitCode::from(2)
            }
        },
        Some("clear") => match argv.get(3) {
            Some(slug) => stage_clear(slug),
            None => {
                eprintln!("conclave: stage clear <workspaceId> <slug>");
                ExitCode::from(2)
            }
        },
        _ => {
            eprint!("{STAGE_USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Truncate `text` to at most `max_bytes` bytes, keeping the TAIL (the most
/// recent output matters for a gate — the failure is usually at the end).
/// Splits on a UTF-8 boundary via `from_utf8_lossy` rather than requiring one,
/// since a byte-offset cut can land mid-codepoint.
fn tail_bytes(text: &str, max_bytes: usize) -> String {
    let bytes = text.as_bytes();
    if bytes.len() <= max_bytes {
        return text.to_string();
    }
    String::from_utf8_lossy(&bytes[bytes.len() - max_bytes..]).into_owned()
}

/// Gate tail is capped at 2000 bytes (ADR 0008: "truncate tail to 2000 bytes").
const GATE_TAIL_MAX_BYTES: usize = 2000;

/// Quote `word` as a single POSIX shell word: wrapped in single quotes with
/// any embedded `'` rewritten to the standard `'\''` escape (close the quote,
/// emit an escaped literal quote, reopen). Single quotes disable all shell
/// expansion, so the result always reaches `sh` as exactly `word`, one token
/// — this is what makes an argv word survive `sh -lc` verbatim regardless of
/// spaces or other metacharacters inside it.
fn shell_quote_word(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

/// Join `words` into a command line where EVERY word is quoted (see
/// [`shell_quote_word`]) — this is what actually gets executed, so a word
/// containing a space (e.g. a path under "Application Support") reaches `sh`
/// as one token instead of being split back apart.
fn shell_join(words: &[String]) -> String {
    words
        .iter()
        .map(|w| shell_quote_word(w))
        .collect::<Vec<_>>()
        .join(" ")
}

/// True if writing `word` bare into a command line (no quotes) would be
/// tokenized or expanded differently than the literal text. Used only to
/// decide whether the RECORDED `cmd` should show the quoted form — for a
/// space-free, metacharacter-free command like `cargo test` the ledger must
/// stay exactly as human-readable as it is today.
fn word_needs_quoting(word: &str) -> bool {
    word.is_empty()
        || word.chars().any(|c| {
            !(c.is_ascii_alphanumeric()
                || matches!(c, '_' | '-' | '.' | '/' | ':' | '@' | '%' | '+' | ','))
        })
}

/// `task gate <workspaceId> <slug> -- <cmd...>` — ADR 0008's risk ledger is
/// explicit that gates NEVER run engine-side (the command is the CALLING
/// agent's own privilege, no escalation), so `conclave-cli` runs it HERE, in
/// the agent's real shell/cwd, and ships the already-computed evidence over
/// the wire. Requires `CONCLAVE_INSTANCE_ID` (gate results are always
/// attributed) — same requirement as `tell`/`restart`.
///
/// Words after `--` are argv words, passed to the shell VERBATIM (one word
/// each) via [`shell_join`] — never re-joined-then-re-parsed, which is what
/// used to split a space-containing word (e.g. a path under "Application
/// Support") back apart. An agent that wants shell syntax (pipes, `&&`,
/// redirects) composes it explicitly: `-- sh -c "<snippet>"`.
///
/// Returns the fully-expanded wire form
/// `["task","gate",actorId,workspaceId,slug,cmd,exit,sha,cwd,tail]` plus the
/// command's own exit code, bypassing `expand_self_args`'s generic task
/// handling entirely (this function already resolved self). `main` propagates
/// that exit code as its own once the gate is recorded — a red gate must fail
/// loudly for any agent scripting `task gate ... && next`, not hide behind a
/// 0 from `conclave-cli` itself.
fn run_task_gate(
    argv: &[String],
    self_instance: Option<&str>,
) -> Result<(Vec<String>, i32), String> {
    let me = self_instance.filter(|s| !s.is_empty()).ok_or_else(|| {
        "conclave: `task gate` is only available inside a spawned agent (CONCLAVE_INSTANCE_ID unset)"
            .to_string()
    })?;
    let usage = "conclave: task gate <workspaceId> <slug> -- <cmd...> (words after -- are passed verbatim; for shell syntax use -- sh -c \"…\")";
    let workspace_id = argv.get(2).ok_or(usage)?.clone();
    let slug = argv.get(3).ok_or(usage)?.clone();
    let dash_pos = argv
        .iter()
        .position(|w| w == "--")
        .filter(|&p| p >= 4)
        .ok_or(usage)?;
    let cmd_words = &argv[dash_pos + 1..];
    if cmd_words.is_empty() {
        return Err(usage.to_string());
    }
    // Each word after `--` is an argv word passed to the shell verbatim, one
    // token — `exec_cmd` quotes every word so a space (or other
    // metacharacter) inside a word can never be re-split by `sh -lc`. The
    // RECORDED `cmd` stays the plain join for the common space-free case
    // (`cargo test` must keep reading as `cargo test`, not `'cargo' 'test'`);
    // it only switches to the quoted form when some word actually needed it.
    let exec_cmd = shell_join(cmd_words);
    let recorded_cmd = if cmd_words.iter().any(|w| word_needs_quoting(w)) {
        exec_cmd.clone()
    } else {
        cmd_words.join(" ")
    };

    let cwd = std::env::current_dir()
        .map_err(|e| format!("conclave: task gate: cannot resolve cwd: {e}"))?;

    // Combined stdout+stderr, non-streaming, in ONE shell invocation — the
    // `2>&1` redirect merges streams without needing to interleave two async
    // pipes ourselves.
    let output = std::process::Command::new("sh")
        .arg("-lc")
        .arg(format!("{exec_cmd} 2>&1"))
        .current_dir(&cwd)
        .output()
        .map_err(|e| format!("conclave: task gate: failed to run '{exec_cmd}': {e}"))?;
    // A killed-by-signal command has no exit code; -1 records "abnormal
    // termination" rather than fabricating a plausible-looking 0/1.
    let exit_code = output.status.code().unwrap_or(-1);
    let combined = String::from_utf8_lossy(&output.stdout).into_owned();
    let tail = tail_bytes(&combined, GATE_TAIL_MAX_BYTES);

    // Best-effort: a gate run outside a git repo (or a detached/corrupt HEAD)
    // must still record its evidence — "unknown" beats failing the whole gate.
    let sha = std::process::Command::new("git")
        .arg("rev-parse")
        .arg("HEAD")
        .current_dir(&cwd)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    Ok((
        vec![
            "task".to_string(),
            "gate".to_string(),
            me.to_string(),
            workspace_id,
            slug,
            recorded_cmd,
            exit_code.to_string(),
            sha,
            cwd.to_string_lossy().into_owned(),
            tail,
        ],
        exit_code,
    ))
}

// ── uishot (ADR 0008: capture runs caller-side) ────────────────────────────
//
// `conclave uishot [--task <slug>] <args...>` is a thin CLIENT-side wrapper
// over the workspace's `package.json` `uishot` capture script — the platform
// binary never reimplements the capture (no puppeteer/chrome in Rust), it just
// runs the workspace's own script HERE, same reasoning as `task gate` (cwd,
// sandbox, env belong to the agent). `--task` additionally records the run on
// that task's gate ledger by composing a `task gate …` argv and routing it
// through [`run_task_gate`], so the ledger entry is byte-identical to a manual
// gate. The workspace for that gate comes from `CONCLAVE_WORKSPACE_ID` (the
// spawner exports it alongside `CONCLAVE_INSTANCE_ID`, which `--task` already
// requires) or an explicit `--ws <workspaceId>` override (ruling on challenge
// 1241c1dc; plan amended @ 43ef7e3).

/// The convention error when a workspace defines no UI capture contract — the
/// exact wording the spec pins
/// (`docs/superpowers/specs/2026-07-05-uishot-cli-native.md`).
const UISHOT_NO_SCRIPT: &str =
    "conclave: no \"uishot\" script in package.json — this workspace has no UI capture contract";

/// The error when `--task` is used but no workspace can be resolved (neither
/// `CONCLAVE_WORKSPACE_ID` nor `--ws`).
const UISHOT_NO_WS: &str =
    "conclave: uishot --task needs a workspace (CONCLAVE_WORKSPACE_ID unset; pass --ws <workspaceId>)";

const UISHOT_USAGE: &str =
    "Usage: conclave uishot [--task <slug>] [--ws <workspaceId>] <args...>  (runs the workspace's package.json \"uishot\" script here; with --task also records it as a task gate)";

/// A parsed `conclave uishot …` invocation: conclave's own flags peeled off,
/// and the remaining words to forward verbatim to the capture script.
struct UishotInvocation {
    /// `Some(slug)` when `--task <slug>` was given — the run records that
    /// task's gate.
    task: Option<String>,
    /// `--ws <workspaceId>` override for the gate's workspace (else the
    /// `CONCLAVE_WORKSPACE_ID` env is used).
    ws_override: Option<String>,
    /// Everything not a conclave flag — passed to `pnpm run uishot -- <args…>`.
    capture_args: Vec<String>,
}

/// Parse the words AFTER the `uishot` verb. `--task` and `--ws` are conclave's
/// own flags (each consumes the next word, which must not itself look like a
/// flag); `--ws` is only meaningful with `--task`. Every other word is a
/// capture arg forwarded verbatim (the same no-shell-reparse rule as
/// `task gate`), and at least one is required.
fn parse_uishot_args(rest: &[String]) -> Result<UishotInvocation, String> {
    let mut task: Option<String> = None;
    let mut ws_override: Option<String> = None;
    let mut capture_args: Vec<String> = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--task" => {
                let slug = rest
                    .get(i + 1)
                    .filter(|s| !s.starts_with("--"))
                    .ok_or(UISHOT_USAGE)?;
                task = Some(slug.clone());
                i += 2;
            }
            "--ws" => {
                let ws = rest
                    .get(i + 1)
                    .filter(|s| !s.starts_with("--"))
                    .ok_or(UISHOT_USAGE)?;
                ws_override = Some(ws.clone());
                i += 2;
            }
            _ => {
                capture_args.push(rest[i].clone());
                i += 1;
            }
        }
    }
    if ws_override.is_some() && task.is_none() {
        return Err("conclave: uishot --ws requires --task".to_string());
    }
    if capture_args.is_empty() {
        return Err(UISHOT_USAGE.to_string());
    }
    Ok(UishotInvocation {
        task,
        ws_override,
        capture_args,
    })
}

/// True if `package_json` (raw file contents) defines a string `scripts.uishot`.
fn has_uishot_script(package_json: &str) -> bool {
    serde_json::from_str::<Value>(package_json)
        .ok()
        .and_then(|v| {
            v.get("scripts")
                .and_then(|s| s.get("uishot"))
                .map(Value::is_string)
        })
        .unwrap_or(false)
}

/// Confirm `<root>/package.json` defines a `uishot` script, or the convention
/// error ([`UISHOT_NO_SCRIPT`]). Split from the git-root walk so it is
/// unit-testable without a repo.
fn read_uishot_contract(root: &std::path::Path) -> Result<(), String> {
    let contents = std::fs::read_to_string(root.join("package.json"))
        .map_err(|_| UISHOT_NO_SCRIPT.to_string())?;
    if has_uishot_script(&contents) {
        Ok(())
    } else {
        Err(UISHOT_NO_SCRIPT.to_string())
    }
}

/// The git top-level containing `cwd` (same primitive `run_task_gate` uses for
/// the gate SHA), or a user-facing error when `cwd` is not inside a repo.
fn git_toplevel(cwd: &std::path::Path) -> Result<PathBuf, String> {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| format!("conclave: uishot: cannot run git: {e}"))?;
    if !out.status.success() {
        return Err("conclave: uishot: not inside a git repository".to_string());
    }
    Ok(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

/// Compose the `task gate` argv that records a `uishot` run: byte-identical in
/// shape to a manual `conclave task gate <ws> <slug> -- pnpm run uishot -- …`,
/// so the ledger entry (cmd, exit, sha, cwd, tail) is indistinguishable.
fn compose_uishot_gate(ws: &str, slug: &str, capture_args: &[String]) -> Vec<String> {
    let mut gate = vec![
        "task".to_string(),
        "gate".to_string(),
        ws.to_string(),
        slug.to_string(),
        "--".to_string(),
        "pnpm".to_string(),
        "run".to_string(),
        "uishot".to_string(),
        "--".to_string(),
    ];
    gate.extend_from_slice(capture_args);
    gate
}

/// What `main` should do with a `uishot` invocation.
enum UishotPlan {
    /// Bare form: exec `pnpm run uishot -- <args…>` locally, propagate its exit.
    Exec(Vec<String>),
    /// `--task` form: this fully-composed `task gate …` argv flows through the
    /// existing gate machinery (records the ledger entry + propagates exit).
    Gate(Vec<String>),
}

/// Plan a `uishot` invocation (`rest` = argv after the verb). Parses the flags
/// (usage error → exit 2), confirms the workspace defines a capture contract
/// (missing → exit 1), and for `--task` resolves the workspace from `--ws` or
/// `workspace_env` (`CONCLAVE_WORKSPACE_ID`; unset → exit 1). The `u8` in the
/// error is the exit code `main` should use.
fn prepare_uishot(
    rest: &[String],
    cwd: &std::path::Path,
    workspace_env: Option<&str>,
) -> Result<UishotPlan, (String, u8)> {
    let inv = parse_uishot_args(rest).map_err(|e| (e, 2u8))?;
    let root = git_toplevel(cwd).map_err(|e| (e, 1u8))?;
    read_uishot_contract(&root).map_err(|e| (e, 1u8))?;
    match inv.task {
        None => Ok(UishotPlan::Exec(inv.capture_args)),
        Some(slug) => {
            let ws = inv
                .ws_override
                .or_else(|| workspace_env.filter(|s| !s.is_empty()).map(str::to_string))
                .ok_or((UISHOT_NO_WS.to_string(), 1u8))?;
            Ok(UishotPlan::Gate(compose_uishot_gate(
                &ws,
                &slug,
                &inv.capture_args,
            )))
        }
    }
}

/// Exec `pnpm run uishot -- <args…>` in the current cwd, inheriting stdio so
/// the agent sees the capture live, and return the child's own exit code. No
/// shell — args are forwarded verbatim (same rule as `task gate`). A missing
/// `pnpm` surfaces the OS error and the attempted command (never a silent
/// npm/npx fallback).
fn exec_uishot(capture_args: &[String]) -> ExitCode {
    let mut cmd = Command::new("pnpm");
    cmd.arg("run").arg("uishot").arg("--").args(capture_args);
    match cmd.status() {
        Ok(status) => ExitCode::from(status.code().unwrap_or(-1).rem_euclid(256) as u8),
        Err(e) => {
            eprintln!(
                "conclave: uishot: failed to run 'pnpm run uishot -- …': {e} (is pnpm on PATH?)"
            );
            ExitCode::from(127)
        }
    }
}

/// `task create … --plan-file <path>` — reads `<path>` relative to the
/// CALLING agent's cwd (only `conclave-cli`, not the engine, knows that cwd)
/// and rewrites the flag to `--plan <contents>` before the request is sent.
/// A no-op when `--plan-file` is absent.
fn resolve_task_create_plan_file(argv: Vec<String>) -> Result<Vec<String>, String> {
    let Some(pos) = argv.iter().position(|w| w == "--plan-file") else {
        return Ok(argv);
    };
    let path = argv
        .get(pos + 1)
        .ok_or("conclave: task create: --plan-file requires a path")?
        .clone();
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("conclave: task create: cannot read --plan-file '{path}': {e}"))?;
    let mut out = argv;
    out[pos] = "--plan".to_string();
    out[pos + 1] = content;
    Ok(out)
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
    /// Just the new artifact's id (`artifact add`) — the caller scripts on it.
    ArtifactId,
    /// A newest-first table of artifacts (`artifact list`), content omitted.
    ArtifactList,
    /// One artifact's metadata header + full content (`artifact get`).
    ArtifactGet,
    /// The updated position row (`position set`) — one line: name · track · level · reports-to.
    PositionRow,
    /// The workspace supervisor forest (`org`) as an indented tree.
    OrgTree,
    /// A chronological inter-agent transcript (`msg list` / `msg all`).
    MsgList,
    /// A human-readable task resume packet (`task brief`).
    TaskBrief,
    /// Pretty-printed JSON (everything else).
    Json,
}

/// A party's display label for the transcript: the enriched `<key>Name` when
/// present, else a short (8-char) prefix of the raw instance id, else `?`.
fn msg_party(row: &Value, name_key: &str, id_key: &str) -> String {
    if let Some(name) = row.get(name_key).and_then(Value::as_str) {
        return name.to_string();
    }
    let id = row.get(id_key).and_then(Value::as_str).unwrap_or("?");
    id.get(..8).unwrap_or(id).to_string()
}

/// Render inter-agent messages as a chronological transcript. Rows arrive
/// newest-first (the DB's DESC order); reverse them so the output reads like a
/// conversation. One token-cheap line per message:
/// `HH:MM  From → To  text` with a ` [queued]` marker on undelivered rows and a
/// name-or-short-id fallback per party. An empty array → a single
/// `(no messages)` line so an agent with no history sees a clean marker, not a
/// bare `[]`.
///
/// `HH:MM` is sliced from the RFC3339 `createdAt` (chars 11..16) — deterministic
/// regardless of the reader's local timezone.
fn render_msg_transcript(rows: &[Value]) -> String {
    if rows.is_empty() {
        return "(no messages)\n".to_string();
    }
    let mut out = String::new();
    for row in rows.iter().rev() {
        let time = row
            .get("createdAt")
            .and_then(Value::as_str)
            .and_then(|ts| ts.get(11..16))
            .unwrap_or("--:--");
        let from = msg_party(row, "fromName", "fromInstanceId");
        let to = msg_party(row, "toName", "toInstanceId");
        let text = row.get("text").and_then(Value::as_str).unwrap_or("");
        let queued = if row.get("status").and_then(Value::as_str) == Some("queued") {
            " [queued]"
        } else {
            ""
        };
        out.push_str(&format!("{time}  {from} → {to}  {text}{queued}\n"));
    }
    out
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

fn render_task_brief_event(row: &Value) -> String {
    let id = row.get("id").and_then(Value::as_str).unwrap_or("-");
    let kind = row.get("kind").and_then(Value::as_str).unwrap_or("-");
    let created = row.get("createdAt").and_then(Value::as_str).unwrap_or("-");
    let short = short_id(id);
    let payload = row.get("payload").and_then(Value::as_object);
    match kind {
        "note" => format!(
            "{short}  note  {created}  {}",
            payload
                .and_then(|p| p.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("")
        ),
        "gate" => format!(
            "{short}  gate  {created}  {}  exit={}  sha={}",
            payload
                .and_then(|p| p.get("cmd"))
                .and_then(Value::as_str)
                .unwrap_or(""),
            payload
                .and_then(|p| p.get("exit"))
                .and_then(Value::as_i64)
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string()),
            payload
                .and_then(|p| p.get("sha"))
                .and_then(Value::as_str)
                .map(short_id)
                .unwrap_or("-")
        ),
        "challenge" => format!(
            "{short}  challenge  {created}  {}",
            payload
                .and_then(|p| p.get("claim"))
                .and_then(Value::as_str)
                .unwrap_or("")
        ),
        "ruling" => format!(
            "{short}  ruling  {created}  {}",
            payload
                .and_then(|p| p.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("")
        ),
        "state" => format!(
            "{short}  state  {created}  {} -> {}",
            payload
                .and_then(|p| p.get("from"))
                .and_then(Value::as_str)
                .unwrap_or(""),
            payload
                .and_then(|p| p.get("to"))
                .and_then(Value::as_str)
                .unwrap_or("")
        ),
        _ => format!("{short}  {kind}  {created}"),
    }
}

fn render_task_brief(result: &Value) -> String {
    let empty_task = serde_json::Map::new();
    let task = result
        .get("task")
        .and_then(Value::as_object)
        .unwrap_or(&empty_task);
    let field = |key: &str| task.get(key).and_then(Value::as_str).unwrap_or("-");
    let mut out = String::new();

    out.push_str(&format!(
        "Task brief: {}  {}\n",
        field("slug"),
        field("title")
    ));
    out.push_str(&format!("state: {}\n", field("state")));
    if let Some(owner) = task.get("ownerAgentId").and_then(Value::as_str) {
        out.push_str(&format!("owner: {owner}\n"));
    }
    if let Some(implementer) = task.get("implementerAgentId").and_then(Value::as_str) {
        out.push_str(&format!("implementer: {implementer}\n"));
    }
    if let Some(canon) = task.get("designCanon").and_then(Value::as_str) {
        out.push_str(&format!("design canon: {canon}\n"));
    }
    if let Some(limit) = result.get("limit").and_then(Value::as_u64) {
        out.push_str(&format!("limit: {limit}\n"));
    }

    out.push_str("file boundary:\n");
    let empty_boundary: Vec<Value> = Vec::new();
    for entry in task
        .get("fileBoundary")
        .and_then(Value::as_array)
        .unwrap_or(&empty_boundary)
    {
        out.push_str("  - ");
        out.push_str(entry.as_str().unwrap_or("-"));
        out.push('\n');
    }

    out.push_str("plan excerpt:\n");
    match task.get("planExcerpt").and_then(Value::as_str) {
        Some(plan) if !plan.is_empty() => {
            for line in plan.lines() {
                out.push_str("  ");
                out.push_str(line);
                out.push('\n');
            }
            if task.get("planTruncated").and_then(Value::as_bool) == Some(true) {
                out.push_str("  … truncated\n");
            }
        }
        _ => out.push_str("  (empty)\n"),
    }

    out.push_str(&format!(
        "open challenges ({}):\n",
        result
            .get("openChallenges")
            .and_then(Value::as_array)
            .map(|rows| rows.len())
            .unwrap_or(0)
    ));
    let empty_challenges: Vec<Value> = Vec::new();
    for challenge in result
        .get("openChallenges")
        .and_then(Value::as_array)
        .unwrap_or(&empty_challenges)
    {
        let id = challenge.get("id").and_then(Value::as_str).unwrap_or("-");
        let claim = challenge
            .get("claim")
            .and_then(Value::as_str)
            .unwrap_or("-");
        let deadline = challenge
            .get("deadlineAt")
            .and_then(Value::as_str)
            .map(|d| format!("  deadline {d}"))
            .unwrap_or_default();
        out.push_str(&format!("  - {}  {}{}\n", short_id(id), claim, deadline));
    }

    out.push_str(&format!(
        "latest gates ({}):\n",
        result
            .get("latestGates")
            .and_then(Value::as_array)
            .map(|rows| rows.len())
            .unwrap_or(0)
    ));
    let empty_gates: Vec<Value> = Vec::new();
    for gate in result
        .get("latestGates")
        .and_then(Value::as_array)
        .unwrap_or(&empty_gates)
    {
        let id = gate.get("id").and_then(Value::as_str).unwrap_or("-");
        let cmd = gate.get("cmd").and_then(Value::as_str).unwrap_or("-");
        let exit = gate
            .get("exit")
            .and_then(Value::as_i64)
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string());
        let sha = gate
            .get("sha")
            .and_then(Value::as_str)
            .map(short_id)
            .unwrap_or("-");
        out.push_str(&format!(
            "  - {}  {}  exit={}  sha={}\n",
            short_id(id),
            cmd,
            exit,
            sha
        ));
    }

    out.push_str(&format!(
        "last events ({}):\n",
        result
            .get("lastEvents")
            .and_then(Value::as_array)
            .map(|rows| rows.len())
            .unwrap_or(0)
    ));
    let empty_events: Vec<Value> = Vec::new();
    for event in result
        .get("lastEvents")
        .and_then(Value::as_array)
        .unwrap_or(&empty_events)
    {
        out.push_str("  - ");
        out.push_str(&render_task_brief_event(event));
        out.push('\n');
    }

    match result.get("memoryError").and_then(Value::as_str) {
        Some(err) => {
            out.push_str(&format!("memory: unavailable ({err})\n"));
        }
        None => {
            out.push_str(&format!(
                "memory hits ({}):\n",
                result
                    .get("memoryHits")
                    .and_then(Value::as_array)
                    .map(|rows| rows.len())
                    .unwrap_or(0)
            ));
            let empty_hits: Vec<Value> = Vec::new();
            for hit in result
                .get("memoryHits")
                .and_then(Value::as_array)
                .unwrap_or(&empty_hits)
            {
                let id = hit.get("id").and_then(Value::as_str).unwrap_or("-");
                let text = hit.get("text").and_then(Value::as_str).unwrap_or("-");
                let score = hit
                    .get("score")
                    .and_then(Value::as_f64)
                    .map(|v| format!("{v:.3}"))
                    .unwrap_or_else(|| "-".to_string());
                let source_kind = hit.get("sourceKind").and_then(Value::as_str).unwrap_or("-");
                let source_id = hit
                    .get("sourceId")
                    .and_then(Value::as_str)
                    .map(short_id)
                    .unwrap_or("-");
                out.push_str(&format!(
                    "  - {}  score={}  {}  ({source_kind}:{source_id})\n",
                    short_id(id),
                    score,
                    text
                ));
            }
        }
    }

    out
}

/// Render `position set`'s updated row as one line: name · track · level ·
/// reports-to. Absent fields show `-`; a cleared/absent supervisor shows
/// `(human)`.
fn render_position_row(row: &Value) -> String {
    let f = |k: &str| row.get(k).and_then(Value::as_str).unwrap_or("-");
    let reports_to = row
        .get("supervisorName")
        .and_then(Value::as_str)
        .unwrap_or("(human)");
    format!(
        "{} · {} · {} · reports to {}",
        f("name"),
        f("roleName"),
        f("level"),
        reports_to
    )
}

/// Depth-first print of one parent's reports (spec §5.3 / Q5: the tree is
/// derived client-side from the flat roster). `visited` breaks any corrupt
/// cycle; each bucket is pre-sorted by (name, id) for a stable rendering.
fn org_walk(
    parent_id: &str,
    depth: usize,
    children: &std::collections::HashMap<String, Vec<&Value>>,
    visited: &mut std::collections::HashSet<String>,
    out: &mut String,
) {
    let Some(kids) = children.get(parent_id) else {
        return;
    };
    for r in kids {
        let id = r
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if !visited.insert(id.clone()) {
            continue; // cycle guard — never recurse into an already-seen node
        }
        let name = r.get("name").and_then(Value::as_str).unwrap_or("(unnamed)");
        let track = r.get("roleName").and_then(Value::as_str).unwrap_or("-");
        let level = r.get("level").and_then(Value::as_str).unwrap_or("-");
        let working = if r.get("working").and_then(Value::as_bool) == Some(true) {
            "working"
        } else {
            "idle"
        };
        let indent = "  ".repeat(depth + 1);
        out.push_str(&format!("{indent}{name} · {track} · {level} · {working}\n"));
        org_walk(&id, depth + 1, children, visited, out);
    }
}

/// Render the workspace supervisor forest as an indented tree from the flat
/// roster. Roots = agents whose `supervisorAgentId` is absent, under an
/// implicit `(human)` top line; each node shows name · track · level ·
/// working|idle.
fn render_org_tree(rows: &[Value]) -> String {
    let mut children: std::collections::HashMap<String, Vec<&Value>> =
        std::collections::HashMap::new();
    for r in rows {
        let parent = r
            .get("supervisorAgentId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        children.entry(parent).or_default().push(r);
    }
    let name_of = |r: &Value| {
        r.get("name")
            .and_then(Value::as_str)
            .unwrap_or("(unnamed)")
            .to_string()
    };
    let id_of = |r: &Value| {
        r.get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    for bucket in children.values_mut() {
        bucket.sort_by(|a, b| {
            name_of(a)
                .cmp(&name_of(b))
                .then_with(|| id_of(a).cmp(&id_of(b)))
        });
    }

    let mut out = String::from("(human)\n");
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    org_walk("", 0, &children, &mut visited, &mut out);
    out
}

/// Return the workspace for public CLI task exits that should print the
/// memory-save nudge after a successful response.
fn memory_reminder_workspace_id(argv: &[String]) -> Option<String> {
    match argv {
        [task, close, workspace_id, _slug] if task == "task" && close == "close" => {
            Some(workspace_id.clone())
        }
        [task, state_verb, workspace_id, _slug, state]
            if task == "task"
                && state_verb == "state"
                && (state == "review" || state == "abandoned") =>
        {
            Some(workspace_id.clone())
        }
        _ => None,
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();

    // Empty invocation or explicit help request — print usage and exit cleanly.
    if argv.is_empty() || argv[0] == "help" || argv[0] == "--help" || argv[0] == "-h" {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    let self_instance = std::env::var("CONCLAVE_INSTANCE_ID").ok();

    // `lane` is handled locally (git worktree lifecycle + hook install), not as
    // a single `cli.exec` — dispatch it before the socket machinery below.
    if argv[0] == "lane" {
        return run_lane(&argv, self_instance.as_deref()).await;
    }

    // `stage` is handled locally too (private-index git plumbing + a UDS
    // round-trip only to read the task's boundary/roster) — dispatch before
    // the generic `cli.exec` machinery below, same as `lane`.
    if argv[0] == "stage" {
        return run_stage(&argv, self_instance.as_deref()).await;
    }

    // `uishot` — client-side wrapper over the workspace's `package.json`
    // "uishot" capture script (ADR 0008: capture runs caller-side, never
    // engine-side, same as `task gate`). The bare form execs it and returns
    // the child's exit; `--task <slug>` rewrites argv into the composed
    // `task gate …` form below so the run is recorded byte-identically to a
    // manual gate (workspace from `CONCLAVE_WORKSPACE_ID` or `--ws`).
    let argv = if argv.first().map(String::as_str) == Some("uishot") {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let workspace_env = std::env::var("CONCLAVE_WORKSPACE_ID").ok();
        match prepare_uishot(&argv[1..], &cwd, workspace_env.as_deref()) {
            Ok(UishotPlan::Exec(capture_args)) => return exec_uishot(&capture_args),
            Ok(UishotPlan::Gate(gate_argv)) => gate_argv,
            Err((e, code)) => {
                eprintln!("{e}");
                return ExitCode::from(code);
            }
        }
    } else {
        argv
    };

    // `task gate` needs to both run the command HERE (never engine-side) and
    // later propagate its exit code as `conclave-cli`'s own; `task create
    // --plan-file` needs the CALLING agent's real cwd to resolve a relative
    // path. Both run before any other expansion, since they can change
    // argv's shape/length. `gate_exit_code` is `Some` only for `task gate` —
    // its value is what `main` exits with once the request completes.
    let mut gate_exit_code: Option<i32> = None;
    let argv = if argv.first().map(String::as_str) == Some("task")
        && argv.get(1).map(String::as_str) == Some("gate")
    {
        match run_task_gate(&argv, self_instance.as_deref()) {
            Ok((a, exit_code)) => {
                gate_exit_code = Some(exit_code);
                a
            }
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::from(2);
            }
        }
    } else if argv.first().map(String::as_str) == Some("task")
        && argv.get(1).map(String::as_str) == Some("create")
    {
        match resolve_task_create_plan_file(argv) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::from(2);
            }
        }
    } else {
        argv
    };

    // `artifact add --file <path>` reads the file at the CLIENT's cwd (the
    // engine may be sandboxed and could not see it) and rewrites it to
    // `--content <text>` before any further expansion — same client-side
    // resolution as `task create --plan-file`.
    let argv = match resolve_artifact_add_file(argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };

    // Capture the public CLI form before self-keyed task verbs gain their
    // actor-id wire slot. The reminder still prints only after a successful
    // response below.
    let task_boundary_workspace_id = memory_reminder_workspace_id(&argv);

    // Expand the self-keyed forms (`tell`, `snapshot save`, `snapshot last`,
    // `task claim`/…) to their wire form, filling the instance id from
    // CONCLAVE_INSTANCE_ID (set on spawned agents).
    let argv = match expand_self_args(argv, self_instance.as_deref()) {
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
    } else if argv.first().map(String::as_str) == Some("artifact") {
        // `add` prints just the id; `list` a content-free table; `get` a
        // metadata header + the full body. (See the render match below.)
        match sub {
            Some("add") => OutMode::ArtifactId,
            Some("list") => OutMode::ArtifactList,
            Some("get") => OutMode::ArtifactGet,
            _ => OutMode::Json,
        }
    } else if argv.first().map(String::as_str) == Some("position") && sub == Some("set") {
        OutMode::PositionRow
    } else if argv.first().map(String::as_str) == Some("org") {
        OutMode::OrgTree
    } else if argv.first().map(String::as_str) == Some("msg") {
        OutMode::MsgList
    } else if argv.first().map(String::as_str) == Some("task") && sub == Some("brief") {
        OutMode::TaskBrief
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
            // `artifact add` → the new id, so a caller can `id=$(conclave
            // artifact add …)` and reference it.
            OutMode::ArtifactId => {
                let id = result.get("id").and_then(Value::as_str).unwrap_or("");
                println!("{id}");
            }
            // `artifact list` → one line per artifact, newest first, WITHOUT the
            // body (the plan's "no content dump"). Empty workspace → a marker.
            OutMode::ArtifactList => {
                let empty: Vec<Value> = Vec::new();
                let rows = result.as_array().unwrap_or(&empty);
                if rows.is_empty() {
                    println!("(no artifacts)");
                } else {
                    for row in rows {
                        let id = row.get("id").and_then(Value::as_str).unwrap_or("");
                        let short = id.get(..8).unwrap_or(id);
                        let kind = row.get("kind").and_then(Value::as_str).unwrap_or("-");
                        let title = row.get("title").and_then(Value::as_str).unwrap_or("-");
                        let agent = row.get("agentId").and_then(Value::as_str).unwrap_or("-");
                        let created = row.get("createdAt").and_then(Value::as_str).unwrap_or("-");
                        println!("{short}  {kind:<8}  {title}  ({agent}, {created})");
                    }
                }
            }
            // `artifact get` → a short metadata header, a blank line, then the
            // full body verbatim to stdout.
            OutMode::ArtifactGet => {
                let field = |k: &str| result.get(k).and_then(Value::as_str).unwrap_or("-");
                println!("id:       {}", field("id"));
                println!("title:    {}", field("title"));
                println!("kind:     {}", field("kind"));
                println!("agent:    {}", field("agentId"));
                if let Some(f) = result.get("filename").and_then(Value::as_str) {
                    println!("filename: {f}");
                }
                println!("created:  {}", field("createdAt"));
                println!();
                println!(
                    "{}",
                    result.get("content").and_then(Value::as_str).unwrap_or("")
                );
            }
            // `position set` → one line describing the updated agent.
            OutMode::PositionRow => {
                println!("{}", render_position_row(result));
            }
            // `org` → the workspace supervisor forest as an indented tree.
            OutMode::OrgTree => {
                let empty: Vec<Value> = Vec::new();
                let rows = result.as_array().unwrap_or(&empty);
                print!("{}", render_org_tree(rows));
            }
            // `msg list` / `msg all` → a chronological transcript, not the JSON
            // array of UUID-keyed rows (which defeats the point of a re-read).
            OutMode::MsgList => {
                let empty: Vec<Value> = Vec::new();
                let rows = result.as_array().unwrap_or(&empty);
                print!("{}", render_msg_transcript(rows));
            }
            OutMode::TaskBrief => {
                print!("{}", render_task_brief(result));
            }
            OutMode::Json => {
                let pretty =
                    serde_json::to_string_pretty(result).expect("serialize result cannot fail");
                println!("{pretty}");
            }
        }
        if let Some(ws) = &task_boundary_workspace_id {
            println!(
                "\nBoundary reached — what did this cost to learn that the repo doesn't record? \
                 `conclave memory remember {ws} ...`"
            );
        }
        // `task gate` propagates the RECORDED command's own exit code, not a
        // blanket 0 — a red gate must fail loudly for `task gate ... && next`
        // scripting, now that the evidence is durably recorded server-side.
        if let Some(code) = gate_exit_code {
            return ExitCode::from(code.rem_euclid(256) as u8);
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
    use std::path::{Path, PathBuf};

    fn v(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    // ── uishot (ADR 0008: caller-side capture wrapper) ─────────────────────

    #[test]
    fn uishot_parse_bare_forwards_all_args() {
        let inv = super::parse_uishot_args(&v(&["home", "--scenario", "empty"])).unwrap();
        assert!(inv.task.is_none());
        assert!(inv.ws_override.is_none());
        assert_eq!(inv.capture_args, v(&["home", "--scenario", "empty"]));
    }

    #[test]
    fn uishot_parse_task_peels_slug_keeps_capture() {
        let inv = super::parse_uishot_args(&v(&["--task", "t1", "home", "--full"])).unwrap();
        assert_eq!(inv.task.as_deref(), Some("t1"));
        assert!(inv.ws_override.is_none());
        assert_eq!(inv.capture_args, v(&["home", "--full"]));
    }

    #[test]
    fn uishot_parse_ws_override_and_task() {
        let inv = super::parse_uishot_args(&v(&["--task", "t1", "--ws", "ws9", "home"])).unwrap();
        assert_eq!(inv.task.as_deref(), Some("t1"));
        assert_eq!(inv.ws_override.as_deref(), Some("ws9"));
        assert_eq!(inv.capture_args, v(&["home"]));
    }

    #[test]
    fn uishot_parse_usage_errors() {
        // no args at all → usage error (exit 2 upstream)
        assert!(super::parse_uishot_args(&v(&[])).is_err());
        // --task consumed the only word, no capture arg left
        assert!(super::parse_uishot_args(&v(&["--task", "t1"])).is_err());
        // --task with no following word
        assert!(super::parse_uishot_args(&v(&["--task"])).is_err());
        // --task's value must not itself look like a flag (would swallow it)
        assert!(super::parse_uishot_args(&v(&["--task", "--ws", "home"])).is_err());
        // --ws without --task is meaningless
        assert!(super::parse_uishot_args(&v(&["--ws", "ws9", "home"])).is_err());
    }

    #[test]
    fn uishot_has_script_detects_string_entry_only() {
        assert!(super::has_uishot_script(
            r#"{"scripts":{"uishot":"node x.mjs"}}"#
        ));
        assert!(!super::has_uishot_script(r#"{"scripts":{"build":"x"}}"#));
        assert!(!super::has_uishot_script(r#"{"scripts":{}}"#));
        assert!(!super::has_uishot_script(r#"{}"#));
        assert!(!super::has_uishot_script("not json at all"));
        // present but not a string → not a runnable script
        assert!(!super::has_uishot_script(r#"{"scripts":{"uishot":123}}"#));
    }

    #[test]
    fn uishot_read_contract_found_and_missing() {
        let base = std::env::temp_dir().join(format!("conclave-uishot-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        // no package.json at all → convention error
        assert!(super::read_uishot_contract(&base).is_err());
        // present but no uishot script → convention error
        std::fs::write(base.join("package.json"), r#"{"scripts":{"build":"x"}}"#).unwrap();
        assert!(super::read_uishot_contract(&base).is_err());
        // present with the script → ok
        std::fs::write(
            base.join("package.json"),
            r#"{"scripts":{"uishot":"node scripts/uishot.mjs"}}"#,
        )
        .unwrap();
        assert!(super::read_uishot_contract(&base).is_ok());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn uishot_compose_gate_is_manual_gate_shape() {
        assert_eq!(
            super::compose_uishot_gate("ws1", "t1", &v(&["home", "--full"])),
            v(&[
                "task", "gate", "ws1", "t1", "--", "pnpm", "run", "uishot", "--", "home", "--full",
            ]),
        );
    }

    #[test]
    fn uishot_prepare_task_resolves_workspace_from_env() {
        // cargo test runs with cwd = the crate dir (src-tauri), which IS inside
        // this repo and whose root package.json defines a `uishot` script.
        let cwd = std::env::current_dir().unwrap();
        match super::prepare_uishot(&v(&["--task", "t1", "home"]), &cwd, Some("ws-env")) {
            Ok(super::UishotPlan::Gate(argv)) => {
                assert_eq!(
                    argv,
                    super::compose_uishot_gate("ws-env", "t1", &v(&["home"]))
                );
            }
            _ => panic!("expected a Gate plan"),
        }
    }

    #[test]
    fn uishot_prepare_ws_override_beats_env() {
        let cwd = std::env::current_dir().unwrap();
        match super::prepare_uishot(
            &v(&["--task", "t1", "--ws", "ws-flag", "home"]),
            &cwd,
            Some("ws-env"),
        ) {
            Ok(super::UishotPlan::Gate(argv)) => {
                assert_eq!(argv[2], "ws-flag");
            }
            _ => panic!("expected a Gate plan using the --ws override"),
        }
    }

    #[test]
    fn uishot_prepare_task_without_workspace_errors_exit1() {
        let cwd = std::env::current_dir().unwrap();
        match super::prepare_uishot(&v(&["--task", "t1", "home"]), &cwd, None) {
            Err((msg, code)) => {
                assert_eq!(code, 1);
                assert!(msg.contains("needs a workspace"), "unexpected: {msg}");
            }
            Ok(_) => panic!("expected a workspace-precondition error"),
        }
    }

    #[test]
    fn uishot_prepare_bare_execs_forwarding_args() {
        let cwd = std::env::current_dir().unwrap();
        match super::prepare_uishot(&v(&["home"]), &cwd, None) {
            Ok(super::UishotPlan::Exec(args)) => assert_eq!(args, v(&["home"])),
            _ => panic!("expected an Exec plan"),
        }
    }

    #[test]
    fn uishot_prepare_missing_contract_errors_exit1() {
        // a throwaway dir that is not inside any git repo → resolution fails
        let base = std::env::temp_dir().join(format!("conclave-uishot-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        match super::prepare_uishot(&v(&["home"]), &base, None) {
            Err((_, code)) => assert_eq!(code, 1),
            Ok(_) => panic!("expected an exit-1 resolution error"),
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn uishot_gate_requires_instance_id() {
        // The --task path routes the composed argv through run_task_gate, which
        // (like `task gate`) refuses to run without CONCLAVE_INSTANCE_ID — the
        // precondition error fires BEFORE any command runs.
        let gate = super::compose_uishot_gate("ws1", "t1", &v(&["home"]));
        assert!(super::run_task_gate(&gate, None).is_err());
        assert!(super::run_task_gate(&gate, Some("")).is_err());
    }

    #[test]
    fn memory_reminder_detection_matches_exit_trigger_matrix() {
        for (argv, expected) in [
            (v(&["task", "close", "ws1", "t1"]), Some("ws1")),
            (v(&["task", "state", "ws1", "t1", "review"]), Some("ws1")),
            (v(&["task", "state", "ws1", "t1", "abandoned"]), Some("ws1")),
            (v(&["task", "state", "ws1", "t1", "in_progress"]), None),
            (v(&["task", "state", "ws1", "t1", "merged"]), None),
            (v(&["task", "note", "ws1", "t1", "review"]), None),
            (v(&["memory", "remember", "ws1", "review"]), None),
            (v(&[]), None),
            (v(&["task"]), None),
            (v(&["task", "close"]), None),
            (v(&["task", "close", "ws1"]), None),
            (v(&["task", "state"]), None),
            (v(&["task", "state", "ws1"]), None),
            (v(&["task", "state", "ws1", "t1"]), None),
        ] {
            assert_eq!(
                super::memory_reminder_workspace_id(&argv),
                expected.map(str::to_string),
                "unexpected reminder detection for {argv:?}"
            );
        }
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
        let out = expand_self_args(
            v(&["memory", "remember", "ws1", "hi", "there"]),
            Some("self1"),
        )
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

    // ── memory review queue (plan memory-distill-queue) ────────────────────

    #[test]
    fn memory_propose_approve_reject_inject_self_agent_id() {
        let propose =
            expand_self_args(v(&["memory", "propose", "ws1", "a", "fact"]), Some("self1")).unwrap();
        assert_eq!(
            propose,
            v(&["memory", "propose", "self1", "ws1", "a", "fact"])
        );
        let approve =
            expand_self_args(v(&["memory", "approve", "ws1", "p1"]), Some("self1")).unwrap();
        assert_eq!(approve, v(&["memory", "approve", "self1", "ws1", "p1"]));
        let reject =
            expand_self_args(v(&["memory", "reject", "ws1", "p1"]), Some("self1")).unwrap();
        assert_eq!(reject, v(&["memory", "reject", "self1", "ws1", "p1"]));
    }

    #[test]
    fn memory_propose_approve_reject_require_a_spawned_agent() {
        // Unlike `remember`, these have no "-" sentinel: the gate needs a real
        // agent id, so they error outside a spawned-agent context.
        assert!(expand_self_args(v(&["memory", "propose", "ws1", "x"]), None).is_err());
        assert!(expand_self_args(v(&["memory", "approve", "ws1", "p1"]), Some("")).is_err());
        assert!(expand_self_args(v(&["memory", "reject", "ws1", "p1"]), None).is_err());
    }

    #[test]
    fn memory_queue_passes_through_untouched() {
        let queue = v(&["memory", "queue", "ws1", "--state", "pending"]);
        assert_eq!(
            expand_self_args(queue.clone(), Some("self1")).unwrap(),
            queue
        );
        assert_eq!(expand_self_args(queue.clone(), None).unwrap(), queue);
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
        for bad in [
            "", "a/b", "..", "../x", "-x", ".hidden", "a b", "a.b", "a\tb",
        ] {
            assert!(
                validate_slug(bad).is_err(),
                "expected '{bad}' to be rejected"
            );
        }
    }

    // ── task (ADR 0008) ──────────────────────────────────────────────────────

    #[test]
    fn task_claim_injects_actor_from_env() {
        let out = expand_self_args(v(&["task", "claim", "ws1", "t1"]), Some("self1")).unwrap();
        assert_eq!(out, v(&["task", "claim", "self1", "ws1", "t1"]));
    }

    #[test]
    fn task_claim_without_instance_id_errors() {
        assert!(expand_self_args(v(&["task", "claim", "ws1", "t1"]), None).is_err());
        assert!(expand_self_args(v(&["task", "claim", "ws1", "t1"]), Some("")).is_err());
    }

    #[test]
    fn task_note_injects_actor_and_keeps_text() {
        let out = expand_self_args(
            v(&["task", "note", "ws1", "t1", "hello", "world"]),
            Some("self1"),
        )
        .unwrap();
        assert_eq!(
            out,
            v(&["task", "note", "self1", "ws1", "t1", "hello", "world"])
        );
    }

    #[test]
    fn task_state_note_gate_challenge_rule_close_watch_unwatch_all_require_self() {
        for verb in [
            "state",
            "note",
            "challenge",
            "rule",
            "close",
            "watch",
            "unwatch",
        ] {
            assert!(
                expand_self_args(v(&["task", verb, "ws1", "t1"]), None).is_err(),
                "task {verb} must require self"
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

    #[test]
    fn task_list_and_get_pass_through_untouched() {
        let list = v(&["task", "list", "ws1"]);
        assert_eq!(expand_self_args(list.clone(), None).unwrap(), list);
        let get = v(&["task", "get", "ws1", "t1"]);
        assert_eq!(expand_self_args(get.clone(), None).unwrap(), get);
    }

    #[test]
    fn task_create_defaults_owner_to_self_when_absent() {
        let out =
            expand_self_args(v(&["task", "create", "ws1", "t1", "Title"]), Some("self1")).unwrap();
        assert_eq!(
            out,
            v(&["task", "create", "ws1", "t1", "Title", "--owner", "self1"])
        );
    }

    #[test]
    fn task_create_leaves_explicit_owner_untouched() {
        let argv = v(&["task", "create", "ws1", "t1", "Title", "--owner", "other"]);
        let out = expand_self_args(argv.clone(), Some("self1")).unwrap();
        assert_eq!(out, argv, "--owner already present must not be overridden");
    }

    #[test]
    fn task_create_without_self_and_without_owner_is_left_ownerless() {
        let argv = v(&["task", "create", "ws1", "t1", "Title"]);
        let out = expand_self_args(argv.clone(), None).unwrap();
        assert_eq!(out, argv, "create must not require self");
    }

    #[test]
    fn task_gate_passes_through_expand_self_args_untouched() {
        // run_task_gate (called earlier in main(), before expand_self_args) is
        // what actually resolves self for `gate` — by the time it reaches
        // expand_self_args the actorId slot is already filled.
        let argv = v(&[
            "task", "gate", "self1", "ws1", "t1", "cmd", "0", "sha", "/cwd", "tail",
        ]);
        assert_eq!(expand_self_args(argv.clone(), None).unwrap(), argv);
    }

    // ── tail_bytes ────────────────────────────────────────────────────────

    #[test]
    fn tail_bytes_keeps_whole_string_under_the_cap() {
        assert_eq!(super::tail_bytes("short", 2000), "short");
    }

    #[test]
    fn tail_bytes_truncates_to_the_tail_over_the_cap() {
        let long = "a".repeat(10) + "END";
        let truncated = super::tail_bytes(&long, 5);
        assert_eq!(truncated, "aaEND");
        assert_eq!(truncated.len(), 5);
    }

    // ── shell_quote_word / shell_join ───────────────────────────────────────

    #[test]
    fn shell_quote_word_wraps_a_plain_word_in_single_quotes() {
        assert_eq!(super::shell_quote_word("cargo"), "'cargo'");
    }

    #[test]
    fn shell_quote_word_keeps_a_space_containing_word_intact() {
        assert_eq!(
            super::shell_quote_word("/tmp/dir with space/tool"),
            "'/tmp/dir with space/tool'"
        );
    }

    #[test]
    fn shell_quote_word_escapes_embedded_single_quotes() {
        assert_eq!(super::shell_quote_word("it's"), "'it'\\''s'");
    }

    #[test]
    fn shell_quote_word_handles_the_empty_word() {
        assert_eq!(super::shell_quote_word(""), "''");
    }

    #[test]
    fn shell_join_quotes_every_word() {
        assert_eq!(super::shell_join(&v(&["cargo", "test"])), "'cargo' 'test'");
    }

    #[test]
    fn shell_join_of_a_space_containing_path_stays_one_token() {
        // Regression shape of the original bug: the conclave binary itself
        // lives under "Application Support" — joining must not produce a
        // string `sh` would split at the "Application"/"Support" boundary.
        let joined = super::shell_join(&v(&["/tmp/dir with space/tool", "status"]));
        assert_eq!(joined, "'/tmp/dir with space/tool' 'status'");
        // A naive unquoted join (the pre-fix behavior) would NOT contain the
        // path as one intact quoted token.
        assert_ne!(joined, "/tmp/dir with space/tool status");
    }

    // ── run_task_gate ─────────────────────────────────────────────────────

    #[test]
    fn run_task_gate_requires_self() {
        let argv = v(&["task", "gate", "ws1", "t1", "--", "true"]);
        assert!(super::run_task_gate(&argv, None).is_err());
    }

    #[test]
    fn run_task_gate_requires_dash_dash() {
        let argv = v(&["task", "gate", "ws1", "t1", "true"]);
        assert!(super::run_task_gate(&argv, Some("self1")).is_err());
    }

    #[test]
    fn run_task_gate_requires_a_command_after_dash_dash() {
        let argv = v(&["task", "gate", "ws1", "t1", "--"]);
        assert!(super::run_task_gate(&argv, Some("self1")).is_err());
    }

    #[test]
    fn run_task_gate_runs_the_command_and_records_zero_exit() {
        let argv = v(&["task", "gate", "ws1", "t1", "--", "echo", "hi"]);
        let (out, exit_code) = super::run_task_gate(&argv, Some("self1")).expect("gate run failed");
        assert_eq!(out[0], "task");
        assert_eq!(out[1], "gate");
        assert_eq!(out[2], "self1");
        assert_eq!(out[3], "ws1");
        assert_eq!(out[4], "t1");
        assert_eq!(out[5], "echo hi");
        assert_eq!(out[6], "0", "echo hi exits 0");
        assert!(
            out[7] == "unknown" || !out[7].is_empty(),
            "sha field present"
        );
        assert!(!out[8].is_empty(), "cwd field present");
        assert!(
            out[9].contains("hi"),
            "tail must contain the command's output"
        );
        assert_eq!(
            exit_code, 0,
            "returned exit code must match the recorded one"
        );
    }

    #[test]
    fn run_task_gate_records_non_zero_exit_as_evidence_not_error() {
        let argv = v(&["task", "gate", "ws1", "t1", "--", "false"]);
        let (out, exit_code) =
            super::run_task_gate(&argv, Some("self1")).expect("gate run must not error");
        assert_eq!(out[6], "1", "false exits 1");
        assert_eq!(
            exit_code, 1,
            "returned exit code must propagate the red gate"
        );
    }

    #[test]
    fn run_task_gate_survives_a_space_containing_path_regression() {
        // Regression test for the live bug: a path with a space (like the
        // conclave binary's own install path under "Application Support")
        // must reach `sh` as ONE word, not be split back apart.
        // Keep the space in the dir name — the space IS the regression under
        // test — but suffix a UUID so concurrent `cargo test` runs on the same
        // machine don't race create/remove on a shared fixed path (matches the
        // stage-test idiom above).
        let dir =
            std::env::temp_dir().join(format!("conclave cli test dir {}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("mkdir fixture failed");
        let script = dir.join("tool.sh");
        std::fs::write(&script, "#!/bin/sh\necho ran-ok\n").expect("write fixture failed");
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        std::fs::set_permissions(&script, perms).expect("chmod fixture failed");

        let argv = v(&[
            "task",
            "gate",
            "ws1",
            "t1",
            "--",
            script.to_str().unwrap(),
            "status",
        ]);
        let (out, exit_code) = super::run_task_gate(&argv, Some("self1")).expect("gate run failed");
        assert_eq!(
            exit_code, 0,
            "the space-containing path must resolve, not 127"
        );
        assert_eq!(out[6], "0");
        assert!(
            out[9].contains("ran-ok"),
            "tail must show the script actually ran: {:?}",
            out[9]
        );
        // The recorded cmd shows the quoted form since a word needed it.
        assert!(
            out[5].starts_with('\''),
            "recorded cmd for a space-containing word must be quoted: {:?}",
            out[5]
        );

        std::fs::remove_dir_all(&dir).expect("cleanup failed");
    }

    // ── resolve_task_create_plan_file ─────────────────────────────────────

    #[test]
    fn resolve_task_create_plan_file_is_a_noop_without_the_flag() {
        let argv = v(&["task", "create", "ws1", "t1", "Title"]);
        assert_eq!(
            super::resolve_task_create_plan_file(argv.clone()).unwrap(),
            argv
        );
    }

    #[test]
    fn resolve_task_create_plan_file_reads_the_file_and_rewrites_the_flag() {
        // Unique per run — a fixed temp path races create/remove with a
        // concurrent `cargo test` and dies with NotFound (see :2333).
        let path = std::env::temp_dir().join(format!(
            "conclave plan-file test {}.txt",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, "the plan body").expect("write fixture failed");

        let argv = v(&["task", "create", "ws1", "t1", "Title", "--plan-file"]);
        let mut argv = argv;
        argv.push(path.to_string_lossy().into_owned());

        let out = super::resolve_task_create_plan_file(argv).expect("resolve failed");
        assert_eq!(out[5], "--plan");
        assert_eq!(out[6], "the plan body");

        std::fs::remove_file(&path).expect("cleanup failed");
    }

    #[test]
    fn resolve_task_create_plan_file_missing_file_errors() {
        let argv = v(&[
            "task",
            "create",
            "ws1",
            "t1",
            "Title",
            "--plan-file",
            "/no/such/file/xyz",
        ]);
        assert!(super::resolve_task_create_plan_file(argv).is_err());
    }

    // ── artifact add: agent-slot injection + client-side --file read ──────

    #[test]
    fn artifact_add_injects_instance_from_env() {
        let out = expand_self_args(
            v(&[
                "artifact",
                "add",
                "ws1",
                "--title",
                "T",
                "--kind",
                "text",
                "--content",
                "x",
            ]),
            Some("self1"),
        )
        .unwrap();
        // artifact add <self1> ws1 --title T --kind text --content x
        assert_eq!(out[0], "artifact");
        assert_eq!(out[1], "add");
        assert_eq!(out[2], "self1");
        assert_eq!(out[3], "ws1");
    }

    #[test]
    fn artifact_add_injects_sentinel_without_instance_id() {
        let out = expand_self_args(
            v(&[
                "artifact",
                "add",
                "ws1",
                "--title",
                "T",
                "--kind",
                "text",
                "--content",
                "x",
            ]),
            None,
        )
        .unwrap();
        assert_eq!(out[2], "-", "no CONCLAVE_INSTANCE_ID → sentinel author");
    }

    #[test]
    fn artifact_list_and_get_pass_through_unexpanded() {
        let list = expand_self_args(v(&["artifact", "list", "ws1"]), Some("self1")).unwrap();
        assert_eq!(list, v(&["artifact", "list", "ws1"]));
        let get = expand_self_args(v(&["artifact", "get", "id1"]), Some("self1")).unwrap();
        assert_eq!(get, v(&["artifact", "get", "id1"]));
    }

    #[test]
    fn resolve_artifact_add_file_is_a_noop_without_the_flag() {
        let argv = v(&["artifact", "add", "ws1", "--content", "inline"]);
        assert_eq!(
            super::resolve_artifact_add_file(argv.clone()).unwrap(),
            argv
        );
    }

    #[test]
    fn resolve_artifact_add_file_reads_the_file_and_derives_filename() {
        // Unique per run — a fixed temp path races a concurrent `cargo test`.
        let path = std::env::temp_dir().join(format!(
            "conclave artifact test {}.md",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, "# the body").expect("write fixture failed");

        let mut argv = v(&[
            "artifact", "add", "ws1", "--title", "T", "--kind", "markdown", "--file",
        ]);
        argv.push(path.to_string_lossy().into_owned());

        let out = super::resolve_artifact_add_file(argv).expect("resolve failed");
        // --file <path> became --content <body>; --filename <basename> appended.
        let cpos = out
            .iter()
            .position(|w| w == "--content")
            .expect("has --content");
        assert_eq!(out[cpos + 1], "# the body");
        assert!(!out.iter().any(|w| w == "--file"), "--file rewritten away");
        let fpos = out
            .iter()
            .position(|w| w == "--filename")
            .expect("has --filename");
        assert_eq!(out[fpos + 1], path.file_name().unwrap().to_string_lossy());

        std::fs::remove_file(&path).expect("cleanup failed");
    }

    #[test]
    fn resolve_artifact_add_file_rejects_both_file_and_content() {
        let argv = v(&[
            "artifact",
            "add",
            "ws1",
            "--content",
            "x",
            "--file",
            "/tmp/whatever",
        ]);
        assert!(super::resolve_artifact_add_file(argv).is_err());
    }

    #[test]
    fn resolve_artifact_add_file_missing_file_errors() {
        let argv = v(&["artifact", "add", "ws1", "--file", "/no/such/file/xyz.md"]);
        assert!(super::resolve_artifact_add_file(argv).is_err());
    }

    // ── position / org rendering (spec position-system §5.3) ──────────────

    #[test]
    fn render_position_row_shows_fields_and_human_default() {
        let row = serde_json::json!({
            "id": "a1", "name": "Vega", "roleName": "Reviewer",
            "level": "senior", "supervisorName": "Detoro"
        });
        assert_eq!(
            super::render_position_row(&row),
            "Vega · Reviewer · senior · reports to Detoro"
        );
        // No supervisor / level → dashes and the (human) default.
        let bare = serde_json::json!({ "id": "a1", "name": "Sol" });
        assert_eq!(
            super::render_position_row(&bare),
            "Sol · - · - · reports to (human)"
        );
    }

    #[test]
    fn render_org_tree_indents_a_three_level_chain_and_a_root() {
        // Chain: Lead -> Sub -> Impl ; plus a separate NULL-supervisor root Solo.
        let rows = vec![
            serde_json::json!({ "id": "lead", "name": "Lead", "roleName": "Lead", "level": "principal", "working": true }),
            serde_json::json!({ "id": "sub", "name": "Sub", "roleName": "Lead", "level": "senior", "supervisorAgentId": "lead", "working": false }),
            serde_json::json!({ "id": "impl", "name": "Impl", "roleName": "Builder", "level": "mid", "supervisorAgentId": "sub" }),
            serde_json::json!({ "id": "solo", "name": "Solo", "roleName": "Researcher", "level": "junior", "working": true }),
        ];
        let tree = super::render_org_tree(&rows);
        let expected = "(human)\n\
            \u{20}\u{20}Lead · Lead · principal · working\n\
            \u{20}\u{20}\u{20}\u{20}Sub · Lead · senior · idle\n\
            \u{20}\u{20}\u{20}\u{20}\u{20}\u{20}Impl · Builder · mid · idle\n\
            \u{20}\u{20}Solo · Researcher · junior · working\n";
        assert_eq!(tree, expected);
    }

    #[test]
    fn render_org_tree_breaks_a_corrupt_cycle() {
        // a -> b -> a (should never happen post-validation, but must not hang).
        let rows = vec![
            serde_json::json!({ "id": "a", "name": "A", "supervisorAgentId": "b" }),
            serde_json::json!({ "id": "b", "name": "B", "supervisorAgentId": "a" }),
        ];
        // Neither is a root (both have a supervisor), so the tree is just the
        // human line — the point is it TERMINATES.
        let tree = super::render_org_tree(&rows);
        assert!(tree.starts_with("(human)\n"), "{tree}");
    }

    // ── msg: self-expansion + transcript rendering ────────────────────────

    #[test]
    fn expand_msg_list_injects_self_before_limit() {
        let out = expand_self_args(v(&["msg", "list"]), Some("self1")).unwrap();
        assert_eq!(out, v(&["msg", "list", "self1"]));
        // The injected id lands at argv[2], BEFORE any --limit tail.
        let out = expand_self_args(v(&["msg", "list", "--limit", "5"]), Some("self1")).unwrap();
        assert_eq!(out, v(&["msg", "list", "self1", "--limit", "5"]));
    }

    #[test]
    fn expand_msg_list_requires_self() {
        assert!(expand_self_args(v(&["msg", "list"]), None).is_err());
        assert!(expand_self_args(v(&["msg", "list"]), Some("")).is_err());
    }

    #[test]
    fn expand_msg_all_passes_through() {
        let all = v(&["msg", "all", "ws1"]);
        assert_eq!(expand_self_args(all.clone(), None).unwrap(), all);
        let all_lim = v(&["msg", "all", "ws1", "--limit", "10"]);
        assert_eq!(
            expand_self_args(all_lim.clone(), Some("self1")).unwrap(),
            all_lim
        );
    }

    // ── browser screenshot: path resolution ────────────────────────────────

    #[test]
    fn expand_browser_screenshot_resolves_relative_path_to_absolute() {
        let out =
            expand_self_args(v(&["browser", "screenshot", "shot.png"]), None).unwrap();
        assert_eq!(out.len(), 3);
        assert!(
            Path::new(&out[2]).is_absolute(),
            "expected absolute path, got {}",
            out[2]
        );
        assert!(
            out[2].ends_with("shot.png"),
            "expected path to end with the given filename, got {}",
            out[2]
        );
    }

    #[test]
    fn expand_browser_screenshot_leaves_absolute_path_unchanged() {
        let out = expand_self_args(
            v(&["browser", "screenshot", "/tmp/shot.png"]),
            None,
        )
        .unwrap();
        assert_eq!(out, v(&["browser", "screenshot", "/tmp/shot.png"]));
    }

    #[test]
    fn expand_browser_screenshot_injects_default_path_when_absent() {
        let out = expand_self_args(v(&["browser", "screenshot"]), None).unwrap();
        assert_eq!(out.len(), 3);
        assert!(
            Path::new(&out[2]).is_absolute(),
            "expected absolute default path, got {}",
            out[2]
        );
        assert!(
            out[2].ends_with("browser-screenshot.png"),
            "expected default filename, got {}",
            out[2]
        );
    }

    #[test]
    fn expand_browser_screenshot_skips_flags_before_path() {
        let out = expand_self_args(
            v(&["browser", "screenshot", "--width", "1440", "shot.png"]),
            None,
        )
        .unwrap();
        // The path token is the last one; --width/1440 pass through untouched.
        assert_eq!(out[2], "--width");
        assert_eq!(out[3], "1440");
        assert!(
            Path::new(&out[4]).is_absolute(),
            "expected the trailing token to be resolved, got {}",
            out[4]
        );
        assert!(out[4].ends_with("shot.png"), "got {}", out[4]);
    }

    #[test]
    fn expand_browser_screenshot_resolves_path_before_trailing_flags() {
        let out = expand_self_args(
            v(&["browser", "screenshot", "shot.png", "--width", "1440"]),
            None,
        )
        .unwrap();
        assert!(
            Path::new(&out[2]).is_absolute(),
            "expected the path token to be resolved, got {}",
            out[2]
        );
        assert!(out[2].ends_with("shot.png"), "got {}", out[2]);
        // The trailing flag pair is untouched and still present.
        assert_eq!(out[3], "--width");
        assert_eq!(out[4], "1440");
    }

    #[test]
    fn render_msg_transcript_reverses_to_chronological_with_names() {
        // DB order is newest-first; input here is [newer, older].
        let rows = vec![
            serde_json::json!({ "createdAt": "2026-07-08T12:05:00+00:00", "fromName": "Bravo", "toName": "Alpha", "text": "reply", "status": "delivered" }),
            serde_json::json!({ "createdAt": "2026-07-08T12:01:00+00:00", "fromName": "Alpha", "toName": "Bravo", "text": "hello", "status": "delivered" }),
        ];
        let expected = "12:01  Alpha → Bravo  hello\n\
                        12:05  Bravo → Alpha  reply\n";
        assert_eq!(super::render_msg_transcript(&rows), expected);
    }

    #[test]
    fn render_msg_transcript_marks_queued_and_falls_back_to_short_id() {
        let rows = vec![serde_json::json!({
            "createdAt": "2026-07-08T09:30:00+00:00",
            "fromInstanceId": "0123456789abcdef", "toInstanceId": "fedcba9876543210",
            "text": "offline", "status": "queued"
        })];
        assert_eq!(
            super::render_msg_transcript(&rows),
            "09:30  01234567 → fedcba98  offline [queued]\n"
        );
    }

    #[test]
    fn render_msg_transcript_empty_is_clean_marker() {
        assert_eq!(super::render_msg_transcript(&[]), "(no messages)\n");
    }

    #[test]
    fn render_task_brief_is_human_readable_and_pointers_are_visible() {
        let brief = serde_json::json!({
            "limit": 3,
            "task": {
                "slug": "t1",
                "title": "Task One",
                "state": "claimed",
                "designCanon": "canon-x",
                "fileBoundary": ["src/a.rs", "src/b.rs"],
                "planExcerpt": "line 1\nline 2\nline 3",
                "planTruncated": true
            },
            "openChallenges": [
                { "id": "challenge-12345678", "claim": "fix it", "status": "open" }
            ],
            "latestGates": [
                { "id": "gate-12345678", "cmd": "cargo test", "exit": 0, "sha": "sha-abcdef12", "createdAt": "2026-07-08T00:00:00Z" }
            ],
            "lastEvents": [
                { "id": "event-12345678", "kind": "note", "createdAt": "2026-07-08T00:00:00Z", "payload": { "text": "note body" } }
            ],
            "memoryHits": [
                { "id": "mem-12345678", "text": "memory text", "score": 0.75, "sourceKind": "manual", "sourceId": "src-12345678", "createdAt": "2026-07-08T00:00:00Z" }
            ],
            "memoryError": null
        });

        let rendered = super::render_task_brief(&brief);
        assert!(rendered.contains("Task brief: t1  Task One"), "{rendered}");
        assert!(rendered.contains("file boundary:"), "{rendered}");
        assert!(rendered.contains("plan excerpt:"), "{rendered}");
        assert!(rendered.contains("open challenges (1):"), "{rendered}");
        assert!(rendered.contains("latest gates (1):"), "{rendered}");
        assert!(rendered.contains("last events (1):"), "{rendered}");
        assert!(rendered.contains("memory hits (1):"), "{rendered}");
        assert!(rendered.contains("challeng"), "{rendered}");
        assert!(rendered.contains("gate-123"), "{rendered}");
        assert!(rendered.contains("event-12"), "{rendered}");
        assert!(rendered.contains("mem-1234"), "{rendered}");
        assert!(rendered.contains("… truncated"), "{rendered}");
    }

    // ── stage: private-index commit + attribution + snapshot op log ───────

    fn run_git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .current_dir(dir)
            .args(args)
            .status()
            .expect("git command failed to run");
        assert!(status.success(), "git {args:?} failed");
    }

    /// A throwaway repo with one committed file inside the (test) boundary
    /// ("in-scope.txt") and one outside it ("out-of-scope.txt").
    fn init_repo() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("conclave-stage-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("mkdir fixture failed");
        run_git(&dir, &["init", "-q", "-b", "main"]);
        run_git(&dir, &["config", "user.name", "Test Human"]);
        run_git(&dir, &["config", "user.email", "human@example.com"]);
        std::fs::write(dir.join("in-scope.txt"), "v1\n").expect("write fixture failed");
        std::fs::write(dir.join("out-of-scope.txt"), "v1\n").expect("write fixture failed");
        run_git(&dir, &["add", "-A"]);
        run_git(&dir, &["commit", "-q", "-m", "initial"]);
        dir
    }

    #[test]
    fn path_in_boundary_matches_exact_and_nested_prefixes_only() {
        let boundary = vec!["src".to_string(), "docs/adr".to_string()];
        assert!(super::path_in_boundary("src", &boundary));
        assert!(super::path_in_boundary("src/lib.rs", &boundary));
        assert!(!super::path_in_boundary("srcfoo", &boundary));
        assert!(super::path_in_boundary("docs/adr/0001.md", &boundary));
        assert!(!super::path_in_boundary("docs/other.md", &boundary));
    }

    #[test]
    fn require_boundary_refuses_an_empty_boundary() {
        assert!(super::require_boundary("t1", vec![]).is_err());
        assert!(super::require_boundary("t1", vec!["a.rs".to_string()]).is_ok());
    }

    #[test]
    fn parse_task_boundary_reads_field_from_the_real_task_get_envelope() {
        let envelope = serde_json::json!({
            "task": { "fileBoundary": ["a.ts"] },
            "events": []
        });
        assert_eq!(
            super::parse_task_boundary(&envelope, "t1").expect("boundary should parse"),
            vec!["a.ts".to_string()]
        );
    }

    #[test]
    fn parse_task_boundary_refuses_an_empty_boundary_inside_the_envelope() {
        let envelope = serde_json::json!({
            "task": { "fileBoundary": [] },
            "events": []
        });
        let err = super::parse_task_boundary(&envelope, "t1").expect_err("must refuse");
        assert!(err.contains("no fileBoundary"), "unexpected error: {err}");
    }

    #[test]
    fn parse_task_boundary_refuses_a_legacy_flat_shape() {
        // No `task` wrapper at all — must not fall back to a top-level
        // `fileBoundary`; the envelope is the only shape the engine sends.
        let flat = serde_json::json!({ "fileBoundary": ["a.ts"] });
        let err = super::parse_task_boundary(&flat, "t1").expect_err("must refuse");
        assert!(err.contains("no fileBoundary"), "unexpected error: {err}");
    }

    #[test]
    fn stage_status_is_clean_after_private_index_commit() {
        let dir = init_repo();
        std::fs::write(dir.join("in-scope.txt"), "v2\n").unwrap();
        let boundary = vec!["in-scope.txt".to_string()];

        super::stage_commit_core(
            &dir,
            "main",
            "t1",
            &boundary,
            "commit boundary",
            "Dabin",
            "dabin-agent-id",
        )
        .expect("stage commit must succeed");

        let head_diff = super::git_output(
            &dir,
            &["diff", "--name-status", "HEAD", "--", "in-scope.txt"],
        )
        .unwrap();
        assert!(
            head_diff.is_empty(),
            "worktree must already match the new HEAD: {head_diff}"
        );
        let porcelain = super::git_output(&dir, &["status", "--porcelain"]).unwrap();
        assert!(
            porcelain.lines().any(|line| line == "MM in-scope.txt"),
            "the shared index must remain stale to reproduce F1: {porcelain}"
        );

        let (in_boundary, out_of_boundary) =
            super::stage_status_entries(&dir, &boundary).expect("stage status must succeed");
        assert!(
            in_boundary.is_empty(),
            "committed boundary file must report CLEAN, got {in_boundary:?}"
        );
        assert!(out_of_boundary.is_empty(), "{out_of_boundary:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stage_status_is_clean_after_committing_a_new_boundary_file() {
        let dir = init_repo();
        std::fs::write(dir.join("new-in-scope.txt"), "new\n").unwrap();
        let boundary = vec!["new-in-scope.txt".to_string()];

        super::stage_commit_core(
            &dir,
            "main",
            "t1",
            &boundary,
            "commit new boundary file",
            "Dabin",
            "dabin-agent-id",
        )
        .expect("stage commit must succeed");

        let (in_boundary, out_of_boundary) =
            super::stage_status_entries(&dir, &boundary).expect("stage status must succeed");
        assert!(
            in_boundary.is_empty(),
            "newly committed boundary file must report CLEAN, got {in_boundary:?}"
        );
        assert!(out_of_boundary.is_empty(), "{out_of_boundary:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stage_status_reports_tracked_and_untracked_boundary_changes() {
        let dir = init_repo();
        std::fs::write(dir.join("in-scope.txt"), "v2\n").unwrap();
        std::fs::write(dir.join("new-in-scope.txt"), "new\n").unwrap();
        let boundary = vec!["in-scope.txt".to_string(), "new-in-scope.txt".to_string()];

        let (in_boundary, out_of_boundary) =
            super::stage_status_entries(&dir, &boundary).expect("stage status must succeed");
        assert!(
            in_boundary.iter().any(|line| line == "M\tin-scope.txt"),
            "tracked modification must keep its letter status: {in_boundary:?}"
        );
        assert!(
            in_boundary
                .iter()
                .any(|line| line == "??\tnew-in-scope.txt"),
            "untracked boundary file must appear in status: {in_boundary:?}"
        );
        assert!(out_of_boundary.is_empty(), "{out_of_boundary:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stage_status_ignores_a_staged_stranger_entry_in_the_shared_index() {
        let dir = init_repo();
        let boundary = vec!["in-scope.txt".to_string()];
        std::fs::write(dir.join("in-scope.txt"), "v2\n").unwrap();

        let before =
            super::stage_status_entries(&dir, &boundary).expect("baseline status must succeed");
        assert_eq!(before.0, vec!["M\tin-scope.txt"]);
        assert!(before.1.is_empty(), "{before:?}");

        std::fs::write(dir.join("out-of-scope.txt"), "staged stranger\n").unwrap();
        run_git(&dir, &["add", "--", "out-of-scope.txt"]);
        std::fs::write(dir.join("out-of-scope.txt"), "v1\n").unwrap();

        let porcelain = super::git_output(&dir, &["status", "--porcelain"]).unwrap();
        assert!(
            porcelain.lines().any(|line| line == "MM out-of-scope.txt"),
            "fixture must leave a staged stranger entry in the shared index: {porcelain}"
        );
        let after = super::stage_status_entries(&dir, &boundary)
            .expect("HEAD-based status must ignore shared-index staging");
        assert_eq!(
            after, before,
            "shared index state must not affect stage status"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stage_commit_only_touches_boundary_paths_and_shared_index_untouched() {
        let dir = init_repo();
        std::fs::write(dir.join("in-scope.txt"), "v2\n").unwrap();
        std::fs::write(dir.join("out-of-scope.txt"), "v2\n").unwrap();

        let index_path = dir.join(".git").join("index");
        let index_before = std::fs::read(&index_path).expect("read shared index");

        let boundary = vec!["in-scope.txt".to_string()];
        let (sha, n_files) = super::stage_commit_core(
            &dir,
            "main",
            "t1",
            &boundary,
            "msg",
            "Test Agent",
            "agent-1",
        )
        .expect("commit must succeed");
        assert_eq!(n_files, 1);

        let index_after = std::fs::read(&index_path).expect("read shared index");
        assert_eq!(
            index_before, index_after,
            "shared .git/index must be byte-identical before/after"
        );

        // The new commit's tree carries the boundary path's NEW content...
        let in_scope = super::git_output(&dir, &["show", &format!("{sha}:in-scope.txt")]).unwrap();
        assert_eq!(in_scope, "v2");
        // ...but the out-of-boundary edit did NOT land — the commit tree
        // still has its OLD content (moving the branch never touches the
        // shared index, so `git status` afterward compares against a now-
        // stale index; that's the tree-content check that actually matters).
        let out_of_scope =
            super::git_output(&dir, &["show", &format!("{sha}:out-of-scope.txt")]).unwrap();
        assert_eq!(
            out_of_scope, "v1",
            "out-of-boundary edit must not be committed"
        );

        // The out-of-boundary edit remains an uncommitted diff against the
        // new HEAD — "stays dirty and uncommitted" (test 1).
        let diff = super::git_output(&dir, &["diff", &sha, "--", "out-of-scope.txt"]).unwrap();
        assert!(
            !diff.is_empty(),
            "out-of-boundary edit must remain dirty/uncommitted"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stage_commit_stamps_author_and_trailers_committer_stays_default() {
        let dir = init_repo();
        std::fs::write(dir.join("in-scope.txt"), "v2\n").unwrap();
        let boundary = vec!["in-scope.txt".to_string()];

        let (sha, _) = super::stage_commit_core(
            &dir,
            "main",
            "t1",
            &boundary,
            "did a thing",
            "Dew",
            "dew-agent-id",
        )
        .unwrap();

        let author = super::git_output(&dir, &["show", "-s", "--format=%an <%ae>", &sha]).unwrap();
        assert_eq!(author, "Dew <dew-agent-id@agents.conclave.local>");

        let committer =
            super::git_output(&dir, &["show", "-s", "--format=%cn <%ce>", &sha]).unwrap();
        assert_eq!(
            committer, "Test Human <human@example.com>",
            "committer stays the repo default"
        );

        let body = super::git_output(&dir, &["show", "-s", "--format=%B", &sha]).unwrap();
        assert!(body.contains("Conclave-Task: t1"), "{body}");
        assert!(body.contains("Conclave-Agent: dew-agent-id"), "{body}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stage_commit_core_retries_after_a_simulated_concurrent_branch_move() {
        let dir = init_repo();
        std::fs::write(dir.join("in-scope.txt"), "v2\n").unwrap();
        let boundary = vec!["in-scope.txt".to_string()];

        let (sha, _) = super::stage_commit_core_with_hook(
            &dir,
            "main",
            "t1",
            &boundary,
            "mine",
            "Dew",
            "dew",
            |attempt, repo| {
                if attempt == 1 {
                    // Simulate a peer's commit landing between attempt 1's
                    // HEAD read and its update-ref — deterministic (no
                    // timing race), exercising the CAS-failure-then-
                    // refresh-and-retry path.
                    super::git_output(
                        repo,
                        &["commit", "--allow-empty", "-q", "-m", "peer commit"],
                    )
                    .unwrap();
                }
            },
        )
        .expect("must succeed via CAS retry after the simulated peer commit");

        let head_now = super::git_output(&dir, &["rev-parse", "main"]).unwrap();
        assert_eq!(
            head_now, sha,
            "our commit must land as the final branch tip"
        );
        let log = super::git_output(&dir, &["log", "--format=%s", "main"]).unwrap();
        assert!(
            log.contains("peer commit"),
            "the peer's commit must survive in history: {log}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stage_commit_core_nothing_to_commit_leaves_branch_unmoved() {
        let dir = init_repo();
        let boundary = vec!["in-scope.txt".to_string()];
        let head_before = super::head_sha(&dir).unwrap();

        let err = super::stage_commit_core(&dir, "main", "t1", &boundary, "msg", "Dew", "dew")
            .unwrap_err();
        assert!(err.contains("nothing to commit"), "{err}");
        assert_eq!(
            super::head_sha(&dir).unwrap(),
            head_before,
            "branch ref must not move"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stage_commit_boundary_deletion_commits_as_a_deletion() {
        let dir = init_repo();
        std::fs::remove_file(dir.join("in-scope.txt")).unwrap();
        let boundary = vec!["in-scope.txt".to_string()];

        let (sha, n_files) =
            super::stage_commit_core(&dir, "main", "t1", &boundary, "delete it", "Dew", "dew")
                .unwrap();
        assert_eq!(n_files, 1);

        let ls = super::git_output(&dir, &["ls-tree", "--name-only", &sha]).unwrap();
        assert!(
            !ls.contains("in-scope.txt"),
            "deleted file must not be in the commit tree: {ls}"
        );
        assert!(
            ls.contains("out-of-scope.txt"),
            "untouched file must survive: {ls}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stage_restore_source_must_be_reachable_from_the_task_snapshot_ref() {
        let dir = init_repo();
        let boundary = vec!["in-scope.txt".to_string()];
        let outside_sha = super::head_sha(&dir).unwrap();
        let snapshot_sha = super::snapshot(
            &dir,
            "t1",
            &boundary,
            "manual",
            "Dabin",
            "dabin@agents.conclave.local",
        )
        .unwrap()
        .expect("snapshot must be created");
        std::fs::write(dir.join("in-scope.txt"), "v2\n").unwrap();
        let snapshot_tip = super::snapshot(
            &dir,
            "t1",
            &boundary,
            "second",
            "Dabin",
            "dabin@agents.conclave.local",
        )
        .unwrap()
        .expect("second snapshot must be created");

        let err = super::validate_stage_restore_source(&dir, "ws-1", "t1", &outside_sha)
            .expect_err("normal branch commit must not be accepted as a task snapshot");
        assert!(err.contains("refs/conclave/stage/t1"), "{err}");
        assert!(err.contains("stage log"), "{err}");
        super::validate_stage_restore_source(&dir, "ws-1", "t1", &snapshot_sha)
            .expect("older snapshot from stage log must be accepted");
        super::validate_stage_restore_source(&dir, "ws-1", "t1", &snapshot_tip)
            .expect("snapshot ref tip must be accepted");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stage_restore_source_reports_a_missing_snapshot_ref_clearly() {
        let dir = init_repo();
        let sha = super::head_sha(&dir).unwrap();

        let err = super::validate_stage_restore_source(&dir, "ws-1", "missing", &sha)
            .expect_err("restore must reject when the task has no snapshot ref");
        assert!(err.contains("refs/conclave/stage/missing"), "{err}");
        assert!(err.contains("does not exist"), "{err}");
        assert!(err.contains("conclave stage log ws-1 missing"), "{err}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stage_snapshot_restore_roundtrip_and_auto_snap_recovers_modified_state() {
        let dir = init_repo();
        let boundary = vec!["in-scope.txt".to_string()];

        let snap1 = super::snapshot(
            &dir,
            "t1",
            &boundary,
            "manual",
            "Dew",
            "dew@agents.conclave.local",
        )
        .unwrap()
        .expect("first snapshot must be created");

        std::fs::write(dir.join("in-scope.txt"), "modified\n").unwrap();

        let auto_snap = super::snapshot(
            &dir,
            "t1",
            &boundary,
            "auto-pre-restore",
            "Dew",
            "dew@agents.conclave.local",
        )
        .unwrap()
        .expect("auto-snap of modified state must be created");
        assert_ne!(auto_snap, snap1);

        run_git(
            &dir,
            &[
                "restore",
                "--worktree",
                "--source",
                &snap1,
                "--",
                "in-scope.txt",
            ],
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("in-scope.txt")).unwrap(),
            "v1\n"
        );

        // The modified state is still recoverable via the auto-snap.
        run_git(
            &dir,
            &[
                "restore",
                "--worktree",
                "--source",
                &auto_snap,
                "--",
                "in-scope.txt",
            ],
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("in-scope.txt")).unwrap(),
            "modified\n"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stage_log_lists_snapshots_newest_first_with_labels() {
        let dir = init_repo();
        let boundary = vec!["in-scope.txt".to_string()];
        super::snapshot(
            &dir,
            "t1",
            &boundary,
            "first",
            "Dew",
            "dew@agents.conclave.local",
        )
        .unwrap();
        std::fs::write(dir.join("in-scope.txt"), "v2\n").unwrap();
        super::snapshot(
            &dir,
            "t1",
            &boundary,
            "second",
            "Dew",
            "dew@agents.conclave.local",
        )
        .unwrap();

        let log = super::git_output(
            &dir,
            &["log", "--format=%s", &super::stage_snapshot_ref("t1")],
        )
        .unwrap();
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("second"), "newest first: {lines:?}");
        assert!(lines[1].contains("first"), "newest first: {lines:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stage_snap_skips_the_ref_update_when_tree_is_unchanged() {
        let dir = init_repo();
        let boundary = vec!["in-scope.txt".to_string()];
        let first = super::snapshot(
            &dir,
            "t1",
            &boundary,
            "first",
            "Dew",
            "dew@agents.conclave.local",
        )
        .unwrap()
        .expect("first snapshot created");

        let second = super::snapshot(
            &dir,
            "t1",
            &boundary,
            "second",
            "Dew",
            "dew@agents.conclave.local",
        )
        .unwrap();
        assert!(second.is_none(), "identical tree must skip the ref update");

        let tip =
            super::git_output(&dir, &["rev-parse", &super::stage_snapshot_ref("t1")]).unwrap();
        assert_eq!(tip, first, "ref must still point at the first snapshot");

        std::fs::remove_dir_all(&dir).ok();
    }
}
