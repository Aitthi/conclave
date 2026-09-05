# Persist and aggregate measured AI usage
owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop
Implementer: Dew (60ff2775-14a2-4db4-ab44-6df5bb13bf2a), allocated by Detoro. Aoki rules and merges. No source edits until ARCHIVE ENGINE MERGED is posted on workspace-archive-engine; start lane from main after that merge. Migrations 0030 and 0031 are reserved for this task. Execution slug usage-engine-v2 replaces the unclaimed usage-engine task after preflight boundary correction b1140da9; canonical plan path is unchanged.

## Reading order and authority
PRODUCT.md → docs/plans/2026-09-05-usage-overview-contract.md (final source ruling wins earlier prose) → docs/research/2026-09-05-usage-transcript-evidence.md → docs/research/2026-09-05-usage-contract-review.md → this plan. The original two-table logical-turn/attempt proposal is rejected. One event relation plus coverage/cursor persistence is the chosen design; small internal context/cursor metadata belongs with the cursor, not an analytics attempt platform.

## Deliverable and boundaries
Deliver durable usage collection for the supported Claude/Codex transcript shapes, direct chat/provider fusion calls, and non-persistent draft stdout, with bounded daily aggregation IPC. No UI, fixtures, cost estimation, credentials, raw prompt/output persistence, provider changes, real model calls, or app restart. Do not repurpose or sum the existing context gauge. Do not migrate existing context values into invented usage history.

Allowed source paths:
- src-tauri/Cargo.toml and src-tauri/Cargo.lock (chrono-tz only if no existing IANA-zone facility; no unrelated dependency updates).
- src-tauri/src/lib.rs and src-tauri/src/engine/mod.rs (minimal production worker startup/state integration).
- src-tauri/src/engine/db.rs, migrations/0030_model_usage.sql and migrations/0031_session_context_provenance.sql.
- src-tauri/src/engine/repo/mod.rs, repo/model_usage.rs and repo/session.rs.
- src-tauri/src/engine/runtime/mod.rs, runtime/usage.rs, runtime/transcript_usage.rs, runtime/transcript_context.rs, runtime/provider.rs, runtime/chat.rs, runtime/cli_oneshot.rs.
- src-tauri/src/engine/commands/mod.rs, commands/usage.rs, commands/instance.rs, commands/draft.rs, commands/fusion.rs, engine/router.rs.
- src/ipc/types.ts and src/ipc/commands.ts.
- This plan file (persist lead-authored plan only; substantive decisions amended by Aoki).
All source paths above under src-tauri/src/engine unless fully qualified otherwise. No whole-file formatting churn in shared modules. If another exact path is necessary, challenge before writing; do not silently widen scope. Frontend will consume this IPC read-only from an independent lane.

### First integration milestone
Implement schema/repository, truthful stored-data aggregation and the frozen IPC before collectors. Tests seed events/coverage and prove filters/timezones/unknowns; an empty production database honestly returns no coverage. Post USAGE FOUNDATION READY with a compiling, tested commit (no stub/fake-data responses). Aoki reviews and merges that ancestor commit to release frontend typing/fixtures while you continue collectors in the SAME lane. No changes to the agreed wire types after that point without an evidence-backed challenge. The task remains in progress until complete collector delivery; this milestone is not feature completion.

## Storage and event truth
Use model_usage_event with UNIQUE versioned source event_key, local immutable id, NULLABLE workspace_id, optional workspace_agent_id/session_id/generation, source session/request/response identity, source_kind and version, event_kind response|invocation, occurred_at/recorded_at, provider, requested_model/served_model, nullable normalized input_tokens/output_tokens and cache/read/write/reasoning subsets, validity/completeness and bounded diagnostic code. No text bodies, raw source JSON or secrets. Integer token fields are nonnegative; use checked addition and reject overflow/invalid counters as unknown with partial coverage.

input_tokens includes cache; output_tokens includes reasoning. Event display total is input+output only when both known. Expose known subtotals and missing usage counts, not a fabricated account total. A conflict on the same response identity never creates another activity; exclude a conflicting event from measured aggregation until reconciled evidence agrees, and retain only bounded metadata/provenance. Preserve evidence across replay/restart. Permanent workspace Delete may cascade usage rows; Archive/Restore never does. Historical joins include archived workspaces; normal aggregate scope excludes hidden internal workspaces.

Store durable source/scope coverage intervals and importer cursors (separate relations permitted; these are not attempt tables). Interval union must retain gaps rather than first_seen pretending continuous coverage. Unknown formats/read errors/conflicts/collector downtime remain partial or none. An event alone does not prove continuous coverage. Imported historical spans are conservatively partial unless complete observation is proven. Complete zero requires verified full-bucket coverage for every included source/scope; unsupported sources in that scope prevent complete. Today's end is now, not tomorrow; today is in progress and partial. No workspaces or never-collected scopes produce none, not a green zero.

