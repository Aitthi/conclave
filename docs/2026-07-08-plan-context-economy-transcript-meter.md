# Plan: context economy + transcript-backed context meter

Date: 2026-07-08
Owner: Aoki d1a70cab-1d34-490c-9fd7-ba4d4c940606
Authority: in-loop
Human request: "ทำทั้งหมดเลย ... context meter ตอนนี้ใช้จริงไม่ได้เลย ไปเอา transcript มาหาเลยได้ไหม ทั้ง claude code และ codex"

## Goal

Make agents spend far less context while staying smart:

1. The board read path must stop dumping full plans unless explicitly requested.
2. Agents need a compact `task brief` read that pulls the minimum useful context for one work item.
3. CLI agents need a usable context meter backed by the harness transcripts, for both Claude Code and Codex.
4. The protocol text must teach the new low-context path so agents use it by default.

## Decisions

1. Transcript usage is the source of truth for CLI agents.
   - Current PTY byte counting is intentionally disabled for CLI agents because terminal redraws are not model context.
   - Claude Code: read `~/.claude/projects/-Users-detoro-code-codeup/*.jsonl`. Match by workspace cwd and Conclave self id in the transcript. Use assistant records with `.message.usage`; de-duplicate streaming repeats by `requestId` first, then `.message.id`, keeping the newest/final record. Context tokens = `input_tokens + cache_creation_input_tokens + cache_read_input_tokens + output_tokens` from the newest unique assistant request. Context limit comes from the launched model/session config when available, else the existing `session.context_limit`.
   - Codex: read `~/.codex/sessions/**/*.jsonl`. Match by `session_meta.payload.cwd` or `turn_context.payload.cwd` and Conclave self id appearing in the recorded context. Use `event_msg` rows with `payload.type == "token_count"`. Context tokens = `payload.info.last_token_usage.total_tokens`; context limit = `payload.info.model_context_window`.
   - Rejected: use `total_token_usage` for Codex. It is cumulative over turns and can exceed the context window; it is not the current-window meter.

2. Transcript text never crosses the runtime boundary.
   - The transcript parser returns only numeric usage, context limit, observed timestamp, and an internal source kind/path used for debugging logs.
   - No raw prompt, tool output, or assistant text is persisted to Conclave DB by this feature.
   - Tests must use synthetic transcripts under temp dirs, not real user transcripts.

3. Do not mutate `task.list`'s default wire shape.
   - Existing UI consumers and the frozen contract expect plan-bearing rows.
   - Add an opt-in slim mode (`includePlan: false` or equivalent) and make the CLI `conclave task list` use it by default.
   - `task.get` remains the full record.

4. Add `task brief` instead of making agents assemble context manually.
   - `task.brief` returns exactly one task's low-context resume packet: task metadata, design canon, file boundary, plan excerpt or full plan under a bounded cap, open challenges, latest gates, last events, and relevant memory hits.
   - The CLI renders it as readable text, not JSON by default.
   - The brief is not a new source of truth; every field points back to task events, plan text, or memory hits.

5. Transcript-backed context updates are reported, not auto-reset.
   - Existing `auto` snapshots reset the output-byte estimate after firing; that is wrong for transcript usage because an external transcript meter will not reset when Conclave writes a marker.
   - For CLI agents, emit/persist transcript usage and show it in UI. If threshold behavior is added in this lane, it must be a nudge to save/restart, not a fake reset.

## Current evidence

- `commands::instance` only tracks live context for `chat` backends; CLI/PTY is deliberately excluded.
- `commands::snapshot` `auto` markers carry no `carried_forward` content.
- `commands::task` currently defines `task.list` rows as full `Task` rows, and `Task` includes `plan`.
- Local transcript probe found:
  - 134 Claude Code JSONL files under `~/.claude/projects/-Users-detoro-code-codeup`.
  - Codex JSONL files under `~/.codex/sessions/YYYY/MM/DD`, with workspace cwd in `session_meta` / `turn_context`.
  - Codex `token_count` events include `last_token_usage.total_tokens` and `model_context_window`.
  - Claude assistant records include `.message.usage`.

## Task A: slim task reads + `task brief`

Owner: Aoki. Implementer: Dabin. Reviewer: Armin.

Boundary:
- `src-tauri/src/engine/commands/task.rs`
- `src-tauri/src/engine/repo/task.rs`
- `src-tauri/src/engine/commands/cli.rs`
- `src-tauri/src/bin/conclave-cli.rs`
- `src-tauri/skills/tool-map/SKILL.md`
- `src-tauri/skills/leadership/SKILL.md`
- `src-tauri/skills/collaboration/SKILL.md`
- `src-tauri/skills/implementer/SKILL.md`

