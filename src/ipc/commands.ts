import { invoke } from "@tauri-apps/api/core";
import type {
  Workspace,
  Provider,
  AgentDefinition,
  WorkspaceAgent,
  Session,
  Message,
  InterAgentMessage,
  BlackboardEntry,
  BlackboardActivity,
  Snapshot,
  FusionRun,
  FusionPanelResponse,
  Tool,
  Skill,
  Role,
  MemoryGraphNode,
  MemoryGraphEdge,
  Task,
  TaskListRow,
  TaskEvent,
  Artifact,
  DesignInfo,
  BrowserState,
  BrowserSnapshot,
  BrowserActionResult,
  BrowserBounds,
  BrowserShot,
  DraftMode,
  DraftResponse,
} from "./types";

// ---------------------------------------------------------------------------
// Command map
// Each entry declares the request payload type and the response type.
// `void` on the req side means the command takes no meaningful payload.
// ---------------------------------------------------------------------------

export interface Commands {
  "workspace.list": {
    req: void;
    res: Workspace[];
  };
  "workspace.link": {
    req: { folderPath: string; name?: string; color?: string; agentDefIds?: string[] };
    res: { workspace: Workspace; agents: WorkspaceAgent[] };
  };
  "workspace.use": {
    req: { workspaceId: string };
    res: void;
  };
  "workspace.update": {
    req: { workspaceId: string; name: string; color?: string };
    res: Workspace;
  };
  "workspace.start": {
    req: { workspaceId: string };
    res: {
      workspace: Workspace;
      readyAgentIds: string[];
      skippedStoppedAgentIds: string[];
      failures: { workspaceAgentId: string; error: string }[];
    };
  };
  "workspace.stop": {
    req: { workspaceId: string };
    res: { workspace: Workspace; stoppedRuntimeIds: string[] };
  };
  "workspace.delete": {
    req: { workspaceId: string };
    res: void;
  };
  "agentDef.list": {
    req: void;
    res: AgentDefinition[];
  };
  "agentDef.save": {
    req: {
      id?: string;
      name: string;
      type: "cli" | "chat" | "orchestrator";
      role?: string;
      /** The chosen first-class role id (builtin slug or custom `Role.id`).
       *  Persisted server-side in Phase B; harmlessly ignored until then. */
      roleId?: string;
      cliKind?: "claude-code" | "codex" | "antigravity" | "custom";
      color?: string;
      providerId?: string;
      model?: string;
      harnessMode: "own" | "central";
      shareBlackboard?: boolean;
      autoSubmitInjected?: boolean;
      allowedSenders?: "all" | "selected" | "none";
      // CLI launch config. `customEnv` is the FULL env map as entered (may
      // include secret values); the backend splits secret-looking keys out to
      // the Keychain and persists only the rest. `secretEnvKeys` is derived
      // server-side, so it is not sent here.
      permissionMode?: "auto" | "default" | "acceptEdits" | "plan" | "bypassPermissions";
      effort?: "low" | "medium" | "high";
      customArgs?: string;
      customEnv?: Record<string, string>;
      contextWindow?: string;
      /** Token filter (rtk) toggle — omitted/`null` means enabled (default ON).
       *  Claude agents only; the engine ignores it for other cli kinds. */
      rtkEnabled?: boolean;
      toolIds?: string[];
      skillIds?: string[];
      // Position System seed (D1/D2, pinned interface contract): omit to leave
      // unchanged on an edit, `null` to clear, a level id to set. Only takes
      // effect for instances created AFTER the save — never retro-applied.
      defaultLevel?: "junior" | "mid" | "senior" | "principal" | null;
    };
    res: AgentDefinition;
  };
  "agentDef.delete": {
    req: { id: string };
    res: void;
  };
  "agentDef.addToWorkspace": {
    req: { agentDefId: string; workspaceIds: string[] };
    res: WorkspaceAgent[];
  };
  "skill.list": {
    req: void;
    res: Skill[];
  };
  "skill.save": {
    req: { id?: string; name: string; description?: string; content: string };
    res: Skill;
  };
  "skill.delete": {
    req: { id: string };
    res: void;
  };
  "skill.startDraftSession": {
    req: { name: string; description?: string; content: string; agentDefId: string };
    res: { workspaceAgentId: string; sessionId: string };
  };
  "skill.syncDraft": {
    req: { workspaceAgentId: string };
    res: { name: string; description?: string; content: string };
  };
  "skill.stopDraftSession": {
    req: { workspaceAgentId: string };
    res: void;
  };
  "role.list": {
    req: void;
    res: Role[];
  };
  "role.save": {
    req: { id?: string; name: string; description: string; skillIds?: string[] };
    res: Role;
  };
  "role.delete": {
    req: { id: string };
    res: void;
  };
  "draft.agents": {
    req: { mode: DraftMode; brief: string; drafterDefId: string; workspaceId?: string };
    res: DraftResponse;
  };
  "instance.list": {
    req: { workspaceId: string };
    res: WorkspaceAgent[];
  };
  "instance.cliStatus": {
    req: { cliKind: "antigravity" };
    res: { available: boolean; installUrl: string };
  };
  "instance.spawn": {
    req: { workspaceAgentId: string };
    res: Session;
  };
  "instance.stop": {
    req: { workspaceAgentId: string };
    res: void;
  };
  "instance.resume": {
    req: { workspaceAgentId: string };
    res: Session;
  };
  "instance.restart": {
    // Restart a CLI agent's process and resume it from a saved handoff.
    // Live agent → save-gated: the backend injects "save your handoff", and the
    // kill → respawn → resume tail fires only once the save lands (phase
    // "saving"). Dead agent → respawns immediately and resumes if a handoff
    // exists (phase "respawning"). Returns as soon as the loop is armed; the
    // rest plays out in the agent's terminal.
    req: { workspaceAgentId: string };
    res: { status: "restarting"; phase: "saving" | "respawning"; instanceId: string };
  };
  "instance.remove": {
    req: { workspaceAgentId: string };
    res: void;
  };
  // Position System (spec §5.1, amended by ruling 71a00512: workspaceId is
  // REQUIRED — the command rejects a mismatch against the agent's real
  // workspace). `level`/`supervisorAgentId` are tri-state: omit the key to keep
  // the current value, `null` to clear, a string to set.
  "instance.setPosition": {
    req: {
      workspaceId: string;
      workspaceAgentId: string;
      level?: string | null;
      supervisorAgentId?: string | null;
    };
    res: WorkspaceAgent;
  };
  "session.resize": {
    req: { sessionId: string; cols: number; rows: number };
    res: void;
  };
  "message.send": {
    /** `paste: true` = composed text delivered as ONE bracketed paste on a PTY
     *  (survives the kernel's ~1 KB read chunking); omit for raw keystrokes. */
    req: { sessionId: string; text: string; paste?: boolean };
    res: Message;
  };
  "message.inject": {
    req: { fromInstanceId: string; toInstanceId: string; text: string };
    res: InterAgentMessage;
  };
  "message.list": {
    req: { instanceId: string; limit?: number };
    res: InterAgentMessage[];
  };
  "message.listForWorkspace": {
    req: { workspaceId: string; limit?: number };
    res: InterAgentMessage[];
  };
  "blackboard.list": {
    req: { workspaceId: string };
    res: { entries: BlackboardEntry[]; activity: BlackboardActivity[] };
  };
  "blackboard.get": {
    req: { workspaceId: string; key: string; readerId?: string };
    res: BlackboardEntry | null;
  };
  "blackboard.set": {
    req: { workspaceId: string; key: string; value: unknown; writerId?: string };
    res: BlackboardEntry;
  };
  "snapshot.create": {
    // `handoff` snapshots are system-generated (agent→agent context handoff),
    // never created from the UI — hence only `auto | manual` here.
    req: { sessionId: string; type: "auto" | "manual"; label?: string };
    res: Snapshot;
  };
  "snapshot.list": {
    req: { sessionId: string };
    res: Snapshot[];
  };
  "snapshot.read": {
    // By-id detail read (Timeline). Fork/Restore are NOT backend operations —
    // they need persisted conversation history (deferred), so the Timeline
    // renders them as honestly disabled rather than calling the backend.
    req: { snapshotId: string };
    res: Snapshot;
  };
  // ── strategic-compact loop (agent self-handoff) ──────────────────────────
  // `save`/`last` are agent-facing (instance-keyed) and reached via the CLI, not
  // the UI. `compact` is the UI entry point: it asks the agent to save a handoff
  // snapshot. Nothing is cleared or restored automatically after that save.
  "snapshot.compact": {
    req: { instanceId: string };
    // Returns immediately once the save prompt is injected. `status` is a fixed
    // acknowledgement, not progress.
    res: { status: "compacting"; instanceId: string };
  };
  "snapshot.resume": {
    // Inject the resume prompt into a live agent so it reloads the last handoff
    // saved for its session (`conclave snapshot last`) and continues from it.
    // Non-destructive (nothing killed or cleared). NotFound when the agent is
    // not running or no handoff snapshot exists.
    req: { instanceId: string };
    res: { status: "resuming"; instanceId: string };
  };
  // Memory-list row actions (replacing the timeline screen).
  "snapshot.delete": {
    req: { snapshotId: string };
    res: { deleted: string };
  };
  "snapshot.send": {
    // Submit a snapshot's content into a live agent's terminal ("send to agent").
    req: { instanceId: string; snapshotId: string };
    res: { status: "sent"; instanceId: string };
  };
  // Knowledge-graph over the workspace semantic memory store. Nodes are chunks
  // (keyed by UUID); edges (`wiki`/`related`) are derived backend-side at query
  // time (ADR 0007). Empty store → `{ nodes: [], edges: [] }`, never an error.
  "memory.graph": {
    req: { workspaceId: string };
    res: { nodes: MemoryGraphNode[]; edges: MemoryGraphEdge[] };
  };
  // ── Agent Work System (task.*) — ADR 0008 ─────────────────────────────────
  // Read-only surface consumed by the Lane Board. Mutations (create/claim/
  // state/note/gate/challenge/rule/close/watch) are agent-facing via the CLI,
  // not the UI, so only `list`/`get` cross the IPC boundary here.
  "task.list": {
    // Every task in the workspace, newest activity first (backend orders).
    // `--state` maps to the optional `state` filter. Empty workspace → `[]`.
    req: { workspaceId: string; state?: Task["state"] };
    res: TaskListRow[];
  };
  "task.get": {
    // One task by slug plus its last 20 events, sorted `createdAt` DESC (newest
    // first — lead ruling @ 5e3e27e). Each event's `payload` arrives as a parsed
    // JSON object, never a double-encoded string. NotFound when the slug doesn't
    // resolve in the workspace.
    req: { workspaceId: string; slug: string };
    res: { task: Task; events: TaskEvent[] };
  };
  // ── Artifacts (artifact.*) — plan design-artifact-store ───────────────────
  // Read surface consumed by the Artifacts view; `add` is agent-facing via the
  // CLI (`conclave artifact add`) but is a valid IPC method too.
  "artifact.add": {
    req: {
      workspaceId: string;
      agentId?: string;
      title: string;
      kind: string;
      filename?: string;
      content: string;
    };
    res: Artifact;
  };
  "artifact.list": {
    // Every artifact in the workspace, newest first. Empty workspace → `[]`.
    req: { workspaceId: string };
    res: Artifact[];
  };
  "artifact.get": {
    // One artifact by id; NotFound when the id doesn't resolve.
    req: { id: string };
    res: Artifact;
  };
  "fusion.run": {
    req: { orchestratorId: string; prompt: string };
    res: FusionRun;
  };
  "fusion.get": {
    req: { runId: string };
    res: { run: FusionRun; responses: FusionPanelResponse[] };
  };
  "provider.upsert": {
    req: { name: Provider["name"]; key?: string; baseUrl?: string };
    res: Provider;
  };
  "provider.list": {
    req: void;
    res: Provider[];
  };
  "tool.list": {
    req: void;
    res: Tool[];
  };
  "cli.exec": {
    // The CLI funnels every subcommand through the allowlisted `cli.exec`
    // router method; the result is whatever the mapped inner method returns,
    // so the shape is not statically known. Callers must validate before use.
    req: { argv: string[] };
    res: unknown;
  };
  // ── design (Phase 1 Lane B) ───────────────────────────────────────────────
  // Scaffolds `.arta/` in the workspace's linked folder if missing, registers
  // it with the sidecar, and ensures the sidecar is running.
  "design.ensure": {
    req: { workspaceId: string };
    res: DesignInfo;
  };
  // Same shape as `ensure`, but never scaffolds, registers, or spawns.
  "design.status": {
    req: { workspaceId: string };
    res: DesignInfo;
  };

