# Lead Council V1 Design

**Status:** Settled for implementation

**Decision owners:** Aoki (chair) and Detoro (co-lead)

**Task owner:** `c5ab26f1-8119-4601-8f84-21094d1f9914`

**Authority:** `in-loop`

**Durable decision record:** Conclave blackboard key `decision:council-v1-composition-first`

**Code baseline:** `e8ce7bad254f6abbd2ac782a9d62b717701b759c`

## Problem

Conclave already permits multiple agents with Lead roles, arbitrary supervisor trees, pairwise agent messages, task watchers, and evidence-bearing task challenges. What it does not yet guarantee is a low-context planning handoff:

1. A chair must subscribe several Lead agents manually after task creation.
2. `task brief` omits the actor, evidence, proposal, and default from open challenges.
3. A task can be claimed without any mechanical signal that its plan was checked.
4. The full task plan is stored at creation, while the canonical repo plan can later change without a task event. This drift occurred in the `rtk` program.
5. Plans have no compact machine-checkable contract that tells a cold implementer exactly what to read, edit, consume, produce, and run.

The goal is not a new chat room. The goal is for three or more Lead agents to deliberate on the durable task ledger, converge under one accountable chair, and produce implementation tasks that a fresh agent can start from a slug without repository-wide discovery.

## Constraints

- A task continues to have exactly one owner. The owner is the council chair and final ruler.
- Existing challenge, ruling, watcher, task brief, task gate, and supervisor/LCA behavior remain canonical.
- Deliberation must survive context clears. Shared context is pulled from `task brief`, not pushed as copied transcripts.
- Implementers receive the settled work packet, not the council conversation.
- V1 adds no council table, message room, voting state, task co-owner field, or frontend view.
- V1 adds no parsing dependency and exactly one database migration: `0018_task_event_plan_check.sql`, widening the `task_event.kind` CHECK set (see Storage; ruling 06a0c35b).
- A new linter warns at the claim boundary before it is trusted enough to block work.
- Any later `src/` follow-up remains subject to the standing UI pixel gate.

## Council Protocol

### Roles

- **Chair:** the task owner. Creates the planning task, names members, synthesizes evidence, rules every material challenge, updates the canonical plan, runs the plan check, and creates downstream implementation tasks.
- **Members:** at least two additional agents with delegated Lead or domain authority. They pull the same brief, contribute bounded evidence, file material disagreement as a challenge, and watch the task.
- **Implementer:** receives a downstream task only after all planning challenges are ruled and the plan check passes.

The chair may rotate between decisions, but a single task never has multiple owners.

### Convene Sequence

1. The chair creates one planning task with a `conclave-plan:v1` execution header and `--watchers <member-ids>`.
2. Task creation subscribes the chair and every supplied member atomically.
3. Each member runs `conclave task brief <workspace> <slug>` and reads only the header's ordered files and anchors.
4. Each member contributes one bounded evidence memo. A material alternative is a task challenge with claim, evidence, proposal, default, and deadline.
5. A counter-position cites the challenged event id. Pairwise `tell` messages may announce work but never settle it.
6. The chair rules every open challenge. Two rounds without new evidence end the exchange and the chair rules.
7. The chair updates the one repo plan named by `planPath`. No member edits a separate copy.
8. The chair records `conclave task gate <workspace> <slug> -- conclave task plan-check <workspace> <slug>`.
9. After a green check and zero open challenges, the chair creates downstream implementation tasks and sends each implementer only the slug, reading order, and escalation target.

If review finds a defect in an unclaimed task header or file boundary, the chair rules the challenge, amends the canonical plan, abandons the immutable task, and recreates it. The lane never relies on an out-of-boundary integration commit as a substitute for a correct task boundary.

## Execution Header

Every council-planned task starts with a ten-line contract. Line 1 is the Markdown title. Lines 2 through 10 are a JSON object inside an HTML comment:

```markdown
# Example Task
<!-- conclave-plan:v1
{
"owner":"<workspace-agent-id>","authority":"in-loop","council":{"chair":"<workspace-agent-id>","members":["<id>","<id>"],"maxRounds":2},
"planPath":"docs/superpowers/plans/example.md","baseSha":"<40-hex-sha>","escalation":"<workspace-agent-id>",
"readingOrder":["docs/superpowers/specs/example.md","docs/superpowers/plans/example.md","src/path.rs#ExistingSymbol"],
"boundary":["src/path.rs"],
"consumes":["src/path.rs#ExistingSymbol"],
"produces":["src/path.rs#NewSymbol"],"gates":["cd src-tauri && cargo test","git diff --check"]
} -->
```

