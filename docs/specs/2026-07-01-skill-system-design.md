# Skill system v1 — design spec

Grilled with the user across a full interview (see [`docs/adr/0001-skill-system-v1.md`](../adr/0001-skill-system-v1.md) for the load-bearing architectural decisions and [`CONTEXT.md`](../../CONTEXT.md) for the `Skill` glossary entry). This spec turns those decisions into concrete files/schema/behavior for implementation.

## Goal

Let a user attach reusable, named instruction modules ("skills") to an `AgentDefinition`, so every `cli`-type agent launched from that definition receives the skills' content as standing instructions — on top of the existing identity/workspace bootstrap preamble — without the user retyping the same instructions into `role`/`customArgs` for every agent.

Two kinds of skill:
- **System** (builtin): shipped with the app, read-only, auto-attached to every `AgentDefinition`, cannot be detached.
- **Custom**: user-authored via full CRUD (name, description, content), freely attached/detached per `AgentDefinition`.

## Non-goals (deferred)

- `chat`-type agents. `runtime::chat::spawn_chat` has no system-prompt parameter today; wiring skills there requires adding that plumbing first, which is separate work.
- `orchestrator`-type agents. Still an unimplemented placeholder (fusion arrives in M4) — irrelevant until then.
- Per-skill icon selection. All custom skills use one fixed icon (`Sparkles`) in v1; the `skill.icon` column exists for a future per-skill picker.
- `agent_tool` / `tool` wiring. Out of scope — this spec only implements the `skill` / `agent_skill` half of the existing dormant scaffold.
- Live-reload of a running instance's skills. A skill change only takes effect on the next launch; this spec adds a UI badge to communicate that, not an actual hot-reload.

## High-level architecture

```
skill (DB)  ──┬── system rows (seeded via migration, kind='builtin')
              └── custom rows (user CRUD, kind='custom')
                       │
                agent_skill (join: agent_def_id, skill_id, sort_order)
                       │
        agentDef.save persists it (unblocks the existing dead `skillIds` field)
                       │
       at instance spawn (cli only): repo::skill::content_for_agent(agent_def_id)
                       │
     write concatenated, ordered, headered content to a per-instance sidecar file
                       │
        append ONE sanitized, single-line sentence to bootstrap_preamble
        pointing the agent at that file (preserves the preamble's single-line,
        no-'=' invariant — see ADR 0001)
                       │
        session.launched_skill_ids snapshot updated to the ids just used
                       │
   Roster compares session.launched_skill_ids vs current agent_skill ids
   per running instance → "Restart to apply" badge on drift
```

## Data model

New migration `src-tauri/src/engine/migrations/0004_skill_system.sql`:

```sql
-- Extend the dormant `skill` table with a builtin/custom discriminator (mirrors
-- `tool.kind`) and the actual instructional content (previously missing —
-- `description` is a short UI blurb, `content` is what gets injected).
ALTER TABLE skill ADD COLUMN kind TEXT NOT NULL DEFAULT 'custom' CHECK(kind IN ('builtin', 'custom'));
ALTER TABLE skill ADD COLUMN content TEXT NOT NULL DEFAULT '';

-- Seed builtin skills. Keyed on a stable id so re-running the migration is
-- harmless (mirrors 0002_seed_core_tools.sql). Empty for v1 — a placeholder
-- row-less seed until product decides what ships by default; add
-- `INSERT OR IGNORE INTO skill (...) VALUES (...)` rows here when that's
-- decided. (No row required for the mechanism itself to work.)

-- Track which skill set was actually used at the last launch, so the UI can
-- detect drift between that and the agent definition's current attachments
-- (`agent_skill`) and show a "Restart to apply" badge. JSON array of skill ids,
-- sorted, so a straight string comparison detects any add/remove/reorder.
ALTER TABLE session ADD COLUMN launched_skill_ids TEXT;
```

Note: `agent_skill.skill_id` already has `ON DELETE CASCADE` (`0001_init.sql:144`) — deleting a custom skill already cleans up its attachments with no further migration work.

## Backend

### `src-tauri/src/engine/repo/skill.rs` (NEW)

Mirrors `repo::agent_definition`'s shape:

