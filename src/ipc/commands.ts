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
      cliKind?: "claude-code" | "codex" | "custom";
      color?: string;
      providerId?: string;
      model?: string;
      harnessMode: "own" | "central";
      shareBlackboard?: boolean;
      autoSubmitInjected?: boolean;
      allowedSenders?: "all" | "selected" | "none";
      // Claude Code / CLI launch config. `customEnv` is the FULL env map as
      // entered (may include secret values); the backend splits secret-looking
      // keys out to the Keychain and persists only the rest. `secretEnvKeys` is
      // derived server-side, so it is not sent here.
      permissionMode?: "auto" | "bypassPermissions";
      customArgs?: string;
      customEnv?: Record<string, string>;
      contextWindow?: "1m" | "200k";
      toolIds?: string[];
      skillIds?: string[];
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
  "instance.list": {
    req: { workspaceId: string };
    res: WorkspaceAgent[];
  };
  "instance.spawn": {
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
  "session.resize": {
    req: { sessionId: string; cols: number; rows: number };
    res: void;
  };
  "message.send": {
    req: { sessionId: string; text: string };
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
  // the UI. `compact` is the UI entry point: it drives the whole loop (inject
  // "save your handoff" → wait for the handoff snapshot → /clear → "restore").
  "snapshot.compact": {
    req: { instanceId: string };
    // Returns immediately once the save prompt is injected; the loop then runs in
    // the agent's terminal. `status` is a fixed acknowledgement, not progress.
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
  instance: {
    list: (req: Commands["instance.list"]["req"]) => call("instance.list", req),
    spawn: (req: Commands["instance.spawn"]["req"]) => call("instance.spawn", req),
    restart: (req: Commands["instance.restart"]["req"]) => call("instance.restart", req),
    remove: (req: Commands["instance.remove"]["req"]) => call("instance.remove", req),
  },
  session: {
    resize: (req: Commands["session.resize"]["req"]) => call("session.resize", req),
  },
  message: {
    send: (req: Commands["message.send"]["req"]) => call("message.send", req),
    inject: (req: Commands["message.inject"]["req"]) => call("message.inject", req),
    list: (req: Commands["message.list"]["req"]) => call("message.list", req),
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
} as const;
