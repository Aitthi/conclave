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

  // Drag a file onto the terminal → type its shell-quoted path into the PTY at
  // the cursor (no submit), exactly like dropping a file into a real terminal.
  // Written straight to stdin via message.send; the running TUI echoes it.
  const { ref: dropRef, isOver } = useFileDrop<HTMLDivElement>((paths) => {
    const insert = paths.map(shellQuotePath).join(" ") + " ";
    void ipc.message.send({ sessionId, text: insert }).catch(() => {});
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
      convertEol: true,
      fontSize: 12,
      fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
      theme: { background: "#1e1e1e", foreground: "#e5e5e5" },
      cursorBlink: true,
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
      webgl.onContextLoss(() => webgl.dispose());
      term.loadAddon(webgl);
    } catch {
      // No WebGL in this webview — the DOM renderer stays active.
    }

    term.focus();

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
      // Dim, full-width divider at the SAVED cols. The snapshot content is
      // hard-wrapped at the width it originated from (fit() has not run yet on
      // this fresh mount — term.cols is still the 80 default), so sizing the
      // divider to snap.cols keeps it flush with the restored lines.
      const label = "─── earlier output ───";
      const pad = Math.max(0, snap.cols - label.length);
      const left = Math.floor(pad / 2);
      const line = "─".repeat(left) + label + "─".repeat(pad - left);
      term.write("\x1b[2m" + line + "\x1b[0m\r\n");
    }

    termRef.current = term;

    // Fit the xterm grid to the container, then push the real (cols, rows) to
    // the PTY so a full-screen TUI lays out at the on-screen size instead of the
    // 80×24 default. Best-effort + deduped (skip if unchanged).
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
      // A hidden tab (display:none) has no used layout: getBoundingClientRect
      // reads the USED width, which is genuinely 0 there — unlike FitAddon's
      // proposeDimensions, which reads getComputedStyle and sees the COMPUTED
      // value '100%', parses it with parseInt, and gets 100(px). That misread
      // 100px never rounds down to 0 cols, so it slips past the cols/rows===0
      // guard below and pushes a bogus ~11-col resize to the live PTY every
      // time a tab is switched away from — the child then rewraps its
      // transcript at 11 cols and pollutes scrollback permanently. Bail here,
      // before fit() ever runs, so `firstSizing` stays unconsumed and the
      // unhide path re-runs the normal 0 → real jiggle.
      if (el.getBoundingClientRect().width === 0) return;
      try {
        fitAddon.fit();
      } catch {
        // fit() can throw if the element is detached mid-teardown — ignore.
        return;
      }
      const { cols, rows } = term;
      // An element detached mid-teardown fits to 0 — never push a
      // zero-dimension resize to the PTY (it would wedge the child's layout).
      if (cols === 0 || rows === 0) return;
      if (cols === lastCols && rows === lastRows) return;
      lastCols = cols;
      lastRows = rows;

      const isFirst = firstSizing;
      firstSizing = false;
      // Real resize (or a degenerate 1-row pane): one resize, natural SIGWINCH.
      if (!isFirst || rows <= 1) {
        void ipc.session.resize({ sessionId, cols, rows }).catch(() => {});
        return;
      }
      // Mount: jiggle so the child repaints even if the persistent PTY already
      // has this size. `rows - 1` then `rows`, with a gap so the two SIGWINCHs
      // aren't coalesced into one.
      void ipc.session.resize({ sessionId, cols, rows: rows - 1 }).catch(() => {});
      clearTimeout(jiggleTimer);
      jiggleTimer = setTimeout(() => {
        void ipc.session.resize({ sessionId, cols, rows }).catch(() => {
          // Session not running yet / no PTY — harmless.
        });
      }, 60);
    };
    // Forward every keystroke (raw, including control sequences) to the PTY
    // stdin. `sessionId` is stable for this mount (the component is keyed by it),
    // so capturing it here is safe.
    const dataSub = term.onData((data) => {
      void ipc.message.send({ sessionId, text: data }).catch(() => {
        // Session not running / backend gone — the output stream will surface
        // the state; dropping the keystroke is the right behavior.
      });
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
      void ipc.message.send({ sessionId, text: seq.repeat(3) }).catch(() => {});
    };
    el.addEventListener("wheel", wheelHandler, { passive: false });

    // Re-fit on container resize (and window resize as a fallback), debounced so
    // a continuous drag-resize coalesces into one settle-time fit + PTY resize.
    const observer = new ResizeObserver(() => {
      clearTimeout(resizeTimer);
      resizeTimer = setTimeout(applyResize, 120);
    });

    // Defer the first sizing+jiggle a beat, THEN start observing. The
    // session-output listener (useSessionOutput) registers via Tauri's ASYNC
    // `listen()`, so on a page reload — where the PTY and its already-painted
    // frame persist — firing the jiggle synchronously makes the child repaint
    // before the listener is in place; that frame reaches no one and the pane
    // stays black until a manual resize. Waiting lets the listener attach first,
    // so the forced repaint is actually received (the buffer is empty/black
    // until then anyway, so this delay is invisible). We only `observe()` after
    // the initial sizing so the observer's own first callback can't pre-empt the
    // deferred jiggle. Cleared on teardown.
    const initialTimer = setTimeout(() => {
      applyResize();
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
            scrollback: 2000,
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
