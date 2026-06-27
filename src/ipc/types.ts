// Domain entity types for the Conclave IPC layer.
// All fields are camelCase, matching the JSON that crosses the Tauri IPC boundary.
// `string` is used for id and datetime fields throughout.

export interface Workspace {
  id: string;
  name: string;
  folderPath: string;
  color?: string;
  createdAt: string;
}

export interface Provider {
  id: string;
  name: "anthropic" | "openai" | "local" | "custom";
  baseUrl?: string;
  maskedKey?: string;
  storedIn: "keychain";
  status?: "connected" | "offline";
}

export interface AgentDefinition {
  id: string;
  name: string;
  role?: string;
  type: "cli" | "chat" | "orchestrator";
  cliKind?: "claude-code" | "codex" | "custom";
  color?: string;
  providerId?: string;
  model?: string;
  harnessMode: "own" | "central";
  shareBlackboard?: boolean;
  autoSubmitInjected?: boolean;
  allowedSenders?: "all" | "selected" | "none";
  createdAt: string;
  /** Annotated by list views: how many workspaces this agent belongs to. */
  inWorkspaces?: number;
}

export interface WorkspaceAgent {
  id: string;
  workspaceId: string;
  agentDefId: string;
  status: "running" | "idle" | "waiting";
  addedAt: string;
}

export interface Session {
  id: string;
  workspaceAgentId: string;
  contextTokens?: number;
  contextLimit?: number;
  startedAt: string;
  lastActiveAt?: string;
}

export interface Message {
  id: string;
  sessionId: string;
  role: "user" | "agent" | "tool" | "system";
  text?: string;
  fromInstanceId?: string;
  injected?: boolean;
  autoSubmitted?: boolean;
  createdAt: string;
}

export interface InterAgentMessage {
  id: string;
  fromInstanceId: string;
  toInstanceId: string;
  text: string;
  status: "queued" | "delivered";
  autoSubmitted?: boolean;
  createdAt: string;
}

export interface BlackboardEntry {
  id: string;
  workspaceId: string;
  key: string;
  value?: unknown;
  lastWriterId?: string;
  updatedAt: string;
}

export interface BlackboardActivity {
  id: string;
  entryId: string;
  instanceId: string;
  action: "read" | "write";
  at: string;
}

export interface Snapshot {
  id: string;
  sessionId: string;
  type: "auto" | "manual" | "handoff";
  label?: string;
  summary?: string;
  tokens?: number;
  triggerPct?: number;
  prevSnapshotId?: string;
  carriedForward?: unknown;
  diff?: unknown;
  createdAt: string;
}

export interface Artifact {
  id: string;
  messageId: string;
  filename?: string;
  html?: string;
  sandboxed?: boolean;
  createdAt: string;
}

export interface FusionRun {
  id: string;
  sessionId: string;
  prompt: string;
  judgeAnalysis?: unknown;
  synthesized?: string;
  createdAt: string;
}

export interface FusionPanelResponse {
  id: string;
  fusionRunId: string;
  instanceId?: string;
  answer?: string;
  status: "running" | "done" | "error";
  createdAt: string;
}
