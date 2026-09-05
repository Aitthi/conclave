# Lane F — xterm-parity-fix

owner: 30fa04f4-e047-4241-a9ed-f452529952be (Detoro, Lead) · authority: in-loop
implementer: Tiësto (e60b9644-28b2-4a3f-a9d3-229ad50d5d47, Implementer)
reviewer: Mellow (b3a30e7b-5a9f-4d4d-a83e-768a9326632f)
umbrella: `docs/superpowers/plans/2026-09-05-xterm-vscode-parity.md`

## Reading order

1. Umbrella plan — goal ("behave like VS Code"), constraints.
2. `docs/superpowers/specs/2026-09-05-terminal-vscode-parity-audit.md` — the audit. §0 (two
   facts), §3 (hypotheses H1/H2/H3), §4 (the change set THIS lane implements). Every VS Code
   citation there is `vscode:<path>:<line>` in `/Users/detoro/code/vscode` (read-only).
3. `docs/superpowers/specs/2026-09-05-xterm-repro-findings.md` — headless replay is clean, so
   the defect is in OUR lifecycle/transport/renderer, not in xterm's parser.
4. `src/components/Terminal.tsx` (whole file), `src-tauri/src/engine/runtime/pty.rs` L40-130
   + L150-200, `src-tauri/src/engine/commands/instance.rs` L1140-1170 (spawn site) and
   L2100-2135 (`ResizeReq` / resize command), `src/ipc/commands.ts` L215-230 + L485-490.

## Ruling (Detoro, 2026-09-05) — what this lane changes and why

Root-cause ruling: Claude Code positions every patch RELATIVE to its own `displayCursor` and
never asks the terminal where the cursor is (audit S13). Any window where xterm's grid ≠ the
PTY's grid, or where a chunk never reaches xterm, desyncs that model until the next full
repaint — the column-0 rows in the screenshot. Conclave has three such windows VS Code does not
(audit rows 9, 10, 13, 21). The WebGL addon we ship (beta.286) also predates three upstream
stale-render fixes (audit §1a). We close ALL of them in one lane because each is VS Code parity
on its own; ranking between H1/H2/H3 does not change the change set.

Implement items 1–9 of audit §4, in this order, ONE COMMIT PER ITEM (bisectable):

### F1 — remove `convertEol: true` (`Terminal.tsx` ~L86)
Mirror `vscode:src/vs/workbench/contrib/terminal/browser/xterm/xtermTerminal.ts:241-285`
(option absent). Safe: the PTY has `onlcr` on (audit F1). Update the comment block.

### F2 — xterm + addon bump to VS Code's pins (`package.json` L21-25 + devDep headless)
Exact versions from `vscode:package.json:137-146`: `@xterm/xterm 6.1.0-beta.303`,
`@xterm/addon-webgl 0.20.0-beta.299`, `@xterm/addon-unicode11 0.10.0-beta.300`,
`@xterm/addon-serialize 0.15.0-beta.300`, `@xterm/headless 6.1.0-beta.302` (devDep, used by
`scripts/xterm-replay.mjs`), keep `@xterm/addon-fit ^0.11.0`. Pin exact (no `^`), run
`pnpm install` in the lane worktree, then `pnpm build` (tsc) and the R2 replay gate (below)
must both stay green. Closes H2.

