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
  Snapshot,
  FusionRun,
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
  "agentDef.list": {
    req: void;
    res: AgentDefinition[];
  };
  "agentDef.save": {
    req: {
      id?: string;
      name: string;
      type: "cli" | "chat" | "orchestrator";
      providerId?: string;
      model?: string;
      harnessMode: "own" | "central";
      toolIds?: string[];
      skillIds?: string[];
      autoSubmitInjected?: boolean;
      allowedSenders?: "all" | "selected" | "none";
    };
    res: AgentDefinition;
  };
  "agentDef.addToWorkspace": {
    req: { agentDefId: string; workspaceIds: string[] };
    res: WorkspaceAgent[];
  };
  "instance.list": {
    req: { workspaceId: string };
    res: WorkspaceAgent[];
  };
  "instance.spawn": {
    req: { workspaceAgentId: string };
    res: Session;
  };
  "message.send": {
    req: { sessionId: string; text: string };
    res: Message;
  };
  "message.inject": {
    req: { fromInstanceId: string; toInstanceId: string; text: string };
    res: InterAgentMessage;
  };
  "blackboard.list": {
    req: { workspaceId: string };
    res: BlackboardEntry[];
  };
  "blackboard.get": {
    req: { workspaceId: string; key: string };
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
    req: { snapshotId: string; mode: "read" | "fork" | "restore" };
    res: void;
  };
  "fusion.run": {
    req: { orchestratorId: string; prompt: string };
    res: FusionRun;
  };
  "provider.upsert": {
    req: { name: Provider["name"]; key?: string; baseUrl?: string };
    res: Provider;
  };
  "cli.exec": {
    req: { argv: string[] };
    res: { stdout: string; exit: number };
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
  },
  agentDef: {
    list: () => call("agentDef.list"),
    save: (req: Commands["agentDef.save"]["req"]) => call("agentDef.save", req),
    addToWorkspace: (req: Commands["agentDef.addToWorkspace"]["req"]) =>
      call("agentDef.addToWorkspace", req),
  },
  instance: {
    list: (req: Commands["instance.list"]["req"]) => call("instance.list", req),
    spawn: (req: Commands["instance.spawn"]["req"]) => call("instance.spawn", req),
  },
  message: {
    send: (req: Commands["message.send"]["req"]) => call("message.send", req),
    inject: (req: Commands["message.inject"]["req"]) => call("message.inject", req),
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
  },
  fusion: {
    run: (req: Commands["fusion.run"]["req"]) => call("fusion.run", req),
  },
  provider: {
    upsert: (req: Commands["provider.upsert"]["req"]) => call("provider.upsert", req),
  },
  cli: {
    exec: (req: Commands["cli.exec"]["req"]) => call("cli.exec", req),
  },
} as const;
