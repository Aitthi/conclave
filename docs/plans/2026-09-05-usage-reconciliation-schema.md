# Durable Claude usage reconciliation evidence
owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop
Implementer: Dew (60ff2775-14a2-4db4-ab44-6df5bb13bf2a). Aoki rules and integrates this supplemental task with usage-engine-v2. Execution slug: usage-reconciliation-schema.

## Reading order and decision
Read docs/plans/2026-09-05-usage-overview-contract.md, docs/research/2026-09-05-usage-transcript-evidence.md (Claude event key and reconciliation), docs/plans/2026-09-05-usage-engine.md, then this amendment. This is the canonical C6 schema amendment to the engine plan, following review a12f77f2, ruling 8b7dc696 and Dew proposal a8d334f5. All parent constraints and frozen IPC apply.

Repeated Claude blocks must agree on response identity, model, stop reason and each relevant usage component across restart and arbitrary replay. A bounded recent-group cursor cache cannot prove agreement after eviction. Persist minimal scalar evidence on the event. Existing normalized cache fields suffice for cache evidence; normalized total input alone cannot recover uncached input when another component was missing. Credit Dew for identifying the additive migration boundary before editing.

## Exact boundary and execution
This supplemental task owns ONLY:
- src-tauri/src/engine/migrations/0032_model_usage_reconciliation.sql
- docs/plans/2026-09-05-usage-reconciliation-schema.md

Claim this task with task claim while remaining in the existing usage-engine-v2 worktree. Do not create another worktree: this is one dependent implementation by the same implementer. Commit these paths separately with stage commit against this slug. The parent task continues owning db.rs registration, repository/parser code, tests and the parent plan. Persist the amendment reference in that parent plan under its existing boundary. Never modify already merged migrations 0030 or 0031. No UI or IPC changes.

## Migration contract
Add nullable stop_reason TEXT, bounded to 128 characters by CHECK, and nullable source_uncached_input_tokens INTEGER, bounded to 0..1099511627776 by CHECK, to model_usage_event. Both default to NULL for pre-migration rows and non-Claude sources. Register migration 0032 in db.rs under the parent task.

Do not add source_block_count: no consumer requires it, and counting reread blocks would add replay-sensitive state without proving agreement. A valid newly imported Claude response requires a non-null bounded stop reason. Malformed/oversized evidence must yield bounded diagnostics and partial coverage; do not truncate distinct strings into apparent agreement. Missing/invalid counters remain unknown under C7, never fabricated zero.

Reconciliation under the parent task compares response/model identity, stop reason, uncached input, normalized input/output and cache/read/write/reasoning subsets. Incomplete historical evidence cannot prove agreement: an existing Claude row with NULL stop_reason remains conservatively unknown/conflicting with partial coverage until a verified replay establishes the full group. A single agreeing block must not silently clear a prior conflict. Preserve maximum source completion time on agreement, at canonical millisecond precision, without another activity. These updates share the events/coverage/cursor transaction.

## Risk ledger and acceptance
Risks: migration from a populated schema31; missing components masked by an unknown normalized total; replay after cache eviction/truncation; cross-midnight completion moving buckets; accidental raw source persistence. Store only the two bounded scalars above, never raw transcript JSON/text.

Parent tests must prove schema31 upgrade preserves populated events and initializes evidence to NULL; fresh database reaches schema32; equal totals with changed cache or uncached components conflict; stop-reason disagreement survives restart and more than 16 intervening groups; agreeing replay uses the maximum timestamp across midnight without changing activity count; replay does not increment any block counter. Retain C1-C8 acceptance in the parent review record.

Run scoped whitespace checks and the parent focused tests, full cargo test --manifest-path src-tauri/Cargo.toml and pnpm build. Record migration commit and gate references in both tasks. READY is accepted only alongside the corrected parent collector delivery; Aoki merges and closes both together. No app rebuild, relaunch or publication.