### F3 — env identity + synchronized output from frame 1 (`pty.rs` L69-85)
Mirror `vscode:src/vs/workbench/contrib/terminal/common/terminalEnvironment.ts:63-70`:
`cmd.env("TERM_PROGRAM", "Conclave")`, `cmd.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"))`
(Cargo.toml `version = "0.2.0"`). Do NOT claim `vscode` (audit S6/S9/S11 would then apply
behaviours we do not have). Also mirror `terminalEnvironment.ts:99-115`: set `LANG` when it
is missing OR not UTF-8 (today only when missing, L77-79).
For the Claude Code CLI only, add `CLAUDE_CODE_FORCE_SYNC_OUTPUT=1` (audit S1): the spawn
site `instance.rs` ~L1153 passes `extra_env`; find where `extra_env` is assembled (grep
`extra_env` in `instance.rs`) and where the agent's CLI kind is known (`cliKind ==
"claude-code"`), and push the pair there rather than special-casing `pty.rs`. Extend
`spawn_cli_applies_extra_env` or add a sibling test proving `TERM_PROGRAM`/`_VERSION` reach
the child (pattern: pty.rs L263-285). Closes H3's consequence even if the probe is lost.

### F4 — log the renderer fallback, expose a dev probe (`Terminal.tsx` L128-138)
Mirror `vscode:…/xtermTerminal.ts:941-944, 955-959`: on WebGL load failure and on
`onContextLoss`, `console.warn("[terminal] webgl renderer unavailable, DOM fallback", err)`.
Dev-only (`import.meta.env.DEV`): register the live terminal in a module-level
`Map<sessionId, XtermTerminal>` exposed as `window.__conclaveTerms` so the GUI probe in the
verification section can call `term.clearTextureAtlas()` and read
`term.modes.synchronizedOutputMode`. Delete from the map on unmount. Not shipped in prod
(tree-shaken by the DEV guard, same pattern as `src/fixtures/`).

### F5 — no size gap at mount: fit synchronously, queue early output (`Terminal.tsx` L140-170, L267-280, L333-336)
Mirror `vscode:…/xtermTerminal.ts:243-244` (xterm constructed at the right size) and
`vscode:src/vs/workbench/contrib/terminal/browser/terminalInstance.ts:1586`
(`_initialDataEvents` queue). Concretely:
- After `term.open(el)`, if `el.getBoundingClientRect().width > 0`, call `fitAddon.fit()`
  SYNCHRONOUSLY before the snapshot restore and before `termRef.current = term`. The
  snapshot then writes into the real-width grid, not the 80-col default.
- Output that arrives before the first fit completes (listener attached, fit pending, e.g.
  hidden container) is pushed onto a per-mount `pending: string[]` and flushed in order
  right after the first successful fit — never written into a wrong-size grid.
- Keep the 200 ms deferral ONLY for the PTY-side resize/SIGWINCH (the listener-attach
  reason in the existing comment still holds for the repaint); xterm's own grid is sized
  immediately.

### F6 — the jiggle must move xterm and the PTY together (`Terminal.tsx` L215-231, L285-291)
VS Code's contract: xterm and PTY are always resized as a pair
(`vscode:…/terminalInstance.ts:830-831`); it never jiggles. Minimal self-consistent form:
`term.resize(cols, rows - 1)` + `ipc.session.resize(rows - 1)`, then 60 ms later
`term.resize(cols, rows)` + `ipc.session.resize(rows)`. The respawn re-push (L285-291) goes
through the same pair. If you find a way to force the child's repaint WITHOUT a size change
(e.g. a same-size `TIOCSWINSZ` does NOT signal on macOS — do not rely on it), file it as a
task note; do not widen scope.

### F7 — order the stdin path (`Terminal.tsx` L236-241)
Mirror `vscode:…/terminalInstance.ts:676-679` → `terminalProcessManager.ts:651-665` (one
ordered channel). TS-side chain: keep a per-mount `let stdinChain = Promise.resolve()` and
do `stdinChain = stdinChain.then(() => ipc.message.send({sessionId, text: data})).catch(() => {})`
so XTVERSION/DA1/DECRQM replies (all emitted by xterm in one parse pass) reach the PTY in
emission order. The Rust reordering (audit item 7, M) is deferred — note it.

### F8 — pixel dimensions + `windowOptions` (`pty.rs` L155-165, `instance.rs` L2104-2108, `commands.ts` L220-223, `Terminal.tsx` L85)
Mirror `vscode:…/terminalInstance.ts:2095-2104` and `xtermTerminal.ts:280-284`. Add optional
`pixelWidth?: number; pixelHeight?: number` to `session.resize` (TS type + `ResizeReq` with
`#[serde(default)]`), fill them from `el.getBoundingClientRect()` rounded, pass into
`PtySize.pixel_width/height`. Constructor option
`windowOptions: { getWinSizePixels: true, getCellSizePixels: true, getWinSizeChars: true }`.