  // ── In-app browser agent tools (runtime::browser) ───────────────────────
  "browser.open": { req: { url: string; bounds?: BrowserBounds }; res: BrowserActionResult };
  "browser.goto": { req: { tabId: string; url: string }; res: BrowserState };
  "browser.status": { req: void; res: BrowserState };
  "browser.newTab": { req: void; res: { tabId: string } };
  "browser.setActive": { req: { tabId: string }; res: BrowserState };
  "browser.snapshot": { req: { maxText?: number } | void; res: BrowserSnapshot };
  "browser.click": { req: { selector: string }; res: BrowserActionResult };
  "browser.type": { req: { selector: string; text: string }; res: BrowserActionResult };
  "browser.eval": { req: { js: string }; res: unknown };
  "browser.close": { req: { tabId: string }; res: BrowserState };
  "browser.setBounds": { req: BrowserBounds; res: void };
  "browser.setVisible": { req: { visible: boolean }; res: void };
  "browser.screenshot": { req: { path: string; width?: number; height?: number }; res: BrowserShot };
}

// ---------------------------------------------------------------------------
// Generic call() wrapper
//
// Commands whose `req` is `void` are callable as:   call("workspace.list")
// Commands with a payload are callable as:           call("workspace.use", { workspaceId })
//
// The conditional rest parameter ensures the arg is required when the payload
// type is not void, and absent (or undefined) when it is.
// ---------------------------------------------------------------------------

