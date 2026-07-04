---
name: Tool Map
description: One-screen map of which conclave verb to use for what — work items on tasks, worktrees on lanes, ad-hoc facts on the blackboard, knowledge in memory, messages via tell.
mandatory: true
---

Work items ride tasks, never bb keys. The blackboard is for durable facts that
fit no task. Memory is for knowledge that outlives the task itself. When
unsure which applies, `conclave task list <ws>` first — see Collaboration,
Implementer/Leadership, and Memory for the protocol each verb serves.

| Family | Run | Purpose |
|--------|-----|---------|
| Work items | `conclave task list <workspaceId> [--state s]` | see the board (optional state filter) |
| Work items | `conclave task get <workspaceId> <slug>` | read plan, boundary, canon, events, gates |
| Work items | `conclave task create <workspaceId> <slug> <title...> [--boundary p1,p2] [--canon txt] [--plan-file path]` | lead cuts a new work item |
| Work items | `conclave task claim <workspaceId> <slug>` | take it |
| Work items | `conclave task state <workspaceId> <slug> <state>` | move state (implementers: review\|abandoned; merged = integrator) |
| Work items | `conclave task note <workspaceId> <slug> <text...>` | log progress, decisions, outcomes |
| Work items | `conclave task gate <workspaceId> <slug> -- <cmd...>` | run a verification, proof recorded on the ledger |
| Work items | `conclave task challenge <workspaceId> <slug> --claim t --evidence t --proposal t --default t [--deadline-min N]` | dispute a plan/decision with a stated default |
| Work items | `conclave task rule <workspaceId> <slug> <challengeEventId> <text...>` | settle a challenge (lead) |
| Work items | `conclave task close <workspaceId> <slug>` | live state → merged shortcut + memory-save reminder |
| Work items | `conclave task watch <workspaceId> <slug>` / `task unwatch <workspaceId> <slug>` | follow / stop following a lane's notifications |
| Lanes | `conclave lane start <workspaceId> <slug>` | claim + worktree in one step |
| Lanes | `conclave lane finish <workspaceId> <slug>` | integrator teardown after merge (remove worktree + delete branch) |
| Lanes | `conclave lane guard install` | install the shared-checkout commit-scope guard |
| Peers | `conclave agent list <workspaceId>` | roster: ids, roles, skills, working flag |
| Peers | `conclave tell <agentId> <text...>` | message a peer — the ONLY channel that reaches one |
| Peers | `conclave send <sessionId> <text...>` | inject into a session by session id (orchestration plumbing; prefer `tell`) |
| Peers | `conclave run <orchestratorId> <prompt...>` | hand a prompt to an orchestrator agent |
| Workspaces | `conclave ws list` | all workspaces |
| Workspaces | `conclave ws use <workspaceId>` | set the default |
| Blackboard | `conclave bb list <workspaceId>` | list ad-hoc durable facts |
| Blackboard | `conclave bb get <workspaceId> <key>` | read one |
| Blackboard | `conclave bb set <workspaceId> <key> <value>` | write one |
| Blackboard | `conclave bb delete <workspaceId> <key>` (alias `bb rm`) | remove a finished key of your own |
| Memory | `conclave memory search <workspaceId> <query...> [--limit N]` | recall before you research |
| Memory | `conclave memory remember <workspaceId> <text...>` | save hard-won knowledge |
| Memory | `conclave memory delete <workspaceId> <chunkId>` | remove a wrong or stale memory |
| Memory | `conclave memory status <workspaceId>` | store health |
| Context | `conclave snapshot save <text...>` | persist YOUR handoff before a clear/restart |
| Context | `conclave snapshot last` | re-read it after |
| Context | `conclave snapshot list <sessionId>` / `snapshot read <snapshotId>` | browse saved handoffs |
| Context | `conclave snapshot create <sessionId> <type> [label]` | snapshot another session (orchestration plumbing) |
| Context | `conclave restart` | self-triggered restart — follow its printed save-then-die contract |
| Help | `conclave help` | this list, live — trust it over any cached copy |
