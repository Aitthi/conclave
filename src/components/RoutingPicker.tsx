import { useEffect, useRef, useState } from "react";
import {
  ChevronDown,
  Check,
  CornerUpRight,
  Terminal as TerminalIcon,
  MessageSquare,
  Waypoints,
} from "lucide-react";

// ---------------------------------------------------------------------------
// Routing target — one routable agent in a workspace. Built from the pane's
// agent tabs; the picker shows SELF (own instance) + every other agent.
// Defined here (the component that owns the picker UI) and re-exported by
// WorkspacePane, which builds the roster.
// ---------------------------------------------------------------------------

export interface RoutingTarget {
  instanceId: string;
  name: string;
  color: string;
  type: "cli" | "chat" | "orchestrator";
}

interface RoutingPickerProps {
  /** Own instance id — the SELF target (sends to own session). */
  selfId: string;
  /** All routable agents (includes self). */
  roster: RoutingTarget[];
  /** Currently-selected target instanceId. */
  value: string;
  /** Called with the chosen target's instanceId. */
  onChange: (instanceId: string) => void;
  /** Disable interaction (e.g. while a send is in flight). */
  disabled?: boolean;
}

// Per-type hint + glyph: how the message reaches the target.
function typeHint(type: RoutingTarget["type"]): { label: string; Icon: typeof TerminalIcon } {
  switch (type) {
    case "cli":
      return { label: "stdin", Icon: TerminalIcon };
    case "chat":
      return { label: "แชท", Icon: MessageSquare };
    case "orchestrator":
      return { label: "fusion", Icon: Waypoints };
  }
}

/**
 * "Send to" target selector — a compact pill that opens a popover listing the
 * roster. Default selection is SELF (own session); choosing another agent routes
 * the next send through `message.inject`. Closes on outside-click / Escape.
 */
export function RoutingPicker({ selfId, roster, value, onChange, disabled }: RoutingPickerProps) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);

  // Close on outside click / Escape.
  useEffect(() => {
    if (!open) return;
    function onDown(e: MouseEvent) {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false);
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const selected = roster.find((t) => t.instanceId === value);
  const isSelf = value === selfId;
  const hint = selected ? typeHint(selected.type) : null;

  return (
    <div ref={rootRef} className="relative shrink-0">
      <button
        type="button"
        disabled={disabled}
        onClick={() => setOpen((v) => !v)}
        title="เลือกผู้รับ"
        className="flex items-center gap-1.5 px-2 py-1 rounded-full ring-1 ring-black/[0.08] bg-white text-[11.5px] text-[#6e6e73] hover:bg-black/[0.03] disabled:opacity-50 max-w-[180px]"
      >
        <CornerUpRight className="w-3 h-3 shrink-0 text-[#a1a1a6]" />
        {selected ? (
          <>
            <span
              className="w-2 h-2 rounded-full shrink-0"
              style={{ backgroundColor: selected.color }}
            />
            <span className="truncate font-medium text-[#1d1d1f]">{selected.name}</span>
            {isSelf ? (
              <span className="text-[#a1a1a6]">· ตัวเอง</span>
            ) : hint ? (
              <span className="text-[#a1a1a6]">· {hint.label}</span>
            ) : null}
          </>
        ) : (
          <span className="text-[#a1a1a6]">เลือกผู้รับ</span>
        )}
        <ChevronDown className="w-3 h-3 shrink-0 text-[#a1a1a6]" />
      </button>

      {open && (
        <div className="absolute bottom-full left-0 mb-1.5 w-[240px] rounded-xl ring-1 ring-black/[0.1] bg-white shadow-lg shadow-black/[0.08] p-1 z-20">
          <div className="text-[10px] font-bold tracking-wider text-[#a1a1a6] uppercase px-2 pt-1 pb-1">
            ส่งไปยัง
          </div>
          {roster.map((t) => {
            const tHint = typeHint(t.type);
            const self = t.instanceId === selfId;
            const isActive = t.instanceId === value;
            return (
              <button
                key={t.instanceId}
                type="button"
                onClick={() => {
                  onChange(t.instanceId);
                  setOpen(false);
                }}
                className="w-full flex items-center gap-2 px-2 py-1.5 rounded-lg text-[12.5px] hover:bg-black/[0.04] text-left"
              >
                <span
                  className="w-2.5 h-2.5 rounded-full shrink-0"
                  style={{ backgroundColor: t.color }}
                />
                <span className="truncate flex-1 min-w-0 font-medium">
                  {t.name}
                  {self && <span className="text-[#a1a1a6] font-normal"> · ตัวเอง</span>}
                </span>
                <span className="flex items-center gap-1 text-[10.5px] text-[#a1a1a6] shrink-0">
                  <tHint.Icon className="w-3 h-3" />
                  {tHint.label}
                </span>
                {isActive && <Check className="w-3.5 h-3.5 text-[#0a84ff] shrink-0" />}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
