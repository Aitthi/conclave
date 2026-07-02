# Skill Editor: Full-Panel + Agent-Assisted Writing — Design

## Goal

Replace the custom-skill editor's small centered modal with a full-panel editor
using a real code editor widget, and add the ability to launch one of the
user's own configured CLI agents (Claude Code / Codex) alongside the editor
to write or revise a custom skill's `name` / `description` / `content`
directly, with its edits synced back into the editor.

## Non-goals

- Builtin (system) skills stay exactly as they are today: bundled `SKILL.md`
  files, never editable in the UI, mandatory or optional per ADR 0002/0003.
  This feature only touches CUSTOM skills.
- No live character-by-character diff/highlight of what the agent changed.
- No support for more than one agent-assist session open at a time per skill.
- No persistence of an agent-assist session across editor close/reopen — each
  "Ask agent to help" click starts a fresh session.
- No new file-watcher (`notify` crate or similar). Sync is driven off the
  existing `session:status` idle transition plus a manual "Sync now" button.

## High-level architecture

The assist feature reuses the existing session/PTY/streaming machinery
end-to-end instead of building a parallel spawn path:

```
SkillEditor (full panel)
 ├─ Name / Description inputs
 ├─ CodeMirror content editor
 └─ Agent assist panel (optional, toggled open)
     ├─ AgentDefinition picker (reuses agentDef.list)
     ├─ Terminal/output view (reuses the session:output streaming the
     │   existing chat view already uses)
     └─ Input box (reuses message.send)
```

When the user starts an assist session:

1. Backend writes the skill's current `name`/`description`/`content` to a
   scratch `SKILL.md` file in a fresh temp directory, in the same
   frontmatter format `parse_skill_md` already parses for builtin skills.
2. Backend creates a **hidden** `Workspace` row pointing at that scratch
   directory, plus the `workspace_agent` and `session` rows the existing
   spawn path requires.
3. Backend calls the existing `instance.spawn` exactly as a normal workspace
   would, using the `AgentDefinition` the user picked. The agent gets real
   file tool access to the scratch directory's `SKILL.md` — nothing new to
   build there.
4. Frontend reuses the existing session-output event stream and
   `message.send` for the chat/terminal experience — no new streaming code.
5. Whenever the session's status transitions to idle (the agent finished a
   turn), the backend re-reads the scratch file, parses it with the same
   `parse_skill_md` used for builtin skills, and emits a new
   `skill:draft-synced` event with `{name, description, content}`. The
   editor updates its fields from that event. A manual "Sync now" button
   triggers the same read+parse+return path on demand via a new command.
6. While a session is active, the editor's Name/Description/Content fields
   are read-only (agent is the sole writer) to avoid a two-writer race on
   the same file. Stopping the session (explicit "Stop agent" button, or
   closing the assist panel/editor) unlocks the fields; the last synced
   values remain in the editor's normal (unsaved) state, and the user saves
   with the existing "Save skill" button exactly as today (a normal
   `skill.save` IPC call — this feature never writes to the DB directly).
7. On stop/cleanup: `instance.stop`, then `workspace.delete` (which already
   cascades to delete `workspace_agent`/`session`/instance rows), then
   delete the scratch directory from disk.

This is additive to the existing spawn/session pipeline. `instance.spawn`,
`instance.stop`, `message.send`, the `session:output`/`session:status`
events, and `parse_skill_md` are all reused unmodified.

## Components

### 1. Data model: hidden workspaces

`workspace` gains a `hidden INTEGER NOT NULL DEFAULT 0` column (migration
0007). `repo::workspace::list` (backing `workspace.list`) filters
`WHERE hidden = 0`. Every other workspace repo function (`get`, `exists`,
`update`, `delete`) is unchanged — a hidden workspace is a completely normal
row otherwise, so `instance.spawn`'s existing prerequisites (workspace_agent
→ session → agent_definition → workspace, all required today) are satisfied
without touching `instance.spawn` itself.

`repo::workspace::create` gains a `hidden: bool` parameter (default `false`
at all existing call sites — the normal "link a project" flow never sets
it).

### 2. Scratch directory management

New module (e.g. `repo::skill_draft` or a section of `repo/skill.rs` —
finalized during planning): given a skill's current `name`/`description`/
`content`, materializes a temp directory
(`<app data dir>/skill-drafts/<uuid>/`) containing one `SKILL.md`, written in
the same two-frontmatter-field format `parse_skill_md` already parses
(`name:`, `description:`, then the body as `content`). Reading it back reuses
`parse_skill_md` verbatim — no new parser.

Cleanup removes the directory recursively; a failed cleanup (e.g. directory
already gone) is logged and swallowed, not surfaced as a user-facing error —
matches this codebase's existing "best-effort cleanup" precedent elsewhere
(confirmed against actual code during planning).

### 3. New IPC commands

- `skill.startDraftSession` — input: skill id (or none, for a new
  unsaved skill) is NOT sent; instead the current in-editor `name`/
  `description`/`content` and the chosen `agentDefinitionId` are sent
  directly (the skill may not be saved yet, so there's no DB id to depend
  on). Creates the scratch dir/file, hidden workspace, workspace_agent,
  session, and spawns the instance. Returns the ids the frontend needs to
  attach the existing session-output hook and `message.send` calls
  (`workspaceAgentId`, `sessionId`/instance id — exact shape finalized
  against the real `instance.spawn` response during planning).
