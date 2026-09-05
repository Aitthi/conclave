//! Transcript USAGE importer — the metadata-only reader that turns Claude Code
//! and Codex transcript files into `model_usage_event` rows
//! (docs/plans/2026-09-05-usage-engine.md, evidence
//! docs/research/2026-09-05-usage-transcript-evidence.md).
//!
//! This is NOT the context gauge. `transcript_context` reduces a file to one
//! latest reading every two seconds and must stay that way; this module keeps
//! its own cursor per file and emits one event per completed model response.
//! The two share only the validated discovery and ownership helpers
//! (`claude_project_dir`, `collect_jsonl_files`, `*_value_declares_owner`).
//!
//! # What this file owns
//!
//! * [`read_batch`] — bounded, complete-line-only reading from a byte offset.
//! * [`scan_claude_lines`] / [`scan_codex_lines`] — pure parsers from JSONL
//!   lines to [`ImportedEvent`]s plus bounded diagnostics.
//! * [`apply_scan`] — events, conflict marks and the cursor advance in ONE
//!   transaction, so a crash between them is impossible and a replay from the
//!   previous offset is a no-op through `event_key`.
//!
//! The worker that schedules files, verifies ownership, records coverage and
//! runs all of this off the async runtime lives in `transcript_worker`.
//!
//! # Truth rules encoded here
//!
//! * A row is imported only with its full identity (session, request/response,
//!   message id, source timestamp, terminal stop reason). Anything less is a
//!   diagnostic, never a guessed event.
//! * Claude's cache-inclusive input is `input + cache_creation + cache_read`
//!   and needs all three; Codex `input_tokens` is already cache-inclusive and
//!   `output_tokens` already includes reasoning. Subsets are stored as
//!   subsets, never re-added.
//! * A repeated Claude row that DISAGREES with what was recorded for its
//!   request marks the event `conflict`: still one activity, no measured
//!   tokens, partial coverage. It never creates a second activity.
//! * Only a TOP-LEVEL Codex `token_usage_record` is an event. `token_count`,
//!   cumulative counters and a `compacted` record's embedded copy are ignored,
//!   and a file that has only the old `token_count` shape is reported as an
//!   unsupported source — never fabricated into rows.
//! * No transcript text is retained: parser continuation state is a bounded
//!   `turn_id → model` map, and diagnostics are codes with counts.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::transcript_context::parse_ts;
use super::usage::{
    counter, MeasuredUsage, COLLECTOR_VERSION, SOURCE_CLAUDE_TRANSCRIPT, SOURCE_CODEX_TRANSCRIPT,
};
use crate::engine::repo::model_usage::{self, CursorRow, NewUsageEvent, UNSUPPORTED_SOURCE};

// ── Bounds ───────────────────────────────────────────────────────────────────

/// Bytes read from one file in one batch under normal conditions.
pub const BATCH_BYTES: usize = 256 * 1024;
/// Largest single source record this importer will hold in memory. A longer
/// record is skipped through its newline with a diagnostic — never re-read
/// forever, never logged.
pub const MAX_RECORD_BYTES: usize = 4 * 1024 * 1024;
/// How many `turn_id → model` pairs Codex parser state keeps across batches.
const TURN_MAP_CAP: usize = 8;

// ── Diagnostics ──────────────────────────────────────────────────────────────

/// Bounded, code-only diagnostics a scan may raise. Each makes the scope's
/// coverage partial; none carries transcript text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Diagnostic {
    /// A record longer than [`MAX_RECORD_BYTES`] was skipped.
    OversizedRecord,
    /// A line was not valid JSON (torn write, foreign file).
    UnparsableRecord,
    /// A Claude assistant row lacked part of its identity or usage shape.
    ClaudeRowIncomplete,
    /// A repeated Claude row disagreed with the recorded response.
    ClaudeConflict,
    /// A Codex usage record lacked its session or response id.
    CodexRecordIncomplete,
    /// A Codex file with only the pre-`token_usage_record` shape.
    CodexUnsupportedFormat,
}

impl Diagnostic {
    pub fn code(self) -> &'static str {
        match self {
            Diagnostic::OversizedRecord => "oversized_record",
            Diagnostic::UnparsableRecord => "unparsable_record",
            Diagnostic::ClaudeRowIncomplete => "claude_row_incomplete",
            Diagnostic::ClaudeConflict => "claude_group_disagrees",
            Diagnostic::CodexRecordIncomplete => "codex_record_incomplete",
            Diagnostic::CodexUnsupportedFormat => UNSUPPORTED_SOURCE,
        }
    }
}

// ── Reading ──────────────────────────────────────────────────────────────────

/// One bounded read from a file: the complete lines it yielded and where the
/// cursor may advance to.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Batch {
    /// Complete, newline-terminated lines (without the newline), in order.
    pub lines: Vec<String>,
    /// The offset just past the last complete line consumed — the ONLY value
    /// a cursor may advance to. A partial tail is never consumed.
    pub next_offset: u64,
    /// The file length observed at read time.
    pub observed_length: u64,
    /// Whether an oversized record was skipped inside this batch.
    pub skipped_oversized: bool,
    /// Whether unread bytes remain past `next_offset`.
    pub has_backlog: bool,
}

