---
date: 2026-06-28
branch: main
head: d7e66c5
status: M6 (themes/menus) + agent-to-agent awareness shipped; awaiting user runtime test of wheel-scroll + agent↔agent chat
---

# Snapshot: Conclave — themes/menus + agent-to-agent awareness

Conclave = native macOS app (Tauri v2 + Rust core + React 19 + Vite + TS strict +
Tailwind v4 + SQLite WAL). A workspace hosts multiple AI CLI agents (Claude Code,
Codex) in live xterm.js terminals; agents can message each other.

Built via `/ny-auto-pipeline` (autonomous). Replies to user in **Thai**; app UI
copy in **English**.

## Standing constraints (carry forward verbatim)
- **Git author MUST be** `detoro <meanstack20@gmail.com>` — commit via
  `git -c user.name='detoro' -c user.email='meanstack20@gmail.com' commit`.
  Verify after every commit: `git log -1 --format='%h  %an <%ae>'`.
- **Commit messages end with** `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.
- Commit messages with backticks / `$` / special chars → write to a file and use
  `git commit -F <file>` (zsh command-substitution corrupts `-m` messages).
- **App UI copy = ENGLISH**; conversational replies to the user = **Thai**.
- **No fabricated data** — honest deferred placeholders only.
- **Secrets** (API keys/tokens) live ONLY in macOS Keychain — never in DB, logs, or IPC.
- **Pipeline discipline:** implement → parallel reviewers → fix inline → re-verify
  ALL baselines myself → commit as detoro.

## Where we are
- Branch `main`, HEAD `d7e66c5`. Working tree clean except `.arta/*` (viewer
  runtime, ignorable) and `scratchpad/` (untracked session scratch).
- Two features landed this session: **M6** (light/dark themes + system accent +
  native macOS menus/shortcuts) and **agent-to-agent awareness** (agents know each
  other and message via `conclave tell`).
- Last user-reported issue (wheel-scroll in full-screen TUIs) fixed in `d7e66c5`,
  not yet confirmed working by the user.

## Baselines (last-known status — all GREEN at d7e66c5)
- Rust: `cd src-tauri && cargo test --lib` · `cargo clippy --all-targets -- -D warnings` · `cargo fmt --check`
- Frontend: `pnpm exec tsc --noEmit` · `pnpm build` (>500kB chunk advisory is OK)
- Frontend-only changes hot-reload under `tauri dev` — no respawn needed.
  Rust changes require a `cargo` rebuild (and rebuild `conclave-cli` bin if its
  source changed — a stale binary is a real bug source, see Gotchas).

## Recent commits (this session, oldest→newest)
| SHA | summary |
| --- | --- |
| b9881a1 | feat(ui): light/dark themes, system accent, native menus + shortcuts |
| d99da6e | fix(ui): dark-mode inverse icon chips (white-on-white in dark) |
| a8a989e | feat(agents): agent-to-agent awareness — spawn briefing, sender tags, conclave on PATH |
| 0cd2845 | fix(agents): inject must submit with a separate CR, not a trailing LF |
| 971d9e3 | feat(agents): teach agents the reply protocol + put sender id in the tag |
| f501887 | fix(agents): bake the agent's own id into the briefing |
| d34e029 | fix(agents): eager-spawn every agent in a workspace, not just the open tab |
| 1fde019 | perf(agents): conclave tell prints a terse confirmation, not the echoed message |
| d7e66c5 | feat(terminal): mouse-wheel scrolls full-screen TUIs (alternate scroll) |

All verified author `detoro <meanstack20@gmail.com>`.

## Deferred / pending (gated on user go-ahead — do NOT start unprompted)
1. **Drain queued messages on spawn** — a message injected the instant an agent
   spawns can be lost: the TUI isn't ready to receive stdin yet. Needs a
   TUI-readiness signal before injecting. Risky without a runtime test. Offered,
   not approved.
2. **Enrich `conclave agent list` / `instance.list` with agent names** — currently
   returns only id/agentDefId/status, ambiguous with 3+ agents. Offered, not approved.
3. Not pending unless asked: full packaging (codesign/notarize/dmg); runtime
   verification of Codex `-c developer_instructions`.

## Load-bearing files (agent-awareness feature)
- `src-tauri/src/engine/agentctx.rs` — `bootstrap_preamble(name, role, ws_name, ws_id, self_id)`
  builds the one-line system-prompt briefing (bakes self_id; tells the agent that
  `[from <name> · <id>]` lines are from peers and it MUST reply via
  `conclave tell <id> <msg>`). `sanitize_field` strips `=`/newlines (Codex `-c key=value`
  safety). `ensure_conclave_shim()` symlinks `conclave`→`conclave-cli` under a 0700
  dir to put it on PATH. Has unit tests.
- `src-tauri/src/engine/commands/instance.rs` — spawn path. Builds the preamble,
  passes it via `claude --append-system-prompt` / `codex -c developer_instructions=…`,
  prepends `export PATH=<shim>:"$PATH"; `, sets `CONCLAVE_WORKSPACE_ID` +
  `CONCLAVE_INSTANCE_ID` env.
- `src-tauri/src/engine/commands/message.rs` — `inject`: delivers
  `[from <sender> · <fromInstanceId>] <text>`, then `sleep(40ms)`, then a SEPARATE
  `send_stdin("\r")` to submit (NOT a trailing LF). Persisted/emitted text stays RAW.
- `src-tauri/src/engine/commands/cli.rs` — `tell <fromId> <toId> <text>` arm → `message.inject`.
- `src-tauri/src/bin/conclave-cli.rs` — `expand_tell_args` turns the agent-facing
  `tell <toId> <text>` into wire form, filling `<fromId>` from `CONCLAVE_INSTANCE_ID`.
  Prints a terse `"{status} -> {to}"` on success (not the echoed message — token saver).
- `src/components/Terminal.tsx` — xterm.js pane. `wheelHandler` does iTerm-style
  "alternate scroll": on the alternate buffer, when the app isn't mouse-tracking,
  wheel notch → arrow-key presses (`\x1b[A`/`\x1b[B` × 3) into the PTY.
- `src/components/WorkspacePane.tsx` — `spawnInstance` + eager-spawn effect (spawns
  every tab's instance, not just the active one — so an unopened agent can still
  receive messages).

## Load-bearing files (M6 themes/menus)
- `src/styles/app.css` — Tailwind v4 `@theme inline` token foundation; `:root`
  (light) vs `.dark` (dark) `--c-*` palettes; `--c-overlay` (black↔white) drives
  auto-inverting hairline/tint utilities; `ink`/`on-ink` inverse pair.
- `src/lib/theme.ts` — `ThemePref` system/light/dark, localStorage `conclave.theme`,
  `apply()`/`initTheme()`/`initSystemAccent()`/`subscribeTheme`.
- `src-tauri/src/menu.rs` — native menu bar (⌘N new_agent, ⌘L library, ⌘B blackboard,
  Appearance submenu); emits `menu` event with item id.
- `src-tauri/src/sysaccent.rs` — reads macOS `AppleAccentColor` → control-accent hex.
- `src/components/AppShell.tsx` — `useEvent<string>("menu", …)` maps menu ids to actions.

## Gotchas
- **SCOPE WARNING / LOOP WARNING / `import.meta.env` LSP diagnostics = FALSE POSITIVES.** Ignore.
- **TUI submit = `\r` (CR), not `\n` (LF).** Codex's paste-burst detection needs the
  `\r` sent as a SEPARATE write after a ~40ms delay, or it just inserts a newline.
- **`message.inject` is the agent↔agent primitive** (instance-keyed, queues if offline).
  `message.send` is the low-level human primitive (sessionId, raw chars). Don't cross them.
- **Rebuild `conclave-cli` when its source changes** — agents invoke the on-PATH shim
  which points at the built binary; a stale binary silently lacks new arg handling.
- **`/clear` wipes conversation history only.** The system-prompt briefing layer
  (`--append-system-prompt` / `-c developer_instructions`) is reconstructed from launch
  args each turn, so agent identity survives `/clear`. This is why the briefing lives there.
- **xterm alternate buffer has no scrollback** — `term.buffer.active.type === "alternate"`;
  history is inside the TUI. That's why wheel-scroll is faked via arrow keys.

## Resume hint
Await the user's runtime test of (a) wheel-scroll in Codex/Claude Code panes and
(b) agent↔agent chat end-to-end. They typically restart the app, test, then either
confirm or report a refinement — respond to that. Do NOT start the deferred items
(queue-drain, agent-list names) without an explicit go-ahead.
