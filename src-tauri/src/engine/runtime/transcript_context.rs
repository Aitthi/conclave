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
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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

/// Reader with per-file incremental scan state.
///
/// The engine clones one reader per poll into `spawn_blocking`
/// (`instance.rs::poll_transcript_context`), so the state MUST be `Arc`-shared
/// across clones — a per-clone cache would never hit and the meter would
/// silently fall back to full re-parsing every 2s (the ~320% CPU incident this
/// state exists to fix). The map is keyed by `(instance_id, path)`: production
/// builds one reader per agent instance, but the key still carries the
/// instance so a reader shared across instances (as some tests do) can never
/// serve one agent a reduction accumulated under another agent's ownership
/// rules. Workspace and start time are constant per (reader, instance).
#[derive(Debug, Clone)]
pub struct TranscriptContextReader {
    config: TranscriptContextConfig,
    scan_state: Arc<Mutex<HashMap<(String, PathBuf), FileScanState>>>,
}

impl TranscriptContextReader {
    pub fn new(config: TranscriptContextConfig) -> Self {
        Self {
            config,
            scan_state: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[cfg(test)]
    fn file_opens(&self, instance_id: &str, path: &Path) -> u64 {
        self.scan_state
            .lock()
            .unwrap()
            .get(&(instance_id.to_owned(), path.to_path_buf()))
            .map_or(0, |state| state.opens)
    }

    #[cfg(test)]
    fn file_offset(&self, instance_id: &str, path: &Path) -> u64 {
        self.scan_state
            .lock()
            .unwrap()
            .get(&(instance_id.to_owned(), path.to_path_buf()))
            .map_or(0, |state| state.offset)
    }

    /// Test hook for the risk-ledger property the whole fix hangs on: a clone
    /// must observe the SAME state map, or the cache never hits from the
    /// per-poll clone in `poll_transcript_context` and the CPU regression is
    /// back with every counter still reading innocently.
    #[cfg(test)]
    fn shares_state_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.scan_state, &other.scan_state)
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
        // Claude Code stores a workspace's transcripts under a per-cwd project
        // directory (`~/.claude/projects/<slug-of-cwd>/`). Scan only that dir
        // instead of the whole projects tree — on a real machine the tree holds
        // thousands of unrelated historical sessions (GBs). If the slug dir is
        // absent (unexpected cwd shape), fall back to the full root; the mtime
        // filter in `collect_jsonl_files` keeps even that fallback cheap.
        let project_dir = claude_project_dir(&self.config.claude_projects_root, workspace_folder);
        let scan_root = if project_dir.is_dir() {
            project_dir
        } else {
            self.config.claude_projects_root.clone()
        };
        let mut best: Option<ScannedReading> = None;
        let mut states = self.scan_state.lock().unwrap();
        for path in collect_jsonl_files(&scan_root, started_at) {
            let state = states
                .entry((instance_id.to_owned(), path.clone()))
                .or_insert_with(|| FileScanState::new(ScanAcc::Claude(ClaudeAcc::default())));
            let reading = scan_file_incremental(
                &path,
                instance_id,
                workspace_folder,
                started_at,
                self.config.fallback_limit,
                state,
            );
            best = choose_newer(best, reading);
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
        // Codex stores sessions under date buckets, not per-cwd, so there is no
        // dir to scope to — but the mtime filter still skips every rollout that
        // was last written before this agent started, i.e. every closed session.
        let mut states = self.scan_state.lock().unwrap();
        for path in collect_jsonl_files(&self.config.codex_sessions_root, started_at) {
            let state = states
                .entry((instance_id.to_owned(), path.clone()))
                .or_insert_with(|| FileScanState::new(ScanAcc::Codex(CodexAcc::default())));
            let reading = scan_file_incremental(
                &path,
                instance_id,
                workspace_folder,
                started_at,
                self.config.fallback_limit,
                state,
            );
            best = choose_newer(best, reading);
        }
        best.map(ScannedReading::into_reading)
    }
}

/// Per-file incremental scan state (see [`TranscriptContextReader`] for why it
/// is `Arc`-shared). `offset` is advanced ONLY through the last COMPLETE
/// newline-terminated line — a partial tail line (writer mid-append) is left
/// unconsumed and re-read on the next poll, so a torn write can never corrupt
/// the accumulated reduction (risk ledger: torn reads).
#[derive(Debug)]
struct FileScanState {
    offset: u64,
    len: u64,
    mtime: Option<DateTime<Utc>>,
    /// How many times the file was actually opened (test hook for the
    /// unchanged-file short-circuit).
    opens: u64,
    cached: Option<ScannedReading>,
    acc: ScanAcc,
}

impl FileScanState {
    fn new(acc: ScanAcc) -> Self {
        Self {
            offset: 0,
            len: 0,
            mtime: None,
            opens: 0,
            cached: None,
            acc,
        }
    }
}

#[derive(Debug)]
enum ScanAcc {
    Claude(ClaudeAcc),
    Codex(CodexAcc),
}

/// Stat-first incremental scan: an unchanged file (same length and mtime as
/// the previous poll) is answered from the cached reduction without an open;
/// a grown file is parsed from the consumed offset only; a shrunken file
/// (rotation, or Claude Code rewriting a session on compaction) resets the
/// state and rescans from zero. The reduction is written so that the reading
/// for any (file, instance) always equals what a fresh scan from byte 0 with
/// the same complete-line rule would produce.
fn scan_file_incremental(
    path: &Path,
    instance_id: &str,
    workspace_folder: &Path,
    started_at: DateTime<Utc>,
    fallback_limit: i64,
    state: &mut FileScanState,
) -> Option<ScannedReading> {
    let meta = fs::metadata(path).ok()?;
    let len = meta.len();
    let mtime = file_modified_at(path);
    if state.opens > 0 && len == state.len && mtime == state.mtime {
        return state.cached.clone();
    }
    if len < state.offset {
        // Rewritten/truncated in place: everything we consumed is stale.
        state.offset = 0;
        match &mut state.acc {
            ScanAcc::Claude(acc) => *acc = ClaudeAcc::default(),
            ScanAcc::Codex(acc) => *acc = CodexAcc::default(),
        }
    }
    state.opens += 1;
    let mut file = File::open(path).ok()?;
    file.seek(SeekFrom::Start(state.offset)).ok()?;
    let mut reader = BufReader::new(file);
    let workspace_text = workspace_folder.to_string_lossy();
    let mut buf: Vec<u8> = Vec::new();
    let mut tail: Option<String> = None;
    loop {
        buf.clear();
        let Ok(n) = reader.read_until(b'\n', &mut buf) else {
            break;
        };
        if n == 0 {
            break;
        }
        if buf.last() != Some(&b'\n') {
            // Partial tail: NEVER consumed into persistent state (the writer
            // may be mid-line — risk ledger: torn reads). It is overlaid
            // transiently below so a flushed-but-unterminated record still
            // counts this poll, exactly as the old full `lines()` scan did
            // (ruling f674c8d1).
            tail = Some(String::from_utf8_lossy(&buf).into_owned());
            break;
        }
        state.offset += n as u64;
        let text = String::from_utf8_lossy(&buf);
        let line = text.trim_end_matches(['\r', '\n']);
        match &mut state.acc {
            ScanAcc::Claude(acc) => acc.ingest_line(line, instance_id, workspace_text.as_ref()),
            ScanAcc::Codex(acc) => acc.ingest_line(
                line,
                instance_id,
                workspace_text.as_ref(),
                path,
                started_at,
                fallback_limit,
            ),
        }
    }
    state.len = len;
    state.mtime = mtime;
    // Finalize over (persistent state + transient tail). A torn/invalid tail
    // simply parses to nothing; when its newline lands on a later poll the
    // record folds into persistent state exactly once. Caching the overlaid
    // result is sound: the short-circuit only replays it while len and mtime
    // are unchanged, i.e. while the tail bytes are still identical.
    state.cached = match &state.acc {
        ScanAcc::Claude(acc) => {
            let acc = match &tail {
                Some(line) => {
                    let mut probe = acc.clone();
                    probe.ingest_line(line, instance_id, workspace_text.as_ref());
                    probe
                }
                None => acc.clone(),
            };
            acc.finalize(path, started_at, fallback_limit)
        }
        ScanAcc::Codex(acc) => {
            let acc = match &tail {
                Some(line) => {
                    let mut probe = acc.clone();
                    probe.ingest_line(
                        line,
                        instance_id,
                        workspace_text.as_ref(),
                        path,
                        started_at,
                        fallback_limit,
                    );
                    probe
                }
                None => acc.clone(),
            };
            acc.finalize()
        }
    };
    state.cached.clone()
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

/// The Claude Code project directory for a workspace cwd. Claude slugifies the
/// absolute cwd by replacing every non-alphanumeric character with `-` (e.g.
/// `/Users/x/code/app` → `-Users-x-code-app`); mirror that so we can scan just
/// this workspace's transcripts instead of the whole projects tree.
fn claude_project_dir(root: &Path, workspace_folder: &Path) -> PathBuf {
    let slug: String = workspace_folder
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    root.join(slug)
}

/// Collect `.jsonl` files under `root` that were modified at or after
/// `min_mtime`. A closed session's transcript can never gain the current
/// agent's usage rows, so filtering by mtime here — a cheap `stat`, before any
/// file is opened or parsed — keeps the meter off the (potentially many GB of)
/// historical transcripts that a full parse would otherwise churn every poll.
fn collect_jsonl_files(root: &Path, min_mtime: DateTime<Utc>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_jsonl_files_inner(root, min_mtime, &mut out);
    out
}

fn collect_jsonl_files_inner(root: &Path, min_mtime: DateTime<Utc>, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files_inner(&path, min_mtime, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            match file_modified_at(&path) {
                Some(modified) if modified >= min_mtime => out.push(path),
                _ => {}
            }
        }
    }
}

/// Context window implied by a Claude model id, when the id itself determines
/// it. `None` means "unknown — use the session fallback". Claude Code
/// transcripts carry no explicit window field (unlike Codex), so the assistant
/// line's `message.model` is the only signal we have.
fn claude_model_context_window(model: &str) -> Option<i64> {
    let m = model.to_ascii_lowercase();
    if m.contains("[1m]") {
        return Some(1_000_000); // explicit 1M-beta variants, e.g. "claude-sonnet-4-5[1m]"
    }
    if m.starts_with("claude-fable-5") {
        return Some(1_000_000); // Fable 5 sessions run the 1M window (verified live)
    }
    None
}

/// Per-line reduction of a Claude Code transcript. Everything the reading
/// depends on is folded in incrementally so a suffix scan continues exactly
/// where the previous one stopped: ownership flags are monotonic, and
/// `last_model_window` keeps the latest RECOGNIZED model's window — a
/// `<synthetic>` or unknown id maps to None and leaves the previous window in
/// place, so mid-session noise never drops us back to the fallback.
///
/// `latest_usage` replaces the old per-requestId map (ruling f674c8d1): every
/// usage insert carries a strictly increasing line number, so the map's
/// max-by-line_no was ALWAYS its most recent insert — the retried-requestId
/// de-dup could never change the winner, only overwrite a key that had already
/// lost. A single `(line_no, tokens)` scalar is the same reduction, O(1) per
/// poll instead of an unbounded map held per file all session.
#[derive(Debug, Default, Clone)]
struct ClaudeAcc {
    line_no: usize,
    saw_workspace: bool,
    saw_instance: bool,
    last_model_window: Option<i64>,
    latest_usage: Option<(usize, i64)>,
}

impl ClaudeAcc {
    fn ingest_line(&mut self, line: &str, instance_id: &str, workspace_text: &str) {
        self.line_no += 1;
        if !self.saw_workspace && line.contains(workspace_text) {
            self.saw_workspace = true;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return;
        };
        if !self.saw_instance && claude_value_declares_owner(&value, instance_id) {
            self.saw_instance = true;
        }
        if value.get("type").and_then(Value::as_str) != Some("assistant") {
            return;
        }
        let Some(message) = value.get("message") else {
            return;
        };
        if let Some(window) = message
            .get("model")
            .and_then(Value::as_str)
            .and_then(claude_model_context_window)
        {
            self.last_model_window = Some(window);
        }
        let Some(usage) = message.get("usage") else {
            return;
        };
        self.latest_usage = Some((self.line_no, sum_claude_usage(usage)));
    }