The stored `task.plan` remains the immutable creation snapshot. `planPath` identifies the canonical repo file. The header in that file must remain byte-for-byte equal to the stored header.

If council review finds a header or boundary defect before claim, the chair amends the canonical plan, abandons the immutable task, and recreates it with the corrected header and boundary. The chair does not replace machine-checkable evidence with a manual integration workaround merely to preserve the old slug.

### Field Rules

- `owner` equals `task.ownerAgentId`.
- `authority` equals `in-loop`.
- `council` is optional for ordinary tasks. When present, its chair equals `owner`, chair plus members contains at least three distinct workspace agents, and `maxRounds` is between 1 and 4.
- `planPath` is a normalized repo-relative path, appears as a plain entry in `readingOrder`, and resolves inside the current checkout.
- `baseSha` is a full commit id and must be an ancestor of the checkout being checked.
- `escalation` names the task owner unless the plan records a stricter supervisor route.
- `readingOrder` is ordered and bounded. Plain paths name files; `path#anchor` names an exact text anchor inside a file.
- `boundary` is the normalized, sorted set-equivalent of `task.fileBoundary`.
- Every `consumes` anchor exists when checked. A consumed file may be outside the writable boundary.
- Every `produces` path is inside the writable boundary. Its anchor may be new.
- `gates` is nonempty and contains the exact commands the lane will run.
- Arrays and strings have conservative length caps so the ten-line header cannot become a context dump.

Paths reject absolute forms, backslashes, empty components, `.` and `..`, and symlink escape from the checkout root. The parser uses existing `serde_json` with unknown fields denied.

## Plan Check

`conclave task plan-check <workspace> <slug>` performs these checks:

1. Load the task and parse only the first ten stored plan lines.
2. Resolve `planPath` relative to the effective implementation checkout: the invoking checkout for direct `task plan-check` and `task claim`, or the newly created lane worktree for `lane start`.
3. Read the canonical plan, require the same ten-line header, and compute SHA-256 over the exact file bytes.
4. Validate the header fields, task owner, task boundary, workspace agents, paths, anchors, required Markdown sections, UI canon rule, and command list.
5. Reject unresolved placeholder markers and discovery instructions that tell the implementer to search broadly instead of naming a path or anchor.
6. Append a typed `plan_check` event only on success. Its payload contains contract version, plan path, plan fingerprint, and base commit; the acting agent rides the event's `actor_agent_id` column, not the payload (adjudicated on the task ledger — Guetta's F2, acting-chair note be2e68d7: column-canonical, no payload duplication).
7. Print a bounded success packet suitable for a recorded task gate.

Required Markdown sections after line 10 are `Goal`, `Non-goals`, `Decisions`, `Ordered edits`, `Verification`, `Risks`, `Rejected alternatives`, and `Escalation`.

The command does not print the full plan. `task brief` remains bounded because the complete execution contract already fits in its existing ten-line excerpt.

## Freshness Warning

The CLI performs a local preflight before sending the existing `task claim` wire request. It reads the task and latest typed `plan_check` event through existing commands, then recomputes the canonical plan fingerprint in the checkout the implementer will read. Direct claim uses the invoking checkout. Lane start creates its worktree first and hashes the plan inside that worktree, even when the CLI process was invoked from main. For a council-tagged task the CLI emits a loud warning when:

- no successful typed `plan_check` event exists;
- the canonical plan cannot be resolved;
- the current fingerprint differs from the latest successful fingerprint; or
- the current execution header differs from the immutable stored header.

V1 warns but does not refuse the claim. This avoids freezing the pipeline on a new linter's false negative. A later version may promote the same predicate to a refusal with an explicit recorded override after dogfood shows the warning is reliable.

The `task claim` argv and engine payload remain unchanged. This keeps old and new clients compatible for the existing claim verb. When a new CLI talks to an old engine, the preflight is unverifiable and warns, but the ordinary claim still proceeds. The newly added `plan-check` verb and create-with-watchers flag require an app restart after installing the new build.

## Command Changes

### `task create --watchers`

