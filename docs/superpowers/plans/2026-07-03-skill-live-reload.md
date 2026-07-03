# Skill live reload — sidecar as mutable source of truth

**Goal:** a running CLI agent's skills can be changed without restarting it: every skill
mutation rewrites the per-instance sidecar file and nudges live instances to re-read it.
Decision record: `docs/adr/0004-skill-live-reload.md` (read it first, plus ADR 0001).

**Owner/lead:** Detoro `bfb737ff-486d-4581-b407-95711d5e07ab` — design/spec conflicts escalate to
me, my ruling is final. Implementation judgment within this plan's intent is yours; log it in
`progress:skill-live-reload`, don't escalate it.

## Global constraints (every task inherits these)

- Repo `/Users/detoro/code/codeup`, branch `main`. Build on the current UNCOMMITTED working-tree
  state. `src-tauri/src/engine/agentctx.rs` already contains an uncommitted fix (three prompt
  strings: `skill_pointer_sentence`, `compact_restore_prompt`, `resume_restore_prompt`) that is
  under separate review — your changes to that file must be ADDITIVE ONLY; do not reword those
  three strings.
- `bootstrap_preamble` + appended pointer stay single-line and `=`-free (ADR 0001; existing tests
  enforce it — keep them green).
- TDD per task: failing test first, then the code. Done means
  `cd src-tauri && cargo test --lib` fully green AND `cargo clippy --lib` clean — paste both
  tails into the progress key as evidence.
- UI copy, code comments, doc text: English.

## Task 1 — unconditional sidecar + pointer

`src-tauri/src/engine/commands/instance.rs`, `apply_skills_to_preamble` (~line 86): drop the
empty-body branch. Always call `write_skill_sidecar` and append `skill_pointer_sentence`. When
`skill_body` is empty, write the placeholder body exactly:
`(no standing instructions attached right now — this file updates in place; re-read it when told to)`.
Update the existing test `apply_skills_to_preamble_is_noop_when_nothing_attached` (name it for
the new behavior) — it must now assert the pointer IS appended and the file exists with the
placeholder.

## Task 2 — the nudge prompt

`src-tauri/src/engine/agentctx.rs` (additive): new
`pub fn skills_updated_prompt(path: &std::path::Path) -> String`. Single line. Must contain the
full sidecar path (via the same `sanitize_field` pipeline as `skill_pointer_sentence`), the word
`UPDATED`, and an instruction to re-read the file NOW before continuing, noting any previously
read copy in context is stale. Tests mirror the existing prompt tests: single-line, names the
path, survives a hostile path (`a=b\nc`).

## Task 3 — reload engine + call sites

New `pub(crate) async fn reload_skills_for_def(state: &AppState, agent_def_id: &str)` in
`src-tauri/src/engine/commands/instance.rs`:

1. List the def's instances — check `repo::workspace_agent` (~line 178) for an existing
   by-def query; add `list_by_def(db, agent_def_id)` if none fits.
2. Compute `repo::skill::content_for_agent(&state.db, agent_def_id)` ONCE (skill.rs:535).
3. Per instance: `write_skill_sidecar` (placeholder body from Task 1 when empty). If
   `state.runtime.is_live(id)`: inject `skills_updated_prompt` via
   `super::snapshot::submit_line`, then `repo::session::set_launched_skill_ids` (session.rs:203)
   so the staleness badge clears. Dead instances: rewrite only (spawn recomputes anyway).
4. Skip an instance ONLY when nothing effective changed: the skill-id set matches the launched
   snapshot AND the freshly computed body equals the current sidecar file's content (missing
   file = changed). Comparing ids alone is WRONG — a `skill.save` content edit keeps the id set
   identical, and an id-only guard silently kills the entire content-edit reload path.
   [PLAN BUG, found in lead review 2026-07-03: the original wording here said "skip when the
   skill-id set is unchanged" — implemented faithfully, then caught in integration review.
   Guard so it can't recur: a test named for the scenario — content-only edit MUST rewrite the
   sidecar and nudge live instances — written failing-first.]

