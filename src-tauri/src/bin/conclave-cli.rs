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
use std::process::ExitCode;
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
    use super::expand_self_args;

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
}
