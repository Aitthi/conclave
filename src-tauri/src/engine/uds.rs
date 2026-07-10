//! Unix-Domain-Socket server: a second front-door onto the command router.
//!
//! The Tauri `invoke("ipc", …)` bridge and this UDS server both funnel every
//! request through [`crate::engine::router::dispatch`]. The socket speaks
//! newline-delimited JSON-RPC 2.0 so the (future M5.2) `conclave` CLI and
//! agents can drive the SAME command surface the GUI uses.
//!
//! The whole module is unix-only — the app targets macOS. Non-unix builds get
//! an empty module and simply have no socket server.
#![cfg(unix)]

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::engine::{router, AppError, AppState};

/// Maximum bytes accepted for a single request line. A client that never sends
/// a newline would otherwise grow the read buffer unbounded (memory DoS) — even
/// a same-user buggy client could OOM the shared GUI process. 4 MiB dwarfs any
/// real JSON-RPC payload.
const MAX_LINE_BYTES: u64 = 4 * 1024 * 1024;

/// Cap on concurrent UDS connections. A bound on per-connection memory (each
/// holds a read buffer); excess connections are dropped rather than unbounded-
/// spawned. Generous for a single-user local socket.
const MAX_CONNECTIONS: usize = 32;

/// Resolve the socket path: `~/Library/Application Support/Conclave/conclave.sock`
/// on macOS (the same `Conclave` data dir as the database). Creates the
/// directory if it does not exist.
///
/// # Panics
/// Panics if the user data directory cannot be resolved or the `Conclave`
/// directory cannot be created — both unrecoverable on a supported install,
/// and mirrored from `db::db_path()`.
pub fn socket_path() -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = dirs::data_dir()
        .expect("could not resolve user data directory")
        .join("Conclave");
    std::fs::create_dir_all(&dir).expect("could not create Conclave data directory");
    // Lock the data dir to owner-only (0700). `create_dir_all` applies the
    // process umask, which can leave the dir world-traversable (0755); a
    // 0700 dir closes the brief window between the socket's bind and its 0600
    // chmod, during which the socket file itself is umask-default (e.g. 0644).
    // With the parent dir owner-only, no other local user can even reach it.
    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    dir.join("conclave.sock")
}

/// Map an [`AppError`] to its JSON-RPC error code.
///
/// `NotFound`/`NotImplemented` → method-not-found (`-32601`),
/// `Invalid` → invalid-params (`-32602`),
/// `Internal` (and anything else) → internal-error (`-32603`).
fn error_code(err: &AppError) -> i32 {
    match err {
        AppError::NotFound(_) | AppError::NotImplemented(_) => -32601,
        AppError::Invalid(_) => -32602,
        AppError::Internal(_) => -32603,
    }
}

/// Build a single-line JSON-RPC success response.
fn ok_response(id: &Value, result: Value) -> String {
    // Serializing a plain object literal cannot fail.
    serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    }))
    .expect("serialize ok response")
}

/// Build a single-line JSON-RPC error response.
fn err_response(id: &Value, code: i32, message: impl Into<String>) -> String {
    serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message.into() },
    }))
    .expect("serialize err response")
}

/// Handle one newline-delimited JSON-RPC request line and return the response
/// line (a single line, no embedded newline). This is the testable core: it has
/// no socket dependency.
pub(crate) async fn handle_line(state: &AppState, line: &str) -> String {
    // Parse the request envelope.
    let req: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return err_response(&Value::Null, -32700, format!("Parse error: {e}")),
    };

    // Preserve the id verbatim (number / string / null); default null if absent.
    let id = req.get("id").cloned().unwrap_or(Value::Null);

    // `method` must be present and a string.
    let method = match req.get("method").and_then(Value::as_str) {
        Some(m) => m,
        None => return err_response(&id, -32600, "Invalid Request: missing method"),
    };

    // `params` defaults to null when absent.
    let params = req.get("params").cloned().unwrap_or(Value::Null);

    match router::dispatch(state, method, params).await {
        Ok(value) => ok_response(&id, value),
        Err(e) => err_response(&id, error_code(&e), e.to_string()),
    }
}

