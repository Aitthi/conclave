# rtk Built-in Integration — Design

**Date:** 2026-07-09
**Owner:** Detoro (lead, 4fb2198c) · authority: in-loop
**Status:** Approved by human (design conversation, 2026-07-09)

## Goal

Bundle [rtk](https://github.com/rtk-ai/rtk) (Rust Token Killer — CLI proxy that
filters/compresses command output before it reaches the LLM, 60–90% token
savings) into the Conclave app so that **every claude-code agent Conclave
spawns gets transparent rtk command rewriting by default**, with a per-agent
opt-out. No manual `brew install rtk`, no `rtk init`, no jq dependency.

This extends the context-economy program (bb key
`plan:context-economy-pipeline`) from Conclave's own CLI surface to *all*
shell commands agents run.

## Decisions (settled with the human)

1. **Acquisition: pinned build-time install.** Build tooling runs
   `cargo install rtk --version <PIN> --locked` into a gitignored staging dir;
   the binary ships inside the .app. We do NOT vendor rtk source into this
   repo and do NOT depend on a user-installed rtk.
2. **Scope: claude-code agents only, default-ON, per-agent-definition toggle
   to disable.** Codex agents get nothing in this phase (only weak
   prompt-level guidance is possible there — rejected as not worth it now).
3. **Telemetry:** rtk telemetry is opt-in (disabled by default, verified in
   `src/core/telemetry.rs` — no URL compiled in → disabled). We never run
   `rtk init` and never enable it. No action needed.

## Architecture

### 1. Binary acquisition & bundling

- `RTK_VERSION` pinned in ONE place: `scripts/fetch-rtk.sh` (env-overridable
  `RTK_VERSION=0.42.4`).
- `scripts/fetch-rtk.sh`: `cargo install rtk --version $RTK_VERSION --locked
  --root <staging>` then copies the binary to
  `src-tauri/binaries/rtk-<target-triple>` (gitignored). Idempotent: skips
  when the staged binary already reports the pinned version.
- `src-tauri/tauri.conf.json`: `bundle.externalBin: ["binaries/rtk"]` —
  Tauri places the binary beside the app executable in `Contents/MacOS/` and
  code-signs it (same physical location as the existing `conclave-cli`
  sibling).
- `pnpm tauri build`/`dev` must run the fetch script first (package.json
  `pretauri`-style hook or documented manual step — implementer's choice,
  recorded in the plan).

### 2. Runtime resolution & PATH

- New resolver in `src-tauri/src/engine/agentctx.rs`, following the
  `current_exe()` wrapper + path-arg inner fn pattern (memory:
  `codeup-current-exe-bundle-path-untestable-from-cargo-test`):
  1. sibling of `current_exe()` named `rtk` (bundled .app case),
  2. dev fallback: `src-tauri/binaries/rtk-<triple>` under
     `CARGO_MANIFEST_DIR`,
  3. last resort: `which rtk`.
  Returns `Option<PathBuf>`; `None` = rtk unavailable → spawn proceeds
  WITHOUT the hook (never blocks agent spawn).
- `ensure_conclave_shim()` (`agentctx.rs:241`) additionally maintains symlink
  `<data_dir>/Conclave/bin/rtk` → resolved binary, using the existing
  `refresh_shim_link` atomic temp-name+rename pattern (`agentctx.rs:279`).
  Agents therefore see `rtk` on PATH (shim dir is already prepended at
  `instance.rs:707-713`).

### 3. PreToolUse hook (no jq)

- Upstream's `hooks/claude/rtk-rewrite.sh` requires `jq`; macOS does not ship
  it. Instead: new **hidden verb on `conclave-cli`**: `rtk-hook`.
