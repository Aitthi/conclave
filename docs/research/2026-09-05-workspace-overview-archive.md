# Usage Overview and reversible workspace archive — discovery

Date: 2026-09-05

Researcher: Marty (`0ed6b21b-8322-46c6-868c-8df84218bd30`)

Decision owner: Aoki (`2004f459-52ad-445c-9c70-e605a0ffdfe3`)

Scope ruling: `ruling:overview-scope` — Overview is AI model usage across workspaces; archive behavior is independent.

## Executive conclusion

The current database cannot produce an honest historical token-usage Overview. `session.context_tokens` is one mutable current-context gauge per agent, not a cumulative ledger, and its meaning differs by harness. Direct chat discards provider usage; CLI transcript readers persist only the latest gauge; model identity is mutable configuration rather than the model actually served. Existing durable records (`message`, `inter_agent_message`, `task_event`, `fusion_*`, `artifact`, `memory_*`) do not form a complete or deduplicable model-turn log. The removed proxy experiment tables are global, partial, and unattributed.

The smallest defensible basis for Overview is prospective instrumentation at model-response completion: an append-only logical turn row plus per-attempt usage rows. The heatmap should count completed logical model turns, not terminal chunks or context samples. Token totals must carry measured/partial/unknown coverage, and current context must remain a separate gauge.

Workspace archive is safe as a separate nullable `workspace.archived_at`. Archive should run under the workspace lifecycle write lock, reject any live runtime, preserve all records and files, normalize the workspace to stopped, and hide it from normal navigation. Restore clears `archived_at` but does not launch anything. Launch eligibility must be enforced centrally so UI, IPC, CLI, detached restart, draft, and fusion routes cannot bypass archive state.

## 1. Usage Overview

### 1.1 Product and visual constraints

`PRODUCT.md` describes a quiet, precise operational tool with native familiarity, explicit state/attribution, durable records, and WCAG AA contrast. The Overview should therefore expose provenance and unknown coverage rather than present guessed totals as facts; it should avoid marketing-style KPI cards or invented cost estimates.

The human reference at `docs/research/2026-09-05-overview-activity-reference.png` (commit `909ed66`) was opened and visually inspected. It is a dark, label-free, wide grid of small rounded cells with nine rows, many columns, and a single indigo intensity hue. It establishes the visual idiom, not the data definition. The implementation should retain accessible labels/tooltips and a non-colour value cue even if the compact grid follows that appearance.

### 1.2 Confirmed current telemetry

#### Persisted session state is a latest-value gauge

- Migration `0001_init.sql:48-55` creates one `session` per `workspace_agent` (`workspace_agent_id UNIQUE`) with only `context_tokens`, `context_limit`, `started_at`, and `last_active_at`.
- `repo/session.rs:127-160` initializes `context_tokens = 0`; `started_at` is session-row creation, not every process launch or model turn.
- `repo/session.rs:199-242` overwrites the same context fields and stamps `last_active_at` with write time. It does not retain provider observation time, source, confidence, generation, request/turn identity, token breakdown, or served model.
- `repo/session.rs:247-257` correctly clears both context fields for Antigravity because no trustworthy source exists. This establishes the product precedent: unknown is `NULL`, not zero.
- A session row exists for every instantiated agent, including stopped and never-launched agents, so `COUNT(session)` is not an active-agent or model-use count.

#### CLI transcript readers

`runtime/transcript_context.rs` supports Claude Code and Codex only and protects attribution with the workspace cwd plus an owner/session marker and active-generation filtering.

- Claude Code: the parser retains the latest assistant usage record and calculates a current-context footprint from input, cache creation, cache read, and output token fields. Those fields describe the latest request's context/response footprint; summing successive gauge readings would double-count repeated context. Transcript streams can repeat request/message records, so any backfill needs a stable request/message identity and a uniqueness constraint.
- Codex: the parser deliberately uses `last_token_usage.total_tokens` and rejects `total_token_usage`; this is also a current-context gauge, not a cumulative usage ledger.
- Both return `tokens`, `limit`, `observed_at`, and `source_kind`, but persistence keeps only `tokens` and `limit` and replaces observation time with database write time.
- `commands/instance.rs` polls every two seconds and writes only changed readings. Restart/compact paths reset or replace the current gauge; prior values are not recoverable as a series.
- Antigravity conversation data is opaque to this parser, so context and token usage remain unknown.