## CLI transcript collector
Implement runtime/transcript_usage.rs as a separate importer, sharing only validated discovery/ownership helpers from transcript_context.rs. Preserve the current context reader's Arc cache, row timestamps, ownership/cwd checks, generation guards and two-second behavior. No duplicate home-tree scan per agent and no scans on output forwarding.

Use one shared production worker; test AppState does not start real filesystem scanning. Tick every 10 seconds; cache candidate discovery for 30 seconds; process a rotating bounded queue (at most 256 KiB per file per batch and 8 MiB per tick, yielding between batches). One worker may be nudged by usage.overview but queries never await a full import or spawn duplicates. Scope candidates to known workspace folders/agent owner IDs and the last 92 UTC dates (buffer for 90 local dates across timezone/DST boundaries); never select by mtime as event time. Old source records may need incremental scanning to reach recent data, always bounded. Return partial coverage while backlog remains. Use async/blocking isolation so filesystem reads and JSON parsing cannot stall terminal output/UI IPC.

Read complete newline-terminated JSONL only. Persist cursor offset, observed length, file/session fingerprint, source version, verified owner/cwd and minimal parser context. Commit events, coverage changes and cursor advance atomically. Restart, replay, duplicate rows, truncation and rotation must not inflate totals. Rescan on shrink/session change, retaining prior events. Ownership conflicts never reassign existing events; mark partial. Source generation is stamped only when an actual launch interval proves it; historical events with known owner but unknown launch generation use null.

Bound partial-line memory as well as per-tick I/O: at most 4 MiB for a source record. Oversized records are skipped through their newline with a bounded diagnostic and partial coverage, not repeatedly reread forever or logged. A record spanning several 256 KiB read batches must still parse correctly when under that limit. Cursor parser metadata must survive restart without retaining raw text.

Claude: key claude-code:v1:<sessionId>:<requestId>, message.id as agreement check. Require stable session/request/message IDs, a source timestamp and non-null stop_reason; tool_use is a completed response. Collapse content-block rows with identical usage/model. Cache-inclusive input is uncached+cache-create+cache-read when verified. Missing required components yield unknown tokens, not zero. Multi-attempt/fallback: top-level verified aggregate once, reported-model attribution unknown if mixed, selected model retained; do not sum iterations again. Conflict handling above applies across separate polling batches/restarts too.

Codex: key codex:v1:<session_id>:<response_id>; import ONLY top-level token_usage_record.payload.usage. Never import token_count, cumulative turn/thread counters or embedded latest_token_usage_record from compacted records. Join turn_context by turn_id for requested_model; served_model stays null without independent source proof. Cache/reasoning are subsets. Completed inner responses remain valid even if their outer turn later aborts. Old unsupported files remain explicit no coverage. Timestamp is the record timestamp, not filesystem metadata.

## Direct provider and one-shot collectors
Introduce terminal ProviderCompletion metadata (text plus optional response id, reported model and usage) without breaking text-only consumers: wrappers may preserve stream_chat/complete_chat compatibility while measured entry points expose metadata. Anthropic completion requires message_stop and terminal usage updates; OpenAI-compatible completion requires its actual terminal marker. Preserve streaming text, UTF-8 handling and output backpressure. Receiver loss/transport error/EOF before terminal is not completed activity. Request include_usage only on compatible OpenAI streaming paths; if a compatible server does not return usage, preserve successful content and record unknown usage. Do not add automatic request retries that can duplicate work.

Generate operation id before each request; dedup by versioned operation identity (provider response id is evidence, not a replacement that changes on late arrival). Runtime stays independent of database: use a metadata-only observer/channel drained by command layer. Persist one completed response per chat request and per fusion panel/judge/synthesis provider call; retain run/stage attribution in bounded metadata. Do not also count the fusion run envelope. Collection failure must not replace a successful answer with a user error; record bounded diagnostic/coverage gap, no silently healthy coverage.

One-shot runner returns structured value plus invocation id/requested model/terminal status/normalized usage metadata. Preserve the existing non-persistent CLI flags and error behavior. Claude parse terminal JSON envelope usage; Codex parse stdout turn.completed usage, retaining the existing last-message output path. One successful draft invocation yields event_kind=invocation, even when usage is unavailable. Failed/cancelled invocation is not activity. Never manufacture per-response identity from aggregate stdout. No transcript overlap because these invocations are ephemeral. Draft and fusion continue holding archive lifecycle guards introduced by archive engine.

