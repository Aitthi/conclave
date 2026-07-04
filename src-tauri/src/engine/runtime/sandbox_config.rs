//! Spawn-time sandbox config so a sandboxed agent CLI can reach the conclave
//! UDS socket without a permission modal.
//!
//! Both harnesses run the agent's shell tool calls under an OS sandbox
//! (macOS seatbelt). The conclave binary and its Unix-domain socket both live
//! under `~/Library/Application Support/Conclave/` — OUTSIDE the workspace — so
//! the sandbox denies the `connect()` to the socket (`EPERM`), and claude-code
//! additionally raises a one-time seatbelt modal. The fix is a spawn-time
//! config that pokes exactly ONE hole for that socket path and leaves the
//! sandbox on for everything else. Recipes are Guetta's research
//! (`docs/2026-07-04-harness-sandbox-conclave-cli.md`), the codex profile
//! proven empirically (test J) and the claude keys taken from the official
//! settings JSON schema.
//!
//! The socket path is resolved at runtime via [`crate::engine::uds::socket_path`]
//! (never hardcoded), so this tracks whatever `data_dir()` the host resolves.

use std::path::PathBuf;

/// The permission mode that turns the sandbox OFF entirely (claude `--yolo`
/// analogue). Under it there is no seatbelt to poke a hole through, so the
/// socket allowance is neither needed nor applied.
const BYPASS_MODE: &str = "bypassPermissions";

/// Whether a spawn in `permission_mode` runs sandboxed and therefore needs the
/// socket hole. Every mode except full bypass keeps the sandbox on (`default`,
/// `auto`, `acceptEdits`, `plan`, and unset all run sandboxed).
pub fn needs_socket_hole(permission_mode: Option<&str>) -> bool {
    permission_mode != Some(BYPASS_MODE)
}

/// The four codex `-c` overrides that allowlist the conclave socket, as
/// `dotted.key=<TOML value>` strings ready to pass one-per-`-c` at spawn.
///
/// Mirrors the `[permissions.conclave]` profile proven in the research (test J):
/// inherit `:workspace` (keeps out-of-workspace exec + writable roots), enable
/// the network permission subsystem (the unix-socket grant is inert without
/// it — no domains are opened, only the one socket), allowlist the socket, and
/// select the profile as the default applied to sandboxed tool calls. These are
/// per-spawn `-c` args and never touch the user's `~/.codex/config.toml`.
pub fn codex_socket_overrides(socket_path: &str) -> Vec<String> {
    vec![
        "permissions.conclave.extends=\":workspace\"".to_string(),
        "permissions.conclave.network.enabled=true".to_string(),
        // Inline TOML table with a double-quoted key: spaces + slashes in the
        // path are fine inside the quotes.
        format!(
            "permissions.conclave.network.unix_sockets={{\"{socket_path}\"=\"allow\"}}"
        ),
        "default_permissions=\"conclave\"".to_string(),
    ]
}