    fn finalize(
        &self,
        path: &Path,
        started_at: DateTime<Utc>,
        fallback_limit: i64,
    ) -> Option<ScannedReading> {
        if !self.saw_workspace || !self.saw_instance {
            return None;
        }
        let (_, tokens) = self.latest_usage?;
        let observed_at = file_modified_at(path)?;
        if observed_at < started_at {
            return None;
        }
        Some(ScannedReading {
            tokens,
            limit: self.last_model_window.unwrap_or(fallback_limit),
            observed_at,
            source_kind: TranscriptSourceKind::ClaudeCode,
        })
    }
}

/// One-shot full scan (no persistent state). The polling path uses
/// [`scan_file_incremental`]; this stays for callers/tests that want the
/// classic read-everything semantics, including an unterminated final line.
#[cfg(test)]
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
    let mut acc = ClaudeAcc::default();
    for line in reader.lines().map_while(Result::ok) {
        acc.ingest_line(&line, instance_id, workspace_text.as_ref());
    }
    acc.finalize(path, started_at, fallback_limit)
}

/// Per-line reduction of a Codex rollout. Ownership flags are monotonic and
/// `best` keeps the LAST admissible token_count (later lines simply overwrite),
/// so a suffix scan continues the same reduction a full scan would compute.
#[derive(Debug, Default, Clone)]
struct CodexAcc {
    saw_workspace: bool,
    saw_instance: bool,
    best: Option<ScannedReading>,
}

