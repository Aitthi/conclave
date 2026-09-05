# Usage measurement contract review

Date: 2026-09-05  
Reviewer: Armin (`be81029a-bde1-4d64-ad03-d3079cb19603`)  
Decision owner: Aoki (`2004f459-52ad-445c-9c70-e605a0ffdfe3`)

## Verdict

**Fix-then-ship the one-event proposal.** A single response/usage-event relation is the right minimum for the first Overview; the proposed logical-turn plus attempt relation is premature for the observed supported paths. It needs two additions before it can truthfully distinguish measured zero from unknown:

1. A durable source-coverage watermark (a small separate state relation, or a non-activity `coverage_opened` record in the same event log).
2. A source-specific stable key and terminal-completion rule. The current context readers cannot supply either for Codex, and must not be repurposed as an activity importer.

This is smaller than a general billing/attempt platform: no price, no inferred cost, no prompt or output retention, and no attempt relation until a real collection seam returns a multi-attempt payload that must be kept.

## Evidence traced

The existing persisted `session` fields are overwritten latest context gauges, not a history (`docs/research/2026-09-05-workspace-overview-archive.md:31-45`). The polling path only writes when its `tokens` or `limit` changes (`src-tauri/src/engine/commands/instance.rs:1632-1694`). Therefore its two-second samples cannot be summed or counted as activity.

For Claude Code, `ClaudeAcc` currently retains just the last assistant usage row (`src-tauri/src/engine/runtime/transcript_context.rs:473-516`) and accepts a transcript only after its workspace and owner marker checks. Test fixtures demonstrate a top-level `requestId`, `message.id`, assistant `message.model`, component usage, and timestamp (`:964-1000`, `:2478-2489`). A duplicate `requestId` can have a different `message.id` (`:1243-1280`), so `message.id` is not a safe replacement deduplication key without further corpus validation.

For Codex, `CodexAcc` deliberately selects `payload.info.last_token_usage.total_tokens` (`transcript_context.rs:586-674`); the test fixture also contains a deliberately rejected `total_token_usage` (`:922-935`). Neither the current reduction nor its public reading contains a turn/response id or served model. It is a context gauge, not an additive event feed.

For direct chat, `Provider::stream_chat` returns only a `String` (`runtime/provider.rs:182-236`). The Anthropic and OpenAI SSE parsing functions discard all non-text payload data (`:246-273`), and `consume_sse` reports `Ok` when its output receiver disappears before the stream terminates (`:379-411`). The chat loop consequently has no terminal provider result to persist (`runtime/chat.rs:47-73`). Any direct-chat collector has to change that result contract; attaching a row to text chunks would create duplicate and cancelled activity.

## Minimal durable contract

### `model_usage_event`

One row represents one **observed completed response** or one **stable, deduplicated provider usage record**. It is the only activity source. Required fields are:

| Field | Contract |
|---|---|
| `id` | Locally generated immutable UUID. |
| `event_key` | Stable, source-namespaced idempotency key; `UNIQUE`. |
| identity | `workspace_id`, nullable `workspace_agent_id`/`session_id`, and nullable launch `generation` captured at collection time. |
| source | Bounded `source_kind` and `source_generation`/collector version. |
| completion | `status` (`completed`, `failed`, `cancelled`), provider/source `occurred_at` where available, and `recorded_at`. |
| model | Nullable `requested_model`, nullable `served_model`, and `provider`. |
| usage | Nullable `input_tokens`, `output_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens`, plus a bounded, explicitly-versioned field only when its provider semantics are known. |
| provenance | `usage_source`, `usage_observed_at`, and an explicit completeness/unknown state. |

The primary heatmap is **Model activity: distinct completed `model_usage_event`s**, grouped by `occurred_at`. It is neither user turns, terminal chunks, provider billing, nor a sum of context samples. A fallback that can only expose one final response produces one event. If a future provider exposes multiple attempts, either (a) emit one response event per completed attempt and label activity accordingly, or (b) add an attempt relation only when product needs one logical activity count plus per-attempt token reporting. The latter is the condition for the research proposal's two-table model, not a first-release prerequisite.

There must be no synthetic `total_tokens` column. A display total is computed only from components whose provider meanings establish that they are additive; unknown components keep the display total unknown. `context_after_tokens` and `context_limit` remain the separate latest context gauge and never participate in time-series totals.

### Coverage state is separately required

The event table alone cannot prove that an empty date bucket is zero: before an agent ever produces a row, the absence could mean no use, a disabled collector, or unsupported source. Persist a compact `model_usage_coverage` state keyed by `workspace_id`, nullable agent/session scope, and `source_kind`, with `collecting_since`, `last_verified_at`, and a bounded capability state. An equivalent non-activity coverage record in the event log is acceptable if it has the same queryable semantics and is excluded from activity/tokens.

Coverage is **not** an attempt table. It is required by the declared unknown-versus-zero behaviour: a zero may be displayed only after the relevant collector reports coverage for the complete bucket/range. Historical dates, unsupported paths, collector outages, and unvalidated import gaps are `no coverage`, never zero. The initial default is 90 calendar days in the requested IANA timezone, using half-open UTC ranges; local calendar boundaries, rather than `24h` arithmetic, handle DST.

## Identity and collection rules