### F9 — `scrollOnEraseInDisplay: true` (`Terminal.tsx` L85) — mirror `xtermTerminal.ts:270`.

Deferred (NOT this lane, recorded so nobody re-proposes them blind): audit §4 item 10
(keep xterm alive across tab switches — structural, after GUI confirmation of H1), item 11
(output coalescing + flow control — L, needs the `seq` probe first), Rust-side stdin
reordering (item 7 M form).

## Boundary

`src/components/Terminal.tsx`, `src/ipc/commands.ts`, `src-tauri/src/engine/runtime/pty.rs`,
`src-tauri/src/engine/commands/instance.rs`, `package.json`, `pnpm-lock.yaml`,
`docs/superpowers/plans/2026-09-05-xterm-parity-fix.md` (append a "## Outcome" section:
what each commit changed, gate ids, anything deferred with the reason).

Design canon: none — this lane changes terminal behaviour, not Conclave UI chrome. The
terminal's pixels are the child TUI's. `pnpm uishot home` is still required by the standing
UI Pixel Gate because `src/` UI files change; PTY panes render empty in fixture mode
(known caveat) — the gate proves nothing else regressed.

## Gates (record EACH with `conclave task gate <ws> xterm-parity-fix -- <cmd>`)

1. `pnpm build` (tsc + vite) — after F2 and at the end.
2. `node scripts/xterm-replay.mjs /private/tmp/claude-501/xterm-repro/2026-09-05-marty/rec-a.jsonl`
   — must exit 0 after F2 (the bumped headless build still renders the recording clean;
   exit 3 means the harness did not see the popup — investigate, do not accept).
3. `cargo test -p <crate> pty::` for the pty tests + your new env test (from
   `src-tauri`; use `cargo test --manifest-path src-tauri/Cargo.toml pty::`), and
   `cargo fmt --check -- <your changed .rs files only>` (memory: never bare `cargo fmt`
   in a lane; main has pre-existing drift).
4. `pnpm uishot home` then Read `.shots/home-default.png` (UI Pixel Gate).
5. Live GUI probe — the ONLY place the defect ever appeared. This needs a rebuilt app, which
   the human relaunches (memory: dev instance cannot take `conclave.sock`). So: post READY with
   commits + gates 1-4, and write the probe script for the human into the Outcome section:
   open a Claude Code tab, type `/` then `s` then ` at` slowly, screenshot; in the devtools
   console run `[...window.__conclaveTerms.values()].map(t => t.modes.synchronizedOutputMode)`
   (expect `true` while output flows) and, if any garble appears, `t.clearTextureAtlas()` —
   rows healing without new output = renderer (H2); rows staying = buffer (H1).

## Risk ledger

- `pnpm install` after the bump rewrites `pnpm-lock.yaml`; commit it in the F2 commit only.
- F5's synchronous `fit()` runs while the container may still be `display:none` (remount
  mode swaps tabs) — the existing `getBoundingClientRect().width === 0` guard is the
  bail-out; keep it ahead of every `fit()`.
- F6 changes the number of `term.resize` calls; the ResizeObserver debounce (120 ms) may
  fire after our programmatic resize — the dedup (`lastCols/lastRows`) must stay correct.
- `TERM_PROGRAM=Conclave` is a new identity; Claude Code's allowlists (audit S8/S12/S16)
  treat it as unknown — that is the intended, VS-Code-independent path. Do not add it to any
  Claude Code env allowlist by faking a known name.