/// Read complete lines starting at `offset`.
///
/// Reads [`BATCH_BYTES`] and returns every newline-terminated line inside it.
/// A record that spans the batch boundary is read further, up to
/// [`MAX_RECORD_BYTES`], so a long-but-legal record still parses whole; past
/// that it is skipped through its newline (or to EOF) with
/// `skipped_oversized`. A partial tail without a newline yet (writer
/// mid-append) is left unconsumed for the next batch.
pub fn read_batch(path: &Path, offset: u64) -> std::io::Result<Batch> {
    let mut file = File::open(path)?;
    let observed_length = file.metadata()?.len();
    let mut batch = Batch {
        observed_length,
        next_offset: offset,
        ..Batch::default()
    };
    if offset >= observed_length {
        return Ok(batch);
    }
    file.seek(SeekFrom::Start(offset))?;

    let mut buf = vec![0u8; BATCH_BYTES];
    let mut pending: Vec<u8> = Vec::new();
    let mut consumed: u64 = offset;
    let mut budget = BATCH_BYTES;

    loop {
        let want = budget.min(buf.len());
        if want == 0 {
            break;
        }
        let read = file.read(&mut buf[..want])?;
        if read == 0 {
            break;
        }
        budget -= read;
        pending.extend_from_slice(&buf[..read]);

        // Drain every complete line.
        while let Some(nl) = pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = pending.drain(..=nl).collect();
            consumed += line.len() as u64;
            batch.next_offset = consumed;
            let text = String::from_utf8_lossy(&line[..line.len() - 1]);
            batch.lines.push(text.trim_end_matches('\r').to_owned());
        }

        // A record longer than the whole batch: keep reading it, bounded.
        if !pending.is_empty() && budget == 0 {
            if pending.len() >= MAX_RECORD_BYTES {
                // Skip through the record's newline without holding it.
                let skipped = skip_to_newline(&mut file, &mut pending)?;
                consumed += skipped;
                batch.next_offset = consumed;
                batch.skipped_oversized = true;
                budget = 0;
                continue;
            }
            // Extend the budget for this one record only.
            budget = MAX_RECORD_BYTES - pending.len();
        }
    }

    batch.has_backlog = batch.next_offset < observed_length;
    Ok(batch)
}

/// Discard `pending` and read forward until the next newline (or EOF),
/// returning how many bytes were consumed in total. Memory stays bounded to
/// one chunk.
fn skip_to_newline(file: &mut File, pending: &mut Vec<u8>) -> std::io::Result<u64> {
    let mut consumed = pending.len() as u64;
    pending.clear();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            return Ok(consumed);
        }
        if let Some(nl) = chunk[..read].iter().position(|&b| b == b'\n') {
            consumed += (nl + 1) as u64;
            // Rewind to just after the newline so the next line is not lost.
            let overshoot = (read - nl - 1) as i64;
            if overshoot > 0 {
                file.seek(SeekFrom::Current(-overshoot))?;
            }
            return Ok(consumed);
        }
        consumed += read as u64;
    }
}

// ── Scope and events ─────────────────────────────────────────────────────────

/// Who a file's events belong to, as verified by the worker.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attribution {
    pub workspace_id: Option<String>,
    /// Set only after the owner marker was seen for this agent.
    pub workspace_agent_id: Option<String>,
    pub session_id: Option<String>,
}

/// An event produced by a scan, before its storage id is minted.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedEvent {
    pub event_key: String,
    pub source_kind: &'static str,
    pub source_session_id: String,
    pub source_request_id: Option<String>,
    pub source_response_id: Option<String>,
    pub occurred_at: DateTime<Utc>,
    pub provider: &'static str,
    pub requested_model: Option<String>,
    pub served_model: Option<String>,
    pub usage: MeasuredUsage,
}

impl ImportedEvent {
    /// The storage row, attributed. `generation` stays `None`: a historical
    /// record has no launch interval proving which generation produced it.
    pub fn into_row(self, attribution: &Attribution, recorded_at: DateTime<Utc>) -> NewUsageEvent {
        NewUsageEvent {
            id: uuid::Uuid::new_v4().to_string(),
            event_key: self.event_key,
            workspace_id: attribution.workspace_id.clone(),
            workspace_agent_id: attribution.workspace_agent_id.clone(),
            session_id: attribution.session_id.clone(),
            generation: None,
            source_kind: self.source_kind.to_owned(),
            source_version: COLLECTOR_VERSION.to_owned(),
            event_kind: "response".to_owned(),
            source_session_id: Some(self.source_session_id),
            source_request_id: self.source_request_id,
            source_response_id: self.source_response_id,
            occurred_at: self.occurred_at,
            recorded_at,
            provider: Some(self.provider.to_owned()),
            requested_model: self.requested_model,
            served_model: self.served_model,
            input_tokens: self.usage.input_tokens,
            output_tokens: self.usage.output_tokens,
            cache_read_input_tokens: self.usage.cache_read_input_tokens,
            cache_write_input_tokens: self.usage.cache_write_input_tokens,
            reasoning_output_tokens: self.usage.reasoning_output_tokens,
            validity: "valid".to_owned(),
            diagnostic_code: None,
        }
    }
}

