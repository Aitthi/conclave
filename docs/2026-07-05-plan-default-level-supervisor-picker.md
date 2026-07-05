# Plan: Default level on the agent definition + supervisor picker (add-flow & roster)

owner: bfb737ff-486d-4581-b407-95711d5e07ab (Detoro) · authority: in-loop
date: 2026-07-05 (late evening) · requester: human (two screenshots: Builder
Position section; Roster "CLI agents" list)

## Human request (verbatim intent, with the one fork settled by them)

1. Supervisor being per-workspace (re-pick after remove/re-add) is intentional
   and stays — confirmed by the human.
2. LEVEL must be REMEMBERED: set at agent CREATION (on the definition), so
   removing from a workspace and re-adding restores it.
3. Supervisor gets a picker MODAL at the moment of adding an agent to a
   workspace.
4. (Asked, human chose:) the same picker also opens from the Roster CLI-agents
   list — clicking a row's reports-to chip changes that member's supervisor
   without going through Builder. "เปิดได้ 2 ทาง".

## Current state (verified)

- `workspace_agent.level` + `supervisor_agent_id` are instance columns
  (migration `0015_position_system.sql`); engine reads (level_rank,
  supervisor_chain, org chart, preamble) all read the INSTANCE.
- `agent_definition` has NO level field.
- IPC already sufficient for the modal: `agentDef.addToWorkspace` returns
  `WorkspaceAgent[]` (`src/ipc/commands.ts:92-95`) and `instance.setPosition`
  exists (`commands.ts:158`, emits `roster:changed`).
- Add flow UI: `AddAgentPicker` in `src/components/Roster.tsx:740` (modal
  shell: 420px, rounded-2xl, h-12 header — the shell pattern to reuse).
- Roster rows already render the position line + reports-to chip (r13 item
  [8], canon @b2ac739).

## Decisions (settled — challenge with evidence via task challenge)

- **D1 — `agent_definition.default_level`** (nullable TEXT, CHECK in
  junior|mid|senior|principal — same enum as 0015) via NEW migration
  `0016_agent_default_level.sql`. It is a SEED, not a live link: the
  instance's `workspace_agent.level` remains the single source of truth for
  every engine read (routing, org chart, preamble). REJECTED: moving level
  fully onto the definition (touches every read path + spec §decision 2 keeps
  instance-level asymmetry; the human asked for "remembered", seeding gives
  exactly that).
- **D2 — Seeding happens ONLY when `instantiate` INSERTS a new row.** The
  idempotent-reuse path (existing row) NEVER overwrites instance level — a
  per-workspace customization must survive relaunches. Setting/changing
  `default_level` later never retro-writes existing instances.
- **D3 — Builder gets a "Level" picker on the DEFINITION** (create AND edit
  modes, all agent types), separate from the existing per-workspace Position
  section (which stays as-is for member-level overrides). Same 4-segment
  control + "Clear to Unranked" idiom as the Position section (canon
  @b2ac739) so the two read as the same concept.
- **D4 — One `SupervisorPicker` modal component, two entry points**:
  (a) add-flow: `AddAgentPicker` gains step 2 after choosing an agent —
  pick a supervisor from CURRENT workspace members, or the explicit
  "No supervisor — reports to Human" row; then `addToWorkspace` →
  `instance.setPosition({ workspaceAgentId: <new instance id from the
  response>, supervisorAgentId })`. (b) roster: clicking the reports-to chip
  on an AgentRow opens the same modal pre-scoped to that member →
  `instance.setPosition`. REJECTED: two bespoke pickers (drift); embedding
  the picker inline in the roster row (no room, and the modal is already
  needed for add-flow).