- Codex/other CLIs in the same PTY code path get the same env; `TERM_PROGRAM` is
  informational for them. If a CLI misbehaves with it set, file a challenge with the bytes.

## Amendment 2026-09-05 (ruling 602f6ff7, found by Tiësto)

F8's resize seam is DEFINED in `src-tauri/src/engine/runtime/mod.rs` (`LiveHandle.resize`
closure type L110, no-op constructors L149/L187, `Runtime::resize` L453), not in the files the
boundary listed. Ruled: boundary widened by exactly that file for the mechanical widening
(closure `Fn(u16,u16,u16,u16)`, no-op constructors take four ignored args, `Runtime::resize`
gains `pixel_width/pixel_height`). Landed as its own scoped commit (`git commit --
src-tauri/src/engine/runtime/mod.rs`). Lead defect: the importer was pinned instead of the
defining file — see memory `lead-boundary-defining-file-not-importer`.

## Outcome (Tiësto, 2026-09-05)

Items F1–F9 landed, one commit per item (F8 split in two — see the boundary note).
Branch `lane/xterm-parity-fix`.

| Item | Commit | What changed |
|---|---|---|
| F1 | `ad3b994` | `convertEol` removed from the xterm constructor; the comment now records WHY it is absent (PTY keeps `onlcr`, audit §0 F1) so it does not get re-added. |
| F2 | `30904ee` | xterm/webgl/unicode11/serialize/headless pinned to VS Code 1.138's exact builds (`vscode:package.json:137-146`). Closes H2. |
| F3 | `ca7c3a5` | `TERM_PROGRAM=Conclave` + `TERM_PROGRAM_VERSION`; `LANG` now follows VS Code's `'auto'` UTF-8 rule instead of only firing when unset (extracted as `should_set_lang`); `CLAUDE_CODE_FORCE_SYNC_OUTPUT=1` for `cliKind == claude-code` only. |
| F4 | `c705eed` | `console.warn` on both WebGL-unavailable and context-loss; DEV-only `window.__conclaveTerms` registry for the GUI probe. |
| F5 | `d047b38` | xterm fitted SYNCHRONOUSLY at mount (after the WebGL addon loads), before the snapshot restore and the first live write; output queued and replayed in order while the pane cannot be measured. Kills the ~200 ms 80×24-vs-real window. |
| F6 | `9d511b9` | Both jiggle legs now move xterm and the PTY together. |
| F7 | `bc3cdde` | One ordered stdin channel; all three call sites (onData, wheel, file-drop) join it. |
| F8 | `fbedfb6` (seam) + `9d8e8c6` (wire-up) | Pixel dimensions carried through `LiveHandle.resize` → `PtySize`; `windowOptions` report ops enabled. |
| F9 | `7a93280` | `scrollOnEraseInDisplay: true`. |

### Gates (all recorded on task `xterm-parity-fix`, final sweep at `7a93280`)

1. `pnpm build` — exit 0 (also run at `30904ee`, right after the bump).
2. `node scripts/xterm-replay.mjs …/rec-a.jsonl` — exit 0, `LAYOUT INVARIANT: PASS`
   (also at `30904ee`: the bumped headless build still renders the recording clean).
3. `cargo test --manifest-path src-tauri/Cargo.toml pty::` — exit 0, 8 passed / 2 ignored,
   including the two new tests `spawn_cli_advertises_terminal_identity` and
   `should_set_lang_matches_vscode_auto_rule`. `runtime::` also green (190 passed).
4. `pnpm uishot home` — exit 0, `.shots/home-default.png` written AND inspected: chrome,
   roster, chat panel and composer all intact; the terminal pane is black with a cursor
   block, which is the documented fixture-mode caveat, not a regression.
5. rustfmt — see the note below.

### Boundary