/// The outcome of parsing one batch of lines.
#[derive(Debug, Default, PartialEq)]
pub struct Scan {
    pub events: Vec<ImportedEvent>,
    /// Which agent ids declared ownership in these lines (owner markers).
    pub owners_declared: Vec<String>,
    /// The cwd the source itself stated (Codex `session_meta` / `turn_context`).
    pub declared_cwd: Option<String>,
    /// The source's own session id, when a line stated it.
    pub source_session_id: Option<String>,
    /// Diagnostic → occurrences. Codes only.
    pub diagnostics: BTreeMap<Diagnostic, u32>,
    /// Earliest and latest event timestamps in this batch.
    pub first_event_at: Option<DateTime<Utc>>,
    pub last_event_at: Option<DateTime<Utc>>,
}

impl Scan {
    fn note(&mut self, diagnostic: Diagnostic) {
        *self.diagnostics.entry(diagnostic).or_insert(0) += 1;
    }

    fn push(&mut self, event: ImportedEvent) {
        self.first_event_at = Some(match self.first_event_at {
            Some(t) => t.min(event.occurred_at),
            None => event.occurred_at,
        });
        self.last_event_at = Some(match self.last_event_at {
            Some(t) => t.max(event.occurred_at),
            None => event.occurred_at,
        });
        self.events.push(event);
    }
}

// ── Claude ───────────────────────────────────────────────────────────────────

/// Parse Claude Code assistant rows into events.
///
/// Every row of one `(sessionId, requestId)` group carries the same message
/// id, model, stop reason and usage (evidence: 605 multi-row groups, zero
/// disagreement), so each row maps to the SAME event and `insert_event`'s
/// `ON CONFLICT DO NOTHING` collapses the group; a later row that disagrees is
/// caught by [`apply_scan`] against the stored identity. `tool_use` IS a
/// completed response; a null `stop_reason` is an in-flight row and is skipped.
pub fn scan_claude_lines(lines: &[String], candidate_owners: &[String]) -> Scan {
    let mut scan = Scan::default();
    for line in lines {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            if !line.trim().is_empty() {
                scan.note(Diagnostic::UnparsableRecord);
            }
            continue;
        };
        for owner in candidate_owners {
            if super::transcript_context::claude_value_declares_owner(&value, owner) {
                scan.owners_declared.push(owner.clone());
            }
        }
        if value["type"].as_str() != Some("assistant") {
            continue;
        }
        let message = &value["message"];
        if message.get("usage").is_none() {
            continue; // an assistant row without usage is not a response record
        }
        let identity = (
            value["sessionId"].as_str(),
            value["requestId"].as_str(),
            message["id"].as_str(),
            value["timestamp"].as_str().and_then(parse_ts),
            message["stop_reason"].as_str(),
        );
        let (Some(session), Some(request), Some(message_id), Some(at), Some(_stop)) = identity
        else {
            if message["stop_reason"].is_null() && message.get("stop_reason").is_some() {
                continue; // in-flight row: not yet a completed response
            }
            scan.note(Diagnostic::ClaudeRowIncomplete);
            continue;
        };
        scan.source_session_id
            .get_or_insert_with(|| session.to_owned());

        let usage = &message["usage"];
        let uncached = counter(&usage["input_tokens"]);
        let cache_write = counter(&usage["cache_creation_input_tokens"]);
        let cache_read = counter(&usage["cache_read_input_tokens"]);
        let input_tokens = match (uncached, cache_write, cache_read) {
            (Some(a), Some(b), Some(c)) => a.checked_add(b).and_then(|ab| ab.checked_add(c)),
            _ => None,
        };
        // Multi-attempt/fallback: several iterations with different models
        // cannot be attributed to one served model.
        let mixed_models = usage["iterations"]
            .as_array()
            .map(|iterations| {
                let models: std::collections::BTreeSet<&str> = iterations
                    .iter()
                    .filter_map(|i| i["model"].as_str())
                    .collect();
                models.len() > 1
            })
            .unwrap_or(false);
        let served_model = if mixed_models {
            None
        } else {
            message["model"]
                .as_str()
                .filter(|m| !m.is_empty())
                .map(str::to_owned)
        };
        scan.push(ImportedEvent {
            event_key: format!(
                "{SOURCE_CLAUDE_TRANSCRIPT}:{COLLECTOR_VERSION}:{session}:{request}"
            ),
            source_kind: SOURCE_CLAUDE_TRANSCRIPT,
            source_session_id: session.to_owned(),
            source_request_id: Some(request.to_owned()),
            source_response_id: Some(message_id.to_owned()),
            occurred_at: at,
            provider: "anthropic",
            requested_model: None,
            served_model,
            usage: MeasuredUsage {
                input_tokens,
                output_tokens: counter(&usage["output_tokens"]),
                cache_read_input_tokens: cache_read,
                cache_write_input_tokens: cache_write,
                reasoning_output_tokens: None,
            },
        });
    }
    scan
}

// ── Codex ────────────────────────────────────────────────────────────────────

