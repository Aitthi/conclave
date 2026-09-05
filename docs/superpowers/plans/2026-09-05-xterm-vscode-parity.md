# Terminal (xterm.js) parity with VS Code — umbrella plan

owner: 30fa04f4-e047-4241-a9ed-f452529952be (Detoro, Lead) · authority: in-loop

## Goal (human's words, 2026-09-05)

"แก้ xterm ให้หน่อย อยากได้แบบ VS Code ที่เปิด app อะไรใน terminal ก็ไม่ bug" and
"clone vscode มาดูเลย vscode ไม่เป็น" — the embedded terminal must render any TUI
(Claude Code, Codex, etc.) as cleanly as VS Code's integrated terminal does. VS Code
is the reference implementation; when in doubt, do what VS Code does and cite the line.

## The reported defect

Screenshot: `docs/superpowers/plans/assets/2026-09-05-xterm-autocomplete-garble.png`
(Read it with an image-capable reader).

Claude Code 2.1.261 running in the Conclave terminal pane. The user typed `/s at` and
the slash-command autocomplete popup rendered corrupted:

- row 1 shows `statusli … S  Toggle th…` at column 0 — the `  /` prefix and the tail
  of `/statusline` are missing, and a stray `S` sits between name and description;
- row 4 shows `ny-orchestrate-subagents` flush-left at column 0 while rows 2, 3, 5
  correctly show `  /setup-…`, `  /agy-…`, `  /workers-…` indented by two spaces;
- everything else (help footer `↑/↓ Navigate · enter Select · tab Co…`, `esc to cancel`)
  renders fine.

So specific rows keep stale content from an earlier frame while neighbours are fresh.
The same Claude Code build in VS Code / Terminal.app / iTerm2 does not do this (human's
statement). Root cause is NOT yet known — that is what the two research lanes settle.

## Facts already established (do not re-derive)

Our stack (`src/components/Terminal.tsx`, `src-tauri/src/engine/runtime/pty.rs`,
`src-tauri/src/engine/commands/instance.rs` ~L1405-1445 forwarder, `src/ipc/events.ts`):

| Area | Conclave today | VS Code (sparse clone at `/Users/detoro/code/vscode`, HEAD d4471383, 2026-09-05) |
|---|---|---|
| xterm version | `@xterm/xterm` 6.1.0-beta.287, webgl 0.20.0-beta.286, unicode11 0.10.0-beta.287, serialize 0.15.0-beta.287, fit 0.11 | xterm 6.1.0-beta.303, webgl 0.20.0-beta.299, unicode11 0.10.0-beta.300, serialize 0.15.0-beta.300, headless 6.1.0-beta.302, node-pty 1.2.0-beta.15 |
| PTY env | `TERM=xterm-256color`, `COLORTERM=truecolor`, `LANG` default `en_US.UTF-8` (pty.rs L75-78). **No `TERM_PROGRAM`.** | `TERM_PROGRAM=vscode`, `TERM_PROGRAM_VERSION=<version>`, `COLORTERM=truecolor` (`src/vs/workbench/contrib/terminal/common/terminalEnvironment.ts` L63-70) |
| Claude Code terminal detection | The `claude` binary (`~/.local/share/claude/versions/2.1.261`) has a quirks table keyed on `TERM_PROGRAM` / `TERM` (`isGhostty`, `isMintty`, `hasOsc52ClipboardUtf8Bug`, `rendersItalicAsStandout`, color-level via `TERM_PROGRAM_VERSION`, tmux probes…). With `TERM_PROGRAM` unset it takes the "unknown terminal" path. What that path changes about RENDERING is unknown → lane R1. | n/a |
| Output decoding | `decode_with_carry` holds back split UTF-8 tails (pty.rs L208-220) — chunk boundaries do NOT produce U+FFFD. Chunks go 1:1 to a Tauri `session:output` event, no coalescing, no drop. | `TerminalDataBufferer` coalesces PTY data for ~5 ms then one `xterm.write` (`src/vs/platform/terminal/common/terminalDataBuffering.ts`); ack-based flow control pauses the pty above `FlowControlConstants.HighWatermarkChars` (`terminalProcess.ts` L325-327, L579-592) |
| Resize path | ResizeObserver → 120 ms debounce → `fit()` → `session.resize` IPC. FIRST sizing after mount sends a **jiggle** (`rows-1` then `rows` 60 ms later) to force a SIGWINCH; a respawn (`status=running`) re-arms the jiggle after 300 ms. | `xterm.onResize` → `_resize()` → `_processManager.setDimensions(cols, rows)` with a small debouncer (`terminalInstance.ts` L2051-2103); **never jiggles**; `resize()` clamps cols/rows ≥ 1 (`terminalProcess.ts` L544-548) |
| xterm options | `convertEol: true`, `fontSize 12`, `fontFamily "ui-monospace, SFMono-Regular, Menlo, monospace"`, `scrollback 12000`, `allowProposedApi`, `rescaleOverlappingGlyphs`, Unicode 11 active, WebGL `customGlyphs: true`, wheel→arrow-keys on alt buffer | `xtermTerminal.ts` L240-290: `allowProposedApi`, `scrollback`, `drawBoldTextInBrightColors`, `minimumContrastRatio`, `letterSpacing`, `lineHeight`, `windowOptions`, `ignoreBracketedPasteMode`, `rescaleOverlappingGlyphs`, `allowTransparency`, `macOptionIsMeta`, … **no `convertEol`**; unicode11 loaded when `unicodeVersion === '11'` (L1149-1155); WebGL with configurable `customGlyphs` (L896-908) |
| Live PTY sizes on this Mac (2026-09-05 09:05) | 3 claude agents at 55 rows × 153 cols, 2 at the 24×80 default (never resized — tabs never mounted) | n/a |
| xterm DECSET 2026 (synchronized output) | supported by our xterm build (`node_modules/@xterm/xterm/lib/xterm.js` has `case 2026`) | same |

