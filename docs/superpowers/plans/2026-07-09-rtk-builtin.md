# rtk Built-in Integration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bundle the rtk token-filter binary into the Conclave app and transparently rewrite every claude-code agent's Bash commands through it, default-on with a per-agent-definition toggle.

**Architecture:** Three independent lanes. Lane A fetches/bundles rtk (Tauri externalBin), resolves it at runtime, maintains a shim symlink, adds the PreToolUse hook to per-instance settings, and adds the `rtk_enabled` column end-to-end in the engine. Lane B adds a local (no-UDS) `rtk-hook` subcommand to conclave-cli implementing the PreToolUse JSON protocol. Lane C adds the Builder UI toggle. A↔B meet only at a string contract; C meets A only at the `rtkEnabled` camelCase field name.

**Tech Stack:** Rust (sqlx+sqlite chain-builder, portable-pty, serde_json), Tauri v2 externalBin, React/TS (existing `Toggle` component), bash fetch script.

**Spec:** `docs/superpowers/specs/2026-07-09-rtk-builtin-design.md` (read it first).

## Global Constraints (every task inherits these)

- `RTK_VERSION` pin: **0.42.4**, defined ONLY in `scripts/fetch-rtk.sh` (env-overridable). rtk exit-code contract requires rtk >= 0.23.0.
- **Fail-open everywhere:** rtk missing, hook errors, malformed stdin, unknown exit codes — the agent spawn and the agent's Bash tool must NEVER break. Silent exit 0 / skip hook.
- Preamble sentences: single line, no `=` character (ADR 0001), pass through the existing `sanitize_field` path in `agentctx.rs`.
- UI copy is **English** (workspace rule).
- New bool DB column follows house style: nullable `INTEGER`, `NULL` means default (= ON here). Migrations register in **BOTH** `db.rs` lists: the `if version < N` block (~`db.rs:72-207`) AND the in-memory test list (~`db.rs:263-273`).
- Toolchain pin 1.96.0; run `cargo fmt` on new Rust before gating. Gates per Rust lane: `cargo test`, `cargo fmt --check`, `cargo clippy`. UI lane additionally: `pnpm build` and the UI Pixel Gate (`pnpm uishot builder` + actually Read the PNG).
- Commit via `conclave stage commit` (boundary-scoped private index), one logical change per commit.
- Hook command string contract (Lane A writes it, Lane B implements it):
  `'<data_dir>/Conclave/bin/conclave' rtk-hook --rtk '<data_dir>/Conclave/bin/rtk'` (absolute shim paths, single-quoted).
- Field name contract (Lane A ⇄ Lane C): DB `rtk_enabled`, wire/JSON `rtkEnabled` (camelCase via existing serde rename), `null`/absent = enabled.

---

## Lane A — bundle, resolve, hook wiring, DB field (slug: `rtk-bundle-wiring`)

**Boundary:** `scripts/fetch-rtk.sh`, `src-tauri/tauri.conf.json`, `.gitignore`, `src-tauri/src/engine/agentctx.rs`, `src-tauri/src/engine/runtime/sandbox_config.rs`, `src-tauri/src/engine/commands/instance.rs`, `src-tauri/src/engine/migrations/0017_agent_rtk_enabled.sql`, `src-tauri/src/engine/db.rs`, `src-tauri/src/engine/repo/agent_definition.rs`, `src-tauri/src/engine/commands/agent.rs`

**Boundary amendment (Detoro ruling on challenge 5432acf7, credit: Tiësto):** plus `src-tauri/build.rs` and `package.json`. tauri-build validates every `externalBin` entry EAGERLY on plain `cargo check/test/clippy` (reproduced: removing the staged binary fails cargo check with "resource path binaries/rtk-<triple> doesn't exist"). Per the immutable-boundary convention these two files land as separate scoped raw `git commit -- <path>` commits, not via `stage commit`.

### Task A1: fetch script + bundle config

**Files:**
- Create: `scripts/fetch-rtk.sh`
- Modify: `src-tauri/tauri.conf.json` (bundle section, ~line 29-55)
- Modify: `.gitignore`

**Interfaces:**
- Produces: staged binary at `src-tauri/binaries/rtk-<host-triple>`; bundled app ships `rtk` beside the app executable (`Contents/MacOS/rtk`).

- [ ] **Step 1: Write `scripts/fetch-rtk.sh`**

