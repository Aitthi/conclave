# Workspace and agent lifecycle UI

owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop

## Goal

Expose the backend workspace Start/Stop and agent Stop/Resume lifecycle in the real app. Entering a stopped workspace must remain inspectable and launch zero agents until the user explicitly starts it.

## Reading order and design canon

1. `design/screens/agent-stop-resume.tsx` at polished commit `a1bfd670d5a6b79f794b0d5c9f80a799875371b0`
2. Task note `491a2c09` on `agent-stop-ui-canon-v2` remains the behavior checklist; the latest `READY POLISH` note supersedes its visual presentation and SHA.
3. This plan.
4. Backend lifecycle task/commit and its routed command/event types after integration.

Designer/escalation for canon ambiguity: Hardwell (`aee0133c-2b94-4ce7-b39a-01ceb26afeb9`). Product/contract ruler: Aoki (`2004f459-52ad-445c-9c70-e605a0ffdfe3`), final.

## Required behavior

- Thread `Workspace.runState` and `WorkspaceAgent.availability` through IPC/view models.
- Selection of a stopped workspace remains allowed. It lists retained roster/tabs and keeps non-runtime persisted views reachable, but both eager and active-tab fallback spawn paths are skipped.
- The selected runtime pane renders `Workspace stopped` and an explicit `Start workspace` action, never an indefinite `Opening session…` state.
- Start uses a duplicate-safe loading state. It starts only individually active agents, preserves stopped agents, and consumes backend batch successes/skips/failures deterministically.
- Partial start leaves successes live, names failures in a `role=alert`, and offers retry scoped only to failed active agents. It must not restart successes or individually stopped agents.
- Started workspace shows a neutral `Stop workspace` control in the existing workspace header. If any agent is working, confirmation states that current work/runtimes end immediately while durable workspace records remain. Loading prevents duplicate input.
- Workspace event updates from CLI/backend patch/refetch AppShell state. Stop removes live input affordances immediately; Start repopulates sessions. Clear/remount stale spawn guards/session maps when run state changes.
- Agent rows show a persistent text `Stopped` label. Stop/Resume are neutral lifecycle actions separate from destructive Remove. A working agent Stop requires confirmation. Loading and inline error states keep the prior stable lifecycle until success.
- A selected stopped agent renders retained-membership explanation and `Resume agent`; it never auto-spawns. Successful Resume opens a fresh runtime for the same identity; failure stays stopped with `role=alert` retry.
- Keep stopped routing targets visible but disabled with visible and accessible `Stopped`; omit them from keyboard selection. If the current target stops, reset before the next send to self or the first eligible target and announce the change.
- Lifecycle copy is exactly Start/Stop workspace and Stop/Resume agent. Snapshot flow uses `Restore snapshot`, not bare Resume.
- Keyboard: independent focus targets for workspace lifecycle, row, lifecycle action, More actions, dialog and routing option. Escape/cancel returns focus. Errors use assertive/alert semantics; success uses polite live region. Respect reduced motion.
- Remove remains red and separately confirmed. Empty roster remains usable: `No agents in this workspace yet` and Add agent are retained with legible workspace lifecycle controls and no phantom runtime.

## Fixtures

- Extend fixture types/handlers with deterministic workspace and agent lifecycle state and commands/events.
- Default home scenario must show a started workspace, a working active agent, and one individually stopped agent without triggering console errors.
- Empty home scenario must exercise the stopped/empty workspace entry without calling missing runtime handlers.
- If a dedicated stopped scenario is needed for development, add it deterministically, but the standing READY gate remains default + empty as required by the canon.
- Fixed literal timestamps only. A missing handler must still throw; never swallow fixture errors.

## Exact boundary

- `src/ipc/types.ts`
- `src/ipc/commands.ts`
- `src/ipc/events.ts`
- `src/components/AppShell.tsx`
- `src/components/Rail.tsx`
- `src/components/Roster.tsx`
- `src/components/WorkspacePane.tsx`
- `src/components/RoutingPicker.tsx`
- `src/components/StdinBar.tsx`
- `src/components/ChatView.tsx`
- `src/components/ContextBars.tsx`
- `src/fixtures/backend.ts`
- `src/fixtures/scenarios/data.ts`
- `src/fixtures/scenarios/default.ts`
- `src/fixtures/scenarios/empty.ts`
- `src/fixtures/scenarios/stopped.ts`
- `scripts/uishot.mjs`

Do not change backend Rust contract in this lane. If the integrated backend shape conflicts with this plan, file a task challenge before adding adapters or editing outside the boundary.

## Verification and standing UI pixel gate

1. Before screenshots, run `lsof -nP -iTCP:1420 -sTCP:LISTEN`; kill only a confirmed foreign/stale Vite server from another checkout.
2. Run `pnpm build`.
3. Run and record `conclave task gate 11ecf99b-53f4-4c24-b538-b19e5933a9e3 workspace-agent-lifecycle-ui-v2 -- pnpm uishot home`.
4. OPEN and LOOK at `.shots/home-default.png`; verify started control, working active agent, persistent stopped agent, stopped routing treatment, and no clipping.
5. Run and record `conclave task gate 11ecf99b-53f4-4c24-b538-b19e5933a9e3 workspace-agent-lifecycle-ui-v2 -- pnpm uishot home --scenario empty`.
6. OPEN and LOOK at `.shots/home-empty.png`; verify explicit stopped workspace Start UI, unchanged Add agent empty state, no phantom/opening runtime, and no clipping.
7. Run targeted frontend tests if present and `pnpm build` again after any pixel-driven edit.
8. Commit only the boundary with `conclave stage commit ...`; READY note must attach commit SHA, green commands, both PNG paths, and an explicit statement that each PNG was opened and visually inspected.