`convertEol: true` is a known suspect: it turns every bare `\n` into `\r\n`. A TUI that
positions with CUP/CUU and emits `\n` deliberately for a line feed without a carriage
return gets its cursor column reset to 0 — exactly the "row content flush-left at col 0"
signature in the screenshot. This is a HYPOTHESIS; lane R2 proves or kills it.

## Lanes

| Slug | Owner / implementer | Deliverable |
|---|---|---|
| `xterm-parity-audit` (R1) | Guetta (Researcher) | `docs/superpowers/specs/2026-09-05-terminal-vscode-parity-audit.md` — ranked list of every behavioural difference between our terminal stack and VS Code's, each with file:line in BOTH repos and a recommendation (adopt / skip + why). Includes what Claude Code changes when `TERM_PROGRAM=vscode` vs unset (from `strings` on the binary). |
| `xterm-repro-harness` (R2) | Marty (Researcher) | `scripts/pty-record.py` + `scripts/xterm-replay.mjs` (record raw PTY bytes from a scripted `claude` session at a given size; replay them through `@xterm/headless` with the SAME options as `Terminal.tsx`; dump the screen as text) + `docs/superpowers/specs/2026-09-05-xterm-repro-findings.md` stating whether the screenshot corruption reproduces in headless xterm with our options, and whether it disappears with `convertEol:false` and/or `TERM_PROGRAM=vscode`. |
| `xterm-parity-fix` (F) | created by the lead AFTER R1+R2 land | code change in `src/components/Terminal.tsx` / `pty.rs` per the ruled root cause, gated by the R2 harness + `pnpm uishot` + human GUI check. |

R1 and R2 are independent (disjoint boundaries) and run in parallel. F waits for both.

## Global constraints (every lane inherits)

- Reference repo: `/Users/detoro/code/vscode` is a sparse, shallow clone (only
  `src/vs/platform/terminal`, `src/vs/workbench/contrib/terminal`,
  `src/vs/workbench/contrib/terminalContrib`, `src/vs/base/node`). Read-only. Cite as
  `vscode:<path>:<line>`.
- Do NOT edit `src/components/Terminal.tsx`, `pty.rs`, or any `src/` file in R1/R2 —
  those belong to lane F. Research lanes write only inside their boundary.
- Spawning `claude` for reproduction: use a scratch cwd under
  `/private/tmp/…`, scrub `CLAUDE_CODE*`/`CLAUDECODE` env (see
  `scripts/pty-inject-repro.py` L41-45 for the proven pattern), never the human's live
  agent PTYs. Each run may cost one small model turn — keep runs few and purposeful.
- Never touch the human's running Conclave app, its `conclave.sock`, or its DB.
- `pnpm install` only inside your own lane worktree
  (memory: lane worktrees have no node_modules).
- Progress and findings go on the task as `conclave task note`; the deliverable file is
  the record. Message the lead (`conclave tell 30fa04f4-e047-4241-a9ed-f452529952be …`)
  only for a decision, a blocker, or "READY".
- Escalations: `conclave task challenge <ws> <slug> …` on your own task; Detoro rules.
- Human-facing text is the lead's job; inter-agent messages in English.

## Risk ledger

- `claude` first-run trust dialog in a fresh scratch cwd — the existing repro script
  already drives it; reuse that code path.
- The autocomplete list depends on installed skills/plugins in `~/.claude`; the same
  machine → same list, but do not assert on exact entries, assert on layout invariants
  (every row starts with two spaces + `/`, no row starts at col 0 with a bare name).
- Headless xterm has no renderer; a defect that lives ONLY in the WebGL renderer will
  not reproduce there. If headless is clean, R2 must say so explicitly and the fix lane
  will need `pnpm uishot`-style pixel evidence instead.
- xterm 6.1 betas differ (287 vs 303); a difference that vanishes after bumping to
  VS Code's pinned versions is a legitimate finding — R1 reports the changelog delta.
