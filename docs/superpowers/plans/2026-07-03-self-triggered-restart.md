# Self-triggered restart — the agent wakes itself near context exhaustion

**Goal:** a CLI agent that sees its harness's "context nearly full" warning restarts ITSELF:
`conclave restart` → automated handoff → kill → fresh terminal → resume. No human action, no
manual snapshot step. Decision record: `docs/adr/0006-self-triggered-restart.md` (read first;
also the restart flow in `commands/instance.rs::restart` / `run_respawn_resume` and the
save-gated tail in `commands/snapshot.rs::save`).

**Owner/lead:** Detoro `bfb737ff` — design conflicts escalate to me; implementation judgment
within the plan is yours, logged in `progress:self-restart`.

## Lane fence — instance.rs is BUSY

Arta (`688719b6`) holds `agent.rs` / `instance.rs` / `agentctx.rs` for role-system Phase B
RIGHT NOW. Do Tasks 1–2 first (no overlap). Task 3 touches `instance.rs` + `state.rs`: START IT
ONLY after the lead posts `restart-lane-clear` (when Arta's Phase B lands). Do not pre-edit.

## Global constraints

- Repo `/Users/detoro/code/codeup`, branch `main`, base c0ac500 or later. No commits — lead
  integrates. TDD failing-test-first per task. Done = `cargo test --lib` green +
  `cargo clippy --lib` clean, evidence in `progress:self-restart`.
- All prompt/CLI text: English. Human-facing rationale in your terminal: Thai or English.

## Task 1 — `conclave restart` CLI verb (self-targeting)

`src-tauri/src/bin/conclave-cli.rs`: new `restart` subcommand, NO arguments — the target is
always the calling agent, resolved from `CONCLAVE_INSTANCE_ID` (same env the `snapshot save`
verb already uses; mirror its resolution + error message when unset). Sends `instance.restart`
on the bus with `{workspaceAgentId: <self>, self: true}`. Prints the engine's returned
`instruction` field verbatim to stdout (that text is the agent's next step — see Task 3).
Update the CLI usage/help text. Tests: mirror the existing CLI arg-parse tests (~line 389).

## Task 2 — standing rule in the Strategic Compact builtin skill

Locate the bundled strategic-compact skill (via `repo::skill::bundled_skills_dir`) and add a
short section: **when your harness warns that context is nearly full (e.g. an auto-compact or
low-context warning), run `conclave restart` yourself, read what it prints, then IMMEDIATELY
write the seven-section handoff and persist it with `conclave snapshot save` — the restart
fires only after your save lands; stalling leaves you degraded.** Keep the skill's existing
voice; English. Test: extend `shipped_skills_all_parse_and_include_collaboration`-style
coverage to assert the strategic-compact body names `conclave restart`.

## Task 3 — engine: `self: true` restart path (WAIT for `restart-lane-clear`)

`commands/instance.rs::restart` (~line 670): accept optional `self: bool` in the payload.
When true and the instance is LIVE: arm `mark_restart_pending` exactly as today, but DO NOT
`submit_line` the `restart_save_prompt` into the TUI (the caller is the agent itself, mid-turn
— injecting would interleave with its own output). Instead return
`{status: "restarting", phase: "saving", instruction: <text>}` where `instruction` tells the
agent: write the richest handoff per your Strategic Compact skill, then run
`conclave snapshot save <text>`; the restart fires after the save. Reuse the wording of
`agentctx::restart_save_prompt` (extract a shared constant rather than duplicating — that fn
is fair game once the lane clears; keep it single-line). Dead-instance and non-self behavior:
unchanged. The save-gated tail (`snapshot.rs::save` → `take_restart_pending` →
`run_respawn_resume`) needs NO changes — verify with a test, don't modify it.
Tests: self restart on a live instance arms the pending flag and does NOT write to the PTY
(assert via the fixture runtime), returns the instruction; non-self path still injects.

## Risk ledger

- **Arm TTL:** `mark_restart_pending` has a TTL — if the agent dawdles past it before saving,
  the save won't fire the tail. Task 3 must surface the TTL in the returned instruction
  ("save within N minutes") and the test must pin that the TTL used for self-restart equals
  the existing one (no silent divergence).
- **Agent restarts itself mid-critical-work:** the handoff protocol already covers this (NOW
  section carries the half-finished edit). No extra guard in v1.
- **Double trigger:** agent runs `conclave restart` twice before saving — `mark_restart_pending`
  overwrites the arm (documented behavior); second call must return the same instruction, not
  an error. Pin with a test.
- **`CONCLAVE_INSTANCE_ID` unset** (human runs `conclave restart` in a normal shell): clear
  error naming the env var, exit non-zero — never guess a target.

## Definition of done

Tasks 1–2 green now; Task 3 after `restart-lane-clear`; evidence per task in
`progress:self-restart`; then message the lead. Mellow reviews the lane before integration.
