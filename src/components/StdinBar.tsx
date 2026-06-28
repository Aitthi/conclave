import { useEffect, useRef, useState } from "react";
import { CornerUpRight, CornerDownLeft } from "lucide-react";
import { ipc } from "../ipc";
import { RoutingPicker, type RoutingTarget } from "./RoutingPicker";

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
    // Append a CARRIAGE RETURN (\r), not a newline (\n): that is the byte the
    // Enter key actually emits in a terminal (xterm's onData sends \r too). A
    // full-screen TUI like Claude Code treats \r as "submit" but \n as "insert a
    // newline in the input field" — sending \n made `/clear` drop to a new line
    // instead of running.
    if (sessionId === null) return;
    setError(null);
    setSending(true);
    try {
      await ipc.message.send({ sessionId, text: text + "\r" });
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

  const placeholder = routingToOther
    ? "Type to inject into the target session…"
    : sessionId === null
      ? "No running session"
      : "Message the agent…";

  return (
    <div className="shrink-0 border-t border-black/[0.06] bg-white">
      {error && (
        <div className="px-3 pt-1.5 text-[11px] text-[#ff3b30]">{error}</div>
      )}
      {outbox && (
        <div className="px-3 pt-1.5 text-[11px] flex items-center gap-1.5 text-[#6e6e73]">
          <CornerUpRight className="w-3 h-3 shrink-0 text-[#a1a1a6]" />
          <span className="font-medium text-[#1d1d1f]">→ sent to {outbox.toName}</span>
          {outbox.status === "delivered" ? (
            <span>· auto-submit</span>
          ) : (
            <span className="text-[#ff9f0a]">· target agent isn't running — queued</span>
          )}
        </div>
      )}
      <div className="px-3 py-3">
        {/* Composer field — rounded box matching ChatView's composer so the CLI
            stdin and the chat input read as the same control. */}
        <div className="flex items-center gap-2.5 rounded-2xl ring-1 ring-black/[0.1] bg-[#f7f7f8] focus-within:ring-[#0a84ff]/50 px-3 py-2.5 transition-shadow">
          <RoutingPicker
            selfId={instanceId}
            roster={roster}
            value={targetId}
            onChange={setTargetId}
            disabled={sending}
          />
          <span className="text-[15px] text-[#a1a1a6] font-mono select-none shrink-0">›</span>
          <input
            value={value}
            disabled={disabled}
            onChange={(e) => {
              setValue(e.target.value);
              if (outbox) setOutbox(null);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                void handleSend();
              }
            }}
            placeholder={placeholder}
            className="flex-1 min-w-0 bg-transparent outline-none text-[14.5px] font-mono placeholder:text-[#a1a1a6] disabled:opacity-50"
          />
          {/* Send — Enter also submits; the button mirrors that for discoverability. */}
          <button
            type="button"
            onClick={() => void handleSend()}
            disabled={disabled || value.length === 0}
            title="Send (Enter)"
            aria-label="Send"
            className="w-9 h-9 rounded-xl bg-[#0a84ff] text-white grid place-items-center shrink-0 hover:brightness-105 disabled:opacity-30 disabled:hover:brightness-100"
          >
            <CornerDownLeft className="w-[18px] h-[18px]" />
          </button>
        </div>
      </div>
    </div>
  );
}