impl CodexAcc {
    fn ingest_line(
        &mut self,
        line: &str,
        instance_id: &str,
        workspace_text: &str,
        path: &Path,
        started_at: DateTime<Utc>,
        fallback_limit: i64,
    ) {
        if !self.saw_workspace && line.contains(workspace_text) {
            self.saw_workspace = true;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return;
        };
        if !self.saw_instance && codex_value_declares_owner(&value, instance_id) {
            self.saw_instance = true;
        }
        if value.get("type").and_then(Value::as_str) != Some("event_msg") {
            return;
        }
        if value.pointer("/payload/type").and_then(Value::as_str) != Some("token_count") {
            return;
        }

        if let Some(cwd) = value
            .pointer("/payload/cwd")
            .and_then(Value::as_str)
            .or_else(|| {
                value
                    .pointer("/payload/session_meta/cwd")
                    .and_then(Value::as_str)
            })
            .or_else(|| {
                value
                    .pointer("/payload/turn_context/cwd")
                    .and_then(Value::as_str)
            })
            .or_else(|| {
                value
                    .pointer("/payload/session_meta/payload/cwd")
                    .and_then(Value::as_str)
            })
            .or_else(|| {
                value
                    .pointer("/payload/turn_context/payload/cwd")
                    .and_then(Value::as_str)
            })
        {
            if cwd == workspace_text {
                self.saw_workspace = true;
            }
        }

        let Some(info) = value.pointer("/payload/info") else {
            return;
        };
        let Some(tokens) = info
            .pointer("/last_token_usage/total_tokens")
            .and_then(Value::as_i64)
        else {
            return;
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
            return;
        }
        self.best = Some(ScannedReading {
            tokens,
            limit,
            observed_at,
            source_kind: TranscriptSourceKind::Codex,
        });
    }

    fn finalize(&self) -> Option<ScannedReading> {
        if self.saw_workspace && self.saw_instance {
            self.best.clone()
        } else {
            None
        }
    }
}

/// One-shot full scan (no persistent state); see [`scan_claude_file`].
#[cfg(test)]
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
    let mut acc = CodexAcc::default();
    for line in reader.lines().map_while(Result::ok) {
        acc.ingest_line(
            &line,
            instance_id,
            workspace_text.as_ref(),
            path,
            started_at,
            fallback_limit,
        );
    }
    acc.finalize()
}

fn text_declares_own_agent_id(text: &str, instance_id: &str) -> bool {
    let text = text.to_ascii_lowercase();
    let needle = format!("own agent id is {}", instance_id.to_ascii_lowercase());
    text.contains(&needle)
}