## Frozen IPC: usage.overview
Add Commands["usage.overview"] and ipc.usage.overview(req), routed to commands/usage.rs. TypeScript camelCase mirrors serialized Rust. No new CLI verb required. Request:

```ts
type UsageOverviewRequest = {
  days: 30 | 90;
  timeZone: string; // required IANA zone, reject invalid
  workspaceId?: string;
  workspaceAgentId?: string;
  modelKey?: string;
};
type UsageCoverageState = "complete" | "partial" | "none";
type UsageModelBasis = "reported" | "selected" | "unknown";
type UsageTotals = {
  activityCount: number;
  responseCount: number;
  invocationCount: number;
  measuredTokens: number | null; // measuredEventCount>0 => sum; otherwise 0 ONLY when activityCount==0 AND coverage==complete; otherwise null
  measuredEventCount: number;
  unknownUsageCount: number;
  inputTokens: number | null; // known-component subtotal; null when none known
  outputTokens: number | null;
  coverage: UsageCoverageState;
};
type UsageDay = UsageTotals & {
  date: string; // YYYY-MM-DD in requested zone
  startUtc: string;
  endUtc: string; // next local midnight, capped at generatedAt today
  inProgress: boolean;
};
type UsageModelOption = { key: string; name: string; provider: string | null; basis: UsageModelBasis };
type UsageWorkspaceOption = { id: string; name: string; archived: boolean };
type UsageAgentOption = { id: string; name: string; workspaceId: string | null };
type UsageModelRow = UsageTotals & UsageModelOption;
type UsageAgentRow = UsageTotals & UsageAgentOption;
type UsageWorkspaceRow = UsageTotals & UsageWorkspaceOption;
type UsageContext = {
  workspaceAgentId: string;
  agentName: string;
  workspaceId: string;
  workspaceName: string;
  archived: boolean;
  modelKey: string;
  tokens: number | null;
  capacity: number | null;
  source: string | null;
  observedAt: string | null;
};
type UsageOverview = {
  generatedAt: string;
  range: { days: 30 | 90; timeZone: string; startDate: string; endDate: string; startUtc: string; endUtc: string };
  summary: UsageTotals;
  daily: UsageDay[];
  models: UsageModelOption[];
  agents: UsageAgentOption[];
  workspaces: UsageWorkspaceOption[];
  byModel: UsageModelRow[];
  byAgent: UsageAgentRow[];
  byWorkspace: UsageWorkspaceRow[];
  contexts: UsageContext[];
  coverage: { state: UsageCoverageState; collectingSince: string | null; lastVerifiedAt: string | null; pendingImport: boolean; unsupportedSources: string[] };
};
```

Model key is opaque stable encoding of provider, name and basis. If served_model exists choose reported; else requested_model yields selected; else unknown key. Separate selected/reported rows even for same name; UI always labels basis. Model options include identities from recorded events and current contexts. Return all nonhidden workspace/agent options (archived included) so filters stay discoverable even with no events; derive all metrics/breakdowns from the exact filters. Mismatched agent/workspace selection returns an empty scoped result, not unrelated data. Reject invalid day count/timezone and nonexistent supplied IDs with actionable Invalid/NotFound. Unknown valid modelKey gives empty result.

All daily buckets use actual IANA calendar midnights, half-open UTC ranges, trailing 30/90 dates including today. Exactly N ascending daily rows, no fabricated events. Context is latest per agent, independent of date range but identity-filtered; preserve latest timestamp/source and unknowns. Do not sum capacities. No raw event list/prompt data in IPC. Query using indexed timestamps/scope and aggregate bounded selected-range data; never return unlimited transcript rows or run filesystem scans inside the query.

## Preflight correction b1140da9 — credit Dew
D1: draft.agents from Library legitimately has no workspace (DraftRequest.workspace_id is optional). Default aggregation includes nonhidden workspace events PLUS NULL-workspace events. Reserve synthetic wire-only workspace id __unscoped__, name No workspace, archived=false, to label/filter/byWorkspace aggregate these records; no fake workspace database row and never pass this id to workspace lifecycle commands. A real workspace filter excludes unscoped events. Missing workspace-agent attribution similarly uses wire-only agent id __unassigned__, name Unassigned activity, workspaceId=null, in options/byAgent when needed; this bucket respects the selected real/unscoped workspace predicate. Agent filter __unassigned__ means workspace_agent_id IS NULL. These reserved ids bypass ordinary existence lookup only in usage.overview. Do not assign a draft to the current workspace or an arbitrary agent. Test unscoped and workspace-scoped drafts with no agent, and reconcile each grouping to the summary totals.

