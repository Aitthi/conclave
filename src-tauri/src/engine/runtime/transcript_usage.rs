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
//! * [`ImportWorker`] — the ONE production worker: discovers candidate files
//!   for known workspaces and agents, verifies ownership, runs the reads and
//!   parses on the blocking pool, records coverage, and stays within a per-tick
//!   byte budget. `usage.overview` may [`nudge`] it but never awaits it.
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

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::transcript_context::{claude_project_dir, collect_jsonl_files, parse_ts};
use super::usage::{
    checked_sum, counter_tracked, MeasuredUsage, COLLECTOR_VERSION, SOURCE_CLAUDE_TRANSCRIPT,
    SOURCE_CODEX_TRANSCRIPT,
};
use crate::engine::repo;
use crate::engine::repo::model_usage::{
    self, CursorRow, NewUsageEvent, ObservedInterval, UNSUPPORTED_SOURCE,
};

// ── Bounds ───────────────────────────────────────────────────────────────────

/// Default bytes read from one file in one batch.
pub const BATCH_BYTES: usize = 256 * 1024;
/// Default bytes read per tick across all files.
pub const TICK_BUDGET_BYTES: usize = 8 * 1024 * 1024;
/// Default cap on a single source record held in memory. A longer record is
/// skipped through its newline — a bounded slice per batch, never re-read
/// forever, never logged.
pub const MAX_RECORD_BYTES: usize = 4 * 1024 * 1024;
/// How many `turn_id → model` pairs Codex parser state keeps across batches.
const TURN_MAP_CAP: usize = 8;
/// How many files may hold an in-memory partial record at once.
const PENDING_RECORDS_CAP: usize = 8;

// ── Diagnostics ──────────────────────────────────────────────────────────────

/// Bounded, code-only diagnostics a scan may raise. Each makes the scope's
/// coverage partial; none carries transcript text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Diagnostic {
    /// A record longer than the record cap was skipped.
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
    /// Owner markers or cwd that do not agree with the file's workspace.
    OwnershipConflict,
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
            Diagnostic::OwnershipConflict => "ownership_conflict",
        }
    }
}

// ── Reading ──────────────────────────────────────────────────────────────────

/// The in-memory continuation of ONE file between batches: the bytes of a
/// record that has started but not yet ended, or the count of bytes already
/// scanned past of a record too long to hold. Never persisted as text; the
/// cursor stays at the record's start, so a restart simply re-reads it
/// bounded batch by bounded batch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PendingRecord {
    /// Bytes of the incomplete record read so far (≤ the record cap).
    pub buffer: Vec<u8>,
    /// Bytes of an OVERSIZED record already scanned past without holding
    /// them. `Some` while skipping; persisted in the cursor's parser state so
    /// a restart resumes the skip instead of re-reading the record.
    pub skipping_scanned: Option<u64>,
}

impl PendingRecord {
    /// Bytes past the cursor that this continuation already accounts for.
    fn scanned(&self) -> u64 {
        self.skipping_scanned
            .unwrap_or(0)
            .max(self.buffer.len() as u64)
    }
}

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
    /// Bytes actually read from the file by this call — the IO counter the
    /// per-file and per-tick budgets are enforced against.
    pub bytes_read: usize,
    /// Whether an oversized record's skip completed inside this batch.
    pub skipped_oversized: bool,
    /// Whether unread bytes remain past what this file's continuation covers.
    pub has_backlog: bool,
}

/// Read complete lines starting at the cursor `offset`, consuming AT MOST
/// `max_bytes` from the file (review a12f77f2 C2: the budget counts actual
/// bytes, including bytes of partial records and of oversized skips).
///
/// `pending` carries the file's continuation between calls. A record longer
/// than one batch accumulates in `pending.buffer` across calls up to
/// `max_record_bytes`; beyond that the buffer is dropped and the record is
/// skipped through its newline, again `max_bytes` per call, and reported once
/// with `skipped_oversized` when the newline is found. A partial tail without
/// a newline yet (writer mid-append) is never consumed.
pub fn read_batch(
    path: &Path,
    offset: u64,
    max_bytes: usize,
    max_record_bytes: usize,
    pending: &mut PendingRecord,
) -> std::io::Result<Batch> {
    let mut file = File::open(path)?;
    let observed_length = file.metadata()?.len();
    let mut batch = Batch {
        observed_length,
        next_offset: offset,
        ..Batch::default()
    };
    let read_from = offset + pending.scanned();
    if read_from >= observed_length || max_bytes == 0 {
        batch.has_backlog = offset + pending.scanned() < observed_length;
        return Ok(batch);
    }
    file.seek(SeekFrom::Start(read_from))?;

    let mut chunk = vec![0u8; max_bytes.min(BATCH_BYTES.max(max_bytes))];
    let mut remaining = max_bytes;
    while remaining > 0 {
        let want = remaining.min(chunk.len());
        let read = file.read(&mut chunk[..want])?;
        if read == 0 {
            break;
        }
        remaining -= read;
        batch.bytes_read += read;
        let mut data = &chunk[..read];

        // Skipping an oversized record: look only for its newline.
        if let Some(scanned) = pending.skipping_scanned {
            match data.iter().position(|&b| b == b'\n') {
                Some(nl) => {
                    let record_len = scanned + (nl + 1) as u64;
                    batch.next_offset += record_len;
                    batch.skipped_oversized = true;
                    pending.skipping_scanned = None;
                    data = &data[nl + 1..];
                }
                None => {
                    pending.skipping_scanned = Some(scanned + read as u64);
                    continue;
                }
            }
        }

        pending.buffer.extend_from_slice(data);
        // Drain every complete line.
        while let Some(nl) = pending.buffer.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = pending.buffer.drain(..=nl).collect();
            batch.next_offset += line.len() as u64;
            let text = String::from_utf8_lossy(&line[..line.len() - 1]);
            batch.lines.push(text.trim_end_matches('\r').to_owned());
        }
        // A record past the cap: stop holding it, start skipping.
        if pending.buffer.len() > max_record_bytes {
            pending.skipping_scanned = Some(pending.buffer.len() as u64);
            pending.buffer.clear();
        }
    }

    batch.has_backlog = batch.next_offset + pending.scanned() < observed_length;
    Ok(batch)
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
            diagnostic_code: self.usage.diagnostic_code().map(str::to_owned),
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
        if let Some(cwd) = value["cwd"].as_str() {
            scan.declared_cwd.get_or_insert_with(|| cwd.to_owned());
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
        let mut invalid = 0;
        let uncached = counter_tracked(&usage["input_tokens"], &mut invalid);
        let cache_write = counter_tracked(&usage["cache_creation_input_tokens"], &mut invalid);
        let cache_read = counter_tracked(&usage["cache_read_input_tokens"], &mut invalid);
        let input_tokens = checked_sum(&[uncached, cache_write, cache_read], &mut invalid);
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
                output_tokens: counter_tracked(&usage["output_tokens"], &mut invalid),
                cache_read_input_tokens: cache_read,
                cache_write_input_tokens: cache_write,
                reasoning_output_tokens: None,
                invalid_counters: invalid,
            },
        });
    }
    scan
}

// ── Codex ────────────────────────────────────────────────────────────────────

/// Everything a file's cursor needs to continue after a restart, serialized
/// into `model_usage_cursor.parser_state`. Never raw transcript text.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileParserState {
    #[serde(default)]
    pub codex: CodexParserState,
    /// Bytes already scanned past of an oversized record being skipped, so a
    /// restart resumes the skip from the cursor instead of re-reading.
    #[serde(default)]
    pub skipping_scanned: Option<u64>,
}

impl FileParserState {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_owned())
    }

    pub fn from_json(text: Option<&str>) -> Self {
        text.and_then(|t| serde_json::from_str(t).ok())
            .unwrap_or_default()
    }
}

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
    /// Span of record timestamps seen so far (any record type, canonical
    /// strings), so an unsupported file can still be reported over the
    /// interval it covers.
    pub first_seen_at: Option<String>,
    pub last_seen_at: Option<String>,
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
        if let Some(at) = value["timestamp"].as_str().and_then(parse_ts) {
            // Canonical strings order like the instants they encode.
            let at = model_usage::canonical_ts(at);
            state.first_seen_at = Some(match state.first_seen_at.take() {
                Some(t) if t <= at => t,
                _ => at.clone(),
            });
            state.last_seen_at = Some(match state.last_seen_at.take() {
                Some(t) if t >= at => t,
                _ => at,
            });
        }
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
                    usage: {
                        let mut invalid = 0;
                        MeasuredUsage {
                            input_tokens: counter_tracked(&usage["input_tokens"], &mut invalid),
                            output_tokens: counter_tracked(&usage["output_tokens"], &mut invalid),
                            cache_read_input_tokens: counter_tracked(
                                &usage["cached_input_tokens"],
                                &mut invalid,
                            ),
                            cache_write_input_tokens: counter_tracked(
                                &usage["cache_write_input_tokens"],
                                &mut invalid,
                            ),
                            reasoning_output_tokens: counter_tracked(
                                &usage["reasoning_output_tokens"],
                                &mut invalid,
                            ),
                            invalid_counters: invalid,
                        }
                    },
                });
            }
            _ => {}
        }
    }
    scan
}

// ── Applying a scan ──────────────────────────────────────────────────────────

