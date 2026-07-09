import type { FixtureHandlers } from "../backend";
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
} from "./data";

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
  "message.listForWorkspace": () => messages,
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
  "skill.list": () => skills,
  "provider.list": () => providers,
  "session.resize": () => undefined,
  // In-app browser control surface: status drives the header, snapshot fills
  // the inspector. Fixed literals only — fixture mode never touches Tauri.
  "browser.status": () => ({
    ok: true,
    url: "https://example.com/",
    title: "Example Domain",
  }),
  // UI-only overlay plumbing — fixture mode has no native webview, so these are
  // no-ops (fixed, no Tauri). A missing handler would throw by design.
  "browser.setBounds": () => ({ ok: true }),
  "browser.setVisible": () => ({ ok: true }),
};
