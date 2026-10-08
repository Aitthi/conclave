# Haiku 5.5 preset, Runner role for Haiku-class chores, model name on the org chart
owner: 30fa04f4-e047-4241-a9ed-f452529952be · authority: in-loop · coordinator: Aitthi (7d560bae-09dc-4535-befb-feed5a95aa05) · escalation: challenges route to the task owner (Detoro); READY / BLOCKED / stall / review verdicts / dispatch are the Coordinator's.
Reviewer: Mellow (b3a30e7b-5a9f-4d4d-a83e-768a9326632f). Detoro rules and merges on `MERGE-READY <slug> @<sha>`. Start every lane from main at or after 082c01f. Three INDEPENDENT lanes — disjoint boundaries, no deps between them.

## Why now
Human request 2026-10-08 (three asks, one plan):
1. Add `claude-haiku-5-5` to the Claude model presets. Verified by Detoro: Claude Code 2.1.294 accepts it (`claude -p --model claude-haiku-5-5` → `modelUsage` key `claude-haiku-5-5`, cost $0.004).
2. A new POSITION (role) for a Haiku-class agent that takes the small chores — run lint/format, run the test suite and report, summarise logs, check gate output — so those tokens stop going to Opus/Sonnet. Under ADR 0010 this is a THIRD dispatch tier below `routine`.
3. Show the model name on the supervisor-chain (Org) card, next to level · role (screenshot: `Prin · Lead` should read `Prin · Lead · fable-5-1`).

The separate bug "new agent shows running but never starts" is its own task (`add-agent-spawn-ghost`, separate plan) — not in this file.

## Global constraints (every lane inherits)
- Record each gate with `conclave task gate 11ecf99b-53f4-4c24-b538-b19e5933a9e3 <slug> -- <cmd>` (words after `--` unquoted). A gate narrated in a note does not count.
- Line numbers are from main 082c01f; anchor on the quoted text if they shift.
- One commit per lane scoped to the boundary (`git commit -- <paths>` or `conclave stage commit`). `pnpm install` once in a fresh worktree before any pnpm gate.
- Report READY to Aitthi in the structured shape (`task:` / `status:` / `files:` / `note:` with SHA, gate ids, test names). Do not touch files outside your boundary — challenge instead.

---

## Task 1 — `model-haiku-5-5` (tier routine, role implementer-routine)

R1. `src-tauri/src/engine/commands/draft.rs` `CLAUDE_MODELS` (~L36-44): insert `"claude-haiku-5-5",` directly AFTER `"claude-sonnet-5-5",` (newest-first within the 5.5 family: opus, sonnet, haiku). Extend the doc comment above it with: `Human request 2026-10-08: add Haiku 5.5 (\`claude-haiku-5-5\`) after Sonnet 5.5.` Keep every existing id (including `claude-haiku-4-5` and `claude-opus-4-8`) so stored rows stay valid.
R2. `src/lib/modelCatalogue.ts` `CLAUDE_MODELS` (~L12-19): same insertion, same position. The Rust test `claude_models_mirror_typescript` (draft.rs ~L985-1010) parses this file and asserts byte-equality of the two lists — it is the guard.
R3. Tests in that same test fn, mirroring the 2026-10-01 shape (commit ab91501): `assert_eq!(CLAUDE_MODELS.get(3).copied(), Some("claude-haiku-5-5"));` and a `haiku` agent through `validate_draft` expecting Ok. No new test fn.
R4. Nothing else: `shortModel` (`src/lib/providerLabel.ts:59`) already renders it as `haiku-5-5`; the `[1m]` suffix is a per-definition choice (`context_window`), not per model. `rg -n "claude-haiku-4-5" src src-tauri/src` must show only the two catalogues + tests + `providerLabel.ts` doc comment; if any other allowlist appears, add the new id there in the same shape and say so in the READY note.

Boundary: `src-tauri/src/engine/commands/draft.rs`, `src/lib/modelCatalogue.ts`.
Gates: `cargo test --manifest-path src-tauri/Cargo.toml --quiet draft` · `cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check` · `cargo clippy --manifest-path src-tauri/Cargo.toml --quiet --all-targets -- -D warnings` · `pnpm exec tsc --noEmit`. No UI pixels change: no uishot.

---

## Task 2 — `role-runner` (tier routine, role implementer-routine)

