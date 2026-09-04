//! One-shot, non-PTY invocation of a CLI harness in print mode
//! (`claude -p --json-schema …` / `codex exec --output-schema …`). Used by
//! `commands::draft` (spec D1). Launched through the user's login shell with the
//! same PATH prefix and env overrides as `instance::spawn`, but with no PTY, no
//! session row and no instance id. Tests never spawn a binary (`Mock`).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::io::AsyncWriteExt;

use super::launch_common::{prefix_conclave_path, shell_quote};
use crate::engine::commands::fusion::strip_code_fences;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliKind {
    ClaudeCode,
    Codex,
}

impl CliKind {
    /// Parse the `agent_definition.cli_kind` column. Chat/custom definitions
    /// store `NULL` and never reach here.
    pub fn parse(s: &str) -> Option<CliKind> {
        match s {
            "claude-code" => Some(CliKind::ClaudeCode),
            "codex" => Some(CliKind::Codex),
            _ => None,
        }
    }

    fn bin(self) -> &'static str {
        match self {
            CliKind::ClaudeCode => "claude",
            CliKind::Codex => "codex",
        }
    }
}

pub struct OneshotSpec {
    pub cli_kind: CliKind,
    /// Already passed through `launch_common::effective_claude_model` by the
    /// caller for claude-code; `None` lets the CLI pick its own default.
    pub model: Option<String>,
    /// Written to the child's stdin, which is then closed so the CLI sees EOF.
    pub prompt: String,
    pub json_schema: Value,
    pub extra_env: Vec<(String, String)>,
    pub cwd: PathBuf,
    pub timeout: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum OneshotError {
    #[error("the drafter did not answer in {0} s")]
    Timeout(u64),
    #[error("{cli} exited with code {code}: {stderr_tail}")]
    Exit {
        cli: &'static str,
        code: i32,
        stderr_tail: String,
    },
    #[error("the drafter reported an error: {0}")]
    Model(String),
    #[error("could not parse the drafter's output: {0}")]
    Parse(String),
    #[error("could not launch {0}")]
    Spawn(String),
}

/// `claude -p` in print mode with a JSON schema, no session row and no tools —
/// the drafter only has to answer, never to act.
pub fn claude_launch(model: Option<&str>, schema: &Value) -> String {
    let mut s = format!(
        "claude -p --output-format json --json-schema {} --no-session-persistence --tools ''",
        shell_quote(&schema.to_string())
    );
    if let Some(m) = model {
        s.push_str(&format!(" --model {}", shell_quote(m)));
    }
    s
}

/// `codex exec` reading the prompt from stdin (`-`), with the schema and the
/// last-message sink as files (codex takes neither inline).
pub fn codex_launch(model: Option<&str>, schema_path: &Path, out_path: &Path) -> String {
    let mut s = format!(
        "codex exec --json --ephemeral --skip-git-repo-check --output-schema {} -o {}",
        shell_quote(&schema_path.to_string_lossy()),
        shell_quote(&out_path.to_string_lossy())
    );
    if let Some(m) = model {
        s.push_str(&format!(" -m {}", shell_quote(m)));
    }
    s.push_str(" -"); // prompt on stdin
    s
}

/// The LAST line of stdout that parses as a JSON object. An interactive login
/// shell may print banners before the CLI's envelope (spec R1).
pub fn extract_last_json_object(stdout: &str) -> Option<Value> {
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .filter(|l| l.starts_with('{'))
        .find_map(|l| {
            serde_json::from_str::<Value>(l)
                .ok()
                .filter(Value::is_object)
        })
}

/// Pull the schema-shaped object out of claude's result envelope. Prefers
/// `structured_output`; falls back to parsing the `result` text (which repeats
/// the same JSON, sometimes fenced) so a harness build that omits the field
/// still works.
pub fn claude_structured_result(envelope: &Value) -> Result<Value, OneshotError> {
    let is_error = envelope["is_error"].as_bool().unwrap_or(false);
    let subtype = envelope["subtype"].as_str().unwrap_or("");
    let result_text = envelope["result"].as_str().unwrap_or("").to_string();
    if is_error || subtype != "success" {
        return Err(OneshotError::Model(if result_text.is_empty() {
            subtype.to_string()
        } else {
            result_text
        }));
    }
    if let Some(v) = envelope.get("structured_output").filter(|v| v.is_object()) {
        return Ok(v.clone());
    }
    serde_json::from_str::<Value>(strip_code_fences(&result_text)).map_err(|e| {
        OneshotError::Parse(format!(
            "{e}; result text: {}",
            result_text.chars().take(300).collect::<String>()
        ))
    })
}

pub fn parse_codex_last_message(text: &str) -> Result<Value, OneshotError> {
    serde_json::from_str::<Value>(strip_code_fences(text)).map_err(|e| {
        OneshotError::Parse(format!(
            "{e}; last message: {}",
            text.chars().take(300).collect::<String>()
        ))
    })
}

pub enum Oneshot {
    Live,
    #[cfg(test)]
    Mock(Result<Value, String>),
}

impl Oneshot {
    pub async fn run(&self, spec: &OneshotSpec) -> Result<Value, OneshotError> {
        match self {
            Oneshot::Live => run_live(spec).await,
            #[cfg(test)]
            Oneshot::Mock(r) => {
                let _ = spec;
                r.clone().map_err(OneshotError::Model)
            }
        }
    }
}

async fn run_live(spec: &OneshotSpec) -> Result<Value, OneshotError> {
    // Temp dir outlives the child: codex reads the schema from it and writes
    // its last message into it.
    let tmp = tempfile::tempdir().map_err(|e| OneshotError::Spawn(e.to_string()))?;
    let schema_path = tmp.path().join("schema.json");
    let out_path = tmp.path().join("last.json");
    let launch = match spec.cli_kind {
        CliKind::ClaudeCode => claude_launch(spec.model.as_deref(), &spec.json_schema),
        CliKind::Codex => {
            std::fs::write(&schema_path, spec.json_schema.to_string())
                .map_err(|e| OneshotError::Spawn(e.to_string()))?;
            codex_launch(spec.model.as_deref(), &schema_path, &out_path)
        }
    };
    let launch = prefix_conclave_path(launch);
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());

