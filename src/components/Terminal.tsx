import { useEffect, useRef } from "react";
import { Terminal as XtermTerminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { SerializeAddon } from "@xterm/addon-serialize";
import "@xterm/xterm/css/xterm.css";
import { ipc, useSessionOutput, useSessionStatus } from "../ipc";
import { useFileDrop, shellQuotePath } from "../lib/fileDrop";
import { getTermTabMode } from "../lib/termMode";

interface TerminalProps {
  sessionId: string;
}

// Pre-remount buffer snapshots, keyed by sessionId. In "remount" tab mode
// (src/lib/termMode.ts) an inactive tab's <Terminal> unmounts; on the next mount
// the saved buffer is written back above a dim divider so the earlier context is
// not lost. This store lives OUTSIDE the component so it survives remounts, and
// it dies on a page reload — the SAME lifetime as the pre-remount hidden-tab
// approach (a reload always started every tab black), so this is no regression.
// Bounded by session count (≤ live agents) × `scrollback` rows per entry; no cap
// needed. In "keep-alive" mode BOTH the save and the restore are gated off (see
// the isRemount checks in the effect), so no snapshot is ever written and a
// terminal that unmounts — which DOES happen in keep-alive too: WorkspacePane is
// keyed `${workspaceId}:${agentsVersion}` (AppShell.tsx:288) and the LaneBoard
// branch swaps the pane out, so a workspace switch / agent add-remove / LaneBoard
// visit remounts terminals — simply loses its buffer, exactly as it did before
// this feature. That loss IS the behavior keep-alive reverts to.
const snapshots = new Map<string, { data: string; cols: number }>();

// DEV-ONLY live-terminal registry for the GUI parity probe. The terminal defect
// this lane chases only ever appears in the real app, so the human needs a way
// to interrogate a live xterm from the devtools console:
//
//   [...window.__conclaveTerms.values()].map(t => t.modes.synchronizedOutputMode)
//   [...window.__conclaveTerms.values()].forEach(t => t.clearTextureAtlas())
//
// Rows that heal on `clearTextureAtlas()` without new PTY output are a renderer
// fault (H2); rows that stay are in the buffer (H1). `clearTextureAtlas` is what
// VS Code's own forceRedraw calls
// (vscode:src/vs/workbench/contrib/terminal/browser/xterm/xtermTerminal.ts:640-642).
// Guarded by `import.meta.env.DEV`, so Vite tree-shakes the whole thing out of a
// production build — the same pattern as `src/fixtures/`.
const devTerms: Map<string, XtermTerminal> | undefined = import.meta.env.DEV
  ? new Map<string, XtermTerminal>()
  : undefined;
if (import.meta.env.DEV && devTerms) {
  (window as unknown as { __conclaveTerms: Map<string, XtermTerminal> }).__conclaveTerms =
    devTerms;
}

/**
 * One live, INTERACTIVE xterm.js terminal bound to a single session.
 *
 * Output (PTY → xterm) streams in via `useSessionOutput`. Input (xterm → PTY)
 * goes through `term.onData`: every keystroke — including control sequences
 * (arrows, Enter as `\r`, Shift-Tab as `\x1b[Z`, Ctrl-C, etc.) — is forwarded
 * VERBATIM to the live PTY stdin via `message.send` (the backend writes the
 * bytes without appending a newline). This is what lets a full TUI like Claude
 * Code be driven directly in the pane.
 *
 * The instance is created ONCE per mount. Because the terminal is tied to a
 * `sessionId`, the call site passes `key={sessionId}` so a session switch
 * remounts the whole component (fresh terminal) instead of reusing this one.
 *
 * React 19 StrictMode double-invokes effects in dev as mount → cleanup →
 * mount. Cleanup disposes the first terminal before the second mount creates
 * its own, so only one terminal is ever alive into the div — the cleanup, not
 * the ref check, is what makes this safe.
 */
export function Terminal({ sessionId }: TerminalProps) {
  const divRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<XtermTerminal | null>(null);
  // Re-push the current size to the PTY (set inside the mount effect). A
  // RESPAWN on the same session id (restart · resume) creates a fresh PTY at
  // the 80×24 default while this component — keyed by sessionId — stays
  // mounted, so no ResizeObserver ever fires; without this the relaunched TUI
  // lays out at the wrong size until a manual window resize.
  const repushSizeRef = useRef<(() => void) | null>(null);
  // True once at least one live output chunk has been written during THIS mount.
  // Gates the snapshot save so we never overwrite a good buffer with an empty
  // one: (a) React 19 StrictMode's dev mount → cleanup → mount would otherwise
  // clobber the snapshot on the first (immediate, output-free) cleanup; (b) a
  // fast tab flip before any output arrives. Reset to false on each mount.
  const receivedOutputRef = useRef(false);
  // Output that arrived before this mount's xterm had its real size. Non-null
  // means "queue, do not write yet" (see the first-sizing block in the effect);
  // null means the grid is sized and writes go straight through.
  const pendingRef = useRef<string[] | null>(null);
  // Tail of the stdin chain — see `sendStdin`.
  const stdinChainRef = useRef<Promise<void>>(Promise.resolve());

  /**
   * Write to the session's PTY stdin, preserving EMISSION ORDER.
   *
   * `ipc.message.send` is a Tauri invoke that resolves a session from the DB
   * and runs two eligibility checks before it ever reaches `send_stdin`, so two
   * sends fired back-to-back can reach the PTY in either order. That is not
   * merely a typing-order nicety: xterm answers a terminal query burst — the
   * XTVERSION report and the DA1 barrier that follows it — from ONE parse pass,
   * as two `onData` calls in the same tick. If the DA1 reply overtakes the
   * XTVERSION reply, Claude Code concludes there was no XTVERSION reply at all,
   * skips its DECRQM(2026) check and runs the WHOLE session without
   * synchronized output, so every multi-chunk frame can be shown mid-patch
   * (audit §3 H3, rows 12-13).
   *
   * Chaining every write through one promise is VS Code's shape: `_handleOnData`
   * awaits `_processManager.write` (vscode:…/browser/terminalInstance.ts:676-679
   * → terminalProcessManager.ts:651-665), one ordered channel for keystrokes,
   * replies and pasted text alike. The chain never rejects — a failed send is
   * swallowed, exactly as before, so one dead invoke cannot wedge the rest.
   */
  const sendStdin = (text: string) => {
    stdinChainRef.current = stdinChainRef.current
      .then(() => ipc.message.send({ sessionId, text }))
      .then(
        () => {},
        () => {
          // Session not running / backend gone — the output stream will surface
          // the state; dropping the write is the right behavior. Swallowed here
          // so the next link still runs.
        },
      );
  };

  // Drag a file onto the terminal → type its shell-quoted path into the PTY at
  // the cursor (no submit), exactly like dropping a file into a real terminal.
  // Written straight to stdin via message.send; the running TUI echoes it.
  const { ref: dropRef, isOver } = useFileDrop<HTMLDivElement>((paths) => {
    sendStdin(paths.map(shellQuotePath).join(" ") + " ");
  });

  useEffect(() => {
    const el = divRef.current;
    if (!el) return;
    receivedOutputRef.current = false;
    // Snapshot save + restore run ONLY in remount mode. keep-alive must stay
    // byte-for-byte the pre-remount behavior: terminals still unmount on a
    // workspace/agents-version/LaneBoard change (see the snapshots comment), but
    // they must lose their buffer then, with no divider ever injected.
    const isRemount = getTermTabMode() === "remount";

    const term = new XtermTerminal({
      // NO `convertEol`. VS Code does not set it either
      // (vscode:src/vs/workbench/contrib/terminal/browser/xterm/xtermTerminal.ts:241-285)
      // and on our PTY it can only ever be a no-op or a hazard: the master keeps
      // `opost onlcr` on for the whole child lifetime (verified with
      // `stty -a -f /dev/ttysN` on a live Conclave-spawned `claude`, audit §0 F1),
      // so every `\n` the child writes already reaches xterm as `\r\n`.
      // `convertEol` only forces `x = 0` on an LF that was NOT preceded by a CR
      // (@xterm/xterm InputHandler.ts, `lineFeed`), which never happens here —
      // except for a child that deliberately disables ONLCR, where it would
      // silently rewrite that child's intended cursor column.
      fontSize: 12,
      fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
      theme: { background: "#1e1e1e", foreground: "#e5e5e5" },
      cursorBlink: true,
      // xterm defaults to 1000 rows of scrollback; agent sessions routinely
      // scroll far past that. Must stay ≥ the serialize cap in the unmount
      // snapshot below, or remounts silently keep less than the live buffer.
      scrollback: 12000,
      // Required by the Unicode 11 addon (it uses xterm's proposed API).
      allowProposedApi: true,
      // Rescale any glyph that renders wider than its cell back into the cell
      // (VS Code enables this by default). Without it a slightly-too-wide glyph
      // bleeds into the next cell and the bleed survives as a stray fragment —
      // the orange leftovers seen after a resize.
      rescaleOverlappingGlyphs: true,
    });
    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    // Serialize addon: snapshots the buffer on unmount and lets us write it back
    // on the next mount ("remount" tab mode). Loaded unconditionally and cheaply
    // — it needs to be present to save even if the mode is flipped later.
    const serializeAddon = new SerializeAddon();
    term.loadAddon(serializeAddon);

    // Unicode 11 width tables (like VS Code). xterm's built-in tables are
    // Unicode 6, which mis-measures the width of modern wide glyphs/emoji — a
    // 2-cell glyph counted as 1 cell leaves a trailing stray cell in the buffer
    // (the leftover fragments that survived even the GPU renderer). Activating
    // version 11 makes the buffer geometry match what the child emits.
    term.loadAddon(new Unicode11Addon());
    term.unicode.activeVersion = "11";

    term.open(el);

    // Use the GPU (WebGL) renderer, like VS Code's terminal. xterm's default DOM
    // renderer repaints rows incrementally and leaves stray cells behind when a
    // resize reflows the buffer (the orange fragments seen at certain widths);
    // the WebGL renderer repaints the whole grid each frame from the buffer, so
    // those artifacts can't survive. `open()` must run first (the renderer needs
    // the canvas). Fall back to the DOM renderer if WebGL is unavailable or its
    // GPU context is lost, so the terminal always renders something.
    try {
      // `customGlyphs` makes the renderer draw box-drawing and block characters
      // itself — crisp and exactly cell-sized — instead of relying on the font.
      // Claude Code's frame is built from those (the box borders + the block
      // mascot), so this is what keeps them aligned. VS Code enables it too.
      const webgl = new WebglAddon({ customGlyphs: true });
      // Mirror vscode:…/xtermTerminal.ts:941-944: a lost GPU context is a
      // SILENT downgrade to the DOM renderer, and the DOM renderer is the one
      // that strands stray cells on reflow. Say so, or a terminal that quietly
      // fell back looks like a fresh xterm bug the next time it garbles.
      webgl.onContextLoss((err) => {
        console.warn("[terminal] webgl context lost, falling back to DOM renderer", err);
        webgl.dispose();
      });
      term.loadAddon(webgl);
    } catch (err) {
      // No WebGL in this webview — the DOM renderer stays active.
      // Mirror vscode:…/xtermTerminal.ts:955-959.
      console.warn("[terminal] webgl renderer unavailable, DOM fallback", err);
    }

    term.focus();

    // ── First sizing, SYNCHRONOUSLY, before anything is written ─────────────
    // VS Code constructs its xterm already at the right size (it passes
    // cols/rows to the constructor, vscode:…/xterm/xtermTerminal.ts:243-244), so
    // no byte is ever parsed against a grid the PTY does not have. We only learn
    // our size from the DOM, so we do the equivalent: fit here, before the
    // snapshot restore and before `termRef` starts accepting live output.
    //
    // Why it matters: Claude Code positions every patch RELATIVE to its own
    // idea of the cursor and never asks the terminal where the cursor is (audit
    // S13). It used to be ~200 ms before we fitted, so a fresh mount parsed a
    // real 153x55 frame into an 80x24 grid; every wrapped row landed somewhere
    // the child did not intend, and from then on relative moves whose target
    // column already matched the believed column emitted no horizontal move and
    // landed at column 0 — the col-0 rows in the reported screenshot (audit §3
    // H1). Fitting after the WebGL addon is loaded is deliberate: the GPU
    // renderer's cell metrics differ from the DOM renderer's, which is why VS
    // Code refreshes dimensions right after loading it (xtermTerminal.ts:947).
    let firstFitDone = false;
    const tryFit = (): boolean => {
      // A hidden tab (display:none) has no used layout: getBoundingClientRect
      // reads the USED width, which is genuinely 0 there — unlike FitAddon's
      // proposeDimensions, which reads getComputedStyle, sees the COMPUTED
      // '100%', parseInt's it and gets 100(px). That misread never rounds down
      // to 0 cols, so it would slip past the cols/rows===0 guard and push a
      // bogus ~11-col resize to the live PTY every time a tab is switched away
      // from — the child then rewraps its transcript at 11 cols and pollutes
      // scrollback permanently. Bail before fit() ever runs.
      if (el.getBoundingClientRect().width === 0) return false;
      try {
        fitAddon.fit();
      } catch {
        // fit() can throw if the element is detached mid-teardown — ignore.
        return false;
      }
      // An element detached mid-teardown fits to 0 — never treat that as sized.
      if (term.cols === 0 || term.rows === 0) return false;
      firstFitDone = true;
      return true;
    };
    tryFit();

    // If we could NOT size yet (hidden container), hold live output instead of
    // writing it into the 80x24 default — mirror of VS Code's `_initialDataEvents`
    // queue (vscode:…/browser/terminalInstance.ts:1586), which buffers everything
    // the process emits until the terminal is attached and replays it in order.
    // Nothing can arrive during this synchronous effect body, so the queue is
    // only ever armed for the deferred path below.
    pendingRef.current = firstFitDone ? null : [];
    const flushPending = () => {
      const queued = pendingRef.current;
      if (!queued) return;
      // Null FIRST: a write below can re-enter through xterm's parser and must
      // find the queue already drained, never append to a list being iterated.
      pendingRef.current = null;
      if (queued.length > 0) receivedOutputRef.current = true;
      for (const chunk of queued) term.write(chunk);
    };

    // Restore the pre-remount buffer (remount tab mode) synchronously, BEFORE
    // termRef is set below — so a late useSessionOutput event can never land
    // mid-restore and interleave with the written-back context. The mount jiggle
    // further down SIGWINCHes the child, which repaints the live frame BELOW the
    // divider. Gated on remount mode: keep-alive never saves and must never
    // restore, so no divider is ever injected there.
    const snap = isRemount ? snapshots.get(sessionId) : undefined;
    if (snap) {
      term.write(snap.data);
      // Explicit SGR reset — serialize output is not guaranteed to end reset, so
      // clear any lingering attributes before the divider and the live frame.
      term.write("\x1b[0m\r\n");
      // Dim, full-width divider at the SAVED cols, NOT at term.cols: the
      // snapshot content is hard-wrapped at the width it was serialized at, so
      // sizing the divider to snap.cols is what keeps it flush with the restored
      // lines even when this mount fitted to a different width just above.
      const label = "─── earlier output ───";
      const pad = Math.max(0, snap.cols - label.length);
      const left = Math.floor(pad / 2);
      const line = "─".repeat(left) + label + "─".repeat(pad - left);
      term.write("\x1b[2m" + line + "\x1b[0m\r\n");
    }

    termRef.current = term;
    devTerms?.set(sessionId, term);

    // Push the fitted (cols, rows) to the PTY so a full-screen TUI lays out at
    // the on-screen size instead of the 80×24 default. Best-effort + deduped
    // (skip if unchanged). xterm's OWN grid is already sized synchronously
    // above; what stays deferred here is only the PTY side — the SIGWINCH and
    // the child's repaint.
    //
    // DEBOUNCED: a window drag-resize or zoom fires the ResizeObserver many
    // times in quick succession. Coalescing the gesture into one settle-time
    // fit + resize lets the child repaint once, cleanly, at the final size.
    //
    // A genuine resize changes the size, so a single PTY resize raises SIGWINCH
    // and the child repaints itself — no jiggle, no flicker.
    //
    // The one exception is the FIRST sizing after mount: the PTY lives in the
    // Rust runtime and OUTLIVES this component, so a fresh mount (tab switch /
    // reload) often fits to the SAME dimensions the PTY already has. The kernel
    // raises SIGWINCH only on an actual change, so that unchanged resize is a
    // no-op and the pane stays BLACK until a manual resize. Only there do we
    // jiggle (send `rows - 1` then `rows`) to force a guaranteed SIGWINCH.
    let lastCols = 0;
    let lastRows = 0;
    let firstSizing = true;
    let resizeTimer: ReturnType<typeof setTimeout> | undefined;
    let jiggleTimer: ReturnType<typeof setTimeout> | undefined;
    const applyResize = () => {
      // `tryFit` carries the hidden-tab / detached-element guards: on a false
      // return `firstSizing` stays unconsumed, so the unhide path still runs the
      // normal 0 → real jiggle, and no zero-dimension resize reaches the PTY.
      if (!tryFit()) return;
      // The grid is now correct — anything held back can be replayed in order.
      flushPending();
      const { cols, rows } = term;
      if (cols === lastCols && rows === lastRows) return;
      lastCols = cols;
      lastRows = rows;

      const isFirst = firstSizing;
      firstSizing = false;
      // Real resize (or a degenerate 1-row pane): xterm is already at (cols,
      // rows) from the fit above, so one PTY push completes the pair and its
      // genuine size change raises SIGWINCH on its own.
      if (!isFirst || rows <= 1) {
        void ipc.session.resize({ sessionId, cols, rows }).catch(() => {});
        return;
      }
      // Mount: jiggle so the child repaints even if the persistent PTY already
      // has this size. `rows - 1` then `rows`, with a gap so the two SIGWINCHs
      // aren't coalesced into one.
      //
      // BOTH SIDES MOVE TOGETHER. VS Code's contract is that xterm and the PTY
      // are always resized as one step — `xterm.resize(cols, rows)` immediately
      // followed by the pty dimension update
      // (vscode:…/browser/terminalInstance.ts:830-831); it never jiggles at all.
      // We still have to, because our PTY outlives the component and an
      // unchanged resize raises no SIGWINCH. But pushing `rows - 1` to the PTY
      // while xterm stayed at `rows` opened exactly the grid disagreement F5
      // closes at mount: the child re-laid its frame for `rows - 1` and we
      // parsed it into a `rows` grid, desyncing its relative-move cursor model
      // (audit §3 H1, rows 9/10). So each leg resizes xterm first, then the PTY.
      term.resize(cols, rows - 1);
      void ipc.session.resize({ sessionId, cols, rows: rows - 1 }).catch(() => {});
      clearTimeout(jiggleTimer);
      jiggleTimer = setTimeout(() => {
        term.resize(cols, rows);
        void ipc.session.resize({ sessionId, cols, rows }).catch(() => {
          // Session not running yet / no PTY — harmless.
        });
      }, 60);
    };
    // Forward every keystroke (raw, including control sequences) AND every
    // terminal-query reply xterm generates to the PTY stdin, through the single
    // ordered channel (`sendStdin`) — the replies xterm emits in one parse pass
    // must reach the child in the order it emitted them. `sessionId` is stable
    // for this mount (the component is keyed by it), so capturing it is safe.
    const dataSub = term.onData((data) => {
      sendStdin(data);
    });

    // Mouse-wheel scrolling for full-screen TUIs. The "alternate" screen buffer
    // a TUI runs in has NO terminal-level scrollback — the history lives inside
    // the app — so the wheel does nothing by default and you're stuck on the
    // current frame. Mirror iTerm's "alternate scroll": translate a wheel notch
    // into arrow-key presses sent to the PTY, so the app scrolls its own
    // transcript (Codex's ↑/↓, Claude Code's pager). Skip it when the app is
    // tracking the mouse itself (xterm already forwards the wheel as mouse events
    // there) and on the normal buffer (xterm scrolls that natively).
    const wheelHandler = (e: WheelEvent) => {
      if (term.buffer.active.type !== "alternate") return;
      if (term.modes.mouseTrackingMode !== "none") return;
      e.preventDefault();
      const seq = e.deltaY < 0 ? "\x1b[A" : "\x1b[B"; // arrow up / down
      sendStdin(seq.repeat(3));
    };
    el.addEventListener("wheel", wheelHandler, { passive: false });

    // Re-fit on container resize (and window resize as a fallback), debounced so
    // a continuous drag-resize coalesces into one settle-time fit + PTY resize.
    const observer = new ResizeObserver(() => {
      clearTimeout(resizeTimer);
      resizeTimer = setTimeout(applyResize, 120);
    });

    // Defer the first PTY resize + jiggle a beat, THEN start observing. The
    // session-output listener (useSessionOutput) registers via Tauri's ASYNC
    // `listen()`, so on a page reload — where the PTY and its already-painted
    // frame persist — firing the jiggle synchronously makes the child repaint
    // before the listener is in place; that frame reaches no one and the pane
    // stays black until a manual resize. Waiting lets the listener attach first,
    // so the forced repaint is actually received. This delay no longer costs
    // correctness: xterm was fitted synchronously at mount, so any frame that
    // arrives during it is parsed against the RIGHT grid. We only `observe()`
    // after the initial sizing so the observer's own first callback can't
    // pre-empt the deferred jiggle. Cleared on teardown.
    const initialTimer = setTimeout(() => {
      applyResize();
      // Last resort: the pane may STILL be hidden (keep-alive tab mode), and
      // holding output for an unbounded time would grow without limit and leave
      // the tab black on unhide. Writing it into the default grid is exactly
      // what happened before this queue existed, and the unhide path's 0 → real
      // jiggle repaints over it — so the queue is bounded at ~200 ms of output.
      flushPending();
      observer.observe(el);
    }, 200);

    // Expose the respawn re-fit: forget the dedup baseline and re-run the
    // first-sizing jiggle so the FRESH PTY (default 80×24) both receives the
    // real dims and gets a guaranteed SIGWINCH even if they happen to match.
    repushSizeRef.current = () => {
      lastCols = 0;
      lastRows = 0;
      firstSizing = true;
      clearTimeout(resizeTimer);
      resizeTimer = setTimeout(applyResize, 300);
    };

    return () => {
      clearTimeout(initialTimer);
      clearTimeout(resizeTimer);
      clearTimeout(jiggleTimer);
      observer.disconnect();
      el.removeEventListener("wheel", wheelHandler);
      dataSub.dispose();
      repushSizeRef.current = null;
      devTerms?.delete(sessionId);
      // Drop anything still queued — this terminal is about to be disposed, and
      // a late event must fall through to the null `termRef` instead of piling
      // onto a queue nobody will flush.
      pendingRef.current = null;
      // Null the ref BEFORE dispose so a late useSessionOutput write can't
      // touch a disposed terminal.
      termRef.current = null;
      // Snapshot the buffer for the next mount (remount tab mode), BEFORE
      // dispose. Only when this mount actually received output — see
      // receivedOutputRef: skipping the empty case keeps a StrictMode
      // double-mount or a fast tab flip from clobbering a good snapshot.
      // excludeAltBuffer: for an alt-screen TUI the transcript worth keeping is
      // the NORMAL buffer's scrollback; the live alt-screen frame is re-drawn by
      // the next mount's jiggle SIGWINCH, so serializing it would only duplicate.
      if (isRemount && receivedOutputRef.current) {
        snapshots.set(sessionId, {
          data: serializeAddon.serialize({
            scrollback: 12000,
            excludeAltBuffer: true,
            excludeModes: true,
          }),
          cols: term.cols,
        });
      }
      term.dispose();
    };
  }, [sessionId]);

  // A `running` status on this session means a backend (re)spawned. For the
  // respawn case the PTY is brand-new — re-push the on-screen size to it.
  useSessionStatus(sessionId, (e) => {
    if (e.status === "running") repushSizeRef.current?.();
  });

  // Stream output chunks for this session into the terminal. The hook already
  // filters by sessionId, so we write every delivered chunk verbatim.
  useSessionOutput(sessionId, (e) => {
    // Queued while this mount's xterm still has no real size: writing a frame
    // into the 80x24 default desyncs the child's relative-move cursor model for
    // the rest of the session (audit §3 H1). `flushPending` replays these in
    // arrival order the moment the first fit succeeds.
    const pending = pendingRef.current;
    if (pending) {
      pending.push(e.chunk);
      return;
    }
    receivedOutputRef.current = true;
    termRef.current?.write(e.chunk);
  });

  return (
    <div
      ref={dropRef}
      className={`flex-1 min-h-0 overflow-hidden bg-[#1e1e1e] p-1.5 transition-shadow${
        isOver ? " ring-2 ring-inset ring-accent" : ""
      }`}
    >
      <div ref={divRef} className="h-full w-full overflow-hidden" />
    </div>
  );
}
