---
name: Strategic Compact
description: How to write and restore a context-compaction handoff — the richest possible memory of the session, structured for a reader with zero context, hard-capped at 10k tokens.
mandatory: true
---

When you see a `[conclave compact]` prompt, your context is about to be erased
and the ONLY thing that survives is the handoff you save. The reader is you, five
seconds later, remembering nothing. Write the richest handoff you can — economize
only against the hard cap: **10k tokens ≈ 40,000 characters**. An over-terse
handoff loses hours of context to save cents.

## The handoff — seven sections, this order

If you must trim to fit the cap, cut from the bottom up — the top sections are
the ones whose loss is unrecoverable.

1. **NOW** — the exact next action first: the command to run, the file and line
   to edit, the message to send. Then every half-finished thing: file, line, what
   the edit was going to be, why. This section decides whether you resume or
   restart.
2. **MISSION & AUTHORITY** — the goal in the requester's own words, who delegated
   it, your role, the authority level (e.g. `authority: in-loop`), and your peers:
   id, name, role, who escalates to whom.
3. **DECISIONS & WHY** — every ruling this session with its reasoning and where
   it is recorded (ADR path, plan section, blackboard key). A ruling that exists
   only in chat: copy it in FULL — it has no other survivor.
4. **OPEN THREADS** — challenges awaiting an answer (with your stated default),
   messages you owe or are owed, blackboard keys you watch, reviews pending,
   anything armed that will fire later.
5. **HARD-WON KNOWLEDGE** — approaches that FAILED and why (so you never retry
   them), environment quirks, exact incantations that finally worked, gotchas in
   specific files, load-bearing line numbers, names that look interchangeable but
   aren't.
6. **DONE** — completed work as `commit SHA — one-line what/why`. Compress freely;
   git holds the detail.
7. **POINTERS** — repo paths, plan file, ADRs, glossary, blackboard keys, log
   locations. References, never contents.

## Hard rules

- **Reference, don't paste.** Commit SHAs, `file:line`, blackboard keys survive
  the wipe; your copies of them are wasted budget. No full task lists, raw
  transcript text, or long logs — point to slugs, event ids, gate ids, paths.
- **REDACT secrets** — API keys, tokens, passwords never enter a snapshot.
- **Facts, not narrative.** "Task 6 additive approach approved, plan patched,
  guard added to Global Constraints" beats a paragraph about the discussion.
- Save with ONE command: `conclave snapshot save <your full handoff text>` — run
  it, don't print it; quote the text so the shell passes it whole, then stop and
  wait. Stay under ~40,000 characters — over means the tail you wrote may be the
  part that gets you killed.

## Don't wait for a forced compact

- When your context meter passes ~70%, don't wait for the harness to force it: at
  the next natural boundary (a task landed, a review sent) write the seven-section
  handoff and run `conclave restart` yourself. A handoff written calmly at 70%
  beats one written at 95% under pressure.
- Same for any low-context or auto-compact warning: run `conclave restart`, read
  what it prints, IMMEDIATELY write the handoff, persist it with `conclave
  snapshot save`. The restart only fires after your save lands.

## Restoring — trust, then verify

After a clear you'll run `conclave snapshot last`. Read it, then VERIFY against
reality before acting — the world may have moved while you were gone:

- `git log`: are the SHAs it names still the head? Did new commits land? In a
  shared workspace, peers kept working.
- `conclave task list <ws>`, then `task brief <ws> <slug>` for each lane it names:
  did states move, notes or gates land while you were gone?
- `conclave bb get` the ad-hoc keys it watches; check for messages that arrived
  during the gap.

Then continue from the EXACT next step the handoff names. Never restart work it
marks done; never re-open decisions it records — they are as final as before the
wipe.