/// What [`apply_scan`] did, for the worker's bookkeeping.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Applied {
    pub inserted: usize,
    pub replayed: usize,
    pub conflicts: usize,
}

/// A coverage interval the worker wants written together with a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageWrite {
    pub workspace_id: Option<String>,
    pub workspace_agent_id: Option<String>,
    pub source_kind: &'static str,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub complete: bool,
    pub diagnostic_code: Option<&'static str>,
}

/// Smallest half-open width an interval is written with: a single event or
/// diagnostic at instant `t` must be visible to overlap queries, and `[t, t)`
/// is not (review a12f77f2 C4).
const MIN_INTERVAL: chrono::Duration = chrono::Duration::milliseconds(1);

async fn write_coverage(
    conn: &mut sqlx::SqliteConnection,
    write: &CoverageWrite,
    verified_at: DateTime<Utc>,
) -> sqlx::Result<()> {
    let end = write.end.max(write.start + MIN_INTERVAL);
    model_usage::record_coverage(
        conn,
        &ObservedInterval {
            workspace_id: write.workspace_id.as_deref(),
            workspace_agent_id: write.workspace_agent_id.as_deref(),
            source_kind: write.source_kind,
            start: write.start,
            end,
            state: if write.complete {
                "complete"
            } else {
                "partial"
            },
            diagnostic_code: write.diagnostic_code,
            collector_version: COLLECTOR_VERSION,
            last_verified_at: verified_at,
        },
    )
    .await
}

/// Persist a scan: insert its events (idempotent), reconcile replays against
/// the stored identity, write the coverage the batch proved, and advance the
/// cursor — all in ONE transaction, so consumed bytes can never outlive their
/// coverage or their events (review a12f77f2 C4).
///
/// A replayed key whose stored identity disagrees on response id, served
/// model or any of the four stored counters is marked `conflict` (one
/// activity, no tokens, partial coverage) and never becomes a second activity;
/// an agreeing later block advances `occurred_at` forward to its timestamp
/// (the group completed at its LAST block).
pub async fn apply_scan(
    pool: &sqlx::SqlitePool,
    scan: Scan,
    attribution: &Attribution,
    cursor: &CursorRow,
    coverage: &[CoverageWrite],
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
            if stored.validity == "conflict" {
                continue; // already withdrawn; nothing more to learn
            }
            let agrees = stored.source_response_id == row.source_response_id
                && stored.served_model == row.served_model
                && stored.input_tokens == row.input_tokens
                && stored.output_tokens == row.output_tokens
                && stored.cache_read_input_tokens == row.cache_read_input_tokens
                && stored.cache_write_input_tokens == row.cache_write_input_tokens;
            if agrees {
                model_usage::advance_occurred_at(&mut *tx, &key, row.occurred_at).await?;
            } else {
                model_usage::mark_conflict(&mut *tx, &key, Diagnostic::ClaudeConflict.code())
                    .await?;
                applied.conflicts += 1;
            }
        }
    }
    for write in coverage {
        write_coverage(&mut tx, write, recorded_at).await?;
    }
    model_usage::upsert_cursor(&mut *tx, cursor).await?;
    tx.commit().await?;
    Ok(applied)
}

// ── The worker ───────────────────────────────────────────────────────────────

/// Tick period of the production worker.
pub const TICK: Duration = Duration::from_secs(10);
/// How long a candidate-discovery pass stays valid.
pub const DISCOVERY_TTL: Duration = Duration::from_secs(30);
/// Files are candidates only if modified within this many UTC days — 92, a
/// buffer over the 90 local dates the Overview shows across zones and DST.
pub const LOOKBACK_DAYS: i64 = 92;
/// A live `complete` interval ends this far behind `now`: a file that appeared
/// after the last discovery pass has not been read yet, so the most recent
/// stretch is not yet proven.
pub const LIVE_LAG: Duration = Duration::from_secs(40);

static NUDGE: OnceLock<Arc<tokio::sync::Notify>> = OnceLock::new();

/// Ask the worker to run a tick soon. Never blocks, never waits for it, never
/// spawns a second worker; a no-op when no worker is running (tests).
pub fn nudge() {
    if let Some(notify) = NUDGE.get() {
        notify.notify_one();
    }
}

/// Where the transcripts live and how much the worker may read. The bounds
/// are configuration so tests exercise the budget paths with small numbers;
/// production uses [`WorkerConfig::default_roots`].
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub claude_projects_root: PathBuf,
    pub codex_sessions_root: PathBuf,
    /// Max bytes read from one file per batch.
    pub batch_bytes: usize,
    /// Max bytes read across all files per tick.
    pub tick_budget_bytes: usize,
    /// Max bytes of one record held in memory.
    pub max_record_bytes: usize,
    pub discovery_ttl: Duration,
    pub live_lag: Duration,
}

impl WorkerConfig {
    pub fn default_roots() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self::with_roots(
            home.join(".claude").join("projects"),
            home.join(".codex").join("sessions"),
        )
    }

    pub fn with_roots(claude_projects_root: PathBuf, codex_sessions_root: PathBuf) -> Self {
        Self {
            claude_projects_root,
            codex_sessions_root,
            batch_bytes: BATCH_BYTES,
            tick_budget_bytes: TICK_BUDGET_BYTES,
            max_record_bytes: MAX_RECORD_BYTES,
            discovery_ttl: DISCOVERY_TTL,
            live_lag: LIVE_LAG,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Source {
    Claude,
    Codex,
}

impl Source {
    fn kind(self) -> &'static str {
        match self {
            Source::Claude => SOURCE_CLAUDE_TRANSCRIPT,
            Source::Codex => SOURCE_CODEX_TRANSCRIPT,
        }
    }
    fn cli_kind(self) -> &'static str {
        match self {
            Source::Claude => "claude-code",
            Source::Codex => "codex",
        }
    }
}

/// One file the worker may read. Claude files are DISCOVERED through the
/// project dir of a workspace folder; Codex files through the date tree. In
/// both cases the workspace a file's events land in is decided by the cwd the
/// file itself declares, never by where it was found (review a12f77f2 C3).
#[derive(Debug, Clone)]
struct Candidate {
    source: Source,
    path: PathBuf,
}

/// A known workspace and its transcript-backed agents, snapshotted at
/// discovery time.
#[derive(Debug, Clone)]
struct KnownWorkspace {
    id: String,
    folder_path: String,
    /// `cli_kind → agent ids`.
    agents: HashMap<&'static str, Vec<String>>,
}

/// A (workspace, agent, source) whose files are owner-verified.
type Scope = (String, String, Source);

#[derive(Debug, Default)]
struct WorkerState {
    discovered_at: Option<Instant>,
    workspaces: Vec<KnownWorkspace>,
    /// Rotating queue: the head is read next, then pushed to the back.
    queue: VecDeque<Candidate>,
    /// Files judged not ours (no workspace, no verified owner), with the
    /// length at which that was decided — re-examined only if they grow.
    not_ours: HashMap<PathBuf, u64>,
    /// In-memory partial-record continuations, bounded in count.
    pending: HashMap<PathBuf, PendingRecord>,
    /// The files each verified scope is made of.
    scope_files: HashMap<Scope, BTreeSet<PathBuf>>,
    /// Start of the CURRENT continuous verified window per scope. Absent
    /// means "not currently proven": the next fully successful tick opens a
    /// new window, and the gap between windows stays visible as two coverage
    /// rows (review a12f77f2 C5).
    verified_since: HashMap<Scope, DateTime<Utc>>,
}

/// The single production importer. Construct once, `run` once.
pub struct ImportWorker {
    db: sqlx::SqlitePool,
    config: WorkerConfig,
    state: Mutex<WorkerState>,
}

/// What one tick did — for tests and for the log line.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TickReport {
    pub files_read: usize,
    /// Actual bytes read from disk this tick (≤ the tick budget).
    pub bytes_read: usize,
    pub inserted: usize,
    pub conflicts: usize,
    /// Some file still has unread bytes (budget exhaustion, partial record,
    /// or a rescan pending).
    pub backlog: bool,
    /// Scopes that received a complete live interval this tick.
    pub live_scopes: usize,
}

/// Outcome of one file visit.
struct FileOutcome {
    bytes_read: usize,
    applied: Applied,
    backlog: bool,
    /// The verified scope this file belongs to, if attribution succeeded.
    scope: Option<Scope>,
}

impl ImportWorker {
    pub fn new(db: sqlx::SqlitePool, config: WorkerConfig) -> Self {
        Self {
            db,
            config,
            state: Mutex::new(WorkerState::default()),
        }
    }

    /// Run forever: one tick every [`TICK`], or sooner when nudged. Started
    /// exactly once from application setup; the test `AppState` never starts
    /// it, so tests drive [`Self::run_tick`] against temp directories.
    pub async fn run(self: Arc<Self>) {
        let notify = NUDGE
            .get_or_init(|| Arc::new(tokio::sync::Notify::new()))
            .clone();
        loop {
            match self.run_tick().await {
                Ok(report) if report.inserted > 0 || report.conflicts > 0 => {
                    eprintln!(
                        "[usage] transcript import: {} file(s), {} byte(s), {} event(s), {} conflict(s){}",
                        report.files_read,
                        report.bytes_read,
                        report.inserted,
                        report.conflicts,
                        if report.backlog {
                            ", backlog remains"
                        } else {
                            ""
                        }
                    );
                }
                Ok(_) => {}
                Err(e) => eprintln!("[usage] transcript import tick failed: {e}"),
            }
            tokio::select! {
                _ = tokio::time::sleep(TICK) => {}
                _ = notify.notified() => {}
            }
        }
    }