/// Bounded parser continuation for a Codex file: the most recent
/// `turn_id → model` pairs, so a usage record in a later batch can still be
/// joined to its turn's selected model. Serialized into the cursor's
/// `parser_state`; never holds transcript text.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexParserState {
    /// Oldest first; capped at [`TURN_MAP_CAP`].
    pub turns: Vec<(String, String)>,
    /// Whether the file has shown the pre-`token_usage_record` shape
    /// (`token_count` rows) — decides "unsupported" when no usage record ever
    /// appears.
    pub saw_token_count: bool,
    pub saw_usage_record: bool,
}

impl CodexParserState {
    fn remember_turn(&mut self, turn_id: &str, model: &str) {
        self.turns.retain(|(id, _)| id != turn_id);
        self.turns.push((turn_id.to_owned(), model.to_owned()));
        while self.turns.len() > TURN_MAP_CAP {
            self.turns.remove(0);
        }
    }

    fn model_for(&self, turn_id: &str) -> Option<String> {
        self.turns
            .iter()
            .rev()
            .find(|(id, _)| id == turn_id)
            .map(|(_, model)| model.clone())
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_owned())
    }

    pub fn from_json(text: Option<&str>) -> Self {
        text.and_then(|t| serde_json::from_str(t).ok())
            .unwrap_or_default()
    }
}

/// Parse Codex rollout records into events, carrying `state` across batches.
///
/// Only a TOP-LEVEL `token_usage_record` is an event; `session_meta` and
/// `turn_context` feed the cwd and the turn → model join; `event_msg`
/// `token_count`, cumulative counters and a `compacted` record's embedded copy
/// are deliberately ignored (evidence: compaction is a double-counting trap).
pub fn scan_codex_lines(
    lines: &[String],
    candidate_owners: &[String],
    state: &mut CodexParserState,
) -> Scan {
    let mut scan = Scan::default();
    for line in lines {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            if !line.trim().is_empty() {
                scan.note(Diagnostic::UnparsableRecord);
            }
            continue;
        };
        for owner in candidate_owners {
            if super::transcript_context::codex_value_declares_owner(&value, owner) {
                scan.owners_declared.push(owner.clone());
            }
        }
        let payload = &value["payload"];
        match value["type"].as_str() {
            Some("session_meta") => {
                if let Some(cwd) = payload["cwd"].as_str() {
                    scan.declared_cwd.get_or_insert_with(|| cwd.to_owned());
                }
                if let Some(id) = payload["id"].as_str() {
                    scan.source_session_id.get_or_insert_with(|| id.to_owned());
                }
            }
            Some("turn_context") => {
                if let Some(cwd) = payload["cwd"].as_str() {
                    scan.declared_cwd.get_or_insert_with(|| cwd.to_owned());
                }
                if let (Some(turn), Some(model)) =
                    (payload["turn_id"].as_str(), payload["model"].as_str())
                {
                    state.remember_turn(turn, model);
                }
            }
            Some("event_msg") if payload["type"].as_str() == Some("token_count") => {
                state.saw_token_count = true;
            }
            Some("token_usage_record") => {
                state.saw_usage_record = true;
                let (Some(session), Some(response), Some(at)) = (
                    payload["session_id"].as_str(),
                    payload["response_id"].as_str(),
                    value["timestamp"].as_str().and_then(parse_ts),
                ) else {
                    scan.note(Diagnostic::CodexRecordIncomplete);
                    continue;
                };
                scan.source_session_id
                    .get_or_insert_with(|| session.to_owned());
                let usage = &payload["usage"];
                let requested_model = payload["turn_id"]
                    .as_str()
                    .and_then(|turn| state.model_for(turn));
                scan.push(ImportedEvent {
                    event_key: format!(
                        "{SOURCE_CODEX_TRANSCRIPT}:{COLLECTOR_VERSION}:{session}:{response}"
                    ),
                    source_kind: SOURCE_CODEX_TRANSCRIPT,
                    source_session_id: session.to_owned(),
                    source_request_id: payload["turn_id"].as_str().map(str::to_owned),
                    source_response_id: Some(response.to_owned()),
                    occurred_at: at,
                    provider: "openai",
                    requested_model,
                    served_model: None,
                    usage: MeasuredUsage {
                        input_tokens: counter(&usage["input_tokens"]),
                        output_tokens: counter(&usage["output_tokens"]),
                        cache_read_input_tokens: counter(&usage["cached_input_tokens"]),
                        cache_write_input_tokens: counter(&usage["cache_write_input_tokens"]),
                        reasoning_output_tokens: counter(&usage["reasoning_output_tokens"]),
                    },
                });
            }
            _ => {}
        }
    }
    scan
}

// ── Applying a scan ──────────────────────────────────────────────────────────

/// What [`apply_scan`] did, for the worker's coverage bookkeeping.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Applied {
    pub inserted: usize,
    pub replayed: usize,
    pub conflicts: usize,
}