- **D5 — Canon by composition**: modal shell = AddAgentPicker's
  (Roster.tsx:773-786), rows = Builder Position supervisor rows (@b2ac739 —
  avatar, name, level glyph + track line), selection = click row (highlight,
  then Confirm) with self/descendants disabled in the roster-edit variant
  (engine's no-cycle rule; the add-flow variant has no descendants yet so
  only "self" never appears — the new agent isn't in the list at all).
  Arta (688719b6) does a DESIGN PASS at review time (joint with Armin) and
  is the design-escalation target; no new proto screen blocks the build.
- **D6 — Failure UX for the add-flow chain**: `addToWorkspace` succeeding but
  `setPosition` failing must NOT roll back the add (the agent IS in the
  workspace); surface the error inline in the modal ("Added, but setting the
  supervisor failed — set it from the roster chip or Builder") and still fire
  `onAdded`. No silent catch.

## Interface contract (PINNED — both lanes build to this, neither edits the other's side)

- SQL: `ALTER TABLE agent_definition ADD COLUMN default_level TEXT CHECK
  (default_level IN ('junior','mid','senior','principal'));`
- Rust: `AgentDefinitionRow.default_level: Option<String>`, serialized to the
  frontend as `defaultLevel` (match the existing serde casing convention in
  `repo/agent_definition.rs`).
- IPC (TS, frontend lane owns the file): `AgentDefinition.defaultLevel?:
  "junior" | "mid" | "senior" | "principal" | null`; `agentDef.create` /
  `agentDef.update` req gain optional `defaultLevel` with the same type.
  Backend accepts the field on the matching commands (absent = null/unchanged
  respectively; explicit null on update = clear).
- Seeding: in `repo/workspace_agent.rs::instantiate`, INSERT path only:
  `level = def.default_level`.

## Lane B — `default-level-engine` (implementer: Dabin · reviewer: Armin)

Boundary: `src-tauri/src/engine/migrations/0016_agent_default_level.sql`,
`src-tauri/src/engine/repo/agent_definition.rs`,
`src-tauri/src/engine/repo/workspace_agent.rs`,
`src-tauri/src/engine/commands/agent.rs`

AMENDED (challenge 194d244b, Dabin — upheld): `src-tauri/src/engine/db.rs`
is a RATIFIED boundary extension, limited to wiring 0016 into `migrate()`
(the `if version < 16` block + version assertions/tests). The plan's own
risk ledger named the loader risk yet the boundary omitted the loader file —
plan defect (Detoro). Mechanism per recorded convention: land the db.rs
edits as a SEPARATE scoped commit (`git commit -- src-tauri/src/engine/db.rs`
with Dabin's authorship) since `stage commit` honors only the original
boundary; everything else rides `stage commit` as usual.

1. Migration 0016 per the pinned contract (check how 0015 registers itself in
   the migrations list/loader and follow the same mechanism).
2. `agent_definition` repo: add the column to the row struct, `create`,
   `update`, and every SELECT that materializes the struct. Update = same
   partial-update idiom the file already uses; explicit-null clears.
3. `commands/agent.rs`: create/update handlers accept optional `defaultLevel`
   (serde default), pass through.
4. `workspace_agent.rs::instantiate`: on the INSERT branch only, read the
   def's `default_level` and write it as the new row's `level`. The reuse
   branch is untouched (D2). Do it in the same tx/query flow the function
   already has — do not add a second round-trip if the def row is already in
   hand at the call site.
5. Tests (same file's `#[cfg(test)]` conventions): (a) create def with
   default_level=senior → instantiate → instance level=senior; (b) re-
   instantiate after a manual instance-level change → unchanged (reuse path);
   (c) remove + re-instantiate → seeded again from the def; (d) update def
   default_level does not touch existing instances.

Gate (commit first): `conclave task gate <ws> default-level-engine -- sh -c
"cd src-tauri && cargo test --lib 2>&1 | tail -3"`

## Lane C — `supervisor-picker-ui` (implementer: Dew · reviewer: Armin + Arta design pass)

Boundary: `src/ipc/commands.ts`, `src/components/Builder.tsx`,
`src/components/Roster.tsx`, `src/components/SupervisorPicker.tsx` (new),
`src/lib/positions.ts`

AMENDED (challenge 639af566, Armin — upheld): `src/ipc/types.ts` is a
RATIFIED boundary extension, limited to adding `defaultLevel` on
`AgentDefinition` (types.ts:37) — the plan pinned the TS contract but pointed
at commands.ts while the interface lives in types.ts. Same defect class as
Lane B's db.rs omission, same session: TWO boundary misses in one plan, both
"the contract names a concept, the boundary misses the file that OWNS it".
Standing guard for future plans: for every type/constant a contract pins,
grep for its DEFINING file and put THAT in the boundary — the file you
remember using is often just the importer. Mechanism: types.ts lands as a
separate scoped commit with Dew's authorship (stage commit honors only the
original boundary).

1. `src/ipc/commands.ts`: pinned contract types (AgentDefinition.defaultLevel
   + create/update req fields).
2. `Builder.tsx`: "Level" 4-segment picker + Clear-to-Unranked on the
   DEFINITION form, create and edit modes, visually consistent with the
   Position section's Level control; wired into the create/update payloads.
   The existing per-workspace Position section is untouched.
3. New `SupervisorPicker.tsx`: modal per D4/D5. Props sketch:
   `{ workspaceId, members, excludeIds, current?, onPick(idOrNull), onClose,
   variant: "add" | "edit" }` — dumb about IPC; callers do the writes. Rows
   from the Roster's already-fetched member data; "No supervisor — reports
   to Human" always present.
4. `Roster.tsx` — add-flow: after clicking an agent in `AddAgentPicker`,
   swap the modal body to the supervisor step (same modal frame, back arrow
   returns to the list); Confirm → `addToWorkspace` → take the returned
   instance for THIS workspace → `setPosition` (D6 failure UX) → `onAdded`.
   Skip button = add with no supervisor.
5. `Roster.tsx` — chip entry: the reports-to chip on AgentRow becomes a
   button (stopPropagation — row click still selects the agent; keyboard
   focusable, aria-label "Change supervisor") → SupervisorPicker
   variant="edit" with self + descendants disabled (compute descendants from
   the roster's supervisor links — helper goes in `src/lib/positions.ts`
   beside the existing chain helpers) → `setPosition` on confirm.
   `roster:changed` already refreshes the list (r12 event).
6. Manual dev check: `pnpm dev` — add-flow both paths (pick + skip), chip
   edit, cycle prevention (a supervisor's supervisor can't pick its own
   descendant), keyboard: modal focus-trapped, Esc closes.

Gate (commit first): `conclave task gate <ws> supervisor-picker-ui -- sh -c
"pnpm build"`

## Global constraints (both lanes inherit)

- UI copy in English only. Level display names follow the existing ladder
  (Junior/Mid/Senior/Principal, "Unranked" for null) — `src/lib/positions.ts`
  is the single source for labels/rank order; do not restate the enum in
  components.
- Shared checkout: `conclave lane start` → work in the lane worktree →
  `conclave stage commit` → commit BEFORE gate.
- Out-of-boundary needs = task challenge to Detoro; pre-ratification
  precedent b90bf164 applies to known-shape one-liners.
- Parallel lanes, disjoint boundaries — neither lane edits the other's files;
  the pinned Interface contract section is the ONLY coupling. If the contract
  proves wrong, challenge the PLAN (I amend), don't patch across the boundary.
- Design canon: @b2ac739 (Position controls/rows) + AddAgentPicker shell;
  design escalations → Arta (688719b6).

## Risk ledger

- **Migration loader**: 0015 was registered somewhere beyond the .sql file —
  find and mirror it; a migration that isn't wired in passes local cargo test
  suites that recreate schema differently. Verify with a fresh-DB test run.
- **`instantiate` is idempotent by UNIQUE(workspace_id, agent_def_id)** — the
  INSERT-vs-reuse branch decides seeding; putting the seed on the wrong
  branch silently clobbers user levels (test b guards this).
- **addToWorkspace returns instances for MULTIPLE workspaces** (`workspaceIds`
  array) — pick the row whose `workspaceId` matches; don't assume `[0]`.
- **Chip-as-button inside a clickable row**: without stopPropagation the
  click also selects/focuses the agent (or removes — check AgentRow's hover
  actions); test both mouse and keyboard.
- **Descendant computation** must use the CURRENT roster snapshot — a stale
  member list can offer a cycle the engine will reject; treat engine
  rejection as the backstop, not the UX (disable rows client-side AND show
  the engine error if it still slips through).
- **dev DB is the production conclave.db** — do NOT test the migration by
  running the real app against a throwaway schema assumption; cargo tests use
  their own pool (see repo test conventions), keep it that way.

## Deferred (recorded, not in scope)

- Backfill/retro-apply of default_level to existing instances (explicitly
  rejected in D2 — record here so it isn't re-proposed).
- Supervisor default on the definition (per-workspace re-pick is intentional
  — human ruling #1).
- Org-chart drag-to-reassign supervisor (LaneBoard Org tab) — natural
  follow-on, needs its own design pass.
