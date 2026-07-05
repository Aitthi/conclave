# Spec — Position System (Track + Level + Supervisor chain)

- **Status:** DRAFT for lead review (task `position-spec`, owner Detoro `bfb737ff`, authority in-loop)
- **Author:** Guetta (`2b110fd3`, Researcher) · 2026-07-05
- **Parallel work:** `position-design` (Arta `688719b6`) owns the UI canon; this spec owns engine semantics.
- **Scope:** research + design. No implementation lanes cut here — this document is what the lead turns into lanes.

## 0. Human-ruled decisions this spec designs within (do NOT re-open)

1. **Positions change engine behavior via escalation ROUTING only** — challenges, escalations, and stall
   alerts route up a supervisor chain; a sub-lead owns its domain, the lead is the tiebreaker. **No hard
   permission enforcement** — a low-level agent is never blocked from any verb. Protocol lives in skills;
   routing lives in the engine. (Rejected: display-only; hard enforcement.)
2. **Structure = Track + Level + supervisor link.** Track ≈ role (Implementer/Reviewer/Researcher/
   Designer/Lead). Level = a small ordered set within a track. Supervisor link is **per workspace
   membership (instance-level)**, not on the global role definition. (Rejected: full levels.fyi ladder;
   supervisor-link-only.)

Everything below is a consequence of these two rulings. Where I had a genuine fork, it is in §7 (Open
questions) with my recommendation, not silently decided.

---

## 1. Current-state map (what exists today, cited)

### 1.1 Role system (= the "Track" axis; already shipped, ADR 0005)

- **`role` table** — custom (user-authored) roles only: `id, name, description, skill_ids (JSON), created_at`.
  `src-tauri/src/engine/migrations/0008_role_system.sql:9-15`.
- **`agent_definition.role_id`** — nullable back-pointer to a role; a *builtin slug* (e.g. `lead`) or a
  custom `role.id`. No FK (builtin ids are never DB rows). `0008_role_system.sql:23`. The legacy free-text
  `agent_definition.role` column (`0001_init.sql`) survives as a display fallback.
- **Builtin roles ship as folders** `roles/<id>/ROLE.md` (frontmatter `name`/`description`/`skills`), read
  at runtime — never DB rows. `src-tauri/src/engine/repo/role.rs:178` (`list_builtin`), `:279` (`roles_dir`),
  parser `:243`. The five shipped tracks — `lead, implementer, reviewer, researcher, designer` — are
  asserted in `role.rs:471` and live in `src-tauri/roles/`.
- **`RoleRow`** = `{id, name, description, skill_ids, kind}`. `role.rs:24-37`.
- **Roster read** `list_by_workspace_with_launched_skills` joins `agent_definition` and resolves role
  name/description + skill names + model + cli_kind into **`WorkspaceAgentWithSkills`**.
  `src-tauri/src/engine/repo/workspace_agent.rs:223` (query), `:131-180` (struct).
- **Builder role picker** — `roleId` state `src/components/Builder.tsx:194`; `applyRoleTransition` (copy-in/
  copy-out of role default skills) `:227`; `selectRole` `:239`. Role look map `:45-63`.
- **Preamble** — an agent's launch briefing that bakes name + role + role description so it knows its job
  before its first roster query: `src/engine/agentctx.rs:35` (`bootstrap_preamble`), role_description baked
  at `:55`.

**Key structural fact:** `role_id`/`role` are on **`agent_definition` (global)**; the same definition
instantiated in two workspaces shares one track. Level and supervisor, per decision 2, are **per
workspace_agent (instance)** — an asymmetry we keep on purpose (§2.3).

### 1.2 The instance table (= where Level + Supervisor attach)

- **`workspace_agent`** = one (workspace, agent_definition) pair: `id, workspace_id, agent_def_id, status,
  added_at`, `UNIQUE(workspace_id, agent_def_id)`. `0001_init.sql` (table); repo `workspace_agent.rs:32-42`.
- `get(id)` `:92`, `list_by_workspace` `:103`, idempotent `instantiate` `:373` (id is **stable across
  relaunches** — instantiate reuses the existing row, so it is safe to reference as a supervisor FK target).
- **`remove(id)`** `:468` deletes an instance and manually NULLs/clears every non-cascading reference
  (`inter_agent_message`, `blackboard_*`, `message.from_instance_id`, `fusion_panel_response`) inside one tx
  before the final DELETE. **A new self-reference (`supervisor_agent_id`) MUST be handled here** (§8 risk 1).