```bash
#!/usr/bin/env bash
# Fetches the pinned rtk binary for bundling. Idempotent.
set -euo pipefail
RTK_VERSION="${RTK_VERSION:-0.42.4}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
DEST="$ROOT/src-tauri/binaries/rtk-$TRIPLE"

if [ -x "$DEST" ] && "$DEST" --version 2>/dev/null | grep -q "$RTK_VERSION"; then
  echo "rtk $RTK_VERSION already staged at $DEST"
  exit 0
fi

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
# --git + tag guarantees we build the rtk-ai project, not a name-squatted crate.
cargo install --git https://github.com/rtk-ai/rtk --tag "v$RTK_VERSION" --locked --root "$STAGE" rtk
mkdir -p "$ROOT/src-tauri/binaries"
cp "$STAGE/bin/rtk" "$DEST"
chmod 755 "$DEST"
echo "staged rtk $RTK_VERSION -> $DEST"
```

- [ ] **Step 2: Run it and verify**

Run: `bash scripts/fetch-rtk.sh && src-tauri/binaries/rtk-$(rustc -vV | sed -n 's/^host: //p') --version`
Expected: `rtk 0.42.4` (build takes a few minutes the first time). If the tag name differs (check `git ls-remote --tags https://github.com/rtk-ai/rtk | grep 0.42.4` — release-please may use `rtk-v0.42.4`), fix the script's `--tag` accordingly and note it on the task.

- [ ] **Step 3: tauri.conf.json + .gitignore**

In `src-tauri/tauri.conf.json` `bundle` object (which currently has NO `externalBin`), add:

```json
"externalBin": ["binaries/rtk"],
```

Append to `.gitignore`:

```
src-tauri/binaries/
```

- [ ] **Step 4: placeholder so plain cargo builds stay green** (ruling 5432acf7)

tauri-build validates every `externalBin` eagerly on plain `cargo check` — a tree
that never ran `fetch-rtk.sh` must still compile. In `src-tauri/build.rs`, BEFORE
`tauri_build::build()`: compute `binaries/rtk-<host-triple>` (host triple from the
`TARGET` env var cargo sets for build scripts) and create it zero-byte
(`std::fs::File::create`) when absent, `mkdir -p` included. Zero-size is the
"placeholder" sentinel — A2's resolver treats zero-size files as unresolvable.

In `package.json`, chain the fetch into the tauri script so real dev/build always
stages the real binary first: `"tauri": "bash scripts/fetch-rtk.sh && tauri"`.

Verify: `rm -f src-tauri/binaries/rtk-*` then `cargo check` (from src-tauri) →
must PASS, and the placeholder file exists with size 0.

- [ ] **Step 5: Commit.** Boundary note: `src-tauri/build.rs` and `package.json`
are post-ruling additions OUTSIDE the original task boundary — land each as its own
scoped raw commit (`git commit -- src-tauri/build.rs`, `git commit -- package.json`),
not via `stage commit`; everything else commits normally (`git status` must show
binaries/ ignored).

### Task A2: runtime resolver + shim symlink

**Files:**
- Modify: `src-tauri/src/engine/agentctx.rs` (near `ensure_conclave_shim` at :241 and `refresh_shim_link` at :279)
- Test: same file `#[cfg(test)]` module (existing tests at :766+)

**Interfaces:**
- Produces: `pub fn resolve_rtk_bin() -> Option<PathBuf>` (thin `current_exe()` wrapper) + testable inner `fn resolve_rtk_bin_from(exe_dir: &Path, dev_binaries_dir: &Path, path_var: Option<&std::ffi::OsStr>) -> Option<PathBuf>`; `ensure_conclave_shim()` now also maintains `<data_dir>/Conclave/bin/rtk` symlink.

