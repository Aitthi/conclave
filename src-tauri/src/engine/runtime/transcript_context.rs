//! Transcript-backed context meter for CLI agents.
//!
//! This module is deliberately narrow:
//! - It discovers candidate Claude Code and Codex transcript files.
//! - It matches them to a specific agent instance using workspace cwd plus the
//!   bootstrap owner marker declaring the agent's own instance id. The marker
//!   counts only where the harness itself writes it — the session-bootstrap
//!   channel (`claude_value_declares_owner` / `codex_value_declares_owner`) —
//!   never wherever the phrase merely appears, because agents quote each
//!   other's briefings and all of a workspace's transcripts share one dir.
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
    /// When the usage was RECORDED — the timestamp on the transcript row the
    /// numbers came from, not when the file was last written to.
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

/// Pick the more recent of two per-file readings. Every workspace agent shares
/// one cwd, so one Claude project dir holds many transcripts and the poll folds
/// this across all of them. Recency is the reading's `observed_at`, i.e. the
/// timestamp of the usage row it came from — NOT the file's mtime, which a
/// metadata-only append to a closed session can bump arbitrarily far ahead.
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
pub(crate) fn claude_project_dir(root: &Path, workspace_folder: &Path) -> PathBuf {
    let slug: String = workspace_folder
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    root.join(slug)
}

