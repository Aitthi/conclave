# Lead Council V1 Tooling Lane
<!-- conclave-plan:v1
{
"owner":"c5ab26f1-8119-4601-8f84-21094d1f9914","authority":"in-loop","council":{"chair":"c5ab26f1-8119-4601-8f84-21094d1f9914","members":["4fb2198c-e0d9-4e4b-af9e-d4e72542bace","d63832da-f4bb-4859-b9a0-4904be11ca8e"],"maxRounds":2},
"planPath":"docs/superpowers/plans/2026-07-10-lead-council-tooling.md","baseSha":"e8ce7bad254f6abbd2ac782a9d62b717701b759c","escalation":"c5ab26f1-8119-4601-8f84-21094d1f9914",
"readingOrder":["docs/superpowers/specs/2026-07-10-lead-council-v1-design.md","docs/superpowers/plans/2026-07-10-lead-council-tooling.md","docs/superpowers/plans/2026-07-10-lead-council-protocol.md","src-tauri/src/engine/commands/task.rs#CreateReq","src-tauri/src/engine/commands/task.rs#derive_brief_board_sections","src-tauri/src/engine/repo/task.rs#TaskEventRow","src-tauri/src/bin/conclave-cli.rs#lane_task_wiring","src-tauri/src/engine/uds.rs#task_verbs_round_trip_over_a_real_socket"],
"boundary":["src-tauri/src/bin/conclave-cli.rs","src-tauri/src/engine/commands/cli.rs","src-tauri/src/engine/commands/task.rs","src-tauri/src/engine/mod.rs","src-tauri/src/engine/plan_contract.rs","src-tauri/src/engine/repo/task.rs","src-tauri/src/engine/router.rs","src-tauri/src/engine/uds.rs","src-tauri/skills/tool-map/SKILL.md"],
"consumes":["src-tauri/src/engine/commands/task.rs#CreateReq","src-tauri/src/engine/commands/task.rs#brief","src-tauri/src/engine/commands/task.rs#claim","src-tauri/src/engine/repo/task.rs#watch","src-tauri/src/bin/conclave-cli.rs#lane_task_wiring","src-tauri/src/engine/uds.rs#task_verbs_round_trip_over_a_real_socket"],
"produces":["src-tauri/src/engine/plan_contract.rs#ExecutionHeader","src-tauri/src/engine/commands/task.rs#plan_check","src-tauri/src/engine/repo/task.rs#add_plan_check","src-tauri/src/bin/conclave-cli.rs#current_plan_fingerprint"],"gates":["cd src-tauri && cargo fmt --check","cd src-tauri && cargo test","cd src-tauri && cargo clippy --all-targets --all-features -- -D warnings","git diff --check"]
} -->

## Goal

Add the minimum engine and CLI support that makes the composition-first Lead council protocol efficient and mechanically visible: atomic watcher subscription, complete bounded challenge context, a strict ten-line plan contract, a fingerprinted plan-check event, and a non-blocking stale-plan warning at claim and lane start.

## Non-goals

- Do not add or migrate database tables.
- Do not add a council domain object, room, broadcast message, vote, quorum, orchestrator, task co-owner, or mutable task-plan command.
- Do not print full plans from `task brief`.
- Do not modify `src/`, frontend IPC types, fixtures, or views.
- Do not hard-block a claim because plan check is absent or stale.
- Do not add YAML, TOML, Markdown, regex, tree-sitter, or hashing dependencies when the standard library and existing crates suffice.

## Decisions

- The header grammar and behavior are frozen by `docs/superpowers/specs/2026-07-10-lead-council-v1-design.md` and the protocol lane.
- The stored task plan is an immutable creation snapshot. The repo file named by `planPath` is the only mutable plan.
- Fingerprints are SHA-256 over exact canonical plan bytes resolved from the CLI's current checkout.
- A successful check appends a typed `plan_check` event. The usual invocation wraps it in `task gate` for ordinary gate evidence.
- Council membership uses existing `task_watch` rows. No council membership is inferred from chat traffic.
- Existing command wire shapes remain byte-for-byte unchanged when the new flags and council header are absent.
- The `task claim` wire shape remains byte-for-byte unchanged in every case. Freshness is a CLI preflight, not a sixth argv word or an engine claim field.

