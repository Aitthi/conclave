# Antigravity UI gap audit

owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop

## Goal

Compare current main plus backend candidate `6030ece3` with the accepted
Antigravity discovery and design canon, then identify the exact remaining changes
needed for a user to create, configure, recognize, and launch an Antigravity CLI
agent from Conclave.

## Reading order

1. `docs/superpowers/plans/2026-09-04-antigravity-cli-discovery.md`
2. Task brief `antigravity-cli-discovery`, READY note `39f84e9b`
3. Task brief `antigravity-ui-canon`
4. `design/screens/antigravity-cli.tsx` at its accepted canon SHA
5. Diff `main..6030ece3fd5fbae51c51ffd3a39a6fed3da23a08`

## Questions to settle

- Which TypeScript unions and IPC payloads still reject or erase
  `cliKind: "antigravity"` and nullable `effort`?
- Which Builder controls/save paths are missing for Antigravity model, effort,
  execution mode, rtk hiding, and defaults?
- Which shared provider-label, Roster, supervisor-picker, LaneBoard, fixture, and
  Skill Assist paths still omit Antigravity?
- Can a newly saved Antigravity definition be added to a workspace and launched
  through the backend candidate without a manual database edit?
- What are the smallest complete file boundary, dependencies, and UI pixel gates
  for an implementation lane?

## Constraints

- Read-only product audit. Do not edit `src/` or `src-tauri/`.
- Ground every finding in current paths/lines; distinguish already-complete work
  from missing work.
- Do not expand into one-shot drafter support, dynamic model catalogues, AGY
  sandbox, rtk hooks, or native AGY conversation resume.

## File boundary

- `docs/superpowers/plans/2026-09-04-antigravity-ui-gap-audit.md`

## Output

Post one READY note on `antigravity-ui-gap-audit` containing the exact remaining
file boundary, ordered implementation steps, dependencies, regression risks, and
verification commands. The note must be sufficient for a zero-context
implementer plan; no product commit is expected.