### 1.3 Task authority today (who owns, who rules)

- **`task`** — `owner_agent_id` (nullable), `implementer_agent_id` (nullable), state machine.
  `0012_task_system.sql:1-16`. `task_event(kind IN note|state|gate|challenge|ruling)` `:17-24`;
  `task_watch(task_id, agent_id)` `:26-30`.
- **Owner** is stamped at `create` (`owner_agent_id` from payload, scope-checked). `commands/task.rs:341`,
  repo `repo/task.rs:162`.
- **Implementer** is stamped atomically at `claim` (`planned -> claimed` + `implementer_agent_id = actor`).
  `repo/task.rs:411`, `commands/task.rs:442`.
- **Challenge** `commands/task.rs:623` — stores an **absolute** `deadlineAt` (`now + deadlineMin`) computed at
  insert `:637`; absent `deadlineMin` = advisory (no timer).
- **Ruling** `commands/task.rs:676` — `payload.by = actor` `:684`. **There is NO authority gate on who may
  rule** — `enforce_scope` (`:61`) only checks the actor exists and belongs to the workspace. Any workspace
  member can file a `ruling`. This is consistent with decision 1 (no hard enforcement) and stays that way;
  the chain changes *who gets notified/expected to rule*, not *who is allowed to*.
- **Watcher wake filter** — `wakes_watchers` `commands/task.rs:871` is the single predicate; `notify_watchers`
  `:908` injects one line per watcher via `message::inject`. Wakes on: every `challenge`/`ruling`, a `state`
  → `review|abandoned|merged`, a failing `gate`, a `READY|BLOCKED|ESCALATION`-prefixed note.

### 1.4 Stall engine (who gets paged today)

- Single app-wide loop `runtime/task_timer.rs`, tick every 5 min (`TICK_INTERVAL:42`); `tick()` `:131` runs
  three checks.
- **Stall check** `check_stalls:148` — a `claimed|in_progress` task whose newest `task_event` is
  `STALL_MINUTES = 10` (`:51`) old pages the **task owner**, attributed `from = implementer` `:186`, at most
  once per 30 min (`:54`). Requires both owner and implementer present `:172`.
- **Challenge-default check** `check_challenge_deadlines:191` — past `deadlineAt` with no matching ruling,
  inserts a default ruling `by:"default"` `:241`, emits `task:changed` `:254`, notifies **actor + owner**
  `:269-277`.
- **Attribution rule (load-bearing, RULED 2026-07-04):** there is no "system" sender —
  `inter_agent_message.from_instance_id` is NOT NULL. Every auto-notify is attributed to a **real** party to
  the event and carries an in-body **`AUTO`** marker so the recipient doesn't read it as hand-typed
  (`:180`, `:268`). Any new routing notification MUST follow this (§8 risk 3).

### 1.5 Migration runner + skill sidecars

- **Migrations** apply via `db.rs migrate()` `:63`: `PRAGMA user_version` counter, one `if version < N { …;
  PRAGMA user_version = N; }` block per file, atomic apply-and-bump. **Highest committed = 13**
  (`0013_memory_proposal`). In-flight lanes already claim `0014` (`design-artifact-store`) — the position
  migration must take the **next free index after those lanes land** (§8 risk 2).