fn claude_value_declares_owner(value: &Value, instance_id: &str) -> bool {
    match value.get("type").and_then(Value::as_str) {
        Some("attachment" | "user" | "system") => value_texts(value)
            .iter()
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

/// Every text fragment a claude transcript line can carry the owner marker in.
///
/// Real transcripts deliver it three ways (verified against live files):
/// - `attachment.content` — a LIST of strings, how claude-code records a
///   SessionStart hook's `additionalContext` (type `hook_additional_context`).
/// - `attachment.stdout` — raw hook stdout (type `hook_success`).
/// - `message.content` — a plain string OR an array of `{type:"text",text}`
///   blocks on `user`/`system` lines.
fn value_texts(value: &Value) -> Vec<&str> {
    let mut out = Vec::new();
    if let Some(t) = value.get("text").and_then(Value::as_str) {
        out.push(t);
    }
    push_content_texts(value.get("content"), &mut out);
    if let Some(att) = value.get("attachment") {
        push_content_texts(att.get("content"), &mut out);
        if let Some(s) = att.get("stdout").and_then(Value::as_str) {
            out.push(s);
        }
    }
    push_content_texts(value.pointer("/message/content"), &mut out);
    if let Some(t) = value.pointer("/message/text").and_then(Value::as_str) {
        out.push(t);
    }
    out
}

/// Push every text a `content` node carries: a plain string, or an array of
/// strings / `{type:"text",text}` blocks.
fn push_content_texts<'v>(node: Option<&'v Value>, out: &mut Vec<&'v str>) {
    match node {
        Some(Value::String(s)) => out.push(s.as_str()),
        Some(Value::Array(items)) => {
            for item in items {
                match item {
                    Value::String(s) => out.push(s.as_str()),
                    Value::Object(_) => {
                        if let Some(t) = item.get("text").and_then(Value::as_str) {
                            out.push(t);
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
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

    /// An assistant usage line that also carries `message.model`, mirroring how
    /// real Claude Code transcripts stamp the model id on every assistant line.
    /// `req` distinguishes lines so the latest-line dedupe keys them apart.
    fn claude_usage_line_with_model(req: &str, tokens: i64, model: &str) -> Value {
        json!({
            "type": "assistant",
            "requestId": req,
            "message": {
                "id": req,
                "role": "assistant",
                "type": "message",
                "model": model,
                "usage": {
                    "input_tokens": tokens - 3,
                    "cache_creation_input_tokens": 1,
                    "cache_read_input_tokens": 1,
                    "output_tokens": 1
                }
            }
        })
    }

    /// The owner marker's real delivery channel: claude-code records a
    /// SessionStart hook's `additionalContext` as an `attachment` line whose
    /// `attachment.content` is a LIST of context strings. The system-prompt
    /// append (`--append-system-prompt`) is never written to the transcript,
    /// so this attachment shape is the only recorded form of the marker.
    #[test]
    fn claude_owner_via_session_start_hook_attachment() {
        let claude_root = tmp_root("claude-root");
        let workspace = tmp_root("workspace");
        let instance_id = "inst-hook-1";
        let file = claude_root.join("session.jsonl");

        write_jsonl(
            &file,
            &[
                json!({
                    "type": "attachment",
                    "cwd": workspace.to_string_lossy(),
                    "attachment": {
                        "type": "hook_additional_context",
                        "hookName": "SessionStart",
                        "hookEvent": "SessionStart",
                        "content": [
                            format!("You are a Conclave agent, and your own agent id is {instance_id}.")
                        ]
                    }
                }),
                claude_usage_line(42),
            ],
        );

        let reading = scan_claude_file(
            &file,
            instance_id,
            &workspace,
            DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH),
            200_000,
        )
        .expect("hook_additional_context attachment must establish ownership");
        assert_eq!(reading.tokens, 42);

        let _ = std::fs::remove_dir_all(&claude_root);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    /// A SessionStart hook that prints plain text (no JSON wrapper) is recorded
    /// as a `hook_success` attachment with the text in `attachment.stdout`.
    #[test]
    fn claude_owner_via_hook_stdout() {
        let claude_root = tmp_root("claude-root");
        let workspace = tmp_root("workspace");
        let instance_id = "inst-stdout-1";
        let file = claude_root.join("session.jsonl");

        write_jsonl(
            &file,
            &[
                json!({
                    "type": "attachment",
                    "cwd": workspace.to_string_lossy(),
                    "attachment": {
                        "type": "hook_success",
                        "hookName": "SessionStart",
                        "hookEvent": "SessionStart",
                        "content": "",
                        "stdout": format!("your own agent id is {instance_id}\n"),
                        "stderr": ""
                    }
                }),
                claude_usage_line(43),
            ],
        );

        let reading = scan_claude_file(
            &file,
            instance_id,
            &workspace,
            DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH),
            200_000,
        )
        .expect("hook_success stdout must establish ownership");
        assert_eq!(reading.tokens, 43);

        let _ = std::fs::remove_dir_all(&claude_root);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    /// User messages in real transcripts frequently store `message.content` as
    /// an ARRAY of typed blocks, not a plain string; a marker typed as a user
    /// prompt (e.g. a post-/clear restore nudge) must still match.
    #[test]
    fn claude_owner_via_user_message_content_blocks() {
        let claude_root = tmp_root("claude-root");
        let workspace = tmp_root("workspace");
        let instance_id = "inst-blocks-1";
        let file = claude_root.join("session.jsonl");

        write_jsonl(
            &file,
            &[
                json!({
                    "type": "user",
                    "cwd": workspace.to_string_lossy(),
                    "message": {
                        "role": "user",
                        "content": [
                            { "type": "text", "text": format!("Startup: your own agent id is {instance_id}.") }
                        ]
                    }
                }),
                claude_usage_line(44),
            ],
        );

        let reading = scan_claude_file(
            &file,
            instance_id,
            &workspace,
            DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH),
            200_000,
        )
        .expect("array-form user message content must establish ownership");
        assert_eq!(reading.tokens, 44);

        let _ = std::fs::remove_dir_all(&claude_root);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    /// Incident regression (task context-meter-stale-model-limit): a fresh
    /// claude-code transcript that reports usage but no model window must read
    /// against the claude-code fallback (1M) — 140k tokens is 14%, not the 70%
    /// the old global 200k default produced.
    #[test]
    fn claude_no_model_window_reads_claude_code_fallback_not_global_default() {
        let claude_root = tmp_root("claude-root");
        let workspace = tmp_root("workspace");
        let instance_id = "inst-claude-1m";
        let file = claude_root.join("session.jsonl");

        write_jsonl(
            &file,
            &[
                json!({
                    "type": "attachment",
                    "cwd": workspace.to_string_lossy(),
                    "text": format!("your own agent id is {instance_id}"),
                }),
                claude_usage_line(140_000),
            ],
        );

        let fallback = crate::engine::repo::session::default_context_limit_for("claude-code");
        let reading = scan_claude_file(
            &file,
            instance_id,
            &workspace,
            DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH),
            fallback,
        )
        .expect("usage line must yield a reading");

        assert_eq!(reading.tokens, 140_000);
        assert_eq!(reading.limit, 1_000_000);
        assert_eq!(reading.tokens * 100 / reading.limit, 14);
        // The pre-fix shape: the same tokens against the conservative default
        // read as 70% — the false warning this lane removes for claude-code.
        assert_eq!(
            reading.tokens * 100 / crate::engine::repo::session::DEFAULT_CONTEXT_LIMIT,
            70
        );

        let _ = std::fs::remove_dir_all(&claude_root);
        let _ = std::fs::remove_dir_all(&workspace);
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
    fn poll_claude_scopes_to_cwd_project_dir_and_skips_other_projects() {
        let claude_root = tmp_root("claude-root");
        let codex_root = tmp_root("codex-root");
        let workspace = tmp_root("workspace");
        let instance_id = "inst-scope-1";

        // In-scope transcript: lives in this workspace's slugified project dir.
        let project_dir = claude_project_dir(&claude_root, &workspace);
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        write_jsonl(
            &project_dir.join("active.jsonl"),
            &[
                json!({
                    "type": "attachment",
                    "cwd": workspace.to_string_lossy(),
                    "text": format!("your own agent id is {instance_id}"),
                }),
                claude_usage_line(50),
            ],
        );

        // Decoy: a DIFFERENT project dir whose transcript also matches this
        // workspace cwd + owner marker, written LATER (newer mtime) with a
        // larger token count. A full-tree scan would read it and win on
        // recency; a cwd-scoped scan must never open it.
        let decoy_dir = claude_root.join("-Users-someone-else-other-project");
        std::fs::create_dir_all(&decoy_dir).expect("create decoy dir");
        write_jsonl(
            &decoy_dir.join("decoy.jsonl"),
            &[
                json!({
                    "type": "attachment",
                    "cwd": workspace.to_string_lossy(),
                    "text": format!("your own agent id is {instance_id}"),
                }),
                claude_usage_line(999),
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
            .expect("expected in-scope reading");

        assert_eq!(
            reading.tokens, 50,
            "scoped scan must read only the cwd project dir, never the decoy"
        );

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

    /// Helper: scan a single-file claude fixture built from an owner line plus
    /// the given assistant lines, returning the reading (or None).
    fn scan_claude_fixture(
        instance_id: &str,
        fallback_limit: i64,
        assistant_lines: &[Value],
    ) -> Option<ScannedReading> {
        let claude_root = tmp_root("claude-root");
        let workspace = tmp_root("workspace");
        let file = claude_root.join("session.jsonl");
        let mut lines = vec![claude_owner_line(instance_id, &workspace)];
        lines.extend_from_slice(assistant_lines);
        write_jsonl(&file, &lines);
        let reading = scan_claude_file(
            &file,
            instance_id,
            &workspace,
            DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH),
            fallback_limit,
        );
        let _ = std::fs::remove_dir_all(&claude_root);
        let _ = std::fs::remove_dir_all(&workspace);
        reading
    }

    #[test]
    fn claude_fable5_model_implies_1m_window() {
        let reading = scan_claude_fixture(
            "inst-fable",
            200_000,
            &[claude_usage_line_with_model(
                "req-1",
                75_900,
                "claude-fable-5",
            )],
        )
        .expect("expected reading");
        assert_eq!(reading.tokens, 75_900);
        assert_eq!(reading.limit, 1_000_000);
    }

    #[test]
    fn claude_1m_beta_variant_implies_1m_window() {
        let reading = scan_claude_fixture(
            "inst-1m",
            200_000,
            &[claude_usage_line_with_model(
                "req-1",
                500,
                "claude-sonnet-4-5[1m]",
            )],
        )
        .expect("expected reading");
        assert_eq!(reading.limit, 1_000_000);
    }

    #[test]
    fn claude_200k_model_uses_fallback_limit() {
        let reading = scan_claude_fixture(
            "inst-opus",
            200_000,
            &[claude_usage_line_with_model(
                "req-1",
                500,
                "claude-opus-4-8",
            )],
        )
        .expect("expected reading");
        assert_eq!(reading.limit, 200_000);
    }

    #[test]
    fn claude_no_model_field_uses_fallback_limit() {
        let reading = scan_claude_fixture("inst-nomodel", 200_000, &[claude_usage_line(500)])
            .expect("expected reading");
        assert_eq!(reading.limit, 200_000);
    }

    #[test]
    fn claude_last_recognized_model_wins_over_synthetic() {
        // fable (1M) first, then a `<synthetic>` model line last. The synthetic
        // id maps to nothing, so the last RECOGNIZED window (1M) must stick.
        let reading = scan_claude_fixture(
            "inst-synthetic",
            200_000,
            &[
                claude_usage_line_with_model("req-1", 500, "claude-fable-5"),
                claude_usage_line_with_model("req-2", 600, "<synthetic>"),
            ],
        )
        .expect("expected reading");
        assert_eq!(reading.limit, 1_000_000);
    }

    #[test]
    fn claude_model_context_window_maps_known_ids() {
        assert_eq!(
            claude_model_context_window("claude-fable-5"),
            Some(1_000_000)
        );
        // case-insensitive
        assert_eq!(
            claude_model_context_window("CLAUDE-FABLE-5"),
            Some(1_000_000)
        );
        assert_eq!(
            claude_model_context_window("claude-sonnet-4-5[1m]"),
            Some(1_000_000)
        );
        assert_eq!(
            claude_model_context_window("claude-sonnet-4-5[1M]"),
            Some(1_000_000)
        );
        assert_eq!(claude_model_context_window("claude-opus-4-8"), None);
        assert_eq!(claude_model_context_window("<synthetic>"), None);
        assert_eq!(claude_model_context_window(""), None);
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

    // ---- Incremental scan state (transcript-context-cpu-fix) -----------------

    /// `write_jsonl` deliberately leaves the final line unterminated; the
    /// incremental tests need real appender semantics: every record ends in
    /// a newline, exactly as Claude Code / Codex write them.
    fn write_jsonl_terminated(path: &Path, lines: &[Value]) {
        let mut body = String::new();
        for line in lines {
            body.push_str(&line.to_string());
            body.push('\n');
        }
        std::fs::write(path, body).expect("write jsonl");
    }

    fn append_raw(path: &Path, text: &str) {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .expect("open for append");
        file.write_all(text.as_bytes()).expect("append");
    }

    fn append_jsonl_terminated(path: &Path, lines: &[Value]) {
        for line in lines {
            append_raw(path, &format!("{line}\n"));
        }
    }

    struct IncrementalSetup {
        claude_root: PathBuf,
        codex_root: PathBuf,
        workspace: PathBuf,
        project_dir: PathBuf,
        reader: TranscriptContextReader,
    }

    impl IncrementalSetup {
        fn new() -> Self {
            let claude_root = tmp_root("claude-root");
            let codex_root = tmp_root("codex-root");
            let workspace = tmp_root("workspace");
            let project_dir = claude_project_dir(&claude_root, &workspace);
            std::fs::create_dir_all(&project_dir).expect("create project dir");
            let reader = TranscriptContextReader::new(TranscriptContextConfig {
                claude_projects_root: claude_root.clone(),
                codex_sessions_root: codex_root.clone(),
                fallback_limit: 200_000,
            });
            Self {
                claude_root,
                codex_root,
                workspace,
                project_dir,
                reader,
            }
        }

        fn poll_claude(&self, instance_id: &str) -> Option<TranscriptContextReading> {
            self.reader.poll(
                instance_id,
                &self.workspace,
                "claude-code",
                DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH),
            )
        }

        fn poll_codex(&self, instance_id: &str) -> Option<TranscriptContextReading> {
            self.reader.poll(
                instance_id,
                &self.workspace,
                "codex",
                DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH),
            )
        }

        /// A fresh reader with NO accumulated state over the same roots — the
        /// full-rescan oracle every incremental reading must equal.
        fn fresh_full_scan(
            &self,
            instance_id: &str,
            cli_kind: &str,
        ) -> Option<TranscriptContextReading> {
            TranscriptContextReader::new(TranscriptContextConfig {
                claude_projects_root: self.claude_root.clone(),
                codex_sessions_root: self.codex_root.clone(),
                fallback_limit: 200_000,
            })
            .poll(
                instance_id,
                &self.workspace,
                cli_kind,
                DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH),
            )
        }

        fn cleanup(&self) {
            let _ = std::fs::remove_dir_all(&self.claude_root);
            let _ = std::fs::remove_dir_all(&self.codex_root);
            let _ = std::fs::remove_dir_all(&self.workspace);
        }
    }

    /// Pinned test 1: incremental reading over appends equals a fresh reader's
    /// full scan — the invariant every other behavior hangs off.
    #[test]
    fn incremental_append_matches_fresh_reader_full_scan() {
        let setup = IncrementalSetup::new();
        let instance_id = "inst-inc-1";
        let file = setup.project_dir.join("session.jsonl");
        write_jsonl_terminated(
            &file,
            &[
                claude_owner_line(instance_id, &setup.workspace),
                claude_usage_line(50),
            ],
        );
        assert_eq!(setup.poll_claude(instance_id).unwrap().tokens, 50);

        let mut later = claude_usage_line(80);
        later["requestId"] = json!("req-2");
        append_jsonl_terminated(&file, &[later]);
        let incremental = setup.poll_claude(instance_id).expect("reading");
        let fresh = setup
            .fresh_full_scan(instance_id, "claude-code")
            .expect("fresh reading");
        assert_eq!(incremental, fresh, "incremental must equal full rescan");
        assert_eq!(incremental.tokens, 80);
        setup.cleanup();
    }

    /// Pinned test 2 (per ruling f674c8d1): a torn tail is never parsed into
    /// state and never errors; a complete-but-unterminated record counts
    /// transiently (old lines() behavior) without advancing the offset; once
    /// the newline lands it folds into persistent state exactly once.
    #[test]
    fn partial_tail_line_is_overlaid_transiently_and_never_torn() {
        let setup = IncrementalSetup::new();
        let instance_id = "inst-tail-1";
        let file = setup.project_dir.join("session.jsonl");
        write_jsonl_terminated(
            &file,
            &[
                claude_owner_line(instance_id, &setup.workspace),
                claude_usage_line(50),
            ],
        );
        assert_eq!(setup.poll_claude(instance_id).unwrap().tokens, 50);
        let consumed = setup.reader.file_offset(instance_id, &file);
        assert_eq!(consumed, std::fs::metadata(&file).unwrap().len());

        // Torn write: the first half of a record, no newline.
        let mut record = claude_usage_line(80);
        record["requestId"] = json!("req-tail");
        let record_text = record.to_string();
        let (head, rest) = record_text.split_at(record_text.len() / 2);
        append_raw(&file, head);
        let reading = setup
            .poll_claude(instance_id)
            .expect("reading survives torn tail");
        assert_eq!(reading.tokens, 50, "torn tail contributes nothing");
        assert_eq!(
            setup.reader.file_offset(instance_id, &file),
            consumed,
            "torn tail is not consumed"
        );

        // The record completes but the newline has not landed yet: it counts
        // transiently, exactly as the old full scan would have read it, while
        // the offset still refuses to move past an unterminated line.
        append_raw(&file, rest);
        let reading = setup.poll_claude(instance_id).expect("reading");
        assert_eq!(reading.tokens, 80, "flushed record counts this poll");
        assert_eq!(
            setup.reader.file_offset(instance_id, &file),
            consumed,
            "unterminated record is still not consumed"
        );

        // Newline lands: consumed exactly once, reading unchanged and equal to
        // a fresh full rescan.
        append_raw(&file, "\n");
        let reading = setup.poll_claude(instance_id).expect("reading");
        assert_eq!(reading.tokens, 80);
        assert_eq!(
            setup.reader.file_offset(instance_id, &file),
            std::fs::metadata(&file).unwrap().len(),
            "terminated record is consumed"
        );
        assert_eq!(
            reading,
            setup.fresh_full_scan(instance_id, "claude-code").unwrap()
        );
        setup.cleanup();
    }

    /// Pinned test 3: a shrunken file (rotation, or Claude Code rewriting the
    /// session on compaction) resets the state and rescans from zero.
    #[test]
    fn truncated_or_rewritten_file_resets_state_and_matches_full_rescan() {
        let setup = IncrementalSetup::new();
        let instance_id = "inst-trunc-1";
        let file = setup.project_dir.join("session.jsonl");
        let mut second = claude_usage_line(80);
        second["requestId"] = json!("req-2");
        write_jsonl_terminated(
            &file,
            &[
                claude_owner_line(instance_id, &setup.workspace),
                claude_usage_line(50),
                second,
            ],
        );
        assert_eq!(setup.poll_claude(instance_id).unwrap().tokens, 80);

        // Rewrite shorter: new content, fewer bytes than the consumed offset.
        write_jsonl_terminated(
            &file,
            &[
                claude_owner_line(instance_id, &setup.workspace),
                claude_usage_line(30),
            ],
        );
        let reading = setup.poll_claude(instance_id).expect("reading");
        assert_eq!(reading.tokens, 30, "state reset, new content wins");
        assert_eq!(
            reading,
            setup.fresh_full_scan(instance_id, "claude-code").unwrap()
        );
        setup.cleanup();
    }

    /// Pinned test 4: an unchanged file is answered from the cached reduction
    /// without reopening — including through a CLONED reader, which is how the
    /// engine actually calls poll (state must be Arc-shared or the cache is a
    /// mirage).
    #[test]
    fn unchanged_file_short_circuits_without_reopening() {
        let setup = IncrementalSetup::new();
        let instance_id = "inst-idle-1";
        let file = setup.project_dir.join("session.jsonl");
        write_jsonl_terminated(
            &file,
            &[
                claude_owner_line(instance_id, &setup.workspace),
                claude_usage_line(50),
            ],
        );
        let first = setup.poll_claude(instance_id).expect("reading");
        assert_eq!(setup.reader.file_opens(instance_id, &file), 1);

        let clone = setup.reader.clone();
        assert!(
            clone.shares_state_with(&setup.reader),
            "a per-poll clone MUST share scan state, or the cache never hits"
        );
        let second = clone
            .poll(
                instance_id,
                &setup.workspace,
                "claude-code",
                DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH),
            )
            .expect("reading");
        assert_eq!(first, second);
        for reader in [&setup.reader, &clone] {
            assert_eq!(
                reader.file_opens(instance_id, &file),
                1,
                "second poll with no writes must not reopen the file"
            );
        }
        setup.cleanup();
    }

    /// Pinned test 5: ownership seen once stays seen — later appends are read
    /// as a suffix (offset already past the owner marker) and the reading is
    /// still owned by the instance.
    #[test]
    fn saw_instance_stays_owned_across_incremental_appends() {
        let setup = IncrementalSetup::new();
        let instance_id = "inst-mono-1";
        let file = setup.project_dir.join("session.jsonl");
        write_jsonl_terminated(
            &file,
            &[
                claude_owner_line(instance_id, &setup.workspace),
                claude_usage_line(50),
            ],
        );
        assert_eq!(setup.poll_claude(instance_id).unwrap().tokens, 50);
        let consumed = setup.reader.file_offset(instance_id, &file);

        for (req, tokens) in [("req-2", 60), ("req-3", 70)] {
            let mut line = claude_usage_line(tokens);
            line["requestId"] = json!(req);
            append_jsonl_terminated(&file, &[line]);
            let reading = setup.poll_claude(instance_id).expect("still owned");
            assert_eq!(reading.tokens, tokens);
        }
        assert!(
            setup.reader.file_offset(instance_id, &file) > consumed,
            "appends were consumed as a suffix, never rescanned from zero"
        );
        setup.cleanup();
    }

    /// Pinned test 6: choose_newer across files is preserved — the file whose
    /// reading is newest wins, even as per-file results come from caches.
    #[test]
    fn newer_reading_in_second_file_still_wins() {
        let setup = IncrementalSetup::new();
        let instance_id = "inst-multi-1";
        let first = setup.project_dir.join("first.jsonl");
        let second = setup.project_dir.join("second.jsonl");
        write_jsonl_terminated(
            &first,
            &[
                claude_owner_line(instance_id, &setup.workspace),
                claude_usage_line(10),
            ],
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        write_jsonl_terminated(
            &second,
            &[
                claude_owner_line(instance_id, &setup.workspace),
                claude_usage_line(99),
            ],
        );
        assert_eq!(
            setup.poll_claude(instance_id).unwrap().tokens,
            99,
            "newer second file wins"
        );

        // Appending to the FIRST file makes it the newest observation; the
        // cached second-file reading must lose to the refreshed first.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let mut line = claude_usage_line(111);
        line["requestId"] = json!("req-2");
        append_jsonl_terminated(&first, &[line]);
        assert_eq!(
            setup.poll_claude(instance_id).unwrap().tokens,
            111,
            "refreshed first file wins on recency"
        );
        setup.cleanup();
    }

    /// Ruling 3c0182aa: the tail overlay applies the FULL per-line scan logic
    /// transiently — ownership flags included, not just usage. An instance
    /// declared ONLY in the unterminated tail must already own the reading,
    /// equal to a fresh full-scan reader, while the offset still refuses to
    /// consume the unterminated line.
    #[test]
    fn instance_declared_only_in_unterminated_tail_owns_the_reading() {
        let setup = IncrementalSetup::new();
        let instance_id = "inst-tail-owner-1";
        let file = setup.project_dir.join("session.jsonl");
        write_jsonl_terminated(&file, &[claude_usage_line(50)]);
        assert!(
            setup.poll_claude(instance_id).is_none(),
            "no owner marker yet — no reading"
        );
        let consumed = setup.reader.file_offset(instance_id, &file);

        // The owner marker arrives as an unterminated tail line.
        append_raw(
            &file,
            &claude_owner_line(instance_id, &setup.workspace).to_string(),
        );
        let reading = setup
            .poll_claude(instance_id)
            .expect("tail-declared owner must own the reading");
        assert_eq!(reading.tokens, 50);
        assert_eq!(
            reading,
            setup.fresh_full_scan(instance_id, "claude-code").unwrap()
        );
        assert_eq!(
            setup.reader.file_offset(instance_id, &file),
            consumed,
            "the unterminated owner line is still not consumed"
        );

        // Newline lands: ownership folds into persistent state; the reading is
        // unchanged and the line is consumed exactly once.
        append_raw(&file, "\n");
        let reading = setup.poll_claude(instance_id).expect("still owned");
        assert_eq!(reading.tokens, 50);
        assert_eq!(
            setup.reader.file_offset(instance_id, &file),
            std::fs::metadata(&file).unwrap().len()
        );
        setup.cleanup();
    }

    /// Ruling 3c0182aa, model-window variant: a model id carried only by an
    /// unterminated tail record already moves the limit this poll — the
    /// overlay runs the full reduction, not a usage-only one.
    #[test]
    fn model_window_declared_only_in_unterminated_tail_moves_the_limit() {
        let setup = IncrementalSetup::new();
        let instance_id = "inst-tail-model-1";
        let file = setup.project_dir.join("session.jsonl");
        write_jsonl_terminated(
            &file,
            &[
                claude_owner_line(instance_id, &setup.workspace),
                claude_usage_line(50),
            ],
        );
        let reading = setup.poll_claude(instance_id).expect("reading");
        assert_eq!(reading.limit, 200_000, "no model id yet — fallback limit");
        let consumed = setup.reader.file_offset(instance_id, &file);

        append_raw(
            &file,
            &claude_usage_line_with_model("req-tail-model", 80, "claude-fable-5[1m]").to_string(),
        );
        let reading = setup.poll_claude(instance_id).expect("reading");
        assert_eq!(reading.tokens, 80);
        assert_eq!(
            reading.limit, 1_000_000,
            "tail-carried model id must move the window this poll"
        );
        assert_eq!(
            reading,
            setup.fresh_full_scan(instance_id, "claude-code").unwrap()
        );
        assert_eq!(
            setup.reader.file_offset(instance_id, &file),
            consumed,
            "the unterminated record is still not consumed"
        );

        append_raw(&file, "\n");
        let reading = setup.poll_claude(instance_id).expect("reading");
        assert_eq!((reading.tokens, reading.limit), (80, 1_000_000));
        assert_eq!(
            setup.reader.file_offset(instance_id, &file),
            std::fs::metadata(&file).unwrap().len()
        );
        setup.cleanup();
    }

    /// Codex gets the same incremental treatment: appended token counts fold
    /// into the cached reduction and match a fresh full scan.
    #[test]
    fn codex_incremental_append_matches_fresh_reader() {
        let setup = IncrementalSetup::new();
        let instance_id = "inst-codex-inc-1";
        let file = setup.codex_root.join("rollout.jsonl");
        write_jsonl_terminated(
            &file,
            &[
                codex_owner_line(instance_id),
                codex_token_line("2099-01-01T00:00:01Z", 101, 1_000, &setup.workspace),
            ],
        );
        assert_eq!(setup.poll_codex(instance_id).unwrap().tokens, 101);
        assert_eq!(setup.reader.file_opens(instance_id, &file), 1);

        append_jsonl_terminated(
            &file,
            &[codex_token_line(
                "2099-01-01T00:00:02Z",
                202,
                2_000,
                &setup.workspace,
            )],
        );
        let incremental = setup.poll_codex(instance_id).expect("reading");
        let fresh = setup
            .fresh_full_scan(instance_id, "codex")
            .expect("fresh reading");
        assert_eq!(incremental, fresh);
        assert_eq!(incremental.tokens, 202);
        assert_eq!(incremental.limit, 2_000);
        // ...and equals the classic one-shot lines() scan too — old and new
        // semantics agree on a fully terminated file.
        let classic = scan_codex_file(
            &file,
            instance_id,
            &setup.workspace,
            DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH),
            200_000,
        )
        .map(ScannedReading::into_reading)
        .expect("classic reading");
        assert_eq!(incremental, classic);

        // Idle codex poll short-circuits too.
        let opens = setup.reader.file_opens(instance_id, &file);
        let _ = setup.poll_codex(instance_id);
        assert_eq!(setup.reader.file_opens(instance_id, &file), opens);
        setup.cleanup();
    }
}
