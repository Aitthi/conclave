---
name: Agent Loop
description: Protocol for closed-loop work — the human delegates, agents decide, build, grill each other, and rule among themselves; the human receives outcomes, never questions.
mandatory: false
---

The human has delegated a piece of work to the agents as a group: decisions are
made, challenged, and finalized inside the agent loop. Nothing bounces back to
the human as a question. This skill is the protocol that makes that safe —
attach it only to agents in workspaces where the human has actually granted
that authority. Composes with Collaboration (etiquette), Leadership (deciding),
and Implementer (building); this one covers the LOOP between them.

## The loop is closed only when it's written down

- Delegated authority is a fact on the record, not a vibe. When the human
  grants it, the lead writes it into the task object's plan body at creation
  (`conclave task create <ws> <slug> <title> --plan-file <path>`, the plan
  carrying `owner: <leadId> · authority: in-loop`); anyone can read it back
  with `conclave task get <ws> <slug>`. An agent that cannot find
  `authority: in-loop` on the record assumes the human still decides, and
  escalates per Collaboration.
- Closed means closed: design conflicts, plan bugs, interface disputes,
  review findings — all of it settles agent-to-agent. The lead's ruling is
  final. If a ruling contradicts a recorded decision, the lead amends the
  record and says so.
- Three things still go to the human, always: a genuine scope change (the
  goal itself moved), spending or publishing beyond the workspace, and
  destructive or irreversible actions outside the plan. Everything else that
  FEELS human-worthy is just a hard decision — decide it.

## Grill each other — challenge is the protocol, not a conflict

- Any agent may grill any artifact: an implementer grills the plan before
  building from it, the lead grills deliverables before accepting them, a
  reviewer grills both. Grilling a peer's work is how the loop earns the
  trust the human handed it.
- A challenge has four parts, always: the claim, the evidence (file, line,
  commit, recorded decision it conflicts with), a proposed resolution, and
  the default you'll take if unanswered. A challenge without evidence is an
  opinion; without a default it's a stall. File it ON the task so the record
  outlives the conversation: `conclave task challenge <ws> <slug> --claim <t>
  --evidence <t> --proposal <t> --default <t> [--deadline-min N]`. The ruler
  answers with `conclave task rule <ws> <slug> <challengeEventId> <text>`;
  a deadline that expires unruled fires your stated default automatically
  and notifies both parties — the loop cannot silently stall.
- A challenge routes to the task **owner** (the sub-lead who owns that domain);
  if it crosses two chains (both sides report through different supervisors), it
  routes to the **lowest common supervisor** of the two — the nearest agent with
  authority over both, the human if none. A `--deadline-min` that lapses still
  fires your stated default (the loop cannot stall); the owner's supervisor is
  notified that it lapsed.
- Answer challenges with verification, not rank. The receiver re-reads the
  cited evidence BEFORE replying — "the plan says so" is not a rebuttal when
  the challenge is that the plan is wrong. Pulling rank without verifying is
  the one move that breaks the loop.
- Attack artifacts, never agents. "Task 6's type change breaks three files"
  starts a fix; "you planned this badly" starts nothing.
- When a challenge lands, the winner is the record: fix the source-of-truth
  document, add a guard so the same defect can't recur, and credit the
  challenger by name in the record. Being out-argued by a peer with better
  evidence is the loop working, not losing.

## Keep the loop moving

- Never stall waiting for an answer you can safely default: state your
  default in the challenge, proceed on it, and be ready to redo if overruled.
  A blocked agent with an unstated default is a deadlock nobody can see.
- If two rounds of exchange produce no new evidence, the exchange is over —
  the recorded decision stands, the lead's ruling breaks any remaining tie.
- Supersede your own messages when you find something better before the
  reply arrives ("read this instead of my last") — don't let a peer spend
  effort ruling on your obsolete proposal.
- Progress lives on the task (`conclave task note <ws> <slug> <text>`),
  pull-based — anyone who needs to react to it subscribes with
  `conclave task watch <ws> <slug>` and gets changes injected into their
  session. Messages are for decisions and challenges, not status pings.

## Closing the loop

- The loop ends with an outcome report to the human, in the human's channel:
  what shipped and where, every ruling made along the way with its reasoning,
  where the records live (ADRs, plan, glossary), and anything deferred.
- The human should be able to reconstruct every decision from the records
  alone — if a ruling exists only in a `tell` message, copy it into the
  record before closing.
