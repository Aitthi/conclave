# Gate-note wake: Runner `GATES-*` and reviewer `REVIEW-REROUTE` notes must wake watchers
owner: 30fa04f4-e047-4241-a9ed-f452529952be · authority: in-loop · coordinator: Aitthi (7d560bae-09dc-4535-befb-feed5a95aa05) · escalation: challenges route to the task owner (Detoro); READY / BLOCKED / stall / review verdicts / dispatch are the Coordinator's.
Implementer: knock2 (5fe5dc51-892d-4fa2-9270-546341cdf938, Implementer (Standard)). Reviewer: Mellow (b3a30e7b-5a9f-4d4d-a83e-768a9326632f, Reviewer (Complex) — `review: complex`, the change is the watcher-notification contract every agent relies on). Detoro merges on `MERGE-READY <slug> @<sha>`. Start the lane from main at or after cb2c010. One task, no deps.

## Why now (human report 2026-10-09, verbatim)
"แก้ปัญหา Role runner หน่อย คือ runner ทำงานเสร็จแล้ว ชอบไม่แจ้ง Coordinator ทำให้งานทั้งหมดเงียบไปเฉยๆ" — the Runner finishes, the Coordinator is never told, the whole pipeline goes silent.

## Root cause (verified in `conclave.db` and `src-tauri/src/engine/commands/task.rs`, main cb2c010)
- The Runner DID report every time: every gate re-run ended with a `GATES-OK @<sha> …` / `GATES-RED @<sha> …` task note (ledger: brust `m1b1-ir-readers` 2026-10-08T16:24Z, `m1b2-placement-tier` 01:38Z; OmniChat `ksw-orderlink-customers` 02:36Z, `ksw-orderlink-products` 02:37Z/02:44Z, `ketshopweb-orderlink-flex` 02:45Z, `ketshopweb-orderlink-ui-core` 03:16Z).
- None of those notes reached the Coordinator. `note_wakes_watchers` (`task.rs:1581-1585`) fans a note out to watchers only when its text starts with `READY`, `BLOCKED` or `ESCALATION`; `GATES-OK` / `GATES-RED` / `GATES-SKIPPED` and the reviewer's `REVIEW-REROUTE` are ledger-only. `inter_agent_message` holds zero `[task …] Illenium: note — GATES-…` rows; the only GATES lines Aitthi ever received were ad-hoc tells Illenium sent after being prodded (01:43Z, 03:35Z "RUN-DONE …").
- The Coordinator skill (`src-tauri/skills/coordinator/SKILL.md:55`) says "wait for that note (your task watch delivers it)" — false for `GATES-*`. Observed silence: `m1b1-ir-readers` GATES-OK 16:24Z → review dispatched 00:20Z next day (after a lead tell); the five OmniChat tasks 02:36–03:16Z → dispatched 03:35Z.
- This is a plan defect, not a Runner defect: ADR 0010 Amendment 3 D7 (plan `docs/plans/2026-10-08-team-tiers-v2.md`, Detoro) introduced the `GATES-*` gate phrases while ADR 0010 L16-19 already recorded that only `READY`/`BLOCKED`/`ESCALATION` notes wake watchers. Credit: the human (report), Illenium's ledger (evidence).

## Decisions (Detoro, final — rejected alternatives recorded)
- D1. Fix at the engine: `note_wakes_watchers` gains the four protocol phrases `GATES-OK`, `GATES-RED`, `GATES-SKIPPED`, `REVIEW-REROUTE` (exact prefix, case-sensitive, position 0 — same rule as the existing three). Rejected: having the Runner ALSO `tell` the Coordinator (double delivery into the Coordinator's context on every re-run, and Comms Protocol says a gate phrase is received through task watch, never relayed); making every note wake watchers (the filter exists to keep progress notes out of watchers' contexts — ADR 0008 Lane B decision 1).
- D2. The Runner skill states the delivery contract in one sentence: the `GATES-` phrase is the FIRST character of the note — no date, name or preamble before it — and the Runner does not also tell. Illenium's notes already comply; the sentence pins it.
- D3. The Coordinator skill gets a bounded fallback for the same failure class: ten minutes with no `GATES-` note → read `conclave task brief <ws> <slug>` ONCE; a `GATES-` note already on the ledger is acted on as if it had just arrived; none → tell the Runner once more, then `GATES-SKIPPED <slug> @<sha> runner silent` and go on. Rejected: periodic polling (Idle time is oversight time: nobody polls).
- D4. Guard: any plan or skill that introduces a phrase a watcher is told to WAIT FOR must add it to `note_wakes_watchers` in the same lane, and the lead's plan names the test that pins it. Recorded in ADR 0010 Amendment 4; the leadership skill is not edited in this lane (one module per standard task).
- D5. Found alongside, ruled out of this lane: role skill bundles are COPIED into a definition at create (ADR 0005, `agent.rs:257-269`), so Amendment 3's `tiering` skill never reached the existing Detoro and Aitthi definitions (`agent_definition.selected_builtin_skill_ids` = `["agent-loop","leadership"]` / `["coordinator"]`; `launchedSkillIds` confirm). Rollout = a Builder edit by the human (add Tiering to Detoro and Aitthi). Recorded in Amendment 4 as the rollout rule for future role-bundle changes.

