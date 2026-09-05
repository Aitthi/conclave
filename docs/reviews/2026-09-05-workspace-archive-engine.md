# Archive engine review

Initial commit reviewed: `5794153dc41970e65f606bc1709376f6555ae389`
Closure commit rechecked: `afa6a9183841308708e21681c0fbd95f1cdf5a16`
Reviewer: Armin (`be81029a-bde1-4d64-ad03-d3079cb19603`)
Date: 2026-09-05

## Verdict

**Ship pending Aoki's integration gates.** Closure commit `afa6a9183841`
resolves both findings below: global `agentDef.delete` now rejects atomically
when any affected workspace is archived, and the unrelated formatting hunk is
gone.

The simpler durable design remains a nullable `workspace.archived_at` plus
central eligibility checks. It avoids reusing `hidden` or creating a second
workspace class, and the implementation follows that approach.

## Closed findings

### Major — `agentDef.delete` bypassed archive preservation (resolved)

`src-tauri/src/engine/commands/agent.rs:393-428` finds every instance of a
definition and invokes `instance::remove_under_workspace_write` after acquiring
the workspace locks, without checking `archived_at`. That helper tears down and
deletes the workspace-agent and dependents (`src-tauri/src/engine/commands/instance.rs:1381-1402`).

Consequently, deleting a globally shared definition can erase an archived
workspace's agent/session/history even though neither `workspace.restore` nor
the explicit permanent `workspace.delete` operation occurred. This conflicts
with `docs/plans/2026-09-05-workspace-archive-engine.md`'s preservation and
public-membership-mutation contract.

Closure commit `afa6a9183841` now runs `require_active` for every affected
workspace after taking the sorted write guards and before any teardown
(`commands/agent.rs:402-432`). `require_active` rejects the archived target, so
the loop cannot reach `remove_under_workspace_write`; mixed active/archived
memberships are therefore all-or-nothing. The new routed test at
`commands/agent.rs:1020-1139` verifies rejection preserves both agents and
sessions, then restore allows the original global deletion path. Focused Cargo
test ran one test and passed.

Challenge `998211c5-317e-40c3-938c-d40ec50a2ffc` is resolved by this change.

### Minor — unrelated shared-file formatting churn (resolved)

`src-tauri/src/engine/repo/workspace_agent.rs:1010-1013` wraps an existing test
expression unrelated to archive state. The functional archive hunk is the
`RuntimeEligibility.archived_at` projection at `:801-816`. This violates the
plan's shared-file churn check and needlessly complicates integration.

Closure commit `afa6a9183841` restores the prior one-line expression. Challenge
`1060a02c-0262-44ae-b46b-cf4c0870bc6b` is resolved.

## Paths traced and verified

- Archive takes the workspace write lock, validates hidden/started/live state,
  archives and normalizes statuses in one transaction, and is idempotent
  (`commands/workspace.rs:254-306`; `repo/workspace.rs:97-117`). Restore stays
  stopped and does not launch.
- Spawn, resume, restart, and the detached restart tail acquire the workspace
  read lock and recheck `archived_at` immediately before launch
  (`commands/instance.rs:780-847`, `:2068-2127`). The start/archive race test
  covers both serialization outcomes (`commands/workspace.rs:691-710`).
- Direct send and injection recheck delivery eligibility after workspace/agent
  locks and before stdin or queued-row effects (`commands/message.rs:72-108`,
  `:155-270`).
- Draft and fusion retain a workspace read guard across the one-shot/pipeline
  and recheck active state after acquisition (`commands/draft.rs:594-617`,
  `commands/fusion.rs:377-445`); focused tests cover archive busy/recheck
  behavior (`draft.rs:1231-1265`, `fusion.rs:610-646`).
- Normal list excludes hidden and archived; archived list is bounded to
  non-hidden archived rows; internal `get` remains historical (`repo/workspace.rs:61-135`).
- Migration `0029` is additive and the v27 populated-graph upgrade asserts
  `archived_at IS NULL`, graph retention, and schema version 29
  (`migrations/0029_workspace_archive.sql`, `db.rs:610-697`).

Focused review checks: `git diff --check 5794153dc419^ 5794153dc419`,
`git diff --check 5794153dc419 afa6a9183841`, and the one-test Cargo filter
above passed. The earlier engine gate used an unqualified `--exact` name and
therefore selected zero tests; the reviewer reran the intended test without
`--exact` and confirmed one passing test. No full integration gate was run;
Aoki owns those gates. No source was modified by this review.
