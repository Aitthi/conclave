---
name: Strategic Compact
description: How to write and restore a context-compaction handoff — the richest possible memory of the session, structured for a reader with zero context, hard-capped at 10k tokens.
mandatory: true
---

When you see a `[conclave compact]` prompt, your context is about to be erased
and the ONLY thing that survives is the handoff you save. The reader is you,
five seconds later, remembering nothing. Write the richest handoff you can —
do NOT economize tokens to look tidy; economize only against the hard cap:
**10k tokens ≈ 40,000 characters**. An over-terse handoff loses hours of
context to save cents.

## The handoff — seven sections, this order

Priority order matters: if you must trim to fit the cap, cut from the bottom
up — the top sections are the ones whose loss is unrecoverable.

1. **NOW** — the exact next action first: the command to run, the file and
   line to edit, the message to send. Then every half-finished thing: file,
   line, what the edit was going to be, why. This is the section that decides
   whether you resume or restart.
2. **MISSION & AUTHORITY** — the goal in the requester's own words, who
   delegated it, your role, the authority level (e.g. `authority: in-loop`),
   and your peers: id, name, role, who escalates to whom.
3. **DECISIONS & WHY** — every ruling made this session, each with its
   reasoning and where it is recorded (ADR path, plan section, blackboard
   key). A ruling that exists only in chat: copy it in FULL here — it has no
   other survivor.
4. **OPEN THREADS** — challenges awaiting an answer (with your stated
   default), messages you owe or are owed, blackboard keys you watch,
   reviews pending, anything armed that will fire later.
5. **HARD-WON KNOWLEDGE** — the expensive lessons: approaches that FAILED
   and why (so you never retry them), environment quirks, exact command
   incantations that finally worked, gotchas in specific files, load-bearing
   line numbers, names that look interchangeable but aren't.
6. **DONE** — completed work as `commit SHA — one-line what/why`, landed
   reviews, closed threads. Compress freely; git holds the detail.
7. **POINTERS** — repo paths, the plan file, ADRs, glossary, blackboard keys,
   log locations. References, never contents.

## Hard rules

- **Reference, don't paste.** Commit SHAs, `file:line`, blackboard keys — the
  artifacts survive the wipe; your copies of them are wasted budget.
- **REDACT secrets** — API keys, tokens, passwords never enter a snapshot.
- **Facts, not narrative.** "Task 6 additive approach approved, plan patched,
  guard added to Global Constraints" beats a paragraph about the discussion.
- Save with ONE command, exactly as instructed:
  `conclave snapshot save <your full handoff text>` — run it, don't print it.
  Quote the text so the shell passes it whole; then stop and wait.
- Estimate the cap by length: stay under ~40,000 characters. Well under is
  fine; over means the tail you wrote may be the part that gets you killed.

## Self-triggered restart

When your harness warns that context is nearly full (e.g. an auto-compact
notice or a low-context warning), don't wait for a human to notice: run
`conclave restart` yourself, read what it prints, then IMMEDIATELY write the
seven-section handoff above and persist it with `conclave snapshot save`. The
restart only fires after your save lands — stalling after the warning just
leaves you degraded until you do.

## Restoring — trust, then verify

After a clear you'll be told to run `conclave snapshot last`. Read it, then
VERIFY it against reality before acting — the world may have moved while you
were gone:

- `git log` the repo: are the SHAs it names still the head? Did new commits
  land? In a shared workspace, peers kept working.
- `conclave bb get` the keys it watches: progress, claims, plan — did anyone
  update them?
- Check for messages that arrived during the gap.

Then continue from the EXACT next step the handoff names. Never restart work
the handoff marks done; never re-open decisions it records — they are as
final as they were before the wipe.