type CallArgs<K extends keyof Commands> =
  Commands[K]["req"] extends void
    ? [cmd: K]
    : [cmd: K, payload: Commands[K]["req"]];

export async function call<K extends keyof Commands>(
  ...[cmd, payload]: CallArgs<K>
): Promise<Commands[K]["res"]> {
  // `null` (not `{}`) is the Rust-compatible sentinel for void-req commands:
  // serde deserializes a unit / Value::Null from JSON `null`, never from `{}`.
  const safePayload: unknown = payload ?? null;
  if (import.meta.env.DEV) {
    // Dynamic import inside the DEV guard: prod builds drop this branch and
    // never bundle src/fixtures/*.
    const { maybeFixtureCall } = await import("../fixtures/backend");
    const routed = await maybeFixtureCall(cmd, safePayload);
    if (routed.hit) return routed.value as Commands[K]["res"];
  }
  return invoke<Commands[K]["res"]>("ipc", { cmd, payload: safePayload });
}

// ---------------------------------------------------------------------------
// Convenience namespace (optional ergonomic layer over call())
// ---------------------------------------------------------------------------

export const ipc = {
  workspace: {
    list: () => call("workspace.list"),
    link: (req: Commands["workspace.link"]["req"]) => call("workspace.link", req),
    use: (req: Commands["workspace.use"]["req"]) => call("workspace.use", req),
    update: (req: Commands["workspace.update"]["req"]) => call("workspace.update", req),
    start: (req: Commands["workspace.start"]["req"]) => call("workspace.start", req),
    stop: (req: Commands["workspace.stop"]["req"]) => call("workspace.stop", req),
    delete: (req: Commands["workspace.delete"]["req"]) => call("workspace.delete", req),
  },
  agentDef: {
    list: () => call("agentDef.list"),
    save: (req: Commands["agentDef.save"]["req"]) => call("agentDef.save", req),
    delete: (req: Commands["agentDef.delete"]["req"]) => call("agentDef.delete", req),
    addToWorkspace: (req: Commands["agentDef.addToWorkspace"]["req"]) =>
      call("agentDef.addToWorkspace", req),
  },
  skill: {
    list: () => call("skill.list"),
    save: (req: Commands["skill.save"]["req"]) => call("skill.save", req),
    delete: (req: Commands["skill.delete"]["req"]) => call("skill.delete", req),
    startDraftSession: (req: Commands["skill.startDraftSession"]["req"]) =>
      call("skill.startDraftSession", req),
    syncDraft: (req: Commands["skill.syncDraft"]["req"]) => call("skill.syncDraft", req),
    stopDraftSession: (req: Commands["skill.stopDraftSession"]["req"]) =>
      call("skill.stopDraftSession", req),
  },
  role: {
    list: () => call("role.list"),
    save: (req: Commands["role.save"]["req"]) => call("role.save", req),
    delete: (req: Commands["role.delete"]["req"]) => call("role.delete", req),
  },
  draft: {
    agents: (req: Commands["draft.agents"]["req"]) => call("draft.agents", req),
  },
  instance: {
    list: (req: Commands["instance.list"]["req"]) => call("instance.list", req),
    cliStatus: (req: Commands["instance.cliStatus"]["req"]) => call("instance.cliStatus", req),
    spawn: (req: Commands["instance.spawn"]["req"]) => call("instance.spawn", req),
    stop: (req: Commands["instance.stop"]["req"]) => call("instance.stop", req),
    resume: (req: Commands["instance.resume"]["req"]) => call("instance.resume", req),
    restart: (req: Commands["instance.restart"]["req"]) => call("instance.restart", req),
    remove: (req: Commands["instance.remove"]["req"]) => call("instance.remove", req),
    setPosition: (req: Commands["instance.setPosition"]["req"]) =>
      call("instance.setPosition", req),
  },
  session: {
    resize: (req: Commands["session.resize"]["req"]) => call("session.resize", req),
  },
  message: {
    send: (req: Commands["message.send"]["req"]) => call("message.send", req),
    inject: (req: Commands["message.inject"]["req"]) => call("message.inject", req),
    list: (req: Commands["message.list"]["req"]) => call("message.list", req),
    listForWorkspace: (req: Commands["message.listForWorkspace"]["req"]) =>
      call("message.listForWorkspace", req),
  },
  blackboard: {
    list: (req: Commands["blackboard.list"]["req"]) => call("blackboard.list", req),
    get: (req: Commands["blackboard.get"]["req"]) => call("blackboard.get", req),
    set: (req: Commands["blackboard.set"]["req"]) => call("blackboard.set", req),
  },
  snapshot: {
    create: (req: Commands["snapshot.create"]["req"]) => call("snapshot.create", req),
    list: (req: Commands["snapshot.list"]["req"]) => call("snapshot.list", req),
    read: (req: Commands["snapshot.read"]["req"]) => call("snapshot.read", req),
    compact: (req: Commands["snapshot.compact"]["req"]) => call("snapshot.compact", req),
    resume: (req: Commands["snapshot.resume"]["req"]) => call("snapshot.resume", req),
    delete: (req: Commands["snapshot.delete"]["req"]) => call("snapshot.delete", req),
    send: (req: Commands["snapshot.send"]["req"]) => call("snapshot.send", req),
  },
  memory: {
    graph: (req: Commands["memory.graph"]["req"]) => call("memory.graph", req),
  },
  task: {
    list: (req: Commands["task.list"]["req"]) => call("task.list", req),
    get: (req: Commands["task.get"]["req"]) => call("task.get", req),
  },
  artifact: {
    add: (req: Commands["artifact.add"]["req"]) => call("artifact.add", req),
    list: (req: Commands["artifact.list"]["req"]) => call("artifact.list", req),
    get: (req: Commands["artifact.get"]["req"]) => call("artifact.get", req),
  },
  fusion: {
    run: (req: Commands["fusion.run"]["req"]) => call("fusion.run", req),
    get: (req: Commands["fusion.get"]["req"]) => call("fusion.get", req),
  },
  provider: {
    upsert: (req: Commands["provider.upsert"]["req"]) => call("provider.upsert", req),
    list: () => call("provider.list"),
  },
  tool: {
    list: () => call("tool.list"),
  },
  cli: {
    exec: (req: Commands["cli.exec"]["req"]) => call("cli.exec", req),
  },
  design: {
    ensure: (req: Commands["design.ensure"]["req"]) => call("design.ensure", req),
    status: (req: Commands["design.status"]["req"]) => call("design.status", req),
  },
  browser: {
    open: (req: Commands["browser.open"]["req"]) => call("browser.open", req),
    goto: (req: Commands["browser.goto"]["req"]) => call("browser.goto", req),
    status: () => call("browser.status"),
    newTab: () => call("browser.newTab"),
    setActive: (req: Commands["browser.setActive"]["req"]) => call("browser.setActive", req),
    snapshot: (req?: Commands["browser.snapshot"]["req"]) => call("browser.snapshot", req),
    click: (req: Commands["browser.click"]["req"]) => call("browser.click", req),
    type: (req: Commands["browser.type"]["req"]) => call("browser.type", req),
    eval: (req: Commands["browser.eval"]["req"]) => call("browser.eval", req),
    close: (req: Commands["browser.close"]["req"]) => call("browser.close", req),
    setBounds: (req: Commands["browser.setBounds"]["req"]) => call("browser.setBounds", req),
    setVisible: (req: Commands["browser.setVisible"]["req"]) => call("browser.setVisible", req),
    screenshot: (req: Commands["browser.screenshot"]["req"]) => call("browser.screenshot", req),
  },
} as const;