The existing Claude transcript corpus may support a bounded best-effort backfill only where owner markers and a stable request/message id are present. It must be labelled imported/partial with a per-source coverage start. The current Codex parser does not expose a stable completed-turn identity, so Codex history must remain unknown until a separate importer proves one.

#### Direct chat and one-shot calls

- `runtime/chat.rs:47-88` keeps chat history in memory only. `commands/message.rs:26-28,106-114` explicitly returns an unpersisted acknowledgement; the `message` table is not populated by production `message.send`.
- `runtime/provider.rs:182-236` returns only accumulated assistant text. `anthropic_text_delta` (`:246-259`) ignores `message_start`/`message_delta`; `openai_text_delta` (`:261-273`) reads only `choices[0].delta.content`. The HTTP drivers (`:300-367`) therefore discard usage and served-model metadata.
- Anthropic's official streaming contract exposes input/model data in `message_start`, cumulative output usage in `message_delta`, and a stable message id. Fallback can span models: `usage.iterations` is the per-attempt record and the top-level usage covers only the attempt that returned the message. See [Anthropic streaming](https://platform.claude.com/docs/en/build-with-claude/streaming) and [Anthropic fallback](https://platform.claude.com/docs/en/build-with-claude/refusals-and-fallback).
- The OpenAI-compatible path calls `/chat/completions`, not Responses, and does not request/include streaming usage. The organization Usage API aggregates by provider dimensions such as model, project, user, and API key; it has no Conclave workspace/agent dimension, so a shared key cannot reconstruct local attribution. See [OpenAI Usage API](https://developers.openai.com/api/reference/resources/admin/subresources/organization/subresources/usage).
- `commands/fusion.rs:71-101,263-420` calls providers directly for panel, judge, and synthesis. `fusion_run`/`fusion_panel_response` persist content/status but no actual model or usage. A run row is created before the pipeline finishes; one run can contain several model calls.
- `commands/draft.rs:590-699` invokes Claude Code or Codex through `runtime/cli_oneshot.rs` and returns parsed JSON without persisting a run, served model, token usage, or completed-turn identity.

Configured `agent_definition.model` cannot stand in for historic model identity. It is nullable and mutable (`repo/agent_definition.rs:60-85,492-589`), and provider fallback may serve a different model. Every new usage record must copy both requested and actually served model at event time.

### 1.3 Provider/harness coverage matrix

| Path | Current context | Additive token usage | Actual served model | Durable completed-turn identity | Honest current state |
|---|---|---|---|---|---|
| Claude Code CLI | Latest transcript gauge | Not persisted; possible partial transcript import | Not persisted | Not exposed by current reader | Context only, latest value |
| Codex CLI | Latest `last_token_usage` gauge | Explicitly not cumulative | Not persisted | Not exposed by current reader | Context only, latest value |
| Antigravity CLI | Unknown (`NULL`) | Unknown | Config only | None | Unknown |
| Anthropic direct chat | Character-based output estimate may update session | Provider usage discarded | Provider result discarded | Message id discarded | Estimate only, not measured usage |
| OpenAI/local compatible chat | Character-based output estimate may update session | Provider usage discarded | Provider result discarded | Response/chunk identity discarded | Estimate only, not measured usage |
| Draft one-shot | None | Not persisted | Config only | No durable run | Unknown after return |
| Fusion panel/judge/synthesis | None | Not persisted | Config only | Run/response rows are not one-to-one with calls | Completion status is partial evidence only |

No existing path provides complete, cross-provider, per-model/per-agent/per-workspace additive token history.

### 1.4 Audit of durable records as heatmap sources

The heatmap definition should be: **one cell counts distinct successfully completed logical model turns whose completion time falls in that bucket**. A logical turn is the response to one user/agent request; provider fallback attempts contribute usage to the same turn rather than inflating activity count.

| Record | Durable semantics | Defensible as model activity? | Reason |
|---|---|---|---|
| `message` (`0001:57-66`) | Intended session messages | No | Production `message.send` does not insert; assistant output is not persisted. |
| `inter_agent_message` (`0001:68-76`) | One injected prompt, possibly `queued` or `delivered` | No | Delivery is not model completion; queued rows may never run, and one prompt may cause zero or many provider calls. |
| `task_event` (`0012:17-25`, rebuilt by `0018`) | Append-only coordination notes/state/gates/challenges/rulings/plan checks | No | Measures Conclave workflow activity, not model calls; many events are CLI/user actions. It could power a separately named coordination metric. |
| `fusion_run` and `fusion_panel_response` (`0001:165-181`) | Pipeline/run and panel response status/content | Partial only | A run can invoke N panel calls plus judge and synthesis; judge/synthesis have no separate event rows, and usage/model is absent. Do not backfill counts from rows. |
| `artifact` (`0014:18-40`) | Explicit significant output, usually added by CLI | No | Optional creator id, often manually published after generation; not one-to-one with a turn. |
| `memory_chunk` (`0009:9-23`) / proposal | Explicit or distilled knowledge | No | Creation/approval is a knowledge-management action, not a provider completion; source attribution is not model usage. |
| `blackboard_activity` / snapshots | Reads/writes and context summaries | No | Reads are not model calls; snapshot tokens can be estimates and are not provider usage. |
| `proxy_*_metric` (`0019`–`0026`) | Removed ctx-proxy experiments | No | The subsystem is removed; tables have no workspace/agent/session key, cover only experiment traffic, and repository/runtime references are gone. They remain migration-history only. |
| Terminal output chunks | Transient byte chunks | Never | Chunk count depends on buffering/transport and can change without any change in semantic activity. |

Code search confirms `proxy_request_metric` appears only in migration `0019`; the other proxy table names appear only in their migrations and migration runner/tests. Historical rows may remain readable, but they cannot be attributed or merged into Overview.

### 1.5 Minimal honest persistence model

Use two append-only tables after schema version 28. A two-table design is slightly larger than a single row but is required to represent fallback/multi-attempt billing without double-counting heatmap activity.

`model_turn_event` — one logical response:

- `id TEXT PRIMARY KEY`
- `workspace_id TEXT NOT NULL`, `workspace_agent_id TEXT`, `session_id TEXT`
- `generation INTEGER` or equivalent launch identity
- `source_kind TEXT NOT NULL` (e.g. `direct_chat`, `claude_transcript`, `codex_transcript`, `draft_oneshot`, `fusion_panel`, `fusion_judge`, `fusion_synthesis`)
- `source_event_id TEXT NOT NULL` and `UNIQUE(source_kind, source_event_id)` for idempotent transcript/import/persistence
- `status TEXT NOT NULL` (`completed`, `failed`, `cancelled`); only `completed` counts in the primary heatmap
- `occurred_at TEXT NOT NULL` (provider/transcript completion time in UTC), `recorded_at TEXT NOT NULL`
- nullable `context_after_tokens`, `context_limit`, `context_observed_at`, `context_source`, `context_is_estimate`
- optional bounded failure classification; no raw prompt/response required for Overview

`model_usage_attempt` — zero or more provider/model attempts per logical turn:

- `id`, `turn_id`, `attempt_index`, with `UNIQUE(turn_id, attempt_index)`
- `provider`, `requested_model`, `served_model`
- nullable `input_tokens`, `cache_creation_input_tokens`, `cache_read_input_tokens`, `output_tokens`, plus explicitly named provider-specific nullable fields only when semantics are known
- `usage_source`, `usage_observed_at`, `usage_complete` and `billable`/billing classification only when the provider reports it

Do not store a synthetic `total_tokens` that loses the component semantics. A display total may be computed from observed components, but it must remain `null` unless the required components are known. Never sum `context_after_tokens`; it is a gauge. If implementation must start smaller, allow zero attempt rows and persist a completed turn with unknown usage rather than writing zero.

Indexes should cover `(occurred_at)`, `(workspace_id, occurred_at)`, `(workspace_agent_id, occurred_at)`, and `(served_model, occurred_at)` on the attempt table through its turn join. Query plans should be checked against the bounded overview query; avoid one query per workspace/agent.

Collection seams:

1. Change the provider stream result from `String` to a result containing response id, completion time, requested/served model, terminal status, and usage/attempts; parse Anthropic terminal events and OpenAI-compatible usage when actually supplied.
2. Persist after a semantic response completes, before returning success to chat/fusion callers. Generate a stable local id before calls that do not provide one.
3. Extend CLI transcript readers/importers to emit idempotent completed-turn candidates separately from the current-context reader. Do not persist the two-second polling samples.
4. Instrument `draft.run` and each fusion panel/judge/synthesis call explicitly. A successful response with missing usage still produces a completed turn plus zero attempt rows or nullable fields.
5. Record failed/cancelled calls separately; do not silently count them as completed use. A future UI may opt into a separate failure overlay.

### 1.6 Overview query/API contract

Proposed IPC request:

```ts
type UsageOverviewRequest = {
  from: string;             // inclusive RFC 3339 instant
  to: string;               // exclusive RFC 3339 instant
  bucket: "day" | "hour";
  timezone: string;         // IANA zone, e.g. Asia/Bangkok
  workspaceIds?: string[];
  agentIds?: string[];
  models?: string[];
};
```

Response requirements:

- Echo the normalized range, IANA timezone, and bucket size.
- Every bucket returns explicit UTC `start`/`end`, a local label, and UTC offset. On DST fallback, repeated local hours remain distinct because their UTC boundaries/offsets differ.
- `activity.completedTurns` is the distinct logical turn count; include failed/cancelled separately if requested.
- Token totals are split by component and paired with `coverage: complete | partial | unknown`, `measuredTurns`, and `unmeasuredTurns`. Zero is valid only when coverage proves the queried turns were observed.
- Current context is a separate latest-observation block per agent, including observation time, source, and estimate flag; never place it in additive time-series totals.
- Return breakdowns by actual served model, agent, and workspace plus source coverage/`coverageStart`. Requested model may be a secondary dimension.
- Archived workspaces remain included in historical usage by default and are labelled archived; archiving must not erase history.

Generate local day/hour boundaries with an IANA timezone implementation in Rust, convert each boundary to UTC, then run bounded range/group queries or one bounded event query followed by in-memory grouping. Do not use fixed `24h` arithmetic for local days. Defaults: daily buckets for the trailing 12 months to match the reference; hourly only for a selected range capped at 31 days. Cap daily queries at 366 buckets and reject larger ranges rather than silently sampling.

Retention/history: initially retain turn/usage metadata until destructive workspace deletion. Expose the earliest observed timestamp per source/agent/model. Dates before collection/import coverage must render as “no coverage,” visually distinct from measured zero activity. Existing history is unknown except any separately validated/imported Claude transcript range.

### 1.7 Usage implementation partitions and tests

Backend/data partition:

- New migrations after `0028_antigravity_cli.sql`; register them in `src-tauri/src/engine/db.rs` without altering historical migrations.
- New `src-tauri/src/engine/repo/model_usage.rs` and `commands/usage.rs`; wire through `repo/mod.rs`, `commands/mod.rs`, and `router.rs`.
- Collection changes in `runtime/provider.rs`, `runtime/chat.rs`, `runtime/transcript_context.rs`, `runtime/cli_oneshot.rs`, `commands/instance.rs`, `commands/draft.rs`, and `commands/fusion.rs`.
- IPC contracts in `src/ipc/types.ts` and `src/ipc/commands.ts`.

UI partition:

- New `src/components/Overview.tsx` (and small focused heatmap/breakdown components if needed).
- Navigation/selection in `src/components/AppShell.tsx` and brand/home affordance in `src/components/Rail.tsx`.
- Fixture handlers and fixed-time datasets in `src/fixtures/scenarios/default.ts`, `empty.ts`, and new named usage scenarios registered by `src/fixtures/backend.ts`.

Required tests/gates:

- Migration upgrade/preservation and uniqueness/idempotency tests in `db.rs` and `repo/model_usage.rs`.
- Parser fixtures for Anthropic terminal usage/fallback, OpenAI-compatible completion usage, duplicate transcript events, restart generations, missing usage, failure, and cancellation.
- Aggregation tests for workspace/agent/model filters, unknown versus measured zero, half-open boundaries, a non-UTC zone, and both DST repeated and skipped hours.
- Query-plan/bounded-query test proving no N+1 path.
- `cargo test --manifest-path src-tauri/Cargo.toml`, `pnpm build`, then the standing UI pixel gate for `home` default and empty/unknown scenarios. Each PNG must be opened and inspected before READY.

## 2. Reversible workspace archive

### 2.1 Confirmed current model and lifecycle

- `workspace` begins with `id`, `name`, `folder_path`, `color`, `created_at` (`0001:6-12`); `0007` adds `hidden`; `0027` adds `run_state` (`started | stopped`) and agent `availability` (`active | stopped`). Current schema version is 28.
- `repo/workspace.rs:39-56` exposes the row. `list` (`:64-85`) selects all then filters only hidden workspaces; `get` (`:88-103`) can fetch hidden rows. Folder path is not unique.
- Normal workspace creation is stopped; hidden scratch workspaces are started (`repo/workspace.rs:115-181`). Hidden is an internal/scratch classification and must not be reused as archive.
- `workspace.use` only validates existence (`commands/workspace.rs:26-39`); it does not persist selection or start anything.
- `workspace.start` (`:146-190`) takes the workspace lifecycle write lock, sets `run_state = started` before spawning eligible agents, and skips agents whose availability is stopped. `workspace.stop` (`:192-215`) tears down live runtimes. `workspace.delete` (`:242-267`) tears down all runtimes and then destructively deletes the database row/cascades.
- Runtime truth is the in-memory registry (`runtime/mod.rs`); `is_live` is the authoritative active-process check. Persisted `workspace_agent.status` can lag a crash/teardown. `run_state` is desired workspace mode, and `availability` is per-agent launch eligibility; neither alone proves a process exists.

### 2.2 Archive contract

Add nullable UTC `workspace.archived_at TEXT`. Keep it independent of `run_state`, `workspace_agent.status`, and `availability`.

Archive operation:

1. Acquire the target workspace lifecycle **write** lock.
2. Refetch the row and reject not-found, already-archived, or `hidden = true`.
3. List its agent instance ids and reject if `runtime.is_live(id)` is true for any one. Do not silently stop or terminate work.
4. In one database transaction set `archived_at = now`, set `run_state = stopped`, and normalize transient agent `status = idle`; preserve every agent's `availability`, sessions, messages, tasks, artifacts, memory, blackboard, usage records, folder path, and files.
5. Emit one workspace change event carrying enough state for the shell to refetch/reselect.

Restore operation:

1. Under the same write lock, reject hidden/not-found/not-archived.
2. Clear only `archived_at`; retain `run_state = stopped` and availability.
3. Emit the workspace change event. Never spawn as part of restore.

Normal `workspace.list` should return only non-hidden, non-archived rows. Add a bounded `workspace.listArchived` rather than an `includeArchived` flag that every caller might accidentally enable. Internal `get` must still fetch archived rows so restore and historical joins work; user-facing `workspace.use` should reject them.

Normalizing `run_state = stopped` is load-bearing. `WorkspacePane` currently eagerly spawns active agents whenever a selected workspace reports `started`; leaving an archived workspace started would allow an open/stale view to relaunch it. `workspace.start` must reject archive state even for a zero-agent workspace.

### 2.3 Launch and mutation guards

Make archive state part of `repo/workspace_agent.rs::runtime_eligibility` and enforce it in the central `commands/instance.rs::require_launch_eligible` check immediately before registration. Recheck under the workspace lifecycle lock so archive and launch are linearizable. This must cover:

- `workspace.start` and its per-agent spawn loop;
- direct instance spawn/resume/restart, including detached restart/compact recovery;
- CLI `ws start`, `agent resume`, and restart routes in `commands/cli.rs` / `bin/conclave-cli.rs`;
- `message.send` and `message.inject` delivery (setting stopped plus their existing eligibility reread provides defense, but explicit archived rejection gives a stable error);
- `draft.run` (`commands/draft.rs:590-699`), which can execute a one-shot CLI in a workspace folder outside the runtime registry;
- `fusion.run` (`commands/fusion.rs:363-420`), which calls providers outside the runtime registry;
- adding/instantiating an agent into an archived workspace (`agentDef.addToWorkspace` and equivalent CLI route).

The last three are bypasses if only runtime spawning is guarded. Decide mutation policy explicitly: the smallest consistent contract is read-only historical access while archived, so adding/removing agents and editing workspace metadata should reject until restored. Usage/history reads remain allowed.

### 2.4 Navigation and empty states

`AppShell.tsx` owns `workspaces` and `activeWorkspaceId` in local state; there is no durable active-workspace preference. Initial selection is first-list-item based, the brand mark in `Rail.tsx` is not a home action, and production has no useful selection-neutral overview. The corrected product direction makes Overview the natural brand/home destination.

Recommended shell behavior:

- Brand/Home opens Overview without changing a workspace's persisted runtime state.
- Normal rail shows only active workspaces. Archived management is a separate Overview section/drawer, not mixed into task/KPI cards.
- If the selected workspace is archived successfully, navigate to Overview (or the next explicit active workspace only if design canon requires it); never keep mounting its `WorkspacePane`.
- All-archived and truly zero-workspace states are distinct: all-archived still shows usage history and restore controls; zero-workspace offers Link Folder.

Current fixture mode has only `default` and `empty`; `empty` contains one stopped workspace rather than zero workspaces. Missing handlers throw by design. Add fixed-literal-timestamp scenarios for:

- usage mixed measured/partial/unknown coverage;
- usage no-coverage and measured-zero buckets;
- true zero workspaces;
- active plus archived workspaces;
- all archived;
- archive rejection while one runtime is live;
- archive success, selection fallback, restore remains stopped.

Archive/restore fixture handlers must mutate the in-memory fixture state so the real UI can be exercised end to end.

### 2.5 Archive implementation partitions and tests

Backend/data partition:

- New archive migration after `0028`; migration runner/upgrade tests in `src-tauri/src/engine/db.rs`.
- `repo/workspace.rs`: row field, active/archived list queries, transactional archive/restore helpers.
- `repo/workspace_agent.rs`: archived eligibility projection and status normalization helper.
- `commands/workspace.rs`: archive/restore orchestration under lifecycle write lock and start rejection.
- Central guards in `commands/instance.rs`; explicit one-shot/mutation guards in `commands/draft.rs`, `commands/fusion.rs`, and agent/workspace mutation commands.
- Router/CLI/IPC surfaces in `router.rs`, `commands/cli.rs`, `bin/conclave-cli.rs`, `src/ipc/types.ts`, and `src/ipc/commands.ts`.

UI partition:

- `AppShell.tsx`, `Rail.tsx`, the Overview/archive components, and the existing workspace edit/overflow surface selected by design canon.
- Fixture scenario/handler files only after canon fixes the exact interaction.

Required lifecycle tests:

- migration preserves all existing rows as active (`archived_at IS NULL`);
- list active versus list archived; internal get still resolves archived and hidden remains distinct;
- archive rejects hidden, unknown, already archived, and any live runtime without terminating it;
- archive with zero/no-live agents atomically sets archived/stopped/idle while preserving availability and child-row counts;
- restore clears archive only and leaves zero runtimes;
- every launch/one-shot/mutation bypass above rejects archived state, including a race between archive and spawn;
- deleting an archived workspace remains the separately explicit destructive action;
- `cargo test --manifest-path src-tauri/Cargo.toml`, `pnpm build`, and `pnpm uishot home` plus archive/empty scenarios, with every shot visually inspected and recorded through the task gate.

## 3. Recommended sequencing

1. Land the usage event schema and collection seams first; Overview cannot show honest history before collection exists.
2. Land archive schema/lifecycle guards independently; it should not wait on usage UI.
3. Add bounded `usage.overview` aggregation and archive list APIs.
4. Implement the design-canon Overview/heatmap and archived management against fixed fixtures.
5. Enable production data with explicit coverage-start messaging. Do not backfill a zero-valued history.

The only material product choice left for the owner is naming/presentation of incomplete coverage; the data contract should remain explicit regardless of copy. No technical blocker was found.
