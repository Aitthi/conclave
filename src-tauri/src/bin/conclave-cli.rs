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

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

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
  snapshot list <sessionId>
  snapshot read <snapshotId>
  snapshot create <sessionId> <type> [label]
  run <orchestratorId> <prompt...>
  help

Requires the Conclave app to be running.
";

/// Expand the agent-friendly `tell <toInstanceId> <text...>` form into the wire
/// form `tell <fromInstanceId> <toInstanceId> <text...>` the server allowlist
/// expects, filling `<fromInstanceId>` from the spawned agent's
/// `CONCLAVE_INSTANCE_ID`. The server tags the delivered message `[from <name>]`,
/// so no client-side tagging is needed. Non-`tell` argv passes through untouched.
///
/// Errors (as a user-facing string) when `tell` is used without a known instance
/// id (i.e. run outside a spawned agent) or with no target/text.
fn expand_tell_args(argv: Vec<String>, self_instance: Option<&str>) -> Result<Vec<String>, String> {
    if argv.first().map(String::as_str) != Some("tell") {
        return Ok(argv);
    }
    let from = self_instance.filter(|s| !s.is_empty()).ok_or_else(|| {
        "conclave: `tell` is only available inside a spawned agent (CONCLAVE_INSTANCE_ID unset)"
            .to_string()
    })?;
    if argv.len() < 3 {
        return Err("conclave: tell <agentId> <text...>".to_string());
    }
    let mut out = Vec::with_capacity(argv.len() + 1);
    out.push("tell".to_string()); // subcommand
    out.push(from.to_string()); // fromInstanceId (injected from env)
    out.extend_from_slice(&argv[1..]); // toInstanceId + text...
    Ok(out)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();

    // Empty invocation or explicit help request — print usage and exit cleanly.
    if argv.is_empty() || argv[0] == "help" || argv[0] == "--help" || argv[0] == "-h" {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    // Expand `tell <agentId> <text>` to its wire form, filling the sender from
    // CONCLAVE_INSTANCE_ID (set on spawned agents).
    let argv = match expand_tell_args(argv, std::env::var("CONCLAVE_INSTANCE_ID").ok().as_deref()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };

    let path = match socket_path() {
        Some(p) => p,
        None => {
            eprintln!("conclave: could not resolve user data directory");
            return ExitCode::from(2);
        }
    };

    // Connect to the running Conclave app via its Unix-domain socket.
    let stream = match UnixStream::connect(&path).await {
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
        let pretty = serde_json::to_string_pretty(result).expect("serialize result cannot fail");
        println!("{pretty}");
        return ExitCode::SUCCESS;
    }

    // Neither `result` nor `error` — malformed response from the server.
    eprintln!("conclave: malformed response (no result or error field)");
    ExitCode::from(2)
}

#[cfg(test)]
mod tests {
    use super::expand_tell_args;

    fn v(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn tell_injects_sender_from_env() {
        let out = expand_tell_args(v(&["tell", "to1", "hello", "there"]), Some("self1")).unwrap();
        assert_eq!(out, v(&["tell", "self1", "to1", "hello", "there"]));
    }

    #[test]
    fn tell_without_instance_id_errors() {
        assert!(expand_tell_args(v(&["tell", "to1", "hi"]), None).is_err());
        assert!(expand_tell_args(v(&["tell", "to1", "hi"]), Some("")).is_err());
    }

    #[test]
    fn tell_without_text_errors() {
        assert!(expand_tell_args(v(&["tell", "to1"]), Some("self1")).is_err());
    }

    #[test]
    fn non_tell_passes_through_untouched() {
        let args = v(&["send", "sess1", "hello"]);
        assert_eq!(expand_tell_args(args.clone(), Some("self1")).unwrap(), args);
        let list = v(&["agent", "list", "ws1"]);
        assert_eq!(expand_tell_args(list.clone(), None).unwrap(), list);
    }
}