- `conclave-cli rtk-hook --rtk <abs-rtk-path>`:
  - reads the PreToolUse JSON from stdin, extracts `.tool_input.command`
    (empty → exit 0, no output);
  - runs `<rtk> rewrite "<command>"`;
  - maps the upstream exit-code contract (rtk >= 0.23.0):
    - `0` + stdout ≠ original → emit `{"hookSpecificOutput":
      {"hookEventName":"PreToolUse","permissionDecision":"allow",
      "permissionDecisionReason":"RTK auto-rewrite","updatedInput":
      {...tool_input, command: rewritten}}}`;
    - `0` + stdout == original → exit 0 silently (already rtk-prefixed);
    - `1` (no equivalent) / `2` (deny rule — let native deny handle) → exit 0
      silently;
    - `3` + stdout → emit `updatedInput` WITHOUT `permissionDecision` (Claude
      Code prompts the user);
    - any other code / rtk spawn failure / malformed stdin → exit 0 silently
      (fail-open: never breaks the agent's Bash tool).
- Wiring: `claude_agent_settings` (`sandbox_config.rs:146`) adds, next to the
  existing SessionStart owner-marker hook, a `PreToolUse` hook with matcher
  `Bash` whose command is
  `"<abs conclave-cli> rtk-hook --rtk <abs rtk>"` (absolute paths embedded at
  settings-write time; hooks must not depend on the agent shell's PATH).
- Routing note: `rtk-hook` is a **conclave-cli local subcommand** (like
  argv expansion), NOT an engine/UDS verb — it must work even when the
  engine is busy or restarting. It therefore does NOT touch
  `engine/router.rs`/`engine/commands/cli.rs`; the engine-verb boundary
  checklist does not apply. It lives in `src-tauri/src/bin/conclave-cli.rs`
  (or a module it owns).

### 4. Per-agent toggle (default ON)

- Agent definitions gain boolean `rtk_enabled`, default `true`.
  - DB: migration adding `rtk_enabled INTEGER NOT NULL DEFAULT 1` to the
    agent-definition table (exact table/migration file pinned in the plan —
    boundary must name the DEFINING migration file, memory
    `lead-boundary-defining-file-not-importer`).
  - Engine: spawn path (`instance.rs` claude branch) consults
    `def.rtk_enabled`; `false` → skip PreToolUse hook injection (shim symlink
    may still exist; harmless).
  - CLI/engine surface: expose the field wherever agent defs are already
    read/written (agent get/update verbs) — additive, backward compatible.
- UI: checkbox in the agent Builder view ("Token filter (rtk)" — English UI
  copy per memory `conclave-ui-copy-english`), default checked. **UI Pixel
  Gate applies** (`pnpm uishot builder` + Read the PNG).

### 5. Agent awareness

- One sentence appended to the claude preamble (same mechanism as
  `conclave_path_sentence`, `instance.rs:604-608`): commands are transparently
  rewritten through `rtk` to save tokens; do not prefix commands with `rtk`
  manually; if full unfiltered output is ever required, re-run the exact
  command with `RTK_FORCE=1 ` prefix or ask the lead to disable rtk for this
  agent. (Exact escape-hatch wording verified against the pinned rtk version's
  actual pass-through mechanism during implementation; if rtk offers no such
  env, the sentence names the toggle only.)

## Failure modes & guardrails

| Risk | Guard |
|---|---|
| rtk binary missing/unresolvable | Fail-open: spawn without hook; log a warning line to engine log |
| rtk crashes or hangs in hook | `rtk rewrite` is <10ms; hook wraps spawn errors → exit 0 pass-through. Claude Code's own hook timeout is the backstop |
| Over-filtered output breaks a workflow | Per-agent toggle OFF; preamble names the escape hatch |
| Upstream contract drift | Version pinned; `rtk-hook` verb tolerates unknown exit codes by passing through |
| Windows/non-unix | All wiring behind existing unix guards (shim/symlink code already unix-only) |

## Testing

1. **Unit (Rust):** `rtk-hook` JSON protocol — fixture stdin × mocked rtk
   exit codes {0-rewritten, 0-identical, 1, 2, 3, 127} → exact expected
   stdout/exit. Mock rtk = tiny shell script fixture, no real rtk needed.
2. **Unit (Rust):** `claude_agent_settings` includes the PreToolUse hook when
   `rtk_enabled=true` + rtk resolvable; omits it when toggled off or
   unresolvable.
3. **Gates:** `cargo test` + `cargo fmt --check` + `cargo clippy` (toolchain
   pin 1.96.0); `pnpm uishot builder` for the UI lane.
4. **Live verification (integration, by lead):** spawn a real claude agent,
   run `git status` inside it, confirm the executed command is
   `rtk git status` and output is the compact rtk form.

## Out of scope (recorded rejections)

- Vendoring rtk source into this repo (maintenance burden; upstream active).
- Codex-agent support (prompt-level only, weak — revisit if rtk grows a
  codex hook).
- Enabling rtk telemetry.
- Rewriting non-Bash tools (Read/Grep are native Claude Code tools, not shell).