- **Skill sidecars** — the leadership/collaboration/agent-loop text agents actually read is generated from
  `src-tauri/skills/<skill>/SKILL.md` and only reaches live agents after **rebuild + relaunch** (the sidecars
  regenerate from the installed bundle's `Resources/skills`). Editing `SKILL.md` is necessary but not
  sufficient — it ships on the next build (§6, §8 risk 5).

---

## 2. Schema delta

### 2.1 New migration `00NN_position_system.sql`

> `00NN` = the next free index at implementation time (≥ `0015`, after the artifact/design-viewer lanes
> land). The matching `if version < NN` block goes in `db.rs` right after the current last block (`db.rs:63`
> pattern). Do not hardcode 15 until the in-flight lanes are merged.

Two nullable columns on the **instance** table. Additive `ALTER TABLE ADD COLUMN` — no table rebuild needed
(we only add nullable columns; every existing row defaults to NULL, which *is* the backward-compat state):

```sql
-- Position System. Level + supervisor are per-workspace-membership (instance),
-- NOT on the global agent_definition/role. Both nullable: an all-NULL workspace
-- behaves exactly as it did before this migration (no chain, no routing change).

-- Level within a track. A fixed, ordered vocabulary (rank resolved in Rust:
-- junior=1 < mid=2 < senior=3 < principal=4; NULL = unranked, sorts lowest).
-- CHECK keeps typos out; NULL stays legal for backward compat.
ALTER TABLE workspace_agent ADD COLUMN level TEXT
  CHECK (level IN ('junior','mid','senior','principal'));

-- Supervisor = another workspace_agent IN THE SAME WORKSPACE, or NULL = reports
-- to the human/top. Self-referential FK; ON DELETE SET NULL so removing a
-- supervisor orphans its reports to the top rather than aborting the delete.
ALTER TABLE workspace_agent ADD COLUMN supervisor_agent_id TEXT
  REFERENCES workspace_agent(id) ON DELETE SET NULL;

CREATE INDEX idx_workspace_agent_supervisor
  ON workspace_agent(supervisor_agent_id);
```

**SQLite caveat to verify at build time (risk, not a blocker):** `ALTER TABLE ADD COLUMN` permits a
`REFERENCES … ON DELETE SET NULL` clause only when the column's default is NULL (it is). Boot a fresh dev DB
and confirm the FK action fires (delete a supervisor, assert reports go NULL). If any SQLite version rejects
the FK-on-add-column, the fallback is the artifact-lane pattern: rebuild `workspace_agent` via
`CREATE TABLE … / INSERT SELECT / DROP / RENAME` (see `0014` sketch for the idiom). Prefer the two
`ADD COLUMN`s; they are far cheaper and touch no existing rows' data.

### 2.2 Level vocabulary (proposed exact set)

`junior < mid < senior < principal` — 4 rungs, matching the human's levels.fyi example, small enough for a
workspace of ~5-10 agents. Stored lowercase; a `level_rank(&str) -> u8` helper in `repo::workspace_agent`
maps to 1-4 (unknown/NULL → 0). Confirmed with Arta (task note) so the roster badge + Builder picker render
this set.

- **Track is NOT part of level.** "Lead" is a track; a Lead may sit at any level. Hierarchy comes from
  supervisor links, not from level. Level is a *secondary* signal — see §2.4.

### 2.3 Track lives on the definition; Level+Supervisor on the instance (intended asymmetry)

Decision 2 is explicit: supervisor is per-membership. Level rides with it on `workspace_agent` (plan §1
names both there). Track stays where it already is (`agent_definition.role_id`). Consequence: an agent
definition reused across workspaces keeps one track everywhere but can hold a different level and a different
supervisor per workspace. This matches how a person is "a backend engineer" globally but "senior, reporting
to X" only within one org. No migration touches `agent_definition`.

### 2.4 What Level is actually FOR (be honest about engine use)

Routing (§3) is **purely structural** — it follows `supervisor_agent_id` links, never level. Level's engine
consumers in v1 are thin and deliberately so:

- surfaced in the roster payload + preamble (so leads/skills can pick a *higher-level* reviewer or ruler);
- a tiebreak input *only* if an LCA is ambiguous (it never is — LCA is deterministic), so effectively display
  + protocol signal.

This is the honest answer to why decision 2 rejected supervisor-only: without a level, nothing tells a lead
which of two peers is the more senior reviewer. Level fills that, as a signal, without gating any verb.
Whether level should ever gate engine behavior is Open Question Q3 (recommend: no, v1).

---

## 3. Routing semantics — precise, per event kind

Primitives (new `repo::workspace_agent` functions; all depth-bounded to the workspace's agent count so a
pre-existing corrupt cycle can never loop forever):

- `supervisor_of(ws_agent_id) -> Option<id>` — one hop up (NULL = human/top).
- `supervisor_chain(ws_agent_id) -> Vec<id>` — `[self, sup, sup², …]` until NULL. The tail is implicitly the
  human.
- `lowest_common_supervisor(a, b) -> Option<id>` — §4.

**Backward-compat invariant (holds for every case below):** with all supervisors NULL, every routing rule
degrades to *today's* behavior byte-for-byte. The chain only ever *adds* a hop when a link exists.

### 3.1 Challenge filed (`task challenge`)

- **Today:** records the event; wakes all watchers (`wakes_watchers` → `challenge` always true).
- **Change (AMENDED per challenge 75af4e44, Tiësto — the original wording notified a non-watching owner
  even at all-NULL, contradicting the supreme invariant):** the expected-ruler supplement is the single
  primitive `lowest_common_supervisor(challenger, owner)` — notify that agent if non-None and not already
  a watcher. This self-gates on the chain being engaged: challenger under the owner's chain → owner;
  cross-chain → the LCA (§4); either party chainless / all-NULL → None → watchers-only, byte-for-byte
  today. No human ping is fabricated (decision 1: the loop settles itself).
- **All-NULL:** identical to today (watchers only) — including for a non-watching owner.

### 3.2 Challenge deadline expiry (`task_timer::check_challenge_deadlines`)

This path carries the hard **"the loop cannot silently stall"** guarantee — the stated default *must* still
fire. So the chain adds **visibility, not a second timer**:

- **Today:** on expiry, insert the default ruling, notify actor + owner.
- **Change:** unchanged default-fires semantics, PLUS — if the owner has a supervisor — one extra `AUTO`
  notification **up one level**: notify `supervisor_of(owner)` that "a challenge your report owned lapsed to
  its default." The supervisor learns their delegate let a dispute default; they can re-open with a fresh
  `rule` if they disagree (rulings are not final-locked — a later ruling can supersede).
- **Rejected alternative (extend-and-escalate):** hold the default, grant the supervisor a second window,
  fire only if *they* also lapse. Rejected for v1: it adds per-challenge timer state and *weakens* the
  cannot-stall guarantee. Recorded as Q1 (recommend: keep default-fires + escalation notice).
- **All-NULL:** identical to today.

### 3.3 Stall alert (`task_timer::check_stalls`)

- **Today:** pages `owner`, `from = implementer`.
- **Change:** page the **implementer's supervisor** (the agent who delegated the work is the one who should
  chase a quiet delegate). Resolution order: `supervisor_of(implementer)` → if NULL, `owner_agent_id`
  (today's target) → if NULL, no page. Attribution/`AUTO` marker unchanged (`from = implementer`, the real
  party). Repeated stalls past the 30-min cooldown keep paging the **same** immediate supervisor in v1 —
  walking a level higher on each repeat is Q4 (recommend: defer; immediate supervisor each time).
