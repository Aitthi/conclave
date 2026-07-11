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
/// `proxy_port` is `Some` when this spawn's env got the context-proxy
/// `ANTHROPIC_BASE_URL` override (agent-proxy spec D8): the profile then also
/// allows the loopback host in its network `domains` map — same inline-TOML
/// map syntax as the proven `unix_sockets` grant — so nested CLI calls made
/// from the sandboxed shell can reach the proxy. Keyed by host only: the
/// domain map (like claude's `allowedDomains`) has no port syntax.
pub fn codex_socket_overrides(socket_path: &str, proxy_port: Option<u16>) -> Vec<String> {
    let mut overrides = vec![
        "permissions.conclave.extends=\":workspace\"".to_string(),
        "permissions.conclave.network.enabled=true".to_string(),
        // Inline TOML table with a double-quoted key: spaces + slashes in the
        // path are fine inside the quotes.
        format!("permissions.conclave.network.unix_sockets={{\"{socket_path}\"=\"allow\"}}"),
        "default_permissions=\"conclave\"".to_string(),
    ];
    if proxy_port.is_some() {
        overrides.push(
            "permissions.conclave.network.domains={\"127.0.0.1\"=\"allow\"}".to_string(),
        );
    }
    overrides
}

/// Build/merge the claude-code settings JSON that allowlists the conclave
/// socket in the sandbox and auto-approves the resulting sandboxed bash call.
///
/// `existing` is the parsed contents of any settings file already at the target
/// path (or `None`/`Null` for a fresh file); its other keys are preserved and
/// only the `sandbox.*` keys we own are set. Route A from the research
/// (surgical — opens exactly the one socket, keeps conclave inside the sandbox):
/// `sandbox.network.allowUnixSockets` + `sandbox.autoAllowBashIfSandboxed`.
///
/// `proxy_port` is `Some` when this spawn's env got the context-proxy
/// `ANTHROPIC_BASE_URL` override (agent-proxy spec D8): the sandbox then also
/// pre-allows the loopback host via `sandbox.network.allowedDomains`, so
/// nested CLI calls from the sandboxed shell can reach the proxy without a
/// per-domain prompt. Entries are domain patterns (no port syntax per the
/// sandboxing docs), so the hole is `127.0.0.1`, not `127.0.0.1:<port>`.
pub fn claude_sandbox_settings(
    socket_path: &str,
    proxy_port: Option<u16>,
    existing: Option<serde_json::Value>,
) -> serde_json::Value {
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

    // sandbox.network.allowedDomains gains the loopback host — only when the
    // context-proxy injection fired, unioned so foreign entries survive and
    // re-merge stays idempotent. When it didn't fire the key is not even
    // created (settings stay byte-identical to the pre-proxy shape).
    if proxy_port.is_some() {
        let domains = network
            .entry("allowedDomains")
            .and_modify(|v| {
                if !v.is_array() {
                    *v = Value::Array(Vec::new());
                }
            })
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .expect("allowedDomains is an array");
        if !domains.iter().any(|v| v.as_str() == Some("127.0.0.1")) {
            domains.push(Value::String("127.0.0.1".to_string()));
        }
    }

    // sandbox.autoAllowBashIfSandboxed = true — skip the permission prompt for
    // commands that stay sandboxed.
    sandbox.insert("autoAllowBashIfSandboxed".to_string(), Value::Bool(true));

    root
}

/// The marker phrase the transcript-backed context meter matches on
/// (`engine::runtime::transcript_context`). Also used here to recognize (and
/// replace) our own hook group on re-merge.
const OWNER_MARKER_PHRASE: &str = "own agent id is";