Required behavior:
- `task.list` accepts an opt-in slim flag and omits `plan` only when explicitly requested.
- `conclave task list <ws> [--state s]` maps to slim mode by default.
- Add `conclave task list <ws> --full` for the old plan-bearing JSON.
- Add `conclave task brief <ws> <slug> [--limit N]`.
- `task.brief` should be bounded and agent-readable. It may include up to 5 memory hits by querying the existing memory search handler or repo path; if embedder is not ready, return the rest of the brief with `memoryError`.
- The brief renderer must not print raw full plans beyond a cap. Use file paths and event ids as pointers.

Tests:
- Rust tests for slim list preserving default full shape when flag absent.
- Rust tests for CLI map: `task list` sends slim; `task list --full` sends full.
- Rust tests for `task brief` shape and caps.
- Full gate: `cd src-tauri && cargo test`.

## Task B: transcript-backed context meter for Claude Code and Codex

Owner: Aoki. Implementer: Dabin. Reviewer: Armin.

Boundary:
- `src-tauri/src/engine/runtime/transcript_context.rs` (new)
- `src-tauri/src/engine/runtime/mod.rs`
- `src-tauri/src/engine/commands/instance.rs`
- `src-tauri/src/engine/repo/session.rs` only if a small helper is needed
- `src-tauri/src/engine/bus.rs` only for optional source labeling
- `src/ipc/events.ts` only if bus shape gains optional source
- `src/components/ContextBars.tsx` only if displaying the source label
- `src/components/LaneBoard.tsx` only if displaying the source label there
- `src/fixtures/scenarios/default.ts` and `src/fixtures/scenarios/empty.ts` only if UI event shape changes

Required module interface:
- `TranscriptContextReader::new(config)` or equivalent small interface.
- `poll(instance_id, workspace_folder, cli_kind, started_at) -> Option<TranscriptContextReading>`.
- `TranscriptContextReading` contains `{ tokens, limit, observed_at, source_kind }`.
- The module owns transcript discovery, parsing, de-duplication, and source matching. Callers do not parse JSONL.

Required behavior:
- For Claude Code CLI agents, poll Claude transcript files and emit/persist the latest matched usage.
- For Codex CLI agents, poll Codex transcript files and emit/persist the latest matched token_count usage.
- Do not parse or expose raw transcript text outside the module.
- Polling must be throttled to avoid scanning the transcript tree on every terminal repaint. A per-forwarder poll interval of about 2s is acceptable.
- If no transcript is matched, leave the previous meter untouched and log at debug level only.
- For transcript-backed CLI meters, do not use the existing auto-compact reset path. If threshold behavior is implemented, inject a one-time low-context warning that tells the agent to run `conclave restart` / save a handoff.

Tests:
- Unit tests with synthetic Claude JSONL showing duplicate assistant usage is de-duplicated.
- Unit tests with synthetic Codex JSONL showing `last_token_usage.total_tokens` is used and `total_token_usage` is rejected.
- Unit tests proving files without matching cwd/self id are ignored.
- `commands::instance` test proving a CLI output chunk can update session context through the transcript reader.
- Full gate: `cd src-tauri && cargo test`.
- If `src/` UI is touched, run and visually inspect:
  - `pnpm uishot home`
  - `pnpm uishot laneboard`
  - repeat affected empty scenarios if fixture data changes.

## Task C: protocol integration

Owner: Aoki. Implementer: Dabin after A/B. Reviewer: Armin.

Boundary:
- `src-tauri/skills/tool-map/SKILL.md`
- `src-tauri/skills/strategic-compact/SKILL.md`
- `src-tauri/skills/memory/SKILL.md`
- `src-tauri/skills/leadership/SKILL.md`
- `src-tauri/skills/implementer/SKILL.md`
- `src-tauri/src/engine/agentctx.rs` only if the preamble needs a short pointer

Required behavior:
- Teach agents: `task list` is slim orientation; `task brief` is the resume packet; `task get` is full deep read.
- Teach low-context policy: do not paste full task lists, transcript text, or long logs into chat; reference paths and event ids.
- Teach context meter meaning: chat meter is engine-reported; CLI meter is transcript-reported.

Tests:
- `cd src-tauri && cargo test` for skill parsing and preamble tests.

## Integration order

1. Task A first. It gives immediate context savings and reduces damage from every future board read.
2. Task B second. It makes context meter usable for active CLI agents.
3. Task C last. It teaches the behavior after the command surface exists.

## Acceptance

- `conclave task list <ws>` is concise and no longer dumps every plan by default.
- `conclave task list <ws> --full` still returns the old information.
- `conclave task brief <ws> <slug>` returns a bounded, useful resume packet.
- Running Codex and Claude Code agents show context readings based on their transcript usage, not PTY byte estimates.
- No raw transcript text is stored or sent through IPC.
- All backend tests pass. UI pixel gate passes if UI touched.