- **All-NULL:** `supervisor_of` is NULL → falls through to `owner` = today's behavior.

### 3.4 Task review-ready (`state -> review`)

- **Today:** wakes watchers (the owner usually watches).
- **Change (AMENDED per challenge 75af4e44, Tiësto — same contradiction as §3.1; review-ready has no
  second party to LCA against, so it gates explicitly):** the owner even-if-not-watching supplement fires
  ONLY when the owner participates in a chain (`supervisor_agent_id` set on the owner). A chainless owner
  = watchers-only, exactly today. "Who integrates" becomes explicit precisely when the workspace has
  opted into the org structure, never before.
- **All-NULL / no owner:** identical to today — including for a non-watching owner. The all-NULL
  byte-for-byte invariant is SUPREME over every routing clause in this section.

### 3.5 Supervisor-link write validation (cycle + scope)

On any write of `supervisor_agent_id = S` for agent `A` (new command §5.1), reject at the command layer
**before** the UPDATE if:

1. `S == A` (self-reference), or
2. `S` is not in the same workspace as `A` (reuse `enforce_scope`), or
3. `A` already appears in `supervisor_chain(S)` — i.e. setting the link would close a cycle `A → … → S → A`.
   Walk `S`'s chain (bounded by workspace size); if `A` is hit, reject.

Rejection is an `AppError::Invalid("supervisor link would create a cycle")`. The Builder picker (§5.2)
pre-filters descendants so the user rarely hits this, but the engine is the guard of record.

---

## 4. Tiebreaker — lowest common ancestor (cross-chain disputes)

When a dispute crosses two chains (e.g. two implementers under different sub-leads argue over a shared
interface), the ruler is the **lowest common ancestor (LCA)** of the two agents in the supervisor forest —
the nearest agent who has authority over both. If the chains never intersect (separate trees, both rooting at
the human), the tiebreaker is the **human**.

**Algorithm** (`lowest_common_supervisor(a, b) -> Option<id>`, None = human):

