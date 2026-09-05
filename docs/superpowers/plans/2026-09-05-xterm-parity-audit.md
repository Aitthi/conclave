# Lane R1 — xterm-parity-audit

owner: 30fa04f4-e047-4241-a9ed-f452529952be (Detoro, Lead) · authority: in-loop
implementer: Guetta (10bd4d86-7d5d-4132-830c-ea912059e71e, Researcher)
umbrella: `docs/superpowers/plans/2026-09-05-xterm-vscode-parity.md` (read first)

## Reading order

1. Umbrella plan (above) — goal, established facts, constraints.
2. `docs/superpowers/plans/assets/2026-09-05-xterm-autocomplete-garble.png` (Read it).
3. Ours: `src/components/Terminal.tsx` (348 lines, whole file),
   `src-tauri/src/engine/runtime/pty.rs` L40-130 + L150-225,
   `src-tauri/src/engine/commands/instance.rs` L1363-1445,
   `src/ipc/events.ts` L150-240.
4. VS Code (`/Users/detoro/code/vscode`, read-only):
   `src/vs/workbench/contrib/terminal/browser/xterm/xtermTerminal.ts`,
   `src/vs/workbench/contrib/terminal/browser/terminalInstance.ts` (resize path
   L2051-2103, `onResize` wiring around L830-845),
   `src/vs/workbench/contrib/terminal/common/terminalEnvironment.ts` L40-80,
   `src/vs/platform/terminal/node/terminalProcess.ts` L100-200, L320-330, L537-600,
   `src/vs/platform/terminal/common/terminalDataBuffering.ts`,
   `src/vs/platform/terminal/common/terminal.ts` (search `FlowControlConstants`,
   `DEFAULT_TERMINAL_OPTIONS`, `unicodeVersion`, `customGlyphs`, `convertEol`).

## Deliverable

`docs/superpowers/specs/2026-09-05-terminal-vscode-parity-audit.md` with these sections:

1. **Diff table** — one row per behavioural difference between Conclave's terminal stack
   and VS Code's. Columns: area · Conclave (file:line) · VS Code (`vscode:file:line`) ·
   could it produce the screenshot symptom? (yes / no / indirect, one sentence) ·
   recommendation (adopt / skip / needs-measurement) · effort (S/M/L).
   Must cover at least: PTY env (`TERM_PROGRAM`, `TERM_PROGRAM_VERSION`, `LANG`/`LC_*`),
   `convertEol`, Unicode version + `rescaleOverlappingGlyphs`, WebGL `customGlyphs` and
   fallback, resize path (debounce, jiggle, clamping, order of `fit()` vs PTY resize),
   data path (coalescing, flow control, write ordering), scrollback/reflow on resize,
   wheel/alt-scroll handling, `windowOptions`, `ignoreBracketedPasteMode`,
   `macOptionIsMeta`, font measurement (family/letterSpacing/lineHeight), xterm and addon
   version deltas (link the xterm.js changelog entries between beta.287 and beta.303
   that touch parsing, reflow, or rendering — `node_modules/@xterm/xterm/package.json`
   vs `vscode:package.json`).
2. **Claude Code's TERM_PROGRAM-dependent behaviour** — from
   `strings -n 8 ~/.local/share/claude/versions/2.1.261 | grep -o '.{120}TERM_PROGRAM.{120}'`
   and neighbours: list every branch keyed on `TERM_PROGRAM`/`TERM`/`COLORTERM`, and say
   for each what the "unknown terminal" (unset) path does versus `vscode`. Flag anything
   touching rendering, cursor positioning, synchronized output, line wrapping, or
   width measurement. Quote the minified snippet (≤ 200 chars each).
3. **Ranked hypotheses for the screenshot** — top 3, each with the evidence row(s)
   above that support it and the ONE experiment lane R2's harness should run to confirm
   or kill it.
4. **Recommended change set for lane F** — ordered, smallest-first, each with the exact
   VS Code line to mirror.

No code changes. No edits outside the boundary. Cite lines, not vibes.

## Gates (record with `conclave task gate <ws> xterm-parity-audit -- <cmd>`)

- `test -s docs/superpowers/specs/2026-09-05-terminal-vscode-parity-audit.md`
- `grep -c 'vscode:' docs/superpowers/specs/2026-09-05-terminal-vscode-parity-audit.md`
  ≥ 15 (every VS Code claim is cited).

READY note must name: the file path, the #1 hypothesis in one sentence, the experiment
R2 should run for it.