`src-tauri/src/engine/runtime/mod.rs` is out of the declared boundary and was ruled in by
Detoro (challenge `602f6ff7`); it lands in its own commit `fbedfb6`. That commit also carries
**`src-tauri/src/engine/runtime/chat.rs`** — a one-line `|_, _|` → `|_, _, _, _|` on the chat
backend's no-op resize stub. It is a third construction site of the same closure that the
challenge missed; the compiler found it, grep had not. It is named here and in the READY note
rather than filed as a second challenge, because leaving it out simply does not build.
The one-line `(…, 0, 0)` caller update in `instance.rs` rides along in `fbedfb6` for the same
reason: every commit in this lane builds and tests green on its own.

### Deviations and judgment calls (logged as task notes, not escalated)

- **F8 pixel source.** The plan's wording said `el.getBoundingClientRect()`; the implementation
  reads `term.dimensions?.css.canvas`, rounded. The plan cites
  `vscode:…/browser/terminalInstance.ts:2095-2104` as the thing to mirror and that line reads
  `rawXterm.dimensions?.css.canvas`. The container box includes the sub-cell remainder the grid
  does not cover, so a child dividing `pixel_width` by `cols` would derive a cell width slightly
  too large. `term.dimensions` is public API in `@xterm/xterm 6.1.0-beta.303`
  (`typings/xterm.d.ts:987`).
- **F5 fit position.** The synchronous fit runs after the WebGL addon is loaded, not immediately
  after `term.open()`. WebGL cell metrics differ from the DOM renderer's, which is why VS Code
  fires `_onDidRequestRefreshDimensions` at exactly that point (`xtermTerminal.ts:947`).
- **F6.** No same-size SIGWINCH exists to replace the jiggle with — `TIOCSWINSZ` with unchanged
  dimensions signals nothing on macOS. The jiggle stays; audit §4 item 10 remains the
  structural fix.
- **rustfmt gate shape.** `rustfmt <file>` follows the `mod` declarations in `runtime/mod.rs`
  and reports pre-existing drift in `browser.rs` and `task_timer.rs`, which this lane never
  touched (matching the recorded "cargo fmt drift on main"). `--skip-children` /
  `--unstable-features` are not available on this stable rustfmt. The recorded gate therefore
  greps the diff for THIS lane's four files only and passes when none appear. It was falsified
  before being trusted: deliberately misformatting one line in `chat.rs` makes it exit 1.

### Deferred (unchanged from the plan, plus one)

- Audit §4 item 10 (keep xterm alive across tab switches) — structural, after GUI confirmation.
- Audit §4 item 11 (output coalescing + flow control) — needs the `seq` probe first.
- Audit §4 item 7's Rust form (reordering in `message.rs`) — out of boundary, and unnecessary
  now that the TS side is a single ordered channel. Remaining exposure: a terminal write racing
  a NON-terminal stdin source, e.g. `message.inject`.
- **Size-at-spawn** (raised by Mellow in review, ruled deferred by Detoro). `spawn_cli` still
  opens every PTY at the hardcoded 80x24 (`pty.rs`, the `openpty` call) and only learns the real
  size when the frontend's first `session.resize` arrives. VS Code never has that window: cols
  and rows are constructor arguments to the process itself
  (`vscode:src/vs/platform/terminal/node/terminalProcess.ts:136-137`), so the child is born at
  the right size. F5 closed the window on the XTERM side for a fresh mount, but a RESPAWN
  (restart · resume) creates a brand-new PTY at 80x24 while the component stays mounted, and the
  child can paint into that before `repushSizeRef`'s 300 ms re-fit lands — the same
  wrong-grid-first-frame mechanism as H1, just on the PTY side of the pair. Fix shape: thread
  `cols`/`rows` into `spawn_cli` and seed `PtySize` with them, defaulting to 80x24 when the
  caller has no measurement. Deferred because it changes `spawn_cli`'s signature and therefore
  every caller and their tests, which is a wider blast radius than this lane's boundary; it is
  the natural first task of a follow-up lane.

