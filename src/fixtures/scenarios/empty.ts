import type { FixtureHandlers } from "../backend";
import type { Workspace } from "../../ipc/types";

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
    createdAt: "2026-07-05T12:00:00.000Z",
  },
];

export const handlers: FixtureHandlers = {
  "workspace.list": () => emptyWorkspace,
  "workspace.use": () => undefined,
  "instance.list": () => [],
  "agentDef.list": () => [],
  "task.list": () => [],
  "memory.graph": () => ({ nodes: [], edges: [] }),
  "artifact.list": () => [],
  "blackboard.list": () => ({ entries: [], activity: [] }),
  "message.listForWorkspace": () => [],
  "snapshot.list": () => [],
  "role.list": () => [],
  "skill.list": () => [],
  "provider.list": () => [],
  "session.resize": () => undefined,
  // Fresh-install look: no tabs. The side rail renders its own empty state
  // (no tabs, just the "+" affordance) and never calls snapshot.
  "browser.status": () => ({ tabs: [], activeTabId: undefined }),
  // Open still works from the empty state (deterministic echo, no Tauri) —
  // a missing handler would throw the loud [fixture] error on first click.
  "browser.open": ({ url }) => ({ ok: true, url }),
  "browser.close": () => ({ tabs: [], activeTabId: undefined }),
  "browser.setBounds": () => undefined,
  "browser.setVisible": () => undefined,
  "browser.screenshot": () => ({ path: "/tmp/browser-screenshot.png", width: 1280, height: 800 }),
};