- [ ] **Step 1: Failing tests** — in the existing `#[cfg(test)]` module: (a) `resolve_rtk_bin_from` returns the sibling `rtk` when present in a tempdir exe_dir; (b) falls back to a `rtk-*`-named file in the dev binaries dir when no sibling; (c) returns None when neither exists and PATH lookup is skipped (pass an empty PATH via the inner fn taking `path_var: Option<&OsStr>`); (d) **zero-size files are unresolvable** (ruling 5432acf7): a 0-byte sibling `rtk` AND a 0-byte dev `rtk-<triple>` placeholder → `None` — the build.rs placeholder must never become a shim link. Follow the memory pattern: current_exe() wrapper is untestable — test the inner fn only.
- [ ] **Step 2: Verify FAIL** (`cargo test -p conclave resolve_rtk` — adjust package name to the actual `src-tauri` crate name, see Cargo.toml).
- [ ] **Step 3: Implement.** Resolution order: (1) `exe_dir.join("rtk")` if `is_file()`; (2) first `is_file()` entry named `rtk-*` in `dev_binaries_dir` (wrapper passes `Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries")`); (3) scan `std::env::split_paths(path_var)` for an executable `rtk`. In `ensure_conclave_shim()`, after the conclave link, `if let Some(rtk) = resolve_rtk_bin() { refresh_shim_link(...) }` reusing the existing atomic pattern (generalize `refresh_shim_link` to take the link name if it currently hardcodes `conclave`). rtk unresolvable = fine, shim just lacks the link.
- [ ] **Step 4: Tests PASS, fmt, commit.**

### Task A3: PreToolUse hook in settings

**Files:**
- Modify: `src-tauri/src/engine/runtime/sandbox_config.rs` (`claude_agent_settings` :146, `write_claude_settings` :213)
- Test: same-file tests (mirror `agent_settings_preserve_foreign_session_start_hooks` :323 and `claude_settings_fresh_has_route_a_keys` :355)

**Interfaces:**
- Consumes: shim paths from A2.
- Produces: `pub struct RtkHook { pub cli_bin: PathBuf, pub rtk_bin: PathBuf }`; new signatures `claude_agent_settings(instance_id, socket_path, existing, rtk: Option<&RtkHook>)`, `write_claude_settings(instance_id, socket_path, rtk: Option<&RtkHook>)`.

- [ ] **Step 1: Failing tests**: (a) with `Some(RtkHook)`, output JSON has `hooks.PreToolUse[0].matcher == "Bash"` and `hooks[0].command == "'<cli>' rtk-hook --rtk '<rtk>'"`; (b) with `None`, no `PreToolUse` key added and an existing FOREIGN PreToolUse entry in `existing` is preserved untouched; (c) re-running with `Some` replaces a prior conclave rtk group (identified by `"rtk-hook"` substring in command) instead of duplicating — mirror the SessionStart owner-marker merge logic.
- [ ] **Step 2: FAIL, Step 3: implement (mirror the SessionStart merge branch), Step 4: PASS + fmt + commit.**

### Task A4: DB column `rtk_enabled` end-to-end

**Files:**
- Create: `src-tauri/src/engine/migrations/0017_agent_rtk_enabled.sql`
- Modify: `src-tauri/src/engine/db.rs` (BOTH registration lists)
- Modify: `src-tauri/src/engine/repo/agent_definition.rs` (`AgentDefRow` :60, `COLS` :180, `AgentDefinitionInput` :211, `create` :307, `update` :456, and the hardcoded col list in `list_with_counts` :265)
- Modify: `src-tauri/src/engine/commands/agent.rs` (`save` :161 — accept optional `rtkEnabled` bool in payload)
- Test: existing roundtrip tests in `agent_definition.rs` `#[cfg(test)]` (:555+)

**Interfaces:**
- Produces: `AgentDefRow.rtk_enabled: Option<bool>` serialized as `rtkEnabled`; `agentDef.save` accepts `rtkEnabled?: bool`. Semantics: `None` OR `Some(true)` = enabled; `Some(false)` = disabled.

- [ ] **Step 1:** Migration SQL: `ALTER TABLE agent_definition ADD COLUMN rtk_enabled INTEGER;` — register in db.rs `if version < 17 { ... }` AND the in-memory list.
- [ ] **Step 2: Failing test** — extend the existing create/update roundtrip test: save with `rtk_enabled: Some(false)`, read back `Some(false)`; save without → `None`; assert camelCase key `rtkEnabled` in serialized JSON (mirror the existing camelCase contract test).
- [ ] **Step 3: FAIL → implement** (struct field, COLS, both col lists, Bind::Bool/Null in create/update, payload read in agent.rs save). **→ PASS + fmt + commit.**

### Task A5: spawn wiring + preamble sentence

**Files:**
- Modify: `src-tauri/src/engine/commands/instance.rs` (claude branch :629-662, preamble assembly :590-610)
- Modify: `src-tauri/src/engine/agentctx.rs` (new `pub fn rtk_awareness_sentence() -> String` next to `conclave_path_sentence` :377; test beside the existing sentence tests :766+)

