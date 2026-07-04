# Plan: tool map — every agent knows which conclave verb does what

Date: 2026-07-04 · Owner: Detoro bfb737ff · authority: in-loop
Task: `tool-map` · Implementer: Dew 40d90aed

## Why

Human request (2026-07-04): the system prompt should state clearly, as a short
table, which tool to use for what. Constraint discovered while scoping: the
spawn preamble is contractually a SINGLE LINE with NO `=` characters (ADR
0001; codex receives it via `-c developer_instructions=…` — see
`src-tauri/src/engine/agentctx.rs:21-31`), so a literal markdown table cannot
live there. Two-layer design instead:

1. the TABLE lives in a new mandatory builtin skill — it lands at a fixed spot
   in every agent's skill sidecar, which the system prompt already orders the
   agent to (re)read on every fresh context;
2. the PREAMBLE gains one short single-line sentence naming the verb families,
   so an agent knows the tools exist even before reading the sidecar.

## Task 1 — new builtin skill `src-tauri/skills/tool-map/SKILL.md`

New folder + file, frontmatter EXACTLY in the house format (see
`src-tauri/skills/memory/SKILL.md` for reference):

```
---
name: Tool Map
description: One-screen map of which conclave verb to use for what — work items on tasks, worktrees on lanes, ad-hoc facts on the blackboard, knowledge in memory, messages via tell.
mandatory: true
---
```

Body: ONE table plus at most 4 lines of framing prose.

AMENDED 2026-07-04 (human scope change, while Dew mid-lane): the table covers
EVERY subcommand `conclave help` prints — no verb left out. Grouped by family,
one row per verb (or per verb pair sharing one purpose). Syntax column must
match `conclave help` exactly; purpose column verified against
`src-tauri/src/bin/conclave-cli.rs` behavior, not guessed. Rows, grouped:

- WORK ITEMS: `task list` (see the board; `--state s` filters) · `task get`
  (plan, boundary, canon, events, gates) · `task create … [--boundary]
  [--canon] [--plan-file]` (lead cuts a lane) · `task claim` (take it) ·
  `task state <state>` (implementers: review|abandoned; merged = integrator)
  · `task note` (log progress/decisions/outcomes) · `task gate -- <cmd…>`
  (run verification, proof on the ledger) · `task challenge --claim
  --evidence --proposal --default [--deadline-min N]` (dispute with a
  default) · `task rule <challengeEventId> <text>` (settle a challenge) ·
  `task close` (live state → merged shortcut + memory reminder) ·
  `task watch` / `task unwatch` (follow/stop following a lane).
- LANES: `lane start` (claim + worktree in one step) · `lane finish`
  (integrator teardown after merge) · `lane guard install` (shared-checkout
  commit-scope guard).
- PEERS: `agent list <ws>` (roster: ids, roles, skills, working flag) ·
  `tell <agentId> <text>` (message a peer — the ONLY channel that reaches
  one) · `send <sessionId> <text>` (inject into a session by session id;
  orchestration plumbing — prefer `tell`) · `run <orchestratorId>
  <prompt…>` (hand a prompt to an orchestrator agent).
- WORKSPACES: `ws list` (all workspaces) · `ws use <workspaceId>` (set the
  default).
- BLACKBOARD: `bb list` / `bb get <key>` / `bb set <key> <value>` /
  `bb delete <key>` (ad-hoc durable facts that fit no task; delete only
  your own finished keys).
- MEMORY: `memory search <query…> [--limit N]` (recall before you research)
  · `memory remember <text…>` (save hard-won knowledge) · `memory delete
  <chunkId>` (remove a wrong/stale memory) · `memory status` (store health).
- CONTEXT: `snapshot save <text…>` (persist YOUR handoff before a
  clear/restart) · `snapshot last` (re-read it after) · `snapshot list
  <sessionId>` / `snapshot read <snapshotId>` (browse saved handoffs) ·
  `snapshot create <sessionId> <type> [label]` (snapshot another session;
  orchestration plumbing) · `restart` (self-triggered restart — follow its
  printed save-then-die contract).
- `help` (this list, live — trust it over any cached copy).

Framing prose (the altitude rule, ≤4 lines): work items ride tasks, never bb
keys; bb is for facts that fit no task; memory is for knowledge that outlives
tasks; when unsure, `task list` first. Point to the deeper skills by name
(Collaboration, Implementer/Leadership, Memory) for protocol detail.

## Task 2 — one-line tool map in the preamble

`src-tauri/src/engine/agentctx.rs`, `bootstrap_preamble` (lines 59-73):

- APPEND one sentence (inside the same single format string), roughly: work
  items and claims ride `conclave task` (list/get/claim/note/gate/challenge)
  and `conclave lane start/finish` for worktrees; durable knowledge via
  `conclave memory search/remember`; your skills file carries the full tool
  table. MUST contain no `=` and no newline in the OUTPUT (the `\`-continued
  source lines are fine — they compile to one line).
- AMEND the existing bb sentence's tail ("check it before starting work
  someone may have claimed or planned") — claims live on tasks now; the
  check-before-starting pointer should name `conclave task list {ws_id}`.
- Update the unit tests at `agentctx.rs:356-380` to assert the new content
  (whatever they assert today, mirror the pattern) AND keep/extend the
  single-line + '='-free assertions if present — that contract is the risk.

## Out of scope

- No change to `commands/instance.rs`, the sidecar composer
  (`repo/skill.rs::content_for_agent`), or any TS/UI file.
- No edits to the seven skills landed in 3274567.

## Gates (run via `conclave task gate` so evidence is on the ledger)

- `cargo test` (src-tauri) — preamble tests + skill frontmatter parsing.
- `cargo clippy --all-targets -- -D warnings`.
- Mellow LAND review before merge (blocking): verb/flag fidelity vs
  `conclave help`, preamble output still single-line and '='-free (assert via
  test, not eyeball), table rows complete per this plan.

## Risk ledger

- THE contract: preamble output single-line, zero `=`. `--deadline-min N` in
  the TABLE is fine (sidecar file, no constraint); in the PREAMBLE never write
  `=`; spell flags without values or omit them.
- `mandatory: true` puts the skill in EVERY sidecar including the designer's —
  intended; keep the table generic (no role-specific advice).
- Builtin skill ordering is fixed id order (folder name); `tool-map` sorts
  after `strategic-compact` — acceptable, do not rename folders to game order.
- Reaches agents on next rebuild+relaunch, same as 3274567.
