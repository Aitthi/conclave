import { Globe, Lock, Plus } from "lucide-react";
import TabRow from "./TabRow";
import type { Tab } from "../lib/tabs";

function SectionLabel({ children, lock }: { children: React.ReactNode; lock?: boolean }) {
  return (
    <div className="flex items-center gap-1 px-2 pb-1 pt-3">
      <span className="text-[10px] font-semibold uppercase tracking-[0.06em] text-text-tertiary">{children}</span>
      {lock && <Lock className="h-2.5 w-2.5 text-text-tertiary" />}
    </div>
  );
}

// The vertical, Arc-like owner rail. Grouped into the human's own tabs (the only
// interactive surfaces) and the agents' sessions (read-only), so the ownership
// model reads at a glance.
export default function TabRail({
  humanTabs,
  agentTabs,
  activeTabId,
  onSelect,
  onNewTab,
}: {
  humanTabs: Tab[];
  agentTabs: Tab[];
  activeTabId?: string;
  onSelect?: (tabId: string) => void;
  onNewTab?: () => void;
}) {
  const liveAgents = agentTabs.filter((t) => t.status !== "ended").length;

  return (
    <aside className="flex w-[264px] shrink-0 flex-col border-r border-border bg-sidebar">
      {/* rail header */}
      <div className="flex items-center gap-2 px-3 pb-2 pt-3.5">
        <Globe className="h-[18px] w-[18px] shrink-0 text-accent" />
        <span className="text-[13px] font-semibold text-text-primary">Browser</span>
        <button
          type="button"
          onClick={onNewTab}
          aria-label="New tab"
          className="ml-auto grid h-7 w-7 place-items-center rounded-md text-text-secondary transition-colors hover:bg-overlay/[0.06] hover:text-text-primary"
        >
          <Plus className="h-4 w-4" />
        </button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
        {humanTabs.length === 0 && agentTabs.length === 0 ? (
          <p className="px-2 pt-2 text-[12px] leading-relaxed text-text-tertiary">
            No open tabs. Agents' sessions appear here as they browse.
          </p>
        ) : (
          <>
            {humanTabs.length > 0 && (
              <>
                <SectionLabel>You</SectionLabel>
                <div className="flex flex-col gap-0.5">
                  {humanTabs.map((t) => (
                    <TabRow key={t.tabId} tab={t} active={t.tabId === activeTabId} onSelect={onSelect} />
                  ))}
                </div>
              </>
            )}

            {agentTabs.length > 0 && (
              <>
                <SectionLabel lock>Agents · {liveAgents} live</SectionLabel>
                <div className="flex flex-col gap-0.5">
                  {agentTabs.map((t) => (
                    <TabRow key={t.tabId} tab={t} active={t.tabId === activeTabId} onSelect={onSelect} />
                  ))}
                </div>
              </>
            )}
          </>
        )}
      </div>
    </aside>
  );
}
