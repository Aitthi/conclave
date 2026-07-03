import { useLayoutEffect, useRef, useState } from "react";

/** One chat message's text, clamped to `lines` lines with a Show-more/less
 *  toggle that appears ONLY when the text actually overflows the clamp
 *  (measured, not guessed — so a long-but-short-lined message shows no dead
 *  toggle). Expand state lives per-instance, so it resets for free when the
 *  message unmounts (no cross-conversation state leak). */
export function ClampText({
  text,
  outgoing,
  lines = 6,
}: {
  text: string;
  outgoing: boolean;
  /** Clamp height in lines — 6 (drawer-sized) or 12 (hub-sized). */
  lines?: 6 | 12;
}) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [expanded, setExpanded] = useState(false);
  const [clampable, setClampable] = useState(false);
  // Measure while clamped (expanded starts false): the element overflows iff
  // its scroll height exceeds its clamped client height. Runs on text change
  // only, so it is NOT re-measured (and hidden) after the user expands.
  useLayoutEffect(() => {
    const el = ref.current;
    if (el) setClampable(el.scrollHeight - el.clientHeight > 4);
  }, [text]);
  const clampClass = lines === 12 ? "line-clamp-[12]" : "line-clamp-6";
  return (
    <>
      <div
        ref={ref}
        className={`whitespace-pre-wrap break-words leading-snug ${
          expanded ? "" : clampClass
        }`}
      >
        {text}
      </div>
      {clampable && (
        <button
          onClick={() => setExpanded((v) => !v)}
          className={`mt-0.5 text-[10px] font-semibold ${
            outgoing ? "text-white/80 hover:text-white" : "text-accent hover:underline"
          }`}
        >
          {expanded ? "Show less" : "Show more"}
        </button>
      )}
    </>
  );
}