- `SkillRow { id, name, description: Option<String>, content: String, kind: String, icon: Option<String> }`
- `list(pool) -> Vec<SkillRow>` — all skills, builtin first (`ORDER BY kind = 'custom', name`)
- `get(pool, id) -> Option<SkillRow>`
- `create(pool, name, description, content) -> SkillRow` — always `kind='custom'`; builtin rows are seed-only, never created through this path
- `update(pool, id, name, description, content) -> Option<SkillRow>` — must reject (return `Err`/`None` with a distinct signal) when the target row's `kind='builtin'`, enforced at the command layer (see below) since the repo layer shouldn't own authorization semantics
- `delete(pool, id) -> bool` — same builtin-guard requirement
- `attached_to_agent(pool, agent_def_id) -> Vec<SkillRow>` — joins `agent_skill`, ordered `kind = 'custom', sort_order`
- `set_custom_attachments(pool, agent_def_id, skill_ids: &[String])` — replaces the agent's **custom** `agent_skill` rows in one transaction (delete-then-insert with the given order), leaving builtin attachments untouched (they're not stored as rows at all — see below)
- `content_for_agent(pool, agent_def_id) -> (String, Vec<String>)` — returns `(concatenated_content, ordered_skill_ids_used)`; builds the sidecar body: all builtin skills (fixed `id` order) then custom skills via `attached_to_agent`, each rendered as:
  ```
  ## Skill: {name}

  {content}
  ```
  joined with blank lines. The returned id list is what gets written to `session.launched_skill_ids`.