D2: preserve genuine context provenance. Add 0031_session_context_provenance.sql with nullable context_source and context_observed_at columns, NULL for old rows. Extend repo/session.rs and instance.rs persistence to retain TranscriptContextReading.source_kind/observed_at with its matching tokens/limit. Persist a newer source observation even when token counts are unchanged (do not substitute current clock or last_active_at); suppress identical observation writes. Legacy setter/clear/reset paths must clear obsolete provenance or retain only provenance that still describes the same reading. Do not relabel old rows as newly measured. Keep existing public Session payload compatible (provenance can remain internal and be exposed only by UsageContext). Migration 0030 remains usage event/coverage/cursor schema, 0031 is provenance. No source dropping/null-only workaround for future supported observations. Both paths are added to the recreated task boundary before claim. Archive engine never touches repo/session.rs and is merged first.

D3: byModel row coverage is explicitly the enclosing filtered source/scope coverage, equal to summary.coverage, NOT a per-model measurement guarantee. Retain the field for a uniform totals type; label coverage at scope level in UI. byAgent/byWorkspace may narrow to their actual observation scopes. Future true per-model coverage requires separate evidence and is outside this feature.

D4: explicit measuredTokens algorithm: if measuredEventCount>0 return the checked sum of fully known event input+output (which itself may equal0); else if activityCount==0 AND coverage==complete return0; else returnnull. Complete observation with existing events whose tokens are all unknown still returnsnull and unknownUsageCount>0. Coverage proves that events were observed, not that missing usage components are zero. This is a necessary correction to the proposed complete=>0 default. Component subtotals use the same complete-empty zero exception. Tests cover measured0, complete-empty0, partial-emptynull, none-emptynull, complete-with-unknown-eventsnull and mixed known/unknown subtotals.

## Foundation assembly clarification — Dew cfb1dc10

Aoki accepts Dew's four assembly decisions after checking the storage, scope, and D3/D4 contract. Hidden workspace IDs are excluded at the repository predicate; an explicit hidden workspace filter returns NotFound. NULL-workspace events remain included in the default scope, and archived workspaces remain included.

For coverage rows only, NULL workspace/agent dimensions mean unrestricted observation, not unattributed event ownership. A compatible narrower row can establish partial observation but cannot alone prove a broader query complete. Complete requires gap-free observation for every included source and scope; never infer the required source set only from whichever coverage rows happen to exist. A collector observing only unscoped events or one agent must not write a globally complete wildcard row. Unscoped/unassigned event filters retain D1 semantics. Test narrower-versus-global proof and missing-source coverage.

The unmerged 0030 coverage table may add nullable diagnostic_code. Derive unsupportedSources from compatible in-range unsupported_source diagnostics; these prevent complete coverage for that scope. Derive pendingImport from relevant persisted cursor backlog (byte_offset < observed_length), never a fabricated worker state. Unrelated hidden or filtered-out scopes must not contaminate these diagnostics.

Today's daily row remains inProgress and never complete: preserve none when no observation exists, otherwise cap it at partial. Summary coverage describes the actual half-open query interval ending at generatedAt, and may be complete only with full observation across that entire interval for every included source/scope. This permits D4's fully observed empty summary to show zero without claiming the unfinished calendar day is complete. Test summary complete with today's row partial, missing-source/gap rejection, and no-coverage empty results. D3 model rows still inherit summary coverage. No wire types change.

## Acceptance and gates
Use sanitized deterministic fixtures and mocked provider/oneshot streams only. Tests must prove migration from schema29 preserves populated graph and starts with no invented events; unique replay/crash/cursor transaction behavior; Claude repeated blocks/conflicts; Codex compaction/model change/aborted outer turn; older unsupported formats; missing/cache/reasoning normalization and overflow; direct terminal/cancellation/UTF-8; one-shot duplicate avoidance; archive historical inclusion/restore invariance/permanent delete; filters/model basis; unknown vs complete-zero vs partial buckets; IANA DST spring/fall days; context independence; worker shared/bounded scheduling with no real home scanning. At least one regression demonstrates replay cannot double count through the production importer/repository path.

Run focused source/aggregate tests, full cargo test --manifest-path src-tauri/Cargo.toml, pnpm build and scoped whitespace/semantic diff checks. Verify no raw transcript content/secrets are stored or logged. UI pixel gate does not apply (no src UI). Record USAGE ENGINE READY with commit, wire contract, gates, supported/unsupported source shapes, coverage behavior and any remaining limitation. Aoki independently reviews/retests/merges. Do not self-merge or release another lane.