## Task `gate-note-wake` — tier standard, role implementer-standard, review: complex
Edits (every path is in the boundary; cite these lines against the working tree before editing — they are from main cb2c010):

R1. `src-tauri/src/engine/commands/task.rs:1582` — `["READY", "BLOCKED", "ESCALATION"]` → `["READY", "BLOCKED", "ESCALATION", "GATES-OK", "GATES-RED", "GATES-SKIPPED", "REVIEW-REROUTE"]` (keep the `.iter().any(starts_with)` shape). Doc comment L1578-1580: after `(decision 1)` append ` plus the ADR 0010 Amendment 4 protocol phrases — the Runner's gate re-run verdicts and the reviewer's re-route`; the lowercase / leading-space sentence stays.

R2. `src-tauri/src/engine/commands/task.rs` test `wakes_watchers_encodes_exactly_the_decision_1_list` (~L4240-4300): the `wake` array (~L4269) gains `"GATES-OK @abc1234"`, `"GATES-RED @abc1234 · 1 gate exit 1"`, `"GATES-SKIPPED slug @abc head is def"`, `"REVIEW-REROUTE slug @abc complex reason"`; the `quiet` array (~L4281) gains `"gates-ok @abc"`, `" GATES-OK @abc"`, `"Gates OK"`, `"RUN-DONE slug @abc"`, `"(Illenium) GATES-OK @abc"`. No new test fn; the comment `// note — exact-prefix, case-sensitive markers only.` stays.

R3. `src-tauri/skills/runner/SKILL.md:46-48` — after the sentence `That one line is the whole report for a re-run.` add, same bullet: ` It reaches your supervisor through task watch ONLY when ``GATES-`` is the first character of the note — no date, name or preamble before it. Do not also tell: the note is the delivery.` (two backticks shown here = one backtick pair in the file).

R4. `src-tauri/skills/coordinator/SKILL.md:55` — replace `and wait for that note (your task watch delivers it).` with `and wait for that note (your task watch delivers any note that starts with ``GATES-``). Ten minutes with no note → ``conclave task brief <ws> <slug>`` ONCE: a ``GATES-`` note already on the ledger is acted on as if it had just arrived; none → tell the Runner once more, then ``GATES-SKIPPED <slug> @<sha> runner silent`` and go on.` Nothing else in the file changes.

R5. `docs/adr/0010-coordinator-layer.md` — append `## Amendment 4 (2026-10-09) — Gate-note wake` (≤ 14 lines): root cause (one line: `GATES-*` / `REVIEW-REROUTE` were ledger-only under `note_wakes_watchers`, so Amendment 3 D7's "wait for the note" never fired; observed on brust and OmniChat 2026-10-08/09), D1–D5 one line each, plan pointer `docs/plans/2026-10-09-gate-note-wake.md` (task `gate-note-wake`), rollout line: engine + skills reach agents after rebuild + relaunch; the `tiering` skill reaches Detoro and Aitthi only via a Builder edit (ADR 0005 copy semantics).

Boundary: `src-tauri/src/engine/commands/task.rs`, `src-tauri/skills/runner/SKILL.md`, `src-tauri/skills/coordinator/SKILL.md`, `docs/adr/0010-coordinator-layer.md`.

Gates: `cargo test --manifest-path src-tauri/Cargo.toml --quiet wakes_watchers` · `cargo test --manifest-path src-tauri/Cargo.toml --quiet skill` · `cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check` · `cargo clippy --manifest-path src-tauri/Cargo.toml --quiet --all-targets -- -D warnings`.

READY evidence: SHA, the four gate ids exit 0, `changed:` / `why:` / `unsure:` lines (Amendment 3 D8), and the test's new wake/quiet strings quoted.

## Review routing for THIS task (ruled, because the fix is what the normal path depends on)
The Runner gate re-run step is SKIPPED for `gate-note-wake`: a Runner `GATES-OK` note on this task would not wake the Coordinator until this very change is live. On the implementer's READY the Coordinator posts `GATES-SKIPPED gate-note-wake @<sha> wake-gap (ruled)` and dispatches review straight to Mellow (`review: complex`); Mellow verifies the four gate rows on the ledger carry the READY sha before reading the diff.

## Task table
| slug | tier | role | deps | review | acceptance |
|---|---|---|---|---|---|
| gate-note-wake | standard | implementer-standard | — | complex | R1-R5; four gates green on the ledger at the READY sha; READY in Amendment 3 shape |

## Risk ledger
- `cargo clippy --all-targets` is the slow gate (minutes); run it last and once.
- The test's `quiet` strings guard the exact-prefix rule; a `contains`-style edit in R1 makes `"(Illenium) GATES-OK @abc"` wake and the test red — that is the point.
- R3/R4 edit bundled skill text: skills are read from the app bundle at launch (`skill.rs:403-420`), so nothing changes for live agents until rebuild + relaunch. Until then the Coordinator runs the OLD flow — hence the ruled skip above.
- The ADR amendment supersedes ADR 0010 L16-19's "only READY/BLOCKED/ESCALATION notes wake" engine fact; do not edit those historical lines, the amendment is the record.
