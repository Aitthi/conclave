//! One-shot, non-PTY invocation of a CLI harness in print mode
//! (`claude -p --json-schema …` / `codex exec …`). Used by
//! `commands::draft` (spec D1). Launched through the user's login shell with the
//! same PATH prefix and env overrides as `instance::spawn`, but with no PTY, no
//! session row and no instance id. Tests never spawn a binary (`Mock`).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::io::AsyncWriteExt;

#[cfg(test)]
use super::launch_common::prefix_conclave_path_with;
use super::launch_common::{prefix_conclave_path, shell_quote};
use super::usage::{counter, MeasuredUsage};
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

/// These `Display` strings are shown VERBATIM as the error headline in the
/// AgentDrafter panel (Arta's design acceptance, 2026-09-04), so they read as
/// sentences and start with a capital. `Exit` is the exception on purpose: it
/// opens with the binary's own name (`claude` / `codex`).
#[derive(Debug, thiserror::Error)]
pub enum OneshotError {
    #[error("The drafter did not answer in {0} s")]
    Timeout(u64),
    #[error("{cli} exited with code {code}: {stderr_tail}")]
    Exit {
        cli: &'static str,
        code: i32,
        stderr_tail: String,
    },
    #[error("The drafter reported an error: {0}")]
    Model(String),
    #[error("Could not parse the drafter's output: {0}")]
    Parse(String),
    #[error("Could not launch {0}")]
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

/// `codex exec` reading the prompt from stdin (`-`), writing its last message
/// to a file (codex takes no inline sink).
///
/// Deliberately NO `--output-schema`: it routes to OpenAI's strict Structured
/// Outputs, which rejects any schema whose `required` does not list every
/// property — our draft schema has optional fields by design, so codex answered
/// HTTP 400 `invalid_json_schema` before the model ran (spec R2, verified in the
/// Task A5 probe; ruling 2026-09-04). The schema instead reaches the model as
/// text inside the prompt (`draft_prompt::build_prompt`), and
/// `commands::draft::validate_draft` is what actually guarantees the shape.
pub fn codex_launch(model: Option<&str>, out_path: &Path) -> String {
    let mut s = format!(
        "codex exec --json --ephemeral --skip-git-repo-check -o {}",
        shell_quote(&out_path.to_string_lossy())
    );
    if let Some(m) = model {
        s.push_str(&format!(" -m {}", shell_quote(m)));
    }
    s.push_str(" -"); // prompt on stdin
    s
}

/// `exec` the CLI so it REPLACES the login shell instead of running as its
/// child. The one-shot shell exists only to source rc files and resolve PATH —
/// with `exec` there is no shell left parked around the CLI, so the process the
/// timeout's `kill_on_drop` reaches IS the CLI, not a wrapper that would leave
/// an orphaned `claude`/`codex` behind on expiry. (The PTY spawn path in
/// `commands::instance` deliberately does NOT do this — its shell stays alive
/// as the terminal.)
fn exec_launch(launch: String) -> String {
    format!("exec {launch}")
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

// ── Measured outcome ─────────────────────────────────────────────────────────

/// Token usage the CLI itself reported for the WHOLE invocation — the shared
/// normalized shape, see [`MeasuredUsage`].
pub type OneshotUsage = MeasuredUsage;

/// What a successful one-shot returns to a measured caller: the schema-shaped
/// answer plus the metadata the usage collector needs. A failed, timed-out or
/// unparsable run never produces one of these — that is what makes "an outcome
/// exists" equal to "completed activity".
#[derive(Debug, Clone, PartialEq)]
pub struct OneshotOutcome {
    pub value: Value,
    /// Generated BEFORE the child is spawned, so the same invocation can never
    /// be recorded twice and two invocations can never share an identity.
    pub invocation_id: String,
    /// The model the CLI was asked for (`OneshotSpec::model`) — a SELECTED
    /// identity, never proof of what served the answer.
    pub requested_model: Option<String>,
    /// Only when the CLI itself named exactly one serving model.
    pub served_model: Option<String>,
    /// The CLI's own session/thread id, kept as source identity for diagnostics.
    pub source_session_id: Option<String>,
    pub usage: OneshotUsage,
    /// When the CLI finished — the runner's clock, since the envelope carries
    /// no timestamp of its own.
    pub completed_at: chrono::DateTime<chrono::Utc>,
}

pub enum Oneshot {
    Live,
    #[cfg(test)]
    Mock(Result<Value, String>),
    /// A canned measured outcome, for tests of the collector path.
    #[cfg(test)]
    MockMeasured(Result<OneshotOutcome, String>),
}

impl Oneshot {
    /// Text-only compatibility entry point: the answer without its metadata.
    pub async fn run(&self, spec: &OneshotSpec) -> Result<Value, OneshotError> {
        self.run_measured(spec).await.map(|outcome| outcome.value)
    }

    /// The measured entry point: answer plus invocation identity and the usage
    /// the CLI reported. See [`OneshotOutcome`].
    pub async fn run_measured(&self, spec: &OneshotSpec) -> Result<OneshotOutcome, OneshotError> {
        match self {
            Oneshot::Live => run_live(spec).await,
            #[cfg(test)]
            Oneshot::Mock(r) => r
                .clone()
                .map(|value| OneshotOutcome {
                    value,
                    invocation_id: uuid::Uuid::new_v4().to_string(),
                    requested_model: spec.model.clone(),
                    served_model: None,
                    source_session_id: None,
                    usage: OneshotUsage::default(),
                    completed_at: chrono::Utc::now(),
                })
                .map_err(OneshotError::Model),
            #[cfg(test)]
            Oneshot::MockMeasured(r) => {
                let _ = spec;
                r.clone().map_err(OneshotError::Model)
            }
        }
    }
}

/// Usage from `claude -p --output-format json`'s terminal `result` envelope.
///
/// Verified shape (recorded envelope /tmp/draft-probe/claude.out, Claude Code
/// 2.1.260, 2026-09-04): `usage.{input_tokens, cache_creation_input_tokens,
/// cache_read_input_tokens, output_tokens, output_tokens_details.thinking_tokens}`
/// plus `modelUsage.{<model id>: {…, canonicalModel}}` and `session_id`. The
/// envelope is the INVOCATION aggregate (`num_turns` may exceed 1), which is
/// exactly what a draft event records — never a per-response proof.
///
/// Cache-inclusive input is `input + cache_creation + cache_read` and requires
/// all three components; a missing one makes the input unknown rather than a
/// partial sum posing as a total. The served model is taken only when
/// `modelUsage` names exactly ONE model — a mixed set (fallback, subagents)
/// leaves it unknown, per the multi-attempt rule.
pub fn claude_envelope_usage(envelope: &Value) -> (OneshotUsage, Option<String>, Option<String>) {
    let usage = &envelope["usage"];
    let uncached = counter(&usage["input_tokens"]);
    let cache_write = counter(&usage["cache_creation_input_tokens"]);
    let cache_read = counter(&usage["cache_read_input_tokens"]);
    let input_tokens = match (uncached, cache_write, cache_read) {
        (Some(a), Some(b), Some(c)) => a.checked_add(b).and_then(|ab| ab.checked_add(c)),
        _ => None,
    };
    let normalized = OneshotUsage {
        input_tokens,
        output_tokens: counter(&usage["output_tokens"]),
        cache_read_input_tokens: cache_read,
        cache_write_input_tokens: cache_write,
        reasoning_output_tokens: counter(&usage["output_tokens_details"]["thinking_tokens"]),
    };
    let served_model = envelope["modelUsage"].as_object().and_then(|models| {
        if models.len() != 1 {
            return None;
        }
        let (key, detail) = models.iter().next()?;
        Some(
            detail["canonicalModel"]
                .as_str()
                .filter(|m| !m.is_empty())
                .unwrap_or(key)
                .to_string(),
        )
    });
    let session_id = envelope["session_id"].as_str().map(str::to_string);
    (normalized, served_model, session_id)
}

/// Usage from `codex exec --json`'s stdout event stream.
///
/// Verified shape (recorded stream /tmp/draft-probe/codex2.out, codex-cli
/// 0.153.2, 2026-09-04): one `{"type":"thread.started","thread_id":…}`, then
/// `{"type":"turn.completed","usage":{input_tokens, cached_input_tokens,
/// cache_write_input_tokens, output_tokens, reasoning_output_tokens}}` — or
/// `turn.failed`. Codex `input_tokens` already includes cached input and
/// `output_tokens` already includes reasoning (evidence doc, Codex component
/// relations), so they map straight through; the subsets are kept as subsets.
///
/// Exactly one `turn.completed` is required: `exec` runs one turn, and summing
/// several without proof of their relation would be a fabricated total.
pub fn codex_turn_usage(stdout: &str) -> (OneshotUsage, Option<String>) {
    let mut thread_id = None;
    let mut completed: Vec<&Value> = Vec::new();
    let records: Vec<Value> = stdout
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('{'))
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect();
    for record in &records {
        match record["type"].as_str() {
            Some("thread.started") => {
                thread_id = record["thread_id"].as_str().map(str::to_string);
            }
            Some("turn.completed") => completed.push(&record["usage"]),
            _ => {}
        }
    }
    let usage = match completed.as_slice() {
        [usage] if usage.is_object() => OneshotUsage {
            input_tokens: counter(&usage["input_tokens"]),
            output_tokens: counter(&usage["output_tokens"]),
            cache_read_input_tokens: counter(&usage["cached_input_tokens"]),
            cache_write_input_tokens: counter(&usage["cache_write_input_tokens"]),
            reasoning_output_tokens: counter(&usage["reasoning_output_tokens"]),
        },
        _ => OneshotUsage::default(),
    };
    (usage, thread_id)
}

async fn run_live(spec: &OneshotSpec) -> Result<OneshotOutcome, OneshotError> {
    // Identity first: whatever happens below, this run is this id.
    let invocation_id = uuid::Uuid::new_v4().to_string();
    // Temp dir outlives the child: codex writes its last message into it.
    let tmp = tempfile::tempdir().map_err(|e| OneshotError::Spawn(e.to_string()))?;
    let out_path = tmp.path().join("last.json");
    let launch = match spec.cli_kind {
        CliKind::ClaudeCode => claude_launch(spec.model.as_deref(), &spec.json_schema),
        CliKind::Codex => codex_launch(spec.model.as_deref(), &out_path),
    };
    // `exec` first, then the PATH export in front of it: the export must run in
    // the shell, so it has to sit BEFORE the exec, never inside it.
    let launch = prefix_conclave_path(exec_launch(launch));
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
    let completed_at = chrono::Utc::now();
    let (value, usage, served_model, source_session_id) = match spec.cli_kind {
        CliKind::ClaudeCode => {
            let envelope = extract_last_json_object(&stdout).ok_or_else(|| {
                OneshotError::Parse(format!(
                    "no JSON envelope on stdout; tail: {}",
                    tail_chars(&stdout, 300)
                ))
            })?;
            let value = claude_structured_result(&envelope)?;
            let (usage, served, session) = claude_envelope_usage(&envelope);
            (value, usage, served, session)
        }
        CliKind::Codex => {
            let text = std::fs::read_to_string(&out_path)
                .map_err(|e| OneshotError::Parse(format!("codex wrote no last message: {e}")))?;
            let value = parse_codex_last_message(&text)?;
            let (usage, thread) = codex_turn_usage(&stdout);
            (value, usage, None, thread)
        }
    };
    Ok(OneshotOutcome {
        value,
        invocation_id,
        requested_model: spec.model.clone(),
        served_model,
        source_session_id,
        usage,
        completed_at,
    })
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
    fn codex_launch_uses_exec_json_and_last_message_file_without_output_schema() {
        // R2 fired: OpenAI strict structured outputs reject a schema with optional
        // properties (HTTP 400), so codex gets the schema in the prompt only.
        let s = codex_launch(Some("gpt-5.5"), std::path::Path::new("/t/o.json"));
        assert_eq!(
            s,
            "codex exec --json --ephemeral --skip-git-repo-check -o '/t/o.json' -m 'gpt-5.5' -"
        );
        assert!(!s.contains("--output-schema"));
    }

    #[test]
    fn exec_launch_prefixes_the_one_shot_command() {
        let claude = exec_launch(claude_launch(Some("claude-sonnet-5"), &json!({})));
        assert!(claude.starts_with("exec "), "got {claude}");
        assert!(claude.starts_with("exec claude -p "), "got {claude}");

        let codex = exec_launch(codex_launch(None, std::path::Path::new("/t/o.json")));
        assert!(codex.starts_with("exec "), "got {codex}");
        assert!(codex.starts_with("exec codex exec "), "got {codex}");
    }

    #[test]
    fn the_path_export_stays_in_front_of_the_exec() {
        // Ordering guard for the composition `run_live` performs: `exec` must
        // replace the shell only AFTER the shell has applied the PATH export,
        // so the export can never end up inside the exec'd command.
        let composed = prefix_conclave_path_with(
            exec_launch(claude_launch(None, &json!({}))),
            Some(std::path::Path::new("/a/b")),
        );
        assert!(
            composed.starts_with("export PATH='/a/b':\"$PATH\"; exec claude -p "),
            "got {composed}"
        );
        // With no shim there is nothing in front of it at all.
        let bare = prefix_conclave_path_with(exec_launch(claude_launch(None, &json!({}))), None);
        assert!(bare.starts_with("exec "), "got {bare}");
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

    // ── Measured usage parsers ───────────────────────────────────────────

    /// The REAL envelope Claude Code 2.1.260 printed in the Task A5 probe
    /// (/tmp/draft-probe/claude.out), reduced to its measurement fields with
    /// every number verbatim.
    fn recorded_claude_envelope() -> Value {
        json!({
            "type": "result", "subtype": "success", "is_error": false,
            "num_turns": 2, "stop_reason": "tool_use", "terminal_reason": "completed",
            "session_id": "f55f7292-64cd-408a-b190-477bb6c6898c",
            "usage": {
                "input_tokens": 2,
                "cache_creation_input_tokens": 31061,
                "cache_read_input_tokens": 0,
                "output_tokens": 3297,
                "output_tokens_details": { "thinking_tokens": 2489 },
                "iterations": [{ "input_tokens": 2, "output_tokens": 3297, "type": "message" }]
            },
            "modelUsage": {
                "claude-sonnet-5": {
                    "inputTokens": 2, "outputTokens": 3297,
                    "cacheReadInputTokens": 0, "cacheCreationInputTokens": 31061,
                    "canonicalModel": "claude-sonnet-5", "provider": "firstParty"
                }
            },
            "structured_output": { "agents": [], "positions": [], "notes": "" },
            "result": "…"
        })
    }

    #[test]
    fn claude_envelope_usage_is_cache_inclusive_and_names_the_single_served_model() {
        let (usage, served, session) = claude_envelope_usage(&recorded_claude_envelope());
        assert_eq!(
            usage.input_tokens,
            Some(2 + 31061),
            "uncached + cache-create + cache-read"
        );
        assert_eq!(usage.output_tokens, Some(3297));
        assert_eq!(usage.cache_read_input_tokens, Some(0));
        assert_eq!(usage.cache_write_input_tokens, Some(31061));
        assert_eq!(usage.reasoning_output_tokens, Some(2489));
        assert_eq!(served.as_deref(), Some("claude-sonnet-5"));
        assert_eq!(
            session.as_deref(),
            Some("f55f7292-64cd-408a-b190-477bb6c6898c")
        );
    }

    /// A missing cache component makes the INPUT unknown — a two-of-three sum
    /// would be a partial number posing as the cache-inclusive total.
    #[test]
    fn claude_envelope_usage_refuses_partial_input_components() {
        let mut envelope = recorded_claude_envelope();
        envelope["usage"]
            .as_object_mut()
            .unwrap()
            .remove("cache_read_input_tokens");
        let (usage, _, _) = claude_envelope_usage(&envelope);
        assert_eq!(usage.input_tokens, None);
        assert_eq!(usage.output_tokens, Some(3297), "output is independent");

        let mut negative = recorded_claude_envelope();
        negative["usage"]["output_tokens"] = json!(-1);
        assert_eq!(claude_envelope_usage(&negative).0.output_tokens, None);

        let mut fractional = recorded_claude_envelope();
        fractional["usage"]["output_tokens"] = json!(12.5);
        assert_eq!(claude_envelope_usage(&fractional).0.output_tokens, None);

        let (usage, served, session) = claude_envelope_usage(&json!({"type": "result"}));
        assert_eq!(usage, OneshotUsage::default());
        assert_eq!(served, None);
        assert_eq!(session, None);
    }

    /// Two serving models (fallback / subagents) cannot be attributed to one.
    #[test]
    fn claude_envelope_usage_leaves_a_mixed_model_set_unknown() {
        let mut envelope = recorded_claude_envelope();
        envelope["modelUsage"]["claude-haiku-4-5"] = json!({ "inputTokens": 1 });
        assert_eq!(claude_envelope_usage(&envelope).1, None);

        let mut no_canonical = recorded_claude_envelope();
        no_canonical["modelUsage"]["claude-sonnet-5"]
            .as_object_mut()
            .unwrap()
            .remove("canonicalModel");
        assert_eq!(
            claude_envelope_usage(&no_canonical).1.as_deref(),
            Some("claude-sonnet-5"),
            "the key itself is the model id when canonicalModel is absent"
        );
    }

    /// The REAL stream codex-cli 0.153.2 printed in the Task A5 probe
    /// (/tmp/draft-probe/codex2.out); numbers verbatim, prose dropped.
    const RECORDED_CODEX_STDOUT: &str = r#"{"type":"thread.started","thread_id":"01a06cbd-aa59-7d02-b5bc-18516f8b4e0f"}
{"type":"turn.started"}
{"type":"item.completed","item":{"id":"item_0","type":"error","message":"…"}}
{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"…"}}
{"type":"turn.completed","usage":{"input_tokens":19357,"cached_input_tokens":5504,"cache_write_input_tokens":0,"output_tokens":848,"reasoning_output_tokens":516}}
"#;

    #[test]
    fn codex_turn_usage_maps_the_completed_turn_and_thread() {
        let (usage, thread) = codex_turn_usage(RECORDED_CODEX_STDOUT);
        assert_eq!(usage.input_tokens, Some(19357), "already cache-inclusive");
        assert_eq!(usage.cache_read_input_tokens, Some(5504));
        assert_eq!(usage.cache_write_input_tokens, Some(0));
        assert_eq!(
            usage.output_tokens,
            Some(848),
            "already reasoning-inclusive"
        );
        assert_eq!(usage.reasoning_output_tokens, Some(516));
        assert_eq!(
            thread.as_deref(),
            Some("01a06cbd-aa59-7d02-b5bc-18516f8b4e0f")
        );
    }

    /// A failed turn, a banner-only stdout, or two completed turns give no
    /// usage — the thread id is still identity, the numbers are not proof.
    #[test]
    fn codex_turn_usage_is_unknown_without_exactly_one_completed_turn() {
        let failed = r#"{"type":"thread.started","thread_id":"t-1"}
{"type":"turn.started"}
{"type":"turn.failed","error":{"message":"…"}}
"#;
        let (usage, thread) = codex_turn_usage(failed);
        assert_eq!(usage, OneshotUsage::default());
        assert_eq!(thread.as_deref(), Some("t-1"));

        let doubled = format!(
            "{RECORDED_CODEX_STDOUT}{{\"type\":\"turn.completed\",\"usage\":{{\"input_tokens\":1,\"output_tokens\":1}}}}\n"
        );
        assert_eq!(codex_turn_usage(&doubled).0, OneshotUsage::default());

        assert_eq!(
            codex_turn_usage("Welcome to zsh\nnot json\n"),
            (OneshotUsage::default(), None)
        );
    }

    /// The text-only entry point still returns the bare answer, and a mocked
    /// measured run reaches the caller with its identity intact.
    #[tokio::test]
    async fn run_is_the_text_only_view_of_run_measured() {
        let spec = OneshotSpec {
            cli_kind: CliKind::ClaudeCode,
            model: Some("claude-sonnet-5".into()),
            prompt: String::new(),
            json_schema: json!({}),
            extra_env: vec![],
            cwd: std::env::temp_dir(),
            timeout: Duration::from_secs(1),
        };
        let mock = Oneshot::Mock(Ok(json!({"ok": true})));
        assert_eq!(mock.run(&spec).await.unwrap(), json!({"ok": true}));
        let outcome = mock.run_measured(&spec).await.unwrap();
        assert_eq!(outcome.requested_model.as_deref(), Some("claude-sonnet-5"));
        assert!(!outcome.invocation_id.is_empty());
        assert_eq!(outcome.usage, OneshotUsage::default());
    }
}
