# `task create --no-self-watch`: let the Lead subscribe the Coordinator without subscribing itself
owner: 30fa04f4-e047-4241-a9ed-f452529952be · authority: in-loop · coordinator: Aitthi (7d560bae-09dc-4535-befb-feed5a95aa05) · escalation: challenges route to the task owner (Detoro); READY / BLOCKED / stall / review verdicts / dispatch are the Coordinator's.
Tier: routine. Reviewer: Mellow (b3a30e7b-5a9f-4d4d-a83e-768a9326632f). Detoro rules and merges on `MERGE-READY`. Start lane from main at or after the commit that carries ADR 0010 Amendment 1.

## Why now
Human observation 2026-10-07 in the OmniChat workspace: workers appeared to report to two leads. Root cause (Detoro, verified): `task create --watchers <id>` subscribes the OWNER plus the listed agents (`src-tauri/src/engine/commands/task.rs:563-567`; lead-council v1 chair semantics, spec `docs/superpowers/specs/2026-07-10-lead-council-v1-design.md` "Command Changes"). Under ADR 0010 the Lead creates every task with `--watchers <coordinatorId>`, so the Lead AND the Coordinator were both in `task_watch` and `notify_watchers` (`task.rs:1582-1611`) sent every READY/BLOCKED note, failing gate and review/merged transition to both. Confirmed in the DB: OmniChat `sweep-canon-nit`, `sweep-customers-api`, `sweep-mock-homes` each have two watcher rows (owner 89ffb764 + Aitthi bf2fdd0f). The Leadership skill line "do NOT watch routine lanes yourself" cannot be followed on the shipped engine. ADR 0010 Amendment 1 (on main) records the corrected facts — read it first; do not edit the ADR (a disagreement is a `task challenge`).

The council use case genuinely wants the chair subscribed, so the default stays. A switch opts the owner out.

## Rulings (final)

R1. Engine `task.create` (`src-tauri/src/engine/commands/task.rs`, `CreateReq` ~L519-538): add
```rust
    /// `--no-self-watch`: when `watcherAgentIds` is non-empty, do NOT add the
    /// owner to the subscription set (ADR 0010 Amendment 1 — a Lead subscribes
    /// the Coordinator without subscribing itself). Default `true` keeps the
    /// council chair semantics byte-for-byte; ignored when no watchers are
    /// supplied (the flag-less create still writes no rows).
    #[serde(default = "default_true")]
    self_watch: bool,
```
with a module-level `fn default_true() -> bool { true }` (grep first — reuse if one already exists in the file). In the watch-set resolution (~L563-567) the owner is pushed only when `req.self_watch`. Everything else (dedupe, cap, scope checks, single transaction, `watcherAgentIds` confirmation array reflecting what was actually subscribed) is unchanged.

R2. Engine argv parser `src-tauri/src/engine/commands/cli.rs` `Some("create")` arm (~L1307-1352): after the `--watchers` `take_flag`, add `let (no_self_watch, rest) = take_switch(&rest, "--no-self-watch");` BEFORE the `rest.is_empty()` title check; emit `params["selfWatch"] = json!(false)` ONLY when the switch is present (flag-less create stays byte-for-byte). All three usage strings in that arm gain ` [--no-self-watch]` after `[--watchers id,id]`. Update the arm's doc comment (~L1301-1306) to list the switch.

R3. `src-tauri/src/bin/conclave-cli.rs` L109 help line becomes:
`task create   <workspaceId> <slug> <title...> [--boundary p1,p2] [--canon txt] [--plan-file path] [--watchers id,id] [--no-self-watch]  (--watchers subscribes the owner + listed workspace agents in one transaction; --no-self-watch leaves the owner out)`. `expand_self_args` needs no logic change (it only appends `--owner self`); prove pass-through with a test (R5).

R4. `src-tauri/skills/leadership/SKILL.md` "Running through a Coordinator" bullet (currently L147-148, anchor on the quoted text) becomes:
`- Create every task with \`--watchers <coordinatorId> --no-self-watch\`; plain \`--watchers\` subscribes you too (council chair semantics), and the owner is still pinged once on the \`review\` transition by design. Watch only what you want woken for.`
No other skill or role text changes.