- `skill.syncDraft` — input: the active draft session's identifier. Re-reads
  the scratch file, parses it, returns `{name, description, content}`.
  Also the handler invoked internally when relaying a `skill:draft-synced`
  event on the idle transition.
- `skill.stopDraftSession` — input: the active draft session's identifier.
  Runs the stop/cleanup sequence in step 7 above.

### 4. Frontend: `SkillEditor.tsx`

- Layout changes from a centered `fixed inset-0` modal to a full panel
  (exact placement — replacing main content vs. a wide slide-over — decided
  against the current `AppShell` layout during planning; both keep the same
  "opened globally, no workspace required" property `SkillLibrary` already
  has).
  - Where the assist panel is closed, this is a pure layout/component swap:
    same three fields, same `handleSave`, same `ipc.skill.save` call.
- `content` field switches from `<textarea>` to CodeMirror 6
  (`@uiw/react-codemirror` + `@codemirror/lang-markdown` — new
  dependencies, pure JS, no native bindings, safe under Tauri).
- New collapsible assist panel:
  - `AgentDefinition` picker (reuses the existing `agentDef.list` IPC call
    the Builder already uses for its own agent picker patterns).
  - Output/terminal view and input box, built from the same primitives
    `ChatView.tsx` already uses for `session:output` streaming and
    `message.send` — extracted into a small shared piece only if the
    existing component isn't already reusable as-is (assessed during
    planning; not assumed here).
  - "Stop agent" button calling `skill.stopDraftSession`.
  - "Sync now" button calling `skill.syncDraft`.
- Field lock: a boolean `assistSessionActive` disables the three fields
  (`disabled`/`readOnly`) and shows a banner, exactly as designed in the
  approved brainstorm.
- Cleanup on unmount / explicit close: if a session is active, call
  `skill.stopDraftSession` before closing (mirrors existing unmount-cleanup
  patterns already in the codebase — confirmed against actual code during
  planning, not assumed).

### 5. `SkillLibrary.tsx`

No behavior change. It still opens `SkillEditor` the same way; the editor's
internal layout change (modal → full panel) is transparent to the library —
though `showEditor`'s open call may need to route through `AppShell` instead
of rendering as an overlay from within `SkillLibrary`, if the full-panel
layout can't sensibly render inside the library's slide-over. This routing
detail is resolved during planning by reading `AppShell.tsx`'s actual
structure, not guessed here.

## Error handling

- `skill.startDraftSession` failing (e.g. scratch dir write failure, spawn
  failure) surfaces the existing `AppError` shape the frontend already
  handles for other IPC calls — same pattern as `SkillEditor`'s existing
  `handleSave` catch block.
- If the scratch file becomes unparsable by `parse_skill_md` (e.g. the agent
  wrote something that breaks the frontmatter block), `skill.syncDraft`
  returns the last successfully parsed values rather than erroring — the
  user is not blocked from continuing to interact with the agent, and can
  ask it to fix the format. This mirrors `parse_skill_md`'s existing
  "skip on malformed input" philosophy from ADR 0002.
- If `workspace.delete` cascade or directory removal fails during cleanup,
  it's logged, not surfaced — consistent with `skill_draft`'s best-effort
  cleanup policy above.

## Testing

- Rust: `repo::workspace::create`/`list` tests confirming `hidden` rows are
  excluded from `list` but behave identically to normal rows for `get`/
  `update`/`delete`/spawn prerequisites. `skill_draft`'s materialize/parse
  round-trip tests (write → `parse_skill_md` → matches input). Command-level
  tests for `skill.startDraftSession`/`syncDraft`/`stopDraftSession` following
  this codebase's existing command test conventions (in-memory DB, no real
  process spawn — the actual `instance.spawn` call is exercised by its own
  existing test suite, not re-tested here).
- Frontend: `tsc`/build clean, as with every prior arc in this codebase.
  Manual smoke-test of the full flow (start session, agent writes file, sync
  fires on idle, stop, save) is called out explicitly as a Tauri-runtime
  action that may not be verifiable in a non-interactive environment,
  matching this session's established disclosure pattern for anything
  requiring a running `.app`.

## Acceptance criteria

- Opening the custom-skill editor shows a full panel (not a small centered
  modal) with a real code editor for `content`.
- A user can pick one of their `AgentDefinition`s, start an assist session,
  chat with it, and watch it edit the skill's `SKILL.md`-shaped scratch
  file; on each idle transition the editor's fields update to match.
- Stopping the session (explicitly or by closing the editor) leaves no
  orphaned `workspace`/`workspace_agent`/`session`/instance rows and no
  leftover scratch directory.
- Hidden workspaces never appear in the normal workspace list/switcher.
- Builtin skills are completely unaffected — still read-only, still never
  routed through this editor.

## Known limitations

- Only one assist session per skill editor instance at a time.
- No cross-session persistence — closing and reopening "Ask agent to help"
  starts clean every time, even for the same skill.
- Sync latency is bounded by the agent's own turn-taking (idle transitions),
  not by keystroke; a long-running single turn won't show partial progress
  until it yields.

## Open questions resolved at plan-time

- Exact full-panel placement within `AppShell` (replace main content region
  vs. wide slide-over) — resolved by reading `AppShell.tsx`'s actual layout
  during planning.
- Exact shape of `instance.spawn`'s response/ids needed to reuse
  `useSessionOutput` from the assist panel — resolved by reading
  `instance.rs`/`events.ts` during planning.
- Whether `ChatView.tsx`'s output-rendering piece is directly reusable as a
  component or needs extracting — resolved by reading `ChatView.tsx` during
  planning.