/// Serve a single accepted connection: read request lines, dispatch each, and
/// write back response lines. Returns when the client disconnects (EOF) or a
/// write fails (client gone).
async fn handle_conn(state: Arc<AppState>, stream: UnixStream) {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    let mut line = String::new();

    loop {
        line.clear();
        // Bound each line read: a fresh `take(MAX_LINE_BYTES)` per iteration caps
        // THIS line without limiting the stream, so a newline-less flood can't
        // grow `line` unbounded.
        let n = match (&mut reader)
            .take(MAX_LINE_BYTES)
            .read_line(&mut line)
            .await
        {
            // EOF — client closed the connection.
            Ok(0) => break,
            Ok(n) => n,
            // Read error (incl. invalid UTF-8) — treat as disconnect.
            Err(_) => break,
        };
        // Hit the cap with no terminating newline → the line is over-long. Reject
        // and close rather than try to resync a half-read frame.
        if n as u64 == MAX_LINE_BYTES && !line.ends_with('\n') {
            let resp = err_response(&Value::Null, -32700, "Request line exceeds 4 MiB limit");
            let _ = write.write_all(resp.as_bytes()).await;
            let _ = write.write_all(b"\n").await;
            break;
        }

        // Skip blank / whitespace-only lines.
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let resp = handle_line(&state, trimmed).await;
        if write.write_all(resp.as_bytes()).await.is_err() {
            break;
        }
        if write.write_all(b"\n").await.is_err() {
            break;
        }
    }
}

/// Claim the socket at `path` for THIS process WITHOUT stealing it from a live
/// instance. Returns the bound listener, or `None` if the path is already
/// served (this process then runs without its own UDS server) or the bind fails.
///
/// The old code did an unconditional `remove_file` + `bind`: a second process
/// (e.g. a debug build launched next to the installed app) would unlink the
/// live socket's directory entry and bind its own, so every agent CLI then
/// connected to the impostor while the real GUI kept an orphaned listener — the
/// 2026-07-03 socket-steal incident. We now PROBE first with a connect:
///
/// - connect **accepted** → a live instance owns the path → abort, never steal;
/// - connect **refused** (`ECONNREFUSED`, a prior instance died without
///   unlinking) → stale entry → remove it and bind;
/// - **no file** (`ENOENT`) → nothing to remove, just bind;
/// - any other probe error → ambiguous, so do NOT remove (removing could steal
///   a live socket we merely failed to probe) — attempt the bind and let
///   `AddrInUse` fall through to the failure path.
async fn acquire_socket(path: &std::path::Path) -> Option<UnixListener> {
    match UnixStream::connect(path).await {
        Ok(_) => {
            eprintln!(
                "[uds] another Conclave instance is already serving {path:?}; \
                 this process will run without its own UDS server"
            );
            return None;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
            if let Err(e) = std::fs::remove_file(path) {
                eprintln!("[uds] could not remove stale socket {path:?}: {e}");
            }
        }
        Err(e) => {
            eprintln!(
                "[uds] could not probe existing socket {path:?}: {e}; \
                 binding without removing (won't steal a live socket)"
            );
        }
    }

    match UnixListener::bind(path) {
        Ok(listener) => Some(listener),
        Err(e) => {
            eprintln!("[uds] failed to bind socket {path:?}: {e}");
            None
        }
    }
}