    /// One bounded pass: refresh discovery when stale, then visit the rotating
    /// queue while budget remains — each file reads at most
    /// `min(batch_bytes, remaining)` ACTUAL bytes — applying each batch
    /// atomically with the coverage it proved. Finally, every verified scope
    /// whose files were ALL visited without backlog or error extends its live
    /// complete window; any other scope loses its window (a gap).
    pub async fn run_tick(&self) -> Result<TickReport, sqlx::Error> {
        let tick_started = Utc::now();
        self.refresh_discovery().await?;
        let mut report = TickReport::default();
        let queue_len = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .queue
            .len();
        // Scopes proven this tick (all files visited, caught up) and scopes
        // disturbed this tick (backlog, unreadable, budget ran out before a
        // visit).
        let mut clean_files: BTreeSet<PathBuf> = BTreeSet::new();
        let mut disturbed: BTreeSet<Scope> = BTreeSet::new();
        let mut visited = 0usize;
        for _ in 0..queue_len {
            let remaining = self
                .config
                .tick_budget_bytes
                .saturating_sub(report.bytes_read);
            if remaining == 0 {
                report.backlog = true;
                break;
            }
            let candidate = {
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                let Some(candidate) = state.queue.pop_front() else {
                    break;
                };
                state.queue.push_back(candidate.clone());
                candidate
            };
            visited += 1;
            let max_bytes = remaining.min(self.config.batch_bytes);
            match self.import_file(&candidate, max_bytes).await? {
                Ok(Some(outcome)) => {
                    if outcome.bytes_read > 0 {
                        report.files_read += 1;
                    }
                    report.bytes_read += outcome.bytes_read;
                    report.inserted += outcome.applied.inserted;
                    report.conflicts += outcome.applied.conflicts;
                    if outcome.backlog {
                        report.backlog = true;
                        if let Some(scope) = outcome.scope {
                            disturbed.insert(scope);
                        }
                    } else {
                        clean_files.insert(candidate.path.clone());
                    }
                }
                Ok(None) => {}
                Err(scope) => {
                    // Unreadable: whatever scope it belonged to is not proven.
                    if let Some(scope) = scope {
                        disturbed.insert(scope);
                    }
                }
            }
        }
        // Files the budget never reached this tick count as disturbed for
        // their scopes.
        let unvisited = queue_len.saturating_sub(visited);
        if unvisited > 0 {
            report.backlog = true;
        }
        report.live_scopes = self
            .record_live_coverage(tick_started, &clean_files, &disturbed, unvisited > 0)
            .await?;
        Ok(report)
    }

    /// Rebuild the candidate list when the last pass is older than the
    /// discovery TTL: known workspaces (archived included, hidden excluded)
    /// with their transcript-backed agents, Claude project dirs derived from
    /// the workspace folders, and the Codex date tree — all filtered to files
    /// modified within [`LOOKBACK_DAYS`]. Filesystem work runs on the blocking
    /// pool.
    async fn refresh_discovery(&self) -> Result<(), sqlx::Error> {
        let stale = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state
                .discovered_at
                .is_none_or(|at| at.elapsed() >= self.config.discovery_ttl)
        };
        if !stale {
            return Ok(());
        }
        let mut workspaces = Vec::new();
        let definitions: HashMap<String, Option<String>> = repo::agent_definition::list(&self.db)
            .await?
            .into_iter()
            .map(|d| (d.id, d.cli_kind))
            .collect();
        let mut rows = repo::workspace::list(&self.db).await?;
        rows.extend(repo::workspace::list_archived(&self.db).await?);
        for row in rows {
            let mut agents: HashMap<&'static str, Vec<String>> = HashMap::new();
            for agent in repo::workspace_agent::list_by_workspace(&self.db, &row.id).await? {
                let kind = match definitions
                    .get(&agent.agent_def_id)
                    .and_then(|k| k.as_deref())
                {
                    Some("claude-code") => Source::Claude.cli_kind(),
                    Some("codex") => Source::Codex.cli_kind(),
                    _ => continue,
                };
                agents.entry(kind).or_default().push(agent.id);
            }
            workspaces.push(KnownWorkspace {
                id: row.id,
                folder_path: row.folder_path,
                agents,
            });
        }