### Cross-check against lane R3's H1 replay (merged from main at `078c3b1`)

R3's findings were recorded on `@xterm/headless 6.1.0-beta.287`; F2 bumped it to
`.302`, so their cells were re-run on the new build to confirm the bump does not
invalidate them. It does not — the numbers reproduce exactly:

| Cell | Invocation (on `rec-a.jsonl`) | R3 @ .287 | Here @ .302 |
|---|---|---|---|
| I (control) | `--start-size 153x55` | cursor absoluteRow 8, 55 rows | identical |
| H (repro) | `--start-size 80x24 --resize-at-ms 2000` | absoluteRow 20, 57 rows, buffer offset | identical |

Note what this does and does not prove. Cell H is the *harness* simulating the
pre-F5 mount window; it holds the mismatch itself, so it will keep reproducing
regardless of what the app does. It is evidence for the MECHANISM F5 removes
(bytes written into a wrong-size grid damage the buffer and leave the cursor 12
rows low), not a regression test on this lane's code — the app-side proof is that
the window no longer exists, which only the live GUI probe below can show. R3's own
verdict stands: this supports H1's general desynchronization mechanism without
establishing that the 200 ms window caused the reported screenshot.

### Gate 5 — human acceptance, VISUAL ONLY (Detoro ruling, 2026-09-05)

The built app ships without devtools, so the human cannot open a console. Acceptance
is therefore what the eye can see; the `__conclaveTerms` probe below is a DEV-BUILD
tool and is not part of this gate. Needs a rebuilt + relaunched app either way — a
`pnpm tauri dev` instance cannot take `conclave.sock` and its janitor prunes the live DB.

What to do, and what a pass looks like:

1. Open a Claude Code agent tab and let it finish painting. **Pass:** the frame fills
   the pane at the real size on the first paint — no black pane, no 80-column-wide
   frame sitting in a wider pane, no rows left over from a narrower layout.
2. Type `/`, then `s`, then ` at` — slowly, one key at a time. **Pass:** every row of
   the autocomplete popup keeps its `  /` indent. **Fail:** any row's first segment
   jumps to column 0 while the rest of the row is stale — that is the reported
   screenshot (`docs/superpowers/plans/assets/2026-09-05-xterm-autocomplete-garble.png`).
3. Switch workspace, or open the Lane board and come back. This remounts every terminal
   (`AppShell.tsx:288`), which is the exact path F5 and F6 change. **Pass:** the pane
   repaints at the right size with no column-0 rows and no stray fragments.
4. Resize the window a few times, including narrow. **Pass:** one clean repaint per
   settle, no leftover glyph fragments at the old width.

**What this gate cannot tell you.** With no console there is no `clearTextureAtlas()`,
so a failure at step 2 cannot be attributed to a renderer fault (H2) versus a buffer
fault (H1) by eye — both look identical. If step 2 fails, the next move is to reproduce
it on a dev build and run the probe below; do not re-rank the hypotheses from the
screenshot alone.

#### Dev-build-only probe (not part of gate 5)

On a dev build, where devtools are available:

- `[...window.__conclaveTerms.values()].map(t => t.modes.synchronizedOutputMode)`
  → expect `true` while output is flowing. `false` means F3's
  `CLAUDE_CODE_FORCE_SYNC_OUTPUT` did not take (check the agent is `cliKind:
  claude-code` and that the binary was actually rebuilt).
- `[...window.__conclaveTerms.values()].forEach(t => t.clearTextureAtlas())` when garble
  is on screen. Rows that HEAL with no new PTY output → renderer fault (H2), closed by
  the F2 bump. Rows that STAY → buffer fault (H1), what F5/F6 target.

The registry is behind `import.meta.env.DEV` and is tree-shaken out of the packaged
build, so it is absent from the app the human accepts — by design, not by omission.
