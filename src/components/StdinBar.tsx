import { useEffect, useRef, useState } from "react";
import { CornerUpRight, CornerDownLeft } from "lucide-react";
import { ipc } from "../ipc";
import { RoutingPicker, type RoutingTarget } from "./RoutingPicker";
import { useFileDrop, shellQuotePath } from "../lib/fileDrop";

interface StdinBarProps {
  /** The focused session to send input to, or null when none is live. */
  sessionId: string | null;
  /** Own instance id — the SELF routing target. */
  instanceId: string;
  /** All routable agents in the workspace (includes self). */
  roster: RoutingTarget[];
}

// A transient outbox confirmation for a routed send (this agent → another).
interface OutboxNote {
  toName: string;
  status: "queued" | "delivered";
}

/**
 * Input bar pinned below the terminal. On Enter, forwards the line (plus a
 * trailing newline — the backend forwards stdin verbatim) to the live session,
 * OR, when a non-self target is picked, injects it into that agent's session.
 *
 * Send failures (e.g. the session is not running) are surfaced inline rather
 * than swallowed.
 *
 * NOTE: inbound injections (this agent RECEIVING) are intentionally NOT banners
 * here — the injected text already appears in the terminal's stdout stream, so a
 * separate line would duplicate the PTY echo. We only surface the OUTBOX
 * confirmation (messages this agent SENT), which has no such overlap.
 */