        let config = self.config.clone();
        let snapshot = workspaces.clone();
        let candidates = tokio::task::spawn_blocking(move || discover(&config, &snapshot))
            .await
            .unwrap_or_default();

        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.workspaces = workspaces;
        // Keep rotation order for files still present; append newcomers.
        let present: BTreeSet<PathBuf> = candidates.iter().map(|c| c.path.clone()).collect();
        state.queue.retain(|c| present.contains(&c.path));
        state.pending.retain(|path, _| present.contains(path));
        let queued: BTreeSet<PathBuf> = state.queue.iter().map(|c| c.path.clone()).collect();
        for candidate in candidates {
            if !queued.contains(&candidate.path) {
                state.queue.push_back(candidate);
            }
        }
        state.discovered_at = Some(Instant::now());
        Ok(())
    }

    /// Read one bounded batch of one file and apply it.
    ///
    /// `Ok(Some)` — the file was read (possibly zero events); `Ok(None)` — not
    /// ours or nothing new; `Err(scope)` — unreadable, with the scope it was
    /// known to belong to. Database errors propagate.
    #[allow(clippy::type_complexity)]
    async fn import_file(
        &self,
        candidate: &Candidate,
        max_bytes: usize,
    ) -> Result<Result<Option<FileOutcome>, Option<Scope>>, sqlx::Error> {
        let Some(fingerprint) = fingerprint(&candidate.path) else {
            return Ok(Ok(None)); // vanished between discovery and read
        };
        let source = candidate.source;
        let source_kind = source.kind();
        let existing = model_usage::get_cursor(&self.db, source_kind, &fingerprint).await?;
        let length_now = std::fs::metadata(&candidate.path)
            .map(|m| m.len())
            .unwrap_or(0);

        // A file judged not ours is re-examined only when it grows.
        {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(decided_at) = state.not_ours.get(&candidate.path) {
                if length_now <= *decided_at {
                    return Ok(Ok(None));
                }
            }
        }

        // Where to read from: the cursor, unless the file shrank — then rescan
        // from zero (events stay; the unique key makes the rescan a
        // reconciliation, not inflation).
        let mut offset = existing.as_ref().map(|c| c.byte_offset as u64).unwrap_or(0);
        let mut file_state = existing
            .as_ref()
            .map(|c| FileParserState::from_json(c.parser_state.as_deref()))
            .unwrap_or_default();
        let shrank = existing
            .as_ref()
            .is_some_and(|c| length_now < c.observed_length as u64);
        if shrank {
            offset = 0;
            file_state = FileParserState::default();
            self.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pending
                .remove(&candidate.path);
        }
        let mut pending = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let pending = state.pending.remove(&candidate.path).unwrap_or_default();
            PendingRecord {
                skipping_scanned: file_state.skipping_scanned.or(pending.skipping_scanned),
                buffer: pending.buffer,
            }
        };
        if offset + pending.scanned() >= length_now {
            return Ok(Ok(None)); // nothing new
        }

        let candidate_agents: Vec<String> = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state
                .workspaces
                .iter()
                .flat_map(|w| w.agents.get(source.cli_kind()).cloned().unwrap_or_default())
                .collect()
        };
        let path = candidate.path.clone();
        let mut codex_state = file_state.codex.clone();
        let max_record = self.config.max_record_bytes;
        let read = tokio::task::spawn_blocking(
            move || -> std::io::Result<(Batch, Scan, CodexParserState, PendingRecord)> {
                let batch = read_batch(&path, offset, max_bytes, max_record, &mut pending)?;
                let scan = match source {
                    Source::Claude => scan_claude_lines(&batch.lines, &candidate_agents),
                    Source::Codex => {
                        scan_codex_lines(&batch.lines, &candidate_agents, &mut codex_state)
                    }
                };
                Ok((batch, scan, codex_state, pending))
            },
        )
        .await
        .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;
        let known_scope = existing.as_ref().and_then(|c| {
            Some((
                c.workspace_id.clone()?,
                c.workspace_agent_id.clone()?,
                source,
            ))
        });
        let (batch, mut scan, codex_state, pending) = match read {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "[usage] transcript {} unreadable: {e}",
                    candidate.path.display()
                );
                return Ok(Err(known_scope));
            }
        };
        if batch.skipped_oversized {
            scan.note(Diagnostic::OversizedRecord);
        }
        // Keep the continuation for the next visit, bounded in count.
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if !pending.buffer.is_empty() {
                if state.pending.len() >= PENDING_RECORDS_CAP {
                    let victim = state.pending.keys().next().cloned();
                    if let Some(victim) = victim {
                        state.pending.remove(&victim); // re-read later from its cursor
                    }
                }
                state.pending.insert(
                    candidate.path.clone(),
                    PendingRecord {
                        buffer: pending.buffer.clone(),
                        skipping_scanned: None,
                    },
                );
            }
        }
        let bytes_read = batch.bytes_read;
        let backlog = batch.has_backlog;
        let now = Utc::now();

        // ── Attribution (C3): the file's own declared cwd names the workspace;
        // the owner marker names the agent; the agent must belong to that
        // workspace. Anything less imports nothing.
        let declared_cwd = scan
            .declared_cwd
            .clone()
            .or_else(|| existing.as_ref().and_then(|c| c.verified_cwd.clone()));
        let (workspace_id, owner, ownership_conflict, verified) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let workspace = declared_cwd.as_deref().and_then(|cwd| {
                state
                    .workspaces
                    .iter()
                    .find(|w| same_folder(&w.folder_path, cwd))
            });
            // The DECLARED owner persists on the cursor from the first marker
            // seen, even before the workspace is known — under a tight budget
            // the marker and the first cwd-bearing row can land in different
            // batches. Verification is re-evaluated every batch from the
            // persisted knowledge.
            let prior_owner = existing.as_ref().and_then(|c| c.verified_owner.clone());
            let owner = prior_owner.or_else(|| scan.owners_declared.first().cloned());
            let conflict = owner
                .as_ref()
                .is_some_and(|o| scan.owners_declared.iter().any(|d| d != o));
            let verified = match (&workspace, &owner) {
                (Some(ws), Some(owner)) => {
                    !conflict
                        && ws
                            .agents
                            .get(source.cli_kind())
                            .is_some_and(|agents| agents.contains(owner))
                }
                _ => false,
            };
            // A marker from an agent outside the declared workspace is a
            // conflict too, once the workspace is known.
            let foreign_owner = matches!((&workspace, &owner), (Some(ws), Some(owner))
                if !ws.agents.get(source.cli_kind()).is_some_and(|a| a.contains(owner)));
            (
                workspace.map(|w| w.id.clone()),
                owner,
                conflict || foreign_owner,
                verified,
            )
        };

        let fully_consumed = !backlog && pending.buffer.is_empty();
        if workspace_id.is_none() && fully_consumed {
            // Read to the end and no row ever named a workspace folder: not
            // ours. Remember the length; re-examine only if it grows.
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.not_ours.insert(candidate.path.clone(), length_now);
            state.pending.remove(&candidate.path);
            return Ok(Ok(None));
        }
        // A cwd may still be ahead (a marker-only head under a tight budget):
        // keep consuming with an unattributed cursor. If a workspace appears
        // later, the newly-verified rescan below recovers the early rows.
        let workspace_id_opt = workspace_id.clone();
        let workspace_id = workspace_id.unwrap_or_default();
        let unattributed = workspace_id_opt.is_none();

        // First verification of a file already consumed unverified: the early
        // rows are attributable now — rescan once from zero.
        let newly_verified = verified
            && existing
                .as_ref()
                .is_some_and(|c| c.workspace_agent_id.is_none() && c.byte_offset > 0);

        let cursor_id = existing
            .as_ref()
            .map(|c| c.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let source_session_id = scan
            .source_session_id
            .clone()
            .or_else(|| existing.as_ref().map(|c| c.source_session_id.clone()))
            .unwrap_or_default();
        // A session identity change is a different file wearing the same
        // fingerprint: start over rather than continue mid-stream.
        let session_changed = existing.as_ref().is_some_and(|c| {
            !c.source_session_id.is_empty()
                && !source_session_id.is_empty()
                && c.source_session_id != source_session_id
                && offset > 0
        });
        let mut cursor = CursorRow {
            id: cursor_id,
            source_kind: source_kind.to_owned(),
            source_session_id,
            path_fingerprint: fingerprint,
            byte_offset: batch.next_offset as i64,
            observed_length: batch.observed_length as i64,
            collector_version: COLLECTOR_VERSION.to_owned(),
            workspace_id: workspace_id_opt.clone(),
            // Attribution only when verified; the declared owner is kept
            // regardless so verification can complete in a later batch.
            workspace_agent_id: if verified { owner.clone() } else { None },
            verified_owner: owner.clone(),
            verified_cwd: declared_cwd.clone(),
            parser_state: Some(
                FileParserState {
                    codex: codex_state.clone(),
                    skipping_scanned: pending.skipping_scanned,
                }
                .to_json(),
            ),
            last_verified_at: model_usage::canonical_ts(now),
        };
        if newly_verified || session_changed {
            cursor.byte_offset = 0;
            cursor.observed_length = 0;
            cursor.parser_state = None;
            self.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pending
                .remove(&candidate.path);
            model_usage::upsert_cursor(&self.db, &cursor).await?;
            return Ok(Ok(Some(FileOutcome {
                bytes_read,
                applied: Applied::default(),
                backlog: true,
                scope: owner.clone().map(|o| (workspace_id, o, source)),
            })));
        }

        if unattributed {
            // Progress only: no events, no coverage claims for a file whose
            // workspace is still unknown.
            let mut unattributed_scan = scan;
            unattributed_scan.events.clear();
            let attribution = Attribution::default();
            let applied =
                apply_scan(&self.db, unattributed_scan, &attribution, &cursor, &[]).await?;
            return Ok(Ok(Some(FileOutcome {
                bytes_read,
                applied,
                backlog,
                scope: None,
            })));
        }

        // ── Coverage the batch proved (written in the same transaction).
        let scope_agent = if verified { owner.clone() } else { None };
        let mut diagnostics = scan.diagnostics.clone();
        if ownership_conflict {
            *diagnostics
                .entry(Diagnostic::OwnershipConflict)
                .or_insert(0) += 1;
        }
        let unsupported = source == Source::Codex
            && !backlog
            && codex_state.saw_token_count
            && !codex_state.saw_usage_record;
        if unsupported {
            diagnostics.insert(Diagnostic::CodexUnsupportedFormat, 1);
        }
        let mut coverage = Vec::new();
        let span = match (scan.first_event_at, scan.last_event_at) {
            (Some(a), Some(b)) if verified => Some((a, b)),
            _ => None,
        };
        if let Some((first, last)) = span {
            // Imported history is conservatively partial: rows prove the
            // responses happened, not that nothing was missed around them.
            coverage.push(CoverageWrite {
                workspace_id: Some(workspace_id.clone()),
                workspace_agent_id: scope_agent.clone(),
                source_kind,
                start: first,
                end: last,
                complete: false,
                diagnostic_code: None,
            });
        }
        if let Some((diagnostic, _)) = diagnostics.iter().next() {
            let (start, end) = span.unwrap_or_else(|| {
                let first = codex_state
                    .first_seen_at
                    .as_deref()
                    .and_then(parse_ts)
                    .unwrap_or(now);
                let last = codex_state
                    .last_seen_at
                    .as_deref()
                    .and_then(parse_ts)
                    .unwrap_or(now);
                (first, last)
            });
            coverage.push(CoverageWrite {
                workspace_id: Some(workspace_id.clone()),
                workspace_agent_id: scope_agent.clone(),
                source_kind,
                start,
                end: end.max(start),
                complete: false,
                diagnostic_code: Some(if unsupported {
                    Diagnostic::CodexUnsupportedFormat.code()
                } else {
                    diagnostic.code()
                }),
            });
        }

        // Unverified: consume the bytes, keep NO events (no guessed
        // attribution, no unassigned transcript activity).
        if !verified {
            scan.events.clear();
        }
        let session_id = match &scope_agent {
            Some(agent) => repo::session::get_by_instance(&self.db, agent)
                .await?
                .map(|s| s.id),
            None => None,
        };
        let attribution = Attribution {
            workspace_id: Some(workspace_id.clone()),
            workspace_agent_id: scope_agent.clone(),
            session_id,
        };
        let applied = apply_scan(&self.db, scan, &attribution, &cursor, &coverage).await?;

        let scope = scope_agent.map(|agent| (workspace_id, agent, source));
        if let Some(scope) = &scope {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state
                .scope_files
                .entry(scope.clone())
                .or_default()
                .insert(candidate.path.clone());
        }
        Ok(Ok(Some(FileOutcome {
            bytes_read,
            applied,
            backlog,
            scope,
        })))
    }

    /// Extend or open the live complete window of every verified scope whose
    /// files were ALL read this tick without backlog; close (forget) the
    /// window of any scope that was disturbed or not fully visited, so the
    /// next success opens a NEW interval and the gap stays on record. Nothing
    /// is claimed before the first fully successful tick, and never for a
    /// scope without owner-verified files (review a12f77f2 C5).
    async fn record_live_coverage(
        &self,
        tick_started: DateTime<Utc>,
        clean_files: &BTreeSet<PathBuf>,
        disturbed: &BTreeSet<Scope>,
        budget_exhausted: bool,
    ) -> Result<usize, sqlx::Error> {
        let now = Utc::now();
        let end = now - chrono::Duration::from_std(self.config.live_lag).unwrap_or_default();
        let writes: Vec<CoverageWrite> = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let scopes: Vec<Scope> = state.scope_files.keys().cloned().collect();
            let mut writes = Vec::new();
            for scope in scopes {
                let files = state.scope_files.get(&scope).cloned().unwrap_or_default();
                let proven = !budget_exhausted
                    && !disturbed.contains(&scope)
                    && !files.is_empty()
                    && files.iter().all(|f| clean_files.contains(f));
                if !proven {
                    state.verified_since.remove(&scope); // the window closes here
                    continue;
                }
                let since = *state
                    .verified_since
                    .entry(scope.clone())
                    .or_insert(tick_started);
                if end > since {
                    writes.push(CoverageWrite {
                        workspace_id: Some(scope.0.clone()),
                        workspace_agent_id: Some(scope.1.clone()),
                        source_kind: scope.2.kind(),
                        start: since,
                        end,
                        complete: true,
                        diagnostic_code: None,
                    });
                }
            }
            writes
        };
        if writes.is_empty() {
            return Ok(0);
        }
        let mut tx = self.db.begin().await?;
        for write in &writes {
            write_coverage(&mut tx, write, now).await?;
        }
        tx.commit().await?;
        Ok(writes.len())
    }
}

