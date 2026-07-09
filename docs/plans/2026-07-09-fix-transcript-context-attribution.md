# Plan: fix transcript context attribution across agents

Date: 2026-07-09
Owner: Aoki d1a70cab-1d34-490c-9fd7-ba4d4c940606
Authority: in-loop
Human bug report:

- "ใน UI มัน bug ถ้าทำงานหลาย agent มันขึ้นของ agent อื่นด้วย ตัง % เด้งกลับไปกลับมา"
- "และมันจะนับเกินจริงไปประมาณ 2%"

## Goal

When multiple CLI agents are active in the same workspace, each agent's UI
context meter must follow only that agent's own transcript. One agent's token
count must never be emitted to another agent's `sessionId`, and the displayed
percentage should track the harness's own context percentage without a systematic
~2 percentage point overcount.

## Diagnosis

Observed symptom: UI context percentages jump between agents when several agents
are running.

Primary hypothesis: `TranscriptContextReader` attributes transcript files using
`line.contains(instance_id)` anywhere in the file. Codex/Claude transcripts often
contain other agents' ids in roster output, inter-agent messages, or task notes,
so transcript B can appear to match agent A. Because the reader then selects the
    newest matched transcript, several forwarders can emit the same token count for
different sessions.

Secondary hypothesis: the Codex numerator/denominator used for display is not
exactly the same as Codex's own context meter. Current code uses
`last_token_usage.total_tokens / model_context_window`; real transcript samples
also expose `input_tokens`, `cached_input_tokens`, `output_tokens`, and
`reasoning_output_tokens`. The implementer must verify which field matches the
harness's displayed percentage before changing this formula.

Evidence:

- `src-tauri/src/engine/runtime/transcript_context.rs` uses `line.contains(instance_id)`
  in both `scan_claude_file` and `scan_codex_file`.
- Real Codex transcripts place the owning Conclave id in the bootstrap/developer
  prompt text as `your own agent id is <uuid>`.
- Real Codex `session_meta.payload.id` is the Codex session id, not the Conclave
  agent id, so matching that field to the Conclave instance id is wrong.
- Real transcripts also contain other agents' ids in later roster/message text.

## Decisions

1. Ownership match must be structured and narrow.
   - Accept an owning match only from bootstrap/developer text that states the
     agent's own id, e.g. `your own agent id is <instanceId>`.
   - Do not treat arbitrary later transcript lines, tool outputs, roster text,
     or inter-agent messages as ownership evidence.

2. Workspace match remains required.
   - A transcript must match both workspace cwd and owner id before its token
     readings can update a session.

3. No raw transcript text leaves the transcript reader.
   - Keep privacy boundary from the previous transcript-meter plan.

4. Fix the existing tests if they encoded the wrong contract.
   - The current Codex test may imply `session_meta.payload.id == instance_id`;
     replace or amend it so the test matches real transcripts.

5. Fix the ~2% overcount only with evidence.
   - Add a small synthetic/fixture test for whichever Codex token formula is
     chosen.
   - If no transcript field can prove the harness-visible percentage, leave the
     formula unchanged and record the evidence gap; do not invent a fudge factor.

## Boundary

- `src-tauri/src/engine/runtime/transcript_context.rs`
- `src-tauri/src/engine/commands/instance.rs` only if needed for test injection
- `docs/plans/2026-07-09-fix-transcript-context-attribution.md`

Do not touch `src/components/*` unless backend tests prove UI-side attribution is
also wrong. If `src/` UI is touched, UI Pixel Gate is mandatory for affected
views (`home` at minimum).

## Required Feedback Loop

Add failing tests before the fix:

1. Codex cross-agent test:
   - Create two synthetic Codex transcripts under one workspace.
   - Transcript A's bootstrap/developer text owns `agent-a`.
   - Transcript B's bootstrap/developer text owns `agent-b`.
   - Transcript B also contains arbitrary later text mentioning `agent-a` (roster
     or tell-style text).
   - Token count B is newer/higher.
   - `reader.poll("agent-a", workspace, "codex", epoch)` must return A, not B.
   - `reader.poll("agent-b", workspace, "codex", epoch)` must return B.

2. Claude equivalent if feasible with the existing synthetic shape:
   - Owner marker is in bootstrap/user/developer style text.
   - Later arbitrary line mentions the other id.
   - The reader must not cross-attribute.

3. Regression guard:
   - A transcript with workspace match and only arbitrary roster mention of an
     id, but no owner marker, must be ignored.

4. Codex formula guard:
   - Add a focused test that pins the chosen field(s) for context percent.
   - Explicitly reject a magic subtraction/offset. The fix cannot be "minus 2%".

## Implementation Guidance

- Prefer a small helper such as `line_owns_instance(value, instance_id)` or
  `text_declares_own_agent_id(text, instance_id)`.
- For Codex, inspect structured JSON:
  - `response_item.payload.type == "message"`
  - `payload.role == "developer"` is the expected owner carrier in real files.
  - Content is an array of `{ type: "input_text", text: ... }`.
- For Claude, use the analogous structured message fields present in tests/real
  files. If exact shape varies, support only clearly bootstrap-like user/system
  lines, not arbitrary tool output.
- The owner marker should be exact enough to avoid matching roster text:
  `own agent id is <id>` is acceptable; a bare `<id>` is not.
- For Codex overcount, inspect real `event_msg.payload.info` shape and compare
  against any harness-visible or transcript-recorded context reading available.
  Current live samples show:
  `last_token_usage.input_tokens`, `cached_input_tokens`, `output_tokens`,
  `reasoning_output_tokens`, `total_tokens`, and `model_context_window`.
  Prefer a documented field choice over arithmetic guessed from the reported
  offset.

## Gates

- Targeted first: `cd src-tauri && cargo test transcript_context`
- Full: `cd src-tauri && cargo test`

## Risk Ledger

- Over-tightening can hide meters for older transcript formats. If the owner
  marker is absent, hiding the meter is safer than showing another agent's
  meter.
- Do not fix by filtering in React. The wrong value is already emitted on the
  wrong `sessionId`; UI cannot reliably know it is poisoned.