/// Build/merge the claude-code settings JSON that allowlists the conclave
/// socket in the sandbox and auto-approves the resulting sandboxed bash call.
///
/// `existing` is the parsed contents of any settings file already at the target
/// path (or `None`/`Null` for a fresh file); its other keys are preserved and
/// only the two `sandbox.*` keys we own are set. Route A from the research
/// (surgical — opens exactly the one socket, keeps conclave inside the sandbox):
/// `sandbox.network.allowUnixSockets` + `sandbox.autoAllowBashIfSandboxed`.
pub fn claude_sandbox_settings(socket_path: &str, existing: Option<serde_json::Value>) -> serde_json::Value {
    use serde_json::Value;
    let mut root = match existing {
        Some(Value::Object(_)) => existing.unwrap(),
        _ => Value::Object(serde_json::Map::new()),
    };
    let obj = root.as_object_mut().expect("root is an object");

    // Ensure `sandbox` is an object, preserving any sibling keys already there.
    let sandbox = obj
        .entry("sandbox")
        .and_modify(|v| {
            if !v.is_object() {
                *v = Value::Object(serde_json::Map::new());
            }
        })
        .or_insert_with(|| Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .expect("sandbox is an object");

    // sandbox.network.allowUnixSockets = [ <socket> ] — union the socket in so a
    // pre-existing allowlist keeps its entries.
    let network = sandbox
        .entry("network")
        .and_modify(|v| {
            if !v.is_object() {
                *v = Value::Object(serde_json::Map::new());
            }
        })
        .or_insert_with(|| Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .expect("network is an object");
    let list = network
        .entry("allowUnixSockets")
        .and_modify(|v| {
            if !v.is_array() {
                *v = Value::Array(Vec::new());
            }
        })
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .expect("allowUnixSockets is an array");
    if !list.iter().any(|v| v.as_str() == Some(socket_path)) {
        list.push(Value::String(socket_path.to_string()));
    }

    // sandbox.autoAllowBashIfSandboxed = true — skip the permission prompt for
    // commands that stay sandboxed.
    sandbox.insert("autoAllowBashIfSandboxed".to_string(), Value::Bool(true));

    root
}

/// Absolute path of the per-instance claude settings file we generate.
/// `<data_dir>/Conclave/agent-settings/<instance_id>.json` — beside the socket
/// and DB, never inside the user's workspace repo.
pub fn claude_settings_path(instance_id: &str) -> PathBuf {
    crate::engine::uds::socket_path()
        .parent()
        .expect("socket path has a Conclave parent dir")
        .join("agent-settings")
        .join(format!("{instance_id}.json"))
}

/// Merge the socket allowance into the per-instance claude settings file and
/// return its path (to pass via `--settings`). Reads any existing file at the
/// path and preserves its other keys; creates the `agent-settings` dir on
/// first use.
pub fn write_claude_settings(instance_id: &str, socket_path: &str) -> std::io::Result<PathBuf> {
    let path = claude_settings_path(instance_id);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let existing = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
    let merged = claude_sandbox_settings(socket_path, existing);
    let body = serde_json::to_string_pretty(&merged).map_err(std::io::Error::other)?;
    std::fs::write(&path, body)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn bypass_mode_needs_no_hole() {
        assert!(!needs_socket_hole(Some("bypassPermissions")));
    }

    #[test]
    fn sandboxed_modes_need_the_hole() {
        for mode in [None, Some("default"), Some("auto"), Some("acceptEdits"), Some("plan")] {
            assert!(needs_socket_hole(mode), "mode {mode:?} runs sandboxed");
        }
    }

    #[test]
    fn codex_overrides_match_proven_recipe() {
        let sock = "/Users/x/Library/Application Support/Conclave/conclave.sock";
        let ov = codex_socket_overrides(sock);
        assert_eq!(
            ov,
            vec![
                "permissions.conclave.extends=\":workspace\"".to_string(),
                "permissions.conclave.network.enabled=true".to_string(),
                format!("permissions.conclave.network.unix_sockets={{\"{sock}\"=\"allow\"}}"),
                "default_permissions=\"conclave\"".to_string(),
            ]
        );
    }

    #[test]
    fn codex_socket_override_embeds_exact_path() {
        let sock = "/tmp/space dir/conclave.sock";
        let ov = codex_socket_overrides(sock);
        assert_eq!(
            ov[2],
            "permissions.conclave.network.unix_sockets={\"/tmp/space dir/conclave.sock\"=\"allow\"}"
        );
    }

    #[test]
    fn claude_settings_fresh_has_route_a_keys() {
        let sock = "/Users/x/Library/Application Support/Conclave/conclave.sock";
        let v = claude_sandbox_settings(sock, None);
        assert_eq!(
            v["sandbox"]["network"]["allowUnixSockets"],
            json!([sock])
        );
        assert_eq!(v["sandbox"]["autoAllowBashIfSandboxed"], json!(true));
    }

    #[test]
    fn claude_settings_preserves_existing_keys_and_unions_socket() {
        let sock = "/tmp/conclave.sock";
        let existing = json!({
            "model": "opus",
            "sandbox": {
                "excludedCommands": ["git"],
                "network": { "allowUnixSockets": ["/other.sock"] }
            }
        });
        let v = claude_sandbox_settings(sock, Some(existing));
        // untouched sibling keys preserved
        assert_eq!(v["model"], json!("opus"));
        assert_eq!(v["sandbox"]["excludedCommands"], json!(["git"]));
        // socket unioned in, existing entry kept
        assert_eq!(
            v["sandbox"]["network"]["allowUnixSockets"],
            json!(["/other.sock", sock])
        );
        assert_eq!(v["sandbox"]["autoAllowBashIfSandboxed"], json!(true));
    }

    #[test]
    fn claude_settings_is_idempotent() {
        let sock = "/tmp/conclave.sock";
        let once = claude_sandbox_settings(sock, None);
        let twice = claude_sandbox_settings(sock, Some(once.clone()));
        assert_eq!(once, twice);
    }

    #[test]
    fn claude_settings_repairs_wrong_typed_nodes() {
        let sock = "/tmp/conclave.sock";
        // sandbox present but the wrong type — must be replaced, not panic.
        let existing = json!({ "sandbox": "oops" });
        let v = claude_sandbox_settings(sock, Some(existing));
        assert_eq!(v["sandbox"]["network"]["allowUnixSockets"], json!([sock]));
        assert!(v["sandbox"].is_object());
    }

    #[test]
    fn claude_settings_path_lives_beside_socket() {
        let p = claude_settings_path("inst-123");
        assert!(p.ends_with("agent-settings/inst-123.json"), "path was {p:?}");
        // sibling of the Conclave data dir, not the workspace
        assert!(p.to_string_lossy().contains("Conclave"));
    }

    #[test]
    fn non_object_existing_settings_start_fresh() {
        let sock = "/tmp/conclave.sock";
        let v: Value = claude_sandbox_settings(sock, Some(json!("not an object")));
        assert_eq!(v["sandbox"]["network"]["allowUnixSockets"], json!([sock]));
    }
}