/// Candidate files for the known workspaces: Claude by project dir, Codex by
/// the whole date tree. Discovery only narrows the search; attribution is
/// decided by each file's declared cwd and owner marker.
fn discover(config: &WorkerConfig, workspaces: &[KnownWorkspace]) -> Vec<Candidate> {
    let min_mtime = Utc::now() - chrono::Duration::days(LOOKBACK_DAYS);
    let mut out = Vec::new();
    let mut any_codex = false;
    for workspace in workspaces {
        if workspace.agents.contains_key(Source::Claude.cli_kind()) {
            let dir = claude_project_dir(
                &config.claude_projects_root,
                Path::new(&workspace.folder_path),
            );
            for path in collect_jsonl_files(&dir, min_mtime) {
                out.push(Candidate {
                    source: Source::Claude,
                    path,
                });
            }
        }
        any_codex |= workspace.agents.contains_key(Source::Codex.cli_kind());
    }
    if any_codex {
        for path in collect_jsonl_files(&config.codex_sessions_root, min_mtime) {
            out.push(Candidate {
                source: Source::Codex,
                path,
            });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out.dedup_by(|a, b| a.path == b.path);
    out
}

/// Stable identity of the file itself: device and inode, so a rotated or
/// replaced file at the same path is a new cursor.
fn fingerprint(path: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path).ok()?;
    Some(format!("{}:{}", meta.dev(), meta.ino()))
}

/// Compare a workspace folder with a cwd a transcript declared. Trailing
/// slashes are noise; nothing else is normalized — a cwd must name the folder.
fn same_folder(folder: &str, cwd: &str) -> bool {
    folder.trim_end_matches('/') == cwd.trim_end_matches('/')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::db::connect_in_memory;
    use serde_json::json;
    use std::io::Write;

    // ── Fixtures (shapes from the evidence doc; content reduced) ─────────

    fn claude_row(request: &str, block: u32, model: &str, output: i64) -> String {
        claude_row_at("/tmp/ws", request, block, model, output)
    }

    fn claude_row_at(cwd: &str, request: &str, block: u32, model: &str, output: i64) -> String {
        json!({
            "type": "assistant",
            "cwd": cwd,
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
        let wrapped = FileParserState {
            codex: state.clone(),
            skipping_scanned: None,
        };
        let restored = FileParserState::from_json(Some(&wrapped.to_json())).codex;
        assert_eq!(restored, state);
        let mut restored = restored;
        let scan = scan_codex_lines(&lines[3..5], &[], &mut restored);
        assert_eq!(scan.events.len(), 1);
        assert_eq!(
            scan.events[0].requested_model.as_deref(),
            Some("gpt-5.6-sol")
        );
        assert_eq!(
            FileParserState::from_json(Some("garbage")),
            FileParserState::default()
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

    /// Completed inner responses stay valid even when the outer turn is later
    /// aborted: the records are what the source completed, the abort is not
    /// a retraction.
    #[test]
    fn codex_completed_records_survive_an_aborted_outer_turn() {
        let mut lines = codex_fixture();
        lines.push(codex_line(json!({
            "type": "event_msg",
            "payload": {"type": "turn_aborted", "turn_id": "T2", "reason": "interrupted"}
        })));
        let mut state = CodexParserState::default();
        let scan = scan_codex_lines(&lines, &[], &mut state);
        assert_eq!(
            scan.events.len(),
            2,
            "RESP-1 and RESP-2 remain completed responses"
        );
        assert!(scan.diagnostics.is_empty());
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

    /// Drive `read_batch` the way the worker does: cursor at the last complete
    /// line, continuation carried in `pending`, `max` bytes per call.
    fn read_all(
        path: &Path,
        max: usize,
        max_record: usize,
    ) -> (Vec<String>, u64, Vec<usize>, bool) {
        let mut offset = 0;
        let mut pending = PendingRecord::default();
        let mut lines = Vec::new();
        let mut reads = Vec::new();
        let mut skipped = false;
        for _ in 0..200 {
            let batch = read_batch(path, offset, max, max_record, &mut pending).unwrap();
            assert!(batch.bytes_read <= max, "a batch never exceeds its cap");
            reads.push(batch.bytes_read);
            lines.extend(batch.lines);
            skipped |= batch.skipped_oversized;
            offset = batch.next_offset;
            if !batch.has_backlog {
                break;
            }
        }
        (lines, offset, reads, skipped)
    }

    #[test]
    fn read_batch_returns_complete_lines_only_and_leaves_the_torn_tail() {
        let path = temp_file("torn", b"{\"a\":1}\n{\"b\":2}\r\n{\"c\":");
        let mut pending = PendingRecord::default();
        let batch = read_batch(&path, 0, BATCH_BYTES, MAX_RECORD_BYTES, &mut pending).unwrap();
        assert_eq!(batch.lines, vec!["{\"a\":1}", "{\"b\":2}"]);
        assert_eq!(batch.next_offset, 17, "just past the second newline");
        assert_eq!(batch.observed_length, 22);
        assert_eq!(batch.bytes_read, 22, "every byte read is counted");
        assert!(
            !batch.has_backlog,
            "the torn tail is held, nothing unread remains"
        );
        assert_eq!(pending.buffer, b"{\"c\":", "the tail waits in memory");

        // The writer finishes the line: the next batch completes it from the
        // continuation without re-reading what it already holds.
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"3}\n")
            .unwrap();
        let next = read_batch(
            &path,
            batch.next_offset,
            BATCH_BYTES,
            MAX_RECORD_BYTES,
            &mut pending,
        )
        .unwrap();
        assert_eq!(next.lines, vec!["{\"c\":3}"]);
        assert_eq!(next.bytes_read, 3);
        assert_eq!(next.next_offset, 25);
        assert!(pending.buffer.is_empty());
    }

    /// Every call consumes at most `max` bytes — a record longer than one
    /// batch accumulates across calls, and the cursor stays at its start until
    /// it completes.
    #[test]
    fn read_batch_never_exceeds_its_byte_cap_and_completes_long_records_across_batches() {
        let long = format!("{{\"pad\":\"{}\"}}\n", "x".repeat(2_500));
        let path = temp_file(
            "long",
            format!("{{\"first\":0}}\n{long}{{\"after\":1}}\n").as_bytes(),
        );
        let (lines, offset, reads, skipped) = read_all(&path, 1_000, 8_000);
        assert_eq!(lines.len(), 3, "the long record parses whole once complete");
        assert_eq!(lines[1].len(), long.len() - 1);
        assert!(!skipped);
        assert!(reads.iter().all(|r| *r <= 1_000), "{reads:?}");
        assert!(
            reads.len() >= 3,
            "it took several bounded batches: {reads:?}"
        );
        assert_eq!(offset as usize, 12 + long.len() + 12);

        // A record that ends without a newline at EOF is still held, not consumed.
        let mut pending = PendingRecord::default();
        let unfinished = temp_file("unfinished", b"{\"a\":1}\n{\"b\":");
        let batch = read_batch(&unfinished, 0, 1_000, 8_000, &mut pending).unwrap();
        assert_eq!(batch.next_offset, 8);
        assert!(!batch.has_backlog);
        assert_eq!(pending.buffer, b"{\"b\":");
    }

    /// A record over the cap is skipped a bounded slice per batch, the cursor
    /// jumps past it once its newline is found, and the line after it is not
    /// lost. The skip resumes from persisted state without the buffer.
    #[test]
    fn read_batch_skips_oversized_records_a_bounded_slice_at_a_time() {
        let huge = format!("{{\"pad\":\"{}\"}}\n", "y".repeat(10_000));
        let path = temp_file(
            "huge",
            format!("{{\"before\":1}}\n{huge}{{\"after\":2}}\n").as_bytes(),
        );
        let (lines, offset, reads, skipped) = read_all(&path, 1_000, 4_000);
        assert!(skipped, "the oversized record was skipped");
        assert_eq!(lines, vec!["{\"before\":1}", "{\"after\":2}"]);
        assert!(reads.iter().all(|r| *r <= 1_000), "{reads:?}");
        assert_eq!(offset as usize, 13 + huge.len() + 12);

        // Restart mid-skip: only the scanned count survives (no text), and
        // the skip continues from the cursor at the record's start.
        let mut pending = PendingRecord::default();
        let mut offset = 0;
        let mut persisted: Option<u64> = None;
        for _ in 0..20 {
            let batch = read_batch(&path, offset, 1_000, 4_000, &mut pending).unwrap();
            offset = batch.next_offset;
            persisted = pending.skipping_scanned;
            if persisted.is_some() {
                break;
            }
        }
        assert_eq!(offset, 13, "cursor still at the oversized record's start");
        assert!(persisted.is_some(), "skip in progress");
        let mut resumed = PendingRecord {
            buffer: Vec::new(),
            skipping_scanned: persisted,
        };
        let mut lines = Vec::new();
        for _ in 0..50 {
            let batch = read_batch(&path, offset, 1_000, 4_000, &mut resumed).unwrap();
            offset = batch.next_offset;
            lines.extend(batch.lines);
            if !batch.has_backlog {
                break;
            }
        }
        assert_eq!(
            lines,
            vec!["{\"after\":2}"],
            "the skip resumed and the next line survived"
        );
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

    async fn total(pool: &sqlx::SqlitePool) -> model_usage::UsageAggregate {
        model_usage::aggregate_range(
            pool,
            &Default::default(),
            "2026-09-01T00:00:00.000Z",
            "2026-09-30T00:00:00.000Z",
        )
        .await
        .unwrap()
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
            &[],
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

        let again = apply_scan(
            &pool,
            scan_claude_lines(&lines, &[]),
            &attribution(),
            &cursor_for("fp", 100, 100),
            &[],
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

        let t = total(&pool).await;
        assert_eq!(t.activity_count, 2);
        assert_eq!(t.measured_tokens, Some((2 + 27984 + 26445) * 2 + 433 + 10));
        let cursor = model_usage::get_cursor(&pool, SOURCE_CLAUDE_TRANSCRIPT, "fp")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cursor.byte_offset, 100);
    }

    /// A later row that disagrees on ANY stored component — here a cache
    /// component with identical totals — makes the event a conflict: one
    /// activity, no measured tokens, and nothing new (review C6).
    #[tokio::test]
    async fn a_disagreeing_replay_marks_the_event_conflict_not_a_second_activity() {
        let pool = connect_in_memory().await;
        apply_scan(
            &pool,
            scan_claude_lines(&[claude_row("R1", 0, "m", 433)], &[]),
            &attribution(),
            &cursor_for("fp", 1, 1),
            &[],
        )
        .await
        .unwrap();
        // Same input total (2 + 27984 + 26445), shuffled between components.
        let mut shuffled: Value = serde_json::from_str(&claude_row("R1", 1, "m", 433)).unwrap();
        shuffled["message"]["usage"]["cache_creation_input_tokens"] = json!(27985);
        shuffled["message"]["usage"]["cache_read_input_tokens"] = json!(26444);
        let applied = apply_scan(
            &pool,
            scan_claude_lines(&[shuffled.to_string()], &[]),
            &attribution(),
            &cursor_for("fp", 2, 2),
            &[],
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
        let t = total(&pool).await;
        assert_eq!(t.activity_count, 1);
        assert_eq!(t.conflict_count, 1);
        assert_eq!(t.measured_tokens, None);

        // Once in conflict, an agreeing replay does not silently clear it.
        let again = apply_scan(
            &pool,
            scan_claude_lines(&[claude_row("R1", 2, "m", 433)], &[]),
            &attribution(),
            &cursor_for("fp", 3, 3),
            &[],
        )
        .await
        .unwrap();
        assert_eq!(again.conflicts, 0);
        assert_eq!(total(&pool).await.conflict_count, 1, "still withdrawn");
    }

    /// An agreeing later block moves the completion time forward — across
    /// midnight into the next bucket — without adding activity (review C6).
    #[tokio::test]
    async fn an_agreeing_later_block_advances_completion_across_midnight() {
        let pool = connect_in_memory().await;
        let mut first: Value = serde_json::from_str(&claude_row("R1", 0, "m", 5)).unwrap();
        first["timestamp"] = json!("2026-09-05T23:59:59.900Z");
        let mut last: Value = serde_json::from_str(&claude_row("R1", 1, "m", 5)).unwrap();
        last["timestamp"] = json!("2026-09-06T00:00:00.100Z");
        apply_scan(
            &pool,
            scan_claude_lines(&[first.to_string()], &[]),
            &attribution(),
            &cursor_for("fp", 1, 1),
            &[],
        )
        .await
        .unwrap();
        let applied = apply_scan(
            &pool,
            scan_claude_lines(&[last.to_string()], &[]),
            &attribution(),
            &cursor_for("fp", 2, 2),
            &[],
        )
        .await
        .unwrap();
        assert_eq!(
            applied,
            Applied {
                inserted: 0,
                replayed: 1,
                conflicts: 0
            }
        );
        let stored: (String,) = sqlx::query_as(
            "SELECT occurred_at FROM model_usage_event WHERE event_key = 'claude-code:v1:S1:R1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            stored.0, "2026-09-06T00:00:00.100Z",
            "the group completed at its last block"
        );
        assert_eq!(total(&pool).await.activity_count, 1);

        // Replaying the EARLIER block afterwards never moves it back.
        apply_scan(
            &pool,
            scan_claude_lines(&[first.to_string()], &[]),
            &attribution(),
            &cursor_for("fp", 3, 3),
            &[],
        )
        .await
        .unwrap();
        let stored: (String,) = sqlx::query_as(
            "SELECT occurred_at FROM model_usage_event WHERE event_key = 'claude-code:v1:S1:R1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(stored.0, "2026-09-06T00:00:00.100Z");
    }

    /// Events, coverage and the cursor land together or not at all (review
    /// C4): a poisoned cursor rolls back the events AND the coverage.
    #[tokio::test]
    async fn events_coverage_and_cursor_are_one_transaction() {
        let pool = connect_in_memory().await;
        let at = Utc.with_ymd_and_hms(2026, 9, 5, 10, 0, 0).unwrap();
        let coverage = vec![CoverageWrite {
            workspace_id: Some("ws".into()),
            workspace_agent_id: Some("agent-1".into()),
            source_kind: SOURCE_CLAUDE_TRANSCRIPT,
            start: at,
            end: at, // zero width on input …
            complete: false,
            diagnostic_code: None,
        }];
        let mut bad_cursor = cursor_for("fp", 10, 10);
        bad_cursor.byte_offset = -1; // violates CHECK (byte_offset >= 0)
        let result = apply_scan(
            &pool,
            scan_claude_lines(&[claude_row("R1", 0, "m", 1)], &[]),
            &attribution(),
            &bad_cursor,
            &coverage,
        )
        .await;
        assert!(result.is_err());
        assert_eq!(
            total(&pool).await.activity_count,
            0,
            "no event without its cursor"
        );
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM model_usage_coverage")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 0, "no coverage without its cursor");

        // The good path writes all three, and the interval has a visible width.
        apply_scan(
            &pool,
            scan_claude_lines(&[claude_row("R1", 0, "m", 1)], &[]),
            &attribution(),
            &cursor_for("fp", 10, 10),
            &coverage,
        )
        .await
        .unwrap();
        let found = model_usage::coverage_overlapping(
            &pool,
            &model_usage::canonical_ts(at),
            &model_usage::canonical_ts(at + chrono::Duration::milliseconds(1)),
        )
        .await
        .unwrap();
        assert_eq!(found.len(), 1, "… but stored half-open with nonzero width");
        assert_eq!(found[0].interval_end, "2026-09-05T10:00:00.001Z");
    }

    // ── Worker ───────────────────────────────────────────────────────────

    use chrono::TimeZone;

    struct Sandbox {
        root: PathBuf,
        config: WorkerConfig,
    }

    impl Sandbox {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "conclave-usage-worker-{}-{tag}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            let mut config =
                WorkerConfig::with_roots(root.join("claude-projects"), root.join("codex-sessions"));
            config.live_lag = Duration::ZERO;
            Self { root, config }
        }

        fn workspace_folder(&self, name: &str) -> String {
            let folder = self.root.join(name);
            std::fs::create_dir_all(&folder).unwrap();
            folder.to_string_lossy().into_owned()
        }

        fn write_claude(&self, folder: &str, file: &str, lines: &[String]) -> PathBuf {
            let dir = claude_project_dir(&self.config.claude_projects_root, Path::new(folder));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join(file);
            std::fs::write(&path, lines.join("\n") + "\n").unwrap();
            path
        }

        fn write_codex(&self, file: &str, lines: &[String]) -> PathBuf {
            let dir = self
                .config
                .codex_sessions_root
                .join("2026")
                .join("09")
                .join("05");
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join(file);
            std::fs::write(&path, lines.join("\n") + "\n").unwrap();
            path
        }
    }

    async fn workspace_with_agent(
        pool: &sqlx::SqlitePool,
        folder: &str,
        cli_kind: &str,
    ) -> (String, String) {
        use crate::engine::repo::agent_definition::AgentDefinitionInput;
        let ws = repo::workspace::create(pool, "WS", folder, None)
            .await
            .unwrap();
        let def = repo::agent_definition::create(
            pool,
            &AgentDefinitionInput {
                name: "Agent".into(),
                role: None,
                agent_type: "cli".into(),
                cli_kind: Some(cli_kind.into()),
                color: None,
                provider_id: None,
                model: None,
                harness_mode: "own".into(),
                share_blackboard: None,
                auto_submit_injected: None,
                allowed_senders: None,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let agent = repo::workspace_agent::instantiate(pool, &ws.id, &def.id)
            .await
            .unwrap();
        (ws.id, agent.id)
    }

    #[derive(sqlx::FromRow, Debug)]
    struct StoredScope {
        event_key: String,
        workspace_id: Option<String>,
        workspace_agent_id: Option<String>,
        session_id: Option<String>,
        generation: Option<i64>,
    }

    async fn stored(pool: &sqlx::SqlitePool) -> Vec<StoredScope> {
        sqlx::query_as(
            "SELECT event_key, workspace_id, workspace_agent_id, session_id, generation
               FROM model_usage_event ORDER BY event_key",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    #[derive(sqlx::FromRow, Debug)]
    struct CoverageRow {
        source_kind: String,
        workspace_agent_id: Option<String>,
        state: String,
        diagnostic_code: Option<String>,
        interval_start: String,
        interval_end: String,
    }

    async fn coverage(pool: &sqlx::SqlitePool) -> Vec<CoverageRow> {
        sqlx::query_as(
            "SELECT source_kind, workspace_agent_id, state, diagnostic_code, interval_start, interval_end
               FROM model_usage_coverage ORDER BY state, interval_start",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    fn force_rediscovery(worker: &ImportWorker) {
        worker.state.lock().unwrap().discovered_at = None;
    }

    /// A Claude transcript whose rows declare the workspace's cwd and carry
    /// the owner marker of an agent in that workspace is attributed to
    /// workspace, agent and session; history lands as partial coverage; a
    /// second tick is a no-op.
    #[tokio::test]
    async fn worker_imports_a_verified_claude_transcript() {
        let pool = connect_in_memory().await;
        let sandbox = Sandbox::new("claude");
        let folder = sandbox.workspace_folder("proj");
        let (ws, agent) = workspace_with_agent(&pool, &folder, "claude-code").await;
        let session = repo::session::get_by_instance(&pool, &agent)
            .await
            .unwrap()
            .unwrap();
        sandbox.write_claude(
            &folder,
            "S1.jsonl",
            &[
                claude_owner_marker(&agent),
                claude_row_at(&folder, "R1", 0, "claude-opus-5", 5),
                claude_row_at(&folder, "R1", 1, "claude-opus-5", 5),
                claude_row_at(&folder, "R2", 0, "claude-opus-5", 6),
            ],
        );

        let worker = ImportWorker::new(pool.clone(), sandbox.config.clone());
        let report = worker.run_tick().await.unwrap();
        assert_eq!(report.files_read, 1);
        assert_eq!(report.inserted, 2);
        assert!(!report.backlog);

        let events = stored(&pool).await;
        assert_eq!(events.len(), 2);
        for e in &events {
            assert_eq!(e.workspace_id.as_deref(), Some(ws.as_str()));
            assert_eq!(e.workspace_agent_id.as_deref(), Some(agent.as_str()));
            assert_eq!(e.session_id.as_deref(), Some(session.id.as_str()));
            assert_eq!(e.generation, None, "history proves no launch interval");
        }
        assert!(coverage(&pool)
            .await
            .iter()
            .any(|c| c.source_kind == "claude-code"
                && c.workspace_agent_id.as_deref() == Some(agent.as_str())
                && c.state == "partial"
                && c.diagnostic_code.is_none()));

        let again = worker.run_tick().await.unwrap();
        assert_eq!(again.files_read, 0);
        assert_eq!(stored(&pool).await.len(), 2);
    }

    /// Review C3: a personal transcript inside a workspace's project dir (no
    /// owner marker), a file whose rows declare another cwd (a colliding
    /// project slug), and a marker from an agent of ANOTHER workspace all
    /// import nothing — no guessed attribution, no unassigned activity.
    #[tokio::test]
    async fn worker_refuses_ownerless_foreign_cwd_and_cross_workspace_files() {
        let pool = connect_in_memory().await;
        let sandbox = Sandbox::new("attribution");
        let folder = sandbox.workspace_folder("proj");
        let (ws, agent) = workspace_with_agent(&pool, &folder, "claude-code").await;
        let other_folder = sandbox.workspace_folder("other");
        let (_other_ws, other_agent) =
            workspace_with_agent(&pool, &other_folder, "claude-code").await;

        // 1. Personal file in the project dir: right cwd, no marker.
        sandbox.write_claude(
            &folder,
            "personal.jsonl",
            &[claude_row_at(&folder, "P1", 0, "m", 1)],
        );
        // 2. Colliding slug: the file sits in this project dir but its rows
        //    declare a cwd that is no workspace.
        sandbox.write_claude(
            &folder,
            "collide.jsonl",
            &[
                claude_owner_marker(&agent),
                claude_row_at("/somewhere/else", "C1", 0, "m", 1),
            ],
        );
        // 3. Cross-workspace marker: right cwd, wrong workspace's agent.
        sandbox.write_claude(
            &folder,
            "cross.jsonl",
            &[
                claude_owner_marker(&other_agent),
                claude_row_at(&folder, "X1", 0, "m", 1),
            ],
        );

        let worker = ImportWorker::new(pool.clone(), sandbox.config.clone());
        let report = worker.run_tick().await.unwrap();
        assert_eq!(report.inserted, 0, "nothing is attributed by guessing");
        assert!(
            stored(&pool).await.is_empty(),
            "and nothing lands as unassigned"
        );
        // The cross-workspace marker is evidence of a damaged observation for
        // the workspace the rows declare.
        let cov = coverage(&pool).await;
        assert!(
            cov.iter().any(
                |c| c.diagnostic_code.as_deref() == Some("ownership_conflict")
                    && c.workspace_agent_id.is_none()
            ),
            "{cov:?}"
        );
        // A second tick re-reads none of them (judged, unchanged).
        let again = worker.run_tick().await.unwrap();
        assert_eq!(again.files_read, 0);
        let _ = ws;
    }

    /// A later, different owner marker never reassigns a file: the first
    /// verified owner keeps its events and the conflict makes coverage partial.
    #[tokio::test]
    async fn worker_never_reassigns_a_file_to_a_later_owner() {
        let pool = connect_in_memory().await;
        let sandbox = Sandbox::new("reassign");
        let folder = sandbox.workspace_folder("proj");
        let (_ws, agent_a) = workspace_with_agent(&pool, &folder, "claude-code").await;
        let (_ws2, agent_b) = workspace_with_agent(&pool, &folder, "claude-code").await;
        let path = sandbox.write_claude(
            &folder,
            "S1.jsonl",
            &[
                claude_owner_marker(&agent_a),
                claude_row_at(&folder, "R1", 0, "m", 1),
            ],
        );
        let worker = ImportWorker::new(pool.clone(), sandbox.config.clone());
        assert_eq!(worker.run_tick().await.unwrap().inserted, 1);

        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(
                format!(
                    "{}\n{}\n",
                    claude_owner_marker(&agent_b),
                    claude_row_at(&folder, "R2", 0, "m", 1)
                )
                .as_bytes(),
            )
            .unwrap();
        force_rediscovery(&worker);
        worker.run_tick().await.unwrap();
        let events = stored(&pool).await;
        assert_eq!(
            events.len(),
            1,
            "rows under a conflicting marker are not attributed to anyone"
        );
        assert!(
            events
                .iter()
                .all(|e| e.workspace_agent_id.as_deref() == Some(agent_a.as_str())),
            "the first owner keeps the file: {events:?}"
        );
        assert!(!events
            .iter()
            .any(|e| e.workspace_agent_id.as_deref() == Some(agent_b.as_str())));
        assert!(coverage(&pool)
            .await
            .iter()
            .any(|c| c.diagnostic_code.as_deref() == Some("ownership_conflict")));
    }

    /// Codex files are routed by the cwd their session_meta declares; an old
    /// file with only token_count rows is reported as unsupported.
    #[tokio::test]
    async fn worker_routes_codex_files_by_declared_cwd_and_flags_old_formats() {
        let pool = connect_in_memory().await;
        let sandbox = Sandbox::new("codex");
        let folder = sandbox.workspace_folder("proj");
        let (ws, agent) = workspace_with_agent(&pool, &folder, "codex").await;

        let mut ours = codex_fixture();
        ours[0] =
            codex_line(json!({"type": "session_meta", "payload": {"id": "SESS", "cwd": folder}}));
        ours[1] = codex_line(
            json!({"type": "response_item", "payload": {"type": "message", "role": "developer",
            "content": [{"type": "input_text", "text": format!("Your own agent id is {agent}.")}]}}),
        );
        ours[2] = codex_line(
            json!({"type": "turn_context", "payload": {"turn_id": "T1", "model": "gpt-5.6-sol", "cwd": folder}}),
        );
        sandbox.write_codex("ours.jsonl", &ours);
        sandbox.write_codex(
            "foreign.jsonl",
            &[
                codex_line(json!({"type": "session_meta", "payload": {"id": "OTHER", "cwd": "/somewhere/else"}})),
                codex_line(codex_usage_record("RESP-X", "T9", 1, 5)),
            ],
        );
        sandbox.write_codex(
            "old.jsonl",
            &[
                codex_line(json!({"type": "session_meta", "timestamp": "2026-09-01T00:00:00.000Z", "payload": {"id": "OLD", "cwd": folder}})),
                codex_line(json!({"type": "response_item", "timestamp": "2026-09-01T00:00:30.000Z", "payload": {"type": "message", "role": "developer",
                    "content": [{"type": "input_text", "text": format!("Your own agent id is {agent}.")}]}})),
                codex_line(json!({"type": "event_msg", "timestamp": "2026-09-01T00:01:00.000Z", "payload": {"type": "token_count", "info": {"last_token_usage": {"total_tokens": 5}}}})),
            ],
        );

        let worker = ImportWorker::new(pool.clone(), sandbox.config.clone());
        let report = worker.run_tick().await.unwrap();
        assert_eq!(
            report.inserted, 2,
            "RESP-1 and RESP-2 from our session only"
        );
        let events = stored(&pool).await;
        assert!(events
            .iter()
            .all(|e| e.workspace_id.as_deref() == Some(ws.as_str())));
        assert!(events
            .iter()
            .all(|e| e.workspace_agent_id.as_deref() == Some(agent.as_str())));
        assert!(
            !events.iter().any(|e| e.event_key.contains("RESP-X")),
            "foreign session skipped"
        );
        let cov = coverage(&pool).await;
        assert!(
            cov.iter().any(|c| c.source_kind == "codex"
                && c.state == "partial"
                && c.diagnostic_code.as_deref() == Some(UNSUPPORTED_SOURCE)),
            "the token_count-only file is an unsupported source: {cov:?}"
        );
        assert_eq!(worker.run_tick().await.unwrap().files_read, 0);
    }

    /// A file that shrinks is rescanned from zero without inflation.
    #[tokio::test]
    async fn worker_rescans_a_shrunk_file_without_double_counting() {
        let pool = connect_in_memory().await;
        let sandbox = Sandbox::new("shrink");
        let folder = sandbox.workspace_folder("proj");
        let (_ws, agent) = workspace_with_agent(&pool, &folder, "claude-code").await;
        let path = sandbox.write_claude(
            &folder,
            "S1.jsonl",
            &[
                claude_owner_marker(&agent),
                claude_row_at(&folder, "R1", 0, "m", 5),
                claude_row_at(&folder, "R2", 0, "m", 6),
                claude_row_at(&folder, "R3", 0, "m", 7),
            ],
        );
        let worker = ImportWorker::new(pool.clone(), sandbox.config.clone());
        assert_eq!(worker.run_tick().await.unwrap().inserted, 3);

        std::fs::write(
            &path,
            [
                claude_owner_marker(&agent),
                claude_row_at(&folder, "R1", 0, "m", 5),
                claude_row_at(&folder, "R2", 0, "m", 6),
            ]
            .join("\n")
                + "\n",
        )
        .unwrap();
        force_rediscovery(&worker);
        let report = worker.run_tick().await.unwrap();
        assert_eq!(report.files_read, 1, "shrink → rescan from zero");
        assert_eq!(report.inserted, 0, "R1 and R2 are replays");
        assert_eq!(
            stored(&pool).await.len(),
            3,
            "prior events are never deleted"
        );
        let cursor: (i64,) = sqlx::query_as("SELECT byte_offset FROM model_usage_cursor")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(cursor.0 as u64, std::fs::metadata(&path).unwrap().len());
    }

    /// Discovery is scoped: no transcript-backed agent → no candidates.
    #[tokio::test]
    async fn worker_has_no_candidates_without_transcript_agents() {
        let pool = connect_in_memory().await;
        let sandbox = Sandbox::new("empty");
        let folder = sandbox.workspace_folder("proj");
        repo::workspace::create(&pool, "WS", &folder, None)
            .await
            .unwrap();
        sandbox.write_claude(
            &folder,
            "S1.jsonl",
            &[claude_row_at(&folder, "R1", 0, "m", 5)],
        );
        let worker = ImportWorker::new(pool.clone(), sandbox.config.clone());
        assert_eq!(worker.run_tick().await.unwrap(), TickReport::default());
        assert!(stored(&pool).await.is_empty());
    }

    /// Review C2 through the production worker: with a small budget, one tick
    /// reads at most that many ACTUAL bytes, rotates fairly across files,
    /// reports backlog (→ pendingImport) and finishes over later ticks with no
    /// inflation. A record longer than a batch but under the cap still parses;
    /// one over the cap is skipped and the line after it survives.
    #[tokio::test]
    async fn worker_enforces_the_byte_budget_fairly_and_reports_backlog() {
        let pool = connect_in_memory().await;
        let sandbox = Sandbox::new("budget");
        let folder = sandbox.workspace_folder("proj");
        let (_ws, agent) = workspace_with_agent(&pool, &folder, "claude-code").await;
        let long_row = {
            let mut row: Value =
                serde_json::from_str(&claude_row_at(&folder, "LONG", 0, "m", 1)).unwrap();
            row["pad"] = json!("x".repeat(3_000));
            row.to_string()
        };
        let huge_row = {
            let mut row: Value =
                serde_json::from_str(&claude_row_at(&folder, "HUGE", 0, "m", 1)).unwrap();
            row["pad"] = json!("y".repeat(20_000));
            row.to_string()
        };
        for file in ["a", "b", "c"] {
            let mut lines = vec![claude_owner_marker(&agent)];
            for i in 0..6 {
                lines.push(claude_row_at(&folder, &format!("{file}-R{i}"), 0, "m", 1));
            }
            if file == "b" {
                lines.push(long_row.clone());
                lines.push(claude_row_at(&folder, "b-after-long", 0, "m", 1));
            }
            if file == "c" {
                lines.push(huge_row.clone());
                lines.push(claude_row_at(&folder, "c-after-huge", 0, "m", 1));
            }
            sandbox.write_claude(&folder, &format!("{file}.jsonl"), &lines);
        }
        let mut config = sandbox.config.clone();
        config.batch_bytes = 1_000;
        config.tick_budget_bytes = 2_500;
        config.max_record_bytes = 8_000;
        let worker = ImportWorker::new(pool.clone(), config);

        let first = worker.run_tick().await.unwrap();
        assert!(
            first.bytes_read <= 2_500,
            "tick budget is hard: {}",
            first.bytes_read
        );
        assert!(first.backlog, "budget exhausted → backlog");
        assert!(
            first.files_read >= 2,
            "fairness: several files share one tick"
        );
        assert!(
            model_usage::pending_cursor_scopes(&pool)
                .await
                .unwrap()
                .len()
                >= 1,
            "pendingImport is truthful while backlog remains"
        );

        let mut ticks = 1;
        loop {
            let report = worker.run_tick().await.unwrap();
            assert!(report.bytes_read <= 2_500);
            ticks += 1;
            if !report.backlog {
                break;
            }
            assert!(ticks < 200, "must converge");
        }
        let keys: Vec<String> = stored(&pool)
            .await
            .into_iter()
            .map(|e| e.event_key)
            .collect();
        // 6 rows × 3 files + LONG + b-after-long + c-after-huge.
        assert_eq!(
            keys.len(),
            6 * 3 + 3,
            "every row imported exactly once: {keys:?}"
        );
        assert!(
            keys.iter().any(|k| k.ends_with(":LONG")),
            "the multi-batch record parsed"
        );
        assert!(
            !keys.iter().any(|k| k.ends_with(":HUGE")),
            "the oversized record was skipped"
        );
        assert!(
            keys.iter().any(|k| k.ends_with(":c-after-huge")),
            "the line after it survived"
        );
        assert!(model_usage::pending_cursor_scopes(&pool)
            .await
            .unwrap()
            .is_empty());
        assert!(coverage(&pool)
            .await
            .iter()
            .any(|c| c.diagnostic_code.as_deref() == Some("oversized_record")));
    }

    /// Review C5: a live complete interval opens only after a tick that read
    /// every file of the scope; a disturbed tick closes the window, and the
    /// gap stays as two separate rows once proof resumes.
    #[tokio::test]
    async fn worker_live_coverage_opens_only_on_proof_and_keeps_gaps() {
        let pool = connect_in_memory().await;
        let sandbox = Sandbox::new("live");
        let folder = sandbox.workspace_folder("proj");
        let (_ws, agent) = workspace_with_agent(&pool, &folder, "claude-code").await;
        let path = sandbox.write_claude(
            &folder,
            "S1.jsonl",
            &[
                claude_owner_marker(&agent),
                claude_row_at(&folder, "R1", 0, "m", 1),
            ],
        );
        let worker = ImportWorker::new(pool.clone(), sandbox.config.clone());
        let complete = |rows: &[CoverageRow]| -> Vec<(String, String)> {
            rows.iter()
                .filter(|c| c.state == "complete")
                .map(|c| (c.interval_start.clone(), c.interval_end.clone()))
                .collect()
        };

        // Tick 1 reads the file fully: the scope is proven for [tick1, now).
        let r1 = worker.run_tick().await.unwrap();
        assert_eq!(r1.live_scopes, 1);
        let c1 = complete(&coverage(&pool).await);
        assert_eq!(c1.len(), 1);

        // Tick 2: a disturbed tick (unreadable file) closes the window.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir_all(&path).unwrap(); // a directory where the file was: unreadable
        let r2 = worker.run_tick().await.unwrap();
        assert_eq!(r2.live_scopes, 0, "no proof this tick");
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(
            &path,
            [
                claude_owner_marker(&agent),
                claude_row_at(&folder, "R1", 0, "m", 1),
            ]
            .join("\n")
                + "\n",
        )
        .unwrap();
        // Tick 3: proof resumes; the new window starts at tick 3, not tick 1.
        force_rediscovery(&worker);
        let r3 = worker.run_tick().await.unwrap();
        let c3 = complete(&coverage(&pool).await);
        assert!(r3.live_scopes >= 1);
        assert_eq!(
            c3.len(),
            2,
            "two windows, the gap between them retained: {c3:?}"
        );
        assert!(
            c3[1].0 > c1[0].1,
            "the second window starts after the first ended"
        );
    }
}
