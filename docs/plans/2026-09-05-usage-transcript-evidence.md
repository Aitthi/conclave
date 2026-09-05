# Verify Claude and Codex usage identities
owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop
Researcher: Marty (0ed6b21b-8322-46c6-868c-8df84218bd30). Aoki rules. Read-only source investigation plus one report, no implementation.

## Goal and reading order
Read docs/research/2026-09-05-usage-contract-review.md → the Usage sections of docs/research/2026-09-05-workspace-overview-archive.md → relevant transcript/runtime symbols. The existing context reader is not an event importer. Determine whether the actual supported Claude Code and Codex transcripts can supply stable, deduplicated usage records and truthful per-model tokens. The user primarily uses CLI agents: do not equate an incomplete existing reader with an impossible source format.

## Bounded investigation
- Inspect a small sample of existing local Claude/Codex transcript metadata, with strict output allowlisting: event types, IDs (hash if useful), timestamp fields, usage component names/numeric values and model names only. NEVER print/store prompts, responses, instructions, tool arguments, credentials or raw transcript lines. Reuse known paths from the reader/session config; do not recursively dump home directories.
- Claude: verify requestId vs message.id namespace and duplicates/revisions; completed response versus intermediate assistant/tool blocks; whether latest cumulative usage replaces earlier rows; model attribution; stable file/session identity and trustworthy timestamp. Explain a prospective incremental import with replay, truncation, crash/restart and old-generation isolation. Do not change the context reader's attribution/performance constraints.
- Codex: inspect session_meta, turn_context, task_started/task_complete and token_count shapes. Verify whether response IDs exist or session cumulative usage deltas have a stable identity and explicitly different activity unit. Prove last_token_usage vs total_token_usage and model changes/compaction/duplicate token_count effects. Recommend an honest parser or state exactly what is unprovable. No guessed completed-response count from a user-turn ID.
- Inspect one-shot output contract: can draft obtain actual CLI usage/result IDs, or only invocation completion? Avoid counting both invocation and imported internal responses for the same work.
- Current official primary docs/source only if needed for semantics (browse required for external assertions); local runtime observation is evidence for installed behavior. No requests to models, spending, app restart or changes to installed CLI.

## Output and gate
Write docs/research/2026-09-05-usage-transcript-evidence.md on main using scoped stage commit. Include metadata-only fixture sketches, precise identity/formula/completion/coverage rules per source, known uncertainties, recommended minimal implementation and exact source files needed. Keep it bounded: this is the remaining collection proof, not an architecture redesign. Record TRANSCRIPT EVIDENCE READY with commit SHA. Aoki will rule before collector implementation; do not wait on archive engine.
