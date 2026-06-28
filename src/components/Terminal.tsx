import { useEffect, useRef } from "react";
import { Terminal as XtermTerminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { ipc, useSessionOutput } from "../ipc";

interface TerminalProps {
  sessionId: string;
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

  useEffect(() => {
    const el = divRef.current;
    if (!el) return;

    const term = new XtermTerminal({
      convertEol: true,
      fontSize: 12,
      fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
      theme: { background: "#1e1e1e", foreground: "#e5e5e5" },
      cursorBlink: true,
    });
    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.open(el);
    term.focus();
    termRef.current = term;

    // Fit the xterm grid to the container, then push the real (cols, rows) to
    // the PTY so a full-screen TUI lays out at the on-screen size instead of the
    // 80×24 default. Best-effort + deduped (skip if unchanged).
    //
    // DEBOUNCED: a window drag-resize or zoom fires the ResizeObserver many
    // times in quick succession. Coalescing the gesture into one settle-time
    // fit + resize lets the child repaint once, cleanly, at the final size.
    //
    // JIGGLE (the load-bearing part): the PTY lives in the Rust runtime and
    // OUTLIVES this component, so a fresh mount (tab switch / reload) very often
    // fits to the SAME dimensions the PTY already has. The kernel only raises
    // SIGWINCH when the size actually CHANGES, so an unchanged resize is a
    // no-op: the child (Claude Code) never repaints and the pane stays BLACK
    // until the user manually resizes the window. The same root cause leaves
    // stale cells after a real resize — one SIGWINCH yields one partial repaint.
    // So we never send the target size alone: we send `rows - 1` then `rows`,
    // forcing two guaranteed SIGWINCHs that end at the right size and make the
    // child clear and fully redraw. `lastRows` is tracked so the dedupe still
    // suppresses redundant work when nothing changed.
    let lastCols = 0;
    let lastRows = 0;
    let resizeTimer: ReturnType<typeof setTimeout> | undefined;
    let jiggleTimer: ReturnType<typeof setTimeout> | undefined;
    const applyResize = () => {
      try {
        fitAddon.fit();
      } catch {
        // fit() can throw if the element is detached mid-teardown — ignore.
        return;
      }
      const { cols, rows } = term;
      if (cols === lastCols && rows === lastRows) return;
      lastCols = cols;
      lastRows = rows;
      // xterm reflows its buffer on resize, which can strand stray cells OUTSIDE
      // the region the child (Claude Code / Ink) manages — Ink only erases and
      // redraws its own frame via RELATIVE cursor moves, so it never clears
      // those leftovers. Wipe the viewport and home the cursor ourselves so the
      // forced repaint below lands on a clean screen. This is written to xterm's
      // DISPLAY only (the child never sees it, so its process state is intact);
      // the immediate full re-render re-aligns xterm's cursor with the child's,
      // and ED (\x1b[2J) keeps scrollback so history isn't lost.
      term.write("\x1b[2J\x1b[H");
      if (rows <= 1) {
        // Degenerate height — nothing to jiggle against; send as-is.
        void ipc.session.resize({ sessionId, cols, rows }).catch(() => {});
        return;
      }
      // Step 1: one row shorter → guaranteed SIGWINCH, child clears + redraws.
      void ipc.session.resize({ sessionId, cols, rows: rows - 1 }).catch(() => {});
      // Step 2 (after a short gap so the two SIGWINCHs aren't coalesced): the
      // real size → child redraws its full frame at the on-screen dimensions,
      // onto the cleared screen.
      clearTimeout(jiggleTimer);
      jiggleTimer = setTimeout(() => {
        void ipc.session.resize({ sessionId, cols, rows }).catch(() => {
          // Session not running yet / no PTY — harmless.
        });
      }, 60);
    };
    // Initial sizing is immediate so the first paint is at the right size.
    applyResize();

    // Forward every keystroke (raw, including control sequences) to the PTY
    // stdin. `sessionId` is stable for this mount (the component is keyed by it),
    // so capturing it here is safe.
    const dataSub = term.onData((data) => {
      void ipc.message.send({ sessionId, text: data }).catch(() => {
        // Session not running / backend gone — the output stream will surface
        // the state; dropping the keystroke is the right behavior.
      });
    });

    // Re-fit on container resize (and window resize as a fallback), debounced so
    // a continuous drag-resize coalesces into one settle-time fit + PTY resize.
    const observer = new ResizeObserver(() => {
      clearTimeout(resizeTimer);
      resizeTimer = setTimeout(applyResize, 120);
    });
    observer.observe(el);

    return () => {
      clearTimeout(resizeTimer);
      clearTimeout(jiggleTimer);
      observer.disconnect();
      dataSub.dispose();
      // Null the ref BEFORE dispose so a late useSessionOutput write can't
      // touch a disposed terminal.
      termRef.current = null;
      term.dispose();
    };
  }, [sessionId]);

  // Stream output chunks for this session into the terminal. The hook already
  // filters by sessionId, so we write every delivered chunk verbatim.
  useSessionOutput(sessionId, (e) => {
    termRef.current?.write(e.chunk);
  });

  return (
    <div className="flex-1 min-h-0 overflow-hidden bg-[#1e1e1e] p-1.5">
      <div ref={divRef} className="h-full w-full overflow-hidden" />
    </div>
  );
}
