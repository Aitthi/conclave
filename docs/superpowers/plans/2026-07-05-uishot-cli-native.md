# Plan: uishot-cli-native

owner: bfb737ff-486d-4581-b407-95711d5e07ab (Detoro) · authority: in-loop
Spec: docs/superpowers/specs/2026-07-05-uishot-cli-native.md (read it first)
Workspace: 11ecf99b-53f4-4c24-b538-b19e5933a9e3 · escalations → Detoro via `task challenge`

## Global constraints (every lane inherits)

- Shared checkout: commit ONLY via `conclave stage commit <ws> <slug> -m "..."` (never plain
  `git commit` — it sweeps the shared index).
- Skill templates and CLI strings are English and workspace-agnostic (no codeup view ids,
  ports, or repo paths in `src-tauri/skills/` or Rust strings).
- Commit first, then `conclave task gate` — the gate pins HEAD at run time.
- Implementation judgment inside the plan's intent is yours; spec conflicts go to Detoro
  as a `task challenge` with evidence + default.

## Lane C — `conclave uishot` verb (task: uishot-verb)

Boundary: `src-tauri/src/bin/conclave-cli.rs`
(if tests must live in a separate file, challenge with the path — do not widen silently)

1. Read `run_task_gate` (conclave-cli.rs:1615) and the verb dispatch + help text
   (~line 105, 207-214). Mirror its structure exactly.
2. Add verb `uishot`:
   - Parse `conclave uishot [--task <slug>] <args...>`. No args after flags → usage
     error, exit 2 (match existing usage-error convention in this file).
   - Resolve capture contract: from cwd, walk to git root (`git rev-parse --show-toplevel`,
     same approach run_task_gate uses for the SHA), read `package.json`, require a
     `scripts.uishot` entry. Missing → stderr
     `conclave: no "uishot" script in package.json — this workspace has no UI capture contract`,
     exit 1.
   - Exec `pnpm run uishot -- <args...>` client-side, args verbatim (NO shell), inherit
     stdio so the agent sees output live; propagate the child's exit code.
   - `--task <slug>`: instead of a bare exec, route through the existing `run_task_gate`
     machinery with the composed command so the ledger entry is byte-identical in shape
     to a manual `task gate` (cmd, exit, sha, cwd, tail). Preserve its
     `CONCLAVE_INSTANCE_ID` requirement and error wording.
3. Help text: one row under the task/gate section:
   `uishot [--task <slug>] <args...>   (runs the workspace's package.json "uishot" script here; with --task also records it as a task gate)`
4. Unit tests beside the existing conclave-cli tests: resolution found/missing,
   usage error, --task precondition error.
5. Gates (run each, in order):
   - `conclave task gate <ws> uishot-verb -- cargo build --manifest-path src-tauri/Cargo.toml --bin conclave`
   - `conclave task gate <ws> uishot-verb -- cargo test --manifest-path src-tauri/Cargo.toml --bin conclave`
   - `conclave task gate <ws> uishot-verb -- sh -c "./src-tauri/target/debug/conclave uishot 2>&1 | grep -qi usage"`

## Lane S — skill templates (task: uishot-skill-templates)

Boundary: `src-tauri/skills/tool-map/SKILL.md`, `src-tauri/skills/implementer/SKILL.md`

1. `tool-map/SKILL.md`: add ONE row to the verb table (Family "Work items" area, near
   `task gate`):
   `| Work items | \`conclave uishot [--task <slug>] <args...>\` | run the workspace's UI capture script (package.json \`uishot\`) and SEE the result; \`--task\` records it as a task gate |`
   Match the table's escaping/voice exactly.
2. `implementer/SKILL.md`: add a short passage (3-6 lines, in the file's existing voice)
   under the verification/done-claim discipline: when the workspace defines a UI capture
   contract (`package.json` script `uishot`; details usually on bb key
   `protocol:ui-pixel-gate` and the repo's CLAUDE.md/AGENTS.md), a lane touching UI must,
   BEFORE claiming READY: run the capture for each affected view, OPEN each PNG with your
   image-capable file reader and look at it, attach the shot paths in the READY note, and
   record the run as a task gate. A green exit code without looking at the pixels is not
   verification.
3. Keep both files workspace-agnostic — no codeup view ids/ports/paths.
4. Gate:
   - `conclave task gate <ws> uishot-skill-templates -- sh -c "grep -q 'conclave uishot' src-tauri/skills/tool-map/SKILL.md && grep -qi 'pixel' src-tauri/skills/implementer/SKILL.md"`

## Risk ledger

- **conclave-cli flaky test:** a space-path regression test uses a fixed global temp path
  and flakes when agents run it concurrently. If exactly that test fails, rerun once
  before escalating.
- **Effect deferred to rebuild:** the installed binary and sidecars only change after the
  human rebuilds + relaunches the app. Lane gates prove code-level correctness; the
  post-relaunch checklist (Detoro) proves the live behavior. Do not try to replace the
  installed binary yourself.
- **pnpm absence:** if `pnpm` is not on PATH the exec fails — surface the OS error plus
  the attempted command; do not silently fall back to npm/npx.
- **Verb collision:** confirm `uishot` collides with no existing verb or prefix in the
  dispatch before wiring it (search the parser, not just the help text).
- **Lane C and Lane S are independent** (disjoint files); Lane S documents the verb Lane C
  builds — if Lane C's final flag/verb spelling changes, Lane C's implementer must note it
  on BOTH task ledgers so Lane S copies the real spelling.