**Builtin attachment is NOT a stored `agent_skill` row.** Since builtin skills apply to every `AgentDefinition` unconditionally, `content_for_agent` fetches ALL `kind='builtin'` rows directly rather than joining through `agent_skill` — this is what makes "cannot be detached" a structural guarantee instead of a UI-only restriction (there's nothing to detach). `agent_skill` rows only ever reference custom skills.

### `src-tauri/src/engine/commands/skill.rs` (NEW)

- `list(state, _payload) -> Vec<SkillRow>` → `skill.list`
- `save(state, payload: {id?, name, description?, content}) -> SkillRow` → `skill.save` (create if `id` absent, update if present; `AppError::Invalid` if updating a builtin row)
- `delete(state, payload: {id}) -> ()` → `skill.delete` (`AppError::Invalid` if builtin; annotate response... actually return `attachedAgentCount` isn't needed server-side, see UI section — the two-step confirm count is derived from `agentDef.list`'s existing per-def skill attachment, or a dedicated `skill.list` annotation `attachedTo: number`, mirroring `AgentDefinition.inWorkspaces`)

Extend `list`'s `SkillRow` → TS `Skill` with `attachedTo?: number` (count of `AgentDefinition`s with this skill attached), computed the same way `inWorkspaces` is computed for `AgentDefinition`.

### `src-tauri/src/engine/commands/agent.rs` (MODIFY)

- `SaveAgentReq.skill_ids` loses its `#[allow(dead_code)]` — `save()` now calls `repo::skill::set_custom_attachments(&state.db, &agent.id, &skill_ids.unwrap_or_default())` after upserting the `agent_definition` row, inside the same transaction. (`tool_ids` stays deferred — out of scope per Non-goals.)
- `list()`'s response gains nothing new here (skill attachments are fetched separately by the Builder via `skill.list` + a per-agent `agentDef.get`-style lookup, or by returning `skillIds` on `AgentDefinition` itself — **pick the latter for symmetry with the existing dead field**: add `skillIds: string[]` to the `AgentDefinition` list/get response, populated from `repo::skill::attached_to_agent`).

### `src-tauri/src/engine/commands/instance.rs` (MODIFY, cli branch only, around line 143)

Right after building `preamble`:

```rust
let (skill_body, skill_ids) = repo::skill::content_for_agent(&state.db, &def.id).await?;
let preamble = if skill_body.is_empty() {
    preamble
} else {
    let path = crate::engine::agentctx::write_skill_sidecar(&id, &skill_body)?;
    format!("{preamble} {}", crate::engine::agentctx::skill_pointer_sentence(&path))
};
repo::session::set_launched_skill_ids(&state.db, &session.id, &skill_ids).await?;
```

### `src-tauri/src/engine/agentctx.rs` (MODIFY)

- `write_skill_sidecar(instance_id: &str, body: &str) -> std::io::Result<PathBuf>` — writes `body` to `dirs::data_dir()/Conclave/skills/<instance_id>.md` (owner-only dir, same `0700` pattern as `ensure_conclave_shim`'s `bin` dir), overwriting on each launch.
- `skill_pointer_sentence(path: &Path) -> String` — one sanitized, single-line sentence: `"Additional standing instructions for this session are at {path} — read that file before your first response."` — run the existing `sanitize_field` on the path string defensively (paths shouldn't contain `=`/newlines, but the function must uphold its own invariant regardless of input).

### `src-tauri/src/engine/router.rs` (MODIFY)

Add `"skill.list"`, `"skill.save"`, `"skill.delete"` dispatch, following the exact pattern of the `agentDef.*` block.

## Frontend

### `src/ipc/types.ts` (MODIFY)

```ts
export interface Skill {
  id: string;
  name: string;
  description?: string;
  content: string;
  kind: "builtin" | "custom";
  icon?: string;
  attachedTo?: number;
}
```

Add `skillIds?: string[]` to `AgentDefinition`.

### `src/ipc/commands.ts` (MODIFY)

Add `skill.list` / `skill.save` / `skill.delete` entries + `ipc.skill.{list,save,delete}` bindings, mirroring `ipc.workspace`/`ipc.agentDef`.

### `src/components/Rail.tsx` (MODIFY)

New icon button (e.g. `Sparkles` from `lucide-react`) next to "Agent Library", `onOpenSkillLibrary?: () => void` prop, same style as the existing library button.

### `src/components/SkillLibrary.tsx` (NEW)

Mirrors `Library.tsx`'s structure: header, search, list. Two sections:
- **"System"** — read-only cards (name, description, no edit/delete buttons, no attach-count needed since they're universal).
- **"Custom"** — `SkillCard` components matching `AgentCard`'s pattern: name, description, `attachedTo` count label ("attached to N agents" / "Not attached to any agent"), two-step delete confirm, edit button opening `SkillEditor`.
- "New skill" button opens `SkillEditor` in create mode.

### `src/components/SkillEditor.tsx` (NEW)

Modal mirroring `Builder.tsx`'s general shape but much smaller: `name` (text input), `description` (text input, short), `content` (large `<textarea>`, markdown, monospace font matching `EditWorkspace`/`Builder`'s code-ish fields). Save via `ipc.skill.save`. No delete here — delete stays in `SkillLibrary`'s card, consistent with `Library`/`AgentCard` (delete lives in the list, not the editor).

### `src/components/Builder.tsx` (MODIFY)

New section (only for `type === "cli"`, since skills are cli-only in v1 — hide it entirely for `chat`/`orchestrator` the same way the CLI config section is conditionally shown):

- **"System skills"** subsection: static, non-interactive list of builtin skill names (fetched via `ipc.skill.list`, filtered `kind === "builtin"`) — no checkboxes, just a label "always on" per row.
- **"Custom skills"** subsection: checklist of all `kind === "custom"` skills; checking/unchecking updates local `skillIds` state, saved via the existing `agentDef.save` call's `skillIds` field (already threaded through, just add the field to the request builder + a "Manage skills" link opening `SkillLibrary` for creating a new one without leaving the flow — optional nicety, not required for v1).

### `src/components/Roster.tsx` (MODIFY)

For each running `WorkspaceAgent` row: compare `session.launchedSkillIds` (new field surfaced on `WorkspaceAgent`/`Session` IPC type) against the live `AgentDefinition.skillIds` (+ implicit builtin ids) for that instance's `agentDefId`. On mismatch, render a small badge ("Restart to apply", warning-colored) next to the agent's status indicator, matching the visual weight of existing status badges in that row.

## Behavior / invariants

- **Builtin skills are never absent.** `content_for_agent` always includes all `kind='builtin'` rows regardless of `agent_skill` — there is no code path that produces a cli agent without them once any exist.
- **Preamble stays single-line, `=`-free.** The skill pointer sentence is the ONLY thing appended to `bootstrap_preamble`'s return value; the multi-line/`=`-containing skill body itself never enters that string. `agentctx.rs`'s existing tests (`preamble_is_single_line_with_no_equals`, etc.) continue to hold; add a new test asserting the pointer-appended preamble also satisfies both invariants even when `skill_body` is pathological (contains `\n`/`=`/is huge).
- **Injection order is deterministic.** System skills first (fixed `id` order), then custom by `agent_skill.sort_order` — both the sidecar file and `session.launched_skill_ids` use this same order, so the snapshot comparison in Roster is a straightforward list-equality check, not a set comparison (order changes also trigger the badge, matching the "content actually differs" intent).
- **Deleting a custom skill cascades silently.** `ON DELETE CASCADE` on `agent_skill.skill_id` already handles this at the DB layer; no extra command-layer cleanup needed. The UI's two-step confirm + "attached to N agents" count is the only safeguard (informational, not a hard block).
- **Builtin skills reject mutation at the command layer**, not just by hiding UI buttons — `skill.save`/`skill.delete` on a `kind='builtin'` id return `AppError::Invalid`, so a stale/tampered frontend request can't mutate them.
- **Sidecar files accumulate one per instance, overwritten each launch.** No cleanup-on-delete-workspace logic is added in this spec — matches the existing precedent of `ensure_conclave_shim`'s shim file not being torn down on app quit either. (Flag as a known limitation below, not silently ignored.)

## Tests

Rust (`cargo test --lib`):
- `repo::skill`: create/update/delete round-trip; `update`/`delete` reject `kind='builtin'`; `set_custom_attachments` replaces cleanly (add, remove, reorder); `content_for_agent` orders builtin-then-custom and renders the `## Skill:` header format; cascade — deleting a skill removes its `agent_skill` rows without touching the `agent_definition`.
- `commands::skill`: `save`/`delete` return `AppError::Invalid` for builtin ids; `list` annotates `attachedTo` correctly.
- `commands::agent`: `save` with `skillIds` persists `agent_skill` rows; a subsequent `save` with a different `skillIds` list replaces rather than appends.
- `commands::instance`: spawning a `cli` agent with attached skills writes the sidecar file with expected content/order and sets `session.launched_skill_ids`; spawning with zero skills attached does NOT append a pointer sentence to the preamble (empty-skill-body short-circuit).
- `agentctx`: existing single-line/no-`=` tests still pass; new test with a pathological skill body (embedded `\n`/`=`) confirms the resulting preamble (identity + pointer sentence) still satisfies both invariants — only the pointer sentence is checked, not the file contents, which are intentionally NOT constrained the same way.

Frontend (`pnpm exec tsc --noEmit` + manual smoke, no existing test runner found for components):
- `SkillLibrary`/`SkillEditor` compile and round-trip through `ipc.skill.*`.
- `Builder`'s custom-skill checklist reads/writes `skillIds` correctly across save/reload.
- `Roster`'s badge appears/disappears correctly given mocked `launchedSkillIds` vs current `skillIds` (manual smoke in the running app, per the CLAUDE.md instruction to verify UI changes empirically rather than by static reading alone).

## Acceptance criteria

- [ ] Migration `0004_skill_system.sql` applies cleanly on top of `0003_agent_cli_config.sql`.
- [ ] A user can create, edit, and delete a custom skill via the new Skill Library UI.
- [ ] Builtin skills (once seeded) show as read-only in the Library and cannot be edited/deleted via UI or a direct IPC call.
- [ ] Attaching/detaching a custom skill to a `cli`-type `AgentDefinition` persists across app restart.
- [ ] Launching a `cli` agent with attached skills produces a sidecar file at the documented path containing the expected concatenated, headered content, and the launched CLI's system prompt (verified via a real Claude Code / Codex launch, not just unit tests) references that file.
- [ ] `chat`/`orchestrator` type agents show no skill UI in `Builder` and are unaffected by this feature end-to-end.
- [ ] Roster shows "Restart to apply" on a running instance after its definition's skill attachments change, and the badge clears after the instance is relaunched.
- [ ] All DoD baselines pass: `cargo test --lib`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `pnpm exec tsc --noEmit`, `pnpm build`.

## Known limitations (v1)

- `chat` and `orchestrator` agent types get no skill support (see Non-goals).
- No default/seeded builtin skill content is decided yet — the seed migration ships with the mechanism but zero rows; product can add `INSERT OR IGNORE` rows later without a further migration.
- Per-instance sidecar files under `Conclave/skills/` are never cleaned up (not on workspace delete, not on app quit) — matches existing precedent (`ensure_conclave_shim`) but is a real, if minor, disk-usage limitation worth revisiting if it becomes noticeable.
- The "Restart to apply" badge only detects drift for `cli` instances; it's meaningless (and won't be shown) for `chat`/`orchestrator` rows.

## Open questions resolved at plan-time

None outstanding — all forks were resolved during the grilling interview (see ADR 0001 for the load-bearing ones). The only remaining judgment call left to the implementer is exact CSS/spacing for the new Rail icon and badge, which should follow existing sibling components' conventions rather than needing a fresh decision.
