---
name: Tool Map
description: One-screen map of which conclave verb to use for what — work items on tasks, worktrees on lanes, ad-hoc facts on the blackboard, knowledge in memory, messages via tell.
mandatory: true
---

Work items ride tasks, never bb keys. Use `conclave task list <ws>` for slim
orientation, `task brief` for a bounded resume packet, and `task get` only for
the full deep record. The blackboard is for durable facts that fit no task;
Memory is for knowledge that outlives the task itself. See Collaboration,
Implementer/Leadership, and Memory for the protocol each verb serves.

| Family | Run | Purpose |
|--------|-----|---------|
| Work items | `conclave task list <workspaceId> [--state s] [--full]` | slim board orientation by default; `--full` includes plan-bearing rows |
| Work items | `conclave task brief <workspaceId> <slug> [--limit N]` | bounded resume packet: metadata, boundary, canon, capped plan excerpt, open challenges, latest gates/events, memory hits |
| Work items | `conclave task get <workspaceId> <slug>` | full deep record: complete plan, boundary, canon, all events, gates, challenges |
| Work items | `conclave task create <workspaceId> <slug> <title...> [--boundary p1,p2] [--canon txt] [--plan-file path]` | lead cuts a new work item |
| Work items | `conclave task claim <workspaceId> <slug>` | take it |
| Work items | `conclave task state <workspaceId> <slug> <state>` | move state (implementers: review\|abandoned; merged = integrator) |
| Work items | `conclave task note <workspaceId> <slug> <text...>` | log progress, decisions, outcomes |
| Work items | `conclave task gate <workspaceId> <slug> -- <cmd...>` | run a verification, proof recorded on the ledger — commit first, then gate — the gate pins `git rev-parse HEAD` at run time; words after `--` pass verbatim (not shell-reparsed); wrap shell syntax in `sh -c "…"` |
| Work items | `conclave uishot [--task <slug>] <args...>` | run the workspace's UI capture script (package.json `uishot`) and SEE the result; `--task` records it as a task gate |
| Work items | `conclave task challenge <workspaceId> <slug> --claim t --evidence t --proposal t --default t [--deadline-min N]` | dispute a plan/decision with a stated default |
| Work items | `conclave task rule <workspaceId> <slug> <challengeEventId> <text...>` | settle a challenge (lead) |
| Work items | `conclave task close <workspaceId> <slug>` | live state → merged shortcut + memory-save reminder |
| Work items | `conclave task watch <workspaceId> <slug>` / `task unwatch <workspaceId> <slug>` | follow / stop following a lane; notes prefixed `READY`/`BLOCKED`/`ESCALATION` wake watchers, while unmarked notes and passing gates are ledger-only |
| Artifacts | `conclave artifact add <workspaceId> --title <t> --kind <k> (--file <path> \| --content <text>)` | persist a significant, self-contained output; kinds `markdown\|code\|html\|svg\|mermaid\|react\|text`; prints the id; your agent id is stamped automatically |
| Artifacts | `conclave artifact list <workspaceId>` | list this workspace's artifacts, newest first (id, kind, title, agent, createdAt — no content dump) |
| Artifacts | `conclave artifact get <id>` | print one artifact's metadata header + full content to stdout |
| Lanes | `conclave lane start <workspaceId> <slug>` | claim + worktree in one step |
| Lanes | `conclave lane finish <workspaceId> <slug>` | integrator teardown after merge (remove worktree + delete branch) |
| Lanes | `conclave lane guard install` | install the shared-checkout commit-scope guard |
| Stage | `conclave stage status <workspaceId> <slug>` | HEAD-vs-worktree tracked and untracked changes, partitioned into in/out of the task's boundary |
| Stage | `conclave stage diff <workspaceId> <slug>` | git diff HEAD, scoped to the boundary |
| Stage | `conclave stage commit <workspaceId> <slug> -m <msg>` | private-index commit of ONLY the boundary paths — the shared `.git/index` is never touched, attribution is native git authorship, ledger note posted automatically |
| Stage | `conclave stage snap <workspaceId> <slug> [-m <label>]` | explicit snapshot of the boundary onto a local op-log ref |
| Stage | `conclave stage log <workspaceId> <slug>` | list snapshots newest-first |
| Stage | `conclave stage restore <workspaceId> <slug> <snapSha>` | restore boundary paths from a snapshot (auto-snaps the current state first, so it's itself undoable) |
| Stage | `conclave stage clear <workspaceId> <slug>` | delete the snapshot ref (only after merged) |
| Stage | (discouraged in the shared checkout) | raw `git add`/`git commit` risk sweeping a peer's staged work (b9ab709) and land under the shared human identity (c3d8fcb) — prefer `stage commit` |
| Peers | `conclave agent list <workspaceId>` | roster: ids, roles, skills, working flag |
| Peers | `conclave tell <agentId> <text...>` | message a peer — the ONLY channel that reaches one |
| Peers | `conclave msg list [--limit N]` | read YOUR own inter-agent inbox+outbox, newest-first (re-read `tell` history after a context clear) |
| Peers | `conclave msg all <workspaceId> [--limit N]` | read the whole workspace's inter-agent traffic, newest-first |
| Peers | `conclave send <sessionId> <text...>` | inject into a session by session id (orchestration plumbing; prefer `tell`) |
| Peers | `conclave run <orchestratorId> <prompt...>` | hand a prompt to an orchestrator agent |
| Workspaces | `conclave ws list` | all workspaces |
| Workspaces | `conclave ws use <workspaceId>` | set the default |
| Blackboard | `conclave bb list <workspaceId>` | list ad-hoc durable facts |
| Blackboard | `conclave bb get <workspaceId> <key>` | read one |
| Blackboard | `conclave bb set <workspaceId> <key> <value>` | write one |
| Blackboard | `conclave bb delete <workspaceId> <key>` (alias `bb rm`) | remove a finished key of your own |
| Blackboard | `config:distill-auto` key | opts a workspace into the timer-driven distiller nudge (`{"distiller","reviewer","cooldownHours"}`); absent = OFF, `bb delete` is the kill switch |
| Memory | `conclave memory search <workspaceId> <query...> [--limit N]` | recall before you research |
| Memory | `conclave memory remember <workspaceId> <text...>` | save hard-won knowledge |
| Memory | `conclave memory delete <workspaceId> <chunkId>` | remove a wrong or stale memory |
| Memory | `conclave memory status <workspaceId>` | store health |
| Memory | `conclave memory propose <workspaceId> <text...> [--source-note NOTE]` | distiller enqueues a candidate memory for review (no embed until approved) |
| Memory | `conclave memory queue <workspaceId> [--state pending\|approved\|rejected]` | list review-queue proposals (default pending, newest first) |
| Memory | `conclave memory approve <workspaceId> <proposalId> [--reason TEXT]` | reviewer (≠ proposer) approves → embeds + stores as a `distilled` chunk |
| Memory | `conclave memory reject <workspaceId> <proposalId> [--reason TEXT]` | reviewer rejects; the row is kept so the fact is not re-proposed |
| Context | `conclave snapshot save <text...>` | persist YOUR handoff before a clear/restart |
| Context | `conclave snapshot last` | re-read it after |
| Context | `conclave snapshot list <sessionId>` / `snapshot read <snapshotId>` | browse saved handoffs |
| Context | `conclave snapshot create <sessionId> <type> [label]` | snapshot another session (orchestration plumbing) |
| Context | `conclave restart` | self-triggered restart — follow its printed save-then-die contract |
| Browser | `conclave browser open <url>` | open/focus the in-app browser window (missing scheme → `https://`); drive a page without Playwright/Puppeteer |
| Browser | `conclave browser goto <url>` | navigate the current browser window |
| Browser | `conclave browser status` | JSON: current URL/title (or `ok:false` when nothing is open) |
| Browser | `conclave browser snapshot [--max-text N]` | JSON DOM/text snapshot — url, title, capped text, headings, links, inputs, buttons — each with a reusable selector |
| Browser | `conclave browser click <selector>` | click an element (selector as emitted by `snapshot`) |
| Browser | `conclave browser type <selector> <text...>` | focus + fill an input-like element |
| Browser | `conclave browser eval <js...>` | escape hatch: run JS in the page, return the JSON result (same-user local tool; never networked) |
| Browser | `conclave browser close` | close the browser window |
| Help | `conclave help` | this list, live — trust it over any cached copy |

## Artifacts — when to save one

Save an artifact when an output is **significant and self-contained**: typically
more than ~15 lines, likely to be edited/iterated/reused outside this
conversation, standing on its own without the surrounding chat, and something
you or the human will refer back to later. Typical kinds: documents
(`markdown`), `code`, single-page `html`, `svg`, diagrams (`mermaid`), and
React components (`react`); use `text` for anything else. Do NOT wrap a throwaway
snippet, a one-line answer, or a value that only makes sense mid-conversation —
those belong in the chat, a task note, or the blackboard. One artifact = one
coherent deliverable; iterate by adding a new version rather than cramming
several outputs into one.
