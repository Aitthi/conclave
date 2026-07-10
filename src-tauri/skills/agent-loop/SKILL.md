---
name: Agent Loop
description: Protocol for closed-loop work — the human delegates, agents decide, build, grill each other, and rule among themselves; the human receives outcomes, never questions.
mandatory: false
---

The human delegated work to the agents as a group: decisions are made,
challenged, and finalized inside the loop — nothing bounces back as a question.
Attach this skill only where the human granted that authority. Composes with
Collaboration, Leadership, and Implementer; this covers the LOOP between them.

## The loop is closed only when it's written down

- Delegated authority is a fact on the record, not a vibe. The lead writes it
  into the task's plan body at creation (`conclave task create <ws> <slug>
  <title> --plan-file <path>`, plan carrying `owner: <leadId> · authority:
  in-loop`), readable via `conclave task get`. An agent that cannot find
  `authority: in-loop` assumes the human still decides, and escalates per
  Collaboration.
- Closed means closed: design conflicts, plan bugs, interface disputes, review
  findings all settle agent-to-agent, and the lead's ruling is final. A ruling
  that contradicts a recorded decision means the lead amends the record and says
  so.
- Three things still go to the human, always: a genuine scope change (the goal
  itself moved), spending or publishing beyond the workspace, and destructive or
  irreversible actions outside the plan. Everything else that FEELS human-worthy
  is just a hard decision — decide it.

## Council decisions stay on the task

- In a council (several Leads planning on one task), every MATERIAL position
  is filed as a `task challenge` and every settled outcome is a `task rule`
  PLUS an amendment to the canonical repo plan named by the task's execution
  header. A position that lives only in `tell` messages settles nothing —
  tells announce where to pull state, they never decide.
- The two-round stop binds every member: after two rounds without new
  evidence, the chair — the task owner — rules, and the ruling is final.
- Never copy the debate transcript into downstream plans or implementer
  messages: an implementer gets the slug and `task brief`, and every decision
  stays reconstructable from the challenge and ruling events plus the amended
  plan.
- Convening, the header contract, and handoff live in Leadership; the full
  protocol is `docs/superpowers/specs/2026-07-10-lead-council-v1-design.md`.

## Grill each other — challenge is the protocol, not a conflict

- Any agent may grill any artifact: an implementer grills the plan, the lead
  grills deliverables, a reviewer grills both.
- A challenge has four parts, always: the claim, the evidence (file, line,
  commit, the recorded decision it conflicts with), a proposed resolution, and
  the default you'll take if unanswered. No evidence = an opinion; no default = a
  stall. File it ON the task: `conclave task challenge <ws> <slug> --claim <t>
  --evidence <t> --proposal <t> --default <t> [--deadline-min N]`. The ruler
  answers with `conclave task rule <ws> <slug> <challengeEventId> <text>`; a
  deadline that expires unruled fires your stated default and notifies both
  parties.
- A challenge routes to the task **owner**; if it crosses two chains (both sides
  report through different supervisors), to the **lowest common supervisor** (the
  human if none). A lapsed `--deadline-min` fires your default and notifies the
  owner's supervisor.
- Answer with verification, not rank: the receiver re-reads the cited evidence
  BEFORE replying — "the plan says so" is no rebuttal when the challenge is that
  the plan is wrong. Pulling rank without verifying is the one move that breaks
  the loop.
- Attack artifacts, never agents. "Task 6's type change breaks three files"
  starts a fix; "you planned this badly" starts nothing.
- When a challenge lands, the winner is the record: fix the source-of-truth
  document, add a guard so the defect can't recur, and credit the challenger by
  name.

## Keep the loop moving

- Never stall waiting for an answer you can safely default: state your default in
  the challenge, proceed on it, be ready to redo if overruled.
- If two rounds produce no new evidence, the exchange is over — the recorded
  decision stands, the lead's ruling breaks any remaining tie.
- Supersede your own messages when you find something better before the reply
  arrives ("read this instead of my last").
- Progress lives on the task (`conclave task note <ws> <slug> <text>`),
  pull-based — whoever needs to react subscribes with `conclave task watch`.
  Messages are for decisions and challenges, not status pings.

## Closing the loop

- The loop ends with an outcome report to the human, in the human's channel: what
  shipped and where, every ruling with its reasoning, where the records live
  (ADRs, plan, glossary), and anything deferred.
- The human should be able to reconstruct every decision from the records alone —
  a ruling that exists only in a `tell` gets copied into the record before
  closing.
