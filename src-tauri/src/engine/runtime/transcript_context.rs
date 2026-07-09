//! Transcript-backed context meter for CLI agents.
//!
//! This module is deliberately narrow:
//! - It discovers candidate Claude Code and Codex transcript files.
//! - It matches them to a specific agent instance using workspace cwd plus the
//!   bootstrap owner marker declaring the agent's own instance id.
//! - It returns only usage numbers, a limit, an observation timestamp, and the
//!   source kind.
//!
//! Raw transcript text stays inside this module.

use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// A discovered transcript source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptSourceKind {
    ClaudeCode,
    Codex,
}

impl fmt::Display for TranscriptSourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TranscriptSourceKind::ClaudeCode => write!(f, "claude-code"),
            TranscriptSourceKind::Codex => write!(f, "codex"),
        }
    }
}

/// Config for [`TranscriptContextReader`].
#[derive(Debug, Clone)]
pub struct TranscriptContextConfig {
    pub claude_projects_root: PathBuf,
    pub codex_sessions_root: PathBuf,
    pub fallback_limit: i64,
}

impl TranscriptContextConfig {
    /// Build a config from the standard transcript roots.
    pub fn default_with_limit(fallback_limit: i64) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self {
            claude_projects_root: home.join(".claude").join("projects"),
            codex_sessions_root: home.join(".codex").join("sessions"),
            fallback_limit,
        }
    }
}

/// One transcript-backed context reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptContextReading {
    pub tokens: i64,
    pub limit: i64,
    pub observed_at: String,
    pub source_kind: TranscriptSourceKind,
}

#[derive(Debug, Clone)]
pub struct TranscriptContextReader {
    config: TranscriptContextConfig,
}

impl TranscriptContextReader {
    pub fn new(config: TranscriptContextConfig) -> Self {
        Self { config }
    }

    pub fn poll(
        &self,
        instance_id: &str,
        workspace_folder: &Path,
        cli_kind: &str,
        started_at: DateTime<Utc>,
    ) -> Option<TranscriptContextReading> {
        match cli_kind {
            "claude-code" => self.poll_claude(instance_id, workspace_folder, started_at),
            "codex" => self.poll_codex(instance_id, workspace_folder, started_at),
            _ => None,
        }
    }

    fn poll_claude(
        &self,
        instance_id: &str,
        workspace_folder: &Path,
        started_at: DateTime<Utc>,
    ) -> Option<TranscriptContextReading> {
        let mut best: Option<ScannedReading> = None;
        for path in collect_jsonl_files(&self.config.claude_projects_root) {
            let Some(reading) = scan_claude_file(
                &path,
                instance_id,
                workspace_folder,
                started_at,
                self.config.fallback_limit,
            ) else {
                continue;
            };
            best = choose_newer(best, Some(reading));
        }
        best.map(ScannedReading::into_reading)
    }

    fn poll_codex(
        &self,
        instance_id: &str,
        workspace_folder: &Path,
        started_at: DateTime<Utc>,
    ) -> Option<TranscriptContextReading> {
        let mut best: Option<ScannedReading> = None;
        for path in collect_jsonl_files(&self.config.codex_sessions_root) {
            let Some(reading) = scan_codex_file(
                &path,
                instance_id,
                workspace_folder,
                started_at,
                self.config.fallback_limit,
            ) else {
                continue;
            };
            best = choose_newer(best, Some(reading));
        }
        best.map(ScannedReading::into_reading)
    }
}

#[derive(Debug, Clone)]
struct ScannedReading {
    tokens: i64,
    limit: i64,
    observed_at: DateTime<Utc>,
    source_kind: TranscriptSourceKind,
}

impl ScannedReading {
    fn into_reading(self) -> TranscriptContextReading {
        TranscriptContextReading {
            tokens: self.tokens,
            limit: self.limit,
            observed_at: self.observed_at.to_rfc3339(),
            source_kind: self.source_kind,
        }
    }
}