**Interfaces:**
- Consumes: A2 `resolve_rtk_bin` (presence check), A3 `RtkHook` + new `write_claude_settings` signature, A4 `def.rtk_enabled`.

- [ ] **Step 1: Failing test** for `rtk_awareness_sentence()`: single line, contains no `=`, mentions `rtk` (mirror existing sentence tests). Wording (verify first whether the pinned rtk documents a bypass env by grepping its README/src in a scratch clone; if none exists, use this toggle-only wording):
  "Your shell commands may be transparently rewritten through the rtk token filter to keep output compact; never prefix commands with rtk yourself, and if you truly need full unfiltered output, ask your lead to disable the rtk toggle for this agent."
- [ ] **Step 2: FAIL → implement sentence → PASS.**
- [ ] **Step 3: Wire spawn:** in the `base == "claude"` branch: `let rtk_on = def.rtk_enabled.unwrap_or(true);` build `Option<RtkHook>` from the shim dir paths (`<bin>/conclave`, `<bin>/rtk`) only when `rtk_on` AND both shim links exist; pass it to `write_claude_settings(&id, socket_path.as_deref(), rtk.as_ref())`; append `rtk_awareness_sentence()` to the preamble ONLY when the hook was actually installed (same append style as `conclave_path_sentence` at :604-610).
- [ ] **Step 4: Full lane gates** (`cargo test`, `cargo fmt --check`, `cargo clippy`), record via `conclave task gate`, commit, READY note.

---

## Lane B — `rtk-hook` subcommand in conclave-cli (slug: `rtk-hook-verb`)

**Boundary:** `src-tauri/src/bin/conclave-cli.rs` only.

**Interfaces:**
- Consumes (contract, not code): invoked as `conclave rtk-hook --rtk <abs-path>`; PreToolUse JSON on stdin.
- Produces: stdout JSON per protocol below; ALWAYS exits 0.

### Task B1: protocol pure functions

- [ ] **Step 1: Failing tests** in conclave-cli's inline `#[cfg(test)]` module for two pure fns:

```rust
fn extract_bash_command(input: &serde_json::Value) -> Option<String>  // .tool_input.command, non-empty
fn rtk_hook_response(input: &serde_json::Value, exit_code: i32, stdout: &str) -> Option<serde_json::Value>
```

Cases for `rtk_hook_response` (original command `"git status"` in `tool_input`):
1. exit 0, stdout `"rtk git status"` → `Some` JSON with `hookSpecificOutput.hookEventName == "PreToolUse"`, `permissionDecision == "allow"`, `permissionDecisionReason == "RTK auto-rewrite"`, `updatedInput.command == "rtk git status"` and all other `tool_input` keys preserved.
2. exit 0, stdout identical to original → `None`.
3. exit 1 → `None`. 4. exit 2 → `None`.
5. exit 3, stdout `"rtk git status"` → `Some` with `updatedInput` set and NO `permissionDecision` key.
6. exit 127 (or any other) → `None`.
Plus `extract_bash_command`: missing `tool_input`/`command`/empty string → `None`.

- [ ] **Step 2: FAIL → Step 3: implement both fns (trim trailing newline from stdout before compare/emit) → Step 4: PASS + commit.**

### Task B2: subcommand wiring

- [ ] **Step 1:** In `main()`'s dispatch (before the UDS round-trip section, alongside the other LOCAL commands like `lane`/`stage` at :2847-2877), add: `if argv[0] == "rtk-hook"` → `run_rtk_hook(&argv[1..])`: parse `--rtk <path>` (missing → exit 0 silently), read stdin to string, `serde_json::from_str` (error → exit 0), `extract_bash_command` (None → exit 0), spawn `Command::new(rtk).arg("rewrite").arg(&cmd)` capturing stdout (spawn error → exit 0), call `rtk_hook_response`, print the JSON compact if `Some`. Always `ExitCode::SUCCESS`. Do NOT touch the engine socket. Add `rtk-hook` to the USAGE text's local-commands section.
- [ ] **Step 2: Manual verification** (expected output shown):

```bash
cargo build --bin conclave-cli
printf '{"tool_input":{"command":"git status"}}' | \
  ./target/debug/conclave-cli rtk-hook --rtk "$(command -v cat)" ; echo "exit=$?"
# cat ignores args and echoes nothing -> rtk "crashes" the contract -> silent pass-through: no output, exit=0
```

