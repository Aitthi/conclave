---
name: Memory
description: The workspace's long-term memory — recall before you research, save hard-won knowledge when you close a task, and keep the store clean enough that search stays trustworthy.
mandatory: true
---

The workspace has a persistent, semantically searchable memory that survives
context clears, restarts, and agent teardown. Nothing is captured
automatically — a fact you never save is a fact the workspace forgets.
Strategic Compact saves YOUR next context; memory saves what every future
agent, on any session, gets to know.

## Recall before you research

- At the start of a task — and before deep-diving any question a past
  session may have solved — read the task's bounded packet first:
  `conclave task brief <workspaceId> <slug> [--limit N]`. It already carries
  relevant memory hits when available. For questions wider than one work item,
  search directly: `conclave memory search <workspaceId> <query...> [--limit N]`.
  Re-deriving a fact that was already paid for (a failed approach, an
  environment quirk, a command incantation) is the most expensive way to
  agree with a dead agent.
- Treat recalled facts as leads, not gospel: they were true when written.
  If one names a file, flag, or command, verify it still exists before
  building on it.

## Save when you close, not while you churn

- The moment to save is a BOUNDARY: a task lands, a bug's root cause is
  confirmed, a review closes. Ask: "what did this cost me to learn that the
  repo does not already record?" — then
  `conclave memory remember <workspaceId> <text...>`.
- Worth saving: approaches that FAILED and why, environment/tooling quirks,
  exact incantations that finally worked, decision REASONING that would
  otherwise live only in chat, cross-session gotchas in specific files.
- One memory = one self-contained fact, written for a stranger: name the
  files, commands, and versions in the text itself — "the fix discussed
  above" is unsearchable and means nothing next week.

## What never goes in

- Secrets: API keys, tokens, passwords — never, in any form.
- What the workspace already records: code structure, git history, plan
  files, ADRs, blackboard state. Memory duplicating the repo goes stale the
  day the repo moves; save the POINTER only if finding it was the hard part.
- Chat transcripts, raw agent transcripts, status updates, full task-list
  dumps, long logs, and in-flight state — that churn belongs to source files,
  task events, gate ids, or transcript readers, not memory.

## Memory is not the coordination layer

- Tasks (`conclave task …`) = LIVE work state: claims, plans, notes, gates,
  challenges — structured, evented, read by peers coordinating NOW. The
  blackboard holds the ad-hoc facts beside them (conventions, anomalies,
  notes).
- `conclave task list` is slim orientation, `conclave task brief` is the
  bounded resume packet, and `conclave task get` is the full deep read. Do not
  paste any of those full records into chat or memory; reference slugs, paths,
  event ids, and gate ids.
- Memory = DURABLE knowledge: unkeyed facts any future session finds by
  MEANING (semantic search), long after the task is closed and its bb notes
  are purged.
- A settled ruling can live in both: the task event coordinates this week's
  work; the memory preserves the reasoning for next month's stranger.
  `conclave task close` reminds you to make exactly this save.

## Keep the store trustworthy

- Search quality is a commons: every junk entry makes every future search
  worse. When in doubt, don't save — a missing memory costs one re-derive;
  a wrong one misleads every agent that finds it.
- Found a memory that is wrong or obsolete? Delete it on the spot:
  `conclave memory delete <workspaceId> <chunkId>` (ids come back with
  search results). Correct-then-save beats leaving both versions to fight
  in the rankings.
- `conclave memory status <workspaceId>` shows chunk count and model
  readiness when you need to sanity-check the system itself.