Call sites, each detached (`tauri::async_runtime::spawn`, mirroring `snapshot.rs`'s tails) so
the IPC call returns promptly:
- `commands/agent.rs::save` (~141) — after skill attachments/builtin selection persist.
- `commands/skill.rs::save` (~55) and `::delete` (~93) — a skill edit affects every def attached
  to it: add a reverse repo query (`agent_def_ids_by_skill`) next to
  `custom_skill_ids_by_agent` (repo/skill.rs:220), loop `reload_skills_for_def` over the result.

Tests: unit-test `reload_skills_for_def` against fixture DB rows (pattern:
`fixture_instance` in instance.rs tests) — asserts sidecar rewritten, `launched_skill_ids`
updated for the live path, unchanged-set short-circuits. The submit_line injection itself is
runtime-verified (risk ledger), not unit-tested.

## Task 4 — verify the badge clears (no planned UI edits)

`src/components/ContextDrawer.tsx` (`computeSkillsStale`, ~163) and `WorkspacePane.tsx`
(`skillsChanged`, ~228) derive staleness from `launched_skill_ids`; Task 3's refresh should
clear the badge with zero UI changes. Verify by reading the derivation; if any copy still
promises "Restart to apply" on a path Task 3 now covers live, update the copy (English) and
note it in progress.

## Task 5 — bake the conclave binary's full path into the preamble

Field bug: agents hit `conclave: command not found` and burn a turn hunting for the binary.
Root cause: the launch shell prepends the shim dir to PATH (`instance.rs` ~257), but the
harness's own tool shells re-source the user's rc files, which frequently RESET PATH instead of
appending — the export is best-effort, not guaranteed. The preamble (system-prompt layer, present
every turn) is the reliable channel.

- `src-tauri/src/engine/agentctx.rs` (additive):
  `pub fn conclave_path_sentence(path: &std::path::Path) -> String` — single line, `=`-free via
  `sanitize_field` (same pipeline as `skill_pointer_sentence`). Content: the `conclave` binary's
  full path is `<path>`; if `conclave` is ever not found on PATH, run it via this full path,
  quoted (the path contains spaces). Do NOT tell the agent to search for it.
- `src-tauri/src/engine/commands/instance.rs` spawn `cli` branch: move the
  `ensure_conclave_shim()` call BEFORE preamble assembly; when it returns `Some(bin)`, append
  `conclave_path_sentence(&bin.join("conclave"))` to the preamble (after the skill pointer) and
  keep using `bin` for the PATH export exactly as today. When `None` (dev run without the CLI
  built), append nothing — there is no path to point at.
- Tests mirror the skill-pointer suite: single-line/`=`-free under a hostile path; names the
  path; the FULL combined preamble (bootstrap + skill pointer + conclave sentence) stays
  single-line and `=`-free.

## Risk ledger (known-fragile, hit it prepared)

- **Injecting into a mid-turn agent:** `submit_line` types into the TUI; a streaming harness
  queues the input. Same risk profile as the existing compact/restart prompts — accepted. Do
  not add retry logic.
- **Codex paste-burst:** `submit_line` already sends CR separately after 40ms; reuse it, do not
  hand-roll stdin writes.
- **`skill.save` fan-out:** one skill can touch many defs across workspaces; the detached task
  must loop defs sequentially (no join-all) to keep PTY writes orderly.
- **Instances launched before Task 1** have no pointer in their preamble when skill-less; the
  nudge carries the full path precisely so these still find the file.
- **agentctx.rs is under review** (bb `review:skills-survive-clear`) — additive edits only.

## Accepted known risks (lead ruling, 2026-07-03 post-review)

- **Cross-task PTY injection race** (review L3): two near-simultaneous skill mutations can each
  spawn a detached reload whose `submit_line` calls interleave into one live TUI. Accepted —
  low probability, self-heals, same posture as the existing compact/restart injections. Revisit
  only on a live repro (a per-instance stdin lock is the known fix).
- **Non-atomic sidecar write** (review L5): `std::fs::write` truncates then writes; a concurrent
  read can see partial content. Accepted — next reload self-heals via the content-compare guard.
  Known fix if ever needed: temp file + atomic rename.

## Definition of done

All tasks green per the global gate, progress key updated per task with evidence, then message
the lead (`conclave tell bfb737ff-486d-4581-b407-95711d5e07ab …`) — do NOT commit; the lead owns
integration (review by `4b13a0e6` lands first).
