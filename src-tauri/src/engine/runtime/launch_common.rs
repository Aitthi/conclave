//! Launch-string and env helpers shared by the PTY spawn path
//! (`commands::instance::spawn`) and the one-shot print-mode runner
//! (`runtime::cli_oneshot`). Pure: no I/O except the Keychain read in
//! `agent_env_overrides`, which tests avoid by leaving `secret_env_keys` None.

use std::path::Path;

use crate::engine::repo::agent_definition::AgentDefRow;

/// Single-quote a value so the shell doesn't glob it; POSIX-escape embedded
/// quotes so the value can't break out of the launch command.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Claude Code names its 1M-context variants with a `[1m]` suffix on the model
/// id; every other window is the bare id.
pub fn effective_claude_model(model: &str, context_window: Option<&str>) -> String {
    if context_window == Some("1m") {
        format!("{model}[1m]")
    } else {
        model.to_string()
    }
}

/// Append the provider-specific reasoning-effort override.
///
/// Values are validated before persistence, but quoting remains mandatory: the
/// launch string crosses `zsh -c`, and this helper is also deliberately safe if
/// a malformed legacy row reaches it. Blank/absent values mean Auto and append
/// nothing. Callers append expert `custom_args` only after this helper.
pub fn append_cli_effort_override(launch: &mut String, cli_kind: &str, effort: Option<&str>) {
    let Some(effort) = effort.filter(|value| !value.trim().is_empty()) else {
        return;
    };
    let effort = effort.trim();
    match cli_kind {
        "claude-code" | "antigravity" => {
            launch.push_str(&format!(" --effort {}", shell_quote(effort)));
        }
        "codex" => {
            launch.push_str(&format!(
                " -c {}",
                shell_quote(&format!(r#"model_reasoning_effort="{effort}""#))
            ));
        }
        _ => {}
    }
}

/// Build Antigravity's complete inner-shell command in Conclave-owned order.
///
/// Typed fields are quoted and precede expert `custom_args`. `None`, blank
/// model/effort and the Default/Auto permission modes deliberately omit flags
/// so AGY retains its own defaults. This helper contains no rtk or sandbox
/// concepts: v1 has neither integration.
pub fn build_antigravity_launch(
    model: Option<&str>,
    effort: Option<&str>,
    permission_mode: Option<&str>,
    bootstrap: &str,
    custom_args: Option<&str>,
) -> String {
    let mut launch = String::from("agy");
    if let Some(model) = model.filter(|value| !value.trim().is_empty()) {
        launch.push_str(&format!(" --model {}", shell_quote(model.trim())));
    }
    append_cli_effort_override(&mut launch, "antigravity", effort);
    match permission_mode {
        Some("acceptEdits") => launch.push_str(" --mode 'accept-edits'"),
        Some("plan") => launch.push_str(" --mode 'plan'"),
        Some("bypassPermissions") => launch.push_str(" --dangerously-skip-permissions"),
        _ => {}
    }
    let bootstrap = format!("{bootstrap} Acknowledge these instructions, then wait for the user.");
    launch.push_str(&format!(
        " --prompt-interactive {}",
        shell_quote(&bootstrap)
    ));
    if let Some(extra) = custom_args.filter(|value| !value.trim().is_empty()) {
        launch.push(' ');
        launch.push_str(extra.trim());
    }
    launch
}

/// Non-secret env from the definition's `custom_env` JSON object, then secret
/// values fetched back from the Keychain by the names in `secret_env_keys`.
pub fn agent_env_overrides(def: &AgentDefRow) -> Vec<(String, String)> {
    let mut extra_env: Vec<(String, String)> = Vec::new();
    if let Some(text) = def.custom_env.as_deref() {
        if let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(text)
        {
            for (k, v) in map {
                if let Some(s) = v.as_str() {
                    extra_env.push((k, s.to_owned()));
                }
            }
        }
    }
    if let Some(text) = def.secret_env_keys.as_deref() {
        if let Ok(serde_json::Value::Array(names)) = serde_json::from_str::<serde_json::Value>(text)
        {
            for name in names.iter().filter_map(|n| n.as_str()) {
                let account = format!("agent_env:{}:{}", def.id, name);
                if let Ok(Some(val)) = crate::engine::secrets::get_key(&account) {
                    extra_env.push((name.to_owned(), val));
                }
            }
        }
    }
    extra_env
}

/// Prepend an ALREADY-RESOLVED `conclave` shim dir to the child shell's PATH.
///
/// The login+interactive shell sources its rc files BEFORE running the `-c`
/// command, so prepending the export here wins over whatever PATH those files
/// set. Best-effort: `None` (shim not staged beside the app) returns `launch`
/// unchanged.
///
/// Callers that already hold the resolved path use this; callers that don't use
/// [`prefix_conclave_path`], which resolves it first. Splitting the two keeps
/// `instance::spawn` at exactly ONE `ensure_conclave_shim()` call per spawn (it
/// resolves the path early for the briefing preamble as well), so the hoist is
/// a pure move with no extra filesystem work.
pub fn prefix_conclave_path_with(launch: String, bin: Option<&Path>) -> String {
    match bin {
        Some(bin) => format!(
            "export PATH={}:\"$PATH\"; {}",
            shell_quote(&bin.to_string_lossy()),
            launch
        ),
        None => launch,
    }
}

/// Resolve the bundled `conclave` shim dir and prepend it to the child shell's
/// PATH. Identity when the shim cannot be staged.
// Used by the one-shot print-mode runner (`runtime::cli_oneshot`);
// `instance::spawn` uses `prefix_conclave_path_with` because it already holds
// the resolved path.
pub fn prefix_conclave_path(launch: String) -> String {
    prefix_conclave_path_with(
        launch,
        crate::engine::agentctx::ensure_conclave_shim().as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `AgentDefRow` derives neither `Deserialize` nor `Default` (and
    /// `repo/agent_definition.rs` is out of this lane's boundary), so the row
    /// is built field-by-field here.
    fn def_with_env(custom_env: Option<&str>) -> AgentDefRow {
        AgentDefRow {
            id: "def-1".into(),
            name: "X".into(),
            role: None,
            role_id: None,
            r#type: "cli".into(),
            cli_kind: Some("claude-code".into()),
            color: None,
            default_level: None,
            provider_id: None,
            model: None,
            effort: None,
            harness_mode: "own".into(),
            share_blackboard: None,
            auto_submit_injected: None,
            allowed_senders: None,
            permission_mode: None,
            custom_args: None,
            custom_env: custom_env.map(str::to_owned),
            secret_env_keys: None,
            context_window: None,
            selected_builtin_skill_ids: None,
            rtk_enabled: None,
            created_at: "2026-09-04T00:00:00Z".into(),
        }
    }

    #[test]
    fn shell_quote_escapes_embedded_single_quote() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
        assert_eq!(shell_quote("plain"), "'plain'");
    }

    #[test]
    fn effective_model_adds_1m_suffix_only_for_1m() {
        assert_eq!(
            effective_claude_model("claude-opus-4-8", Some("1m")),
            "claude-opus-4-8[1m]"
        );
        assert_eq!(
            effective_claude_model("claude-opus-4-8", Some("200k")),
            "claude-opus-4-8"
        );
        assert_eq!(
            effective_claude_model("claude-opus-4-8", None),
            "claude-opus-4-8"
        );
    }

    #[test]
    fn first_class_cli_effort_overrides_are_quoted_and_auto_is_omitted() {
        for effort in ["low", "medium", "high"] {
            for kind in ["claude-code", "codex", "antigravity"] {
                let (mut launch, expected) = match kind {
                    "claude-code" => ("claude".to_owned(), format!("claude --effort '{effort}'")),
                    "codex" => (
                        "codex".to_owned(),
                        format!(r#"codex -c 'model_reasoning_effort="{effort}"'"#),
                    ),
                    _ => ("agy".to_owned(), format!("agy --effort '{effort}'")),
                };
                append_cli_effort_override(&mut launch, kind, Some(effort));
                assert_eq!(launch, expected);
                assert_eq!(
                    launch.matches("model_reasoning_effort").count(),
                    usize::from(kind == "codex")
                );

                launch.push_str(" --expert-custom-arg");
                let typed = if kind == "codex" {
                    launch.find("model_reasoning_effort").unwrap()
                } else {
                    launch.find("--effort").unwrap()
                };
                assert!(typed < launch.find("--expert-custom-arg").unwrap());
            }
        }

        for kind in ["claude-code", "codex", "antigravity", "custom"] {
            for effort in [None, Some(""), Some("   ")] {
                let mut launch = kind.to_owned();
                append_cli_effort_override(&mut launch, kind, effort);
                assert_eq!(launch, kind, "{kind} {effort:?}");
            }
        }

        let mut codex = "codex".to_owned();
        append_cli_effort_override(
            &mut codex,
            "codex",
            Some("high'; $(touch /tmp/conclave-effort-injection)"),
        );
        assert_eq!(
            codex,
            "codex -c 'model_reasoning_effort=\"high'\\''; $(touch /tmp/conclave-effort-injection)\"'"
        );
    }

    #[test]
    fn antigravity_launch_orders_typed_flags_bootstrap_then_custom_args() {
        let launch = build_antigravity_launch(
            Some("gemini 'pro'"),
            Some("high"),
            Some("acceptEdits"),
            "identity 'quoted'",
            Some("--expert value"),
        );
        assert_eq!(
            launch,
            "agy --model 'gemini '\\''pro'\\''' --effort 'high' --mode 'accept-edits' \
             --prompt-interactive 'identity '\\''quoted'\\'' Acknowledge these instructions, \
             then wait for the user.' --expert value"
        );
        assert!(launch.find("--model").unwrap() < launch.find("--prompt-interactive").unwrap());
        assert!(launch.find("--prompt-interactive").unwrap() < launch.find("--expert").unwrap());
    }

    #[test]
    fn antigravity_launch_covers_effort_and_permission_matrix() {
        for effort in [None, Some(""), Some("low"), Some("medium"), Some("high")] {
            let launch = build_antigravity_launch(None, effort, None, "boot", None);
            match effort.filter(|value| !value.is_empty()) {
                Some(value) => assert!(launch.contains(&format!("--effort '{value}'"))),
                None => assert!(!launch.contains("--effort")),
            }
        }

        for (mode, expected) in [
            (None, None),
            (Some("default"), None),
            (Some("auto"), None),
            (Some("acceptEdits"), Some("--mode 'accept-edits'")),
            (Some("plan"), Some("--mode 'plan'")),
            (
                Some("bypassPermissions"),
                Some("--dangerously-skip-permissions"),
            ),
        ] {
            let launch = build_antigravity_launch(Some(""), None, mode, "boot", None);
            assert!(!launch.contains("--model"));
            match expected {
                Some(flag) => assert!(launch.contains(flag), "{mode:?}: {launch}"),
                None => {
                    assert!(!launch.contains("--mode"), "{mode:?}: {launch}");
                    assert!(!launch.contains("--dangerously-skip-permissions"));
                }
            }
            assert!(!launch.contains("--sandbox"));
            assert!(!launch.contains("rtk"));
        }
    }

    #[test]
    fn env_overrides_reads_string_values_only() {
        let def = def_with_env(Some(r#"{"A":"1","B":2,"C":"x"}"#));
        let env = agent_env_overrides(&def);
        assert_eq!(
            env,
            vec![("A".into(), "1".into()), ("C".into(), "x".into())]
        );
    }

    #[test]
    fn env_overrides_empty_when_no_custom_env() {
        assert!(agent_env_overrides(&def_with_env(None)).is_empty());
    }

    #[test]
    fn prefix_conclave_path_with_is_identity_without_a_shim() {
        assert_eq!(
            prefix_conclave_path_with("claude".into(), None),
            "claude".to_string()
        );
        assert_eq!(
            prefix_conclave_path_with("claude".into(), Some(Path::new("/a/b"))),
            "export PATH='/a/b':\"$PATH\"; claude".to_string()
        );
    }
}