## Ordered edits

### 1. Atomic watcher subscription

1. Add optional watcher ids to `src-tauri/src/engine/commands/task.rs#CreateReq` and validate a bounded, deduplicated list.
2. Extend `src-tauri/src/engine/repo/task.rs#NewTask` and `create` so owner plus supplied watchers are inserted into `task_watch` in the same transaction as the task and its created event.
3. Reject unknown and cross-workspace watcher ids without leaving a task behind.
4. Add `--watchers <comma-separated-ids>` to `task create` parsing, usage, self-argument expansion, payload mapping, and tests in `src-tauri/src/bin/conclave-cli.rs` and `src-tauri/src/engine/commands/cli.rs`.
5. Preserve current create output when the flag is absent. Add an additive `watcherAgentIds` array only when it is useful to confirm the requested subscriptions.

### 2. Complete bounded open challenges

1. Extend `PendingChallenge` and `derive_brief_board_sections` in `src-tauri/src/engine/commands/task.rs` with `actorAgentId`, `evidence`, `proposal`, and `default` from the open challenge event.
2. Keep id, claim, status, and `deadlineAt` unchanged.
3. Bound the number of open challenges by the existing brief limit and bound each rendered field so a hostile or accidental memo cannot turn `task brief` into a transcript dump.
4. Update the CLI brief renderer to show the new fields in a stable order.
5. Add tests for full fields, absent deadline, multiple tasks, truncation, and ruled challenges disappearing from the open set.

### 3. Header parser and plan contract

1. Add `src-tauri/src/engine/plan_contract.rs` and export it from `src-tauri/src/engine/mod.rs`.
2. Define `ExecutionHeader`, `CouncilHeader`, anchor/path helpers, validation errors, and serialization with existing `serde` and `serde_json`; use `deny_unknown_fields`.
3. Parse at most the first ten lines and 16 KiB. Require title on line 1, the version opener on line 2, and the closing delimiter on line 10.
4. Normalize paths, reject escape forms, cap arrays and strings, and validate all fields from the design spec.
5. Treat `path#anchor` as an exact text anchor. Consumed anchors must exist. Produced paths must fall inside the task boundary; produced anchors may be new.
6. Require canonical header equality between the immutable stored snapshot and the current `planPath` file.
7. Require the standard plan headings and reject unresolved placeholder markers or broad discovery instructions.
8. Unit-test valid protocol, code, and UI headers plus every rejection rule without touching the database.

### 4. Plan check and fingerprint event

1. Add `task.planCheck` to `src-tauri/src/engine/router.rs` and the CLI route allowlist in `src-tauri/src/engine/commands/cli.rs`.
2. Add `conclave task plan-check <workspace> <slug>` parsing, usage, payload construction, rendering, and self-argument expansion in `src-tauri/src/bin/conclave-cli.rs`.
3. The CLI resolves the plan relative to its current checkout, reads exact bytes, computes SHA-256, and sends the content, path, and fingerprint for engine validation. Reuse the existing `sha2` crate already present in `src-tauri/Cargo.toml`; do not add a dependency.
4. The engine loads the task, parses the immutable header, validates the supplied canonical file content and task fields, and confirms council agents belong to the workspace.
5. Add `src-tauri/src/engine/repo/task.rs#add_plan_check` using the existing append-only event path. On success record `contractVersion`, `planPath`, `planFingerprint`, `baseSha`, and actor. A validation failure returns an error and appends no successful event.
6. Return a bounded JSON/text success result with the slug, plan path, fingerprint, boundary count, anchor count, and gate count.
7. Add command, router, CLI mapping, renderer, repository persistence, and failure tests in the existing inline suites.
8. Extend `src-tauri/src/engine/uds.rs#task_verbs_round_trip_over_a_real_socket` with exact live-socket argv coverage for create-with-watchers and plan-check, plus a regression assertion that claim retains its existing five-word expanded wire form. This test is part of the lane boundary, not an integration follow-up.

### 5. Freshness warning at the implementation boundary

