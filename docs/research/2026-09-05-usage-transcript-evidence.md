# Claude and Codex transcript usage evidence

Date: 2026-09-05

Researcher: Marty (`0ed6b21b-8322-46c6-868c-8df84218bd30`)

Decision owner: Aoki (`2004f459-52ad-445c-9c70-e605a0ffdfe3`)

## Verdict

The installed CLI formats contain substantially more evidence than the current context reducer exposes.

- Claude Code `2.1.261` transcripts support a conservative response importer. Within one transcript session, `(sessionId, requestId)` identifies one model response; Claude writes one assistant row per API content block, repeating the same `message.id`, model, stop reason, and full usage on every row. Collapse the group once. `message.usage` is per-request usage, not a session cumulative, and is additive across distinct requests after deduplication.
- Codex CLI `0.153.4` writes a top-level `token_usage_record` for every completed upstream response. It contains stable `response_id`, session/thread/turn ids, exact per-response `usage`, and cumulative turn/thread totals. The current `event_msg.payload.type = token_count` is only the older context/cumulative view; it must not be the usage importer.
- Both sources can populate a one-row `model_usage_event` contract now. They do not require a speculative attempt table for the observed non-fallback case. Claude fallback remains the case that may later require per-attempt storage.
- Draft one-shots are deliberately non-persistent (`claude --no-session-persistence`, `codex exec --ephemeral`). Their stdout result/JSONL is therefore the only collection seam and does not overlap the normal transcript importer. The current runner discards that metadata.

This supersedes the earlier “Codex activity unknown” conclusion in `2026-09-05-workspace-overview-archive.md`. It does not change the separate current-context gauge or its attribution/performance constraints.

## Method and privacy boundary

I inspected only metadata from the exact roots in `TranscriptContextConfig::default_with_limit` (`runtime/transcript_context.rs:49-57`): the codeup Claude project directory and the 2026-09-05 Codex session date directory. Scripts allowlisted event types, hashed ids/paths, timestamps, model names, stop reasons, usage field names/numbers, and structural key shapes. They never printed or stored prompts, responses, system/developer instructions, tool arguments/results, credentials, or raw JSONL lines. No model request, CLI update, app restart, or installed-tool change was made.

Installed versions observed read-only: Claude Code `2.1.261`; Codex CLI `0.153.4`.

## Claude Code evidence

### Metadata shape and identity

Sanitized fixture sketch (content deliberately reduced to block types):

```json
{
  "type": "assistant",
  "sessionId": "S",
  "requestId": "R",
  "apiBlockIndex": 0,
  "timestamp": "RFC3339 UTC",
  "message": {
    "id": "M",
    "model": "claude-fable-5-1",
    "stop_reason": "tool_use",
    "contentTypes": ["thinking"],
    "usage": {
      "input_tokens": 2,
      "cache_creation_input_tokens": 27984,
      "cache_read_input_tokens": 26445,
      "output_tokens": 433
    }
  }
}
```

In the eight newest sampled files at the fixed observation:

- 1,844 assistant rows with usage collapsed to 989 distinct `requestId` groups.
- 605 request groups had multiple rows (up to seven observed). Every repeated group had one `message.id`, one model, one stop reason, and one identical usage object. The rows differed by top-level UUID, timestamp, `apiBlockIndex`, and content-block type.
- No sampled assistant usage row lacked `requestId`, `message.id`, top-level timestamp, or non-null `stop_reason`.
- No request id crossed files, no request group contained multiple message ids, and no message id belonged to multiple requests in the sample.
- Each of the eight files carried exactly one `sessionId`/`session_id`, and that id matched the JSONL basename.

One four-row group showed indices 0–3 and block types `thinking`, `text`, `tool_use`, `tool_use`; all four repeated the same request id, message id, model, `stop_reason = tool_use`, and four usage components. Counting lines would have multiplied one response fourfold.

The synthetic test `claude_dedupes_duplicate_assistant_usage` (`transcript_context.rs:1242-1304`) permits the same request id with a different message id and usage, but the live sample did not exhibit that shape. Treat it as a conflict/revision case, not evidence that either identifier is globally unstable.

### Usage and completion semantics

