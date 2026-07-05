# uishot-cli-native — `conclave uishot` verb + UI Pixel Gate in skill templates

**Date:** 2026-07-05 · **Owner:** Detoro (bfb737ff) · **Authority:** in-loop
**Approved by human:** full scope, both layers ("ทำเต็มทั้งสองชั้น", 2026-07-05 ~23:00).

## Problem

The UI Pixel Gate (run uishot + open the PNG before READY) exists only in repo docs
(CLAUDE.md/AGENTS.md @ 607ae88), the blackboard, and workspace memory. The app-generated
skill sidecars — the layer every agent re-reads on every fresh context — cannot carry it,
because they are regenerated from `src-tauri/skills/*/SKILL.md` templates on each launch.
And the capture tool itself is a repo-local `pnpm uishot`, invisible in `conclave help`
and not integrated with the task gate ledger.

## Decision

Two layers, both landing in the app itself (effective on next app rebuild + relaunch):

1. **CLI verb** `conclave uishot <args...> [--task <slug>]` — a thin client-side wrapper:
   - Resolves the workspace's capture script by convention: a `package.json` script named
     `uishot` at the git root of the caller's cwd. Missing script → loud error naming the
     convention (`conclave: no "uishot" script in package.json — this workspace has no UI
     capture contract`), exit 1.
   - Runs it client-side (`pnpm run uishot -- <args...>`), forwarding args verbatim
     (no shell re-parse, same rule as `task gate`), streaming output, propagating the
     exit code. NEVER engine-side (ADR 0008).
   - `--task <slug>`: additionally records the run on that task's gate ledger via the
     existing `run_task_gate` path (identical wire format: cmd, exit, sha, cwd, tail).
     Requires `CONCLAVE_INSTANCE_ID` (spawned agent), same as `task gate`.
   - Help text gains one row.

2. **Skill templates** (`src-tauri/skills/`):
   - `tool-map/SKILL.md`: one new table row for `conclave uishot`.
   - `implementer/SKILL.md`: a short "UI pixel gate" passage — when the workspace defines
     a UI capture contract (`package.json` script `uishot` / bb key `protocol:ui-pixel-gate`),
     a lane touching UI must, before READY: run the capture per affected view, OPEN each
     PNG with the image-capable file reader and look at it, attach shot paths in the READY
     note, and record the run as a task gate (`conclave uishot --task <slug> <view>`).

## Constraints (global)

- **Templates stay workspace-agnostic.** No codeup view ids, ports, or paths in
  `src-tauri/skills/` — those live in the repo's CLAUDE.md/AGENTS.md. The templates ship
  to every workspace this app ever runs.
- The verb is a wrapper, not a reimplementation: no puppeteer/chrome logic in Rust.
- English only in templates and CLI strings (app copy is English).

## Rejected alternatives

- **Native Rust capture (CDP/puppeteer-rs):** duplicates a working 200-line node script,
  couples the platform binary to one project's rendering stack. Rejected.
- **Editing generated sidecars directly:** proven regenerated on every app relaunch
  (all 10 files mtime = relaunch time 22:49:49). Rejected as a write target.
- **Engine-side execution of the capture:** ADR 0008 already rules gates run caller-side;
  same reasoning applies (cwd, sandbox, env belong to the agent).

## Acceptance

- `conclave uishot` (built binary): usage error without args; loud convention error in a
  repo without an `uishot` script; propagates the wrapped script's exit code; `--task`
  outside a spawned agent errors exactly like `task gate` does.
- Unit tests cover: script resolution (found/missing), arg forwarding, `--task` gating
  precondition. `cargo build` + scoped `cargo test` green.
- Templates: `tool-map` row and `implementer` passage present, workspace-agnostic,
  consistent with the existing table/voice of each file.
- Real-binary verification (uishot verb visible in `conclave help`, gate recorded on a
  real task) happens after the human's next app rebuild + relaunch — post-relaunch
  checklist, not a lane gate.