fn choose_newer(
    current: Option<ScannedReading>,
    next: Option<ScannedReading>,
) -> Option<ScannedReading> {
    match (current, next) {
        (None, None) => None,
        (Some(cur), None) => Some(cur),
        (None, Some(next)) => Some(next),
        (Some(cur), Some(next)) => {
            if next.observed_at >= cur.observed_at {
                Some(next)
            } else {
                Some(cur)
            }
        }
    }
}

fn collect_jsonl_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_jsonl_files_inner(root, &mut out);
    out
}

fn collect_jsonl_files_inner(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files_inner(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            out.push(path);
        }
    }
}

fn scan_claude_file(
    path: &Path,
    instance_id: &str,
    workspace_folder: &Path,
    started_at: DateTime<Utc>,
    fallback_limit: i64,
) -> Option<ScannedReading> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    let workspace_text = workspace_folder.to_string_lossy();
    let mut saw_workspace = false;
    let mut saw_instance = false;
    let mut latest_by_key: HashMap<String, (usize, i64)> = HashMap::new();
    let mut line_no = 0usize;

    for line in reader.lines().map_while(Result::ok) {
        line_no += 1;
        if !saw_workspace && line.contains(workspace_text.as_ref()) {
            saw_workspace = true;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if !saw_instance && claude_value_declares_owner(&value, instance_id) {
            saw_instance = true;
        }
        if value.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(message) = value.get("message") else {
            continue;
        };
        let Some(usage) = message.get("usage") else {
            continue;
        };
        let tokens = sum_claude_usage(usage);
        let key = value
            .get("requestId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| message.get("id").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_else(|| format!("line:{line_no}"));
        latest_by_key.insert(key, (line_no, tokens));
    }

    if !saw_workspace || !saw_instance || latest_by_key.is_empty() {
        return None;
    }

    let Some((_, tokens)) = latest_by_key.into_values().max_by_key(|(line_no, _)| *line_no) else {
        return None;
    };
    let observed_at = file_modified_at(path)?;
    if observed_at < started_at {
        return None;
    }
    Some(ScannedReading {
        tokens,
        limit: fallback_limit,
        observed_at,
        source_kind: TranscriptSourceKind::ClaudeCode,
    })
}

fn scan_codex_file(
    path: &Path,
    instance_id: &str,
    workspace_folder: &Path,
    started_at: DateTime<Utc>,
    fallback_limit: i64,
) -> Option<ScannedReading> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    let workspace_text = workspace_folder.to_string_lossy();
    let mut saw_workspace = false;
    let mut saw_instance = false;
    let mut best: Option<ScannedReading> = None;

    for line in reader.lines().map_while(Result::ok) {
        if !saw_workspace && line.contains(workspace_text.as_ref()) {
            saw_workspace = true;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if !saw_instance && codex_value_declares_owner(&value, instance_id) {
            saw_instance = true;
        }
        if value.get("type").and_then(Value::as_str) != Some("event_msg") {
            continue;
        }
        if value
            .pointer("/payload/type")
            .and_then(Value::as_str)
            != Some("token_count")
        {
            continue;
        }

        if let Some(cwd) = value
            .pointer("/payload/cwd")
            .and_then(Value::as_str)
            .or_else(|| value.pointer("/payload/session_meta/cwd").and_then(Value::as_str))
            .or_else(|| value.pointer("/payload/turn_context/cwd").and_then(Value::as_str))
            .or_else(|| value.pointer("/payload/session_meta/payload/cwd").and_then(Value::as_str))
            .or_else(|| value.pointer("/payload/turn_context/payload/cwd").and_then(Value::as_str))
        {
            if cwd == workspace_text.as_ref() {
                saw_workspace = true;
            }
        }

        let Some(info) = value.pointer("/payload/info") else {
            continue;
        };
        let Some(tokens) = info
            .pointer("/last_token_usage/total_tokens")
            .and_then(Value::as_i64)
        else {
            continue;
        };
        let limit = info
            .pointer("/model_context_window")
            .and_then(Value::as_i64)
            .unwrap_or(fallback_limit);
        let observed_at = value
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_ts)
            .unwrap_or_else(|| file_modified_at(path).unwrap_or_else(Utc::now));
        if observed_at < started_at {
            continue;
        }
        best = Some(ScannedReading {
            tokens,
            limit,
            observed_at,
            source_kind: TranscriptSourceKind::Codex,
        });
    }

    if saw_workspace && saw_instance {
        best
    } else {
        None
    }
}

