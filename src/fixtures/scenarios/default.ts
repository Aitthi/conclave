import type { FixtureHandlers } from "../backend";
import type { BrowserTab, Message, Session, Skill, Workspace, WorkspaceAgent } from "../../ipc/types";
import { emitFixtureEvent } from "../events";
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
let workspaceState: Workspace = workspaces[0];
let agentsState: WorkspaceAgent[] = agents.map((agent) => ({ ...agent }));

function fixtureSession(workspaceAgentId: string): Session {
  const agent = agentsState.find((candidate) => candidate.id === workspaceAgentId);
  return {
    id: `fx-sess-${workspaceAgentId}`,
    workspaceAgentId,
    ...(agent?.cliKind === "antigravity"
      ? {}
      : { contextTokens: 42000, contextLimit: 200000 }),
    startedAt: "2026-07-05T11:00:00.000Z",
    lastActiveAt: "2026-07-05T11:58:00.000Z",
  };
}

function browserSnapshot(): { tabs: BrowserTab[]; activeTabId?: string } {
  return { tabs: tabsState, activeTabId: activeTabIdState };
}

// Handler coverage for every command the v1 views invoke on their render path.
// `workspace.use` / `session.resize` are no-op void handlers so an incidental
// call (workspace activation, terminal fit) doesn't throw the loud
// missing-handler error. Mutations that only fire on user interaction are
// intentionally absent — reaching one in a screenshot is a real bug worth the
// loud failure.
// ── Skill-assist harness seam (task skill-assist-repair) ───────────────────
// Mutable module state, like tabsState above. `skillsState` makes skill.save
// durable within a page load so the harness can save then reopen the library.
const DRAFT_WA_ID = "fx-skill-draft-wa";
const DRAFT_SESSION_ID = "fx-skill-draft-session";

let skillsState: typeof skills = [...skills];
let skillDraft = {
  started: false,
  startedWith: null as null | { name: string; description?: string; content: string; agentDefId: string },
  file: { name: "", content: "" } as { name: string; description?: string; content: string },
  startShouldFail: false,
  syncShouldFail: false,
  stopShouldFail: false,
  // Holds startDraftSession open so the harness can close the editor while the
  // call is still in flight (R4's leaked-session case).
  startDelayMs: 0,
};

type ProbeCall = Record<string, unknown> & { cmd: string };
const probe = {
  calls: [] as ProbeCall[],
  sessionId: DRAFT_SESSION_ID,
  workspaceAgentId: DRAFT_WA_ID,
  reset() {
    probe.calls = [];
  },
  /** Stand in for the agent rewriting SKILL.md, so a sync has something new. */
  setDraftFile(file: { name: string; description?: string; content: string }) {
    skillDraft = { ...skillDraft, file };
  },
  setStartDelay(ms: number) {
    skillDraft = { ...skillDraft, startDelayMs: ms };
  },
  fail(which: "start" | "sync" | "stop", on: boolean) {
    if (which === "start") skillDraft = { ...skillDraft, startShouldFail: on };
    if (which === "sync") skillDraft = { ...skillDraft, syncShouldFail: on };
    if (which === "stop") skillDraft = { ...skillDraft, stopShouldFail: on };
  },
  emitOutput(chunk: string) {
    emitFixtureEvent("session:output", { sessionId: DRAFT_SESSION_ID, chunk });
  },
  emitStatus(status: string) {
    emitFixtureEvent("session:status", { sessionId: DRAFT_SESSION_ID, status });
  },
};

// DEV-only test seam: scripts/skill-assist-repro.mjs drives the real components
// and reads this back. `fixtureScenario()` already gates every handler here to
// DEV + ?fixture=, and nothing imports this module in a production build.
(globalThis as unknown as { skillAssistProbe?: typeof probe }).skillAssistProbe = probe;

