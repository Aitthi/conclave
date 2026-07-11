import clsx from "clsx";
import { Loader2 } from "lucide-react";
import { hueVar, type Tab } from "../lib/tabs";

// One session row in the rail. Identity leads (avatar chip + name) — the page is
// secondary. This inversion is the point: a normal browser leads with the page;
// this leads with who owns the surface.
export default function TabRow({
  tab,
  active,
  onSelect,
}: {
  tab: Tab;
  active?: boolean;
  onSelect?: (tabId: string) => void;
}) {
  const { owner, title, url, status } = tab;
  const ended = status === "ended";

  return (
    <button
      type="button"
      onClick={() => onSelect?.(tab.tabId)}
      aria-current={active ? "true" : undefined}
      className={clsx(
        "group flex w-full items-center gap-2.5 rounded-row px-2 py-2 text-left transition-colors",
        active ? "bg-accent/[0.12]" : "hover:bg-overlay/[0.05]",
      )}
    >
      {/* identity chip */}
      <span className="relative shrink-0">
        <span
          className={clsx(
            "grid h-8 w-8 place-items-center rounded-chip text-[11px] font-semibold text-white",
            ended && "opacity-40 grayscale",
          )}
          style={{ backgroundColor: hueVar(owner) }}
        >
          {owner.initials}
        </span>
        {status === "live" && !ended && (
          <span className="absolute -bottom-0.5 -right-0.5 h-2.5 w-2.5 rounded-full border-2 border-sidebar bg-live" />
        )}
      </span>

      {/* name + page */}
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-1.5">
          <span
            className={clsx(
              "truncate text-[13px] font-medium",
              active ? "text-text-primary" : ended ? "text-text-muted" : "text-text-primary",
            )}
          >
            {owner.label}
          </span>
          {owner.role && (
            <span className="shrink-0 text-[10px] font-medium uppercase tracking-wide text-text-tertiary">
              {owner.role}
            </span>
          )}
        </span>
        <span className={clsx("mt-0.5 block truncate text-[12px]", ended ? "text-text-tertiary" : "text-text-secondary")}>
          {ended ? url : title}
        </span>
      </span>

      {/* trailing status */}
      <span className="shrink-0 self-start pt-0.5">
        {status === "loading" && <Loader2 className="h-3.5 w-3.5 animate-spin text-waiting" />}
        {ended && (
          <span className="rounded-full bg-fill px-1.5 py-0.5 text-[10px] font-medium text-text-tertiary">
            Ended
          </span>
        )}
      </span>
    </button>
  );
}