/// Shell command for a SessionStart hook that injects the owner marker as
/// `additionalContext`. claude-code records that context in the transcript
/// (as a `hook_additional_context` attachment) on EVERY session start —
/// startup, resume, /clear, compact — which is what lets the transcript
/// context meter attribute the transcript to this instance. The system-prompt
/// append carries the same sentence but is never written to the transcript.
pub fn owner_marker_command(instance_id: &str) -> String {
    let payload = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "SessionStart",
            "additionalContext": format!(
                "You are a Conclave agent, and your own agent id is {instance_id}."
            )
        }
    });
    // Instance ids are UUIDs (no quotes to escape); the JSON body uses only
    // double quotes, so single-quoting it is a single safe shell word.
    format!("echo '{payload}'")
}

/// Absolute paths (resolved at spawn/settings-write time, per A2) of the
/// conclave CLI shim and the bundled rtk binary, embedded verbatim into the
/// generated PreToolUse hook command. Claude Code hook execution does not
/// inherit the agent shell's PATH, so these must be absolute — never a bare
/// `conclave`/`rtk`.
pub struct RtkHook {
    pub cli_bin: PathBuf,
    pub rtk_bin: PathBuf,
}

/// Substring that identifies a conclave-owned PreToolUse hook group (as
/// opposed to a foreign one) on re-merge, mirroring [`OWNER_MARKER_PHRASE`]
/// for the SessionStart group.
const RTK_HOOK_MARKER: &str = "rtk-hook";

/// The exact hook command contract (Lane A writes it, Lane B implements it):
/// both absolute paths single-quoted, e.g.
/// `'<cli_bin>' rtk-hook --rtk '<rtk_bin>'`.
fn rtk_hook_command(rtk: &RtkHook) -> String {
    format!(
        "'{}' rtk-hook --rtk '{}'",
        rtk.cli_bin.display(),
        rtk.rtk_bin.display()
    )
}

/// Build the single `-c` override value that registers the rtk PreToolUse hook
/// on a **codex** spawn (Lane K). Where claude-code gets a persisted
/// per-instance settings JSON, codex takes the whole hook table inline on one
/// `-c` flag as an array-of-tables TOML literal on the dotted leaf
/// `hooks.PreToolUse`. The embedded command is the SAME [`rtk_hook_command`]
/// claude uses (both paths single-quoted); it lands inside a TOML
/// double-quoted string, which is safe because the command never contains a
/// double quote. `matcher="^Bash$"` scopes the hook to Bash tool calls (codex
/// mirrors claude's tool naming); `timeout` is in SECONDS — codex's config key
/// is literally `timeout` (default 600), NOT `timeoutSec`.
///
/// The caller (`commands::instance`) MUST also pass
/// `--dangerously-bypass-hook-trust` on the same spawn — without it a
/// `-c`-injected hook SILENTLY never fires (no warning, no error), because
/// injected hooks aren't in codex's persisted trust store (codex-cli 0.144.1,
/// verified live in Guetta's research, task codex-hooks-research).
pub fn codex_rtk_hook_override(rtk: &RtkHook) -> String {
    format!(
        "hooks.PreToolUse=[{{matcher=\"^Bash$\",hooks=[{{type=\"command\",command=\"{}\",timeout=30}}]}}]",
        rtk_hook_command(rtk)
    )
}