/// Bind the UDS at `path` (0600), then accept connections forever, spawning a
/// task per connection. Never panics: if the socket cannot be claimed (already
/// served by a live instance, or bind failure), the GUI must still run, so we
/// log and return.
pub async fn serve(state: Arc<AppState>, path: PathBuf) {
    use std::os::unix::fs::PermissionsExt;

    let Some(listener) = acquire_socket(&path).await else {
        return;
    };

    // Lock the socket down to owner-only (0600). Log + continue on error.
    if let Err(e) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)) {
        eprintln!("[uds] could not set 0600 permissions on {path:?}: {e}");
    }

    // Bound concurrent connections so a flood of opens can't spawn unbounded
    // tasks (each with its own read buffer).
    let limiter = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                // Drop the connection if we're at the cap rather than queueing.
                let Ok(permit) = Arc::clone(&limiter).try_acquire_owned() else {
                    eprintln!("[uds] connection limit ({MAX_CONNECTIONS}) reached — dropping");
                    continue;
                };
                let conn_state = Arc::clone(&state);
                tokio::spawn(async move {
                    handle_conn(conn_state, stream).await;
                    drop(permit); // release the slot when the connection ends
                });
            }
            Err(e) => {
                eprintln!("[uds] accept error: {e}");
                // Yield so a *persistent* accept error (e.g. EMFILE) interleaves
                // fairly with other tasks instead of pinning a worker at 100%.
                tokio::task::yield_now().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[tokio::test]
    async fn handle_line_valid_request_returns_result() {
        let state = AppState::for_tests().await;
        let resp = handle_line(
            &state,
            r#"{"jsonrpc":"2.0","id":1,"method":"workspace.list"}"#,
        )
        .await;
        let v: Value = serde_json::from_str(&resp).expect("response is JSON");

        assert_eq!(v.get("id"), Some(&json!(1)));
        assert!(v.get("error").is_none(), "no error expected: {resp}");
        let result = v.get("result").expect("result present");
        assert!(result.is_array(), "workspace.list returns an array: {resp}");
        assert_eq!(result.as_array().unwrap().len(), 0, "empty in-memory DB");
    }

    #[tokio::test]
    async fn handle_line_unknown_method_is_method_not_found() {
        let state = AppState::for_tests().await;
        let resp = handle_line(&state, r#"{"jsonrpc":"2.0","id":1,"method":"nope.nope"}"#).await;
        let v: Value = serde_json::from_str(&resp).expect("response is JSON");

        assert_eq!(v.get("id"), Some(&json!(1)), "id preserved");
        assert_eq!(
            v.pointer("/error/code").and_then(Value::as_i64),
            Some(-32601)
        );
    }

    #[tokio::test]
    async fn handle_line_malformed_json_is_parse_error() {
        let state = AppState::for_tests().await;
        let resp = handle_line(&state, "{not json").await;
        let v: Value = serde_json::from_str(&resp).expect("response is JSON");

        assert_eq!(v.get("id"), Some(&Value::Null), "id is null on parse error");
        assert_eq!(
            v.pointer("/error/code").and_then(Value::as_i64),
            Some(-32700)
        );
    }

    #[tokio::test]
    async fn handle_line_missing_method_is_invalid_request() {
        let state = AppState::for_tests().await;
        let resp = handle_line(&state, r#"{"jsonrpc":"2.0","id":"abc"}"#).await;
        let v: Value = serde_json::from_str(&resp).expect("response is JSON");

        // String id round-trips unchanged.
        assert_eq!(v.get("id"), Some(&json!("abc")));
        assert_eq!(
            v.pointer("/error/code").and_then(Value::as_i64),
            Some(-32600)
        );
    }

    #[tokio::test]
    async fn handle_line_bad_params_is_invalid_params() {
        let state = AppState::for_tests().await;
        // instance.list requires `workspaceId`; an empty params object fails
        // deserialization → AppError::Invalid → -32602.
        let resp = handle_line(
            &state,
            r#"{"jsonrpc":"2.0","id":2,"method":"instance.list","params":{}}"#,
        )
        .await;
        let v: Value = serde_json::from_str(&resp).expect("response is JSON");

        assert_eq!(v.get("id"), Some(&json!(2)));
        assert_eq!(
            v.pointer("/error/code").and_then(Value::as_i64),
            Some(-32602)
        );
    }

    #[tokio::test]
    async fn code_verbs_round_trip_through_cli_exec() {
        let state = AppState::for_tests().await;
        let dir = tempfile::tempdir().expect("temp code root");
        std::fs::write(
            dir.path().join("lib.rs"),
            "fn greet() {}\nfn caller() { greet(); }\n",
        )
        .expect("write fixture");
        let root = dir.path().to_string_lossy();

        async fn call(state: &AppState, id: i64, argv: Value) -> Value {
            let request = json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "cli.exec",
                "params": { "argv": argv },
            });
            let line = serde_json::to_string(&request).expect("serialize request");
            let response = handle_line(state, &line).await;
            let value: Value = serde_json::from_str(&response).expect("response is JSON");
            assert!(value.get("error").is_none(), "unexpected error: {value}");
            value["result"].clone()
        }

        let found = call(&state, 1, json!(["code", "find", "greet", "--path", root])).await;
        assert_eq!(found["schema_version"], json!(1));
        assert_eq!(found["data"][0]["name"], json!("greet"));

        let refs = call(&state, 2, json!(["code", "refs", "greet", "--path", root])).await;
        assert!(
            refs["data"]
                .as_array()
                .expect("refs data array")
                .iter()
                .any(|hit| hit["kind"] == "call"),
            "refs should include the call site: {refs}"
        );
    }

    /// One real socket round-trip: bind, connect, send a request, read the
    /// response, and assert the socket file is 0600.
    #[tokio::test]
    async fn socket_round_trip_and_perms() {
        use std::os::unix::fs::PermissionsExt;

        let state = Arc::new(AppState::for_tests().await);
        let path = std::env::temp_dir().join("conclave-uds-test-round-trip.sock");
        let _ = std::fs::remove_file(&path);

        let server = tokio::spawn(serve(Arc::clone(&state), path.clone()));

        // Bind races the spawned task — poll-connect (busy loop with yield_now,
        // avoiding the tokio "time" feature).
        let mut stream = None;
        for _ in 0..5000 {
            match UnixStream::connect(&path).await {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                Err(_) => tokio::task::yield_now().await,
            }
        }
        let stream = stream.expect("could not connect to UDS within retry budget");

        // Socket file must be owner-only.
        let mode = std::fs::metadata(&path)
            .expect("socket metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "socket must be 0600, got {:o}", mode);

        let (read, mut write) = stream.into_split();
        write
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"workspace.list\"}\n")
            .await
            .expect("write request");

        let mut reader = BufReader::new(read);
        let mut line = String::new();
        reader.read_line(&mut line).await.expect("read response");

        let v: Value = serde_json::from_str(line.trim()).expect("response is JSON");
        assert_eq!(v.get("id"), Some(&json!(7)));
        assert!(v.get("result").is_some(), "result present: {line}");

        server.abort();
        let _ = std::fs::remove_file(&path);
    }

    /// ADR 0008 acceptance: `task` CLI verbs round-trip against a LIVE engine
    /// — a real `serve()` listener on a real Unix socket, driven with the
    /// exact `cli.exec` JSON-RPC lines `conclave-cli` sends (not direct Rust
    /// calls into `commands::task`). Runs on its own temp socket path so it
    /// never touches the shared production socket other agents are using.
    #[tokio::test]
    async fn task_verbs_round_trip_over_a_real_socket() {
        let state = Arc::new(AppState::for_tests().await);
        let ws = crate::engine::repo::workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed")
            .id;
        let agent_def = crate::engine::repo::agent_definition::create(
            &state.db,
            &crate::engine::repo::agent_definition::AgentDefinitionInput {
                name: "Agent".into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed");
        let actor =
            crate::engine::repo::workspace_agent::instantiate(&state.db, &ws, &agent_def.id)
                .await
                .expect("instantiate failed")
                .id;
        // A second workspace agent for `--watchers` coverage. `instantiate`
        // is idempotent per (workspace, definition), so it needs its own def.
        let member_def = crate::engine::repo::agent_definition::create(
            &state.db,
            &crate::engine::repo::agent_definition::AgentDefinitionInput {
                name: "Member".into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create member def failed");
        let member =
            crate::engine::repo::workspace_agent::instantiate(&state.db, &ws, &member_def.id)
                .await
                .expect("instantiate member failed")
                .id;

        let path = std::env::temp_dir().join("conclave-uds-test-task-verbs.sock");
        let _ = std::fs::remove_file(&path);
        let server = tokio::spawn(serve(Arc::clone(&state), path.clone()));

        let mut stream = None;
        for _ in 0..5000 {
            match UnixStream::connect(&path).await {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                Err(_) => tokio::task::yield_now().await,
            }
        }
        let stream = stream.expect("could not connect to UDS within retry budget");
        let (read, mut write) = stream.into_split();
        let mut reader = BufReader::new(read);

        // One request-per-line helper, over the SAME live connection every
        // `conclave-cli` invocation would open fresh — reused here to keep the
        // test linear.
        async fn call(
            write: &mut tokio::net::unix::OwnedWriteHalf,
            reader: &mut BufReader<tokio::net::unix::OwnedReadHalf>,
            id: i64,
            argv: Value,
        ) -> Value {
            let request = json!({ "jsonrpc": "2.0", "id": id, "method": "cli.exec", "params": { "argv": argv } });
            let mut line = serde_json::to_string(&request).expect("serialize request");
            line.push('\n');
            write
                .write_all(line.as_bytes())
                .await
                .expect("write request");

            let mut resp_line = String::new();
            reader
                .read_line(&mut resp_line)
                .await
                .expect("read response");
            let v: Value = serde_json::from_str(resp_line.trim()).expect("response is JSON");
            assert!(v.get("error").is_none(), "unexpected error: {v}");
            v["result"].clone()
        }

        // `task create <ws> <slug> <title...>`
        let created = call(
            &mut write,
            &mut reader,
            1,
            json!([
                "task",
                "create",
                ws,
                "acceptance-task",
                "Acceptance",
                "Task"
            ]),
        )
        .await;
        assert_eq!(created["slug"], json!("acceptance-task"));
        assert_eq!(created["state"], json!("planned"));

        // `task claim <actorId> <ws> <slug>` (already self-expanded, mirroring
        // what `conclave-cli` would send). This call is the engine ACCEPTING
        // the five-word expanded wire form; the form itself is pinned
        // CLI-side, where it is produced (`task_claim_injects_actor_from_env`
        // pins the expansion, `direct_claim_preflight_matches_only_the_
        // public_claim_form` pins that the freshness preflight never grows
        // it) — ruling be2e68d7 F4 removed a tautological length assert here
        // that advertised that coverage without exercising it.
        let claim_argv = json!(["task", "claim", actor, ws, "acceptance-task"]);
        let claimed = call(&mut write, &mut reader, 2, claim_argv).await;
        assert_eq!(claimed["state"], json!("claimed"));
        assert_eq!(claimed["implementerAgentId"], json!(actor));

        // `task gate <actorId> <ws> <slug> <cmd> <exit> <sha> <cwd> <tail>`
        let gated = call(
            &mut write,
            &mut reader,
            3,
            json!([
                "task",
                "gate",
                actor,
                ws,
                "acceptance-task",
                "cargo test --lib",
                "0",
                "abc123",
                "/repo",
                "ok"
            ]),
        )
        .await;
        assert_eq!(gated["kind"], json!("gate"));
        assert_eq!(gated["payload"]["exit"], json!(0));

        // `task get <ws> <slug>` — sees the claim's state event AND the gate
        // event, newest-first.
        let got = call(
            &mut write,
            &mut reader,
            4,
            json!(["task", "get", ws, "acceptance-task"]),
        )
        .await;
        assert_eq!(got["task"]["state"], json!("claimed"));
        let events = got["events"].as_array().expect("events array");
        assert_eq!(events.len(), 2, "state event from claim + gate event");
        assert_eq!(events[0]["kind"], json!("gate"), "newest first");
        assert_eq!(events[1]["kind"], json!("state"));

        // `task list <ws>` — ruling 3f1ab20e (task cli-output-economy): the
        // bare CLI wire form is SLIM — board-orientation fields only, no
        // derived `lastGates`/`challenges` rows.
        let listed = call(&mut write, &mut reader, 5, json!(["task", "list", ws])).await;
        let row = listed
            .as_array()
            .and_then(|a| a.iter().find(|t| t["slug"] == "acceptance-task"))
            .expect("acceptance-task present in list");
        assert_eq!(row["state"], json!("claimed"));
        assert_eq!(row["openChallenges"], json!(0));
        assert!(
            row.get("lastGates").is_none(),
            "slim rows must not carry lastGates"
        );

        // `task list <ws> --full` — RULED 2026-07-04 #2 (amended, Arta
        // fidelity F1): full rows carry the board's derived
        // `lastGates`/`challenges` fields over the SAME real wire path
        // (Arta's canon renders these on every card; no N+1 `task.get`).
        let full = call(
            &mut write,
            &mut reader,
            6,
            json!(["task", "list", ws, "--full"]),
        )
        .await;
        let row = full
            .as_array()
            .and_then(|a| a.iter().find(|t| t["slug"] == "acceptance-task"))
            .expect("acceptance-task present in full list");
        let last_gates = row["lastGates"].as_array().expect("lastGates array");
        assert_eq!(last_gates.len(), 1);
        assert_eq!(last_gates[0]["cmd"], json!("cargo test --lib"));
        assert_eq!(last_gates[0]["exit"], json!(0));
        assert_eq!(
            row["challenges"],
            json!([]),
            "no challenges raised on this task"
        );

        // ── lead-council v1 additions ─────────────────────────────────────

        // A valid `conclave-plan:v1` plan owned by `actor` (docs-only
        // boundary; the ten-line contract shape `plan_contract` pins).
        let plan_path = "docs/superpowers/plans/council.md";
        let spec_path = "docs/superpowers/specs/council.md";
        let base_sha = "e8ce7bad254f6abbd2ac782a9d62b717701b759c";
        let mut body = String::new();
        for section in crate::engine::plan_contract::REQUIRED_SECTIONS {
            body.push_str(&format!("\n## {section}\n\nBounded prose.\n"));
        }
        let plan = format!(
            "# Council Task\n\
             <!-- conclave-plan:v1\n\
             {{\n\
             \"owner\":\"{actor}\",\"authority\":\"in-loop\",\n\
             \"planPath\":\"{plan_path}\",\"baseSha\":\"{base_sha}\",\"escalation\":\"{actor}\",\n\
             \"readingOrder\":[\"{spec_path}#Field Rules\",\"{plan_path}\"],\n\
             \"boundary\":[\"{plan_path}\"],\n\
             \"consumes\":[\"{spec_path}#Field Rules\"],\n\
             \"produces\":[\"{plan_path}#Ordered edits\"],\"gates\":[\"git diff --check\"]\n\
             }} -->\n{body}"
        );

        // `task create … --owner <actor> --boundary <planPath> --plan <text>
        //  --watchers <member>` — the exact create-with-watchers wire form the
        // CLI sends after `--plan-file` resolution and owner self-defaulting.
        let created = call(
            &mut write,
            &mut reader,
            7,
            json!([
                "task",
                "create",
                ws,
                "council-task",
                "Council",
                "Task",
                "--owner",
                actor,
                "--boundary",
                plan_path,
                "--plan",
                plan,
                "--watchers",
                member
            ]),
        )
        .await;
        assert_eq!(created["slug"], json!("council-task"));
        assert_eq!(
            created["watcherAgentIds"],
            json!([actor, member]),
            "owner first, then the supplied watcher, atomically subscribed"
        );

        // `task plan-check <actorId> <ws> <slug> <planPath> <fingerprint>
        //  <ancestor 0|1> <planContent> <filesJson>` — the exact 10-word wire
        // form `run_task_plan_check` builds after its checkout reads.
        use sha2::{Digest, Sha256};
        let fingerprint = format!("{:x}", Sha256::digest(plan.as_bytes()));
        let files = json!({ spec_path: "### Field Rules\n\n- owner equals task.ownerAgentId.\n" })
            .to_string();
        let checked = call(
            &mut write,
            &mut reader,
            8,
            json!([
                "task",
                "plan-check",
                actor,
                ws,
                "council-task",
                plan_path,
                fingerprint,
                "1",
                plan,
                files
            ]),
        )
        .await;
        assert_eq!(
            checked,
            json!({
                "slug": "council-task",
                "planPath": plan_path,
                "fingerprint": fingerprint,
                "boundaryCount": 1,
                "anchorCount": 2,
                "gateCount": 1,
            }),
            "bounded packet only — never the plan text"
        );

        // The typed event is on the ledger, newest-first.
        let got = call(
            &mut write,
            &mut reader,
            9,
            json!(["task", "get", ws, "council-task"]),
        )
        .await;
        let events = got["events"].as_array().expect("events array");
        assert_eq!(events[0]["kind"], json!("plan_check"));
        assert_eq!(events[0]["payload"]["planFingerprint"], json!(fingerprint));
        assert_eq!(
            events[0]["payload"]["contractVersion"],
            json!("conclave-plan:v1")
        );

        server.abort();
        let _ = std::fs::remove_file(&path);
    }

    /// Regression for the 2026-07-03 socket-steal incident: a second process
    /// must NOT hijack a socket a live instance is already serving. The old
    /// unconditional `remove_file` + `bind` unlinked the live entry and bound
    /// an impostor; `acquire_socket` now probes first and refuses.
    #[tokio::test]
    async fn acquire_socket_refuses_to_steal_a_live_owner() {
        let path =
            std::env::temp_dir().join(format!("conclave-uds-steal-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);

        // "Instance A" — a live listener accepting on the path.
        let a = UnixListener::bind(&path).expect("bind A");
        let a_task = tokio::spawn(async move { while a.accept().await.is_ok() {} });

        // "Instance B" tries to acquire the SAME path. It must decline.
        let claimed = acquire_socket(&path).await;
        assert!(
            claimed.is_none(),
            "acquire_socket must not steal a live socket"
        );

        // A's socket is intact — a fresh client still reaches the live owner.
        assert!(
            UnixStream::connect(&path).await.is_ok(),
            "live owner must remain reachable after the refused steal"
        );

        a_task.abort();
        let _ = std::fs::remove_file(&path);
    }

    /// The guard must still recover a genuinely stale socket: a file left by a
    /// crashed instance (bound then dropped without unlinking → `ECONNREFUSED`)
    /// is removed and rebound, so a clean restart isn't blocked by debris.
    #[tokio::test]
    async fn acquire_socket_replaces_a_stale_socket() {
        let path =
            std::env::temp_dir().join(format!("conclave-uds-stale-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);

        // Leave a stale socket FILE with no listener: bind then drop. Rust does
        // not unlink a UnixListener's path on drop, so the file remains and a
        // connect to it is refused.
        drop(UnixListener::bind(&path).expect("bind stale"));
        assert!(path.exists(), "stale socket file should remain after drop");
        assert!(
            UnixStream::connect(&path).await.is_err(),
            "stale socket must refuse connections"
        );

        let claimed = acquire_socket(&path).await;
        assert!(
            claimed.is_some(),
            "acquire_socket must replace a stale (refused) socket"
        );
        // The fresh listener is live — a client can connect to us now.
        assert!(UnixStream::connect(&path).await.is_ok());

        drop(claimed);
        let _ = std::fs::remove_file(&path);
    }

    /// No socket file at all (`ENOENT`) → bind cleanly, nothing to remove.
    #[tokio::test]
    async fn acquire_socket_binds_when_absent() {
        let path =
            std::env::temp_dir().join(format!("conclave-uds-absent-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let claimed = acquire_socket(&path).await;
        assert!(claimed.is_some(), "must bind when the path is free");

        drop(claimed);
        let _ = std::fs::remove_file(&path);
    }
}
