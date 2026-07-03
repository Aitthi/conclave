---
status: accepted
---

# Role system: roles are first-class bundles, and the roster tells agents who is who

Today `agent_definition.role` is free text whose only effect is one clause in the bootstrap
preamble, and `conclave agent list` returns bare ids + skill ids — a peer cannot tell who
anyone IS, what they're FOR, or what they can do. Both halves get fixed together.

- **A role is a first-class entity, not a label.** It carries: display name, a one-paragraph
  responsibility description (used verbatim in the preamble and the roster), and a default
  skill set. The rejected alternative — keeping free text and letting users hand-assemble
  role + skills every time — is exactly the current state that produced skill-less "leads".
- **Choosing a role at agent creation COPIES its default skills onto the agent** (editable
  afterwards like any attachment). It is a starting bundle, not a live link: editing a role's
  defaults later does NOT retro-change existing agents. The live-link alternative was rejected
  because a role edit would silently mutate running agents' skill sets — surprising, and it
  couples two lifecycles that users reason about separately.
- **Builtin roles ship as a bundled folder, custom roles live in the DB** — mirroring ADR 0002's
  builtin-skills pattern (`roles/<id>/ROLE.md`, frontmatter: `name`, `description`, `skills:`
  comma-separated builtin/custom skill ids). Five builtins: `lead` (leadership, agent-loop),
  `implementer` (implementer), `reviewer` (implementer), `researcher`, `designer`
  (arta-designer) — all on top of the mandatory skills (collaboration, strategic-compact) that
  attach to every agent regardless.
- **`agent_definition` gains `role_id`** (builtin slug or custom row id). The legacy free-text
  `role` column stays as a display fallback for existing rows; new saves write both (`role` =
  the role's display name) so nothing downstream breaks mid-migration.
- **The roster becomes self-describing.** `agent.list` (and therefore `conclave agent list`)
  returns per agent: display name, role name, role description, and skill NAMES (not just ids).
  The preamble keeps pointing at `conclave agent list` for the live roster (ADR 0001's
  static-preamble rule); it additionally bakes the agent's OWN role description so an agent
  knows its job even before its first roster query.
- **Roles are descriptive, not enforced.** Authority still lives in blackboard records
  (`plan:<task>` owner, `authority:` grants). A v1 that hard-enforces "only leads may X" was
  rejected: enforcement needs an authorization model the engine doesn't have yet, and the
  skills themselves already encode the behavioral contract.