fn text_declares_own_agent_id(text: &str, instance_id: &str) -> bool {
    let text = text.to_ascii_lowercase();
    let needle = format!("own agent id is {}", instance_id.to_ascii_lowercase());
    text.contains(&needle)
}

fn claude_value_declares_owner(value: &Value, instance_id: &str) -> bool {
    match value.get("type").and_then(Value::as_str) {
        Some("attachment" | "user" | "system") => value_texts(value)
            .any(|text| text_declares_own_agent_id(text, instance_id)),
        _ => false,
    }
}

fn codex_value_declares_owner(value: &Value, instance_id: &str) -> bool {
    if value.get("type").and_then(Value::as_str) != Some("response_item") {
        return false;
    }
    let Some(payload) = value.get("payload") else {
        return false;
    };
    if payload.get("type").and_then(Value::as_str) != Some("message")
        || payload.get("role").and_then(Value::as_str) != Some("developer")
    {
        return false;
    }
    payload
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("input_text"))
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .any(|text| text_declares_own_agent_id(text, instance_id))
}

fn value_texts(value: &Value) -> impl Iterator<Item = &str> {
    [
        value.get("text").and_then(Value::as_str),
        value.get("content").and_then(Value::as_str),
        value.pointer("/message/content").and_then(Value::as_str),
        value.pointer("/message/text").and_then(Value::as_str),
    ]
    .into_iter()
    .flatten()
}

