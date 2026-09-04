import type { FixtureHandlers } from "../backend";
import type { BrowserTab, Workspace } from "../../ipc/types";
import { emitFixtureEvent } from "../events";

// `empty` scenario — the "fresh install" look. DECISION (F3 Step 4): keep ONE
// linked-but-empty workspace rather than zero. With zero workspaces the shell
// falls through to the generic "Select a workspace" prompt for every view; a
// single empty workspace instead exercises each view's OWN empty state (empty
// roster, empty lane board, empty memory graph), which is what a designer
// reviewing the fresh-install experience actually wants to see. Every child
// collection is empty; the same handler keys as `default`.
const emptyWorkspace: Workspace[] = [
  {
    id: "fx-ws-empty",
    name: "new-project",
    folderPath: "/Users/dev/code/new-project",
    color: "#7c6af2",
    runState: "stopped",
    createdAt: "2026-07-05T12:00:00.000Z",
  },
];

// Mutable module state (see default.ts for why: the 2s status() poll must
// see what newTab/goto/setActive/close just did, or a tab the human creates
// from this empty state visibly vanishes on the next tick).
let tabsState: BrowserTab[] = [];
let activeTabIdState: string | undefined = undefined;
let humanSeq = 0;

function browserSnapshot(): { tabs: BrowserTab[]; activeTabId?: string } {
  return { tabs: tabsState, activeTabId: activeTabIdState };
}

export const handlers: FixtureHandlers = {
  "workspace.list": () => emptyWorkspace,
  "workspace.use": () => undefined,
  "workspace.start": ({ workspaceId }) => {
    const workspace: Workspace = { ...emptyWorkspace[0], id: workspaceId, runState: "started" };
    emptyWorkspace[0] = workspace;
    emitFixtureEvent("workspace:changed", { workspaceId, runState: "started" });
    return {
      workspace,
      readyAgentIds: [],
      skippedStoppedAgentIds: [],
      failures: [],
    };
  },
  "workspace.stop": ({ workspaceId }) => {
    const workspace: Workspace = { ...emptyWorkspace[0], id: workspaceId, runState: "stopped" };
    emptyWorkspace[0] = workspace;
    emitFixtureEvent("workspace:changed", { workspaceId, runState: "stopped" });
    return { workspace, stoppedRuntimeIds: [] };
  },
  "instance.list": () => [],
  "agentDef.list": () => [],
  "task.list": () => [],
  "memory.graph": () => ({ nodes: [], edges: [] }),
  "artifact.list": () => [],
  "blackboard.list": () => ({ entries: [], activity: [] }),
  "message.listForWorkspace": () => [],
  "snapshot.list": () => [],
  "role.list": () => [],
  // Fresh install: nothing to draft with — the drafter renders its empty state.
  "draft.agents": () => ({
    agents: [],
    positions: [],
    notes: "",
    drafter: { defId: "", cliKind: "", model: "" },
  }),
  "skill.list": () => [],
  "provider.list": () => [],
  "session.resize": () => undefined,
  // Fresh-install look: no tabs. The side rail renders its own empty state
  // (no tabs, just the "+" affordance) and never calls snapshot.
  "browser.status": () => browserSnapshot(),
  // Open still works from the empty state (deterministic echo, no Tauri) —
  // a missing handler would throw the loud [fixture] error on first click.
  "browser.open": ({ url }) => ({ ok: true, url }),
  // The empty state's own "+ New tab" button drives newTab() -> setActive()
  // (InAppBrowserView's doNewTab), so both need a handler here too, not just
  // in default.ts — a missing one throws the loud [fixture] error on click
  // (caught in review: FixtureHandlers is Partial, so tsc doesn't flag a
  // scenario that's missing a handler another scenario defines).
  "browser.newTab": () => {
    humanSeq += 1;
    const tabId = `human-${humanSeq}`;
    tabsState = [
      ...tabsState,
      { tabId, owner: { kind: "human", id: "human", label: "You" }, loading: false, ended: false },
    ];
    return { tabId };
  },
  "browser.setActive": ({ tabId }) => {
    activeTabIdState = tabId;
    return browserSnapshot();
  },
  "browser.goto": ({ tabId, url }) => {
    tabsState = tabsState.map((t) =>
      t.tabId === tabId ? { ...t, url, title: "Example Domain", loading: false } : t,
    );
    activeTabIdState = tabId;
    return browserSnapshot();
  },
  "browser.close": ({ tabId }) => {
    tabsState = tabsState.filter((t) => t.tabId !== tabId);
    if (activeTabIdState === tabId) activeTabIdState = tabsState[0]?.tabId;
    return browserSnapshot();
  },
  "browser.setBounds": () => undefined,
  "browser.setVisible": () => undefined,
  "browser.screenshot": () => ({ path: "/tmp/browser-screenshot.png", width: 1280, height: 800 }),
};
