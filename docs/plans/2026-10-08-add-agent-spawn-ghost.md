# Agent added after Start never spawns: the stale start batch gate
owner: 30fa04f4-e047-4241-a9ed-f452529952be · authority: in-loop · coordinator: Aitthi (7d560bae-09dc-4535-befb-feed5a95aa05) · escalation: challenges route to the task owner (Detoro); READY / BLOCKED / stall / review verdicts / dispatch are the Coordinator's.
Tier: complex (state-flow fix across two components, no existing frontend test harness). Role: implementer-complex. Reviewer: Mellow (b3a30e7b-5a9f-4d4d-a83e-768a9326632f). Detoro rules and merges on `MERGE-READY add-agent-spawn-ghost @<sha>`. Start the lane from main at or after 082c01f.

## Bug (human report 2026-10-08)
"Add a new agent to the workspace → it does not start, but the state says started; you have to stop and start again."

## Root cause (traced 2026-10-08, read-only; verified by Detoro against the code)
- Adding an agent (`Roster.tsx:1262` `handleConfirmAdd` → `agentDef.addToWorkspace`, or `applyTeamDraft.ts:195`) only inserts the `workspace_agent` + `session` rows with status `idle` (`agent.rs:461-511`, `repo/workspace_agent.rs:723+`). It never spawns. The UI then bumps `agentsVersion` (`AppShell.tsx:966`, `:1200`), `WorkspacePane` remounts (keyed `${activeWorkspaceId}:${agentsVersion}`, `AppShell.tsx:1086`) and refetches `instance.list` — the roster is fresh.
- Spawning is the pane's job: two effects in `src/components/WorkspacePane.tsx` (eager, L482-492; active-tab fallback, L495-506). BOTH skip any instance id that is absent from `workspaceStartBatch.readyAgentIds`.
- `workspaceStartBatch` is set ONCE from the `workspace.start` result (`AppShell.tsx:624-628`, computed at start time by `workspace.rs:191-212`) and is only cleared on workspace switch/close/start (`AppShell.tsx:287`, `516`, `585`, `612`). `onAgentsChanged` does not touch it; only `handleAgentLifecycleChanged` (`AppShell.tsx:748-760`) adds ids.
- So: Start the workspace, then add an agent in the same session → the new id is in none of the batch lists → both effects `continue`/`return` → no `instance.spawn` → DB row stays `idle`, pane shows "Opening session…" forever. The "started" the human sees is the WORKSPACE `runState`. Switching workspaces first (batch → null) makes it work, which is why it looks intermittent; Stop + Start recomputes the batch, which is the workaround the human found.
- Engine side is sound: `instance.spawn` marks `running` only after the PTY is registered (`instance.rs:1270-1421`), EOF flips it back to `idle` (`:1660`, `:1753`), and stop/remove/workspace.stop really kill the child (`teardown_under_lifecycle_lock` → `runtime.unregister` → `pty.rs:201-204` `child.kill()`). No engine change in this task.
- Non-finding (recorded so nobody chases it): the "two claude processes per agent" Detoro saw in `ps` are the OmniChat and codeup teams, two workspaces — not a leak. Evidence: `~/Library/Application Support/Conclave/evidence/2026-10-08-dup-claude-procs.txt`.