`conclave task create ... --watchers <id,id,...>` adds the owner plus the supplied workspace agents to `task_watch` in the same transaction as task creation. Unknown agents, cross-workspace agents, and malformed lists fail the create. The existing command without the flag is unchanged.

### `task brief`

Each open challenge adds bounded `actorAgentId`, `evidence`, `proposal`, and `default` fields alongside the existing id, claim, status, and deadline. The CLI renderer shows those fields without printing closed council history.

### `task plan-check`

This additive command validates and fingerprints the canonical plan. It is registered through the existing task router and CLI allowlist. It is normally executed through `task gate` so the ledger contains both the typed fingerprint event and ordinary gate evidence.

## Storage

One migration is required — nothing else changes shape (ruling 06a0c35b on challenge 731c8b3c, amending this section's original "no migration" claim, which assumed `task_event.kind` was open-ended and was disproven empirically at `0012_task_system.sql:20`):

- migration `0018_task_event_plan_check.sql` rebuilds `task_event` to widen the `kind` CHECK set with `'plan_check'`, preserving all rows and `idx_task_event_task` (SQLite cannot alter a CHECK in place);
- task ownership, plan snapshot, boundary, and canon remain on `task`;
- council membership uses the plan header plus existing `task_watch` rows;
- evidence, challenges, rulings, gate results, and plan fingerprints use append-only `task_event` rows — `plan_check` as its own typed kind, because encoding it under `note`/`gate` would bake mistyped rows into the permanent ledger;
- the repo plan named by `planPath` remains the only mutable plan.

## UI

V1 makes no `src/` changes. Multiple Lead definitions and supervisor forests already render. Chat Hub remains a read-only view over pairwise traffic, not a new group room. A later Lane Board detail enhancement may expose plan, activity, and escalation tabs, but it is not required for the low-context handoff.

## Acceptance

1. A chair and at least two council members complete one real planning task using watches, challenges, and rulings.
2. Every material decision and rejection is reconstructable from the repo spec/plan plus challenge and ruling events.
3. The planning task has zero open challenges before implementation tasks are created.
4. `task brief` gives every council member the complete execution header and complete bounded open-challenge fields.
5. Each downstream task records a green plan-check gate whose fingerprint matches the plan file in the implementer's checkout.
6. A cold-start implementer receives only the slug. In its first eight tool calls it uses `task brief` and bounded reads named by the header, with no repo-wide tree, code map, grep, or task-history sweep.
7. Claiming a council-tagged task with a missing or stale fingerprint visibly warns but still succeeds.
8. Existing non-council task creation, claim, lane start, brief, watcher, challenge, and routing tests remain green.
9. `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, and `git diff --check` pass.

## Rejected Alternatives

- **Multi-owner tasks:** make challenge routing, stall paging, supervisor/LCA escalation, and integration accountability ambiguous.
- **Voting or quorum state:** counts rank rather than evidence and still needs a tie-breaker.
- **Council tables or a read model:** duplicate the task ledger before any real council has demonstrated a missing durable entity.
- **A new group-chat UI:** improves conversation appearance but does not produce an executable work packet.
- **Fusion:** fans isolated prompts to providers and synthesizes them; it does not let live workspace agents deliberate on a task.
- **Passing the council transcript to implementers:** directly spends the context the feature is meant to save.
- **A mutable task-plan update command:** creates two mutable plan copies. The immutable header plus fingerprinted repo plan makes staleness explicit.
- **Hard-blocking claim in v1:** lets a first-version validator freeze otherwise valid work.
- **Printing the full plan from `task brief`:** turns a bounded resume packet into a context dump.

## Risks

- **Chair latency:** mitigate with per-domain chairs, deadline defaults, and the two-round stop rule.
- **Noisy councils:** use one bounded evidence memo per member and challenge only material alternatives.
- **False freshness:** hash exact bytes from the checkout the implementer will use.
- **Brittle prose parsing:** validate the strict header and required headings; prose remains rationale rather than validator input.
- **Watcher fan-out:** cap watcher count and deduplicate ids before insert.
- **Context regression in skills:** keep the canonical protocol in the sidecars concise and self-contained. Human directive 2026-07-10 (blackboard `directive:no-docs-paths-in-skills`): SKILL.md files must not reference `docs/superpowers/...` paths — skills are app-bundled templates regenerated per workspace. A council member finds this spec through the task header's `readingOrder`, never through a skill.