/// Collect `.jsonl` files under `root` that were modified at or after
/// `min_mtime`. Shared with the usage importer (`transcript_usage`) together
/// with the owner-marker and project-dir helpers — the importer reuses ONLY
/// these validated discovery/ownership rules, never this reader's scan state. This is a cheap `stat`-only PRE-FILTER — before any file is
/// opened or parsed — that keeps the meter off the (potentially many GB of)
/// historical transcripts a full parse would otherwise churn every poll. A
/// file whose newest usage row is at or after the anchor necessarily has an
/// mtime at or after it too, so nothing admissible is ever dropped here.
///
/// It deliberately OVER-admits: Claude Code appends non-usage metadata to
/// closed session files hours after their last usage row, so a bumped mtime
/// proves nothing about the usage inside. Admissibility is decided by the
/// usage row's own timestamp in [`ClaudeAcc::finalize`] (and
/// [`CodexAcc::ingest_line`] for rollouts).
pub(crate) fn collect_jsonl_files(root: &Path, min_mtime: DateTime<Utc>) -> Vec<PathBuf> {
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
/// lost. A single `(line_no, tokens, observed_at)` scalar is the same
/// reduction, O(1) per poll instead of an unbounded map held per file all
/// session.
///
/// The third element is the winning usage row's OWN top-level `timestamp`.
/// It is what dates the reading — see [`ClaudeAcc::finalize`] for why the
/// file's mtime cannot.
#[derive(Debug, Default, Clone)]
struct ClaudeAcc {
    line_no: usize,
    saw_workspace: bool,
    saw_instance: bool,
    last_model_window: Option<i64>,
    latest_usage: Option<(usize, i64, Option<DateTime<Utc>>)>,
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
        // The row's own `timestamp` (RFC3339 with `Z`, millisecond precision —
        // e.g. `2026-08-31T15:23:04.140Z`) dates the usage itself, mirroring
        // what the codex path already reads off its token_count events.
        let row_observed_at = value
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_ts);
        self.latest_usage = Some((self.line_no, sum_claude_usage(usage), row_observed_at));
    }

    /// A reading is dated by the winning usage ROW, never by the file's mtime.
    ///
    /// Claude Code keeps appending non-usage metadata (`agent-name`,
    /// `last-prompt`, `cost-state`, `system`/`bridge_status`, summary lines) to
    /// CLOSED session files, sometimes hours after the last assistant response
    /// — a live 2026-08-31 file had its final usage row at 15:23:04.140Z and an
    /// mtime of 19:51:32Z. An mtime-dated reading therefore drifted past the
    /// current generation's `started_at`, the guard below stopped rejecting the
    /// dead generation, and its full accumulated usage won `choose_newer`
    /// whenever the live transcript was momentarily idle — the repeating
    /// fixed-percentage context nudge reported from the lokal-llm workspace.
    /// Anchored to the row, that rejection is permanent.
    ///
    /// The file's mtime remains the fallback for a usage row that carries no
    /// parseable `timestamp` (legacy or unknown shapes); every row observed in
    /// live transcripts has one.
    fn finalize(
        &self,
        path: &Path,
        started_at: DateTime<Utc>,
        fallback_limit: i64,
    ) -> Option<ScannedReading> {
        if !self.saw_workspace || !self.saw_instance {
            return None;
        }
        let (_, tokens, row_observed_at) = self.latest_usage?;
        let observed_at = match row_observed_at {
            Some(at) => at,
            None => file_modified_at(path)?,
        };
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

/// EVERY agent id a marker text declares, whoever it names: each token after
/// an `own agent id is`, lower-cased like the ids it is compared with. One
/// text can carry several declarations (ruling 1db7827f); stopping at the
/// first would let a second owner hide behind the first.
fn declared_agent_ids(text: &str) -> Vec<String> {
    const PHRASE: &str = "own agent id is ";
    let lower = text.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(found) = lower[from..].find(PHRASE) {
        let start = from + found + PHRASE.len();
        let id: String = lower[start..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if !id.is_empty() {
            out.push(id);
        }
        from = start;
    }
    out
}

/// Every agent id a Claude SessionStart hook attachment structurally
/// declares — registered with the workspace or not. Attribution needs the
/// UNKNOWN owner as much as the known one (usage review a12f77f2 C3): a
/// marker naming an agent the workspace cannot place is an ownership
/// conflict, never silence. Same structure gate as
/// [`claude_value_declares_owner`].
pub(crate) fn claude_value_declared_owners(value: &Value) -> Vec<String> {
    if value.get("type").and_then(Value::as_str) != Some("attachment") {
        return Vec::new();
    }
    let Some(attachment) = value.get("attachment") else {
        return Vec::new();
    };
    if !is_session_start_hook(attachment) {
        return Vec::new();
    }
    session_start_hook_texts(attachment)
        .iter()
        .flat_map(|text| declared_agent_ids(text))
        .collect()
}

/// Every agent id a Codex developer message structurally declares; see
/// [`claude_value_declared_owners`].
pub(crate) fn codex_value_declared_owners(value: &Value) -> Vec<String> {
    if value.get("type").and_then(Value::as_str) != Some("response_item") {
        return Vec::new();
    }
    let Some(payload) = value.get("payload") else {
        return Vec::new();
    };
    if payload.get("type").and_then(Value::as_str) != Some("message")
        || payload.get("role").and_then(Value::as_str) != Some("developer")
    {
        return Vec::new();
    }
    payload
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("input_text"))
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .flat_map(declared_agent_ids)
        .collect()
}

/// Ownership is bound to the ONE channel Conclave actually writes the marker
/// on: the SessionStart hook (`runtime::sandbox_config::owner_marker_command`),
/// which claude-code records as an `attachment` line on every session start —
/// startup, resume, `/clear`, compact. The system-prompt append carries the
/// same sentence but never reaches the transcript.
///
/// Anything else that CONTAINS the phrase is an echo, not a declaration. Every
/// workspace agent shares one cwd, so one project dir holds every agent's
/// transcript and `poll_claude` ranks them by recency — accepting the phrase
/// from conversational text let a busy agent's transcript claim ownership of
/// any peer whose briefing it happened to quote (a `ps eww` dump, a forwarded
/// `conclave tell`, a pasted sidecar) and serve that peer's meter its own
/// tokens. Structure, not content, decides: see
/// `peer_briefing_echo_never_declares_ownership`.
///
/// Verified against 689 live transcripts (2026-08-16): all 518 marker
/// declarations arrive as SessionStart hook attachments
/// (`hook_additional_context` / `hook_success`), none from a user or system
/// line — so this restriction loses no real declaration.
pub(crate) fn claude_value_declares_owner(value: &Value, instance_id: &str) -> bool {
    if value.get("type").and_then(Value::as_str) != Some("attachment") {
        return false;
    }
    let Some(attachment) = value.get("attachment") else {
        return false;
    };
    if !is_session_start_hook(attachment) {
        return false;
    }
    session_start_hook_texts(attachment)
        .iter()
        .any(|text| text_declares_own_agent_id(text, instance_id))
}

/// Is this attachment a SessionStart hook record? claude-code stamps
/// `hookEvent: "SessionStart"` and a `hookName` of `SessionStart`,
/// `SessionStart:startup` or `SessionStart:compact` (all three observed live);
/// either field is accepted so a version that drops one still binds.
fn is_session_start_hook(attachment: &Value) -> bool {
    const SESSION_START: &str = "SessionStart";
    if attachment.get("hookEvent").and_then(Value::as_str) == Some(SESSION_START) {
        return true;
    }
    attachment
        .get("hookName")
        .and_then(Value::as_str)
        .is_some_and(|name| name.starts_with(SESSION_START))
}

pub(crate) fn codex_value_declares_owner(value: &Value, instance_id: &str) -> bool {
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

/// The text a SessionStart hook attachment carries the owner marker in. Real
/// transcripts deliver it two ways (verified against live files):
/// - `attachment.content` — a LIST of strings, how claude-code records a
///   hook's `additionalContext` (attachment type `hook_additional_context`).
/// - `attachment.stdout` — raw hook stdout, which for our hook is the
///   `hookSpecificOutput` JSON (attachment type `hook_success`).
///
/// Deliberately NOT the hook's `command` field: the marker command's own text
/// is an argument we could read back before the hook ever ran, and the field
/// exists on every hook record, ours or foreign.
fn session_start_hook_texts(attachment: &Value) -> Vec<&str> {
    let mut out = Vec::new();
    push_content_texts(attachment.get("content"), &mut out);
    if let Some(stdout) = attachment.get("stdout").and_then(Value::as_str) {
        out.push(stdout);
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

pub(crate) fn file_modified_at(path: &Path) -> Option<DateTime<Utc>> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    Some(DateTime::<Utc>::from(modified))
}

pub(crate) fn parse_ts(text: &str) -> Option<DateTime<Utc>> {
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

    #[test]
    fn antigravity_is_unsupported_and_never_scans_opaque_conversations() {
        let root = tmp_root("antigravity-unsupported");
        let opaque = root.join("conversation.pb");
        std::fs::write(&opaque, [0_u8, 1, 2, 3]).unwrap();
        let reader = TranscriptContextReader::new(TranscriptContextConfig {
            claude_projects_root: root.join("claude"),
            codex_sessions_root: root.clone(),
            fallback_limit: 200_000,
        });

        let reading = reader.poll("agent-1", &root, "antigravity", Utc::now());

        assert!(reading.is_none());
        assert!(
            reader.scan_state.lock().unwrap().is_empty(),
            "unsupported harnesses must not open or cache conversation files"
        );
        let _ = std::fs::remove_dir_all(root);
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

    /// The owner marker exactly as claude-code records it: the SessionStart
    /// hook's `additionalContext` as a `hook_additional_context` attachment
    /// whose `content` is a LIST of context strings (copied from a live
    /// transcript, 2026-08-16). This is the ONLY channel that declares
    /// ownership — every fixture that just needs "this file belongs to X"
    /// must use it, or it pins a shape production never writes.
    fn claude_owner_line(instance_id: &str, workspace: &Path) -> Value {
        json!({
            "type": "attachment",
            "cwd": workspace.to_string_lossy(),
            "attachment": {
                "type": "hook_additional_context",
                "hookName": "SessionStart",
                "toolUseID": "SessionStart",
                "hookEvent": "SessionStart",
                "content": [
                    format!("You are a Conclave agent, and your own agent id is {instance_id}.")
                ]
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

    /// The inverse of the hook tests, and the reason this lane exists: a user
    /// turn is CONVERSATION, and conversation quotes things. Whether the marker
    /// arrives as a plain string or as an array of typed blocks, a user line
    /// must never declare ownership — otherwise quoting a peer's briefing
    /// hands that peer's meter this transcript's tokens.
    ///
    /// This replaces `claude_owner_via_user_message_content_blocks`, which
    /// pinned the opposite behaviour on a hypothetical "marker typed as a user
    /// prompt" channel. No such channel exists: Conclave writes the marker only
    /// through the SessionStart hook (`sandbox_config::owner_marker_command`),
    /// and 689 live transcripts contain zero user-line declarations.
    #[test]
    fn claude_user_message_marker_does_not_declare_owner() {
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
                json!({
                    "type": "user",
                    "cwd": workspace.to_string_lossy(),
                    "message": {
                        "role": "user",
                        "content": format!("...and again: your own agent id is {instance_id}.")
                    }
                }),
                claude_usage_line(44),
            ],
        );

        assert!(
            scan_claude_file(
                &file,
                instance_id,
                &workspace,
                DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH),
                200_000,
            )
            .is_none(),
            "a user-turn marker is quotable content, never an ownership declaration"
        );

        let _ = std::fs::remove_dir_all(&claude_root);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    /// A hook attachment that is NOT the SessionStart hook must not declare
    /// ownership either: hook records carry the wrapped tool's own text
    /// (`stdout`, `command`), so a PreToolUse hook wrapping `ps eww` would
    /// otherwise become a second echo channel. Only the session-bootstrap hook
    /// speaks for the instance.
    #[test]
    fn claude_non_session_start_hook_does_not_declare_owner() {
        let claude_root = tmp_root("claude-root");
        let workspace = tmp_root("workspace");
        let instance_id = "inst-pretool-1";
        let file = claude_root.join("session.jsonl");

        write_jsonl(
            &file,
            &[
                json!({
                    "type": "attachment",
                    "cwd": workspace.to_string_lossy(),
                    "attachment": {
                        "type": "hook_success",
                        "hookName": "PreToolUse:Bash",
                        "hookEvent": "PreToolUse",
                        "content": "",
                        "stdout": format!(
                            "74400 claude --append-system-prompt You are a Conclave agent, and your own agent id is {instance_id}.\n"
                        ),
                        "stderr": "",
                        "exitCode": 0
                    }
                }),
                claude_usage_line(45),
            ],
        );

        assert!(
            scan_claude_file(
                &file,
                instance_id,
                &workspace,
                DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH),
                200_000,
            )
            .is_none(),
            "only the SessionStart hook declares ownership"
        );

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
                claude_owner_line(instance_id, &workspace),
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
                claude_owner_line(instance_id, &workspace),
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
                claude_owner_line(instance_id, &workspace),
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
                claude_owner_line(instance_id, &workspace),
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

    /// Force `path`'s mtime far into the future.
    ///
    /// Readings are dated by the winning usage ROW, so for a fixture whose
    /// usage rows carry a `timestamp` this steers only the `collect_jsonl_files`
    /// pre-filter — and is how the metadata-append incident is reproduced. For
    /// a legacy fixture built from timestamp-less `claude_usage_line`s the
    /// reading falls back to the mtime, so this still gives `choose_newer` a
    /// deterministic winner instead of leaving it to same-second timestamps and
    /// readdir order.
    fn bump_mtime(path: &Path, secs_ahead: u64) {
        let when = std::time::SystemTime::now() + std::time::Duration::from_secs(secs_ahead);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open for set_times");
        file.set_times(std::fs::FileTimes::new().set_modified(when))
            .expect("set mtime");
    }

    /// Incident regression (task ctxmeter-ownership-leak, human-reported):
    /// every workspace agent shares one cwd, so ONE Claude project dir holds
    /// every agent's transcript and `poll_claude` takes `choose_newer` across
    /// all of them. Ownership therefore has to be un-forgeable by CONTENT: a
    /// busy agent's transcript routinely quotes a peer's launch briefing —
    /// `ps eww`/`pgrep -fl claude` dumps, a forwarded `conclave tell`, a
    /// pasted sidecar — and the marker phrase rides along verbatim.
    ///
    /// If any of those echoes could declare ownership, the busiest file wins
    /// `choose_newer` for the peer it echoed and an IDLE agent's meter shows
    /// the WORKING agent's tokens (the sighting this test exists to make
    /// impossible). Both echo shapes below are real: the plain-string user
    /// prompt is the channel the reader actually read before this fix; the
    /// `tool_result` block is the live 2026-08-16 `ps eww` dump.
    #[test]
    fn peer_briefing_echo_never_declares_ownership() {
        let claude_root = tmp_root("claude-root");
        let codex_root = tmp_root("codex-root");
        let workspace = tmp_root("workspace");
        let busy = "inst-busy-lead";
        let idle = "inst-idle-peer";

        let project_dir = claude_project_dir(&claude_root, &workspace);
        std::fs::create_dir_all(&project_dir).expect("create project dir");

        // The busy agent's own transcript: legitimately owned by `busy`, and
        // echoing the IDLE peer's briefing twice.
        let busy_file = project_dir.join("busy.jsonl");
        write_jsonl(
            &busy_file,
            &[
                claude_owner_line(busy, &workspace),
                // Echo #1 — a plain-string user prompt (peer message, paste).
                json!({
                    "type": "user",
                    "cwd": workspace.to_string_lossy(),
                    "message": {
                        "role": "user",
                        "content": format!(
                            "roster: 74400 claude --append-system-prompt You are a Conclave agent, and your own agent id is {idle}."
                        )
                    }
                }),
                // Echo #2 — a `ps eww` dump landing as a tool_result block.
                json!({
                    "type": "user",
                    "cwd": workspace.to_string_lossy(),
                    "message": {
                        "role": "user",
                        "content": [
                            {
                                "tool_use_id": "toolu_echo",
                                "type": "tool_result",
                                "content": format!(
                                    "74400 claude --append-system-prompt You are a Conclave agent, and your own agent id is {idle}."
                                ),
                                "is_error": false
                            }
                        ]
                    }
                }),
                claude_usage_line(900_000),
            ],
        );

        // The idle peer's own transcript: quiet, and (mtime) older.
        let idle_file = project_dir.join("idle.jsonl");
        write_jsonl(
            &idle_file,
            &[
                claude_owner_line(idle, &workspace),
                claude_usage_line(1_234),
            ],
        );

        // The busy file is the NEWEST, so any leaked ownership wins outright.
        bump_mtime(&busy_file, 3_600);

        let reader = TranscriptContextReader::new(TranscriptContextConfig {
            claude_projects_root: claude_root.clone(),
            codex_sessions_root: codex_root.clone(),
            fallback_limit: 200_000,
        });
        let epoch = DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH);

        let idle_reading = reader
            .poll(idle, &workspace, "claude-code", epoch)
            .expect("the idle agent's own transcript must still be readable");
        assert_eq!(
            idle_reading.tokens, 1_234,
            "an echoed briefing must not hand the busy agent's tokens to the idle peer's meter"
        );

        let busy_reading = reader
            .poll(busy, &workspace, "claude-code", epoch)
            .expect("the busy agent must still own its own transcript");
        assert_eq!(busy_reading.tokens, 900_000);

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
    ///
    /// Recency is the USAGE ROW's timestamp, so the fixtures carry the
    /// production row shape and order the two files explicitly (the previous
    /// sleep-and-write-order steering dated readings by file mtime, which
    /// production no longer does).
    #[test]
    fn newer_reading_in_second_file_still_wins() {
        let setup = IncrementalSetup::new();
        let instance_id = "inst-multi-1";
        let now = Utc::now();
        let first = setup.project_dir.join("first.jsonl");
        let second = setup.project_dir.join("second.jsonl");
        write_jsonl_terminated(
            &first,
            &[
                claude_owner_line(instance_id, &setup.workspace),
                claude_usage_line_at("req-first-1", 10, now - chrono::Duration::seconds(20)),
            ],
        );
        write_jsonl_terminated(
            &second,
            &[
                claude_owner_line(instance_id, &setup.workspace),
                claude_usage_line_at("req-second-1", 99, now - chrono::Duration::seconds(10)),
            ],
        );
        assert_eq!(
            setup.poll_claude(instance_id).unwrap().tokens,
            99,
            "newer second file wins"
        );

        // Appending to the FIRST file makes it the newest observation; the
        // cached second-file reading must lose to the refreshed first.
        append_jsonl_terminated(&first, &[claude_usage_line_at("req-first-2", 111, now)]);
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

    /// An assistant usage line carrying the row's OWN top-level `timestamp`,
    /// exactly as claude-code stamps every assistant record (RFC3339 with `Z`,
    /// millisecond precision — live sample `2026-08-31T15:23:04.140Z`).
    /// Fixtures that need to express *when the usage happened*, as opposed to
    /// when the file was last touched, must use this shape;
    /// `claude_usage_line` deliberately keeps the timestamp-less legacy shape
    /// that exercises the mtime fallback.
    fn claude_usage_line_at(req: &str, tokens: i64, timestamp: DateTime<Utc>) -> Value {
        let mut line = claude_usage_line(tokens);
        line["requestId"] = json!(req);
        line["message"]["id"] = json!(req);
        line["timestamp"] = json!(timestamp.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
        line
    }

    /// A non-usage metadata record of the shape claude-code appends to a
    /// CLOSED session file long after its last usage row (live tail,
    /// 2026-08-31: a `system` / `bridge_status` line, alongside `agent-name`,
    /// `last-prompt` and `cost-state` records). It contributes no usage, but
    /// writing it bumps the file's mtime.
    fn claude_bridge_status_line(timestamp: DateTime<Utc>) -> Value {
        json!({
            "type": "system",
            "subtype": "bridge_status",
            "timestamp": timestamp.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "content": "bridge status changed"
        })
    }

    /// Incident regression (cross-workspace report, 2026-08-31): after an
    /// agent's first `conclave restart` the `[conclave context]` nudge fired
    /// four times with a FIXED stale 70–71% while the live meter read 25–31%.
    ///
    /// Root cause: claude-code appends non-usage metadata (`agent-name`,
    /// `last-prompt`, `cost-state`, `system`/`bridge_status`) to a CLOSED
    /// session file hours after its last usage row — the observed file's last
    /// usage row was 15:23:04.140Z but its mtime was 19:51:32Z. While a
    /// reading's `observed_at` came from the FILE MTIME, that append made the
    /// dead generation admissible again (`observed_at < started_at` stopped
    /// rejecting it) and its full accumulated usage won `choose_newer` over
    /// the quiet live transcript, re-persisting a closed generation's tokens.
    ///
    /// Anchoring `observed_at` to the usage ROW makes the rejection permanent:
    /// no number of metadata appends can move a closed generation's last usage
    /// row past the new generation's anchor.
    #[test]
    fn closed_generation_metadata_append_never_resurrects_its_usage() {
        let claude_root = tmp_root("claude-root");
        let codex_root = tmp_root("codex-root");
        let workspace = tmp_root("workspace");
        let instance_id = "inst-restart-1";
        let project_dir = claude_project_dir(&claude_root, &workspace);
        std::fs::create_dir_all(&project_dir).expect("create project dir");

        let now = Utc::now();
        // The generation that ended BEFORE the restart...
        let closed_usage_at = now - chrono::Duration::hours(4);
        // ...and the anchor stamped just before the respawn.
        let started_at = now - chrono::Duration::hours(1);

        let closed = project_dir.join("closed-generation.jsonl");
        write_jsonl(
            &closed,
            &[
                claude_owner_line(instance_id, &workspace),
                claude_usage_line_at(
                    "req-old-1",
                    700_000,
                    closed_usage_at - chrono::Duration::minutes(5),
                ),
                claude_usage_line_at("req-old-2", 710_000, closed_usage_at),
                // Metadata-only tail, written hours after the session closed.
                claude_bridge_status_line(now),
            ],
        );
        // That append lands well after the new generation's anchor, so the
        // cheap `collect_jsonl_files` mtime pre-filter still admits the file —
        // the rejection has to happen in `finalize`.
        bump_mtime(&closed, 3_600);

        let reader = TranscriptContextReader::new(TranscriptContextConfig {
            claude_projects_root: claude_root.clone(),
            codex_sessions_root: codex_root.clone(),
            fallback_limit: 200_000,
        });

        assert_eq!(
            reader.poll(instance_id, &workspace, "claude-code", started_at),
            None,
            "a closed generation's usage must stay rejected however often a \
             metadata append bumps the file's mtime"
        );

        // The live generation, mid-turn and far below the stale reading.
        let live = project_dir.join("live-generation.jsonl");
        write_jsonl(
            &live,
            &[
                claude_owner_line(instance_id, &workspace),
                claude_usage_line_at("req-live-1", 62_000, now),
            ],
        );
        let reading = reader
            .poll(instance_id, &workspace, "claude-code", started_at)
            .expect("the live generation is owned and admissible");
        assert_eq!(
            reading.tokens, 62_000,
            "the meter must report the LIVE generation, not the closed one the \
             metadata append kept on disk"
        );

        let _ = std::fs::remove_dir_all(&claude_root);
        let _ = std::fs::remove_dir_all(&codex_root);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    /// `choose_newer` ranks claude readings by USAGE recency, not by file
    /// mtime: the file whose last usage row is older loses even when something
    /// touched it afterwards. Before the entry-derived `observed_at`, a
    /// metadata-only append was enough to hand the stale file the win.
    #[test]
    fn choose_newer_ranks_by_usage_row_timestamp_not_file_mtime() {
        let claude_root = tmp_root("claude-root");
        let codex_root = tmp_root("codex-root");
        let workspace = tmp_root("workspace");
        let instance_id = "inst-recency-1";
        let project_dir = claude_project_dir(&claude_root, &workspace);
        std::fs::create_dir_all(&project_dir).expect("create project dir");

        let now = Utc::now();
        let fresher = project_dir.join("fresher-usage.jsonl");
        write_jsonl(
            &fresher,
            &[
                claude_owner_line(instance_id, &workspace),
                claude_usage_line_at("req-fresh", 111, now - chrono::Duration::minutes(1)),
            ],
        );
        let touched = project_dir.join("older-usage-newer-mtime.jsonl");
        write_jsonl(
            &touched,
            &[
                claude_owner_line(instance_id, &workspace),
                claude_usage_line_at("req-stale", 222, now - chrono::Duration::hours(3)),
                claude_bridge_status_line(now),
            ],
        );
        // The stale file is by far the newest on disk.
        bump_mtime(&touched, 3_600);

        let reader = TranscriptContextReader::new(TranscriptContextConfig {
            claude_projects_root: claude_root.clone(),
            codex_sessions_root: codex_root.clone(),
            fallback_limit: 200_000,
        });
        let epoch = DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH);

        let reading = reader
            .poll(instance_id, &workspace, "claude-code", epoch)
            .expect("both files are owned and admissible");
        assert_eq!(
            reading.tokens, 111,
            "the file with the LATER usage row must win, even though the other \
             file carries the newer mtime"
        );

        let _ = std::fs::remove_dir_all(&claude_root);
        let _ = std::fs::remove_dir_all(&codex_root);
        let _ = std::fs::remove_dir_all(&workspace);
    }

    /// Legacy fallback: a usage row with no top-level `timestamp` keeps the
    /// pre-fix semantics — `observed_at` comes from the file's mtime — so an
    /// unknown or older row shape still produces a reading instead of dropping
    /// silently out of the meter.
    #[test]
    fn usage_row_without_timestamp_falls_back_to_file_mtime() {
        let claude_root = tmp_root("claude-root");
        let workspace = tmp_root("workspace");
        let instance_id = "inst-legacy-1";
        let project_dir = claude_project_dir(&claude_root, &workspace);
        std::fs::create_dir_all(&project_dir).expect("create project dir");

        let file = project_dir.join("legacy.jsonl");
        write_jsonl(
            &file,
            &[
                claude_owner_line(instance_id, &workspace),
                claude_usage_line(4_242),
            ],
        );

        let reading = scan_claude_file(
            &file,
            instance_id,
            &workspace,
            DateTime::<Utc>::from(std::time::SystemTime::UNIX_EPOCH),
            200_000,
        )
        .expect("a timestamp-less usage row still reads");
        assert_eq!(reading.tokens, 4_242);
        assert_eq!(
            reading.observed_at,
            file_modified_at(&file).expect("file mtime"),
            "with no row timestamp the reading falls back to the file's mtime"
        );

        let _ = std::fs::remove_dir_all(&claude_root);
        let _ = std::fs::remove_dir_all(&workspace);
    }
}