1. Add a CLI helper such as `current_plan_fingerprint` that gets the bounded task header, resolves `planPath` against an explicit checkout root, and computes the exact-byte fingerprint.
2. Before direct `task claim`, the CLI uses the existing task read surface to load the stored header and newest `plan_check` event, then compares that fingerprint against the plan in the invoking checkout.
3. `lane start` creates the worktree first, then `lane_task_wiring` runs the same local preflight with `.claude/worktrees/<slug>` as the root so the hash matches the plan the implementer will read, even when main has uncommitted plan edits.
4. Print a prominent stderr warning when the check event is missing, the preflight cannot be verified, the file is unreadable, the header differs, or the fingerprint mismatches. Do not reject or alter the subsequent claim.
5. Send the existing five-word expanded `task claim <actor> <workspace> <slug>` argv unchanged. Do not add a fingerprint argv word or `ClaimReq` field. This preserves compatibility with the pinned allowlist and old running engines.
6. Test fresh, missing event, unverifiable old-engine response, stale, unreadable, non-council, direct-claim, and lane-start paths. The lane-start case must make main and the new worktree plans differ and prove the worktree fingerprint is the referent. Existing claim mapping and non-council response snapshots must remain unchanged.
7. Add an integration/release note that the Conclave app must restart immediately after installing this build before `task plan-check` or create-with-watchers is used; the existing claim verb continues to work across version skew.

### 6. Tool documentation

Update `src-tauri/skills/tool-map/SKILL.md` with the exact `task create --watchers` and `task plan-check` syntax, the dual event behavior when wrapped in `task gate`, the bounded-header invariant, and the warning-only v1 claim behavior. Point to the design spec instead of copying its full schema.

## Verification

Run `cd src-tauri && cargo fmt --check`. It must exit 0 without a diff.

Run `cd src-tauri && cargo test`. All existing and new repository, task command, CLI mapping, router, renderer, and lane tests must pass.

Run `cd src-tauri && cargo clippy --all-targets --all-features -- -D warnings`. It must exit 0 with no warning.

Run `git diff --check`. It must print nothing.

After integration, run a real council check as a recorded gate:

```sh
conclave task gate <workspace> <slug> -- conclave task plan-check <workspace> <slug>
```

The task must contain a successful typed `plan_check` event and a successful ordinary gate event with the same invocation. Changing one byte in the canonical plan must make a subsequent cold claim print a stale fingerprint warning while still claiming successfully.

## Risks

- `task.rs` and `conclave-cli.rs` are shared command surfaces with frozen wire-shape tests. Keep all new response fields conditional and additive.
- `uds.rs` pins the real `cli.exec` wire through a live socket and can fail only at the full-suite gate. Keep it in-boundary and update its exact argv for create-with-watchers and plan-check while preserving the existing five-word claim form.
- A new CLI cannot use new plan-check/create-with-watchers forms against an old running engine. Restart the app after install; do not widen the existing claim wire merely to carry freshness data.
- Creating the task before validating watchers can leave partial state. Put watcher validation and inserts inside the creation transaction.
- Resolving against the workspace's main checkout would miss lane-local changes. Hash the file in the CLI's current checkout.
- Exact anchor matching is a stale-anchor guard, not semantic name resolution. Do not claim AST-level certainty.
- A broad prose validator will create false positives. Restrict prose checks to required headings and explicit placeholder/discovery markers.
- A passing typed event without an ordinary gate is weaker evidence. The Leadership protocol requires the gate wrapper; v1 claim warnings key off the fingerprint event.
- Unbounded challenge fields can erase context savings. Apply stable count and character caps.

## Rejected alternatives

- A full-plan brief flag is unnecessary because the complete contract fits the existing ten-line excerpt.
- A plan update command creates a second mutable plan copy and repeats the drift already observed in the `rtk` program.
- Comparing only git HEAD misses uncommitted plan edits and can warn on unrelated commits.
- Blocking claim in v1 gives a new validator authority to halt the workspace before dogfood calibrates it.
- A schema migration is unnecessary because watchers and task events already store the required durable facts.

## Escalation

Spec, header, wire-contract, or scope conflicts are filed as task challenges to Aoki (`c5ab26f1-8119-4601-8f84-21094d1f9914`), who rules them in-loop. Implementation choices that preserve the frozen behavior belong to the lane implementer and are recorded as task notes.