Then with the real staged rtk (if Lane A's A1 has landed) or any local rtk:

```bash
printf '{"tool_input":{"command":"git status"}}' | \
  ./target/debug/conclave-cli rtk-hook --rtk <path-to-rtk>
# -> {"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow",...,"updatedInput":{"command":"rtk git status"}}}
```

- [ ] **Step 3: Lane gates + READY.**

---

## Lane C — Builder UI toggle (slug: `rtk-toggle-ui`)

**Boundary:** `src/components/Builder.tsx`, `src/ipc/commands.ts`, `src/fixtures/scenarios/data.ts`

**Boundary amendment (Detoro ruling on challenge a44bd6ec, credit: Dew):** plus `src/ipc/types.ts` — `AgentDefinition` is DEFINED at `src/ipc/types.ts:37` and merely imported by the three boundary files; without it the `rtkEnabled` field cannot typecheck. One additive line (`rtkEnabled?: boolean | null;`), landed as a separate scoped raw commit (`git commit -- src/ipc/types.ts`) per the immutable-boundary convention. Same defect class as ruling 5432acf7's sibling (defining file vs importer).

**Interfaces:**
- Consumes: field name `rtkEnabled?: boolean | null` (absent/null = ON) on agentDef save req and def objects (Lane A4 defines the engine side; the wire name is fixed by this plan — no need to wait for A).

### Task C1: toggle end-to-end in UI

- [ ] **Step 1:** `src/ipc/types.ts`: add `rtkEnabled?: boolean | null;` to `export interface AgentDefinition` (:37) — commit this one line as its own scoped raw commit. Then `src/ipc/commands.ts`: add `rtkEnabled?: boolean` to the `agentDef.save` request type (:63-96).
- [ ] **Step 2:** `src/components/Builder.tsx`: state `const [rtkEnabled, setRtkEnabled] = useState<boolean>(initialDef?.rtkEnabled ?? true);` (reset alongside the other initialDef effects); reuse the in-file `Toggle` component (:198-212) — label **"Token filter (rtk)"**, helper text **"Rewrites shell commands through rtk to compress output and save tokens. Claude agents only."** Render it with the other CLI-config toggles, only when the definition's cli kind is claude. Include `rtkEnabled` in the `ipc.agentDef.save({...})` payload (:512-536).
- [ ] **Step 3:** `src/fixtures/scenarios/data.ts`: add `rtkEnabled: true` to one claude agent def and `rtkEnabled: false` to another (so the toggle renders both states from fixtures).
- [ ] **Step 4: Gates:** `pnpm build` (tsc), then UI Pixel Gate: `pnpm uishot builder` — **open and Read `.shots/builder-default.png`**, confirm the toggle renders, is ON by default, no layout break (StdinBar width budget does not apply here, but check the row fits). Grep uishot console output for `[fixture]` errors. Record via `conclave task gate`, attach shot path in the READY note.

---

## Integration (lead)

Merge order: any (A/B/C independent). After all merged: rerun 3 Rust gates + uishot on main, then LIVE verification: run `bash scripts/fetch-rtk.sh`, launch dev app, spawn a claude agent, have it run `git status`, confirm the executed command is `rtk git status` and settings file `<data_dir>/Conclave/agent-settings/<instance>.json` contains the PreToolUse hook. Then flip the Builder toggle off, respawn, confirm hook absent.

## Risk ledger

- **tauri-build validates `externalBin` eagerly on plain cargo builds** (found live by Tiësto, challenge 5432acf7): without the build.rs zero-byte placeholder, any tree that hasn't run `fetch-rtk.sh` fails `cargo check`. The placeholder + zero-size resolver guard + package.json fetch chain keep both cargo and runtime fail-open.
- `cargo install --git --tag`: tag naming may be `rtk-v0.42.4` (release-please) — A1 Step 2 verifies and fixes.
- `refresh_shim_link` may hardcode the `conclave` link name — A2 generalizes it; keep the conclave behavior byte-identical.
- `list_with_counts` has a HARDCODED column list that must stay in sync with `COLS` — A4 touches both or the query breaks at runtime, not compile time.
- Claude Code hook execution does not inherit the agent shell's PATH exports — that is WHY the hook command embeds absolute shim paths; do not "simplify" to bare `conclave`.
- rtk >= 0.23.0 contract only; unknown exit codes must pass through (B1 case 6).
