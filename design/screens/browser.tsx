import { useState } from "react";
import TabRail from "../components/TabRail";
import BrowserChrome from "../components/BrowserChrome";
import PageSurface from "../components/PageSurface";
import { agentTabs, allTabs, humanTabs } from "../lib/tabs";

export const meta = { title: "Browser — per-agent tabs" };

// In-app browser, redesigned as an ownership-first side rail. Each agent drives
// its own live surface; the human has their own interactive tabs plus a read-only
// window into every agent's tab. Click any row to inspect its detail — human tabs
// steer, agent tabs are observed, an ended agent stays for review.
export default function Browser() {
  const [activeTabId, setActiveTabId] = useState("human-1");
  const active = allTabs.find((t) => t.tabId === activeTabId) ?? humanTabs[0];

  return (
    <div className="flex h-screen bg-canvas font-sans text-text-primary antialiased">
      <TabRail
        humanTabs={humanTabs}
        agentTabs={agentTabs}
        activeTabId={activeTabId}
        onSelect={setActiveTabId}
        onNewTab={() => {}}
      />
      <main className="flex min-w-0 flex-1 flex-col bg-surface">
        <BrowserChrome tab={active} />
        <PageSurface tab={active} />
      </main>
    </div>
  );
}