Anthropic documents a response `usage` as billing/rate-limit usage for that request and states that total input is `input_tokens + cache_creation_input_tokens + cache_read_input_tokens`; `output_tokens` is the response output. See [Messages API reference](https://platform.claude.com/docs/en/api/typescript/messages). Therefore:

```text
measured_input = input_tokens + cache_creation_input_tokens + cache_read_input_tokens
measured_total = measured_input + output_tokens
```

These components are additive across **distinct request groups**. They are not cumulative session totals. The current context reader's `sum_claude_usage` (`transcript_context.rs:829-846`) uses the same formula only to display the latest request footprint; that latest-value reduction must remain separate from historical summation.

A group is importable when it has an accepted owner marker, session id, request id, message id, model, timestamp, usage components, and non-null stop reason. `tool_use` is a completed model response even though the agent will make another request after executing the tool; `end_turn` is only the outer agent-turn boundary. Use the maximum row timestamp/api-block index in the group as its recorded completion time.

`message.model` is the response's model identity candidate. Every sampled usage row also carried `usage.iterations`, but all sampled arrays had one entry and no iteration model/fallback marker. Anthropic documents multi-attempt fallback in `usage.iterations`; if such a row appears, the one-row collector may preserve the completed response and top-level totals/model, but per-attempt per-model totals are partial until an attempt representation is approved. See [Anthropic fallback semantics](https://platform.claude.com/docs/en/build-with-claude/refusals-and-fallback).

### Claude event key and reconciliation

Recommended key:

```text
claude-code:v1:<sessionId>:<requestId>
```

Store `message.id` separately as `provider_response_id` and require it, model, stop reason, and usage to agree across repeated rows. On agreement, insert/upsert one immutable event. On disagreement, do not create a second event: retain the latest complete group as a pending conflict, mark source coverage partial, and surface a diagnostic counter. A bare `requestId` or `message.id` is not assumed globally namespaced.

## Codex evidence

### The decisive record is `token_usage_record`

Sanitized fixture sketch from the installed rollout format:

```json
{
  "type": "token_usage_record",
  "ordinal": 473,
  "timestamp": "RFC3339 UTC",
  "payload": {
    "response_id": "RESP",
    "session_id": "SESSION",
    "thread_id": "THREAD",
    "turn_id": "TURN",
    "root_turn_id": "TURN",
    "usage": {
      "input_tokens": 210724,
      "cached_input_tokens": 209536,
      "cache_write_input_tokens": 0,
      "output_tokens": 1250,
      "reasoning_output_tokens": 358,
      "total_tokens": 211974
    },
    "turn_token_usage": { "...": "cumulative within turn" },
    "thread_token_usage": { "...": "cumulative within thread" }
  }
}
```

The same file has:

- `session_meta.payload.id`, cwd, CLI version, and model provider;
- `turn_context.payload.turn_id` plus the configured model for that turn;
- `event_msg` `task_started`/`task_complete` for the outer user turn;
- `event_msg` `token_count` containing `last_token_usage` and `total_token_usage` without a response id.

Stable completed file `63ac6fcf9de9` (path hashed) contained 82 top-level usage records: 82 unique non-null response ids and 82 unique ordinals. For all 81 records after the first, the delta of cumulative `thread_token_usage` exactly equalled that record's `usage`; every `total_tokens` equalled `input_tokens + output_tokens`. The file contained a compaction and continued with unique response ids and monotonic cumulative totals afterward.

Compaction is a double-counting trap: the top-level `compacted` record embeds `latest_token_usage_record`. Import only records whose **top-level** `type` is `token_usage_record`; never recursively search for a usage object or response id.

A second stable file `7a3272cb1a95` proved model changes within one persisted session: 56 unique response records mapped by `turn_id` to `turn_context`; 26 mapped to `gpt-5.6-sol` and 30 to `gpt-6-astra`, with zero unmapped. Session cumulative differences cannot be assigned to a model after such a change; per-response records joined to their turn context can.

OpenAI's Codex source confirms that `response.completed` yields the response id and exact upstream token usage, and that response ids are the API response bookmarks. See [Codex SSE parser](https://github.com/openai/codex/blob/main/codex-rs/codex-api/src/sse/responses.rs) and [protocol description](https://github.com/openai/codex/blob/main/codex-rs/docs/protocol_v1.md). The transcript's `turn_context.model` is the model selected for the turn; the sampled `token_usage_record` does not carry an independently server-reported served model. Store it as `requested_model`; leave `served_model` null unless a future stable field proves it.

### Codex event key and formulas

Recommended key:

```text
codex:v1:<session_id>:<response_id>
```

The event timestamp is the top-level record timestamp. Join `turn_id` to the matching `turn_context` for requested model. Import `usage` directly; do not derive it from cumulative differences when the per-response object exists.

Codex component relations observed and reflected in OpenAI's source:

```text
total_tokens = input_tokens + output_tokens
cached_input_tokens and cache_write_input_tokens are subsets/details of input_tokens
reasoning_output_tokens is a subset/detail of output_tokens
```

Thus the display total is `input_tokens + output_tokens`; never add cached/reasoning again. `turn_token_usage` and `thread_token_usage` are validation/cursor aids, not additional billable rows.

`last_token_usage.total_tokens` is the latest context size; `total_token_usage` is accumulated session usage (also documented in `codex-rs/tui/src/token_usage.rs`). `task_complete` is a completed user turn, not a model-response identity. One turn can contain many response records, and an aborted outer turn may still contain already completed, valid usage records. Heatmap activity should count the response records if it is labelled model activity.

## Incremental importer contract

Keep the existing `TranscriptContextReader` unchanged for the live gauge. Add a separate metadata-only importer with these rules:

1. Scope discovery exactly as today: Claude by cwd project dir; Codex by date tree, then strict cwd plus owner-marker validation (`claude_value_declares_owner` / `codex_value_declares_owner`). Do not broaden the two-second context poll.
2. Read only complete newline-terminated records. Keep per-source-file cursor state: source kind, source session id, path fingerprint, byte offset, observed length, collector version, and last verified timestamp.
3. Insert usage events and advance the cursor in one database transaction. A crash before commit replays safely; the unique event key turns replay into a no-op/reconciliation.
4. If a file shrinks, rotates, or its session identity changes, reset its cursor and rescan from zero. Never delete prior events; unique keys prevent duplicates.
5. Claude: accumulate rows by `(sessionId, requestId)` and finalize/reconcile after the highest complete block observed. Codex: emit each top-level `token_usage_record` immediately; ignore `token_count`, embedded compaction copies, and `token_usage_record`-like nested shapes.
6. Attribute to a workspace agent only after the existing structural owner marker and cwd checks succeed. Persist `source_session_id` and source turn/request ids. Prospective collection may stamp the current runtime generation; historical import must leave generation null when no durable launch interval proves it. Old events remain history but never become the current generation's context reading.
7. Coverage begins at the first fully scanned, owner-validated source interval. Conflict, malformed terminal metadata, unreadable gaps, unsupported older Codex files, or cursor loss makes coverage partial/unknown—not zero.

Metadata-only fixtures should cover: Claude multi-block duplicate rows, request conflict, missing timestamp/id/usage, tool-use then end-turn responses, fallback iterations; Codex per-response/cumulative equality, duplicate response replay, embedded compaction copy, model change by turn id, outer turn abort, truncation/rescan, and older files without top-level usage records.

## Draft one-shot overlap and collection

`runtime/cli_oneshot.rs:79-112` launches:

- Claude: `claude -p --output-format json ... --no-session-persistence --tools ''`.
- Codex: `codex exec --json --ephemeral ... -o <last-message>`.

Neither writes a normal persistent transcript, so the regular importer cannot double-count it. Current `run_live` (`:194-255`) captures stdout but reduces Claude to only the structured result and ignores Codex stdout entirely after reading the last-message file.

Claude JSON result output includes a terminal result envelope with `session_id`, aggregate `usage`, `num_turns`, and success/error metadata; it does not provide a verified per-request response id in the documented result shape. The existing parser already receives this envelope. See [Anthropic's Agent SDK result example](https://platform.claude.com/cookbook/claude-agent-sdk-07-hosting-the-agent). Codex officially documents that `--json` stdout is JSONL and `turn.completed` carries aggregate usage; `--ephemeral` disables rollout persistence. See [Codex non-interactive mode](https://developers.openai.com/codex/noninteractive).

Change the runner result, not the transcript reader: return `{value, invocation_id, requested_model, usage, source_terminal_id, completion_status}`. Generate `invocation_id` before spawn. For Claude, parse the result envelope's aggregate usage; for Codex, parse exactly one successful `turn.completed` and its usage from stdout. This is an **invocation/outer-turn usage event**, not proof of each internal provider response. Record one event with source kind `draft_oneshot`; leave served model null unless the envelope proves it. Exit zero without a terminal metadata event may still be a successful draft result, but usage remains unknown.

If persistence flags are ever removed, namespace the one-shot operation id into the transcript/session and choose one authoritative source before enabling import; never write both invocation aggregate and its internal response rows into the same additive total.

## Minimal implementation delta

The one-event-plus-coverage design from `2026-09-05-usage-contract-review.md` remains sufficient. Required collector work:

- `runtime/transcript_context.rs`: leave gauge path intact; factor only shared safe discovery/ownership helpers if needed.
- New `runtime/transcript_usage.rs`: Claude grouping/reconciliation, Codex top-level usage-record parser, cursor/coverage output.
- `repo/model_usage.rs` plus migration: unique source-namespaced key, token components, requested/served model distinction, source ids/version, completion timestamp, completeness; separate coverage/cursor relation.
- `commands/instance.rs`: schedule bounded import outside the two-second gauge reducer and never block output forwarding.
- `runtime/cli_oneshot.rs` and `commands/draft.rs`: preserve terminal stdout metadata and write one non-overlapping invocation event.

Implementation gate must use sanitized fixtures only; production transcript content must never enter tests, logs, task notes, or the usage database.

## Rulings for Aoki

1. Approve Claude transcript import keyed by session plus request id, with message id as an agreement check and one event per completed request—not one row/block or one outer agent turn.
2. Approve Codex `token_usage_record` import keyed by session plus response id; retract the earlier “Codex importer unprovable” default for CLI `0.153.4+`. Keep older `token_count`-only history unknown.
3. Label Codex's `turn_context.model` as requested/selected model, not independently verified served model.
4. Treat draft usage as one non-persistent invocation event collected from stdout. It cannot be mixed with internal response counts without an explicit unit change.
5. Keep coverage versioned by CLI/source format so upgrading or losing a collector creates an explicit partial interval rather than a false zero.
