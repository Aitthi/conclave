import { useEffect, useRef, useState } from "react";
import { ipc } from "../ipc";

interface StdinBarProps {
  /** The focused session to send input to, or null when none is live. */
  sessionId: string | null;
}

/**
 * Input bar pinned below the terminal. On Enter, forwards the line (plus a
 * trailing newline — the backend forwards stdin verbatim) to the live session.
 *
 * Send failures (e.g. the session is not running) are surfaced inline rather
 * than swallowed.
 */
export function StdinBar({ sessionId }: StdinBarProps) {
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);

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

  const disabled = sessionId === null || sending;

  async function handleSend() {
    const text = value;
    if (sessionId === null || text.length === 0) return;
    setError(null);
    setSending(true);
    try {
      await ipc.message.send({ sessionId, text: text + "\n" });
      if (mounted.current) setValue("");
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      if (mounted.current) setError(`ส่งไม่สำเร็จ: ${msg}`);
    } finally {
      if (mounted.current) setSending(false);
    }
  }

  return (
    <div className="shrink-0 border-t border-black/[0.06] bg-white">
      {error && (
        <div className="px-3 pt-1.5 text-[11px] text-[#ff3b30]">{error}</div>
      )}
      <div className="flex items-center gap-2 px-3 py-2">
        <span className="text-[13px] text-[#a1a1a6] font-mono select-none">›</span>
        <input
          value={value}
          disabled={disabled}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              void handleSend();
            }
          }}
          placeholder={sessionId === null ? "ไม่มี session ที่กำลังทำงาน" : "พิมพ์ข้อความถึง agent…"}
          className="flex-1 bg-transparent outline-none text-[13px] font-mono placeholder:text-[#a1a1a6] disabled:opacity-50"
        />
      </div>
    </div>
  );
}
