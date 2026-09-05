# Terminal stack parity audit — Conclave vs VS Code (lane R1 `xterm-parity-audit`)

author: Guetta (Researcher, 10bd4d86) · owner: Detoro · date: 2026-09-05
umbrella: `docs/superpowers/plans/2026-09-05-xterm-vscode-parity.md` · lane plan:
`docs/superpowers/plans/2026-09-05-xterm-parity-audit.md`

Reference: `/Users/detoro/code/vscode` @ d4471383 (VS Code 1.138.0, `vscode:package.json:3`),
cited as `vscode:<path>:<line>`. Ours: lane worktree @ c2111cf (main). Claude Code binary:
`~/.local/share/claude/versions/2.1.261` (Mach-O arm64, Bun-compiled), inspected with
`strings -n 8` (327,506 lines; scratch copy `claude-strings.txt`).

Legend for "could it produce the screenshot symptom?": **yes** = a direct mechanism exists ·
**indirect** = changes Claude Code's byte stream or the terminal's state so that another
mechanism can fire · **no** = cosmetic / perf / unrelated.

## 0. Two facts that re-rank everything (established today, verifiable)

**F1 — the PTY keeps `ONLCR` on while Claude Code runs.** `stty -a -f /dev/ttys007` on a live
Conclave-spawned `claude` 2.1.261 (this agent's own PTY, 2026-09-05) reports
`oflags: opost onlcr -oxtabs -onocr -onlret` and `lflags: -icanon -isig -iexten -echo`. Bun's raw
mode disables echo/canonical input but leaves output post-processing on, so every `\n` the app
writes reaches xterm as `\r\n`. xterm's `convertEol` only adds `x = 0` on LF
(`node_modules/@xterm/xterm/src/common/InputHandler.ts:742-747`) — a no-op after a CR. **The
umbrella's `convertEol` hypothesis cannot produce the column-0 rows.** Keep the option delta in
the table (VS Code does not set it) but do not expect R2's `convertEol:false` replay to differ.

**F2 — Claude Code's byte stream for us is the same as for VS Code, except synchronized output
timing.** Claude Code identifies xterm.js via the XTVERSION reply (`DCS > | xterm.js(<ver>) ST`,
which our build sends: `node_modules/@xterm/xterm/lib/xterm.js` `sendXtVersion`) and, for
xterm.js terminals, **disables its DECSTBM scroll-region renderer** whether or not
`TERM_PROGRAM=vscode` (§2, snippets S2/S3). So VS Code and Conclave both run the same
"gated" render path; the only TERM_PROGRAM-driven rendering difference is *when* DECSET 2026
(synchronized output) turns on: at startup in VS Code (allowlist) vs after a runtime probe in
Conclave — and that probe's replies traverse our per-keystroke IPC path (§1 rows 12–13).

Consequence: the screenshot is not explained by "Claude Code renders differently for an unknown
terminal". It has to come from **our side of the same bytes** — the resize/mount lifecycle, the
data/stdin paths, or the renderer build. §3 ranks those.

## 1. Diff table

Columns: area · Conclave (file:line) · VS Code (`vscode:file:line`) · symptom? · recommendation ·
effort.

| # | Area | Conclave | VS Code | Symptom? | Rec. | Effort |
|---|---|---|---|---|---|---|
| 1 | PTY env `TERM_PROGRAM` / `TERM_PROGRAM_VERSION` | Not set. Only `TERM=xterm-256color`, `COLORTERM=truecolor` (`src-tauri/src/engine/runtime/pty.rs:75-76`) | `env['TERM_PROGRAM']='vscode'`, `TERM_PROGRAM_VERSION=<version>` (`vscode:src/vs/workbench/contrib/terminal/common/terminalEnvironment.ts:63-65`), `COLORTERM=truecolor` (`:70`); pty name `xterm-256color` (`vscode:src/vs/platform/terminal/node/terminalProcess.ts:152-155`) | **indirect** — with it unset Claude Code's `aO()` sync-output allowlist misses and it falls back to the DECRQM(2026) probe (§2 S1); DECSTBM stays gated either way (S3) | adopt an identity (`TERM_PROGRAM=Conclave`, `TERM_PROGRAM_VERSION=<app ver>`) — **not** `vscode`: that flips VS Code-only quirks (S6 bidi, S9 hyperlink click, S11 OSC52 range). For claude-code add `CLAUDE_CODE_FORCE_SYNC_OUTPUT=1` (S1 line 3) so sync is on from frame 1 | S |
| 2 | PTY env `LANG` / `LC_*` | `LANG=en_US.UTF-8` only if unset (`pty.rs:77-79`); `LC_*` untouched | `detectLocale='auto'` (default, `vscode:src/vs/workbench/contrib/terminal/common/terminalConfiguration.ts:322`): set `LANG` when missing **or not UTF-8** (`terminalEnvironment.ts:99-108`), value from locale, fallback `en_US.UTF-8` (`:110-115`) | no (both give UTF-8 here; `env` shows `LANG=en_US.UTF-8`) | adopt the "not UTF-8" check (`lang.search(/\.UTF-8$/)`) | S |
| 3 | `convertEol` | `true` (`src/components/Terminal.tsx:86`) | not passed (`vscode:src/vs/workbench/contrib/terminal/browser/xterm/xtermTerminal.ts:241-285`); xterm default `false` (`node_modules/@xterm/xterm/src/common/services/OptionsService.ts:57`) | **no** — see F1 (`onlcr` on) | adopt (remove) — zero risk, kills a false lead | S |
| 4 | Unicode width + glyph rescale | Unicode11 addon, `activeVersion='11'` (`Terminal.tsx:116-117`); `rescaleOverlappingGlyphs:true` (`:101`) | `unicodeVersion` default `'11'` (`terminalConfiguration.ts:532`), addon loaded lazily (`xtermTerminal.ts:1148-1157`); `rescaleOverlappingGlyphs` default `true` (`terminalConfiguration.ts:587`, passed `xtermTerminal.ts:274`) | no (identical) | skip | – |
| 5 | WebGL + `customGlyphs` + fallback | `new WebglAddon({customGlyphs:true})` after `open()`, `onContextLoss → dispose` (DOM fallback), `catch {}` silent (`Terminal.tsx:128-138`) | `gpuAcceleration:'auto'` (`terminalConfiguration.ts:333`), `customGlyphs` default `true` (`:574`); addon loaded after `open` (`xtermTerminal.ts:505-512`), `onContextLoss → _disposeOfWebglRenderer` **logged** (`:941-944`), load failure logged + `_suggestedRendererType='dom'` (`:955-959`) | **indirect** — a silent fallback means nobody knows which renderer painted the screenshot | adopt: log the fallback + expose active renderer (needed by §3 H2's experiment) | S |
| 6 | xterm / addon versions | `@xterm/xterm 6.1.0-beta.287`, webgl `0.20.0-beta.286`, unicode11 `0.10.0-beta.287`, serialize `0.15.0-beta.287`, fit `^0.11.0` (`package.json:21-25`) | xterm `^6.1.0-beta.303`, webgl `^0.20.0-beta.299`, unicode11/serialize `^…-beta.300`, headless `^6.1.0-beta.302` (`vscode:package.json:137-146`), node-pty `1.2.0-beta.15` (`:159`) | **yes (renderer)** — see §1a: the core delta is nil, the **webgl** delta contains three "stale rendering" fixes | adopt VS Code pins (exact, suffix-matched per memory c427347d) | S–M |
| 7 | Resize: debounce + axis split | ResizeObserver → 120 ms → `fit()` → `session.resize` (`Terminal.tsx:262-265`, `:189-232`); both axes together | `TerminalResizeDebouncer`: immediate when normal buffer < 200 rows, else rows **immediately** and cols after 100 ms (`vscode:src/vs/workbench/contrib/terminal/browser/terminalResizeDebouncer.ts:11-17`, `:43-88`); hidden terminals resize in idle callbacks (`:60-81`) | no | skip (ours is simpler and equivalent for a TUI) | – |
| 8 | Resize: order xterm vs PTY, clamping, pixel size | `fit()` (xterm) then IPC resize (`Terminal.tsx:202-219`); PTY `resize(PtySize{rows,cols,0,0})` (`pty.rs:155-165`); guards `cols===0||rows===0` (`Terminal.tsx:210`) | `xterm.resize(cols,rows)` then `_updatePtyDimensions` (`vscode:src/vs/workbench/contrib/terminal/browser/terminalInstance.ts:830-831`) which also sends **pixel** width/height (`:2095-2104`); `terminalProcess.resize` clamps `≥1` (`vscode:src/vs/platform/terminal/node/terminalProcess.ts:544-548`) | no (same order) | adopt pixel size (needed for Claude Code image/“getCellSizePixels” paths; cheap) | S |
| 9 | Resize: mount **jiggle** | First sizing after mount sends `rows-1` then `rows` 60 ms later, **xterm itself stays at `rows`** (`Terminal.tsx:215-231`); respawn re-arms after 300 ms (`:285-291`) | **never jiggles**; the same xterm object is re-attached and `setVisible` flushes the debouncer + `_resize()` (`terminalInstance.ts:1441-1453`). Grep `jiggle\|rows - 1` over the reference terminal dirs: no hits | **yes** — for 60 ms the app believes the terminal is `rows-1` tall while xterm is `rows`; Claude Code positions with **relative** CUU/CUD clamped to its own `rows-1` (§2 S13, `pW`) so its cursor model and xterm's cursor can diverge until its next full repaint | needs-measurement (§3 H1 experiment) then adopt: resize xterm to the same dims you push to the PTY, or nudge with a same-size `SIGWINCH` from the Rust side | S |
| 10 | Mount: initial xterm size + 200 ms window | New xterm is created at the default 80×24; `fit()` first runs after 200 ms (`Terminal.tsx:277-280`); the PTY stays at e.g. 153×55 meanwhile; snapshot restore writes into the 80-col grid (`:148-163`) | xterm created with `cols/rows` (`xtermTerminal.ts:243-244`) and attached before the process launches; process data before xterm is ready is **buffered** and replayed (`terminalInstance.ts:1586`, `_initialDataEvents`) | **yes** — any frame written in that window wraps at 80 cols and moves xterm's cursor somewhere Claude Code's model does not expect (H1) | adopt: pass the PTY's last known `cols/rows` to the constructor; queue output until `termRef` is set | M |
| 11 | Output data path: coalescing / flow control | Reader thread 4 KB reads → `decode_with_carry` → bounded channel (1024) → forwarder → one Tauri `session:output` **per chunk**, emit errors ignored (`pty.rs:116-130`, `:31`; `src-tauri/src/engine/commands/instance.rs:1466-1477`, `:1519-1529`; `src-tauri/src/engine/bus.rs:208-219`); frontend `listen()` → `term.write(chunk)` (`src/ipc/events.ts:193-195`, `Terminal.tsx:333-336`) | `TerminalDataBufferer` joins chunks for 5 ms then one write (`vscode:src/vs/platform/terminal/common/terminalDataBuffering.ts:28-51`, wired `vscode:src/vs/platform/terminal/node/ptyService.ts:819-820`); ack-based flow control pauses the pty above 100 000 unacked chars, resumes below 5 000 (`terminalProcess.ts:323-330`, `:579-588`; constants `vscode:src/vs/platform/terminal/common/terminal.ts:876-897`); xterm write callback acks (`vscode:src/vs/workbench/contrib/terminal/browser/terminalInstance.ts:1700-1708`) | no for correctness (ordering is a single tokio task → single `app.emit` sequence); **indirect** for tearing when sync output is off (per-chunk paints) | needs-measurement: add a `seq` field to `SessionOutput` and assert monotonic in `useSessionOutput` (10 lines) before spending on buffering | L (full) / S (probe) |
| 12 | Stdin path: ordering | Every `onData` → separate `ipc.message.send` (`Terminal.tsx:236-241`) → `message::send` does a DB read, two eligibility checks, an RwLock + Mutex **before** `send_stdin` (`src-tauri/src/engine/commands/message.rs:76-93`; `src-tauri/src/engine/runtime/mod.rs:405-415`) | `xterm.raw.onData → _handleOnData → processManager.write → _process.input` on one ordered channel (`terminalInstance.ts:883-885`, `:676-679`; `vscode:src/vs/workbench/contrib/terminal/browser/terminalProcessManager.ts:651-665`) | **indirect** — two replies xterm emits back-to-back (XTVERSION reply, then the DA1 barrier `CSI c`) can overtake each other; Claude Code's querier resolves XTVERSION as "no reply" when DA1 arrives first (§2 S4) → no DECRQM(2026) → sync output **off for the session**; the late DCS then lands as keystrokes | adopt: keystrokes bypass the DB/eligibility awaits, or the frontend chains sends per session (`prev.then(() => send)`) | S (chain) / M (Rust) |
| 13 | Stdin path: query replies before listener attach | `listen()` is async (`events.ts:193-203`); output emitted before it resolves is lost (documented at `Terminal.tsx:267-276`); a Claude Code launched while its tab is unmounted (remount mode, `src/components/WorkspacePane.tsx:898-900`) never sees its XTVERSION query answered | xterm attached before launch; early data buffered (`terminalInstance.ts:1586`) | **indirect** (same consequence as row 12: no sync output, plus Claude Code's `tengu_terminal_probe` records `xtversion:"no_reply"`) | adopt row 10's queue; or answer XTVERSION/DA1/DECRQM from Rust when no terminal is attached | M |
| 14 | Scrollback / reflow | `scrollback:12000` (`Terminal.tsx:94`); serialize cap 12000 (`:313-317`) | default 1000 (`terminalConfiguration.ts:317`) | no (reflow cost only) | skip | – |
| 15 | Wheel on alt buffer | Wheel → 3× arrow keys when `buffer.active.type==='alternate'` and no mouse tracking (`Terminal.tsx:251-258`) | none — grep `alternateScroll\|1007\|wheel.*arrow` over `vscode:src/vs/workbench/contrib/terminal/browser/**` = 0 hits; xterm.js has no DECSET 1007 (`grep -c 1007 lib/xterm.js` = 0). VS Code only classifies wheel events for smooth scrolling (`xtermTerminal.ts:527-535`) | no (Claude Code enables mouse tracking "full" — §2 S15 — so the handler is inert for it) | skip (keep for Codex) | – |
| 16 | `windowOptions` | not set (`Terminal.tsx:85-102`) | `getWinSizePixels/getCellSizePixels/getWinSizeChars: true` (`xtermTerminal.ts:280-284`) | no (no `CSI 14/16/18 t` found in the binary) | adopt (cheap, future-proof) | S |
| 17 | `ignoreBracketedPasteMode` / `macOptionIsMeta` / `altClickMovesCursor` | xterm defaults (false / false / true) | defaults false (`terminalConfiguration.ts:652`), false (`:143`), true (`:153`), passed at `xtermTerminal.ts:273`, `:265`, `:246` | no | skip | – |
| 18 | Font measurement | `fontSize 12`, `fontFamily "ui-monospace, SFMono-Regular, Menlo, monospace"` (`Terminal.tsx:87-88`); no `letterSpacing`/`lineHeight` (defaults 0 / 1) | `fontFamily` falls back to `editor.fontFamily` → `EDITOR_FONT_DEFAULTS` (Menlo on mac) (`vscode:src/vs/workbench/contrib/terminal/browser/terminalConfigurationService.ts:135`); `letterSpacing 0`, `lineHeight 1` on mac (`vscode:src/vs/workbench/contrib/terminal/common/terminal.ts:30`, `:37`), re-applied only when visible (`vscode:src/vs/workbench/contrib/terminal/browser/terminalInstance.ts:2059-2078`) | no | skip | – |
| 19 | Other xterm options | `cursorBlink:true`; `minimumContrastRatio` default 1; `scrollOnEraseInDisplay` default false | `cursorBlinking` default false (`terminalConfiguration.ts:292`); `minimumContrastRatio 4.5` (`:227`); `scrollOnEraseInDisplay:true` hard-coded (`vscode:src/vs/workbench/contrib/terminal/browser/xterm/xtermTerminal.ts:270`); `drawBoldTextInBrightColors true` (`:249`) | no | adopt `scrollOnEraseInDisplay` (keeps transcript on `ESC[2J`) | S |
| 20 | Kitty keyboard / win32-input | not set | `vtExtensions.kittyKeyboard` default **true** (`terminalConfiguration.ts:592`), win32InputMode false (`:599`), passed `xtermTerminal.ts:275-278` | no (Claude Code only pushes kitty flags for an allowlist that excludes both — §2 S16) | skip | – |
| 21 | Tab switch lifecycle | remount mode (default, `src/lib/termMode.ts:13`): inactive tab's xterm is **disposed**, buffer restored from a serialize snapshot on return (`Terminal.tsx:148-163`, `:311-320`); keep-alive mounts all (`WorkspacePane.tsx:898-900`) | one long-lived xterm per terminal, re-attached (`terminalInstance.ts:1441-1453`) | **indirect** — every remount replays rows 9–10 and 13 | adopt keep-alive semantics for the *xterm object* (dispose only on session end) — this is the structural fix behind rows 9/10/13 | M |

### 1a. xterm.js delta beta.287 → beta.303 (2026-06-13 → 2026-08-24, `npm view @xterm/xterm time`)

Core files diffed against upstream `0659493` (the beta.303 head): `src/browser/services/RenderService.ts` **identical**,
`src/browser/RenderDebouncer.ts` **identical**, `src/browser/CoreBrowserTerminal.ts` +1 line
(`autocomplete=off`, PR #6057), `src/common/InputHandler.ts` +5 lines (ESC[3J on the alt screen no
longer resets `isUserScrolling`, PR #6081). No parser, reflow or DECSET-2026 changes. The
synchronized-output refresh buffering (`RenderService.ts:156-203`, 1 s safety timeout `:22`)
is the same in both builds.

Merged PRs in the window that touch rendering (`gh pr list --state merged --search "merged:2026-06-13..2026-08-25"`):

| PR | merged | what | ships in |
|---|---|---|---|
| [#6042](https://github.com/xtermjs/xterm.js/pull/6042) | 2026-07-13 | Fix **stale rendering** after shared texture-atlas page merges (`pageLayoutVersion` per `GlyphRenderer`) — issue [#6038](https://github.com/xtermjs/xterm.js/issues/6038): "buffer content remains correct, but the renderer can draw with stale glyph/page state" | webgl ≥ beta.289 |
| [#6043](https://github.com/xtermjs/xterm.js/pull/6043) | 2026-07-15 | Prevent WebGL atlas pages exceeding texture capacity (render-loop exception on overflow) | webgl ≥ beta.290 |
| [#6055](https://github.com/xtermjs/xterm.js/pull/6055) | 2026-07-27 | Fix stale rendering after a shared atlas is **cleared** (issue [#6014](https://github.com/xtermjs/xterm.js/issues/6014): sibling terminals "garbled or blank until a resize") | webgl ≥ beta.292 |
| [#6081](https://github.com/xtermjs/xterm.js/pull/6081) | 2026-08-10 | Viewport pinned to top after ESC[3J | core beta.302 |
| [#6114](https://github.com/xtermjs/xterm.js/pull/6114) | 2026-08-20 | BufferLine perf (string cache, `copyFrom`/`clone`) — buffer internals, no semantics | core beta.303 |
| [#6128](https://github.com/xtermjs/xterm.js/pull/6128) | 2026-08-24 | Pattern custom glyphs (`░▒▓`) could throw and **stop the WebGL render loop** in a foreign document | webgl beta.299 |

Our `@xterm/addon-webgl 0.20.0-beta.286` has zero occurrences of `pageLayoutVersion`
(`grep -c` on `node_modules/@xterm/addon-webgl/lib/addon-webgl.js`), i.e. it predates all
three atlas fixes. Caveat for #6042/#6055: they need two live renderers sharing one atlas;
in remount mode only one `<Terminal>` is mounted (`WorkspacePane.tsx:898-900`), but
`SkillAssistPanel.tsx:89` creates a second xterm and keep-alive mode mounts every tab.

## 2. Claude Code 2.1.261 — every branch keyed on `TERM_PROGRAM` / `TERM` / `COLORTERM`

Extraction: `strings -n 8 ~/.local/share/claude/versions/2.1.261 | grep -o '.{160}TERM_PROGRAM.{160}'`
(30 hits) plus neighbours. Minified identifiers are per-chunk; `a` = env accessor, `sk()` =
terminal-probe state, `CT` = quirks table. **Rendering-relevant rows are bold.** "unset" = what
Conclave gets today.

| # | Snippet (≤200 chars) | unset (Conclave) | `vscode` | touches |
|---|---|---|---|---|
| **S1** | `function aO(){…if(a.CLAUDE_CODE_FORCE_SYNC_OUTPUT)return!0;let t=a.TERM_PROGRAM,r=a.TERM;if(t==="iTerm.app"\|\|t==="WezTerm"\|\|…\|\|t==="vscode"\|\|…)return!0;…if(sk().synchronizedOutputSupported)return!0;return!1}` | false at startup; true only after the DECRQM(2026) probe (S4) succeeds | true from startup | **synchronized output** (`CSI ?2026 h/l` around every frame, `Dtn()`/`syncViewport`) |
| **S2** | `function aDt(){if(a.CLAUDE_BG_BACKEND==="daemon")return!1;return aO()&&a.TMUX==null&&process.env.ZELLIJ==null&&!CT.isJetBrainsIdeTerminal()&&!Zd()&&a.WT_SESSION==null…}` and `var cDt=aDt();` (module load) | false (`aO()` false at load; `Zd()` true after XTVERSION) | false (`Zd()` true via TERM_PROGRAM) | **DECSTBM scroll regions**: `this.log.render(L,x,this.altScreenActive,cDt&&!this.altScreenFullRepaint)`; debug line `DECSTBM: ${cDt?"enabled":"gated"} (… TERM_PROGRAM=… TERM=…)` |
| **S3** | `function Zd(){if(dl()?.isVscodeTerm)return!0;if(a.TERM_PROGRAM==="vscode")return!0;return sk().xtversionName?.startsWith("xterm.js")??!1}` | true once XTVERSION answered (xterm.js replies `xterm.js(6.1.0-beta.287)`) | true | is-xterm.js flag feeding S2, S9, S10 |
| **S4** | `async function qv(t){let[s]=await Promise.all([t.send(fBn()),t.flush()]);…let c=!s\|\|a.TERM_PROGRAM==="Apple_Terminal",[f]=await …t.send(uBn(zf.SYNCHRONIZED_UPDATE))…m=f?.status===1\|\|f?.status===2;QUn(m)` — `fBn()={request:CSI >0q}`, `flush()` writes `CSI c` (DA1) as the barrier; when the DA1 reply arrives, every still-unanswered query queued before the sentinel resolves `undefined` (verified: `Utn.onResponse`: `if(e.type==="da1"){…for(let t of this.queue.splice(0,r+1))if(t.kind==="query")t.resolve(void 0)…}`); xterm.js answers DA1 with `CSI ?1;2c` | XTVERSION → DA1 → DECRQM(2026) → xterm answers `?2026;2$y` (`InputHandler.ts:2387`) → supported — **only if our stdin path delivers the two replies in order** (§1 row 12) and the query bytes reached a mounted xterm (row 13) | probe still runs (not Apple_Terminal), but S1 already true | **whether S1 ever turns on** |
| S5 | `decstbmRendererEnabled … if(!aDt())return …=!1; … if(Ie(a.CLAUDE_CODE_DECSTBM))return …=!0;return …=H("tengu_marlin_porch",!1)` | off | off | the newer `j8` viewport renderer (uses `CSI t;b r`) — feature-flagged off for everyone; irrelevant |
| S6 | `class rg{…isNeeded(){…this.needed=typeof process.env.WT_SESSION==="string"\|\|a.TERM_PROGRAM==="vscode"}}` → `og(t)` reorders runs by `getEmbeddingLevels(s,"auto")` | no bidi reordering | bidi visual reordering of RTL runs | rendering of RTL text only (Thai is LTR) |
| S7 | `function le(){if(a.TERM_PROGRAM==="vscode"&&u.level===2)return u.level=3,!0;return!1}` and supports-color: `if(H.COLORTERM==="truecolor")return 3;if("TERM_PROGRAM"in H){…case"iTerm.app":return i>=3?3:2;case"Apple_Terminal":return 2}` | level 3 via `COLORTERM` | level 3 | colour depth only |
| S8 | `function $it(){…if(a.TERM_PROGRAM==="Apple_Terminal"\|\|e==="linux")return!1;return z.has(a.TERM_PROGRAM??"")\|\|CT.isGhostty()\|\|CT.isMintty()\|\|CT.isJetBrainsIdeTerminal()\|\|a.LC_TERMINAL==="iTerm2"` (`z` = vscode, WezTerm, WarpTerminal, Hyper, Tabby, rio, contour, alacritty) | strikethrough (SGR 9) **not emitted** | emitted | SGR 9 only (no cursor/width effect) |
| S9 | `if(S&&a.TERM_PROGRAM!=="vscode"&&!Zd()&&((s.button&24)!==0\|\|CT.macCmdClickArrivesWithoutSgrModifierBit()\|\|KUn()))` | hyperlink click needs modifier (xterm.js path) | same (vscode path) | mouse click handling |
| S10 | `function Htn(){if(process.env.CURSOR_TRACE_ID!==void 0)return!0;…if(a.TERM_PROGRAM==="vscode"){let t=Q(a.TERM_PROGRAM_VERSION);if(t!==null)return t>=1092000&&t<1105000}return Dat()?.startsWith("xterm.js")??!1}` | `wheelFlood=true` (xterm.js) → decay-curve wheel handling (`useDecayCurve`, `useAdaptiveDrain`) | false for VS Code ≥1.105 | wheel scrolling feel only |
| S11 | `hasOsc52ClipboardUtf8Bug(){…TERM_PROGRAM_VERSION…return r!==null&&r>=1123000&&r<1125000}`; `supportsVirtualTerminalSequences(){…win32&&TERM_PROGRAM==="vscode"&&TERM_PROGRAM_VERSION…}`; `isGhostty(){TERM==="xterm-ghostty"\|\|TERM_PROGRAM==="ghostty"}`; `isMintty(){TERM_PROGRAM==="mintty"…}`; `rendersItalicAsStandout(){(TERM??"").startsWith("screen")}` | all false | OSC52 bug only for 1.123–1.124 | clipboard / italics / Windows |
| S12 | `function Tf(r){…if(r?.stdoutSupported??supportsHyperlink(process.stdout))return!0;let p=e.TERM_PROGRAM;if(p&&h.includes(p))return!0;…` | OSC 8 hyperlinks off | on | OSC 8 emission only |
| **S13** | renderer ops (`Dtn`): `case"cursorMove":i+=pW(l.x,…l.y);case"cursorTo":i+=$ie(l.col);case"clear":i+=axt(l.count);case"clearTerminal":i+=l.altScreen?JUn():Lat(l.viewportRows)` with `pW(t,e)=CUB/CUF(t)+CUU/CUD(e)`, `$ie(t)=CSI t G`, `axt(n)=(EL2+CUU1)×n+CSI G`, `Lat(r)=CUP+(EL2+CUD1)×r+CUP`; on the normal screen the cursor is moved **relatively** from `this.displayCursor` (`U.unshift({type:"stdout",content:pW(L.cursor.x-ce.x,Se(L.cursor.y-ce.y))})`, `Se` clamps to `±(rows-1)`) | same | same | **cursor positioning** — not keyed on TERM_PROGRAM, but it is why any xterm/PTY size disagreement or lost chunk persists (§3 H1) |
| S14 | `n(\`DECRQM(2026): ${…} → sync ${m?"supported":"unsupported"}\`),n(\`DECSTBM: ${cDt?"enabled":"gated"} (TMUX=… ZELLIJ=… TERM_PROGRAM=${a.TERM_PROGRAM??"unset"} TERM=${a.TERM??"unset"})\`)` | logged with `--debug-file <path>` (flag present in the binary) | same | **the oracle** R2/F should read |
| S15 | `function xH(){if(a.CLAUDE_CODE_DISABLE_MOUSE!==void 0)…;return"full"}` → `CSI ?1000h ?1002h ?1003h ?1006h` | mouse tracking on | on | makes `Terminal.tsx:251-258` wheel translation inert for Claude Code |
| S16 | `var ee=["iTerm.app","kitty","WezTerm","ghostty","tmux","windows-terminal","WarpTerminal"];function lDt(t){return ee.includes(t??a.terminal??"")}` → kitty keyboard flags `CSI >5u` | not pushed | not pushed | keyboard protocol |
| S17 | `xo(){switch(a.TERM_PROGRAM){case"vscode":…}}`, `kn()`/`F()` deep-link terminal from `TERM_PROGRAM`, `SN()` iTerm2 detection, `f()` tmux control mode (`TERM_PROGRAM!=="iTerm.app"`), `b$()` keyboard layout, `sat()/hle()` spinner sets for `TERM==="xterm-ghostty"` | telemetry / defaults | — | none |

Not found in the binary: any `CSI 6n` (cursor position report) use, any `CSI 14/16/18 t`
window query, any `eraseLines` (Ink's classic full-redraw). Claude Code never asks the
terminal where its cursor is — it trusts its own `displayCursor` (S13).

**"Unknown terminal" path, summarised:** identical bytes to VS Code minus SGR 9 and OSC 8,
plus synchronized output that starts only after the runtime probe (S4) — and is **absent for
the whole session** if that probe's replies are lost or reordered (§1 rows 12–13).

## 3. Ranked hypotheses for the screenshot

The screenshot (`docs/superpowers/plans/assets/2026-09-05-xterm-autocomplete-garble.png`)
shows three rows whose *first segment sits at column 0* while the rest of the row is stale:
the prompt row reads `t /s at` (the last typed `t` at col 0 where `>` belongs), popup row 1
`statusli … S  Toggle th`, popup row 4 `ny-orchestrate-subagents` flush-left while rows 2, 3,
5 keep their `  /` indent. Coherent character runs relocated to col 0 is the signature of
Claude Code's **relative-move renderer (S13) writing from a wrong `displayCursor`**, which
lives in the *buffer*; cell-level stale pixels with a correct buffer is the signature of a
*renderer* fault. R2's headless replay separates the two.

### H1 — cursor-model desync from a Conclave-only xterm/PTY size or data gap (rows 9, 10, 13, 21; S13)

Mechanism: Claude Code positions every patch relative to where it believes the cursor is and
clamps vertical moves to its own `rows-1`. Any window in which xterm's grid ≠ the PTY's grid
(new xterm at 80×24 for ≥200 ms while the PTY is 153×55; the jiggle's `rows-1` while xterm
stays at `rows`), or in which a chunk never reaches xterm (listener-attach gap, ignored emit
error), leaves xterm's cursor somewhere else. From then on **every relative move lands
offset** — patches whose target column equals the believed column emit no horizontal move and
land at col 0 — until the next full repaint. VS Code has none of these windows (rows 9, 10,
13, 21). Evidence weight: the col-0 signature; three independent Conclave-only windows; F2
(the bytes are otherwise the same).

Experiment for R2 (one run, two replays of the same recording): record a scripted session at
153×55 that types `/s at`. Replay A: headless xterm at 153×55 for the whole stream (our
options). Replay B: headless xterm created at **80×24**, the first 200 ms of bytes written,
then `resize(153,55)`, then a PTY-side `rows-1`/`rows` pair (write nothing; just note the
app's SIGWINCH repaints are already in the stream), then the rest. If A is clean and B shows
col-0 rows → H1 confirmed and lane F's fix is rows 9+10 (+21). If A itself shows col-0 rows →
the bytes are wrong on their own; go to H3's sync check and to the Claude Code side.

### H2 — renderer-side stale cells: WebGL addon beta.286 predates the atlas invalidation fixes (row 6, §1a)

Mechanism: issue #6038 / #6014 describe rows drawn with stale glyph/page state "while the
buffer content remains correct", under heavy TUI use, fixed in webgl beta.289–.292. Our addon
is beta.286. Precondition for #6042/#6055 is two renderers sharing one atlas
(`SkillAssistPanel` + the terminal, or keep-alive mode); #6043 is single-terminal but freezes
the whole pane rather than rows. Evidence weight: exact upstream match for "stale rows,
buffer fine"; the prompt row was live (cursor after `at`), which argues against a frozen
render loop and for a partial-model fault.

Experiment for R2: same recording as H1, replay A only, then diff the headless screen dump
against the screenshot rows. **Headless has no renderer**: if the dump is clean R2 must say
"not reproducible headless" and hand F the GUI probe — when the garble appears, run
`term.clearTextureAtlas()` (VS Code's `forceRedraw`, `vscode:src/vs/workbench/contrib/terminal/browser/xterm/xtermTerminal.ts:640-642`) or dispose
the WebGL addon; if the rows heal without new PTY output, H2 is confirmed and the fix is the
version bump (row 6).

### H3 — synchronized output never enabled (or torn) because the startup probe's replies were reordered or lost on our stdin/listener path (rows 12, 13; S1, S4)

Mechanism: xterm answers XTVERSION and the DA1 barrier from one parse pass; our path sends
them as two racing Tauri invokes, each awaiting a DB read and two eligibility checks before
`send_stdin`. If DA1 overtakes, Claude Code records "no XTVERSION reply", skips DECRQM(2026),
and renders the entire session **without** `CSI ?2026h/l` — every multi-chunk frame can be
painted mid-patch. The screenshot would then be a torn frame. Evidence weight: mechanism is
real and Conclave-only; but tearing self-heals on the next keystroke, so it explains a
*captured* frame better than a *persistent* one.

Experiment for R2: make the recorder a **terminal-in-the-loop** (feed the PTY output through
`@xterm/headless` and write its `onData` replies back to the PTY, so XTVERSION/DA1/DECRQM are
answered exactly as our app does) and launch `claude --debug-file <scratch>`; assert the debug
file contains `DECRQM(2026): status=2 → sync supported` and the recording contains
`\x1b[?2026h` before the popup frames. Then re-run with the replies delayed/reordered
(write DA1's reply first) and confirm the debug file flips to `skipped (no XTVERSION reply)`.
Lane F can check live sessions with `term.modes.synchronizedOutputMode`
(`node_modules/@xterm/xterm/src/browser/public/Terminal.ts:122`) toggling during output.

Dropped: `convertEol` (F1), DECSTBM gating (S2 — same for VS Code), Unicode/width tables
(row 4 identical), xterm **core** parser/reflow deltas (§1a nil).

## 4. Recommended change set for lane F (smallest first)

1. **Remove `convertEol: true`** — `Terminal.tsx:86`; mirror `vscode:…/xtermTerminal.ts:241-285` (option absent). S. Safe by F1; removes a false lead.
2. **Env identity + sync output from frame 1** — `pty.rs:75-79`: add `TERM_PROGRAM=Conclave`, `TERM_PROGRAM_VERSION=<app version>` (mirror `terminalEnvironment.ts:63-65`, keep the UTF-8 `LANG` check from `:99-108`), and for `cliKind == claude-code` add `CLAUDE_CODE_FORCE_SYNC_OUTPUT=1` (S1). S. Do **not** claim `vscode` (S6/S9/S11).
3. **Log the renderer fallback and expose the active renderer** — `Terminal.tsx:128-138`; mirror `xtermTerminal.ts:941-944`, `:955-959`. S. Prerequisite for H2's GUI probe.
4. **Bump xterm + addons to VS Code's pins** — `package.json:21-25` → `vscode:package.json:137-146` (`6.1.0-beta.303` / webgl `0.20.0-beta.299` / unicode11+serialize `-beta.300`, exact, suffix-matched). S–M; run `pnpm install` in the lane only. Closes H2 regardless of ranking.
5. **Create the xterm at the PTY's size and queue early output** — `Terminal.tsx:85`, `:277-280`, `:333-336`; mirror `xtermTerminal.ts:243-244` (constructor `cols/rows`) and `terminalInstance.ts:1586` (`_initialDataEvents`). M. Kills row 10's window and row 13's lost queries.
6. **Make the jiggle self-consistent** — `Terminal.tsx:215-231`: resize xterm to `rows-1` before pushing `rows-1` to the PTY (or replace the jiggle with a same-size SIGWINCH from `pty.rs:155-165`). VS Code's contract: xterm and PTY are always resized together (`terminalInstance.ts:830-831`). S.
7. **Order the stdin path** — `message.rs:76-93`: for the raw keystroke path call `send_stdin` before the eligibility bookkeeping, or chain sends in `Terminal.tsx:236-241`; mirror `terminalInstance.ts:676-679` → `terminalProcessManager.ts:651-665`. S (chain) / M (Rust). Closes H3.
8. **Send pixel dimensions + `windowOptions`** — `pty.rs:158-163` (`pixel_width/height`), `Terminal.tsx:85`; mirror `terminalInstance.ts:2095-2104`, `xtermTerminal.ts:280-284`. S.
9. **`scrollOnEraseInDisplay: true`** — mirror `xtermTerminal.ts:270`. S, cosmetic.
10. **Keep the xterm object alive across tab switches** (dispose only on session end) — `termMode.ts:13`, `WorkspacePane.tsx:898-900`, `Terminal.tsx:293-322`; mirror `terminalInstance.ts:1441-1453`. M. Structural fix that makes 5 and 6 unnecessary; do it last, after H1 is confirmed.
11. **Output coalescing + flow control** (`terminalDataBuffering.ts:28-51`, `terminalProcess.ts:323-330`, `:579-588`) — L; only after the `seq` probe (row 11) shows a need.

## 5. Verification trail

- Termios: `stty -a -f /dev/ttys007` (this agent's Conclave PTY, claude 2.1.261 attached), 2026-09-05.
- xterm core diff: `curl` of `RenderService.ts`, `RenderDebouncer.ts`, `CoreBrowserTerminal.ts`, `InputHandler.ts` at `0659493` vs `node_modules/@xterm/xterm/src/...`; `diff` outputs quoted in §1a.
- Version dates: `npm view @xterm/xterm time --json` (beta.287 = 2026-06-13, beta.303 = 2026-08-24).
- PR/issue text: `gh pr view 6042 6043 6055 6114 6128`, `gh issue view 6038 6014` (xtermjs/xterm.js), `gh issue view 322756` (microsoft/vscode).
- Binary: `strings -n 8` scratch copy; snippets quoted verbatim (minified names are chunk-local; `Zd`/`aO`/`aDt`/`cDt` all come from `chunk-rvxxpz38.js`, confirmed by its import list).
- Assumption not verified here: the human's statement that Terminal.app/iTerm2/VS Code render this popup cleanly on the same build — R2's terminal-in-the-loop recording can be replayed with `TERM_PROGRAM=Apple_Terminal` to check the "probe skipped" path if that ever needs confirming.
