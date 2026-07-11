import clsx from "clsx";
import { ArrowLeft, ArrowRight, Eye, Lock, RotateCw, X } from "lucide-react";
import { hueVar, type Tab } from "../lib/tabs";

// The detail-pane top bar. Human tabs get a live, editable URL field + reload;
// agent tabs get a locked field and an "observing" affordance — no navigation
// controls, because the human never steers an agent mid-task (D2).
export default function BrowserChrome({ tab }: { tab: Tab }) {
  const human = tab.owner.kind === "human";
  const ended = tab.status === "ended";

  return (
    <div className="flex h-12 shrink-0 items-center gap-2 border-b border-border bg-surface px-3">
      {/* nav controls — real only for the human's own tab */}
      <div className="flex items-center gap-0.5">
        <NavBtn disabled={!human}>
          <ArrowLeft className="h-4 w-4" />
        </NavBtn>
        <NavBtn disabled>
          <ArrowRight className="h-4 w-4" />
        </NavBtn>
        <NavBtn disabled={!human || ended}>
          <RotateCw className="h-4 w-4" />
        </NavBtn>
      </div>

      {/* url field */}
      <div
        className={clsx(
          "flex h-8 min-w-0 flex-1 items-center gap-2 rounded-md px-2.5 text-[12px]",
          human ? "bg-fill text-text-primary" : "bg-fill-soft text-text-secondary",
        )}
      >
        {human ? (
          <span className="h-3.5 w-3.5 shrink-0 rounded-full bg-live/90" />
        ) : (
          <Lock className="h-3.5 w-3.5 shrink-0 text-text-tertiary" />
        )}
        <span className="truncate font-mono">{tab.url}</span>
        {human && <span className="ml-auto h-4 w-px shrink-0 animate-pulse bg-accent" aria-hidden />}
      </div>

      {/* right-side state */}
      {human ? (
        <button
          type="button"
          aria-label="Close tab"
          className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-text-secondary transition-colors hover:bg-overlay/[0.06] hover:text-text-primary"
        >
          <X className="h-4 w-4" />
        </button>
      ) : (
        <span
          className={clsx(
            "flex shrink-0 items-center gap-1.5 rounded-full py-1 pl-1.5 pr-2.5 text-[11px] font-medium",
            ended ? "bg-fill text-text-tertiary" : "bg-overlay/[0.05] text-text-secondary",
          )}
        >
          {ended ? (
            "Session ended"
          ) : (
            <>
              <Eye className="h-3 w-3" />
              Observing
              <span
                className="ml-0.5 h-3.5 w-3.5 rounded-[0.3rem] text-[8px] font-bold leading-[0.875rem] text-center text-white"
                style={{ backgroundColor: hueVar(tab.owner) }}
              >
                {tab.owner.initials.slice(0, 1)}
              </span>
            </>
          )}
        </span>
      )}
    </div>
  );
}

function NavBtn({ children, disabled }: { children: React.ReactNode; disabled?: boolean }) {
  return (
    <button
      type="button"
      disabled={disabled}
      className={clsx(
        "grid h-7 w-7 place-items-center rounded-md transition-colors",
        disabled
          ? "text-text-tertiary/50"
          : "text-text-secondary hover:bg-overlay/[0.06] hover:text-text-primary",
      )}
    >
      {children}
    </button>
  );
}
