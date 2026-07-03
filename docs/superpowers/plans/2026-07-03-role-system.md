# Role system — first-class roles, self-describing roster

**Goal:** users pick a role (builtin or custom) when creating an agent; the role carries a
description + default skill bundle; every agent can see who its peers are, their role, and
their skills via `conclave agent list`. Decision record: `docs/adr/0005-role-system.md` (read
first, plus ADR 0001/0002 for the patterns this mirrors).

**Owner/lead:** Detoro `bfb737ff-486d-4581-b407-95711d5e07ab` — design/spec conflicts escalate
to me; implementation judgment within the plan is yours, logged in `progress:role-system`.

## Lane boundary — read this before anything else

Another implementer (Dew `40d90aed`) is actively editing
`src-tauri/src/engine/commands/agent.rs`, `src-tauri/src/engine/commands/instance.rs`, and
`src-tauri/src/engine/agentctx.rs` (plan `2026-07-03-skill-live-reload.md`). Those three files
are OUT OF BOUNDS for you until the lead posts `phase-b-clear` on
`progress:role-system`'s thread. Phase A below never touches them.

## Global constraints

- Repo `/Users/detoro/code/codeup`, branch `main`, current uncommitted tree. Do NOT commit —
  the lead owns integration.
- TDD per task: failing test first. Done = `cd src-tauri && cargo test --lib` green +
  `cargo clippy --lib` clean + (for UI tasks) `npm run build` clean; paste evidence into
  `progress:role-system`.
- UI copy in English. Follow the existing builtin-skills code as the template wherever the ADR
  says "mirror" — read `src-tauri/src/engine/repo/skill.rs` (`read_builtin_skills_from`,
  `bundled_skills_dir`, `parse_skill_md`) before writing the role equivalents.

## Phase A (start now — no lane conflicts)

### A1 — bundled builtin roles

`src-tauri/roles/<slug>/ROLE.md` for the five builtins (`lead`, `implementer`, `reviewer`,
`researcher`, `designer`). Frontmatter: `name`, `description` (one paragraph, English, written
as the agent's job description — it goes into preambles verbatim), `skills:` comma-separated
skill ids (per ADR 0005; mandatory skills are NOT listed — they attach regardless). New
`src-tauri/src/engine/repo/role.rs`: `read_builtin_roles_from(dir)` + `bundled_roles_dir()`
mirroring skill.rs; same parse-or-skip tolerance; tests mirror
`read_builtin_skills_from_parses_one_skill_per_subdir_skips_bad_ones` and
`shipped_skills_all_parse_and_include_collaboration` (assert all five parse and `lead` includes
`leadership`).

### A2 — custom roles in the DB

Migration in `src-tauri/src/engine/db.rs` (follow `migrate_adds_skill_system_columns`'s
pattern): new `role` table (id, name, description, skill_ids JSON, created_at) and
`agent_definition.role_id TEXT NULL`. Repo CRUD in `repo/role.rs`; commands in a new
`commands/role.rs` (`role.list` merges builtin + custom, `role.save`, `role.delete` — builtin
ids are rejected for save/delete, mirroring how builtin skills are protected). Register on the
IPC bus next to the `skill.*` methods. Deleting a custom role NULLs `agent_definition.role_id`
referencing it (keep the display-text `role` column value, so nothing loses its label).

### A3 — self-describing roster

`repo/workspace_agent.rs::list_by_workspace_with_launched_skills` (~155): extend the row with
agent display name, role name, role description, and launched skill NAMES (join/lookup —
builtin skill names come from the bundled reader, custom from the `skill` table). The CLI
(`src-tauri/src/bin/conclave-cli.rs`, `agent list`) passes the enriched JSON through untouched
— verify and add a CLI-shape test if one exists for `agent list`. Test: a fixture with a role
and two skills lists all four new fields.

### A4 — UI

- `src/components/Builder.tsx`: role picker at agent creation — dropdown of `role.list`
  (builtin first, then custom, then "Custom…" which opens inline name+description+skill-picker
  and saves via `role.save`). Picking a role pre-selects its default skills in the existing
  skill attachment UI (user can still edit before saving the agent). On save, send `roleId`
  and the final (possibly edited) skill selection — the COPY semantics from ADR 0005 live
  here in the UI, not in the engine.
- `src/components/Roster.tsx` + `ContextDrawer.tsx`: show role name (and description on
  hover/expand) per agent, using A3's enriched list.
- The `ipc.agent.save` payload gains `roleId`; engine-side handling of `role_id` persistence is
  Phase B (agent.rs is out of bounds) — until then the UI may send it and the engine ignores
  it; gate the UI work so nothing breaks when the field is dropped.

## Phase B (only after the lead posts `phase-b-clear`)

- `commands/agent.rs::save`: persist `role_id`; when a NEW definition is created with a role
  and the request carries no explicit skill list, attach the role's default skills (copy).
  Write `role` display-text from the role's name for fallback.
  **REVIEW OBLIGATION (Mellow, bb review:role-system):** `repo::role` returns a role's
  `skill_ids` UNFILTERED — the copy step here MUST drop ids that no longer resolve to an
  existing skill (mirror `effective_builtin_skills`'s ignore-unknown behavior), and the UI
  picker must do the same, else a role naming a deleted skill silently attaches nothing.
  Test: role with one live + one deleted skill id → only the live one attaches.
- `repo/workspace_agent.rs`: one-line comment on the roster's `INNER JOIN agent_definition`
  stating it relies on the enforced FK + restricted (non-cascade) delete — if either changes,
  agents silently vanish from the roster (review note 1).
- `agentctx.rs::bootstrap_preamble` (additive; coordinate with the lead — Dew may still hold
  the file): bake the agent's own role name + description (sanitized), and extend the roster
  sentence: `conclave agent list` now shows each peer's role and skills — consult it before
  delegating or asking a peer for something outside their role.
- `commands/instance.rs`: pass the resolved role into the preamble builder.

## Risk ledger

- `skill_ids` in the `role` table are unvalidated references; a deleted custom skill leaves a
  dangling id — filter unknown ids at read time (mirror `effective_builtin_skills`'s
  ignore-unknown behavior) instead of failing.
- Builtin role slugs collide with a future custom role name — custom roles get generated ids
  (uuid), never slugs; `role.save` rejects the five builtin slugs as names case-insensitively.
- The enriched roster JSON is consumed by running agents' `conclave agent list` — additive
  fields only, never rename/remove existing ones (`id`, `status`, `launchedSkillIds`).

## Definition of done

Phase A tasks green per the global gate with evidence in `progress:role-system`, then message
the lead. Phase B starts only on `phase-b-clear`.