| Source | Event key and completion rule | Token/model truth at first release |
|---|---|---|
| Anthropic direct chat | Generate a local operation UUID before the request; after `message_start` capture provider message id/model and commit once only after `message_stop`. Use provider id when present, namespaced with the local collector/source. | Capture requested model at dispatch and served model plus official component usage from terminal stream events. A receiver drop or transport error is cancelled/failed, never completed activity. |
| OpenAI-compatible direct chat | Local operation UUID is the fallback key; use streamed response `id` only if supplied. Commit on a terminal stream completion, not on a delta. | Request `stream_options.include_usage` where supported; take usage/model only when the provider actually supplies them. Otherwise completed activity with unknown usage and nullable served model is honest. |
| Claude Code transcript import | Only after a dedicated importer validates a file/session namespace plus top-level `requestId` as a stable response key. On replay, upsert/reconcile the same key by source ordering; do not create another activity event. | `message.model` is candidate served model and assistant usage components are candidates for measured usage. Imported coverage is partial and begins at the validated cursor/range. A missing `requestId` is not imported as activity. |
| Codex transcript | **No importer from current `token_count`.** It lacks a stable completed-response id in the observed reader. | Keep only the latest context gauge. Codex history/activity remains unknown until a dedicated reader proves a completion identity and component semantics. |
| Draft one-shot | Generate one local operation UUID per invocation and record completion only when the CLI operation returns successfully. | Requested configuration is not served identity. Token and served model remain unknown unless the invoked CLI exposes a verified result. |
| Fusion panel, judge, synthesis | Generate one local operation UUID for each provider invocation, scoped by durable run id + stage + ordinal; do not count the enclosing `fusion_run` as a response. | Capture the same direct-provider terminal metadata where available; otherwise retain completed activity with unknown usage. |

The local operation UUID distinguishes an application invocation, not an unobservable provider retry after a crash. A request whose transport outcome is unknown must not be silently converted into a completed event. The unique key makes stream replay and the incremental transcript scanner idempotent; it does not invent exactly-once provider execution.

For bounded incremental import, keep an importer cursor per transcript file (stable file/session namespace plus byte offset/revision) and commit candidate events with their source key in the same database transaction as cursor advance. The present `TranscriptContextReader` scan state is intentionally a reduction to one latest value and cannot be reused as that cursor. Metadata-only writes also invalidate file mtime as an event timestamp (`transcript_context.rs:519-548`); use the source row timestamp, falling back only to an explicitly partial import record.

## Query and archive behaviour

Aggregate only `completed` rows by actual `served_model`; requested model is a secondary diagnostic dimension. Return separate component totals, measured and unknown event counts, coverage state/range, and the latest context observations. Do not infer money or coerce a missing component to zero. A model filter must offer an explicit `unknown served model` bucket so configured model does not masquerade as served model.

Archive preserves and reads these rows. Historical usage from archived workspaces remains in the default Overview, marked archived; archive neither starts collectors nor rewrites coverage. This agrees with the archive contract at `workspace-overview-archive.md:193-209` and prevents archive status from becoming a retroactive data filter.

## Review findings

### Major — the “only second table for attempts” claim cannot meet measured-zero semantics

**Evidence.** The review plan requires honest unknown-versus-zero and history-start behaviour. The existing research contract says a zero is valid only when coverage proves observation and returns source `coverageStart` (`workspace-overview-archive.md:143-155`). An event-only relation contains no fact for a collector-enabled period with zero completed responses.

**Resolution.** Retain Aoki's one usage-event relation, but add the compact coverage state described above (or equivalent excluded coverage events). Do not add `model_usage_attempt` until a supported payload needs it.

**Default if unanswered.** Treat all empty buckets as `no coverage` rather than measured zero until coverage state exists.

### Major — current Codex and stream paths cannot support the proposed generic completed-event assertion

**Evidence.** Codex exposes only a last-token gauge with no response key or model (`transcript_context.rs:647-673`). Provider streams currently discard identity, usage, and terminal state, and return success on receiver drop (`provider.rs:246-273`, `379-411`).

**Resolution.** Gate each source behind the rules in the identity table. Do not enable Codex historical activity; refactor provider output to a terminal result before enabling direct-chat, fusion, or fallback collection.

**Default if unanswered.** Launch with direct paths only after terminal metadata is collected; Claude import remains explicitly partial; Codex remains unknown.

### Major — `source_event_id` alone needs a source namespace and replay rule

**Evidence.** Claude fixtures show one `requestId` with changing `message.id` (`transcript_context.rs:1243-1280`), while the current reader discards both and keeps only a scalar (`:473-516`). A bare provider id/request id has no stated file/session or collector namespace, and append-only duplicate inserts would inflate activity.

**Resolution.** Make `event_key` a versioned source namespace plus verified stable response identity, define one conflict policy (idempotent no-op for the same immutable terminal result; source-order reconciliation for a validated transcript record), and test a replay plus duplicate line.

**Default if unanswered.** No transcript backfill is enabled; prospective events use locally generated operation UUIDs plus provider ids when available.

## Decisions requested from Aoki

1. Approve one `model_usage_event` relation plus durable coverage state; defer the logical-turn/attempt pair.
2. Define the heatmap label as **Model activity — completed measured response events**, with unknown source coverage visibly distinct from zero.
3. Hold Codex transcript activity/totals at unknown until a dedicated parser proves event identity; retain its current context gauge separately.
4. Make archive a historical-read filter only: it preserves rows and includes them by default, labelled archived.