1. `chain_a = supervisor_chain(a)` (includes `a` itself as position 0).
2. Walk `supervisor_chain(b)` in order; return the **first** id that is also in `set(chain_a)`.
3. If the walk exhausts with no hit → `None` (human is the tiebreaker).

Including each agent as position-0 of its own chain means: if `a` is `b`'s supervisor, the LCA is `a`
itself (correct — `a` already has authority over `b`). Depth-bounded to workspace size; a corrupt cycle
returns the first repeat rather than looping.

**Where the engine uses it:** the LCA is *surfaced*, not auto-enforced — a cross-chain `challenge` computes
the LCA and notifies that agent as the expected ruler (and the human if None), the same
notify-the-expected-ruler mechanism as §3.1. No verb is blocked. Level is not consulted; the LCA is
structurally unique.

---

## 5. IPC / CLI surface

### 5.1 New command: set an instance's position

Level + supervisor live on `workspace_agent`, so they **cannot** ride `agent.save` (which writes the global
`agent_definition`). New command targeting the instance:

- **`instance.setPosition`** `{ workspaceAgentId, level?, supervisorAgentId? }` → updated
  `WorkspaceAgentWithSkills`. `level`/`supervisorAgentId` each nullable (send `null` to clear). Validates
  cycle + scope (§3.5) before writing. Emits an existing roster-refresh event (reuse whatever
  `instance.list` consumers already subscribe to; if none, add `roster:changed { workspaceId }` following the
  `task:changed` template).
- **CLI:** `conclave position set <ws> <agentId> [--level <l>] [--supervisor <agentId>|--supervisor none]`.
  `--supervisor none` clears the link. Prints the resulting row. (Author resolution reuses the existing
  calling-agent mechanism, same as `task note`.)

### 5.2 `conclave agent list` additions

Extend the roster payload struct `WorkspaceAgentWithSkills` (`repo/workspace_agent.rs:131`) and its query
(`:233`) with:

- `level: Option<String>` (the enum, or absent),
- `supervisor_agent_id: Option<String>` (`supervisorAgentId` on the wire),
- `supervisor_name: Option<String>` (`supervisorName` — resolved via the same name-resolution the roster
  already does; NULL = reports to human).

All three additive and `skip_serializing_if = "Option::is_none"`, so existing consumers and all-NULL
workspaces see no shape change.

### 5.3 New verb: `conclave org <ws>`

Print the workspace hierarchy as an indented tree (roots = agents with NULL supervisor, under an implicit
"human" top), each node showing `name · track · level · working/idle`. Backed by a new read command
`instance.orgChart { workspaceId }` returning the roster plus the resolved forest (or the frontend derives
the tree from the flat roster + `supervisorAgentId` — recommend deriving on the client to avoid a second
command; §7 Q5). CLI name `org` (not `org chart` — single-token verb matches `task`, `lane`, `bb`, `memory`).

### 5.4 Preamble: an agent learns its own position on launch

Thread level + supervisor name into `bootstrap_preamble` (`agentctx.rs:35`) so a fresh/restarted agent knows
where it sits before its first roster query — the same rationale role_description is already baked (`:55`).
Add one sanitized clause after the role clause, e.g.: *"Your level is Senior and you report to \"Detoro\"
(escalate up your chain, not around it); an agent with no supervisor reports to the human."* Empty when both
are NULL (backward compat). Requires the launch path that builds the preamble to pass the two new fields
(they are already fetched for the roster).

---

## 6. UI touchpoints (list only — Arta owns designs in `position-design`)

1. **Roster card badge** — track + level + reporting line on the 266px card (`Roster.tsx`).
2. **Builder position section** — level segmented picker (4 rungs) + supervisor dropdown (peers minus self
   minus descendants) + live chain preview; extends the existing role/card picker language (`Builder.tsx`,
   role picker at `:192-256`).
3. **Org chart display** — the supervisor forest; placement per Arta (candidate: a tab in Lane board).
4. **Escalation trace** — on a Lane board task detail, show a routed challenge walking up the owner's chain.

Data contract already sent to Arta (task note, this task): `level`, `supervisorAgentId`, `supervisorName` on
the roster; org tree = the `supervisor_agent_id` forest; escalation trace = challenge→ruling events walked up
the owner's chain.

---

## 7. Skill-text amendments (exact edits; ship on next rebuild+relaunch)

These teach agents the chain protocol. Source files under `src-tauri/skills/`. Sidecars regenerate on
rebuild+relaunch only.