Decision (Detoro, final): the new role id is `runner`, name `Runner`, bundled skill `runner` (new, compact — NOT `implementer`, whose 183 lines are wasted on a Haiku context). Dispatch tier name is `trivial`. Rejected: `implementer-trivial` (it does not implement; it runs and reports) and reusing the implementer skill (too heavy, wrong instructions). The model is NOT pinned by the role — the human (or Detoro via `agentDef.save`) creates an agent definition with `model: claude-haiku-5-5` on this role after the rebuild.

R5. New `src-tauri/roles/runner/ROLE.md`, frontmatter keys exactly `name` / `description` / `skills` (parser `role.rs:243-275` reads only those). Body two sentences like `implementer-routine/ROLE.md`. Description (verbatim, one line):
`You are a runner: the agent that takes the mechanical chores a plan tags tier trivial so the implementers and reviewers keep their context for judgment. You run exactly the command a task names — lint, format, test suites, build checks, gate commands — and report the exit code, the log path, and a five-line summary of what failed and where; you summarise logs and transcripts into the facts a reader needs; you never change program logic, never decide a design question, and never widen a task — if the command you ran shows a defect, post it as a task note with the evidence and stop. A formatter run is the only edit you make, and only when the task says so. You claim a task before you touch it and never claim done on a command you did not run and watch finish. All human-facing terminal output must be in Thai or English only (technical terms, code identifiers, paths, and commands stay as-is); inter-agent messages stay in English.`
R6. New `src-tauri/skills/runner/SKILL.md` (frontmatter `name: Runner`, `description: How to take a trivial-tier chore — run the named command, record it as a gate, report exit code + log path + five-line summary, summarise logs on request, never edit logic. Use whenever a runner task or a summarise/verify request reaches you.`, `mandatory: false`). Body ≤ 60 lines, sections: *What you take* (lint, format, test/build runs, gate re-runs, log/transcript summaries, "did the tests pass?" checks), *How you run* (claim → `conclave task gate <ws> <slug> -- <cmd>` so the ledger holds the log, never narrate a result; bounded reads only — `tail`, `rg`, the gate's `logPath`), *How you report* (note shape: `RUN <cmd>` / `exit <code>` / `log <path>` / up to five lines of failures `file:line — message`; a summary is ≤ 10 lines of facts, no advice), *What you never do* (edit logic, pick between designs, claim complex/routine tasks, retry a failing command with a "fix"), *Escalation* (your supervisor only; a defect is a task note with evidence, not a fix). Composes with Collaboration and Comms Protocol.
R7. `src-tauri/skills/coordinator/SKILL.md` "Watch everything, dispatch by the table" (~L31-38): tier set becomes `` (`complex` | `routine` | `trivial`) ``; add after the "Tier is a hint" sentence: `A trivial task goes to an idle Runner; a Routine implementer may take it when no Runner is idle; a Runner never takes a routine or complex task.`
R8. `src-tauri/src/engine/commands/draft_prompt.rs` `RULES_TEAM` (~L11): after `…and then supervises them;` add ` a runner (role runner, a Haiku-class model such as claude-haiku-5-5) may take lint/format/test runs and log summaries so the implementers keep their context;`. Keep the rest byte-identical.
R9. `src-tauri/src/engine/repo/role.rs` builtin test (~L486-495 list + ~L524-533 tiered block): add `"runner"` to the expected ids and a block asserting `runner` ships with `name == "Runner"` and `skill_ids` containing `"runner"`.
R10. `src/fixtures/scenarios/data.ts` `roles` (~L887-907): add `{ id: "runner", name: "Runner", description: "Lint, format, test runs and log summaries on a Haiku-class model.", skillIds: ["runner", "collaboration"], kind: "builtin" }` after `implementer-routine`.
R11. `docs/adr/0010-coordinator-layer.md`: append `## Amendment 2 (2026-10-08) — trivial tier and the Runner role` (≤ 15 lines): third tier `trivial` → role `runner` on a Haiku-class model; what it takes and never does (two lines); plan pointer to this file; the model lives on the agent definition, not the role.
R12. Pins: `rg -n '"coordinator"' src-tauri/src src --glob '!*.snap'` — every hit that is an allowlist or pin of builtin role/skill ids (not the router) gets a `runner` sibling in the same shape; list each in the READY note (Detoro expects only role.rs tests and data.ts).

Boundary: `src-tauri/roles/runner/ROLE.md`, `src-tauri/skills/runner/SKILL.md`, `src-tauri/skills/coordinator/SKILL.md`, `src-tauri/src/engine/commands/draft_prompt.rs`, `src-tauri/src/engine/repo/role.rs`, `src/fixtures/scenarios/data.ts`, `docs/adr/0010-coordinator-layer.md`.
Gates: `cargo test --manifest-path src-tauri/Cargo.toml --quiet role` · `cargo test --manifest-path src-tauri/Cargo.toml --quiet draft` · `cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check` · `cargo clippy --manifest-path src-tauri/Cargo.toml --quiet --all-targets -- -D warnings` · `pnpm exec tsc --noEmit` · `pnpm uishot builder` (the Builder lists roles from the fixture — OPEN the PNG and confirm `Runner` appears in the role picker; attach the path).

---

## Task 3 — `orgchart-model-name` (tier routine, role implementer-routine, UI)

Canon: none exists for the Org pane (`design/screens/` has no org-chart proto); this is an additive text slot inside the existing row, so no designer gate — Arta is the escalation target only if layout breaks at the narrow width.

R13. `src/components/LaneBoard.tsx` `OrgNodeRow` (~L797-835): after the `<PositionLine …/>`'s track, show the model. Implementation: extend `PositionLine` is OUT of boundary — instead render the model as a sibling on the same line: wrap the existing `<PositionLine … className="mt-0.5" />` and a new `<span className="font-mono truncate" title={agent.model} style={{ fontSize: "0.62rem", color: FAINT }}>· {shortModel(agent.model)}</span>` in `<div className="mt-0.5 flex items-center gap-1 min-w-0">` (drop `className="mt-0.5"` from PositionLine). Render the span only when `shortModel(agent.model)` is non-null. Import `shortModel` from `../lib/providerLabel`.
R14. The second chain card (`OrgChartPane` detail header, ~L1098-1108, the `PositionLine` under `{human ? … : …}`) gets the same treatment when `agent` is set, so the selected node and the row agree.
R15. Fixture rows already carry `model` (`src/fixtures/scenarios/data.ts:338` etc.) — no fixture change. The Org pane is in-component state (`viewMode`, L183), not URL-routed: `pnpm uishot laneboard` shows the board only. Pixel gate for this lane: `pnpm uishot laneboard` (board must be unchanged) PLUS a capture of the Org pane via `conclave browser open http://localhost:1420/?fixture=default#view=laneboard` → `conclave browser click` on the `Org` toggle (L373) → `conclave browser screenshot`; OPEN both PNGs and attach the paths. Expected: every row reads `<Lvl> · <Role> · <model>` with the model truncating, never wrapping, at 2880x1800 and at the narrow Builder width (`--viewport 1440x1900`).

Boundary: `src/components/LaneBoard.tsx`.
Gates: `pnpm exec tsc --noEmit` · `pnpm uishot laneboard` · the Org-pane capture above recorded via `conclave task gate … -- conclave browser screenshot …` (or attach the screenshot path with the gate id of the `uishot laneboard` run and say which command produced it).

---

## Task table
| slug | tier | role | deps | acceptance |
|---|---|---|---|---|
| model-haiku-5-5 | routine | implementer-routine | — | R1-R4; four gates green on the ledger; READY with SHA + R4 outcome |
| role-runner | routine | implementer-routine | — | R5-R12; six gates green; READY with SHA, R12 list, builder shot path |
| orgchart-model-name | routine | implementer-routine | — | R13-R15; gates green; READY with SHA + board shot + Org-pane shot paths |

## Risk ledger
- Task 1: `claude_models_mirror_typescript` reads `modelCatalogue.ts` by relative path — run `cargo test` from the worktree root, not from `src-tauri/`, if it cannot find the file (check how the test resolves the path before moving).
- Task 2: a `roles/<id>` folder with a malformed frontmatter is SILENTLY skipped in production (`role.rs:211-222`) — the role.rs test `every shipped ROLE.md must parse` is the guard; keep `description:` on ONE line.
- Task 2: the live app only picks up new ROLE.md/SKILL.md after rebuild + relaunch (bundled Resources). Not a lane concern; Detoro reports it to the human.
- Task 3: `FAINT` is a module const in LaneBoard.tsx (~L60s); `shortModel` returns null for blank — guard the separator too, or the row shows a dangling `·`.
- All: `pnpm uishot` reuses a stale vite server on :1420 from another worktree — `lsof -nP -iTCP:1420 -sTCP:LISTEN` first (CLAUDE.md caveat).

## Done means
Each lane: READY note on its task (shape above), Mellow `READY REVIEW-PASS @<sha>`, Aitthi `MERGE-READY <slug> @<sha>`, Detoro merges and `lane finish`. After all three merge: app rebuild + relaunch (human), then create the Haiku agent definition on role `runner` with `model: claude-haiku-5-5` and add it under Aitthi.
