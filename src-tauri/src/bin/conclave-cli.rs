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
use sha2::{Digest, Sha256};
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
  orient <workspaceId>              (one bounded fresh-context packet: slim tasks, roster, your messages/watches, blackboard heads, self; inside a spawned agent)
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
  task create   <workspaceId> <slug> <title...> [--boundary p1,p2] [--canon txt] [--plan-file path] [--watchers id,id]  (--watchers subscribes the owner + listed workspace agents to the task in one transaction)
  task list     <workspaceId> [--state s] [--full | --all]  (slim open-task rows by default; --full = full rows incl plan; --all = slim rows incl merged/abandoned)
  task get      <workspaceId> <slug>
  task brief    <workspaceId> <slug> [--limit N]
  task claim    <workspaceId> <slug>          (inside a spawned agent)
  task plan-check <workspaceId> <slug>        (validate + fingerprint the canonical council plan against this checkout; records a typed plan_check event on success; inside a spawned agent)
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
  rtk-hook --rtk <absRtkPath>          (local Claude Code PreToolUse hook body: stdin JSON -> rtk rewrite -> hook response; always exits 0)
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
        // task conclave-orient: the fresh-context orientation packet is
        // always about the CALLING agent (its inbox, its watches, its meter),
        // so the actor slot is self-keyed exactly like `msg list`.
        Some("orient") => {
            let me = require_self("orient")?;
            if argv.len() != 2 {
                return Err("conclave: orient <workspaceId>".to_string());
            }
            Ok(vec!["orient".to_string(), me.to_string(), argv[1].clone()])
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
        // `plan-check` mirrors `gate`: `run_task_plan_check` already resolved
        // self (plus checkout reads) and built the full wire argv before this
        // runs — pass it through untouched.
        Some("task") if argv.get(1).map(String::as_str) == Some("plan-check") => Ok(argv),
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
                    out.push(
                        cwd.join("browser-screenshot.png")
                            .to_string_lossy()
                            .into_owned(),
                    );
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
    // Same freshness preflight as a direct claim (lead-council v1, RULING
    // 2987c0b6), but hashed against the plan INSIDE the just-created
    // worktree — that is the checkout the implementer will read, so the
    // fingerprint stays honest even when main has uncommitted plan edits
    // (spec "Freshness Warning": lane start creates its worktree first,
    // then hashes there). Warning-only; the claim below is sent unchanged.
    // `suppress_unverifiable`: when `task get` itself soft-fails (task
    // absent, app down) the claim below is about to soft-fail for the same
    // reason with the calm note — the loud unverifiable warn would be pure
    // noise here (ruling be2e68d7 F1).
    claim_freshness_preflight(
        ws,
        slug,
        Some(&lane_worktree_root(slug)),
        self_instance,
        true,
    )
    .await;

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
///
/// A boundary entry that exists neither in the working tree nor in `base` —
/// a file the plan only creates in a LATER task, or a rename's old name
/// already committed away — matches no pathspec at all, and one such entry
/// used to abort the whole `git add` and with it every stage commit/snap for
/// the lane (task stage-commit-missing-boundary-path; both failure shapes in
/// memories ae4615e9 and 2e6ecaeb). Such entries can carry no changes, so
/// they are SKIPPED and returned alongside the tree for the caller to
/// surface; with nothing left to add the tree is simply `base`'s own.
fn build_boundary_tree(
    repo: &std::path::Path,
    base: &str,
    boundary: &[String],
) -> Result<(String, Vec<String>), String> {
    let index_path = tmp_index_path();
    let result = (|| {
        git_with_index(repo, &index_path, &["read-tree", base])?;
        let mut kept: Vec<&str> = Vec::new();
        let mut skipped: Vec<String> = Vec::new();
        for entry in boundary {
            let trimmed = entry.strip_suffix('/').unwrap_or(entry.as_str());
            // `symlink_metadata` (not `exists`): a dangling symlink is still
            // content git can add. The private index holds `base` right now,
            // so `ls-files` against it answers "tracked in base?" without
            // ever reading the shared index.
            let on_disk = !trimmed.is_empty() && repo.join(trimmed).symlink_metadata().is_ok();
            if on_disk || !git_with_index(repo, &index_path, &["ls-files", "--", entry])?.is_empty()
            {
                kept.push(entry.as_str());
            } else {
                skipped.push(entry.clone());
            }
        }
        // A pathspec-less `git add -A` would stage the ENTIRE tree — with
        // every entry skipped there is nothing to layer, never everything.
        if !kept.is_empty() {
            let mut args: Vec<&str> = vec!["add", "-A", "--"];
            args.extend(kept);
            git_with_index(repo, &index_path, &args)?;
        }
        git_with_index(repo, &index_path, &["write-tree"]).map(|tree| (tree, skipped))
    })();
    let _ = std::fs::remove_file(&index_path);
    result
}

/// One-line stderr note for skipped boundary entries — commit/snap skip
/// entries absent from the checkout ([`build_boundary_tree`]), restore skips
/// entries absent from the snapshot tree
/// ([`restore_boundary_from_snapshot`]); `absent_from` names which. A note,
/// not a warning: a plan legitimately declares files its later tasks will
/// create.
fn warn_skipped_boundary(verb: &str, skipped: &[String], absent_from: &str) {
    if !skipped.is_empty() {
        eprintln!(
            "conclave: stage {verb}: note — skipped {} declared boundary path(s) absent from \
             {absent_from}: {}",
            skipped.len(),
            skipped.join(", ")
        );
    }
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
/// the new commit sha, the number of changed boundary files, and any
/// declared-but-absent boundary paths that were skipped.
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
) -> Result<(String, usize, Vec<String>), String> {
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
) -> Result<(String, usize, Vec<String>), String> {
    let branch_ref = format!("refs/heads/{branch}");
    let author_email = format!("{author_id}@agents.conclave.local");
    let full_message = format!("{message}\n\nConclave-Task: {slug}\nConclave-Agent: {author_id}");

    for attempt in 1..=3u32 {
        let old_head = head_sha(repo)?;
        race_hook(attempt, repo);
        let head_tree = tree_of(repo, &old_head)?;
        let (new_tree, skipped) = build_boundary_tree(repo, &old_head, boundary)?;
        if new_tree == head_tree {
            // Name the skipped paths here: when the ONLY expected change
            // lived in an absent path, this is the line that explains why
            // the commit found nothing.
            let skipped_hint = if skipped.is_empty() {
                String::new()
            } else {
                format!(" (skipped absent boundary path(s): {})", skipped.join(", "))
            };
            return Err(format!(
                "nothing to commit in boundary for '{slug}'{skipped_hint}"
            ));
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
                return Ok((new_commit, n_files, skipped));
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
/// `<label|auto-...>` segment of the commit message. Returns the snapshot
/// sha (`None` when unchanged) plus any skipped absent boundary paths.
fn snapshot(
    repo: &std::path::Path,
    slug: &str,
    boundary: &[String],
    reason: &str,
    author_name: &str,
    author_email: &str,
) -> Result<(Option<String>, Vec<String>), String> {
    let snap_ref = stage_snapshot_ref(slug);
    let prev = git_output(repo, &["rev-parse", "--verify", "-q", &snap_ref]).ok();

    let base = prev.clone().unwrap_or_else(|| "HEAD".to_string());
    let (new_tree, skipped) = build_boundary_tree(repo, &base, boundary)?;

    if let Some(prev_sha) = &prev {
        let prev_tree = tree_of(repo, prev_sha)?;
        if new_tree == prev_tree {
            return Ok((None, skipped));
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
    Ok((Some(new_commit), skipped))
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
        Ok((sha, n_files, skipped)) => {
            warn_skipped_boundary("commit", &skipped, "this checkout");
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
        Ok((Some(sha), skipped)) => {
            warn_skipped_boundary("snap", &skipped, "this checkout");
            println!("stage snap: {} ({reason})", &sha[..12.min(sha.len())]);
            ExitCode::SUCCESS
        }
        Ok((None, skipped)) => {
            warn_skipped_boundary("snap", &skipped, "this checkout");
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

/// Worktree-restores `boundary` from `snap_sha`, tolerating declared entries
/// the snapshot tree does not contain — the restore-side twin of the skip
/// [`build_boundary_tree`] does for commit/snap (task
/// stage-restore-missing-boundary-path): one absent entry used to make the
/// single `git restore` invocation abort the whole restore. An entry absent
/// from the snapshot has nothing to restore FROM (and `git restore` never
/// deletes paths its source lacks), so it is skipped and returned for the
/// caller to surface; an entry present in the snapshot but deleted from the
/// worktree is a real restore (git recreates it). With every entry skipped no
/// `git restore` runs at all — an empty pathspec would be an error, not a
/// no-op. Returns `(restored, skipped)`.
fn restore_boundary_from_snapshot(
    repo: &std::path::Path,
    snap_sha: &str,
    boundary: &[String],
) -> Result<(Vec<String>, Vec<String>), String> {
    let mut kept: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for entry in boundary {
        let trimmed = entry.strip_suffix('/').unwrap_or(entry.as_str());
        // `cat-file -e <snap>:<path>` answers "is this path in the snapshot
        // tree" (blob or subtree alike) without touching any index.
        let present = !trimmed.is_empty()
            && Command::new("git")
                .current_dir(repo)
                .args(["cat-file", "-e", &format!("{snap_sha}:{trimmed}")])
                .output()
                .map_err(|e| format!("could not run git cat-file: {e}"))?
                .status
                .success();
        if present {
            kept.push(entry.clone());
        } else {
            skipped.push(entry.clone());
        }
    }
    if !kept.is_empty() {
        let mut args: Vec<&str> = vec!["restore", "--worktree", "--source", snap_sha, "--"];
        args.extend(kept.iter().map(String::as_str));
        git_output(repo, &args)?;
    }
    Ok((kept, skipped))
}

/// `stage restore <ws> <slug> <snapSha>`: auto-snaps the current state first
/// (decision 6 — so the restore itself is undoable), then `git restore
/// --worktree` from `snapSha` for the boundary paths the snapshot actually
/// contains ([`restore_boundary_from_snapshot`]). Never touches the index
/// (no `--staged`).
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
        Ok((Some(sha), _)) => println!(
            "stage restore: auto-snapshotted current state as {}",
            &sha[..12.min(sha.len())]
        ),
        Ok((None, _)) => {}
        Err(e) => {
            eprintln!("conclave: stage restore: auto-snapshot failed: {e}");
            return ExitCode::FAILURE;
        }
    }

    match restore_boundary_from_snapshot(&repo, snap_sha, &boundary) {
        Ok((restored, skipped)) => {
            warn_skipped_boundary("restore", &skipped, &format!("snapshot {snap_sha}"));
            if restored.is_empty() {
                println!(
                    "stage restore: nothing to restore — every declared boundary path is \
                     absent from snapshot {snap_sha}"
                );
                return ExitCode::SUCCESS;
            }
            println!(
                "stage restore: restored {} path(s) from {snap_sha}:",
                restored.len()
            );
            for p in &restored {
                println!("  {p}");
            }
            println!("stage restore: done");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("conclave: stage restore: {e}");
            ExitCode::FAILURE
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

// ── output economy (task cli-output-economy) ────────────────────────────────
//
// Every verb's rendered response passes through ONE cap point in `main`: past
// `CLI_OUTPUT_CAP_BYTES` the full text is written under the cli-output dir and
// only a head + pointer reaches the terminal, so no future verb can flood an
// agent's context. Gate runs additionally keep their FULL build log there and
// print only a bounded excerpt. `CONCLAVE_NO_CAP` (any non-empty value)
// disables the cap; the `snapshot` family is always exempt — a restore must
// arrive whole.

/// A rendered response longer than this gets capped to a head + file pointer.
const CLI_OUTPUT_CAP_BYTES: usize = 10_000;

/// How much of a capped response still reaches the terminal.
const CLI_OUTPUT_HEAD_BYTES: usize = 2_000;

/// Files in the cli-output dir older than this are pruned on each new write.
const CLI_OUTPUT_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Lines of gate log printed to the terminal: the last 40, plus the first 10
/// as a header when anything was omitted between them.
const GATE_PRINT_TAIL_LINES: usize = 40;
const GATE_PRINT_HEAD_LINES: usize = 10;

/// First `max_bytes` of `text`, lossy at a broken UTF-8 boundary — the head
/// counterpart of [`tail_bytes`].
fn head_bytes(text: &str, max_bytes: usize) -> String {
    let bytes = text.as_bytes();
    if bytes.len() <= max_bytes {
        return text.to_string();
    }
    String::from_utf8_lossy(&bytes[..max_bytes]).into_owned()
}

/// `~/Library/Application Support/Conclave/cli-output` — capped responses and
/// full gate logs land here (same `dirs::data_dir()` root as the socket).
fn cli_output_dir() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("Conclave").join("cli-output"))
}

/// Best-effort housekeeping: delete cli-output files older than
/// [`CLI_OUTPUT_MAX_AGE`]. Errors are ignored — pruning must never fail the
/// write that triggered it.
fn prune_cli_output(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > CLI_OUTPUT_MAX_AGE);
        if stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Write `content` to `<cli-output>/<label>-<utcTs>-<pid>.<ext>` (pruning old
/// files first) and return the file's path.
fn write_cli_output(label: &str, ext: &str, content: &str) -> Result<PathBuf, String> {
    let dir = cli_output_dir().ok_or("could not resolve user data directory")?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    prune_cli_output(&dir);
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%3fZ");
    let path = dir.join(format!("{label}-{stamp}-{}.{ext}", std::process::id()));
    std::fs::write(&path, content)
        .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    Ok(path)
}

/// A filesystem-safe label for the invoked verb (`task-list`, `bb-get`, …):
/// the first two argv words, lowercased, non `[a-z0-9-]` squashed to `-`.
fn verb_label(argv: &[String]) -> String {
    let label = argv
        .iter()
        .take(2)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>();
    if label.is_empty() {
        "output".to_string()
    } else {
        label
    }
}

/// True when the cap is disabled for this process (`CONCLAVE_NO_CAP` set to
/// any non-empty value).
fn cap_disabled() -> bool {
    std::env::var("CONCLAVE_NO_CAP").is_ok_and(|v| !v.is_empty())
}

/// The single cap point: print `rendered` whole when it fits (or `no_cap`),
/// else persist the full text and print only [`CLI_OUTPUT_HEAD_BYTES`] plus a
/// pointer. A failed persist prints everything — losing evidence to save
/// bytes would invert the point.
fn emit_capped(label: &str, rendered: &str, no_cap: bool) {
    if no_cap || rendered.len() <= CLI_OUTPUT_CAP_BYTES {
        print!("{rendered}");
        return;
    }
    match write_cli_output(label, "txt", rendered) {
        Ok(path) => {
            let head = head_bytes(rendered, CLI_OUTPUT_HEAD_BYTES);
            let newline = if head.ends_with('\n') { "" } else { "\n" };
            println!(
                "{head}{newline}[capped: full output at {} ({} bytes)]",
                path.display(),
                rendered.len()
            );
        }
        Err(e) => {
            print!("{rendered}");
            eprintln!("conclave: output cap: {e}");
        }
    }
}

/// The bounded terminal view of a gate log: the whole thing when it is at
/// most [`GATE_PRINT_TAIL_LINES`] lines, else the first
/// [`GATE_PRINT_HEAD_LINES`] lines, an omission marker, and the last
/// [`GATE_PRINT_TAIL_LINES`] lines. Always newline-terminated when non-empty.
fn gate_log_excerpt(combined: &str, log_path: Option<&std::path::Path>) -> String {
    if combined.is_empty() {
        return String::new();
    }
    let lines: Vec<&str> = combined.lines().collect();
    if lines.len() <= GATE_PRINT_TAIL_LINES {
        let newline = if combined.ends_with('\n') { "" } else { "\n" };
        return format!("{combined}{newline}");
    }
    let marker = |omitted: usize| match log_path {
        Some(p) => format!("[... {omitted} lines omitted — full log: {}]", p.display()),
        None => format!("[... {omitted} lines omitted ...]"),
    };
    let tail = lines[lines.len() - GATE_PRINT_TAIL_LINES..].join("\n");
    // A 10-line header only fits when it cannot overlap the 40-line tail.
    if lines.len() <= GATE_PRINT_HEAD_LINES + GATE_PRINT_TAIL_LINES {
        let omitted = lines.len() - GATE_PRINT_TAIL_LINES;
        return format!("{}\n{tail}\n", marker(omitted));
    }
    let omitted = lines.len() - GATE_PRINT_HEAD_LINES - GATE_PRINT_TAIL_LINES;
    let head = lines[..GATE_PRINT_HEAD_LINES].join("\n");
    format!("{head}\n{}\n{tail}\n", marker(omitted))
}

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

    // Task cli-output-economy: the FULL log goes to a file (so the evidence
    // survives without flooding the caller's context), the terminal gets a
    // bounded excerpt, and the gate event records where the file is. A failed
    // write is non-fatal — the gate still records with its 2000-byte tail,
    // exactly like a pre-logPath build.
    let log_path = match write_cli_output(&format!("gate-{slug}"), "log", &combined) {
        Ok(path) => Some(path),
        Err(e) => {
            eprintln!("conclave: task gate: could not persist the full log ({e})");
            None
        }
    };
    print!("{}", gate_log_excerpt(&combined, log_path.as_deref()));

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

    let mut wire = vec![
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
    ];
    if let Some(path) = &log_path {
        wire.push(path.to_string_lossy().into_owned());
    }
    Ok((wire, exit_code))
}

// ── task plan-check (lead-council v1: checkout reads run caller-side) ──────
//
// `conclave task plan-check <ws> <slug>` validates the canonical council plan
// against the task. All CHECKOUT work happens HERE (the engine may run from
// the app bundle with an unrelated cwd, so it can never read the agent's
// checkout — same reasoning as `task gate`): load the stored snapshot over
// the ordinary `task get` read surface, resolve `planPath` against THIS
// checkout's git toplevel, hash the exact plan bytes, ship the contents of
// every anchored `consumes`/`readingOrder` file, and run the `baseSha`
// ancestry check with local git. VALIDATION itself is entirely engine-side
// (`task.planCheck`), which appends the typed `plan_check` event only on
// success.
//
// The ten-line-header grammar is duplicated here only MINIMALLY (extract four
// fields, no validation): the bin target cannot import
// `engine::plan_contract` — `lib.rs` keeps `mod engine` private — and the
// engine re-validates everything anyway, so a parse disagreement can only
// produce a loud engine error, never a silently green check.

/// Ship-cap on plan + referenced-file bytes. The UDS server rejects request
/// lines over 4 MiB (`engine::uds::MAX_LINE_BYTES`); 2 MiB of raw content
/// leaves headroom for JSON escaping and the envelope.
const PLAN_CHECK_MAX_CONTENT_BYTES: usize = 2 * 1024 * 1024;

/// The four header fields the CLIENT needs before the engine validates: which
/// file to hash (`planPath`), which commit to ancestry-check (`baseSha`), and
/// which referenced files to ship (`consumes` + `readingOrder`).
struct MinPlanHeader {
    plan_path: Option<String>,
    base_sha: Option<String>,
    consumes: Vec<String>,
    reading_order: Vec<String>,
    /// Whether the header carries a `council` object. The claim-freshness
    /// preflight (edit group 5) only speaks up for council-tagged tasks —
    /// non-council claims stay noise-free.
    has_council: bool,
}

/// Best-effort extraction from a `conclave-plan:v1` ten-line header. `None`
/// when the envelope or JSON is unreadable — callers then either fail loudly
/// (no `planPath` to resolve) or ship a bare payload and let the engine's
/// strict parser produce the canonical error.
fn parse_min_plan_header(plan: &str) -> Option<MinPlanHeader> {
    let lines: Vec<&str> = plan
        .split('\n')
        .take(10)
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    if lines.len() < 10 || lines[1].trim_end() != "<!-- conclave-plan:v1" {
        return None;
    }
    // The JSON object = lines 3-9 plus the `}` that opens line 10 (the same
    // envelope shape `engine::plan_contract::parse_header` pins).
    let json_text = format!("{}\n}}", lines[2..9].join("\n"));
    let v: Value = serde_json::from_str(&json_text).ok()?;
    let strings = |key: &str| -> Vec<String> {
        v.get(key)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    Some(MinPlanHeader {
        plan_path: v
            .get("planPath")
            .and_then(Value::as_str)
            .map(str::to_string),
        base_sha: v.get("baseSha").and_then(Value::as_str).map(str::to_string),
        consumes: strings("consumes"),
        reading_order: strings("readingOrder"),
        has_council: v.get("council").is_some(),
    })
}

/// The git toplevel containing `cwd` — the checkout root every repo-relative
/// plan path resolves against (spec "Plan Check" step 2: the INVOKING
/// checkout for a direct `task plan-check`).
fn plan_check_checkout_root(cwd: &std::path::Path) -> Result<PathBuf, String> {
    let out = Command::new("git")
        .current_dir(cwd)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| format!("conclave: task plan-check: cannot run git: {e}"))?;
    if !out.status.success() {
        return Err("conclave: task plan-check: not inside a git repository".to_string());
    }
    Ok(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

/// Read `rel` under `root` with symlink/traversal containment: the
/// canonicalized target must stay inside the canonicalized root (the field
/// rule "paths reject … symlink escape from the checkout root" — the one
/// path check `plan_contract` explicitly leaves to the disk-touching side).
fn read_checkout_file(root: &std::path::Path, rel: &str) -> Result<Vec<u8>, String> {
    let root_canon = root
        .canonicalize()
        .map_err(|e| format!("cannot resolve checkout root: {e}"))?;
    let canon = root
        .join(rel)
        .canonicalize()
        .map_err(|e| format!("cannot read `{rel}`: {e}"))?;
    if !canon.starts_with(&root_canon) {
        return Err(format!(
            "`{rel}` escapes the checkout root (symlink or traversal)"
        ));
    }
    std::fs::read(&canon).map_err(|e| format!("cannot read `{rel}`: {e}"))
}

/// `git merge-base --is-ancestor <baseSha> HEAD` in `root`. Exit 0 → ancestor;
/// exit 1, an unknown sha, or any git failure → NOT an ancestor of this
/// checkout (a sha this checkout has never seen cannot be its ancestor). The
/// result rides the wire as a flag the ENGINE enforces — the engine itself
/// has no checkout to run git in, so ancestry is computed here, exactly like
/// the file bytes it accompanies.
fn base_sha_is_ancestor(root: &std::path::Path, base_sha: &str) -> bool {
    Command::new("git")
        .current_dir(root)
        .args(["merge-base", "--is-ancestor", base_sha, "HEAD"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Collect the wire payload's checkout-derived words from the CURRENT plan
/// content: the referenced-files JSON object and the ancestry flag. Pure
/// given a reader, so the path/dedupe/cap logic is unit-testable without git.
fn plan_check_files_and_total(
    header: &MinPlanHeader,
    mut read: impl FnMut(&str) -> Option<String>,
) -> (serde_json::Map<String, Value>, usize) {
    let mut files = serde_json::Map::new();
    let mut total = 0usize;
    for entry in header.consumes.iter().chain(header.reading_order.iter()) {
        // Only anchored entries need content shipped (the anchor's exact-text
        // check); plain readingOrder paths carry nothing exact to verify.
        let Some((path, _anchor)) = entry.split_once('#') else {
            continue;
        };
        if files.contains_key(path) {
            continue;
        }
        // Unreadable/missing files are OMITTED — the engine reports them as
        // the canonical validation failure (and appends no event).
        if let Some(text) = read(path) {
            total += text.len();
            files.insert(path.to_string(), Value::String(text));
        }
    }
    (files, total)
}

/// Build the fully-expanded `task plan-check` wire argv (the 10-word form the
/// `cli.exec` allowlist pins):
/// `task plan-check <actorId> <ws> <slug> <planPath> <fingerprint>
///  <ancestor 0|1> <planContent> <filesJson>`.
async fn run_task_plan_check(
    argv: &[String],
    self_instance: Option<&str>,
) -> Result<Vec<String>, String> {
    let me = self_instance.filter(|s| !s.is_empty()).ok_or_else(|| {
        "conclave: `task plan-check` is only available inside a spawned agent (CONCLAVE_INSTANCE_ID unset)"
            .to_string()
    })?;
    let usage = "conclave: task plan-check <workspaceId> <slug>";
    let (ws, slug) = match argv {
        [_, _, ws, slug] => (ws.clone(), slug.clone()),
        _ => return Err(usage.to_string()),
    };

    // The stored snapshot names the canonical `planPath` (spec steps 1-2) —
    // fetched over the same `task get` read surface every client already has.
    let got = uds_task_call(
        vec!["task".into(), "get".into(), ws.clone(), slug.clone()],
        self_instance,
    )
    .await
    .map_err(|e| format!("conclave: task plan-check: cannot load task '{slug}': {e}"))?;
    let stored_plan = got
        .pointer("/task/plan")
        .and_then(Value::as_str)
        .unwrap_or("");
    let stored = parse_min_plan_header(stored_plan).ok_or_else(|| {
        format!(
            "conclave: task plan-check: task '{slug}' has no parseable `conclave-plan:v1` header in its stored plan"
        )
    })?;
    let plan_path = stored.plan_path.ok_or_else(|| {
        "conclave: task plan-check: the stored plan header names no planPath".to_string()
    })?;

    // Exact bytes of the canonical plan in THIS checkout → SHA-256.
    let cwd = std::env::current_dir()
        .map_err(|e| format!("conclave: task plan-check: cannot resolve cwd: {e}"))?;
    let root = plan_check_checkout_root(&cwd)?;
    let plan_bytes = read_checkout_file(&root, &plan_path)
        .map_err(|e| format!("conclave: task plan-check: {e}"))?;
    let fingerprint = sha256_hex(&plan_bytes);
    let plan_content = String::from_utf8(plan_bytes)
        .map_err(|_| format!("conclave: task plan-check: `{plan_path}` is not valid UTF-8"))?;

    // Referenced-file contents + ancestry from the CURRENT header,
    // best-effort: when the current header is unparseable we ship an empty
    // map and a red flag and let the ENGINE'S strict parser produce the
    // canonical error — the engine is the validator of record, this side
    // only ships checkout bytes.
    let mut files = serde_json::Map::new();
    let mut ancestor = false;
    if let Some(current) = parse_min_plan_header(&plan_content) {
        let (shipped, files_total) = plan_check_files_and_total(&current, |path| {
            read_checkout_file(&root, path)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
        });
        if plan_content.len() + files_total > PLAN_CHECK_MAX_CONTENT_BYTES {
            return Err(format!(
                "conclave: task plan-check: plan + referenced files exceed {PLAN_CHECK_MAX_CONTENT_BYTES} bytes; the payload must stay bounded"
            ));
        }
        files = shipped;
        if let Some(base_sha) = &current.base_sha {
            ancestor = base_sha_is_ancestor(&root, base_sha);
        }
    }

    Ok(vec![
        "task".to_string(),
        "plan-check".to_string(),
        me.to_string(),
        ws,
        slug,
        plan_path,
        fingerprint,
        if ancestor { "1" } else { "0" }.to_string(),
        plan_content,
        Value::Object(files).to_string(),
    ])
}

/// Render the engine's bounded `task.planCheck` success packet — counts and
/// the fingerprint, never the plan text (spec: "The command does not print
/// the full plan").
fn render_plan_check_result(result: &Value) -> String {
    let s = |k: &str| result.get(k).and_then(Value::as_str).unwrap_or("?");
    let n = |k: &str| result.get(k).and_then(Value::as_i64).unwrap_or(-1);
    format!(
        "plan-check OK: {}\n  plan: {}\n  fingerprint: {}\n  boundary={} anchors={} gates={}\n",
        s("slug"),
        s("planPath"),
        s("fingerprint"),
        n("boundaryCount"),
        n("anchorCount"),
        n("gateCount"),
    )
}

// ── task claim freshness preflight (lead-council v1, edit group 5) ─────────
//
// RULING 2987c0b6 (binding): freshness is a CLI-LOCAL preflight over the
// EXISTING task read surface (`task get`) and the newest typed `plan_check`
// event; the CLI then sends the existing five-word `task claim` argv
// UNCHANGED — no fingerprint argv word, no new `ClaimReq` field. A missing or
// unverifiable preflight WARNS (stderr only) and the claim proceeds; V1 never
// refuses a claim. This preserves compatibility with the pinned `cli.exec`
// allowlist and old running engines.
//
// INTEGRATION NOTE (release): after installing this build, restart the
// Conclave app IMMEDIATELY before using `task plan-check` or task
// create-with-watchers — both need the new engine surface (migration 0018 +
// `task.planCheck`). The existing `task claim` verb keeps working across
// version skew: against an old engine this preflight is merely unverifiable —
// it warns and the ordinary claim still proceeds.

/// Outcome of the claim-freshness preflight (spec "Freshness Warning").
#[derive(Debug, PartialEq)]
enum PlanFreshness {
    /// Not a council-tagged task (no parseable `conclave-plan:v1` header, or
    /// a header without a `council` object) — no preflight noise at all.
    NotCouncil,
    /// Council-tagged and everything matches — silent too.
    Fresh,
    /// Council-tagged with freshness problems, or the engine answered with a
    /// malformed envelope — warn loudly on stderr, never block the claim.
    Warn(Vec<String>),
    /// `task get` itself failed (app down, task absent, old engine):
    /// council-ness is unknowable. A direct claim still warns loudly, but
    /// `lane start` suppresses this variant — its claim is about to soft-fail
    /// for the same reason with the documented calm "skipping task claim"
    /// note, and a loud unverifiable warn on top of it is pure noise on every
    /// pre-task and app-down lane start (ruling be2e68d7 F1).
    Unverifiable(Vec<String>),
}

/// The canonical ten-line header block: the RAW BYTES up to (not including)
/// the newline terminating line 10 — the minimal client-side mirror of
/// `engine::plan_contract::canonical_header` (the bin cannot import it; same
/// reasoning as [`parse_min_plan_header`], and this side only ever WARNS, so
/// a disagreement can never wrongly pass the engine's validator of record).
/// NO normalization: the header contract is byte-for-byte (ruling on
/// challenge d4bf5714), so a CRLF header differs from its LF twin here too.
/// `None` under ten lines.
fn min_canonical_header(plan: &str) -> Option<&str> {
    let mut newlines = 0usize;
    for (i, b) in plan.bytes().enumerate() {
        if b == b'\n' {
            newlines += 1;
            if newlines == 10 {
                return Some(&plan[..i]);
            }
        }
    }
    // Exactly ten lines with an unterminated tenth still counts.
    (newlines == 9 && !plan.is_empty()).then_some(plan)
}

/// The newest successful `plan_check` fingerprint from a `task get`
/// envelope. The engine appends the typed event ONLY on a successful check,
/// so presence means success.
///
/// Prefers the envelope's `latestPlanCheck` field — a dedicated by-kind
/// lookup that survives any number of newer events (ruling be2e68d7 F3: the
/// `events` window is capped at the last 20, so scanning it false-warns "no
/// plan-check" on any council task that accrued 20+ events after its newest
/// check). When the field is absent (old engine) it falls back to scanning
/// the capped window — exactly the pre-F3 behavior, warn-only either way.
fn latest_plan_check_fingerprint(envelope: &Value) -> Option<String> {
    if let Some(latest) = envelope.get("latestPlanCheck") {
        // Present means the engine performed the dedicated lookup; `null`
        // is an authoritative "no successful plan-check exists".
        return latest
            .pointer("/payload/planFingerprint")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    envelope
        .get("events")?
        .as_array()?
        .iter()
        .find(|e| e.get("kind").and_then(Value::as_str) == Some("plan_check"))?
        .pointer("/payload/planFingerprint")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// First 12 hex chars of a fingerprint for one-line warning text.
fn short_fp(fp: &str) -> &str {
    fp.get(..12).unwrap_or(fp)
}

/// Assess plan freshness for a claim from a `task get` result and a plan
/// reader bound to the checkout the implementer will READ (the invoking
/// checkout for a direct claim; the fresh lane worktree for `lane start`).
/// Pure given the reader, so every warning path is unit-testable.
fn assess_plan_freshness(
    task_get: &Result<Value, String>,
    read_plan: impl Fn(&str) -> Result<Vec<u8>, String>,
) -> PlanFreshness {
    // Unverifiable read surface: council status itself is unknowable, and
    // the claim always proceeds (spec: "the preflight is unverifiable and
    // warns, but the ordinary claim still proceeds"). A failed `task get`
    // (engine down, task absent, old engine) is `Unverifiable` so `lane
    // start` can stay calm about it; a malformed response from a LIVE engine
    // stays a loud `Warn` on every path.
    let envelope = match task_get {
        Ok(v) => v,
        Err(e) => {
            return PlanFreshness::Unverifiable(vec![format!(
                "the preflight could not be verified: reading the task failed ({e})"
            )]);
        }
    };
    let Some(task) = envelope.get("task") else {
        return PlanFreshness::Warn(vec![
            "the preflight could not be verified: malformed `task get` response (no task object)"
                .to_string(),
        ]);
    };
    let stored_plan = task.get("plan").and_then(Value::as_str).unwrap_or("");
    // No parseable v1 header (incl. legacy free-text plans) or no `council`
    // object: not council-tagged — stay silent.
    let Some(stored) = parse_min_plan_header(stored_plan) else {
        return PlanFreshness::NotCouncil;
    };
    if !stored.has_council {
        return PlanFreshness::NotCouncil;
    }

    let mut problems = Vec::new();
    let checked = latest_plan_check_fingerprint(envelope);
    if checked.is_none() {
        problems.push(
            "no successful `task plan-check` event exists on this task — run \
             `conclave task plan-check <ws> <slug>` before claiming"
                .to_string(),
        );
    }
    match &stored.plan_path {
        None => problems.push(
            "the stored plan header names no planPath; the canonical plan cannot be resolved"
                .to_string(),
        ),
        Some(plan_path) => match read_plan(plan_path) {
            Err(e) => {
                problems.push(format!("cannot read the canonical plan `{plan_path}`: {e}"));
            }
            Ok(bytes) => {
                let fingerprint = sha256_hex(&bytes);
                if let Some(checked) = &checked {
                    if *checked != fingerprint {
                        problems.push(format!(
                            "the plan changed since its last successful plan-check \
                             (checked {}…, this checkout now hashes to {}…)",
                            short_fp(checked),
                            short_fp(&fingerprint)
                        ));
                    }
                }
                let current = String::from_utf8_lossy(&bytes);
                if min_canonical_header(&current) != min_canonical_header(stored_plan) {
                    problems.push(
                        "the execution header in this checkout differs from the immutable \
                         stored header"
                            .to_string(),
                    );
                }
            }
        },
    }
    if problems.is_empty() {
        PlanFreshness::Fresh
    } else {
        PlanFreshness::Warn(problems)
    }
}

/// Print the loud stderr block for a stale/unverifiable preflight. Every line
/// carries the `warning:` prefix (the bin's stderr convention is prefixed
/// one-liners; a fenced multi-line block is what makes this unmissable in an
/// agent transcript). stdout is untouched — scripts parse claim/lane-start
/// stdout — and the claim itself is never blocked.
fn print_plan_freshness_warning(ws: &str, slug: &str, problems: &[String]) {
    eprintln!("warning: ================ PLAN FRESHNESS WARNING ================");
    eprintln!("warning: task '{slug}' (workspace '{ws}'):");
    for p in problems {
        eprintln!("warning:   - {p}");
    }
    eprintln!("warning: The claim proceeds anyway. Verify you are reading the");
    eprintln!("warning: canonical plan, then re-run `conclave task plan-check`");
    eprintln!("warning: before implementing.");
    eprintln!("warning: ========================================================");
}

/// `Some((ws, slug))` only for the exact public `task claim <ws> <slug>`
/// form. Anything else (wrong arity, already-expanded wire forms, other
/// verbs) skips the preflight and falls through to the ordinary paths
/// untouched — the preflight must never change what gets sent or rejected.
fn direct_claim_target(argv: &[String]) -> Option<(&str, &str)> {
    match argv {
        [cmd, sub, ws, slug] if cmd == "task" && sub == "claim" => {
            Some((ws.as_str(), slug.as_str()))
        }
        _ => None,
    }
}

/// The lane worktree directory `lane start` creates (relative to the
/// invoking cwd, exactly like `lane_start`'s `git worktree add` path) — the
/// checkout root the lane preflight hashes against.
fn lane_worktree_root(slug: &str) -> PathBuf {
    PathBuf::from(format!(".claude/worktrees/{slug}"))
}

/// Run the freshness preflight before a claim: load the task over the
/// EXISTING `task get` surface, recompute the plan fingerprint in
/// `checkout_root` (`None` = resolve the INVOKING checkout's git toplevel,
/// for a direct claim; `Some` = an explicit root, the fresh worktree for
/// `lane start`), and warn on stderr when anything is off. Infallible by
/// design — whatever happens here, the caller sends the existing five-word
/// claim argv unchanged afterwards.
/// The problem lines a preflight outcome should print, if any. Pure, so the
/// lane-vs-direct suppression rule is unit-testable: `suppress_unverifiable`
/// (the `lane start` path) silences only [`PlanFreshness::Unverifiable`] —
/// real freshness problems stay loud on every path (ruling be2e68d7 F1).
fn freshness_problems_to_print(
    outcome: PlanFreshness,
    suppress_unverifiable: bool,
) -> Option<Vec<String>> {
    match outcome {
        PlanFreshness::Warn(problems) => Some(problems),
        PlanFreshness::Unverifiable(_) if suppress_unverifiable => None,
        PlanFreshness::Unverifiable(problems) => Some(problems),
        PlanFreshness::NotCouncil | PlanFreshness::Fresh => None,
    }
}

async fn claim_freshness_preflight(
    ws: &str,
    slug: &str,
    checkout_root: Option<&std::path::Path>,
    self_instance: Option<&str>,
    suppress_unverifiable: bool,
) {
    let got = uds_task_call(
        vec![
            "task".to_string(),
            "get".to_string(),
            ws.to_string(),
            slug.to_string(),
        ],
        self_instance,
    )
    .await;
    let read_plan = |rel: &str| -> Result<Vec<u8>, String> {
        let root = match checkout_root {
            Some(r) => r.to_path_buf(),
            None => {
                let cwd =
                    std::env::current_dir().map_err(|e| format!("cannot resolve cwd: {e}"))?;
                plan_check_checkout_root(&cwd)?
            }
        };
        read_checkout_file(&root, rel)
    };
    if let Some(problems) = freshness_problems_to_print(
        assess_plan_freshness(&got, read_plan),
        suppress_unverifiable,
    ) {
        print_plan_freshness_warning(ws, slug, &problems);
    }
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
    /// A compact fresh-context orientation packet (`orient`).
    Orient,
    /// One-line gate confirmation (`task gate`) — the log excerpt already
    /// printed client-side; echoing the event JSON would re-send the tail.
    GateResult,
    /// The bounded plan-check packet (`task plan-check`) — counts and the
    /// fingerprint, never the plan text.
    PlanCheck,
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

/// The orient packet as a compact text block (task conclave-orient) — one
/// line per row, transcript-style messages. Pretty-printing the same packet
/// as JSON measured ~7.2KB on 2026-07-09 data, over the plan's ≤6KB target;
/// this rendering carries the same fields at roughly half the bytes. The
/// WIRE stays the structured JSON packet for any future UI consumer.
fn render_orient(result: &Value) -> String {
    let rows = |key: &str, result: &Value| -> Vec<Value> {
        result
            .get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let mut out = String::new();

    if let Some(me) = result.get("self").filter(|v| !v.is_null()) {
        let name = me.get("name").and_then(Value::as_str).unwrap_or("?");
        let level = me.get("level").and_then(Value::as_str).unwrap_or("-");
        let id = me.get("id").and_then(Value::as_str).unwrap_or("-");
        out.push_str(&format!("you: {name} ({level})  {id}"));
        if let Some(pct) = me.get("contextPct").and_then(Value::as_i64) {
            out.push_str(&format!("  context={pct}%"));
        }
        out.push('\n');
    }

    let tasks = rows("tasks", result);
    let total = result
        .get("tasksTotal")
        .and_then(Value::as_u64)
        .unwrap_or(tasks.len() as u64);
    let shown = if (tasks.len() as u64) < total {
        format!(", first {}", tasks.len())
    } else {
        String::new()
    };
    out.push_str(&format!("\ntasks ({total} live{shown}):\n"));
    if tasks.is_empty() {
        out.push_str("  (none)\n");
    }
    for t in &tasks {
        let field = |k: &str| t.get(k).and_then(Value::as_str).unwrap_or("-");
        let implementer = t
            .get("implementerAgentId")
            .and_then(Value::as_str)
            .map(short_id)
            .unwrap_or("-");
        let challenges = t.get("openChallenges").and_then(Value::as_u64).unwrap_or(0);
        let marker = if challenges > 0 {
            format!("  [open challenges: {challenges}]")
        } else {
            String::new()
        };
        out.push_str(&format!(
            "  {}  {}  impl={implementer}  {}{marker}\n",
            field("slug"),
            field("state"),
            field("title"),
        ));
    }

    let roster = rows("roster", result);
    out.push_str(&format!("\nroster ({}):\n", roster.len()));
    for a in &roster {
        let field = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("-");
        let working = match a.get("working").and_then(Value::as_bool) {
            Some(true) => "working",
            Some(false) => "idle",
            None => "offline",
        };
        out.push_str(&format!(
            "  {} ({})  {}  {working}  {}\n",
            field("name"),
            field("level"),
            field("id"),
            field("model"),
        ));
    }

    let messages = rows("messages", result);
    out.push_str(&format!("\nmessages (latest {}):\n", messages.len()));
    if messages.is_empty() {
        out.push_str("  (none)\n");
    }
    for m in &messages {
        let time = m
            .get("createdAt")
            .and_then(Value::as_str)
            .and_then(|ts| ts.get(11..16))
            .unwrap_or("--:--");
        let field = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or("?");
        out.push_str(&format!(
            "  {time}  {} → {}  {}\n",
            field("from"),
            field("to"),
            field("text"),
        ));
    }

    let blackboard = rows("blackboard", result);
    out.push_str(&format!("\nblackboard ({}):\n", blackboard.len()));
    for entry in &blackboard {
        let field = |k: &str| entry.get(k).and_then(Value::as_str).unwrap_or("");
        out.push_str(&format!("  {} = {}\n", field("key"), field("value")));
    }

    let watches: Vec<String> = rows("watches", result)
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    out.push_str(&format!(
        "\nwatches: {}\n",
        if watches.is_empty() {
            "(none)".to_string()
        } else {
            watches.join(", ")
        }
    ));

    out
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
        // Council fields (lead-council v1) in a stable order; the engine
        // already char-caps each value, and a line only appears when the
        // brief packet carries a non-empty value.
        for (label, key) in [
            ("by", "actorAgentId"),
            ("evidence", "evidence"),
            ("proposal", "proposal"),
            ("default", "default"),
        ] {
            if let Some(value) = challenge.get(key).and_then(Value::as_str) {
                if !value.is_empty() {
                    out.push_str(&format!("    {label}: {value}\n"));
                }
            }
        }
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
/// One line for a recorded gate event: cmd, exit, pinned SHA, and where the
/// full log landed. The 40-line excerpt already printed while the command
/// ran; echoing the event JSON here would just re-send the 2000-byte tail.
fn render_gate_result(result: &Value) -> String {
    let payload = result.get("payload");
    let get_str = |k: &str| {
        payload
            .and_then(|p| p.get(k))
            .and_then(Value::as_str)
            .unwrap_or("?")
    };
    let exit = payload
        .and_then(|p| p.get("exit"))
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    let sha = get_str("sha");
    let mut line = format!(
        "gate recorded: {} exit={exit} sha={}",
        get_str("cmd"),
        short_id(sha)
    );
    if let Some(log_path) = payload
        .and_then(|p| p.get("logPath"))
        .and_then(Value::as_str)
    {
        line.push_str(&format!(" log={log_path}"));
    }
    line.push('\n');
    line
}

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

/// `conclave rtk-hook --rtk <abs-path>` — the Claude Code PreToolUse hook
/// body. LOCAL by design: it never touches the engine socket, so it keeps
/// working while the engine is busy or restarting. Reads the PreToolUse JSON
/// from stdin, runs `<rtk> rewrite <command>`, and prints the hook response
/// when a rewrite applies. Fail-open by contract: every error path (missing
/// flag, unreadable/malformed stdin, no command, rtk spawn failure) exits 0
/// with no output so the agent's Bash tool never breaks.
fn run_rtk_hook(rest: &[String]) -> ExitCode {
    let rtk = match rest {
        [flag, path, ..] if flag == "--rtk" => PathBuf::from(path),
        _ => return ExitCode::SUCCESS,
    };
    let mut stdin_text = String::new();
    if std::io::Read::read_to_string(&mut std::io::stdin(), &mut stdin_text).is_err() {
        return ExitCode::SUCCESS;
    }
    let Ok(input) = serde_json::from_str::<Value>(&stdin_text) else {
        return ExitCode::SUCCESS;
    };
    let Some(cmd) = extract_bash_command(&input) else {
        return ExitCode::SUCCESS;
    };
    // `output()` nulls the child's stdin and captures stderr, so a chatty or
    // prompt-happy rtk can neither hang the hook nor leak into the protocol.
    let Ok(output) = Command::new(&rtk).arg("rewrite").arg(&cmd).output() else {
        return ExitCode::SUCCESS;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    // A signal-killed rtk has no exit code; -1 falls into the unknown-code
    // pass-through arm.
    let exit_code = output.status.code().unwrap_or(-1);
    if let Some(response) = rtk_hook_response(&input, exit_code, &stdout) {
        // `Value`'s Display is compact JSON — exactly what the hook wants.
        println!("{response}");
    }
    ExitCode::SUCCESS
}

/// Extract `.tool_input.command` from a Claude Code PreToolUse payload.
/// `None` when the path is missing, not a string, or empty — the caller
/// treats every `None` as "emit nothing, exit 0" (fail-open).
fn extract_bash_command(input: &Value) -> Option<String> {
    let cmd = input.get("tool_input")?.get("command")?.as_str()?;
    if cmd.is_empty() {
        None
    } else {
        Some(cmd.to_string())
    }
}

/// Observation-critical rtk subcommands whose rendering is lossy-and-false
/// under rtk 0.42.4 (task rtk-hook-denylist; diagnosis on task
/// rtk-output-distortion): `rtk grep` fabricates "N matches in 0 files" and
/// drops every match line when rg is absent from PATH, rg flags collide with
/// rtk grep's own (`-l` = max-len, not files-with-matches), and `rtk ls` is a
/// git-aware lister that hides gitignored entries. A rewrite that routes any
/// of these through rtk is dropped so the RAW command runs unfiltered.
const RTK_DENYLISTED_VERBS: [&str; 5] = ["grep", "ls", "tree", "find", "read"];

/// True when `rewritten` invokes a denylisted `rtk <verb>` at any command
/// position: start of string or after `&&`, `||`, `;`, `|`, `$(`. Matched on
/// the REWRITTEN side — `rtk rewrite` is the single source of truth for the
/// source→rtk mapping, so every source spelling (ls, rg, grep, fd, cat…) is
/// caught without re-parsing the source command here (plan risk ledger).
fn rewrite_is_denylisted(rewritten: &str) -> bool {
    // Longer separators split first so `&&`/`||` are consumed whole and the
    // later `|` pass only sees genuine pipes.
    let mut segments = vec![rewritten];
    for sep in ["&&", "||", ";", "|", "$("] {
        segments = segments.into_iter().flat_map(|s| s.split(sep)).collect();
    }
    segments.into_iter().any(|seg| {
        let mut toks = seg.split_whitespace();
        toks.next() == Some("rtk")
            && toks
                .next()
                // `$(rtk ls)` leaves the closing paren glued to the verb.
                .is_some_and(|v| RTK_DENYLISTED_VERBS.contains(&v.trim_end_matches(')')))
    })
}

/// Map one `rtk rewrite` run onto the PreToolUse hook response (rtk >= 0.23.0
/// exit-code contract): 0 = rewrite (auto-allow), 3 = rewrite but let Claude
/// Code prompt the user (no `permissionDecision`), 1/2/anything else = pass
/// through. `None` means "emit nothing, exit 0". An empty or identical
/// rewrite is also `None`: emitting it would clobber or churn the command.
fn rtk_hook_response(input: &Value, exit_code: i32, stdout: &str) -> Option<Value> {
    if exit_code != 0 && exit_code != 3 {
        return None;
    }
    let original = extract_bash_command(input)?;
    let rewritten = stdout.trim_end_matches('\n');
    if rewritten.is_empty() || rewritten == original {
        return None;
    }
    if rewrite_is_denylisted(rewritten) {
        return None;
    }
    // `updatedInput` replaces tool_input wholesale, so carry every sibling
    // key (description, timeout, …) and swap only `command`.
    let mut updated = input["tool_input"].clone();
    updated["command"] = json!(rewritten);
    let output = if exit_code == 0 {
        json!({
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "permissionDecisionReason": "RTK auto-rewrite",
            "updatedInput": updated,
        })
    } else {
        json!({
            "hookEventName": "PreToolUse",
            "updatedInput": updated,
        })
    };
    Some(json!({ "hookSpecificOutput": output }))
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

    // `rtk-hook` is fully LOCAL (Claude Code PreToolUse hook body): it must
    // keep working while the engine is busy or restarting, so it never
    // touches the socket at all.
    if argv[0] == "rtk-hook" {
        return run_rtk_hook(&argv[1..]);
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
    } else if argv.first().map(String::as_str) == Some("task")
        && argv.get(1).map(String::as_str) == Some("plan-check")
    {
        // All checkout reads (plan bytes, SHA-256, referenced files, git
        // ancestry) run HERE, then the expanded wire argv goes to the engine
        // for validation — same client-side split as `task gate`.
        match run_task_plan_check(&argv, self_instance.as_deref()).await {
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

    // Freshness preflight before a direct `task claim` (lead-council v1,
    // RULING 2987c0b6): CLI-local and warning-only, hashed against the
    // INVOKING checkout. The claim argv below is sent UNCHANGED (the same
    // five-word wire form after expansion), so the pinned allowlist and old
    // running engines keep working. See `claim_freshness_preflight` for the
    // release/version-skew integration note.
    if let Some((ws, slug)) = direct_claim_target(&argv) {
        claim_freshness_preflight(ws, slug, None, self_instance.as_deref(), false).await;
    }

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
    } else if argv.first().map(String::as_str) == Some("task") && sub == Some("plan-check") {
        OutMode::PlanCheck
    } else if argv.first().map(String::as_str) == Some("orient") {
        OutMode::Orient
    } else if gate_exit_code.is_some() {
        OutMode::GateResult
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

    // Captured before `argv` moves into the request: the label a capped
    // response's spill file is named after.
    let out_label = verb_label(&argv);

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
        // Each arm RENDERS; the print happens once below, through the global
        // output cap (task cli-output-economy). The `snapshot` family is
        // exempt — a restored handoff must arrive whole.
        let rendered = match out_mode {
            // Terse: `tell` → "delivered -> <to>"; `snapshot save` → "saved <id>".
            // Never echo the payload text back into the agent's context.
            OutMode::Terse => {
                if let Some(to) = result.get("toInstanceId").and_then(Value::as_str) {
                    let status = result
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("sent");
                    format!("{status} -> {to}\n")
                } else {
                    let id = result.get("id").and_then(Value::as_str).unwrap_or("");
                    format!("saved snapshot {id}\n")
                }
            }
            // The carried-forward handoff — the whole point of `snapshot last` is
            // for the agent to READ this, so print the content, not the JSON row.
            // If the latest snapshot somehow carries no handoff text, fail LOUDLY
            // (non-zero) rather than print a placeholder the agent would silently
            // "restore" from.
            OutMode::Handoff => match result.get("carriedForward").and_then(Value::as_str) {
                Some(content) => format!("{content}\n"),
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
                Some(text) => format!("{text}\n"),
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
                format!("{id}\n")
            }
            // `artifact list` → one line per artifact, newest first, WITHOUT the
            // body (the plan's "no content dump"). Empty workspace → a marker.
            OutMode::ArtifactList => {
                let empty: Vec<Value> = Vec::new();
                let rows = result.as_array().unwrap_or(&empty);
                if rows.is_empty() {
                    "(no artifacts)\n".to_string()
                } else {
                    let mut out = String::new();
                    for row in rows {
                        let id = row.get("id").and_then(Value::as_str).unwrap_or("");
                        let short = id.get(..8).unwrap_or(id);
                        let kind = row.get("kind").and_then(Value::as_str).unwrap_or("-");
                        let title = row.get("title").and_then(Value::as_str).unwrap_or("-");
                        let agent = row.get("agentId").and_then(Value::as_str).unwrap_or("-");
                        let created = row.get("createdAt").and_then(Value::as_str).unwrap_or("-");
                        out.push_str(&format!(
                            "{short}  {kind:<8}  {title}  ({agent}, {created})\n"
                        ));
                    }
                    out
                }
            }
            // `artifact get` → a short metadata header, a blank line, then the
            // full body verbatim to stdout.
            OutMode::ArtifactGet => {
                let field = |k: &str| result.get(k).and_then(Value::as_str).unwrap_or("-");
                let mut out = String::new();
                out.push_str(&format!("id:       {}\n", field("id")));
                out.push_str(&format!("title:    {}\n", field("title")));
                out.push_str(&format!("kind:     {}\n", field("kind")));
                out.push_str(&format!("agent:    {}\n", field("agentId")));
                if let Some(f) = result.get("filename").and_then(Value::as_str) {
                    out.push_str(&format!("filename: {f}\n"));
                }
                out.push_str(&format!("created:  {}\n", field("createdAt")));
                out.push('\n');
                out.push_str(&format!(
                    "{}\n",
                    result.get("content").and_then(Value::as_str).unwrap_or("")
                ));
                out
            }
            // `position set` → one line describing the updated agent.
            OutMode::PositionRow => {
                format!("{}\n", render_position_row(result))
            }
            // `org` → the workspace supervisor forest as an indented tree.
            OutMode::OrgTree => {
                let empty: Vec<Value> = Vec::new();
                let rows = result.as_array().unwrap_or(&empty);
                render_org_tree(rows)
            }
            // `msg list` / `msg all` → a chronological transcript, not the JSON
            // array of UUID-keyed rows (which defeats the point of a re-read).
            OutMode::MsgList => {
                let empty: Vec<Value> = Vec::new();
                let rows = result.as_array().unwrap_or(&empty);
                render_msg_transcript(rows)
            }
            OutMode::TaskBrief => render_task_brief(result),
            // `orient` → the compact text packet; pretty JSON of the same
            // data overshoots the plan's byte target (see `render_orient`).
            OutMode::Orient => render_orient(result),
            // `task gate` → one confirmation line; the log excerpt already
            // printed client-side and the tail is on the ledger.
            OutMode::GateResult => render_gate_result(result),
            // `task plan-check` → the bounded success packet (spec: the
            // command does not print the full plan).
            OutMode::PlanCheck => render_plan_check_result(result),
            OutMode::Json => {
                let pretty =
                    serde_json::to_string_pretty(result).expect("serialize result cannot fail");
                format!("{pretty}\n")
            }
        };
        emit_capped(&out_label, &rendered, cap_disabled() || is_snapshot);
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
    use super::{
        expand_self_args, parse_min_plan_header, plan_check_files_and_total, read_checkout_file,
        render_plan_check_result, sha256_hex, validate_slug, GUARD_HOOK, GUARD_MARKER,
    };
    use std::path::{Path, PathBuf};

    fn v(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    // ── output economy (task cli-output-economy) ────────────────────────────

    #[test]
    fn head_bytes_passes_short_text_through_and_cuts_lossily() {
        assert_eq!(super::head_bytes("short", 10), "short");
        assert_eq!(super::head_bytes("abcdef", 3), "abc");
        // 'ก' is 3 bytes; cutting mid-codepoint must stay lossy, not panic.
        let cut = super::head_bytes("กข", 4);
        assert!(cut.starts_with('ก'));
    }

    #[test]
    fn gate_log_excerpt_short_log_prints_whole_and_newline_terminated() {
        assert_eq!(super::gate_log_excerpt("", None), "");
        assert_eq!(super::gate_log_excerpt("one\ntwo", None), "one\ntwo\n");
        assert_eq!(super::gate_log_excerpt("one\ntwo\n", None), "one\ntwo\n");
    }

    #[test]
    fn gate_log_excerpt_medium_log_keeps_tail_only() {
        // 45 lines: a 10-line header would overlap the 40-line tail.
        let log = (1..=45).map(|i| format!("line{i}\n")).collect::<String>();
        let excerpt = super::gate_log_excerpt(&log, None);
        assert!(excerpt.starts_with("[... 5 lines omitted ...]\nline6\n"));
        assert!(excerpt.ends_with("line45\n"));
        assert!(!excerpt.contains("line5\n"), "overlap lines appear once");
    }

    #[test]
    fn gate_log_excerpt_long_log_has_head_marker_and_tail() {
        let log = (1..=100).map(|i| format!("line{i}\n")).collect::<String>();
        let excerpt = super::gate_log_excerpt(&log, Some(Path::new("/logs/g.log")));
        assert!(excerpt.starts_with("line1\n"));
        assert!(
            excerpt.contains("line10\n[... 50 lines omitted — full log: /logs/g.log]\nline61\n")
        );
        assert!(excerpt.ends_with("line100\n"));
        assert!(
            !excerpt.contains("line60\n"),
            "omitted middle stays omitted"
        );
    }

    #[test]
    fn verb_label_is_filesystem_safe() {
        assert_eq!(super::verb_label(&v(&["task", "list", "ws1"])), "task-list");
        assert_eq!(super::verb_label(&v(&["bb", "get"])), "bb-get");
        assert_eq!(super::verb_label(&v(&["ws"])), "ws");
        assert_eq!(super::verb_label(&v(&["a/b", "c d"])), "a-b-c-d");
        assert_eq!(super::verb_label(&[]), "output");
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
    fn task_create_watchers_survive_owner_default_expansion() {
        // --watchers passes through untouched; --owner self is still appended.
        let out = expand_self_args(
            v(&["task", "create", "ws1", "t1", "Title", "--watchers", "a,b"]),
            Some("self1"),
        )
        .unwrap();
        assert_eq!(
            out,
            v(&[
                "task",
                "create",
                "ws1",
                "t1",
                "Title",
                "--watchers",
                "a,b",
                "--owner",
                "self1"
            ])
        );
    }

    #[test]
    fn task_create_watchers_survive_with_explicit_owner() {
        let argv = v(&[
            "task",
            "create",
            "ws1",
            "t1",
            "Title",
            "--owner",
            "other",
            "--watchers",
            "a,b",
        ]);
        let out = expand_self_args(argv.clone(), Some("self1")).unwrap();
        assert_eq!(
            out, argv,
            "explicit owner + watchers must pass through as-is"
        );
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

    #[test]
    fn task_plan_check_passes_through_expand_self_args_untouched() {
        // run_task_plan_check (called earlier in main()) already filled the
        // actorId slot and built the full 10-word wire form — mirror `gate`.
        let argv = v(&[
            "task",
            "plan-check",
            "self1",
            "ws1",
            "t1",
            "docs/plans/x.md",
            "deadbeef",
            "1",
            "# plan",
            "{}",
        ]);
        assert_eq!(expand_self_args(argv.clone(), None).unwrap(), argv);
    }

    // ── task plan-check client helpers ─────────────────────────────────────

    const MIN_HEADER_PLAN: &str = "# Example Task\n\
        <!-- conclave-plan:v1\n\
        {\n\
        \"owner\":\"o-1\",\"authority\":\"in-loop\",\n\
        \"planPath\":\"docs/plans/x.md\",\"baseSha\":\"abc\",\"escalation\":\"o-1\",\n\
        \"readingOrder\":[\"docs/spec.md#Rules\",\"docs/plans/x.md\"],\n\
        \"boundary\":[\"docs/plans/x.md\"],\n\
        \"consumes\":[\"docs/spec.md#Rules\",\"src/a.rs#fn a\"],\n\
        \"produces\":[],\"gates\":[\"git diff --check\"]\n\
        } -->\n\nbody\n";

    #[test]
    fn parse_min_plan_header_extracts_the_four_client_fields() {
        let h = parse_min_plan_header(MIN_HEADER_PLAN).expect("header parses");
        assert_eq!(h.plan_path.as_deref(), Some("docs/plans/x.md"));
        assert_eq!(h.base_sha.as_deref(), Some("abc"));
        assert_eq!(h.consumes, vec!["docs/spec.md#Rules", "src/a.rs#fn a"]);
        assert_eq!(
            h.reading_order,
            vec!["docs/spec.md#Rules", "docs/plans/x.md"]
        );
    }

    #[test]
    fn parse_min_plan_header_handles_crlf_and_rejects_non_contract_plans() {
        let crlf = MIN_HEADER_PLAN.replace('\n', "\r\n");
        assert!(parse_min_plan_header(&crlf).is_some());
        assert!(parse_min_plan_header("").is_none());
        assert!(parse_min_plan_header("just prose\nno header\n").is_none());
        let short = "# T\n<!-- conclave-plan:v1\n{\n} -->\n";
        assert!(parse_min_plan_header(short).is_none(), "under ten lines");
    }

    #[test]
    fn plan_check_files_ships_anchored_entries_once_and_skips_plain_paths() {
        let h = parse_min_plan_header(MIN_HEADER_PLAN).expect("header parses");
        let mut reads: Vec<String> = Vec::new();
        let (files, total) = plan_check_files_and_total(&h, |path| {
            reads.push(path.to_string());
            match path {
                "docs/spec.md" => Some("## Rules\n".to_string()),
                "src/a.rs" => Some("fn a() {}\n".to_string()),
                _ => None,
            }
        });
        // docs/spec.md appears in BOTH consumes and readingOrder — read once.
        assert_eq!(reads, vec!["docs/spec.md", "src/a.rs"]);
        assert_eq!(files.len(), 2);
        assert_eq!(files["docs/spec.md"], serde_json::json!("## Rules\n"));
        assert_eq!(total, "## Rules\n".len() + "fn a() {}\n".len());
    }

    #[test]
    fn plan_check_files_omits_unreadable_entries_for_the_engine_to_reject() {
        let h = parse_min_plan_header(MIN_HEADER_PLAN).expect("header parses");
        let (files, total) = plan_check_files_and_total(&h, |_| None);
        assert!(files.is_empty());
        assert_eq!(total, 0);
    }

    #[test]
    fn read_checkout_file_reads_inside_and_rejects_traversal_and_symlink_escape() {
        let root =
            std::env::temp_dir().join(format!("conclave-plan-check-read-{}", std::process::id()));
        let outside = std::env::temp_dir().join(format!(
            "conclave-plan-check-outside-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(root.join("docs")).expect("mkdir root/docs");
        std::fs::create_dir_all(&outside).expect("mkdir outside");
        std::fs::write(root.join("docs/x.md"), "inside").expect("write inside");
        std::fs::write(outside.join("secret.md"), "outside").expect("write outside");
        std::os::unix::fs::symlink(outside.join("secret.md"), root.join("docs/link.md"))
            .expect("symlink");

        assert_eq!(
            read_checkout_file(&root, "docs/x.md").expect("read inside"),
            b"inside".to_vec()
        );
        assert!(
            read_checkout_file(&root, "../secret.md").is_err(),
            "traversal must be rejected"
        );
        assert!(
            read_checkout_file(&root, "docs/link.md")
                .unwrap_err()
                .contains("escapes the checkout root"),
            "a symlink pointing outside the root must be rejected"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn sha256_hex_matches_the_known_empty_and_abc_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn render_plan_check_result_is_bounded_and_never_echoes_the_plan() {
        let rendered = render_plan_check_result(&serde_json::json!({
            "slug": "t1",
            "planPath": "docs/plans/x.md",
            "fingerprint": "ba7816bf8f01",
            "boundaryCount": 3,
            "anchorCount": 5,
            "gateCount": 2,
        }));
        assert_eq!(
            rendered,
            "plan-check OK: t1\n  plan: docs/plans/x.md\n  fingerprint: ba7816bf8f01\n  boundary=3 anchors=5 gates=2\n"
        );
    }

    // ── task claim freshness preflight (edit group 5) ──────────────────────

    /// A council-tagged plan: identical shape to [`MIN_HEADER_PLAN`] plus the
    /// `council` object on header line 4 (what tags a task for the preflight).
    const COUNCIL_PLAN: &str = "# Council Task\n\
        <!-- conclave-plan:v1\n\
        {\n\
        \"owner\":\"o-1\",\"authority\":\"in-loop\",\"council\":{\"chair\":\"o-1\",\"members\":[\"m-1\"]},\n\
        \"planPath\":\"docs/plans/x.md\",\"baseSha\":\"abc\",\"escalation\":\"o-1\",\n\
        \"readingOrder\":[\"docs/plans/x.md\"],\n\
        \"boundary\":[\"docs/plans/x.md\"],\n\
        \"consumes\":[],\n\
        \"produces\":[],\"gates\":[\"git diff --check\"]\n\
        } -->\n\nbody\n";

    /// The `task get` envelope shape the engine returns (`{"task": {...},
    /// "events": [...]}`, events newest-first), as the preflight's input.
    fn task_get_envelope(
        plan: &str,
        events: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({ "task": { "plan": plan }, "events": events }))
    }

    /// A typed `plan_check` event row carrying `fp` (the engine appends this
    /// payload shape only on a SUCCESSFUL check).
    fn plan_check_event(fp: &str) -> serde_json::Value {
        serde_json::json!({
            "id": "e1", "taskId": "t1", "kind": "plan_check",
            "payload": {
                "contractVersion": "conclave-plan:v1",
                "planPath": "docs/plans/x.md",
                "planFingerprint": fp,
                "baseSha": "abc",
            },
            "createdAt": "2026-07-10T00:00:00Z",
        })
    }

    #[test]
    fn parse_min_plan_header_detects_the_council_tag() {
        assert!(parse_min_plan_header(COUNCIL_PLAN).unwrap().has_council);
        assert!(!parse_min_plan_header(MIN_HEADER_PLAN).unwrap().has_council);
    }

    #[test]
    fn claim_preflight_fresh_council_plan_is_silent() {
        let fp = sha256_hex(COUNCIL_PLAN.as_bytes());
        let got = task_get_envelope(
            COUNCIL_PLAN,
            serde_json::json!([
                { "id": "e2", "kind": "note", "payload": {} },
                plan_check_event(&fp),
            ]),
        );
        let out = super::assess_plan_freshness(&got, |_| Ok(COUNCIL_PLAN.as_bytes().to_vec()));
        assert_eq!(out, super::PlanFreshness::Fresh);
    }

    #[test]
    fn preflight_prefers_the_dedicated_latest_plan_check_lookup() {
        // 20-event-cap regression (ruling be2e68d7 F3): the capped `events`
        // window carries no plan_check row — 20+ newer events pushed it out —
        // but the engine's dedicated by-kind lookup still finds it. The
        // preflight must trust the lookup and stay silent, not false-warn
        // "no successful plan-check".
        let fp = sha256_hex(COUNCIL_PLAN.as_bytes());
        let mut envelope = task_get_envelope(
            COUNCIL_PLAN,
            serde_json::json!([{ "id": "e2", "kind": "note", "payload": {} }]),
        )
        .unwrap();
        envelope["latestPlanCheck"] = plan_check_event(&fp);
        let got: Result<serde_json::Value, String> = Ok(envelope);
        let out = super::assess_plan_freshness(&got, |_| Ok(COUNCIL_PLAN.as_bytes().to_vec()));
        assert_eq!(out, super::PlanFreshness::Fresh);
    }

    #[test]
    fn preflight_treats_a_null_latest_plan_check_as_authoritative() {
        // A live F3 engine answering `latestPlanCheck: null` means "no
        // successful check exists" — the capped-window fallback is for old
        // engines that omit the key entirely, never for a live answer.
        let fp = sha256_hex(COUNCIL_PLAN.as_bytes());
        let mut envelope =
            task_get_envelope(COUNCIL_PLAN, serde_json::json!([plan_check_event(&fp)])).unwrap();
        envelope["latestPlanCheck"] = serde_json::Value::Null;
        let got: Result<serde_json::Value, String> = Ok(envelope);
        let out = super::assess_plan_freshness(&got, |_| Ok(COUNCIL_PLAN.as_bytes().to_vec()));
        match out {
            super::PlanFreshness::Warn(problems) => {
                assert!(problems[0].contains("no successful `task plan-check` event"));
            }
            other => panic!("expected a missing-event warning, got {other:?}"),
        }
    }

    #[test]
    fn claim_preflight_warns_when_the_plan_check_event_is_missing() {
        let got = task_get_envelope(COUNCIL_PLAN, serde_json::json!([]));
        let out = super::assess_plan_freshness(&got, |_| Ok(COUNCIL_PLAN.as_bytes().to_vec()));
        match out {
            super::PlanFreshness::Warn(problems) => {
                assert_eq!(problems.len(), 1);
                assert!(problems[0].contains("no successful `task plan-check` event"));
            }
            other => panic!("expected a missing-event warning, got {other:?}"),
        }
    }

    #[test]
    fn claim_preflight_unverifiable_old_engine_warns_but_never_blocks() {
        // Old engine / engine down / task absent: the read surface itself
        // errors — `Unverifiable`, so the lane path can suppress it while a
        // direct claim still warns (ruling be2e68d7 F1).
        let got: Result<serde_json::Value, String> = Err("unknown subcommand".to_string());
        match super::assess_plan_freshness(&got, |_| Ok(Vec::new())) {
            super::PlanFreshness::Unverifiable(problems) => {
                assert!(problems[0].contains("could not be verified"));
            }
            other => panic!("expected an unverifiable outcome, got {other:?}"),
        }
        // A malformed response from a LIVE engine stays a loud `Warn` — the
        // lane claim that follows would NOT soft-fail, so nothing else would
        // surface the problem.
        let malformed: Result<serde_json::Value, String> = Ok(serde_json::json!({ "ok": true }));
        match super::assess_plan_freshness(&malformed, |_| Ok(Vec::new())) {
            super::PlanFreshness::Warn(problems) => {
                assert!(problems[0].contains("could not be verified"));
            }
            other => panic!("expected an unverifiable warning, got {other:?}"),
        }
    }

    #[test]
    fn lane_start_suppresses_only_the_unverifiable_outcome() {
        let unverifiable = || super::PlanFreshness::Unverifiable(vec!["down".to_string()]);
        let warn = || super::PlanFreshness::Warn(vec!["stale".to_string()]);
        // Lane path (`suppress_unverifiable`): a soft-failed `task get` stays
        // calm — the claim's own "skipping task claim" note covers it…
        assert_eq!(
            super::freshness_problems_to_print(unverifiable(), true),
            None
        );
        // …but a REAL freshness problem still prints on the lane path.
        assert_eq!(
            super::freshness_problems_to_print(warn(), true),
            Some(vec!["stale".to_string()])
        );
        // Direct claims print both.
        assert_eq!(
            super::freshness_problems_to_print(unverifiable(), false),
            Some(vec!["down".to_string()])
        );
        assert_eq!(
            super::freshness_problems_to_print(warn(), false),
            Some(vec!["stale".to_string()])
        );
        // Silent outcomes stay silent everywhere.
        assert_eq!(
            super::freshness_problems_to_print(super::PlanFreshness::Fresh, false),
            None
        );
        assert_eq!(
            super::freshness_problems_to_print(super::PlanFreshness::NotCouncil, false),
            None
        );
    }

    #[test]
    fn claim_preflight_warns_on_a_stale_fingerprint() {
        let checked = sha256_hex(b"the plan as it was when plan-check ran");
        let got = task_get_envelope(
            COUNCIL_PLAN,
            serde_json::json!([plan_check_event(&checked)]),
        );
        let out = super::assess_plan_freshness(&got, |_| Ok(COUNCIL_PLAN.as_bytes().to_vec()));
        match out {
            super::PlanFreshness::Warn(problems) => {
                assert_eq!(problems.len(), 1);
                assert!(problems[0].contains("changed since its last successful plan-check"));
            }
            other => panic!("expected a stale-fingerprint warning, got {other:?}"),
        }
    }

    #[test]
    fn claim_preflight_warns_when_the_canonical_plan_is_unreadable() {
        let fp = sha256_hex(COUNCIL_PLAN.as_bytes());
        let got = task_get_envelope(COUNCIL_PLAN, serde_json::json!([plan_check_event(&fp)]));
        let out = super::assess_plan_freshness(&got, |rel| {
            Err(format!("cannot read `{rel}`: No such file or directory"))
        });
        match out {
            super::PlanFreshness::Warn(problems) => {
                assert_eq!(problems.len(), 1);
                assert!(problems[0].contains("cannot read the canonical plan `docs/plans/x.md`"));
            }
            other => panic!("expected an unreadable-plan warning, got {other:?}"),
        }
    }

    #[test]
    fn claim_preflight_warns_when_the_header_differs_from_the_stored_one() {
        // The checkout file hashes to exactly what the event recorded, but
        // its header no longer equals the immutable stored header.
        let current = COUNCIL_PLAN.replace("# Council Task", "# Council Task v2");
        let fp = sha256_hex(current.as_bytes());
        let got = task_get_envelope(COUNCIL_PLAN, serde_json::json!([plan_check_event(&fp)]));
        let out = super::assess_plan_freshness(&got, |_| Ok(current.as_bytes().to_vec()));
        match out {
            super::PlanFreshness::Warn(problems) => {
                assert_eq!(problems.len(), 1);
                assert!(problems[0].contains("differs from the immutable stored header"));
            }
            other => panic!("expected a header-mismatch warning, got {other:?}"),
        }
    }

    #[test]
    fn claim_preflight_is_silent_for_non_council_tasks() {
        // No council object → no preflight noise, even with no plan_check
        // event and an unreadable checkout (the non-council path must stay
        // byte-identical to today).
        let got = task_get_envelope(MIN_HEADER_PLAN, serde_json::json!([]));
        assert_eq!(
            super::assess_plan_freshness(&got, |_| Err("unreadable".to_string())),
            super::PlanFreshness::NotCouncil
        );
        // Legacy free-text plans (no v1 header) are not council-tagged either.
        let legacy = task_get_envelope("just prose\nno header\n", serde_json::json!([]));
        assert_eq!(
            super::assess_plan_freshness(&legacy, |_| Err("unreadable".to_string())),
            super::PlanFreshness::NotCouncil
        );
    }

    #[test]
    fn direct_claim_preflight_matches_only_the_public_claim_form() {
        // The four-word public form is what main() preflights…
        let argv = v(&["task", "claim", "ws1", "t1"]);
        assert_eq!(super::direct_claim_target(&argv), Some(("ws1", "t1")));
        // …and everything else passes by untouched: the expanded five-word
        // wire form (pinned by `task_claim_injects_actor_from_env`), other
        // verbs, and wrong arity all skip it.
        assert_eq!(
            super::direct_claim_target(&v(&["task", "claim", "self1", "ws1", "t1"])),
            None
        );
        assert_eq!(
            super::direct_claim_target(&v(&["task", "get", "ws1", "t1"])),
            None
        );
        assert_eq!(
            super::direct_claim_target(&v(&["task", "claim", "ws1"])),
            None
        );
    }

    #[test]
    fn lane_preflight_hashes_the_worktree_plan_even_when_main_differs() {
        let root =
            std::env::temp_dir().join(format!("conclave-lane-preflight-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("docs/plans")).expect("mkdir repo");
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .current_dir(&root)
                .args(args)
                .output()
                .expect("run git");
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-b", "main"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(root.join("docs/plans/x.md"), COUNCIL_PLAN).expect("write plan");
        git(&["add", "."]);
        git(&["commit", "-m", "plan"]);
        // `lane start`'s exact order: the worktree exists FIRST, then the
        // preflight roots at `.claude/worktrees/<slug>` inside it.
        git(&[
            "worktree",
            "add",
            "-b",
            "lane/t1",
            ".claude/worktrees/t1",
            "main",
        ]);
        assert_eq!(
            super::lane_worktree_root("t1"),
            PathBuf::from(".claude/worktrees/t1"),
            "lane_task_wiring roots the preflight at the worktree lane start creates"
        );
        // Uncommitted plan edit on MAIN — the worktree still has the checked
        // bytes; the two checkouts now genuinely differ.
        std::fs::write(
            root.join("docs/plans/x.md"),
            format!("{COUNCIL_PLAN}\nmain-only uncommitted edit\n"),
        )
        .expect("edit main plan");

        let fp = sha256_hex(COUNCIL_PLAN.as_bytes());
        let got = task_get_envelope(COUNCIL_PLAN, serde_json::json!([plan_check_event(&fp)]));

        // Rooted at the worktree (what lane_task_wiring passes): fresh — the
        // WORKTREE fingerprint is the referent, not main's dirty copy.
        let worktree = root.join(".claude/worktrees/t1");
        assert_eq!(
            super::assess_plan_freshness(&got, |rel| read_checkout_file(&worktree, rel)),
            super::PlanFreshness::Fresh
        );
        // Rooted at main instead, the same task state would (correctly) warn —
        // proving the root, not the event, decides what gets hashed.
        match super::assess_plan_freshness(&got, |rel| read_checkout_file(&root, rel)) {
            super::PlanFreshness::Warn(problems) => {
                assert!(problems[0].contains("changed since its last successful plan-check"));
            }
            other => panic!("expected main's dirty plan to warn, got {other:?}"),
        }

        let _ = std::process::Command::new("git")
            .current_dir(&root)
            .args(["worktree", "remove", "--force", ".claude/worktrees/t1"])
            .output();
        let _ = std::fs::remove_dir_all(&root);
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

    // ── orient (task conclave-orient) ──────────────────────────────────────

    #[test]
    fn render_orient_is_compact_text_with_all_sections() {
        let packet = serde_json::json!({
            "tasks": [{
                "slug": "t1", "state": "review", "title": "T1",
                "implementerAgentId": "07fd0f59-3e13", "updatedAt": "2026-07-09T14:00:00Z",
                "openChallenges": 1
            }],
            "tasksTotal": 2,
            "roster": [{
                "id": "id-1", "name": "Tiësto", "level": "senior",
                "working": true, "model": "claude-fable-5"
            }],
            "messages": [{
                "from": "Peer", "to": "Me", "text": "hello",
                "createdAt": "2026-07-09T14:21:00Z"
            }],
            "blackboard": [{ "key": "k1", "value": "v1" }],
            "watches": ["t1"],
            "self": { "id": "id-1", "name": "Tiësto", "level": "senior", "contextPct": 61 }
        });
        let out = super::render_orient(&packet);
        assert!(out.starts_with("you: Tiësto (senior)  id-1  context=61%\n"));
        assert!(out.contains(
            "tasks (2 live, first 1):\n  t1  review  impl=07fd0f59  T1  [open challenges: 1]\n"
        ));
        assert!(out.contains("roster (1):\n  Tiësto (senior)  id-1  working  claude-fable-5\n"));
        assert!(out.contains("messages (latest 1):\n  14:21  Peer → Me  hello\n"));
        assert!(out.contains("blackboard (1):\n  k1 = v1\n"));
        assert!(out.ends_with("watches: t1\n"));
        assert!(!out.contains('{'), "text rendering, not JSON: {out}");
    }

    #[test]
    fn render_orient_empty_sections_have_markers() {
        let packet = serde_json::json!({
            "tasks": [], "tasksTotal": 0, "roster": [], "messages": [],
            "blackboard": [], "watches": [], "self": null
        });
        let out = super::render_orient(&packet);
        assert!(out.contains("tasks (0 live):\n  (none)\n"));
        assert!(out.contains("messages (latest 0):\n  (none)\n"));
        assert!(out.ends_with("watches: (none)\n"));
    }

    #[test]
    fn expand_orient_injects_self_and_requires_it() {
        let out = expand_self_args(v(&["orient", "ws1"]), Some("self1")).unwrap();
        assert_eq!(out, v(&["orient", "self1", "ws1"]));
        assert!(expand_self_args(v(&["orient", "ws1"]), None).is_err());
        assert!(expand_self_args(v(&["orient", "ws1"]), Some("")).is_err());
        // Arity is checked client-side: no workspace, or stray words, error out.
        assert!(expand_self_args(v(&["orient"]), Some("self1")).is_err());
        assert!(expand_self_args(v(&["orient", "ws1", "extra"]), Some("self1")).is_err());
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
        let out = expand_self_args(v(&["browser", "screenshot", "shot.png"]), None).unwrap();
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
        let out = expand_self_args(v(&["browser", "screenshot", "/tmp/shot.png"]), None).unwrap();
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
                {
                    "id": "challenge-12345678", "claim": "fix it", "status": "open",
                    "actorAgentId": "actor-0123456789", "evidence": "gate log red",
                    "proposal": "revert Y", "default": "escalate"
                }
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
        // Council fields render in a stable by/evidence/proposal/default order.
        let by = rendered
            .find("    by: actor-0123456789\n")
            .expect("actor line");
        let evidence = rendered
            .find("    evidence: gate log red\n")
            .expect("evidence line");
        let proposal = rendered
            .find("    proposal: revert Y\n")
            .expect("proposal line");
        let default = rendered
            .find("    default: escalate\n")
            .expect("default line");
        assert!(by < evidence && evidence < proposal && proposal < default);
    }

    #[test]
    fn render_task_brief_omits_absent_council_fields() {
        let brief = serde_json::json!({
            "task": { "slug": "t1", "title": "T1", "state": "claimed" },
            "openChallenges": [
                { "id": "challenge-12345678", "claim": "fix it", "status": "open" }
            ],
        });

        let rendered = super::render_task_brief(&brief);
        assert!(rendered.contains("open challenges (1):"), "{rendered}");
        for label in ["by:", "evidence:", "proposal:", "default:"] {
            assert!(
                !rendered.contains(label),
                "absent field must not render a '{label}' line:\n{rendered}"
            );
        }
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
        let (sha, n_files, _) = super::stage_commit_core(
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

        let (sha, _, _) = super::stage_commit_core(
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

        let (sha, _, _) = super::stage_commit_core_with_hook(
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

        let (sha, n_files, _) =
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
    fn stage_commit_skips_a_boundary_path_that_never_existed() {
        // Failure shape 1 (memory ae4615e9): the boundary declares a file a
        // LATER task will create (e.g. a future migration). `git add` used to
        // abort the whole commit over the non-matching pathspec.
        let dir = init_repo();
        std::fs::write(dir.join("in-scope.txt"), "v2\n").unwrap();
        let boundary = vec![
            "in-scope.txt".to_string(),
            "migrations/0017_not_written_yet.sql".to_string(),
        ];

        let (sha, n_files, skipped) =
            super::stage_commit_core(&dir, "main", "t1", &boundary, "real change", "Dew", "dew")
                .expect("an absent boundary path must not abort the commit");
        assert_eq!(n_files, 1);
        assert_eq!(
            skipped,
            vec!["migrations/0017_not_written_yet.sql".to_string()]
        );
        let show = super::git_output(&dir, &["show", "--stat", "--format=", &sha]).unwrap();
        assert!(show.contains("in-scope.txt"), "{show}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stage_commit_skips_a_renamed_away_boundary_path() {
        // Failure shape 2 (memory 2e6ecaeb): a rename's OLD path, deleted
        // from both the working tree and HEAD, matches nothing anywhere —
        // permanently. It must be skipped, not abort every later commit.
        let dir = init_repo();
        std::fs::write(dir.join("old-name.rs"), "content\n").unwrap();
        run_git(&dir, &["add", "old-name.rs"]);
        run_git(&dir, &["commit", "-q", "-m", "add old name"]);
        run_git(&dir, &["rm", "-q", "old-name.rs"]);
        run_git(&dir, &["commit", "-q", "-m", "rename away"]);
        std::fs::write(dir.join("in-scope.txt"), "v2\n").unwrap();
        let boundary = vec!["in-scope.txt".to_string(), "old-name.rs".to_string()];

        let (_, n_files, skipped) =
            super::stage_commit_core(&dir, "main", "t1", &boundary, "later work", "Dew", "dew")
                .expect("a renamed-away boundary path must not abort the commit");
        assert_eq!(n_files, 1);
        assert_eq!(skipped, vec!["old-name.rs".to_string()]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stage_commit_deleted_but_tracked_path_is_kept_not_skipped() {
        // The skip must never swallow a REAL deletion: absent from the
        // working tree but still tracked in HEAD means "commit the deletion"
        // (the add -A semantics test 7 pins), not "skip".
        let dir = init_repo();
        std::fs::remove_file(dir.join("in-scope.txt")).unwrap();
        let boundary = vec!["in-scope.txt".to_string()];

        let (sha, n_files, skipped) =
            super::stage_commit_core(&dir, "main", "t1", &boundary, "delete it", "Dew", "dew")
                .expect("a tracked deletion must still commit");
        assert_eq!(n_files, 1);
        assert!(
            skipped.is_empty(),
            "tracked deletion wrongly skipped: {skipped:?}"
        );
        let ls = super::git_output(&dir, &["ls-tree", "--name-only", &sha]).unwrap();
        assert!(!ls.contains("in-scope.txt"), "{ls}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stage_commit_with_every_boundary_path_absent_reports_nothing_to_commit() {
        // With ALL entries skipped nothing must be staged (a pathspec-less
        // `add -A` would sweep the whole tree) — the outcome is the ordinary
        // "nothing to commit" error, now naming the skipped paths.
        let dir = init_repo();
        std::fs::write(dir.join("in-scope.txt"), "v2\n").unwrap(); // outside boundary here
        let boundary = vec!["ghost-a.rs".to_string(), "ghost-b.rs".to_string()];

        let err = super::stage_commit_core(&dir, "main", "t1", &boundary, "msg", "Dew", "dew")
            .expect_err("nothing stageable must stay an error");
        assert!(err.contains("nothing to commit"), "{err}");
        assert!(
            err.contains("ghost-a.rs") && err.contains("ghost-b.rs"),
            "{err}"
        );
        // And nothing outside the boundary leaked into any new commit: the
        // branch tip is untouched.
        let head = super::git_output(&dir, &["log", "--oneline", "main"]).unwrap();
        assert_eq!(head.lines().count(), 1, "no commit may have landed: {head}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn snapshot_skips_absent_boundary_paths_instead_of_aborting() {
        // The auto-snapshot before a commit runs the same tree build (memory
        // 2e6ecaeb saw the abort there first) — it must skip and report too.
        let dir = init_repo();
        std::fs::write(dir.join("in-scope.txt"), "v2\n").unwrap();
        let boundary = vec!["in-scope.txt".to_string(), "ghost.rs".to_string()];

        let (sha, skipped) = super::snapshot(
            &dir,
            "t1",
            &boundary,
            "manual",
            "Dew",
            "dew@agents.conclave.local",
        )
        .expect("an absent boundary path must not abort the snapshot");
        assert!(sha.is_some(), "changed boundary content must snapshot");
        assert_eq!(skipped, vec!["ghost.rs".to_string()]);

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
        .0
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
        .0
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
    fn restore_skips_boundary_paths_absent_from_the_snapshot() {
        // Restore-side twin of stage_commit_skips_a_boundary_path_that_never_
        // existed: the boundary declares a file no task has created yet, so
        // the snapshot tree lacks it — that entry must be skipped, not abort
        // the whole `git restore`.
        let dir = init_repo();
        let boundary = vec![
            "in-scope.txt".to_string(),
            "migrations/0017_not_written_yet.sql".to_string(),
        ];
        let snap = super::snapshot(
            &dir,
            "t1",
            &boundary,
            "manual",
            "Dew",
            "dew@agents.conclave.local",
        )
        .unwrap()
        .0
        .expect("snapshot must be created");
        std::fs::write(dir.join("in-scope.txt"), "modified\n").unwrap();

        let (restored, skipped) = super::restore_boundary_from_snapshot(&dir, &snap, &boundary)
            .expect("an absent boundary path must not abort the restore");
        assert_eq!(restored, vec!["in-scope.txt".to_string()]);
        assert_eq!(
            skipped,
            vec!["migrations/0017_not_written_yet.sql".to_string()]
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("in-scope.txt")).unwrap(),
            "v1\n",
            "the present path must actually be restored"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn restore_with_every_boundary_path_absent_is_a_no_op_not_an_error() {
        // With ALL entries skipped no `git restore` may run at all: an empty
        // pathspec is a git error, and any wider pathspec would sweep paths
        // outside the boundary.
        let dir = init_repo();
        let snap = super::snapshot(
            &dir,
            "t1",
            &["in-scope.txt".to_string()],
            "manual",
            "Dew",
            "dew@agents.conclave.local",
        )
        .unwrap()
        .0
        .expect("snapshot must be created");
        let boundary = vec!["ghost-a.rs".to_string(), "ghost-b.rs".to_string()];
        std::fs::write(dir.join("in-scope.txt"), "modified\n").unwrap();

        let (restored, skipped) = super::restore_boundary_from_snapshot(&dir, &snap, &boundary)
            .expect("an all-absent restore must be a no-op, not an error");
        assert!(restored.is_empty(), "nothing must restore: {restored:?}");
        assert_eq!(skipped, boundary);
        assert_eq!(
            std::fs::read_to_string(dir.join("in-scope.txt")).unwrap(),
            "modified\n",
            "a no-op restore must leave the worktree untouched"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn restore_recreates_a_snapshot_path_deleted_from_the_worktree() {
        // The skip must never swallow a REAL restore: gone from the worktree
        // but present in the snapshot tree means "restore it" (git recreates
        // the file), never "absent, skip".
        let dir = init_repo();
        let boundary = vec!["in-scope.txt".to_string()];
        let snap = super::snapshot(
            &dir,
            "t1",
            &boundary,
            "manual",
            "Dew",
            "dew@agents.conclave.local",
        )
        .unwrap()
        .0
        .expect("snapshot must be created");
        std::fs::remove_file(dir.join("in-scope.txt")).unwrap();

        let (restored, skipped) = super::restore_boundary_from_snapshot(&dir, &snap, &boundary)
            .expect("a deleted-but-snapshotted path must restore, not skip");
        assert_eq!(restored, boundary);
        assert!(
            skipped.is_empty(),
            "snapshotted path wrongly skipped: {skipped:?}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("in-scope.txt")).unwrap(),
            "v1\n",
            "the deleted file must be recreated from the snapshot"
        );

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
        .0
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
        .0
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
        .0
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
        assert!(
            second.0.is_none(),
            "identical tree must skip the ref update"
        );

        let tip =
            super::git_output(&dir, &["rev-parse", &super::stage_snapshot_ref("t1")]).unwrap();
        assert_eq!(tip, first, "ref must still point at the first snapshot");

        std::fs::remove_dir_all(&dir).ok();
    }

    // ── rtk-hook PreToolUse protocol (task rtk-hook-verb) ───────────────────

    /// A realistic PreToolUse payload: `tool_input` carries the command plus
    /// sibling keys that must survive into `updatedInput` untouched.
    fn rtk_hook_input() -> serde_json::Value {
        serde_json::json!({
            "tool_name": "Bash",
            "tool_input": {"command": "git status", "description": "Show status"}
        })
    }

    #[test]
    fn extract_bash_command_reads_tool_input_command() {
        assert_eq!(
            super::extract_bash_command(&rtk_hook_input()),
            Some("git status".to_string())
        );
    }

    #[test]
    fn extract_bash_command_missing_or_empty_is_none() {
        assert_eq!(super::extract_bash_command(&serde_json::json!({})), None);
        assert_eq!(
            super::extract_bash_command(&serde_json::json!({"tool_input": {}})),
            None
        );
        assert_eq!(
            super::extract_bash_command(&serde_json::json!({"tool_input": {"command": ""}})),
            None
        );
        assert_eq!(
            super::extract_bash_command(&serde_json::json!({"tool_input": {"command": 3}})),
            None
        );
    }

    #[test]
    fn rtk_hook_response_exit0_rewritten_allows_with_updated_input() {
        // Trailing newline from the rtk process is trimmed before emit.
        let out = super::rtk_hook_response(&rtk_hook_input(), 0, "rtk git status\n")
            .expect("rewritten command must produce a hook response");
        let hso = &out["hookSpecificOutput"];
        assert_eq!(hso["hookEventName"], "PreToolUse");
        assert_eq!(hso["permissionDecision"], "allow");
        assert_eq!(hso["permissionDecisionReason"], "RTK auto-rewrite");
        assert_eq!(hso["updatedInput"]["command"], "rtk git status");
        assert_eq!(
            hso["updatedInput"]["description"], "Show status",
            "sibling tool_input keys must be preserved"
        );
    }

    #[test]
    fn rtk_hook_response_exit0_identical_is_none() {
        assert!(super::rtk_hook_response(&rtk_hook_input(), 0, "git status").is_none());
        // Identical after trailing-newline trim counts as identical.
        assert!(super::rtk_hook_response(&rtk_hook_input(), 0, "git status\n").is_none());
    }

    #[test]
    fn rtk_hook_response_exit0_empty_stdout_is_none() {
        // An empty rewrite would clobber the command — fail-open instead.
        assert!(super::rtk_hook_response(&rtk_hook_input(), 0, "").is_none());
        assert!(super::rtk_hook_response(&rtk_hook_input(), 0, "\n").is_none());
    }

    #[test]
    fn rtk_hook_response_exit1_and_exit2_pass_through() {
        assert!(super::rtk_hook_response(&rtk_hook_input(), 1, "anything").is_none());
        assert!(super::rtk_hook_response(&rtk_hook_input(), 2, "anything").is_none());
    }

    #[test]
    fn rtk_hook_response_exit3_asks_without_permission_decision() {
        let out = super::rtk_hook_response(&rtk_hook_input(), 3, "rtk git status\n")
            .expect("exit 3 with a rewrite must produce a hook response");
        let hso = &out["hookSpecificOutput"];
        assert_eq!(hso["hookEventName"], "PreToolUse");
        assert_eq!(hso["updatedInput"]["command"], "rtk git status");
        assert_eq!(
            hso["updatedInput"]["description"], "Show status",
            "sibling tool_input keys must be preserved"
        );
        assert!(hso.get("permissionDecision").is_none());
        assert!(hso.get("permissionDecisionReason").is_none());
    }

    #[test]
    fn rtk_hook_response_exit3_empty_stdout_is_none() {
        assert!(super::rtk_hook_response(&rtk_hook_input(), 3, "").is_none());
    }

    #[test]
    fn rtk_hook_response_unknown_exit_is_none() {
        assert!(super::rtk_hook_response(&rtk_hook_input(), 127, "rtk git status").is_none());
        assert!(super::rtk_hook_response(&rtk_hook_input(), -1, "rtk git status").is_none());
    }

    #[test]
    fn rtk_hook_response_input_without_command_is_none() {
        assert!(super::rtk_hook_response(&serde_json::json!({}), 0, "rtk x").is_none());
    }

    /// Denylist (task rtk-hook-denylist): rewrites that would run the
    /// observation-critical commands through rtk are dropped entirely — the
    /// raw command must pass through unfiltered, on BOTH exit-code paths.
    #[test]
    fn rtk_hook_response_denylists_grep_rewrite_on_both_exit_paths() {
        let rw = "rtk grep -n pat src/main.rs\n";
        assert!(super::rtk_hook_response(&rtk_hook_input(), 0, rw).is_none());
        assert!(super::rtk_hook_response(&rtk_hook_input(), 3, rw).is_none());
    }

    #[test]
    fn rtk_hook_response_denylists_every_observation_verb() {
        for rw in [
            "rtk ls\n",
            "rtk ls -la src\n",
            "rtk tree src\n",
            "rtk find . -name x\n",
            "rtk read foo.txt\n",
        ] {
            assert!(
                super::rtk_hook_response(&rtk_hook_input(), 0, rw).is_none(),
                "must denylist rewrite: {rw:?}"
            );
        }
    }

    #[test]
    fn rtk_hook_response_denylists_compound_and_pipeline_positions() {
        for rw in [
            "cd /x && rtk ls\n",
            "true || rtk grep -n pat f\n",
            "echo a; rtk tree src\n",
            "sort names.txt | rtk read out.txt\n",
            "echo $(rtk ls)\n",
        ] {
            assert!(
                super::rtk_hook_response(&rtk_hook_input(), 0, rw).is_none(),
                "must denylist rewrite at compound position: {rw:?}"
            );
        }
    }

    /// The denylist matches whole verbs at command positions only — `rtk`
    /// appearing as an argument, or a verb-prefixed word, must still rewrite.
    #[test]
    fn rtk_hook_response_denylist_ignores_non_command_positions_and_prefixes() {
        for rw in ["echo rtk ls\n", "rtk grepx foo\n", "rtk lsof\n"] {
            assert!(
                super::rtk_hook_response(&rtk_hook_input(), 0, rw).is_some(),
                "must NOT denylist rewrite: {rw:?}"
            );
        }
    }
}