### 7.1 `src-tauri/skills/collaboration/SKILL.md` — Escalation section (`:65-71`)

Add after the existing two bullets:

> - Escalate **up your supervisor chain**, not sideways or around it. Your supervisor is named in your launch
>   briefing and in `conclave agent list` (`supervisorName`); an agent with no supervisor reports to the
>   human. Take a blocker to your supervisor first — only a genuine scope change, spend/publish, or
>   irreversible action goes past the chain to the human.

### 7.2 `src-tauri/skills/agent-loop/SKILL.md` — "Grill each other" section (after `:46`)

Add a bullet on where a challenge routes:

> - A challenge routes to the task **owner** (the sub-lead who owns that domain); if it crosses two chains
>   (both sides report through different supervisors), it routes to the **lowest common supervisor** of the
>   two — the nearest agent with authority over both, the human if none. A `--deadline-min` that lapses still
>   fires your stated default (the loop cannot stall); the owner's supervisor is notified that it lapsed.

### 7.3 `src-tauri/skills/leadership/SKILL.md` — the "sub-lead" sentence

Leadership already says (serialization-point section): *"Scale by adding a sub-lead with its own recorded
authority, not by widening one lead's span."* Amend to make the mechanism concrete:

> Scale by adding a **sub-lead**: give a senior agent a supervisor link to you and make it the `owner` of a
> domain's tasks (`conclave position set <ws> <agentId> --level senior --supervisor <yourId>`). Its reports
> escalate to it; unresolved disputes and lapsed challenges surface up to you automatically. You remain the
> tiebreaker at the lowest common ancestor.

And in the "Split authority explicitly" bullet, add: *challenges route to the task owner by default and to
the lowest common supervisor across chains; you rule anything that reaches you.*

### 7.4 `ROLE.md` files — no change required

Track descriptions in `roles/*/ROLE.md` are unchanged; position is orthogonal to the role's job text.

---

## 8. Risk ledger

1. **`workspace_agent::remove` must clear the new self-FK.** `ON DELETE SET NULL` handles it automatically
   *if* FK enforcement is on for that delete path — but `remove` (`workspace_agent.rs:468`) runs a manual
   multi-statement tx. Verify the cascade fires there; if not, add
   `UPDATE workspace_agent SET supervisor_agent_id = NULL WHERE supervisor_agent_id = ?` alongside the
   existing NULL-out statements, and extend the `remove_clears_all_references` test (`:610`) to seed + assert
   it. This is the single most likely FK-abort bug.
2. **Migration numbering is contended.** Head is `0013`; `0014` (artifact) and a design-viewer lane are
   in-flight. The position migration takes the next free index **after** those land, with the matching
   `db.rs` `if version < NN` block. Do not author `0015`/`db.rs` until the in-flight lanes merge, or rebase
   the number at integration.
3. **Auto-notify attribution (AMENDED at P3 review — the original lumped two classes together).**
   TIMER-generated pages (stall→supervisor, deadline→supervisor) reuse `message::inject`, attribute to a
   **real** party, and carry the in-body `AUTO` marker — they fire spontaneously in a party's voice, which
   is what AUTO disambiguates. COMMAND-path supplements (challenge→ruler, review-ready→owner) are
   event-notification lines fanned out from a live actor's action, the same class as today's watcher
   wakes — which carry NO `AUTO` (verified: task.rs has none) — so they match the watcher-line format
   exactly, no AUTO. A watching ruler must receive the byte-identical line format they receive today.
   The RULED-2026-07-04 contract (`task_timer.rs:180,268`) governs the timer class: a TIMER page
   missing `AUTO` is a review block; a command-path supplement carrying a spurious `AUTO` is equally a
   defect (it would differ from the watcher line beside it).
4. **Live task system, 13+ merged tasks of history.** Every existing task has NULL owner/implementer
   supervisors → the backward-compat invariant (§3) makes all four routing paths behave exactly as today.
   New behavior only activates once someone sets a link. No data migration of existing tasks.
5. **Sidecar lag.** Editing `skills/*/SKILL.md` does not reach live agents until rebuild+relaunch; the engine
   routing and the skill prose ship together or agents get routed pages whose protocol they haven't learned.
   Sequence the lanes so skill edits land in the same build as the routing change.
