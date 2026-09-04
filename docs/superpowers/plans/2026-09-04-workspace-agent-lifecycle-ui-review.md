# Workspace and agent lifecycle UI review

owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop

## Goal

Independently review lifecycle UI commit `a64a032d1d3e9a0e8a2990bb97f2fd7fecd6126d` against the implementation plan and the pinned design canon before it is integrated to main.

## Reading order

1. `docs/superpowers/plans/2026-09-04-workspace-agent-lifecycle-ui.md`
2. `design/screens/agent-stop-resume.tsx` at `a1bfd670d5a6b79f794b0d5c9f80a799875371b0`
3. Task brief `workspace-agent-lifecycle-ui-v2`, especially READY note `f0d25d99`
4. Diff from parent to `a64a032d1d3e9a0e8a2990bb97f2fd7fecd6126d`
5. The two recorded PNGs in the implementation worktree

## Review requirements

- Trace workspace Start/Stop and agent Stop/Resume through the actual React state/event paths. Verify stopped workspaces cannot produce hidden runtime spawns or accept runtime input, and that explicit Start launches only active agents.
- Check stale-event and rapid workspace-switch races, partial batch-start failure handling, retry behavior, focus recovery, confirmation behavior, accessibility announcements, and routing exclusion/reset.
- Verify the diff preserves the merged AgentDrafter, provider-label chips, and model-catalogue paths from main `fd126ca311f967b9b6fcaee360c5b33cdaa97f9a`.
- Open and visually inspect both exact PNGs named in READY note `f0d25d99`; do not accept the gate exit codes alone.
- Run `pnpm build`. Rerun targeted UI gates if review changes or doubts the fixture/render behavior.
- Treat any functional, race, accessibility, or visual mismatch as FIX. Report SHIP only with concrete evidence.

## Output

Post one task note with verdict `SHIP` or `FIX`, findings ordered by severity, paths/lines, commands run, and screenshot inspection result. Do not edit product files in this review task; any fix will be assigned back to the implementer under the implementation task.

## File boundary

Read-only review of the implementation task boundary. This task may modify only this plan file for reviewer notes; product fixes are out of scope.
