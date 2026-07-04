---
name: Tool Map
description: One-screen map of which conclave verb to use for what — work items on tasks, worktrees on lanes, ad-hoc facts on the blackboard, knowledge in memory, messages via tell.
mandatory: true
---

Work items ride tasks, never bb keys. The blackboard is for durable facts that
fit no task. Memory is for knowledge that outlives the task itself. When
unsure which applies, `conclave task list <ws>` first — see Collaboration,
Implementer/Leadership, and Memory for the protocol each verb serves.

| When you need to…                       | Run                                             |
|-----------------------------------------|--------------------------------------------------|
| see the board / read a work item        | `conclave task list <ws>` / `task get <ws> <slug>` |
| take a work item                        | `conclave task claim <ws> <slug>`               |
| take it AND get a lane worktree         | `conclave lane start <ws> <slug>`               |
| log progress, decisions, outcomes       | `conclave task note <ws> <slug> <text>`         |
| run a verification with recorded proof  | `conclave task gate <ws> <slug> -- <cmd…>`      |
| dispute a plan/decision, with a default | `conclave task challenge <ws> <slug> --claim --evidence --proposal --default [--deadline-min N]` |
| rule on a challenge (lead)              | `conclave task rule <ws> <slug> <challengeEventId> <text>` |
| move work state / hand back             | `conclave task state <ws> <slug> review\|abandoned` (`merged` = integrator; `task close` = shortcut to merged from any live state) |
| follow a lane you care about            | `conclave task watch <ws> <slug>`               |
| tear down a merged lane (integrator)    | `conclave lane finish <ws> <slug>`              |
| message a peer / see the roster         | `conclave tell <id> <text>` / `conclave agent list <ws>` |
| durable ad-hoc fact (no task fits)      | `conclave bb set/get/list <ws> …`               |
| recall / save cross-session knowledge   | `conclave memory search/remember <ws> …`        |
| context about to be cleared             | `conclave snapshot save <handoff>` then, after, `snapshot last` |
