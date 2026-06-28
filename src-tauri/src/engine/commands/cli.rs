//! `cli.exec` — the allowlisted choke point for all CLI-to-router traffic.
//!
//! The Unix-Domain-Socket server (`uds.rs`) grants any same-user client access
//! to the full command router. `cli.exec` is the *only* method the external
//! `conclave-cli` binary calls; it maps a small, curated set of shell-style
//! subcommands to internal router methods and rejects everything else.
//!
//! **Security note (M5.1 review):** the match arms in [`map_argv`] are the
//! security boundary. There is intentionally no passthrough — a caller cannot
//! name an arbitrary router method through this path.

use crate::engine::{router, AppError, AppState};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
struct ExecReq {
    argv: Vec<String>,
}

/// Allowlisted CLI entry point — the single choke point through which the
/// external `conclave-cli` binary reaches the router (M5.1 security review).
///
/// Accepts `{ "argv": ["subcommand", …] }`, maps the argv vector to an
/// internal router method + params via [`map_argv`], then dispatches. Every
/// reachable router method is enumerated explicitly in `map_argv`; there is no
/// passthrough that allows a caller to name an arbitrary method.
///
/// # Note on `Box::pin`
/// The router contains `"cli.exec" => cli::exec(…)`, creating a potential
/// type-level recursive async cycle (`exec` → `dispatch` → `exec`). Because
/// `map_argv` never produces `"cli.exec"` as a method name, the recursion
/// never fires at runtime, but the compiler cannot verify this. `Box::pin`
/// wraps the dispatch future in a heap allocation, giving it a fixed pointer
/// size and breaking the infinite-type cycle (E0733).
pub async fn exec(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: ExecReq = serde_json::from_value(payload)
        .map_err(|e| AppError::Invalid(format!("cli.exec: bad payload: {e}")))?;
    let (method, params) = map_argv(&req.argv)?;
    Box::pin(router::dispatch(state, method, params)).await
}