fn sum_claude_usage(usage: &Value) -> i64 {
    usage
        .pointer("/input_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        + usage
            .pointer("/cache_creation_input_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0)
        + usage
            .pointer("/cache_read_input_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0)
        + usage
            .pointer("/output_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0)
}

fn file_modified_at(path: &Path) -> Option<DateTime<Utc>> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    Some(DateTime::<Utc>::from(modified))
}

fn parse_ts(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    fn tmp_root(prefix: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("{}-{}", prefix, Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create temp root");
        root
    }

    fn write_jsonl(path: &Path, lines: &[Value]) {
        let mut body = String::new();
        for (idx, line) in lines.iter().enumerate() {
            if idx > 0 {
                body.push('\n');
            }
            body.push_str(&line.to_string());
        }
        std::fs::write(path, body).expect("write jsonl");
    }

    fn codex_owner_line(instance_id: &str) -> Value {
        json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "developer",
                "content": [
                    {
                        "type": "input_text",
                        "text": format!("You are Dabin, and your own agent id is {instance_id}.")
                    }
                ]
            }
        })
    }

    fn codex_token_line(timestamp: &str, tokens: i64, limit: i64, workspace: &Path) -> Value {
        json!({
            "type": "event_msg",
            "timestamp": timestamp,
            "payload": {
                "type": "token_count",
                "cwd": workspace.to_string_lossy(),
                "info": {
                    "last_token_usage": {
                        "input_tokens": tokens - 30,
                        "cached_input_tokens": 10,
                        "output_tokens": 11,
                        "reasoning_output_tokens": 9,
                        "total_tokens": tokens
                    },
                    "total_token_usage": { "total_tokens": tokens + 10_000 },
                    "model_context_window": limit
                }
            }
        })
    }

    fn claude_owner_line(instance_id: &str, workspace: &Path) -> Value {
        json!({
            "type": "user",
            "cwd": workspace.to_string_lossy(),
            "message": {
                "role": "user",
                "content": format!("Startup context: your own agent id is {instance_id}.")
            }
        })
    }

    fn claude_usage_line(tokens: i64) -> Value {
        json!({
            "type": "assistant",
            "requestId": "req-1",
            "message": {
                "id": "msg-1",
                "role": "assistant",
                "type": "message",
                "usage": {
                    "input_tokens": tokens - 3,
                    "cache_creation_input_tokens": 1,
                    "cache_read_input_tokens": 1,
                    "output_tokens": 1
                }
            }
        })
    }

    #[test]
    fn claude_dedupes_duplicate_assistant_usage() {
        let claude_root = tmp_root("claude-root");
        let codex_root = tmp_root("codex-root");
        let workspace = tmp_root("workspace");
        let instance_id = "inst-claude-1";
        let file = claude_root.join("session.jsonl");

        write_jsonl(
            &file,
            &[
                json!({
                    "type": "attachment",
                    "cwd": workspace.to_string_lossy(),
                    "text": format!("your own agent id is {instance_id}"),
                }),
                json!({
                    "type": "assistant",
                    "requestId": "req-1",
                    "message": {
                        "id": "msg-1",
                        "role": "assistant",
                        "type": "message",
                        "usage": {
                            "input_tokens": 10,
                            "cache_creation_input_tokens": 1,
                            "cache_read_input_tokens": 2,
                            "output_tokens": 3
                        }
                    }
                }),
                json!({
                    "type": "assistant",
                    "requestId": "req-1",
                    "message": {
                        "id": "msg-2",
                        "role": "assistant",
                        "type": "message",
                        "usage": {
                            "input_tokens": 20,
                            "cache_creation_input_tokens": 4,
                            "cache_read_input_tokens": 5,
                            "output_tokens": 6
                        }
                    }
                }),
            ],
        );

        let reader = TranscriptContextReader::new(TranscriptContextConfig {
            claude_projects_root: claude_root.clone(),
            codex_sessions_root: codex_root.clone(),
            fallback_limit: 200_000,
        });
        let reading = reader
            .poll(
                instance_id,
                &workspace,
                "claude-code",
                DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH),
            )
            .expect("expected matching claude reading");

        assert_eq!(reading.tokens, 20 + 4 + 5 + 6);
        assert_eq!(reading.limit, 200_000);
        assert_eq!(reading.source_kind, TranscriptSourceKind::ClaudeCode);
        assert!(!reading.observed_at.is_empty());

        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_dir_all(&claude_root);
        let _ = std::fs::remove_dir_all(&codex_root);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn codex_prefers_last_token_count_and_rejects_total_token_usage() {
        let claude_root = tmp_root("claude-root");
        let codex_root = tmp_root("codex-root");
        let workspace = tmp_root("workspace");
        let instance_id = "inst-codex-1";
        let file = codex_root.join("rollout.jsonl");

        write_jsonl(
            &file,
            &[
                json!({
                    "type": "session_meta",
                    "timestamp": "2099-01-01T00:00:00Z",
                    "payload": {
                        "cwd": workspace.to_string_lossy(),
                        "id": "codex-session-id-not-conclave-instance-id",
                        "originator": "codex-tui"
                    }
                }),
                codex_owner_line(instance_id),
                codex_token_line("2099-01-01T00:00:01Z", 111, 4_000, &workspace),
                codex_token_line("2099-01-01T00:00:02Z", 222, 8_000, &workspace),
            ],
        );

        let reader = TranscriptContextReader::new(TranscriptContextConfig {
            claude_projects_root: claude_root.clone(),
            codex_sessions_root: codex_root.clone(),
            fallback_limit: 200_000,
        });
        let reading = reader
            .poll(
                instance_id,
                &workspace,
                "codex",
                DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH),
            )
            .expect("expected matching codex reading");

        assert_eq!(reading.tokens, 222);
        assert_eq!(reading.limit, 8_000);
        assert_eq!(reading.observed_at, "2099-01-01T00:00:02+00:00");
        assert_eq!(reading.source_kind, TranscriptSourceKind::Codex);

        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_dir_all(&claude_root);
        let _ = std::fs::remove_dir_all(&codex_root);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn codex_uses_owner_marker_not_later_roster_mentions() {
        let claude_root = tmp_root("claude-root");
        let codex_root = tmp_root("codex-root");
        let workspace = tmp_root("workspace");
        let agent_a = "agent-a";
        let agent_b = "agent-b";

        write_jsonl(
            &codex_root.join("agent-a.jsonl"),
            &[
                json!({
                    "type": "session_meta",
                    "timestamp": "2099-01-01T00:00:00Z",
                    "payload": {
                        "cwd": workspace.to_string_lossy(),
                        "id": "codex-session-a",
                        "originator": "codex-tui"
                    }
                }),
                codex_owner_line(agent_a),
                codex_token_line("2099-01-01T00:00:01Z", 101, 1_000, &workspace),
            ],
        );
        write_jsonl(
            &codex_root.join("agent-b.jsonl"),
            &[
                json!({
                    "type": "session_meta",
                    "timestamp": "2099-01-01T00:00:00Z",
                    "payload": {
                        "cwd": workspace.to_string_lossy(),
                        "id": "codex-session-b",
                        "originator": "codex-tui"
                    }
                }),
                codex_owner_line(agent_b),
                json!({
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "assistant",
                        "content": [
                            {
                                "type": "output_text",
                                "text": format!("Roster output mentions {agent_a}, but this is not ownership.")
                            }
                        ]
                    }
                }),
                codex_token_line("2099-01-01T00:00:02Z", 202, 2_000, &workspace),
            ],
        );

        let reader = TranscriptContextReader::new(TranscriptContextConfig {
            claude_projects_root: claude_root.clone(),
            codex_sessions_root: codex_root.clone(),
            fallback_limit: 200_000,
        });

        let reading_a = reader
            .poll(
                agent_a,
                &workspace,
                "codex",
                DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH),
            )
            .expect("expected agent-a reading");
        let reading_b = reader
            .poll(
                agent_b,
                &workspace,
                "codex",
                DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH),
            )
            .expect("expected agent-b reading");

        assert_eq!(reading_a.tokens, 101);
        assert_eq!(reading_a.limit, 1_000);
        assert_eq!(reading_b.tokens, 202);
        assert_eq!(reading_b.limit, 2_000);

        let _ = std::fs::remove_dir_all(&claude_root);
        let _ = std::fs::remove_dir_all(&codex_root);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn codex_ignores_workspace_file_with_only_arbitrary_id_mention() {
        let claude_root = tmp_root("claude-root");
        let codex_root = tmp_root("codex-root");
        let workspace = tmp_root("workspace");
        let instance_id = "agent-mentioned-only";

        write_jsonl(
            &codex_root.join("mention-only.jsonl"),
            &[
                json!({
                    "type": "session_meta",
                    "timestamp": "2099-01-01T00:00:00Z",
                    "payload": {
                        "cwd": workspace.to_string_lossy(),
                        "id": "codex-session",
                        "originator": "codex-tui"
                    }
                }),
                json!({
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "assistant",
                        "content": [
                            {
                                "type": "output_text",
                                "text": format!("Task note mentions {instance_id}, but no owner marker exists.")
                            }
                        ]
                    }
                }),
                codex_token_line("2099-01-01T00:00:01Z", 303, 3_000, &workspace),
            ],
        );

        let reader = TranscriptContextReader::new(TranscriptContextConfig {
            claude_projects_root: claude_root.clone(),
            codex_sessions_root: codex_root.clone(),
            fallback_limit: 200_000,
        });
        assert!(
            reader
                .poll(
                    instance_id,
                    &workspace,
                    "codex",
                    DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH),
                )
                .is_none(),
            "arbitrary transcript text must not establish ownership"
        );

        let _ = std::fs::remove_dir_all(&claude_root);
        let _ = std::fs::remove_dir_all(&codex_root);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn codex_token_formula_uses_reported_last_total_without_offset() {
        let claude_root = tmp_root("claude-root");
        let codex_root = tmp_root("codex-root");
        let workspace = tmp_root("workspace");
        let instance_id = "agent-formula";

        write_jsonl(
            &codex_root.join("formula.jsonl"),
            &[
                json!({
                    "type": "session_meta",
                    "timestamp": "2099-01-01T00:00:00Z",
                    "payload": {
                        "cwd": workspace.to_string_lossy(),
                        "id": "codex-session-formula",
                        "originator": "codex-tui"
                    }
                }),
                codex_owner_line(instance_id),
                codex_token_line("2099-01-01T00:00:01Z", 222, 8_000, &workspace),
            ],
        );

        let reader = TranscriptContextReader::new(TranscriptContextConfig {
            claude_projects_root: claude_root.clone(),
            codex_sessions_root: codex_root.clone(),
            fallback_limit: 200_000,
        });
        let reading = reader
            .poll(
                instance_id,
                &workspace,
                "codex",
                DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH),
            )
            .expect("expected codex reading");

        assert_eq!(reading.tokens, 222);
        assert_eq!(reading.limit, 8_000);

        let _ = std::fs::remove_dir_all(&claude_root);
        let _ = std::fs::remove_dir_all(&codex_root);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn claude_uses_owner_marker_not_later_mentions() {
        let claude_root = tmp_root("claude-root");
        let codex_root = tmp_root("codex-root");
        let workspace = tmp_root("workspace");
        let agent_a = "claude-agent-a";
        let agent_b = "claude-agent-b";

        let file = claude_root.join("agent-b.jsonl");
        write_jsonl(
            &file,
            &[
                claude_owner_line(agent_b, &workspace),
                json!({
                    "type": "user",
                    "message": {
                        "role": "user",
                        "content": format!("Later roster text mentions {agent_a}.")
                    }
                }),
                claude_usage_line(77),
            ],
        );

        assert!(
            scan_claude_file(
                &file,
                agent_a,
                &workspace,
                DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH),
                200_000,
            )
            .is_none(),
            "later mentions must not establish Claude transcript ownership"
        );
        assert!(
            scan_claude_file(
                &file,
                agent_b,
                &workspace,
                DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH),
                200_000,
            )
            .is_some(),
            "owner marker should match the owning Claude agent"
        );

        let _ = std::fs::remove_dir_all(&claude_root);
        let _ = std::fs::remove_dir_all(&codex_root);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn unmatched_files_are_ignored() {
        let claude_root = tmp_root("claude-root");
        let codex_root = tmp_root("codex-root");
        let workspace = tmp_root("workspace");
        let other_workspace = tmp_root("other-workspace");
        let instance_id = "inst-ignored-1";

        write_jsonl(
            &claude_root.join("wrong-cwd.jsonl"),
            &[json!({
                "type": "assistant",
                "requestId": "req-1",
                "message": {
                    "id": instance_id,
                    "role": "assistant",
                    "type": "message",
                    "usage": {
                        "input_tokens": 1,
                        "cache_creation_input_tokens": 0,
                        "cache_read_input_tokens": 0,
                        "output_tokens": 1
                    }
                },
                "cwd": other_workspace.to_string_lossy()
            })],
        );
        write_jsonl(
            &codex_root.join("wrong-instance.jsonl"),
            &[json!({
                "type": "session_meta",
                "timestamp": "2099-01-01T00:00:00Z",
                "payload": {
                    "cwd": workspace.to_string_lossy(),
                    "id": "different-instance",
                    "originator": "codex-tui"
                }
            })],
        );

        let reader = TranscriptContextReader::new(TranscriptContextConfig {
            claude_projects_root: claude_root.clone(),
            codex_sessions_root: codex_root.clone(),
            fallback_limit: 200_000,
        });
        assert!(
            reader
                .poll(
                    instance_id,
                    &workspace,
                    "claude-code",
                    DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH),
                )
                .is_none(),
            "files that fail the cwd/self-id match must be ignored"
        );
        assert!(
            reader
                .poll(
                    instance_id,
                    &workspace,
                    "codex",
                    DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH),
                )
                .is_none(),
            "files that fail the cwd/self-id match must be ignored"
        );

        let _ = std::fs::remove_dir_all(&claude_root);
        let _ = std::fs::remove_dir_all(&codex_root);
        let _ = std::fs::remove_dir_all(&workspace);
        let _ = std::fs::remove_dir_all(&other_workspace);
    }
}