/// Persist a scan: insert its events (idempotent), reconcile replays against
/// the stored identity, and advance the cursor — all in ONE transaction.
///
/// A replayed key whose stored identity disagrees on message id, served model
/// or measured counters is marked `conflict` (one activity, no tokens, partial
/// coverage); an agreeing replay is a no-op. The cursor is written last inside
/// the same transaction, so a crash before commit replays cleanly.
pub async fn apply_scan(
    pool: &sqlx::SqlitePool,
    scan: Scan,
    attribution: &Attribution,
    cursor: &CursorRow,
) -> sqlx::Result<Applied> {
    let recorded_at = Utc::now();
    let mut applied = Applied::default();
    let mut tx = pool.begin().await?;
    for event in scan.events {
        let key = event.event_key.clone();
        let row = event.into_row(attribution, recorded_at);
        if model_usage::insert_event(&mut *tx, &row).await? {
            applied.inserted += 1;
            continue;
        }
        applied.replayed += 1;
        if let Some(stored) = model_usage::stored_identity(&mut *tx, &key).await? {
            let agrees = stored.source_response_id == row.source_response_id
                && stored.served_model == row.served_model
                && (stored.validity == "conflict"
                    || (stored.input_tokens == row.input_tokens
                        && stored.output_tokens == row.output_tokens));
            if !agrees && stored.validity != "conflict" {
                model_usage::mark_conflict(&mut *tx, &key, Diagnostic::ClaudeConflict.code())
                    .await?;
                applied.conflicts += 1;
            }
        }
    }
    model_usage::upsert_cursor(&mut *tx, cursor).await?;
    tx.commit().await?;
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::db::connect_in_memory;
    use serde_json::json;
    use std::io::Write;

    // ── Fixtures (shapes from the evidence doc; content reduced) ─────────

    fn claude_row(request: &str, block: u32, model: &str, output: i64) -> String {
        json!({
            "type": "assistant",
            "uuid": format!("u-{request}-{block}"),
            "sessionId": "S1",
            "requestId": request,
            "apiBlockIndex": block,
            "timestamp": format!("2026-09-05T10:00:0{block}.000Z"),
            "message": {
                "id": format!("msg-{request}"),
                "model": model,
                "stop_reason": "tool_use",
                "usage": {
                    "input_tokens": 2,
                    "cache_creation_input_tokens": 27984,
                    "cache_read_input_tokens": 26445,
                    "output_tokens": output,
                    "iterations": [{"type": "message", "model": model}]
                }
            }
        })
        .to_string()
    }

    fn claude_owner_marker(agent: &str) -> String {
        json!({
            "type": "attachment",
            "attachment": {
                "hookEvent": "SessionStart",
                "hookName": "SessionStart:startup",
                "content": [format!("Your own agent id is {agent}.")]
            }
        })
        .to_string()
    }

    #[test]
    fn claude_rows_of_one_request_collapse_to_one_event_with_cache_inclusive_input() {
        let lines = vec![
            claude_owner_marker("agent-1"),
            claude_row("R1", 0, "claude-fable-5-1", 433),
            claude_row("R1", 1, "claude-fable-5-1", 433),
            claude_row("R1", 2, "claude-fable-5-1", 433),
            claude_row("R2", 0, "claude-fable-5-1", 10),
        ];
        let scan = scan_claude_lines(&lines, &["agent-1".into(), "agent-2".into()]);
        assert_eq!(scan.owners_declared, vec!["agent-1".to_string()]);
        assert_eq!(scan.source_session_id.as_deref(), Some("S1"));
        // Four rows, but the three R1 rows share ONE key: the store collapses them.
        assert_eq!(scan.events.len(), 4);
        let keys: std::collections::BTreeSet<&str> =
            scan.events.iter().map(|e| e.event_key.as_str()).collect();
        assert_eq!(
            keys,
            std::collections::BTreeSet::from(["claude-code:v1:S1:R1", "claude-code:v1:S1:R2"])
        );
        let first = &scan.events[0];
        assert_eq!(first.usage.input_tokens, Some(2 + 27984 + 26445));
        assert_eq!(first.usage.output_tokens, Some(433));
        assert_eq!(first.usage.cache_write_input_tokens, Some(27984));
        assert_eq!(first.usage.cache_read_input_tokens, Some(26445));
        assert_eq!(first.served_model.as_deref(), Some("claude-fable-5-1"));
        assert_eq!(
            first.requested_model, None,
            "a transcript states no selection"
        );
        assert_eq!(first.source_response_id.as_deref(), Some("msg-R1"));
        assert!(scan.diagnostics.is_empty());
        assert_eq!(
            scan.first_event_at.unwrap().to_rfc3339(),
            "2026-09-05T10:00:00+00:00"
        );
    }

    /// Missing identity is a diagnostic, never a guessed event; an in-flight
    /// row (null stop_reason) is simply not yet a response.
    #[test]
    fn claude_rows_without_identity_are_diagnostics_not_events() {
        let mut no_request: Value = serde_json::from_str(&claude_row("R1", 0, "m", 1)).unwrap();
        no_request.as_object_mut().unwrap().remove("requestId");
        let mut no_timestamp: Value = serde_json::from_str(&claude_row("R2", 0, "m", 1)).unwrap();
        no_timestamp.as_object_mut().unwrap().remove("timestamp");
        let mut in_flight: Value = serde_json::from_str(&claude_row("R3", 0, "m", 1)).unwrap();
        in_flight["message"]["stop_reason"] = Value::Null;
        let mut no_cache: Value = serde_json::from_str(&claude_row("R4", 0, "m", 7)).unwrap();
        no_cache["message"]["usage"]
            .as_object_mut()
            .unwrap()
            .remove("cache_read_input_tokens");

        let lines = vec![
            no_request.to_string(),
            no_timestamp.to_string(),
            in_flight.to_string(),
            no_cache.to_string(),
            "not json at all".to_string(),
            json!({"type": "user", "message": {"content": "hi"}}).to_string(),
        ];
        let scan = scan_claude_lines(&lines, &[]);
        assert_eq!(scan.events.len(), 1, "only R4 is a complete response");
        let r4 = &scan.events[0];
        assert_eq!(
            r4.usage.input_tokens, None,
            "two of three components: unknown input"
        );
        assert_eq!(r4.usage.output_tokens, Some(7));
        assert_eq!(
            scan.diagnostics.get(&Diagnostic::ClaudeRowIncomplete),
            Some(&2)
        );
        assert_eq!(
            scan.diagnostics.get(&Diagnostic::UnparsableRecord),
            Some(&1)
        );
    }

    /// Fallback across models: the aggregate is kept once, the served model
    /// is unknown rather than pinned to one of them.
    #[test]
    fn claude_mixed_iterations_leave_the_served_model_unknown() {
        let mut row: Value =
            serde_json::from_str(&claude_row("R1", 0, "claude-opus-5", 5)).unwrap();
        row["message"]["usage"]["iterations"] = json!([
            {"type": "message", "model": "claude-opus-5"},
            {"type": "message", "model": "claude-sonnet-5"}
        ]);
        let scan = scan_claude_lines(&[row.to_string()], &[]);
        assert_eq!(scan.events[0].served_model, None);
        assert_eq!(scan.events[0].usage.output_tokens, Some(5), "counted once");
    }

    fn codex_line(value: Value) -> String {
        value.to_string()
    }

    fn codex_usage_record(response: &str, turn: &str, ordinal: u32, input: i64) -> Value {
        json!({
            "type": "token_usage_record",
            "ordinal": ordinal,
            "timestamp": format!("2026-09-05T11:00:{ordinal:02}.000Z"),
            "payload": {
                "response_id": response,
                "session_id": "SESS",
                "thread_id": "THREAD",
                "turn_id": turn,
                "root_turn_id": turn,
                "usage": {
                    "input_tokens": input,
                    "cached_input_tokens": 209536,
                    "cache_write_input_tokens": 0,
                    "output_tokens": 1250,
                    "reasoning_output_tokens": 358,
                    "total_tokens": input + 1250
                },
                "turn_token_usage": {"total_tokens": 999999},
                "thread_token_usage": {"total_tokens": 9999999}
            }
        })
    }

    fn codex_fixture() -> Vec<String> {
        vec![
            codex_line(
                json!({"type": "session_meta", "payload": {"id": "SESS", "cwd": "/Users/x/proj", "cli_version": "0.153.4"}}),
            ),
            codex_line(
                json!({"type": "response_item", "payload": {"type": "message", "role": "developer",
                "content": [{"type": "input_text", "text": "Your own agent id is agent-c."}]}}),
            ),
            codex_line(
                json!({"type": "turn_context", "payload": {"turn_id": "T1", "model": "gpt-5.6-sol", "cwd": "/Users/x/proj"}}),
            ),
            codex_line(
                json!({"type": "event_msg", "payload": {"type": "token_count", "info": {"last_token_usage": {"total_tokens": 5}, "total_token_usage": {"total_tokens": 50}}}}),
            ),
            codex_line(codex_usage_record("RESP-1", "T1", 1, 210724)),
            codex_line(
                json!({"type": "compacted", "payload": {"latest_token_usage_record": codex_usage_record("RESP-1", "T1", 1, 210724)["payload"]}}),
            ),
            codex_line(json!({"type": "event_msg", "payload": {"type": "task_complete"}})),
            codex_line(
                json!({"type": "turn_context", "payload": {"turn_id": "T2", "model": "gpt-6-astra"}}),
            ),
            codex_line(codex_usage_record("RESP-2", "T2", 2, 1000)),
        ]
    }

    #[test]
    fn codex_imports_only_top_level_usage_records_joined_to_their_turn_model() {
        let mut state = CodexParserState::default();
        let scan = scan_codex_lines(&codex_fixture(), &["agent-c".into()], &mut state);
        assert_eq!(scan.owners_declared, vec!["agent-c".to_string()]);
        assert_eq!(scan.declared_cwd.as_deref(), Some("/Users/x/proj"));
        assert_eq!(scan.source_session_id.as_deref(), Some("SESS"));
        assert_eq!(
            scan.events.len(),
            2,
            "token_count, the compacted copy and task_complete are not events"
        );
        let first = &scan.events[0];
        assert_eq!(first.event_key, "codex:v1:SESS:RESP-1");
        assert_eq!(first.requested_model.as_deref(), Some("gpt-5.6-sol"));
        assert_eq!(
            first.served_model, None,
            "turn_context proves a selection only"
        );
        assert_eq!(
            first.usage.input_tokens,
            Some(210724),
            "already cache-inclusive"
        );
        assert_eq!(first.usage.cache_read_input_tokens, Some(209536));
        assert_eq!(
            first.usage.output_tokens,
            Some(1250),
            "already includes reasoning"
        );
        assert_eq!(first.usage.reasoning_output_tokens, Some(358));
        assert_eq!(first.source_request_id.as_deref(), Some("T1"));
        let second = &scan.events[1];
        assert_eq!(
            second.requested_model.as_deref(),
            Some("gpt-6-astra"),
            "a model change mid-session follows the turn"
        );
        assert!(state.saw_token_count && state.saw_usage_record);
        assert!(scan.diagnostics.is_empty());
    }

    /// Parser state carries the turn join across batches and restarts.
    #[test]
    fn codex_turn_join_survives_a_batch_boundary_through_parser_state() {
        let lines = codex_fixture();
        let mut state = CodexParserState::default();
        let _ = scan_codex_lines(&lines[..3], &[], &mut state);
        let restored = CodexParserState::from_json(Some(&state.to_json()));
        assert_eq!(restored, state);
        let mut restored = restored;
        let scan = scan_codex_lines(&lines[3..5], &[], &mut restored);
        assert_eq!(scan.events.len(), 1);
        assert_eq!(
            scan.events[0].requested_model.as_deref(),
            Some("gpt-5.6-sol")
        );
        assert_eq!(
            CodexParserState::from_json(Some("garbage")),
            CodexParserState::default()
        );
    }

    #[test]
    fn codex_turn_map_is_bounded() {
        let mut state = CodexParserState::default();
        for i in 0..20 {
            state.remember_turn(&format!("T{i}"), "m");
        }
        assert_eq!(state.turns.len(), TURN_MAP_CAP);
        assert_eq!(state.model_for("T0"), None, "evicted");
        assert_eq!(state.model_for("T19").as_deref(), Some("m"));
    }

    #[test]
    fn codex_record_without_identity_is_a_diagnostic() {
        let mut record = codex_usage_record("RESP-1", "T1", 1, 5);
        record["payload"]
            .as_object_mut()
            .unwrap()
            .remove("response_id");
        let mut state = CodexParserState::default();
        let scan = scan_codex_lines(&[record.to_string()], &[], &mut state);
        assert!(scan.events.is_empty());
        assert_eq!(
            scan.diagnostics.get(&Diagnostic::CodexRecordIncomplete),
            Some(&1)
        );
    }

    // ── Reading ──────────────────────────────────────────────────────────

    fn temp_file(name: &str, content: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "conclave-transcript-usage-{}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rollout.jsonl");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(content)
            .unwrap();
        path
    }

    #[test]
    fn read_batch_returns_complete_lines_only_and_leaves_the_torn_tail() {
        let path = temp_file("torn", b"{\"a\":1}\n{\"b\":2}\r\n{\"c\":");
        let batch = read_batch(&path, 0).unwrap();
        assert_eq!(batch.lines, vec!["{\"a\":1}", "{\"b\":2}"]);
        assert_eq!(batch.next_offset, 17, "just past the second newline");
        assert_eq!(batch.observed_length, 22);
        assert!(batch.has_backlog, "the torn tail is unread");
        assert!(!batch.skipped_oversized);

        // The writer finishes the line: the next batch picks it up from the offset.
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"3}\n")
            .unwrap();
        let next = read_batch(&path, batch.next_offset).unwrap();
        assert_eq!(next.lines, vec!["{\"c\":3}"]);
        assert!(!next.has_backlog);
        assert_eq!(
            read_batch(&path, next.next_offset).unwrap(),
            Batch {
                observed_length: 25,
                next_offset: 25,
                ..Batch::default()
            }
        );
    }

    /// A record longer than one batch but under the record cap still parses
    /// whole; one over the cap is skipped through its newline with a
    /// diagnostic, and the line after it is not lost.
    #[test]
    fn read_batch_handles_long_and_oversized_records() {
        let long = format!("{{\"pad\":\"{}\"}}\n", "x".repeat(BATCH_BYTES + 1000));
        let path = temp_file("long", format!("{long}{{\"after\":1}}\n").as_bytes());
        let batch = read_batch(&path, 0).unwrap();
        assert_eq!(
            batch.lines.len(),
            2,
            "long record read whole, then the next line"
        );
        assert_eq!(batch.lines[0].len(), long.len() - 1);
        assert!(!batch.skipped_oversized);

        let huge = format!("{{\"pad\":\"{}\"}}\n", "y".repeat(MAX_RECORD_BYTES + 10));
        let path = temp_file(
            "huge",
            format!("{{\"before\":1}}\n{huge}{{\"after\":2}}\n").as_bytes(),
        );
        let mut offset = 0;
        let mut lines = Vec::new();
        let mut skipped = false;
        for _ in 0..8 {
            let batch = read_batch(&path, offset).unwrap();
            lines.extend(batch.lines);
            skipped |= batch.skipped_oversized;
            offset = batch.next_offset;
            if !batch.has_backlog {
                break;
            }
        }
        assert!(skipped, "the oversized record was skipped");
        assert_eq!(lines, vec!["{\"before\":1}", "{\"after\":2}"]);
        // `{"before":1}\n` is 13 bytes, `{"after":2}\n` is 12.
        assert_eq!(offset as usize, 13 + huge.len() + 12);
    }

    // ── Applying ─────────────────────────────────────────────────────────

    fn cursor_for(path: &str, offset: i64, length: i64) -> CursorRow {
        CursorRow {
            id: "c1".into(),
            source_kind: SOURCE_CLAUDE_TRANSCRIPT.into(),
            source_session_id: "S1".into(),
            path_fingerprint: path.into(),
            byte_offset: offset,
            observed_length: length,
            collector_version: COLLECTOR_VERSION.into(),
            workspace_id: Some("ws".into()),
            workspace_agent_id: Some("agent-1".into()),
            verified_owner: Some("agent-1".into()),
            verified_cwd: None,
            parser_state: None,
            last_verified_at: "2026-09-05T12:00:00.000Z".into(),
        }
    }

    fn attribution() -> Attribution {
        Attribution {
            workspace_id: Some("ws".into()),
            workspace_agent_id: Some("agent-1".into()),
            session_id: None,
        }
    }

    /// The plan's replay regression through the production importer path:
    /// three rows of one request are one activity; replaying the batch after a
    /// "crash" changes nothing; the cursor moves with the events.
    #[tokio::test]
    async fn replay_through_the_importer_cannot_double_count() {
        let pool = connect_in_memory().await;
        let lines = vec![
            claude_row("R1", 0, "m", 433),
            claude_row("R1", 1, "m", 433),
            claude_row("R1", 2, "m", 433),
            claude_row("R2", 0, "m", 10),
        ];
        let first = apply_scan(
            &pool,
            scan_claude_lines(&lines, &[]),
            &attribution(),
            &cursor_for("fp", 100, 100),
        )
        .await
        .unwrap();
        assert_eq!(
            first,
            Applied {
                inserted: 2,
                replayed: 2,
                conflicts: 0
            }
        );

        // Crash before the cursor was observed by the worker: replay the batch.
        let again = apply_scan(
            &pool,
            scan_claude_lines(&lines, &[]),
            &attribution(),
            &cursor_for("fp", 100, 100),
        )
        .await
        .unwrap();
        assert_eq!(
            again,
            Applied {
                inserted: 0,
                replayed: 4,
                conflicts: 0
            }
        );

        let total = model_usage::aggregate_range(
            &pool,
            &Default::default(),
            "2026-09-01T00:00:00.000Z",
            "2026-09-30T00:00:00.000Z",
        )
        .await
        .unwrap();
        assert_eq!(total.activity_count, 2);
        assert_eq!(
            total.measured_tokens,
            Some((2 + 27984 + 26445) * 2 + 433 + 10)
        );
        let cursor = model_usage::get_cursor(&pool, SOURCE_CLAUDE_TRANSCRIPT, "fp")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cursor.byte_offset, 100);
    }

    /// A later row that disagrees with the recorded response makes the event a
    /// conflict: one activity, no measured tokens, and nothing new.
    #[tokio::test]
    async fn a_disagreeing_replay_marks_the_event_conflict_not_a_second_activity() {
        let pool = connect_in_memory().await;
        apply_scan(
            &pool,
            scan_claude_lines(&[claude_row("R1", 0, "m", 433)], &[]),
            &attribution(),
            &cursor_for("fp", 1, 1),
        )
        .await
        .unwrap();
        let applied = apply_scan(
            &pool,
            scan_claude_lines(&[claude_row("R1", 1, "m", 999)], &[]), // output differs
            &attribution(),
            &cursor_for("fp", 2, 2),
        )
        .await
        .unwrap();
        assert_eq!(
            applied,
            Applied {
                inserted: 0,
                replayed: 1,
                conflicts: 1
            }
        );
        let total = model_usage::aggregate_range(
            &pool,
            &Default::default(),
            "2026-09-01T00:00:00.000Z",
            "2026-09-30T00:00:00.000Z",
        )
        .await
        .unwrap();
        assert_eq!(total.activity_count, 1);
        assert_eq!(total.conflict_count, 1);
        assert_eq!(total.measured_tokens, None);

        // Once in conflict, a further replay does not re-mark or resurrect it.
        let again = apply_scan(
            &pool,
            scan_claude_lines(&[claude_row("R1", 2, "m", 433)], &[]),
            &attribution(),
            &cursor_for("fp", 3, 3),
        )
        .await
        .unwrap();
        assert_eq!(again.conflicts, 0);
    }

    /// The transaction is atomic: a cursor advance never lands without its
    /// events (simulated by a poisoned cursor row that violates a CHECK).
    #[tokio::test]
    async fn a_failed_cursor_write_rolls_the_events_back() {
        let pool = connect_in_memory().await;
        let mut bad_cursor = cursor_for("fp", 10, 10);
        bad_cursor.byte_offset = -1; // violates CHECK (byte_offset >= 0)
        let result = apply_scan(
            &pool,
            scan_claude_lines(&[claude_row("R1", 0, "m", 1)], &[]),
            &attribution(),
            &bad_cursor,
        )
        .await;
        assert!(result.is_err());
        let total = model_usage::aggregate_range(
            &pool,
            &Default::default(),
            "2026-09-01T00:00:00.000Z",
            "2026-09-30T00:00:00.000Z",
        )
        .await
        .unwrap();
        assert_eq!(total.activity_count, 0, "no event without its cursor");
    }
}
