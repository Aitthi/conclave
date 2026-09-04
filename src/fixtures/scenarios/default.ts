import type { FixtureHandlers } from "../backend";
import type { BrowserTab } from "../../ipc/types";
import {
  workspaces,
  agents,
  agentDefs,
  tasks,
  blackboardEntries,
  blackboardActivity,
  artifacts,
  memoryGraph,
  messages,
  roles,
  skills,
  providers,
  draftTeam,
} from "./data";

// One human tab (active) + two agent tabs, one `ended` — exercises every
// side-rail chrome variant (human vs agent, active/inactive, ended badge)
// in a single scenario. `fx-ag-tiesto`/`fx-ag-dew` reuse the fixture agent
// roster ids/names from ./data for a consistent fixture identity.
//
// Mutable module state (not the fixed-literal-timestamps kind — BrowserTab
// carries none): newTab/goto/setActive/close all read AND write this same
// list, and `browser.status` reads it back, so the 2s poll in
// InAppBrowserView doesn't stomp a tab the human just created back out of
// existence with stale static data. Caught in review (Mellow) — a stateless
// per-call echo, like `browser.open`'s below, made the empty scenario's new
// tab visibly vanish ~2s after clicking "+ New tab".
let tabsState: BrowserTab[] = [
  {
    tabId: "human-1",
    owner: { kind: "human", id: "human", label: "You" },
    url: "https://example.com/",
    title: "Example Domain",
    loading: false,
    ended: false,
  },
  {
    tabId: "fx-ag-tiesto",
    owner: { kind: "agent", id: "fx-ag-tiesto", label: "Tiesto" },
    url: "https://developer.mozilla.org/en-US/",
    title: "MDN Web Docs",
    loading: false,
    ended: false,
  },
  {
    tabId: "fx-ag-dew",
    owner: { kind: "agent", id: "fx-ag-dew", label: "Dew" },
    url: "https://github.com/",
    title: "GitHub",
    loading: false,
    ended: true,
  },
];
let activeTabIdState: string | undefined = "human-1";
let humanSeq = 1;

function browserSnapshot(): { tabs: BrowserTab[]; activeTabId?: string } {
  return { tabs: tabsState, activeTabId: activeTabIdState };
}

// Handler coverage for every command the v1 views invoke on their render path.
// `workspace.use` / `session.resize` are no-op void handlers so an incidental
// call (workspace activation, terminal fit) doesn't throw the loud
// missing-handler error. Mutations that only fire on user interaction are
// intentionally absent — reaching one in a screenshot is a real bug worth the
// loud failure.
export const handlers: FixtureHandlers = {
  "workspace.list": () => workspaces,
  "workspace.use": () => undefined,
  "instance.list": ({ workspaceId }) =>
    agents.filter((a) => a.workspaceId === workspaceId),
  // The WorkspacePane auto-opens a session for the focused agent on mount.
  // Synthesize a deterministic one; the Terminal then renders an empty frame
  // (no session:output on the fixture bus) — the accepted PTY-in-fixture v1
  // limitation, not an error.
  "instance.spawn": ({ workspaceAgentId }) => ({
    id: `fx-sess-${workspaceAgentId}`,
    workspaceAgentId,
    contextTokens: 42000,
    contextLimit: 200000,
    startedAt: "2026-07-05T11:00:00.000Z",
    lastActiveAt: "2026-07-05T11:58:00.000Z",
  }),
  "agentDef.list": () => agentDefs,
  "task.list": ({ workspaceId, state }) =>
    tasks.filter((t) => t.workspaceId === workspaceId && (!state || t.state === state)),
  "memory.graph": () => memoryGraph,
  "artifact.list": ({ workspaceId }) =>
    artifacts.filter((a) => a.workspaceId === workspaceId),
  "blackboard.list": ({ workspaceId }) => ({
    entries: blackboardEntries.filter((e) => e.workspaceId === workspaceId),
    activity: blackboardActivity,
  }),
  // Order intentionally shuffled (deterministic id sort ≠ chronological) — the
  // feed must sort by createdAt itself, never render array order. Permanent
  // regression tripwire from task chat-feed-order-check-v2.
  "message.listForWorkspace": () =>
    [...messages].sort((a, b) => a.id.localeCompare(b.id)),
  // A spawned session mounts the context/snapshot UI, which lists snapshots.
  "snapshot.list": ({ sessionId }) => [
    {
      id: `fx-snap-${sessionId}-1`,
      sessionId,
      type: "manual",
      label: "before refactor",
      summary: "Manual checkpoint before the fixture seam refactor.",
      tokens: 38000,
      triggerPct: 19,
      createdAt: "2026-07-05T11:10:00.000Z",
    },
    {
      id: `fx-snap-${sessionId}-2`,
      sessionId,
      type: "auto",
      summary: "Auto-compact at 82% context.",
      tokens: 164000,
      triggerPct: 82,
      prevSnapshotId: `fx-snap-${sessionId}-1`,
      createdAt: "2026-07-05T11:45:00.000Z",
    },
  ],
  "role.list": () => roles,
  // AI drafter: agent mode returns the single new-role agent (no positions),
  // team mode the whole fixed team. Same literal either way — no Date.now().
  "draft.agents": ({ mode }) =>
    mode === "agent"
      ? { ...draftTeam, agents: [draftTeam.agents[1]], positions: [] }
      : draftTeam,
  "skill.list": () => skills,
  "provider.list": () => providers,
  "session.resize": () => undefined,
  // In-app browser: status drives the side rail; the live page renders in a
  // native webview overlay. Fixed literals only — fixture mode never touches
  // Tauri. One human tab (active) + two agent tabs, one of them `ended` —
  // exercises every chrome variant (human vs agent, active/inactive, ended
  // badge) in a single scenario.
  "browser.status": () => browserSnapshot(),
  // Open echoes the requested URL back (deterministic — no Tauri, no clock).
  // Without this handler the view's Open button throws the loud [fixture]
  // error in fixture mode.
  "browser.open": ({ url }) => ({ ok: true, url, title: "Example Domain" }),
  "browser.newTab": () => {
    humanSeq += 1;
    const tabId = `human-${humanSeq}`;
    tabsState = [
      ...tabsState,
      { tabId, owner: { kind: "human", id: "human", label: "You" }, loading: false, ended: false },
    ];
    return { tabId };
  },
  "browser.goto": ({ tabId, url }) => {
    tabsState = tabsState.map((t) =>
      t.tabId === tabId ? { ...t, url, title: "Example Domain", loading: false } : t,
    );
    activeTabIdState = tabId;
    return browserSnapshot();
  },
  "browser.setActive": ({ tabId }) => {
    activeTabIdState = tabId;
    return browserSnapshot();
  },
  "browser.close": ({ tabId }) => {
    tabsState = tabsState.filter((t) => t.tabId !== tabId);
    if (activeTabIdState === tabId) activeTabIdState = tabsState[0]?.tabId;
    return browserSnapshot();
  },
  // UI-only overlay plumbing — fixture mode has no native webview, so these are
  // no-ops (fixed, no Tauri). A missing handler would throw by design.
  "browser.setBounds": () => undefined,
  "browser.setVisible": () => undefined,
  "browser.screenshot": () => ({ path: "/tmp/browser-screenshot.png", width: 1280, height: 800 }),
};