/// Build the full per-instance claude settings: always the owner-marker
/// SessionStart hook; plus the sandbox socket allowance when the spawn runs
/// sandboxed (`socket_path` present); plus an optional PreToolUse hook that
/// routes Bash calls through the bundled rtk token filter (`rtk` present).
/// Preserves foreign keys and any foreign SessionStart/PreToolUse hook groups
/// in `existing`; our own marker groups (identified by the marker phrase /
/// `"rtk-hook"` substring) are replaced, so a stale instance id or a stale rtk
/// path self-repairs. When `rtk` is `None`, the `PreToolUse` key is left
/// entirely untouched (not even created empty) — fail-open, no-op.
pub fn claude_agent_settings(
    instance_id: &str,
    socket_path: Option<&str>,
    existing: Option<serde_json::Value>,
    rtk: Option<&RtkHook>,
    proxy_port: Option<u16>,
) -> serde_json::Value {
    use serde_json::Value;
    let mut root = match socket_path {
        Some(sock) => claude_sandbox_settings(sock, proxy_port, existing),
        None => match existing {
            Some(v @ Value::Object(_)) => v,
            _ => Value::Object(serde_json::Map::new()),
        },
    };
    let obj = root.as_object_mut().expect("root is an object");

    let hooks = obj
        .entry("hooks")
        .and_modify(|v| {
            if !v.is_object() {
                *v = Value::Object(serde_json::Map::new());
            }
        })
        .or_insert_with(|| Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .expect("hooks is an object");
    let session_start = hooks
        .entry("SessionStart")
        .and_modify(|v| {
            if !v.is_array() {
                *v = Value::Array(Vec::new());
            }
        })
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .expect("SessionStart is an array");

    // Drop any previous conclave marker group (ours carry the marker phrase in
    // their command), keep everything else, then append the current one.
    session_start.retain(|group| {
        !group
            .pointer("/hooks/0/command")
            .and_then(Value::as_str)
            .is_some_and(|cmd| cmd.contains(OWNER_MARKER_PHRASE))
    });
    session_start.push(serde_json::json!({
        "hooks": [{ "type": "command", "command": owner_marker_command(instance_id) }]
    }));

    // PreToolUse rtk-hook group: only touch the key at all when rtk routing
    // is requested. Mirrors the SessionStart dedup/replace above, but keyed
    // on the `"rtk-hook"` substring rather than the owner-marker phrase.
    if let Some(rtk) = rtk {
        let pre_tool_use = hooks
            .entry("PreToolUse")
            .and_modify(|v| {
                if !v.is_array() {
                    *v = Value::Array(Vec::new());
                }
            })
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .expect("PreToolUse is an array");

        // Drop any previous conclave rtk-hook group, keep everything else
        // (foreign PreToolUse entries), then append the current one.
        pre_tool_use.retain(|group| {
            !group
                .pointer("/hooks/0/command")
                .and_then(Value::as_str)
                .is_some_and(|cmd| cmd.contains(RTK_HOOK_MARKER))
        });
        pre_tool_use.push(serde_json::json!({
            "matcher": "Bash",
            "hooks": [{ "type": "command", "command": rtk_hook_command(rtk) }]
        }));
    }

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

/// Write the per-instance claude settings file and return its path (to pass
/// via `--settings`). Always merges the owner-marker SessionStart hook; the
/// socket allowance is merged only when `socket_path` is present (sandboxed
/// spawn); the PreToolUse rtk-hook group is merged only when `rtk` is present.
/// Reads any existing file at the path and preserves its other keys; creates
/// the `agent-settings` dir on first use.
pub fn write_claude_settings(
    instance_id: &str,
    socket_path: Option<&str>,
    rtk: Option<&RtkHook>,
    proxy_port: Option<u16>,
) -> std::io::Result<PathBuf> {
    let path = claude_settings_path(instance_id);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let existing = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
    let merged = claude_agent_settings(instance_id, socket_path, existing, rtk, proxy_port);
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
        for mode in [
            None,
            Some("default"),
            Some("auto"),
            Some("acceptEdits"),
            Some("plan"),
        ] {
            assert!(needs_socket_hole(mode), "mode {mode:?} runs sandboxed");
        }
    }

    #[test]
    fn codex_overrides_match_proven_recipe() {
        let sock = "/Users/x/Library/Application Support/Conclave/conclave.sock";
        let ov = codex_socket_overrides(sock, None);
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
        let ov = codex_socket_overrides(sock, None);
        assert_eq!(
            ov[2],
            "permissions.conclave.network.unix_sockets={\"/tmp/space dir/conclave.sock\"=\"allow\"}"
        );
    }

    #[test]
    fn codex_rtk_hook_override_embeds_command_and_bash_matcher() {
        // The inline-TOML value must carry the exact rtk_hook_command claude
        // uses, an `^Bash$` matcher, and a `timeout` (seconds) key — the whole
        // thing on the single dotted leaf `hooks.PreToolUse`.
        let rtk = RtkHook {
            cli_bin: PathBuf::from("/Users/x/Library/Application Support/Conclave/bin/conclave"),
            rtk_bin: PathBuf::from("/Users/x/Library/Application Support/Conclave/bin/rtk"),
        };
        let ov = codex_rtk_hook_override(&rtk);
        assert_eq!(
            ov,
            "hooks.PreToolUse=[{matcher=\"^Bash$\",hooks=[{type=\"command\",command=\"'/Users/x/Library/Application Support/Conclave/bin/conclave' rtk-hook --rtk '/Users/x/Library/Application Support/Conclave/bin/rtk'\",timeout=30}]}]"
        );
        // No double quote inside the embedded command (paths are single-quoted),
        // so the TOML double-quoted `command="..."` string never needs escaping.
        assert!(!rtk_hook_command(&rtk).contains('"'));
    }

    #[test]
    fn agent_settings_always_carry_owner_marker_hook() {
        // The transcript context meter can only attribute a claude transcript
        // to an instance if the owner marker is RECORDED in it; the SessionStart
        // hook is the recorded channel (--append-system-prompt never is).
        let v = claude_agent_settings("inst-9", None, None, None, None);
        let cmd = v["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .expect("hook command present");
        assert!(cmd.contains("own agent id is inst-9"), "command was {cmd}");
        // No socket → no sandbox keys.
        assert!(v.get("sandbox").is_none());
    }

    #[test]
    fn agent_settings_with_socket_carry_hook_and_sandbox() {
        let sock = "/tmp/conclave.sock";
        let v = claude_agent_settings("inst-9", Some(sock), None, None, None);
        assert_eq!(v["sandbox"]["network"]["allowUnixSockets"], json!([sock]));
        let cmd = v["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .expect("hook command present");
        assert!(cmd.contains("own agent id is inst-9"));
    }

    #[test]
    fn agent_settings_owner_hook_is_idempotent_and_self_repairing() {
        let once = claude_agent_settings("inst-9", None, None, None, None);
        let twice = claude_agent_settings("inst-9", None, Some(once.clone()), None, None);
        assert_eq!(once, twice, "re-merge must not duplicate the hook group");

        // A stale marker for a different id (e.g. file reused) is replaced.
        let stale = claude_agent_settings("inst-old", None, None, None, None);
        let repaired = claude_agent_settings("inst-9", None, Some(stale), None, None);
        let groups = repaired["hooks"]["SessionStart"]
            .as_array()
            .expect("SessionStart groups");
        assert_eq!(groups.len(), 1);
        assert!(groups[0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("own agent id is inst-9"));
    }

    #[test]
    fn agent_settings_with_rtk_adds_pre_tool_use_bash_hook() {
        let rtk = RtkHook {
            cli_bin: PathBuf::from("/Users/x/Library/Application Support/Conclave/bin/conclave"),
            rtk_bin: PathBuf::from("/Users/x/Library/Application Support/Conclave/bin/rtk"),
        };
        let v = claude_agent_settings("inst-9", None, None, Some(&rtk), None);
        assert_eq!(v["hooks"]["PreToolUse"][0]["matcher"], json!("Bash"));
        let cmd = v["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .expect("hook command present");
        assert_eq!(
            cmd,
            "'/Users/x/Library/Application Support/Conclave/bin/conclave' rtk-hook --rtk '/Users/x/Library/Application Support/Conclave/bin/rtk'"
        );
    }

    #[test]
    fn agent_settings_without_rtk_adds_no_pre_tool_use_key() {
        let v = claude_agent_settings("inst-9", None, None, None, None);
        assert!(v["hooks"].get("PreToolUse").is_none());
    }

    #[test]
    fn agent_settings_without_rtk_preserves_foreign_pre_tool_use_untouched() {
        let existing = json!({
            "hooks": { "PreToolUse": [
                { "matcher": "Bash", "hooks": [{ "type": "command", "command": "echo foreign" }] }
            ]}
        });
        let v = claude_agent_settings("inst-9", None, Some(existing.clone()), None, None);
        assert_eq!(
            v["hooks"]["PreToolUse"], existing["hooks"]["PreToolUse"],
            "foreign PreToolUse entry must survive untouched when rtk is None"
        );
    }

    #[test]
    fn agent_settings_rtk_replaces_prior_conclave_group_not_duplicate() {
        let rtk = RtkHook {
            cli_bin: PathBuf::from("/bin/conclave"),
            rtk_bin: PathBuf::from("/bin/rtk"),
        };
        let once = claude_agent_settings("inst-9", None, None, Some(&rtk), None);
        let twice = claude_agent_settings("inst-9", None, Some(once.clone()), Some(&rtk), None);
        assert_eq!(
            once, twice,
            "re-merge must not duplicate the rtk hook group"
        );
        let groups = twice["hooks"]["PreToolUse"].as_array().expect("groups");
        assert_eq!(groups.len(), 1);
    }

    #[test]
    fn agent_settings_rtk_preserves_foreign_pre_tool_use_entries() {
        let rtk = RtkHook {
            cli_bin: PathBuf::from("/bin/conclave"),
            rtk_bin: PathBuf::from("/bin/rtk"),
        };
        let existing = json!({
            "hooks": { "PreToolUse": [
                { "matcher": "Write", "hooks": [{ "type": "command", "command": "echo foreign" }] }
            ]}
        });
        let v = claude_agent_settings("inst-9", None, Some(existing), Some(&rtk), None);
        let groups = v["hooks"]["PreToolUse"].as_array().expect("groups");
        assert_eq!(
            groups.len(),
            2,
            "foreign PreToolUse group must survive the merge"
        );
        assert!(groups
            .iter()
            .any(|g| g["hooks"][0]["command"].as_str() == Some("echo foreign")));
        assert!(groups.iter().any(|g| g["matcher"] == json!("Bash")));
    }

    #[test]
    fn agent_settings_preserve_foreign_session_start_hooks() {
        let existing = json!({
            "hooks": { "SessionStart": [
                { "hooks": [{ "type": "command", "command": "echo unrelated" }] }
            ]}
        });
        let v = claude_agent_settings("inst-9", None, Some(existing), None, None);
        let groups = v["hooks"]["SessionStart"].as_array().expect("groups");
        assert_eq!(groups.len(), 2, "foreign hook group must survive the merge");
        assert!(groups
            .iter()
            .any(|g| g["hooks"][0]["command"].as_str() == Some("echo unrelated")));
    }

    #[test]
    fn owner_marker_command_emits_additional_context_json() {
        let cmd = owner_marker_command("inst-9");
        // Single shell word via echo + single quotes; payload is the documented
        // SessionStart additionalContext envelope.
        assert!(cmd.starts_with("echo '"), "command was {cmd}");
        let json_body = cmd.trim_start_matches("echo '").trim_end_matches('\'');
        let v: Value = serde_json::from_str(json_body).expect("payload is valid JSON");
        assert_eq!(
            v["hookSpecificOutput"]["hookEventName"],
            json!("SessionStart")
        );
        assert!(v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("own agent id is inst-9"));
    }

    /// Context-proxy loopback allowlist (agent-proxy Task 11): when the spawn
    /// injected `ANTHROPIC_BASE_URL`, the sandboxed shell's network allowlist
    /// gains the loopback host so nested CLI calls can reach the proxy. Claude
    /// `allowedDomains` entries are domain patterns (no port syntax per the
    /// sandboxing docs), so the hole is `127.0.0.1`, not `127.0.0.1:<port>`.
    #[test]
    fn claude_settings_with_proxy_allow_loopback_domain() {
        let sock = "/tmp/conclave.sock";
        let v = claude_sandbox_settings(sock, Some(18787), None);
        assert_eq!(
            v["sandbox"]["network"]["allowedDomains"],
            json!(["127.0.0.1"])
        );
        // The socket hole and auto-allow are unaffected.
        assert_eq!(v["sandbox"]["network"]["allowUnixSockets"], json!([sock]));
        assert_eq!(v["sandbox"]["autoAllowBashIfSandboxed"], json!(true));
    }

    /// No proxy → the settings are byte-identical to the pre-task shape: the
    /// `allowedDomains` key is not even created.
    #[test]
    fn claude_settings_without_proxy_add_no_allowed_domains() {
        let v = claude_sandbox_settings("/tmp/conclave.sock", None, None);
        assert!(v["sandbox"]["network"].get("allowedDomains").is_none());
    }

    /// Re-merge with the proxy on must not duplicate the loopback entry, and a
    /// pre-existing foreign allowlist keeps its entries.
    #[test]
    fn claude_settings_proxy_domain_unions_and_stays_idempotent() {
        let sock = "/tmp/conclave.sock";
        let existing = json!({
            "sandbox": { "network": { "allowedDomains": ["api.github.com"] } }
        });
        let once = claude_sandbox_settings(sock, Some(18787), Some(existing));
        assert_eq!(
            once["sandbox"]["network"]["allowedDomains"],
            json!(["api.github.com", "127.0.0.1"])
        );
        let twice = claude_sandbox_settings(sock, Some(18787), Some(once.clone()));
        assert_eq!(once, twice);
    }

    /// Codex analogue: proxy on → the `[permissions.conclave]` profile also
    /// allowlists the loopback host in its network `domains` map (same map
    /// syntax as the proven `unix_sockets` override); proxy off → the override
    /// list is exactly the proven 4-entry recipe.
    #[test]
    fn codex_overrides_with_proxy_allow_loopback_domain() {
        let sock = "/tmp/conclave.sock";
        let ov = codex_socket_overrides(sock, Some(18787));
        assert_eq!(ov.len(), 5);
        assert!(ov.contains(
            &"permissions.conclave.network.domains={\"127.0.0.1\"=\"allow\"}".to_string()
        ));
        // The proven recipe entries are all still present, unchanged.
        for base in codex_socket_overrides(sock, None) {
            assert!(ov.contains(&base), "missing base override {base}");
        }
    }

    #[test]
    fn claude_settings_fresh_has_route_a_keys() {
        let sock = "/Users/x/Library/Application Support/Conclave/conclave.sock";
        let v = claude_sandbox_settings(sock, None, None);
        assert_eq!(v["sandbox"]["network"]["allowUnixSockets"], json!([sock]));
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
        let v = claude_sandbox_settings(sock, None, Some(existing));
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
        let once = claude_sandbox_settings(sock, None, None);
        let twice = claude_sandbox_settings(sock, None, Some(once.clone()));
        assert_eq!(once, twice);
    }

    #[test]
    fn claude_settings_repairs_wrong_typed_nodes() {
        let sock = "/tmp/conclave.sock";
        // sandbox present but the wrong type — must be replaced, not panic.
        let existing = json!({ "sandbox": "oops" });
        let v = claude_sandbox_settings(sock, None, Some(existing));
        assert_eq!(v["sandbox"]["network"]["allowUnixSockets"], json!([sock]));
        assert!(v["sandbox"].is_object());
    }

    #[test]
    fn claude_settings_path_lives_beside_socket() {
        let p = claude_settings_path("inst-123");
        assert!(
            p.ends_with("agent-settings/inst-123.json"),
            "path was {p:?}"
        );
        // sibling of the Conclave data dir, not the workspace
        assert!(p.to_string_lossy().contains("Conclave"));
    }

    #[test]
    fn non_object_existing_settings_start_fresh() {
        let sock = "/tmp/conclave.sock";
        let v: Value = claude_sandbox_settings(sock, None, Some(json!("not an object")));
        assert_eq!(v["sandbox"]["network"]["allowUnixSockets"], json!([sock]));
    }
}