/// Pure subcommand-to-router mapping — the explicit allowlist.
///
/// Returns `(&'static str method, Value params)` for a recognised subcommand,
/// or `AppError::Invalid` for anything outside the allowlist. This function
/// has no I/O side effects; it is called synchronously from [`exec`].
///
/// # Security
/// Every `match` arm names a concrete router method literal. There is no arm
/// that passes the subcommand through as a method name. Unknown subcommands
/// (including plausible router methods like `"provider.upsert"` or
/// `"cli.exec"`) fall through to the catch-all error.
fn map_argv(argv: &[String]) -> Result<(&'static str, Value), AppError> {
    let first = argv
        .first()
        .map(String::as_str)
        .ok_or_else(|| AppError::Invalid("cli: no subcommand given (try `help`)".into()))?;

    match first {
        // ── ws ────────────────────────────────────────────────────────────
        "ws" => match argv.get(1).map(String::as_str) {
            Some("list") => {
                if argv.len() != 2 {
                    return Err(AppError::Invalid("cli: ws list".into()));
                }
                Ok(("workspace.list", Value::Null))
            }
            Some("use") => {
                let workspace_id = argv
                    .get(2)
                    .ok_or_else(|| AppError::Invalid("cli: ws use <workspaceId>".into()))?;
                if argv.len() != 3 {
                    return Err(AppError::Invalid("cli: ws use <workspaceId>".into()));
                }
                Ok(("workspace.use", json!({ "workspaceId": workspace_id })))
            }
            _ => Err(AppError::Invalid(
                "cli: ws <list|use> — unknown ws subcommand".into(),
            )),
        },

        // ── agent ─────────────────────────────────────────────────────────
        "agent" => match argv.get(1).map(String::as_str) {
            Some("list") => {
                let workspace_id = argv
                    .get(2)
                    .ok_or_else(|| AppError::Invalid("cli: agent list <workspaceId>".into()))?;
                if argv.len() != 3 {
                    return Err(AppError::Invalid("cli: agent list <workspaceId>".into()));
                }
                Ok(("instance.list", json!({ "workspaceId": workspace_id })))
            }
            _ => Err(AppError::Invalid(
                "cli: agent <list> — unknown agent subcommand".into(),
            )),
        },

        // ── send ──────────────────────────────────────────────────────────
        "send" => {
            let session_id = argv
                .get(1)
                .ok_or_else(|| AppError::Invalid("cli: send <sessionId> <text...>".into()))?;
            if argv.len() < 3 {
                return Err(AppError::Invalid("cli: send <sessionId> <text...>".into()));
            }
            let text = argv[2..].join(" ");
            Ok((
                "message.send",
                json!({ "sessionId": session_id, "text": text }),
            ))
        }

        // ── tell (agent→agent injection) ──────────────────────────────────
        // `tell <fromInstanceId> <toInstanceId> <text...>` → message.inject.
        // The `conclave-cli` client fills <fromInstanceId> from CONCLAVE_INSTANCE_ID
        // so a spawned agent only types `conclave tell <agentId> <text>`. Identity
        // is client-supplied; consistent with the same-user UDS trust model (a
        // local process can already reach `send`).
        "tell" => {
            let from = argv.get(1).ok_or_else(|| {
                AppError::Invalid("cli: tell <fromInstanceId> <toInstanceId> <text...>".into())
            })?;
            let to = argv.get(2).ok_or_else(|| {
                AppError::Invalid("cli: tell <fromInstanceId> <toInstanceId> <text...>".into())
            })?;
            if argv.len() < 4 {
                return Err(AppError::Invalid(
                    "cli: tell <fromInstanceId> <toInstanceId> <text...>".into(),
                ));
            }
            let text = argv[3..].join(" ");
            Ok((
                "message.inject",
                json!({ "fromInstanceId": from, "toInstanceId": to, "text": text }),
            ))
        }

        // ── bb ────────────────────────────────────────────────────────────
        "bb" => match argv.get(1).map(String::as_str) {
            Some("list") => {
                let workspace_id = argv
                    .get(2)
                    .ok_or_else(|| AppError::Invalid("cli: bb list <workspaceId>".into()))?;
                if argv.len() != 3 {
                    return Err(AppError::Invalid("cli: bb list <workspaceId>".into()));
                }
                Ok(("blackboard.list", json!({ "workspaceId": workspace_id })))
            }
            Some("get") => {
                let workspace_id = argv
                    .get(2)
                    .ok_or_else(|| AppError::Invalid("cli: bb get <workspaceId> <key>".into()))?;
                let key = argv
                    .get(3)
                    .ok_or_else(|| AppError::Invalid("cli: bb get <workspaceId> <key>".into()))?;
                if argv.len() != 4 {
                    return Err(AppError::Invalid("cli: bb get <workspaceId> <key>".into()));
                }
                Ok((
                    "blackboard.get",
                    json!({ "workspaceId": workspace_id, "key": key }),
                ))
            }
            Some("set") => {
                let workspace_id = argv.get(2).ok_or_else(|| {
                    AppError::Invalid("cli: bb set <workspaceId> <key> <value>".into())
                })?;
                let key = argv.get(3).ok_or_else(|| {
                    AppError::Invalid("cli: bb set <workspaceId> <key> <value>".into())
                })?;
                let raw = argv.get(4).ok_or_else(|| {
                    AppError::Invalid("cli: bb set <workspaceId> <key> <value>".into())
                })?;
                if argv.len() != 5 {
                    return Err(AppError::Invalid(
                        "cli: bb set <workspaceId> <key> <value>".into(),
                    ));
                }
                // Parse `<value>` as JSON; fall back to bare string on failure.
                let value: Value =
                    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.clone()));
                Ok((
                    "blackboard.set",
                    json!({ "workspaceId": workspace_id, "key": key, "value": value }),
                ))
            }
            _ => Err(AppError::Invalid(
                "cli: bb <list|get|set> — unknown bb subcommand".into(),
            )),
        },

        // ── snapshot ──────────────────────────────────────────────────────
        "snapshot" => match argv.get(1).map(String::as_str) {
            Some("list") => {
                let session_id = argv
                    .get(2)
                    .ok_or_else(|| AppError::Invalid("cli: snapshot list <sessionId>".into()))?;
                if argv.len() != 3 {
                    return Err(AppError::Invalid("cli: snapshot list <sessionId>".into()));
                }
                Ok(("snapshot.list", json!({ "sessionId": session_id })))
            }
            Some("read") => {
                let snapshot_id = argv
                    .get(2)
                    .ok_or_else(|| AppError::Invalid("cli: snapshot read <snapshotId>".into()))?;
                if argv.len() != 3 {
                    return Err(AppError::Invalid("cli: snapshot read <snapshotId>".into()));
                }
                Ok(("snapshot.read", json!({ "snapshotId": snapshot_id })))
            }
            Some("create") => {
                let session_id = argv.get(2).ok_or_else(|| {
                    AppError::Invalid("cli: snapshot create <sessionId> <type> [label]".into())
                })?;
                let snap_type = argv.get(3).ok_or_else(|| {
                    AppError::Invalid("cli: snapshot create <sessionId> <type> [label]".into())
                })?;
                if argv.len() > 5 {
                    return Err(AppError::Invalid(
                        "cli: snapshot create <sessionId> <type> [label]".into(),
                    ));
                }
                let mut params = json!({ "sessionId": session_id, "type": snap_type });
                if let Some(label) = argv.get(4) {
                    params["label"] = json!(label);
                }
                Ok(("snapshot.create", params))
            }
            // Agent self-handoff (strategic-compact). `save`/`last` are keyed on
            // the INSTANCE id; the `conclave-cli` client fills it from
            // CONCLAVE_INSTANCE_ID so a spawned agent types only
            // `conclave snapshot save <text>` / `conclave snapshot last`.
            Some("save") => {
                let instance_id = argv.get(2).ok_or_else(|| {
                    AppError::Invalid("cli: snapshot save <instanceId> <text...>".into())
                })?;
                if argv.len() < 4 {
                    return Err(AppError::Invalid(
                        "cli: snapshot save <instanceId> <text...>".into(),
                    ));
                }
                let text = argv[3..].join(" ");
                Ok((
                    "snapshot.save",
                    json!({ "instanceId": instance_id, "text": text }),
                ))
            }
            Some("last") => {
                let instance_id = argv
                    .get(2)
                    .ok_or_else(|| AppError::Invalid("cli: snapshot last <instanceId>".into()))?;
                if argv.len() != 3 {
                    return Err(AppError::Invalid("cli: snapshot last <instanceId>".into()));
                }
                Ok(("snapshot.last", json!({ "instanceId": instance_id })))
            }
            _ => Err(AppError::Invalid(
                "cli: snapshot <list|read|create|save|last> — unknown snapshot subcommand".into(),
            )),
        },

        // ── run ───────────────────────────────────────────────────────────
        "run" => {
            let orchestrator_id = argv
                .get(1)
                .ok_or_else(|| AppError::Invalid("cli: run <orchestratorId> <prompt...>".into()))?;
            if argv.len() < 3 {
                return Err(AppError::Invalid(
                    "cli: run <orchestratorId> <prompt...>".into(),
                ));
            }
            let prompt = argv[2..].join(" ");
            Ok((
                "fusion.run",
                json!({ "orchestratorId": orchestrator_id, "prompt": prompt }),
            ))
        }

        // ── unknown — security catch-all ──────────────────────────────────
        other => Err(AppError::Invalid(format!(
            "cli: unknown subcommand '{other}' (allowed: ws, agent, send, tell, bb, snapshot, run)"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── helpers ───────────────────────────────────────────────────────────

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    fn ok_method(words: &[&str]) -> &'static str {
        map_argv(&argv(words)).expect("expected Ok").0
    }

    fn ok_params(words: &[&str]) -> Value {
        map_argv(&argv(words)).expect("expected Ok").1
    }

    fn is_invalid(words: &[&str]) -> bool {
        matches!(map_argv(&argv(words)), Err(AppError::Invalid(_)))
    }

    // ── ws ────────────────────────────────────────────────────────────────

    #[test]
    fn ws_list_maps_correctly() {
        assert_eq!(ok_method(&["ws", "list"]), "workspace.list");
        assert_eq!(ok_params(&["ws", "list"]), Value::Null);
    }

    #[test]
    fn ws_use_maps_correctly() {
        assert_eq!(ok_method(&["ws", "use", "abc"]), "workspace.use");
        assert_eq!(
            ok_params(&["ws", "use", "abc"]),
            json!({ "workspaceId": "abc" })
        );
    }

    #[test]
    fn ws_list_rejects_extra_args() {
        assert!(is_invalid(&["ws", "list", "extra"]));
    }

    #[test]
    fn ws_use_missing_id_is_invalid() {
        assert!(is_invalid(&["ws", "use"]));
    }

    #[test]
    fn ws_use_extra_args_is_invalid() {
        assert!(is_invalid(&["ws", "use", "id", "extra"]));
    }

    #[test]
    fn ws_unknown_sub_is_invalid() {
        assert!(is_invalid(&["ws", "delete"]));
    }

    // ── agent ─────────────────────────────────────────────────────────────

    #[test]
    fn agent_list_maps_correctly() {
        assert_eq!(ok_method(&["agent", "list", "ws1"]), "instance.list");
        assert_eq!(
            ok_params(&["agent", "list", "ws1"]),
            json!({ "workspaceId": "ws1" })
        );
    }

    #[test]
    fn agent_list_missing_workspace_is_invalid() {
        assert!(is_invalid(&["agent", "list"]));
    }

    #[test]
    fn agent_list_extra_args_is_invalid() {
        assert!(is_invalid(&["agent", "list", "ws1", "extra"]));
    }

    #[test]
    fn agent_unknown_sub_is_invalid() {
        assert!(is_invalid(&["agent", "spawn"]));
    }

    // ── send ──────────────────────────────────────────────────────────────

    #[test]
    fn send_single_word_maps_correctly() {
        assert_eq!(ok_method(&["send", "sess1", "hello"]), "message.send");
        assert_eq!(
            ok_params(&["send", "sess1", "hello"]),
            json!({ "sessionId": "sess1", "text": "hello" })
        );
    }

    #[test]
    fn send_multi_word_text_joined_with_spaces() {
        let params = ok_params(&["send", "sess1", "hello", "world", "foo"]);
        assert_eq!(params["text"], json!("hello world foo"));
        assert_eq!(params["sessionId"], json!("sess1"));
    }

    #[test]
    fn send_missing_text_is_invalid() {
        assert!(is_invalid(&["send", "sess1"]));
    }

    #[test]
    fn send_missing_session_is_invalid() {
        assert!(is_invalid(&["send"]));
    }

    // ── tell ──────────────────────────────────────────────────────────────

    #[test]
    fn tell_maps_to_inject() {
        assert_eq!(ok_method(&["tell", "from1", "to1", "hi"]), "message.inject");
        assert_eq!(
            ok_params(&["tell", "from1", "to1", "hi"]),
            json!({ "fromInstanceId": "from1", "toInstanceId": "to1", "text": "hi" })
        );
    }

    #[test]
    fn tell_joins_multiword_text() {
        let params = ok_params(&["tell", "from1", "to1", "hello", "there", "peer"]);
        assert_eq!(params["text"], json!("hello there peer"));
    }

    #[test]
    fn tell_missing_text_is_invalid() {
        assert!(is_invalid(&["tell", "from1", "to1"]));
    }

    #[test]
    fn tell_missing_target_is_invalid() {
        assert!(is_invalid(&["tell", "from1"]));
    }

    // ── bb ────────────────────────────────────────────────────────────────

    #[test]
    fn bb_list_maps_correctly() {
        assert_eq!(ok_method(&["bb", "list", "ws1"]), "blackboard.list");
        assert_eq!(
            ok_params(&["bb", "list", "ws1"]),
            json!({ "workspaceId": "ws1" })
        );
    }

    #[test]
    fn bb_list_missing_workspace_is_invalid() {
        assert!(is_invalid(&["bb", "list"]));
    }

    #[test]
    fn bb_list_extra_args_is_invalid() {
        assert!(is_invalid(&["bb", "list", "ws1", "extra"]));
    }

    #[test]
    fn bb_get_maps_correctly() {
        assert_eq!(ok_method(&["bb", "get", "ws1", "mykey"]), "blackboard.get");
        assert_eq!(
            ok_params(&["bb", "get", "ws1", "mykey"]),
            json!({ "workspaceId": "ws1", "key": "mykey" })
        );
    }

    #[test]
    fn bb_get_missing_key_is_invalid() {
        assert!(is_invalid(&["bb", "get", "ws1"]));
    }

    #[test]
    fn bb_get_missing_workspace_is_invalid() {
        assert!(is_invalid(&["bb", "get"]));
    }

    #[test]
    fn bb_get_extra_args_is_invalid() {
        assert!(is_invalid(&["bb", "get", "ws1", "key", "extra"]));
    }

    #[test]
    fn bb_set_maps_correctly() {
        assert_eq!(ok_method(&["bb", "set", "ws1", "k", "v"]), "blackboard.set");
        let params = ok_params(&["bb", "set", "ws1", "k", "v"]);
        assert_eq!(params["workspaceId"], json!("ws1"));
        assert_eq!(params["key"], json!("k"));
        assert_eq!(params["value"], json!("v"));
    }

    #[test]
    fn bb_set_missing_value_is_invalid() {
        assert!(is_invalid(&["bb", "set", "ws1", "k"]));
    }

    #[test]
    fn bb_set_missing_key_is_invalid() {
        assert!(is_invalid(&["bb", "set", "ws1"]));
    }

    #[test]
    fn bb_set_extra_args_is_invalid() {
        assert!(is_invalid(&["bb", "set", "ws1", "k", "v", "extra"]));
    }

    #[test]
    fn bb_unknown_sub_is_invalid() {
        assert!(is_invalid(&["bb", "delete", "ws1", "k"]));
    }

    /// `bb set` JSON value coercion: numeric string → JSON number.
    #[test]
    fn bb_set_numeric_string_becomes_number() {
        let params = ok_params(&["bb", "set", "ws1", "k", "42"]);
        assert_eq!(params["value"], json!(42));
    }

    /// `bb set` JSON value coercion: `"true"` → JSON boolean.
    #[test]
    fn bb_set_bool_string_becomes_bool() {
        let params = ok_params(&["bb", "set", "ws1", "k", "true"]);
        assert_eq!(params["value"], json!(true));
    }

    /// `bb set` JSON value coercion: bare word → JSON string.
    #[test]
    fn bb_set_bare_word_becomes_string() {
        let params = ok_params(&["bb", "set", "ws1", "k", "hello"]);
        assert_eq!(params["value"], json!("hello"));
    }

    /// `bb set` JSON value coercion: valid JSON object stays an object.
    #[test]
    fn bb_set_json_object_stays_object() {
        let params = ok_params(&["bb", "set", "ws1", "k", r#"{"a":1}"#]);
        assert_eq!(params["value"], json!({ "a": 1 }));
    }

    // ── snapshot ──────────────────────────────────────────────────────────

    #[test]
    fn snapshot_list_maps_correctly() {
        assert_eq!(ok_method(&["snapshot", "list", "sess1"]), "snapshot.list");
        assert_eq!(
            ok_params(&["snapshot", "list", "sess1"]),
            json!({ "sessionId": "sess1" })
        );
    }

    #[test]
    fn snapshot_list_missing_session_is_invalid() {
        assert!(is_invalid(&["snapshot", "list"]));
    }

    #[test]
    fn snapshot_list_extra_args_is_invalid() {
        assert!(is_invalid(&["snapshot", "list", "s1", "extra"]));
    }

    #[test]
    fn snapshot_read_maps_correctly() {
        assert_eq!(ok_method(&["snapshot", "read", "snap1"]), "snapshot.read");
        assert_eq!(
            ok_params(&["snapshot", "read", "snap1"]),
            json!({ "snapshotId": "snap1" })
        );
    }

    #[test]
    fn snapshot_read_missing_id_is_invalid() {
        assert!(is_invalid(&["snapshot", "read"]));
    }

    #[test]
    fn snapshot_read_extra_args_is_invalid() {
        assert!(is_invalid(&["snapshot", "read", "s1", "extra"]));
    }

    #[test]
    fn snapshot_create_without_label_maps_correctly() {
        assert_eq!(
            ok_method(&["snapshot", "create", "sess1", "auto"]),
            "snapshot.create"
        );
        let params = ok_params(&["snapshot", "create", "sess1", "auto"]);
        assert_eq!(params["sessionId"], json!("sess1"));
        assert_eq!(params["type"], json!("auto"));
        assert!(params.get("label").is_none(), "label absent when not given");
    }

    #[test]
    fn snapshot_create_with_label_maps_correctly() {
        let params = ok_params(&["snapshot", "create", "sess1", "manual", "my-label"]);
        assert_eq!(params["sessionId"], json!("sess1"));
        assert_eq!(params["type"], json!("manual"));
        assert_eq!(params["label"], json!("my-label"));
    }

    #[test]
    fn snapshot_create_missing_type_is_invalid() {
        assert!(is_invalid(&["snapshot", "create", "sess1"]));
    }

    #[test]
    fn snapshot_create_extra_args_is_invalid() {
        assert!(is_invalid(&[
            "snapshot", "create", "sess1", "auto", "label", "extra"
        ]));
    }

    #[test]
    fn snapshot_create_missing_session_is_invalid() {
        assert!(is_invalid(&["snapshot", "create"]));
    }

    #[test]
    fn snapshot_unknown_sub_is_invalid() {
        assert!(is_invalid(&["snapshot", "delete", "s1"]));
    }

    #[test]
    fn snapshot_save_maps_correctly() {
        assert_eq!(
            ok_method(&["snapshot", "save", "inst1", "my", "handoff"]),
            "snapshot.save"
        );
        let params = ok_params(&["snapshot", "save", "inst1", "my", "handoff"]);
        assert_eq!(params["instanceId"], json!("inst1"));
        assert_eq!(params["text"], json!("my handoff"));
    }

    #[test]
    fn snapshot_save_missing_text_is_invalid() {
        assert!(is_invalid(&["snapshot", "save", "inst1"]));
    }

    #[test]
    fn snapshot_last_maps_correctly() {
        assert_eq!(ok_method(&["snapshot", "last", "inst1"]), "snapshot.last");
        assert_eq!(
            ok_params(&["snapshot", "last", "inst1"]),
            json!({ "instanceId": "inst1" })
        );
    }

    #[test]
    fn snapshot_last_missing_instance_is_invalid() {
        assert!(is_invalid(&["snapshot", "last"]));
    }

    #[test]
    fn snapshot_last_extra_args_is_invalid() {
        assert!(is_invalid(&["snapshot", "last", "inst1", "extra"]));
    }

    // ── run ───────────────────────────────────────────────────────────────

    #[test]
    fn run_single_word_prompt_maps_correctly() {
        assert_eq!(ok_method(&["run", "orch1", "go"]), "fusion.run");
        assert_eq!(
            ok_params(&["run", "orch1", "go"]),
            json!({ "orchestratorId": "orch1", "prompt": "go" })
        );
    }

    #[test]
    fn run_multi_word_prompt_joined_with_spaces() {
        let params = ok_params(&["run", "orch1", "do", "the", "thing"]);
        assert_eq!(params["orchestratorId"], json!("orch1"));
        assert_eq!(params["prompt"], json!("do the thing"));
    }

    #[test]
    fn run_missing_prompt_is_invalid() {
        assert!(is_invalid(&["run", "orch1"]));
    }

    #[test]
    fn run_missing_orchestrator_is_invalid() {
        assert!(is_invalid(&["run"]));
    }

    // ── empty argv ────────────────────────────────────────────────────────

    #[test]
    fn empty_argv_is_invalid() {
        assert!(is_invalid(&[]));
    }

    // ── unknown top-level subcommand ──────────────────────────────────────

    #[test]
    fn unknown_subcommand_is_invalid() {
        assert!(is_invalid(&["nope"]));
    }

    // ── security tests ────────────────────────────────────────────────────

    /// Callers must NOT be able to reach `provider.upsert` via cli.exec.
    #[test]
    fn security_provider_upsert_is_rejected() {
        assert!(
            is_invalid(&["provider.upsert", "x"]),
            "router method names must not pass through as subcommands"
        );
    }

    /// Callers must NOT be able to invoke `cli.exec` recursively.
    #[test]
    fn security_cli_exec_is_rejected() {
        assert!(
            is_invalid(&["cli", "exec"]),
            "cli.exec must not be reachable as a subcommand"
        );
    }

    // ── async integration test through exec() ─────────────────────────────

    #[tokio::test]
    async fn exec_ws_list_returns_ok_array() {
        let state = AppState::for_tests().await;
        let result = exec(&state, json!({ "argv": ["ws", "list"] })).await;
        let value = result.expect("exec ws list should succeed");
        assert!(value.is_array(), "workspace.list returns an array: {value}");
    }
}