R5. Tests (mirror the neighbours' shape exactly):
- `commands/task.rs` after `create_with_watchers_deduplicates_owner_and_repeats` (~L1891): `create_with_watchers_and_no_self_watch_omits_owner` — payload `watcherAgentIds: [w1], selfWatch: false` → `created["watcherAgentIds"] == json!([w1])` and `repo::task::watchers` == `[w1]`; and `create_no_self_watch_without_watchers_is_the_flagless_create` — `selfWatch: false`, no watchers → no `watcherAgentIds` field, no rows.
- `commands/cli.rs` after `task_create_watchers_maps_to_watcher_agent_ids_and_drops_empties` (~L2479): `task_create_no_self_watch_maps_to_self_watch_false` (switch anywhere in the tail, title intact, `params["selfWatch"] == json!(false)`) and extend `task_create_without_watchers_omits_the_field` with `assert!(params.get("selfWatch").is_none())`.
- `bin/conclave-cli.rs` after `task_create_watchers_survive_with_explicit_owner` (~L4798): `task_create_no_self_watch_survives_owner_default_expansion` — `--watchers a --no-self-watch` + self → the switch survives and `--owner self1` is appended.
- `engine/uds.rs` ~L684 council round-trip test is untouched (plain form keeps chair semantics).

R6. Check `rg -n '"--watchers"' src-tauri/src` for any allowlist/pin of create flags beyond the parser and tests; if one exists, add the switch there in the same shape and say so in the READY note. (Detoro saw none: parser + tests only.)

## Files (boundary)
- `src-tauri/src/engine/commands/task.rs` (R1, R5)
- `src-tauri/src/engine/commands/cli.rs` (R2, R5)
- `src-tauri/src/bin/conclave-cli.rs` (R3, R5)
- `src-tauri/skills/leadership/SKILL.md` (R4)
Nothing else.

## Task table
| slug | tier | role | deps | acceptance |
|---|---|---|---|---|
| task-create-no-self-watch | routine | implementer-routine | — | gates below green on the ledger; READY note with SHA, test names added, R6 outcome |

## Steps
1. `conclave lane start 11ecf99b-53f4-4c24-b538-b19e5933a9e3 task-create-no-self-watch`; `pnpm install` once in the worktree.
2. Read ADR 0010 Amendment 1, then this plan. Apply R1-R6.
3. Gates (record each with `conclave task gate 11ecf99b-53f4-4c24-b538-b19e5933a9e3 task-create-no-self-watch -- <cmd>`; words after `--` unquoted):
   - `cargo test --manifest-path src-tauri/Cargo.toml --quiet task`
   - `cargo test --manifest-path src-tauri/Cargo.toml --quiet cli`
   - `cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check`
   - `cargo clippy --manifest-path src-tauri/Cargo.toml --quiet --all-targets -- -D warnings`
   No UI touched: no uishot gate.
4. One commit on `lane/task-create-no-self-watch` scoped to the boundary (`git commit -- <paths>` or `conclave stage commit`).

## Risk ledger
- Line numbers are from main a013f78; anchor on the quoted text if they shift.
- `#[serde(default)]` on a `bool` yields `false` — R1 MUST use `default = "default_true"` or the plain create silently drops the chair. The two R5 task.rs tests are the guard.
- Do not touch `docs/adr/0010-*` or this plan; challenge instead.

## Done means
READY note in the structured shape (`task:` / `status:` / `files:` / `note:` with SHA, the four gates recorded, test names, R6 outcome) posted on the task; report to Aitthi. Mellow reviews (`READY REVIEW-PASS @<sha>` / `BLOCKED REVIEW-FAIL @<sha>`); Aitthi posts `MERGE-READY task-create-no-self-watch @<sha>`; Detoro merges. Needs app rebuild + relaunch before the switch is usable from the live CLI.
