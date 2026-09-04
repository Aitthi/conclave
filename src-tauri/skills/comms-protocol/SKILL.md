---
name: Comms Protocol
description: Use whenever you are about to send a message to another agent or to the human in a Conclave workspace — a question, a request, a status update, a result, an escalation — or when a message you received looks cut off. Also use when the human has asked for less chatter or for all coordination to go through the lead.
mandatory: true
---

The human owns the workspace and reads only what the lead sends. Messages are
the most expensive and least reliable channel: every one lands in another
agent's context, long ones can arrive with their head cut off, and a message
dies at the receiver's next context clear. Records survive; chat does not.
Composes with Collaboration (etiquette) and Leadership (who rules).

## Where each kind of content lives

| Content | Lives in | Never only in |
|---|---|---|
| A request, a decision, a default, a number, a gate phrase | `conclave task note`, `task challenge`, `task rule` on the task it concerns | a `tell` |
| A standing rule (protocol, language, format) | blackboard key + the skill/role layer the agent re-reads | a `tell` |
| A durable fact that outlives the task | `conclave memory remember` | a note |
| A pointer to any of the above | the `tell` | — |

A `tell` announces; it never decides and never carries the only copy of anything.

## Every message goes through the lead

- An implementer sends messages to ONE address: the lead. Anything needed from a
  peer (a measurement, a file, a confirmation, a slot on a machine) is a task
  note or a short tell to the lead, who decides and tells whoever must act.
- No peer-to-peer tells, no side agreements between implementers, no "I asked
  Guetta directly because it was faster". Two implementers never negotiate an
  interface, a schedule, or a shared file between themselves.
- The lead is the only agent who writes to the human. Implementers report at
  boundaries on the task; the lead summarises.

## What a message IS

A message is sent only when one of these is true: a decision is needed, a gate
opened, a result is ready, a blocker appeared. It has exactly this shape:

1. First sentence: the ask or the fact, complete on its own ("Claim X", "x2
   pair done, note 81e09d08", "RULED: ...").
2. Then at most a few lines of pointers: task slug, note or event id, file
   path, commit SHA, blackboard key.
3. Under about 600 characters. If it needs more, the body is a task note and the
   message is its pointer.

Not a message: an acknowledgement, a thank-you, a status ping, a restatement of
a note, a second message that adds no new fact. Progress is a task note read by
whoever watches the task. Automatic stall alerts are not messages either: the
lead verifies on the machine before acting on one.

## Reading a message you received

- If it begins mid-sentence or mid-word, its head is missing. Do not infer the
  ask: read the sender's task notes (`conclave task brief <ws> <slug>`) and act
  on the record, not on the fragment.
- When you act on a tell, quote its first words back ("acting on: 'Claim X…'")
  so the sender can see whether the head arrived.
- A request that reaches you only as a tell and not as a note: act on it, then
  file it as a note on the task yourself so the record is complete.

## Gates between agents

Work that waits on another agent's work waits on an EXACT PHRASE the plan names
("x2 pair done", "run done", "remote idle"), posted as a task note by the
implementer and received through `conclave task watch`. The watcher gets the
note itself: nobody relays it, nobody polls, nobody asks "is it done yet", and
a gate phrase is never sent only as a tell.

## Reporting to the human

The lead sends one summary per boundary in the human's language: what the
result is, what was decided and why, what the human must decide, where the
records are. No running commentary. If the human asks for less, less.

## Red flags — stop before sending

- "Quick question for <peer>" — route it through the lead.
- "Just confirming…" / "Got it" — do not send.
- A tell longer than the note it points at.
- Acting on a tell whose first line does not read as a first line.
- A gate phrase typed into a tell instead of a task note.