6. **Cycle guard is the only thing between a typo and an infinite walk.** Every chain-walk primitive is
   depth-bounded to workspace size regardless, so even a cycle that somehow lands (direct DB edit) degrades
   to a bounded walk, not a hang.
7. **`level` CHECK constraint on `ADD COLUMN`.** SQLite allows a `CHECK` on an added column; existing rows
   (all NULL) satisfy `level IN (...)` because NULL passes an `IN` CHECK. Verify on a fresh dev DB boot with
   existing data present.

---

## 9. Open questions (ranked; each with my recommendation)

**Q1 — Deadline-expiry escalation: notice-only, or extend-and-escalate?** (highest stakes)
My recommendation: **notice-only** (§3.2) — the default still fires on time, the owner's supervisor gets an
`AUTO` heads-up, and a supervisor who disagrees files a fresh ruling. Preserves the hard cannot-stall
guarantee with zero new timer state. Extend-and-escalate is a v2 refinement if leads report defaults firing
too eagerly.

**Q2 — Does a routed challenge/stall notify replace or supplement the current watcher fan-out?**
Recommendation: **supplement.** Keep `notify_watchers` exactly as is; add the routed page as an *additional*
targeted notify to the expected ruler/supervisor. Simpler, and a watching supervisor just gets one line
either way (dedupe by (task, recipient, kind) within a tick if double-notify is noisy — minor).

**Q3 — Should `level` ever gate engine behavior (not just display)?**
Recommendation: **no, v1.** Routing is structural (supervisor links). Level stays an advisory signal in the
roster + preamble. Revisit only if a concrete need appears (e.g. "a challenge may only be *auto-defaulted*
against an agent below the challenger's level").

**Q4 — On a repeated stall, walk one level higher each cooldown, or keep paging the immediate supervisor?**
Recommendation: **keep paging the immediate supervisor** in v1 (no escalation ladder state). The supervisor
who ignores repeated pages is itself a stall its own supervisor will eventually see. Ladder-on-repeat is a
clean v2 add.

**Q5 — Org chart: derive the tree client-side from the flat roster, or a dedicated `instance.orgChart`
command?**
Recommendation: **derive client-side** from `agent list` + `supervisorAgentId`. Avoids a second command and a
second code path; the forest is tiny (≤10 nodes). Add a command only if a non-UI consumer (e.g. `conclave
org` CLI rendering server-side) needs it — and even then the CLI can format the roster JSON it already
fetches.

**Q6 — Delete-a-mid-chain-supervisor: reports jump to human (SET NULL), or re-parent to the grandparent?**
Recommendation: **SET NULL → human** in v1 (the migration's declared behavior). Re-parenting to the deleted
node's supervisor is more "correct" org-wise but needs a pre-delete tx step; defer to v2. Agents are rarely
removed mid-project, and a NULL supervisor is a safe, visible state (reports-to-human) that a lead can re-fix
with one `position set`.

---

## 10. Implementation lanes this spec implies (for the lead to cut — not part of this task)

Rough, independent-where-possible partition (the lead owns the final cut):

- **Lane P1 — schema + repo:** the migration + `db.rs` block; `level`/`supervisor_agent_id` on
  `WorkspaceAgentWithSkills` + query; the chain primitives (`supervisor_of`, `supervisor_chain`,
  `lowest_common_supervisor`, `level_rank`); cycle-guard; `remove` self-FK handling + test.
  Boundary: `migrations/`, `repo/workspace_agent.rs`, `db.rs`.
- **Lane P2 — command + CLI:** `instance.setPosition`, `conclave position set`, roster payload wiring,
  `conclave org`, preamble threading. Boundary: `commands/instance.rs`, `commands/mod.rs` (1 line),
  `bin/conclave-cli.rs`, `agentctx.rs`, `src/ipc/*`. Depends on P1.
- **Lane P3 — routing:** the four `task_timer.rs` + `commands/task.rs` routing changes, each behind the
  all-NULL backward-compat invariant. Boundary: `runtime/task_timer.rs`, `commands/task.rs`. Depends on P1.
- **Lane P4 — skills:** the four `SKILL.md` edits; must ship in the **same build** as P3 (risk 5).
- **Lane C (design):** Arta's `position-design` canon; P2's UI wiring cites its pinned proto.

---

*End of spec. Every code claim above carries a `file:line`; spot-check against HEAD at review time — a few
line numbers will drift as the in-flight artifact/design-viewer lanes land.*