## Rulings (final)
R1. The batch gate's purpose is to not auto-retry what the Start batch already decided about (a failed launch, a stopped agent). It must say nothing about an agent the batch never saw. New pure module `src/lib/startBatchGate.ts`:
```ts
import type { WorkspaceStartBatch } from "../components/WorkspacePane";
/** True when the Start batch already ruled on this instance and it is NOT ready:
 *  it failed to launch or was skipped as stopped. An id the batch never saw
 *  (added after Start) is never blocked — the pane spawns it like any other
 *  active tab. */
export function blockedByStartBatch(batch: WorkspaceStartBatch | null, instanceId: string): boolean {
  if (!batch) return false;
  if (batch.readyAgentIds.includes(instanceId)) return false;
  return (
    batch.skippedStoppedAgentIds.includes(instanceId) ||
    batch.failures.some((f) => f.workspaceAgentId === instanceId)
  );
}
```
If importing the type from the component creates a cycle `tsc` complains about, move `WorkspaceStartBatch` into `src/lib/startBatchGate.ts` and re-export it from `WorkspacePane.tsx` under the same name (AppShell imports it from there, `AppShell.tsx:15`).
R2. `WorkspacePane.tsx`: both effects replace their `readyAgentIds` checks with `if (blockedByStartBatch(workspaceStartBatch, id)) continue/return;`. Keep the dependency arrays; delete the now-unused `readyIds` set. Update the comment on the fallback effect (L494) to say it is the path a tab added after Start takes.
R3. Do NOT also mutate `readyAgentIds` from `onAgentsChanged` — one mechanism, not two. Rejected alternative, recorded: appending added ids to the batch in AppShell would work but leaves the pane's gate semantically wrong for any other future caller of `setAgentsVersion`.
R4. Regression test, Node's built-in runner (no new dev dependency; Node v22.23.1 strips types natively): `src/lib/startBatchGate.test.ts` with `import { test } from "node:test"; import assert from "node:assert/strict"; import { blockedByStartBatch } from "./startBatchGate.ts";` covering: null batch → false; ready id → false; failed id → true; skipped-stopped id → true; UNKNOWN id (the bug) → false. If `tsconfig.json` `include: ["src"]` pulls the test file into `tsc --noEmit` and the `node:test` types are missing, add `/// <reference types="node" />` or a minimal `declare module` in the test file — do not add `@types/node` to package.json without a challenge. The run command is `node --experimental-strip-types --test src/lib/startBatchGate.test.ts` (if that flag is rejected on this Node, use `node --test` — v22.23 enables stripping by default; say which in the READY note).
R5. A second, cheaper tripwire for the human's exact scenario: nothing in the pane tells the human WHY a tab is stuck at "Opening session…". Out of scope here — note it as a follow-up in the READY note only if you observe it while testing; do not build it.

## Files (boundary)
- `src/lib/startBatchGate.ts` (new, R1)
- `src/lib/startBatchGate.test.ts` (new, R4)
- `src/components/WorkspacePane.tsx` (R2)
Nothing else. `AppShell.tsx` stays untouched unless R1's cycle clause applies — then only the type import line moves, and the READY note says so.

## Task table
| slug | tier | role | deps | acceptance |
|---|---|---|---|---|
| add-agent-spawn-ghost | complex | implementer-complex | — | R1-R4; gates below green; READY with SHA, test names, and which node flag ran |

## Steps
1. `conclave lane start 11ecf99b-53f4-4c24-b538-b19e5933a9e3 add-agent-spawn-ghost`; `pnpm install` once in the worktree.
2. Read this plan, then `WorkspacePane.tsx:478-506` and `AppShell.tsx:605-632`, `:748-760`. Apply R1-R4.
3. Gates (record each with `conclave task gate 11ecf99b-53f4-4c24-b538-b19e5933a9e3 add-agent-spawn-ghost -- <cmd>`; words after `--` unquoted):
   - `node --experimental-strip-types --test src/lib/startBatchGate.test.ts`
   - `pnpm exec tsc --noEmit`
   - `pnpm uishot home` and `pnpm uishot home --scenario workspace-started` (if `workspace-started` is not a valid `--scenario` value, say so and run `default` + `empty`); OPEN each PNG — the workspace pane must render as before; attach paths.
   Always `lsof -nP -iTCP:1420 -sTCP:LISTEN` first and kill a foreign vite server (CLAUDE.md caveat).
4. One commit on `lane/add-agent-spawn-ghost` scoped to the boundary.

## Risk ledger
- Line numbers from main 082c01f; anchor on the quoted text.
- `failures[].workspaceAgentId` is the field name (`src/ipc/commands.ts:75-78`, `WorkspacePane.tsx:43`) — not `instanceId`.
- The eager effect's dependency array includes `tabs`, which changes on every `instance.list` refresh — the `spawnInstance` ref guard (`WorkspacePane.tsx` "Idempotent via the ref guard") is what keeps it from double-spawning; do not remove it.
- Live verification of the human's scenario (Start → add agent → tab opens a session without Stop/Start) needs the rebuilt app; `pnpm tauri dev` cannot take `conclave.sock` (memory). The human does that check after rebuild + relaunch; the lane's evidence is R4 + tsc + shots.

## Done means
READY note on the task (shape: `task:` / `status:` / `files:` / `note:` with SHA, gate ids, test names, node flag); Mellow reviews; Aitthi posts `MERGE-READY add-agent-spawn-ghost @<sha>`; Detoro merges, then reports to the human that the fix needs rebuild + relaunch and the manual check above.