export const handlers: FixtureHandlers = {
  "workspace.list": () => [workspaceState],
  "workspace.use": () => undefined,
  "workspace.start": ({ workspaceId }) => {
    workspaceState = { ...workspaceState, id: workspaceId, runState: "started" };
    const readyAgentIds = agentsState
      .filter((agent) => agent.availability === "active")
      .map((agent) => agent.id);
    const skippedStoppedAgentIds = agentsState
      .filter((agent) => agent.availability === "stopped")
      .map((agent) => agent.id);
    emitFixtureEvent("workspace:changed", { workspaceId, runState: "started" });
    return { workspace: workspaceState, readyAgentIds, skippedStoppedAgentIds, failures: [] };
  },
  "workspace.stop": ({ workspaceId }) => {
    workspaceState = { ...workspaceState, id: workspaceId, runState: "stopped" };
    const stoppedRuntimeIds = agentsState
      .filter((agent) => agent.availability === "active" && agent.status === "running")
      .map((agent) => agent.id);
    agentsState = agentsState.map((agent) => ({
      ...agent,
      status: "idle",
      working: false,
      sessionId: undefined,
    }));
    emitFixtureEvent("workspace:changed", { workspaceId, runState: "stopped" });
    return { workspace: workspaceState, stoppedRuntimeIds };
  },
  "instance.list": ({ workspaceId }) =>
    agentsState.filter((a) => a.workspaceId === workspaceId),
  "instance.cliStatus": () => ({
    available: true,
    installUrl: "https://antigravity.google/docs/cli/install/",
  }),
  // Deterministic stand-in for `agy models` on an authenticated machine. These
  // ids are FIXTURE DATA, not a product model list — the real catalog comes
  // from the user's own CLI. The long third row is deliberate: it is how the
  // dropdown's truncation gets inspected in the pixel gate.
  "instance.cliModels": () => ({
    models: [
      { id: "gemini-3.8-pro", label: "Gemini 3.8 Pro" },
      { id: "gemini-3.8-flash", label: "Gemini 3.8 Flash" },
      {
        id: "gemini-3.8-pro-experimental-context-extended",
        label: "Gemini 3.8 Pro Experimental (extended context)",
      },
    ],
  }),
  // The WorkspacePane auto-opens a session for the focused agent on mount.
  // Synthesize a deterministic one; the Terminal then renders an empty frame
  // (no session:output on the fixture bus) — the accepted PTY-in-fixture v1
  // limitation, not an error.
  "instance.spawn": ({ workspaceAgentId }) => {
    const agent = agentsState.find((candidate) => candidate.id === workspaceAgentId);
    if (workspaceState.runState !== "started") {
      throw new Error("[fixture] attempted to spawn an agent in a stopped workspace");
    }
    if (!agent || agent.availability !== "active") {
      throw new Error("[fixture] attempted to spawn a stopped or missing agent");
    }
    return fixtureSession(workspaceAgentId);
  },
  "instance.stop": ({ workspaceAgentId }) => {
    agentsState = agentsState.map((agent) =>
      agent.id === workspaceAgentId
        ? { ...agent, availability: "stopped", status: "idle", working: false, sessionId: undefined }
        : agent,
    );
  },
  "instance.resume": ({ workspaceAgentId }) => {
    const session = fixtureSession(workspaceAgentId);
    agentsState = agentsState.map((agent) =>
      agent.id === workspaceAgentId
        ? { ...agent, availability: "active", status: "running", working: false, sessionId: session.id }
        : agent,
    );
    return session;
  },
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
  "skill.list": () => skillsState,
  // ── Skill-editor agent-assist (task skill-assist-repair) ────────────────
  // These back scripts/skill-assist-repro.mjs, which drives the REAL
  // SkillEditor/SkillAssistPanel through this seam. Every call is recorded on
  // `skillAssistProbe` so the harness can assert on what the components
  // actually sent (a PTY resize with positive dims, a paste followed by a
  // standalone CR) rather than on rendered text. Fixed literals only.
  "skill.startDraftSession": ({ name, description, content, agentDefId }) => {
    skillDraft = {
      ...skillDraft,
      started: true,
      startedWith: { name, description, content, agentDefId },
      // The scratch file starts as whatever the editor handed over — the same
      // thing repo::skill::write_draft does.
      file: { name, description, content },
    };
    probe.calls.push({ cmd: "skill.startDraftSession", agentDefId });
    if (skillDraft.startShouldFail) throw new Error("fixture: startDraftSession failed");
    const result = { workspaceAgentId: DRAFT_WA_ID, sessionId: DRAFT_SESSION_ID };
    if (skillDraft.startDelayMs > 0) {
      const ms = skillDraft.startDelayMs;
      return new Promise<typeof result>((resolve) => setTimeout(() => resolve(result), ms));
    }
    return result;
  },
  "skill.syncDraft": () => {
    probe.calls.push({ cmd: "skill.syncDraft" });
    if (skillDraft.syncShouldFail) throw new Error("fixture: SKILL.md is mid-write");
    return skillDraft.file;
  },
  "skill.stopDraftSession": () => {
    probe.calls.push({ cmd: "skill.stopDraftSession" });
    if (skillDraft.stopShouldFail) throw new Error("fixture: stop failed");
    skillDraft = { ...skillDraft, started: false };
    return undefined;
  },
  "skill.save": ({ id, name, description, content }) => {
    probe.calls.push({ cmd: "skill.save", name });
    const saved: Skill = {
      id: id ?? "fx-skill-saved",
      name,
      description,
      content,
      kind: "custom",
      mandatory: true,
    };
    // Persist so the harness can reopen the library and see the saved skill.
    skillsState = skillsState.some((k) => k.id === saved.id)
      ? skillsState.map((k) => (k.id === saved.id ? saved : k))
      : [...skillsState, saved];
    return saved;
  },
  "message.send": ({ sessionId, text, paste }) => {
    probe.calls.push({ cmd: "message.send", sessionId, text, paste: paste === true });
    return {
      id: `fx-msg-${probe.calls.length}`,
      sessionId,
      role: "user",
      text,
      createdAt: "2026-09-05T04:00:00.000Z",
    } as Message;
  },
  "provider.list": () => providers,
  "session.resize": ({ sessionId, cols, rows, pixelWidth, pixelHeight }) => {
    probe.calls.push({ cmd: "session.resize", sessionId, cols, rows, pixelWidth, pixelHeight });
    return undefined;
  },
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
