import { Globe, Plus } from "lucide-react";
import TabRail from "../components/TabRail";

export const meta = { title: "Browser — empty" };

// Empty state: no open tabs. The rail shell persists (its header + "+" stay put,
// so the browser never looks broken) while the detail pane holds a single quiet
// primary action. Agents' sessions land in the rail on their own when they browse.
export default function BrowserEmpty() {
  return (
    <div className="flex h-screen bg-canvas font-sans text-text-primary antialiased">
      <TabRail humanTabs={[]} agentTabs={[]} onNewTab={() => {}} />
      <main className="grid min-w-0 flex-1 place-items-center bg-surface">
        <div className="flex max-w-sm flex-col items-center px-8 text-center">
          <div className="grid h-14 w-14 place-items-center rounded-panel bg-fill text-text-tertiary">
            <Globe className="h-7 w-7" />
          </div>

          <h1 className="mt-5 text-[19px] font-semibold tracking-tight text-text-primary">No open tabs</h1>
          <p className="mt-2 text-[13px] leading-relaxed text-text-secondary">
            Open a page to browse it yourself. When an agent starts browsing, its session shows up in the rail
            as a read-only tab.
          </p>

          <button
            type="button"
            className="mt-6 inline-flex items-center gap-1.5 rounded-md bg-accent px-3.5 py-2 text-[13px] font-medium text-white transition-colors hover:bg-accent-hover"
          >
            <Plus className="h-4 w-4" />
            New tab
          </button>
        </div>
      </main>
    </div>
  );
}
