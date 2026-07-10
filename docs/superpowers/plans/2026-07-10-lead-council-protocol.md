# Lead Council V1 Protocol Lane
<!-- conclave-plan:v1
{
"owner":"c5ab26f1-8119-4601-8f84-21094d1f9914","authority":"in-loop","council":{"chair":"c5ab26f1-8119-4601-8f84-21094d1f9914","members":["4fb2198c-e0d9-4e4b-af9e-d4e72542bace","d63832da-f4bb-4859-b9a0-4904be11ca8e"],"maxRounds":2},
"planPath":"docs/superpowers/plans/2026-07-10-lead-council-protocol.md","baseSha":"e8ce7bad254f6abbd2ac782a9d62b717701b759c","escalation":"c5ab26f1-8119-4601-8f84-21094d1f9914",
"readingOrder":["docs/superpowers/specs/2026-07-10-lead-council-v1-design.md","docs/superpowers/plans/2026-07-10-lead-council-protocol.md","docs/adr/0008-agent-work-system.md","src-tauri/skills/leadership/SKILL.md#Settle decisions before anyone builds","src-tauri/skills/agent-loop/SKILL.md#The loop is closed only when it's written down"],
"boundary":["src-tauri/skills/agent-loop/SKILL.md","src-tauri/skills/leadership/SKILL.md"],
"consumes":["src-tauri/skills/leadership/SKILL.md#Plan for a stranger","src-tauri/skills/leadership/SKILL.md#Delegate and stay out","src-tauri/skills/agent-loop/SKILL.md#Keep the loop moving"],
"produces":["src-tauri/skills/leadership/SKILL.md#Convene a Lead council","src-tauri/skills/agent-loop/SKILL.md#Council decisions stay on the task"],"gates":["git diff --check","rg -q '^## Convene a Lead council$' src-tauri/skills/leadership/SKILL.md","rg -q '^## Council decisions stay on the task$' src-tauri/skills/agent-loop/SKILL.md","rg -q 'conclave-plan:v1' src-tauri/skills/leadership/SKILL.md src-tauri/skills/agent-loop/SKILL.md","cd src-tauri && cargo test agentctx"]
} -->

## Goal

Encode the settled Lead council protocol in the two sidecars that every chair and council member re-read after a context clear. A fresh Lead must be able to convene three or more agents, deliberate on one task ledger, stop the exchange, validate the plan, and hand downstream slugs to implementers without copying council history.

## Non-goals

- Do not edit Rust, TypeScript, SQL, UI fixtures, or CLI help in this lane.
- Do not add a council table, group chat, voting protocol, task co-owner, or orchestrator.
- Do not duplicate the full design spec inside either skill.
- Do not change ordinary one-owner delegation semantics.

## Decisions

- The task owner is the single chair and final ruler.
- A council has the chair plus at least two watched members.
- Material positions use challenge/rule events. `tell` only announces where to pull state.
- Two rounds without new evidence end with a chair ruling.
- The canonical mutable plan is the repo file named by the immutable ten-line task header.
- The implementer gets the task slug and bounded `task brief`, never the council transcript.
- Council-tagged claims warn on a missing or stale plan fingerprint in v1; they do not block.

## Ordered edits

1. In `src-tauri/skills/leadership/SKILL.md`, add a compact `Convene a Lead council` section adjacent to the existing decision/delegation guidance.
2. State the exact sequence: create with council header and watchers, members pull `task brief`, bounded evidence, formal challenges, chair rulings, two-round stop, canonical plan update, plan-check gate, zero open challenges, downstream tasks, slug-only handoff.
3. State that the execution header must occupy lines 1 through 10 and that its `planPath`, `readingOrder`, `boundary`, anchors, and gates are the cold-start contract.
4. State that a council task retains one owner; role rotation happens between tasks, not inside one task.
5. In `src-tauri/skills/agent-loop/SKILL.md`, add `Council decisions stay on the task` beside the existing loop-record rules.
6. Require every material position to use a challenge, every settled outcome to use a ruling plus a repo-plan amendment, and all members to stop after two no-new-evidence rounds.
7. Prohibit copying the debate transcript into downstream plans or implementer messages.
8. Point both concise sections to `docs/superpowers/specs/2026-07-10-lead-council-v1-design.md` for the full contract.
9. Remove or tighten any newly duplicated wording so the sidecars remain bounded.

## Verification

Run `git diff --check`. It must print nothing and exit 0.

Run each of the header's three `rg -q` commands as a separate recorded gate. Each must exit 0: the Leadership heading in Leadership, the Agent Loop heading in Agent Loop, and the contract version reference in at least one edited skill. One match cannot make the other required checks pass.

Run `cd src-tauri && cargo test agentctx`. Existing briefing tests must pass even though they do not validate the skill prose itself.

Read the final two sections as a cold Lead and confirm they name the exact commands and stop conditions without requiring this conversation.

## Risks

- Adding long prose to every Lead context would spend the context this feature saves. Keep each new section operational and link the full spec.
- Duplicating chair rules between Leadership and Agent Loop can drift. Leadership owns convening and handoff; Agent Loop owns evidence, rulings, and stopping.
- Referring to commands before the backend lane lands is temporarily aspirational. The task remains in review until tooling integration completes.

## Rejected alternatives

- A standalone Council skill would require every workspace to opt into a new sidecar and could be absent exactly when a Lead needs the protocol.
- Putting the protocol only in docs would not survive fresh agent launch or context clear.
- Copying the full schema into both skills would inflate every Lead prompt and create two editable contracts.

## Escalation

Design or protocol conflicts go to Aoki (`c5ab26f1-8119-4601-8f84-21094d1f9914`) as task owner and chair. Implementation wording within the settled contract belongs to the lane implementer and is recorded as a task note.