export function StdinBar({ sessionId, instanceId, roster }: StdinBarProps) {
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  // Routing target for the NEXT send. Default = self (own session).
  const [targetId, setTargetId] = useState(instanceId);
  // Last routed-send confirmation (cleared on the next keystroke).
  const [outbox, setOutbox] = useState<OutboxNote | null>(null);

  // Guard setState against a send that resolves after unmount (e.g. the pane
  // closes mid-send) — React 19 no-ops the update but `sending` would otherwise
  // be left stuck. Cheap insurance for a high-latency send.
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  // Reset the routing target to SELF when the focused instance changes. Unlike
  // ChatView (keyed by sessionId → remounts per tab), StdinBar persists across
  // CLI tab switches, so without this the previous tab's target would carry over
  // and silently inject into the wrong agent.
  useEffect(() => {
    setTargetId(instanceId);
    setOutbox(null);
  }, [instanceId]);

  const routingToOther = targetId !== instanceId;
  // Self send needs a live session; a routed inject only needs own instanceId.
  const disabled = sending || (!routingToOther && sessionId === null);

  async function handleSend() {
    const text = value;
    if (text.length === 0) return;

    const target = roster.find((t) => t.instanceId === targetId);
    if (target && target.instanceId !== instanceId) {
      await handleRoutedSend(target, text);
      return;
    }

    // ── Self send (own PTY) — submit the line ──
    // Submit with a CARRIAGE RETURN (\r), the byte the Enter key emits in a
    // terminal (xterm's onData sends \r too); a TUI treats \r as submit and \n
    // as "insert a newline".
    //
    // But send the \r SEPARATELY, a beat after the text — not as one "text\r"
    // burst. Some TUIs (Codex) run paste-burst detection: a flurry of bytes
    // arriving together is treated as a PASTE, so an embedded \r is inserted
    // literally (a stray ␍) instead of submitting. A standalone \r a moment
    // later reads as a real Enter keypress, so both Claude Code and Codex submit.
    if (sessionId === null) return;
    setError(null);
    setSending(true);
    try {
      await ipc.message.send({ sessionId, text });
      await new Promise((r) => setTimeout(r, 40));
      await ipc.message.send({ sessionId, text: "\r" });
      if (mounted.current) setValue("");
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      if (mounted.current) setError(`Send failed: ${msg}`);
    } finally {
      if (mounted.current) setSending(false);
    }
  }

  // Routed send: inject into a TARGET agent's live input. `inject` appends the
  // newline server-side (no trailing "\n" here). On success an outbox note
  // confirms delivery (delivered vs queued); target resets to self afterward.
  async function handleRoutedSend(target: RoutingTarget, text: string) {
    setError(null);
    setSending(true);
    try {
      const msg = await ipc.message.inject({
        fromInstanceId: instanceId,
        toInstanceId: target.instanceId,
        text,
      });
      if (mounted.current) {
        setValue("");
        setOutbox({ toName: target.name, status: msg.status });
        setTargetId(instanceId);
      }
    } catch (err) {
      const detail = err instanceof Error ? err.message : String(err);
      if (mounted.current) setError(`Failed to send to ${target.name}: ${detail}`);
    } finally {
      if (mounted.current) setSending(false);
    }
  }

  // Drag a file onto the composer → insert its shell-quoted path at the end, the
  // way a normal terminal does (a CLI like Claude Code reads the path from the
  // line). Multiple files are space-separated; a trailing space readies the next
  // token. The field re-focuses so the user can keep typing.
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const { ref: dropRef, isOver } = useFileDrop<HTMLDivElement>((paths) => {
    const insert = paths.map(shellQuotePath).join(" ");
    setValue((v) => (v.length === 0 || v.endsWith(" ") ? v : v + " ") + insert + " ");
    setOutbox(null);
    inputRef.current?.focus();
  });

  // Auto-grow with content, capped by the `max-h-40` on the element itself —
  // "auto" first so a shrink (e.g. clearing after send) isn't stuck at the
  // tallest height it ever reached (scrollHeight only ever grows otherwise).
  useEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [value]);

  const placeholder = routingToOther
    ? "Type to inject into the target session…"
    : sessionId === null
      ? "No running session"
      : "Message the agent…  (Enter to send · Shift+Enter for newline)";

  return (
    <div className="shrink-0 border-t border-overlay/[0.06] bg-surface">
      {error && (
        <div className="px-3 pt-1.5 text-[11px] text-danger">{error}</div>
      )}
      {outbox && (
        <div className="px-3 pt-1.5 text-[11px] flex items-center gap-1.5 text-text-secondary">
          <CornerUpRight className="w-3 h-3 shrink-0 text-text-tertiary" />
          <span className="font-medium text-text-primary">→ sent to {outbox.toName}</span>
          {outbox.status === "delivered" ? (
            <span>· auto-submit</span>
          ) : (
            <span className="text-warning">· target agent isn't running — queued</span>
          )}
        </div>
      )}
      <div className="px-3 py-3">
        {/* Composer field — rounded box matching ChatView's composer so the CLI
            stdin and the chat input read as the same control. */}
        <div
          ref={dropRef}
          className={`flex items-center gap-2.5 rounded-2xl ring-1 bg-fill-softer px-3 py-2.5 transition-shadow${
            isOver
              ? " ring-2 ring-accent bg-accent/[0.06]"
              : " ring-overlay/[0.1] focus-within:ring-accent/50"
          }`}
        >
          <RoutingPicker
            selfId={instanceId}
            roster={roster}
            value={targetId}
            onChange={setTargetId}
            disabled={sending}
          />
          <span className="text-[15px] text-text-tertiary font-mono select-none shrink-0">›</span>
          <textarea
            ref={inputRef}
            rows={1}
            value={value}
            disabled={disabled}
            onChange={(e) => {
              setValue(e.target.value);
              if (outbox) setOutbox(null);
            }}
            onKeyDown={(e) => {
              // Enter submits; Shift+Enter falls through to the browser default
              // (insert a literal newline) — matches ChatView's composer. The
              // embedded "\n" rides along in the single `text` send below, and
              // Claude Code / Codex read it as "insert a line" in their own
              // input box rather than submitting (only the trailing standalone
              // "\r" does that).
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                void handleSend();
              }
            }}
            placeholder={placeholder}
            className="flex-1 min-w-0 bg-transparent outline-none resize-none overflow-y-auto text-[14.5px] font-mono placeholder:text-text-tertiary py-0.5 max-h-40 disabled:opacity-50"
          />
          {/* Send — Enter also submits; the button mirrors that for discoverability. */}
          <button
            type="button"
            onClick={() => void handleSend()}
            disabled={disabled || value.length === 0}
            title="Send (Enter)"
            aria-label="Send"
            className="w-9 h-9 rounded-xl bg-accent text-white grid place-items-center shrink-0 hover:brightness-105 disabled:opacity-30 disabled:hover:brightness-100"
          >
            <CornerDownLeft className="w-[18px] h-[18px]" />
          </button>
        </div>
      </div>
    </div>
  );
}
