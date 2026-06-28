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
    fitAddon.fit();
    term.focus();
    termRef.current = term;

    // Forward every keystroke (raw, including control sequences) to the PTY
    // stdin. `sessionId` is stable for this mount (the component is keyed by it),
    // so capturing it here is safe.
    const dataSub = term.onData((data) => {
      void ipc.message.send({ sessionId, text: data }).catch(() => {
        // Session not running / backend gone — the output stream will surface
        // the state; dropping the keystroke is the right behavior.
      });
    });

    // Re-fit on container resize (and window resize as a fallback).
    const observer = new ResizeObserver(() => {
      try {
        fitAddon.fit();
      } catch {
        // fit() can throw if the element is detached mid-teardown — ignore.
      }
    });
    observer.observe(el);

    return () => {
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