    let mut cmd = tokio::process::Command::new(&shell);
    cmd.args(["-l", "-i", "-c", &launch])
        .current_dir(&spec.cwd)
        .env("CONCLAVE_DRAFT", "1")
        .envs(spec.extra_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| OneshotError::Spawn(format!("{shell} -c {launch}: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(spec.prompt.as_bytes())
            .await
            .map_err(|e| OneshotError::Spawn(e.to_string()))?;
        // Drop closes stdin so the CLI sees EOF and starts.
    }
    let output = match tokio::time::timeout(spec.timeout, child.wait_with_output()).await {
        Ok(r) => r.map_err(|e| OneshotError::Spawn(e.to_string()))?,
        Err(_) => return Err(OneshotError::Timeout(spec.timeout.as_secs())),
    };
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        return Err(OneshotError::Exit {
            cli: spec.cli_kind.bin(),
            code: output.status.code().unwrap_or(-1),
            stderr_tail: tail_chars(&stderr, 2048),
        });
    }
    match spec.cli_kind {
        CliKind::ClaudeCode => {
            let envelope = extract_last_json_object(&stdout).ok_or_else(|| {
                OneshotError::Parse(format!(
                    "no JSON envelope on stdout; tail: {}",
                    tail_chars(&stdout, 300)
                ))
            })?;
            claude_structured_result(&envelope)
        }
        CliKind::Codex => {
            let text = std::fs::read_to_string(&out_path)
                .map_err(|e| OneshotError::Parse(format!("codex wrote no last message: {e}")))?;
            parse_codex_last_message(&text)
        }
    }
}

/// Last `n` CHARS of `s` — byte slicing would panic mid-UTF-8 on a stderr tail
/// that happens to cut a multi-byte sequence.
fn tail_chars(s: &str, n: usize) -> String {
    let total = s.chars().count();
    s.chars().skip(total.saturating_sub(n)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_launch_has_print_schema_no_session_and_model() {
        let s = claude_launch(Some("claude-sonnet-5"), &json!({"type":"object"}));
        assert_eq!(
            s,
            "claude -p --output-format json --json-schema '{\"type\":\"object\"}' --no-session-persistence --tools '' --model 'claude-sonnet-5'"
        );
    }

    #[test]
    fn claude_launch_omits_model_when_none() {
        let s = claude_launch(None, &json!({}));
        assert!(s.ends_with("--tools ''"));
        assert!(!s.contains("--model"));
    }

    #[test]
    fn codex_launch_uses_exec_json_schema_and_last_message_file() {
        let s = codex_launch(
            Some("gpt-5.5"),
            std::path::Path::new("/t/s.json"),
            std::path::Path::new("/t/o.json"),
        );
        assert_eq!(
            s,
            "codex exec --json --ephemeral --skip-git-repo-check --output-schema '/t/s.json' -o '/t/o.json' -m 'gpt-5.5' -"
        );
    }

    #[test]
    fn extract_last_json_object_skips_shell_banner_lines() {
        let out = "Welcome to zsh\n{\"type\":\"system\"}\nnoise\n{\"type\":\"result\",\"n\":1}\n";
        let v = extract_last_json_object(out).unwrap();
        assert_eq!(v["type"], "result");
        assert_eq!(v["n"], 1);
    }

    #[test]
    fn extract_last_json_object_none_when_no_json() {
        assert!(extract_last_json_object("nothing here").is_none());
    }

    #[test]
    fn claude_structured_result_returns_structured_output_on_success() {
        let env = json!({"type":"result","subtype":"success","is_error":false,"structured_output":{"a":1},"result":"{\"a\":1}"});
        assert_eq!(claude_structured_result(&env).unwrap(), json!({"a":1}));
    }

    #[test]
    fn claude_structured_result_falls_back_to_result_text() {
        let env = json!({"type":"result","subtype":"success","is_error":false,"result":"```json\n{\"a\":2}\n```"});
        assert_eq!(claude_structured_result(&env).unwrap(), json!({"a":2}));
    }

    #[test]
    fn claude_structured_result_errors_on_is_error() {
        let env = json!({"type":"result","subtype":"error_during_execution","is_error":true,"result":"boom"});
        match claude_structured_result(&env) {
            Err(OneshotError::Model(m)) => assert!(m.contains("boom")),
            other => panic!("expected Model error, got {other:?}"),
        }
    }

    #[test]
    fn parse_codex_last_message_strips_fences() {
        assert_eq!(
            parse_codex_last_message("```json\n{\"k\":true}\n```").unwrap(),
            json!({"k":true})
        );
    }

    #[test]
    fn tail_chars_never_splits_a_multibyte_char() {
        assert_eq!(tail_chars("héllo", 3), "llo");
        assert_eq!(tail_chars("hé", 99), "hé");
    }

    #[tokio::test]
    async fn mock_runner_returns_canned_value() {
        let spec = OneshotSpec {
            cli_kind: CliKind::ClaudeCode,
            model: None,
            prompt: "p".into(),
            json_schema: json!({}),
            extra_env: vec![],
            cwd: std::env::temp_dir(),
            timeout: std::time::Duration::from_secs(1),
        };
        let v = Oneshot::Mock(Ok(json!({"ok":1}))).run(&spec).await.unwrap();
        assert_eq!(v, json!({"ok":1}));
        let e = Oneshot::Mock(Err("nope".into()))
            .run(&spec)
            .await
            .unwrap_err();
        assert!(matches!(e, OneshotError::Model(_)));
    }
}
