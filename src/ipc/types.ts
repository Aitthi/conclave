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

/** A first-class agent role (ADR 0005): a display name, a one-paragraph job
 *  description baked verbatim into the preamble + roster, and a default skill
 *  bundle. Builtin roles ship from a bundled folder (`kind: "builtin"`, id =
 *  folder slug); custom roles are DB rows (`kind: "custom"`, id = uuid). */
export interface Role {
  id: string;
  name: string;
  description: string;
  /** Default skill ids this role attaches when chosen (mandatory skills are
   *  never listed — they attach to every agent regardless). Editable in the
   *  Builder after picking a role; the final selection is what gets saved. */
  skillIds: string[];
  kind: "builtin" | "custom";
}

export interface AgentDefinition {
  id: string;
  name: string;
  /** Legacy free-text role label — kept as a display fallback for rows with no
   *  `roleId` (ADR 0005). New saves write both `roleId` and this (= the role's
   *  display name). */
  role?: string;
  /** The chosen first-class role: a builtin slug (e.g. "lead") or a custom
   *  `Role.id`. Persisted server-side in Phase B; the Builder sends it on save. */
  roleId?: string;
  type: "cli" | "chat" | "orchestrator";
  cliKind?: "claude-code" | "codex" | "custom";
  color?: string;
  providerId?: string;
  model?: string;
  harnessMode: "own" | "central";
  shareBlackboard?: boolean;
  autoSubmitInjected?: boolean;
  allowedSenders?: "all" | "selected" | "none";
  // ── Claude Code / CLI launch config ────────────────────────────────────────
  /** `--permission-mode` value passed to the CLI. */
  permissionMode?: "auto" | "bypassPermissions";
  /** Extra CLI args appended verbatim to the launch command. */
  customArgs?: string;
  /** NON-secret env overrides; secret values live in the Keychain, not here. */
  customEnv?: Record<string, string>;
  /** Names of env vars whose VALUES are stored in the Keychain (not the values). */
  secretEnvKeys?: string[];
  /** Model context window: "1m" appends the [1m] suffix, "200k" is standard. */
  contextWindow?: "1m" | "200k";
  /** Annotated by `agentDef.list`: the FULL set of skill ids (builtin first,
   *  then attached custom, matching the launch-snapshot ordering used by
   *  `repo::skill::content_for_agent`) a `cli` agent would use if launched
   *  right now. Matches `WorkspaceAgent.launchedSkillIds`' basis exactly —
   *  that symmetry is what lets the Roster detect skill drift. */
  skillIds?: string[];
  /** Raw storage: which OPTIONAL builtin skill ids (`mandatory: false`) this
   *  definition has selected (see ADR 0003). `skillIds` above already
   *  reflects the full effective set (mandatory + this list + custom) — the
   *  Builder's checkboxes read `skillIds`, not this field, directly. */
  selectedBuiltinSkillIds?: string[];
  createdAt: string;
  /** Annotated by list views: how many workspaces this agent belongs to. */
  inWorkspaces?: number;
}

export interface Skill {
  id: string;
  name: string;
  description?: string;
  content: string;
  kind: "builtin" | "custom";
  /** Only meaningful when `kind === "builtin"` — a mandatory builtin is
   *  auto-attached to every AgentDefinition and cannot be detached; an
   *  optional one (`mandatory: false` in its SKILL.md frontmatter) is
   *  picked per agent, like a custom skill, but still read-only content
   *  (see ADR 0003). Always `true` for `kind === "custom"` — there's
   *  nothing to opt into, custom skills are already opt-in via agent_skill. */
  mandatory: boolean;
  icon?: string;
  /** Annotated by `skill.list`: how many AgentDefinitions have this attached. */
  attachedTo?: number;
}

export interface WorkspaceAgent {
  id: string;
  workspaceId: string;
  agentDefId: string;
  status: "running" | "idle" | "waiting";
  addedAt: string;
  /** Annotated by `instance.list` (ADR 0005 self-describing roster): the agent
   *  definition's display name. */
  name?: string;
  /** Annotated by `instance.list`: the resolved role name (first-class role, or
   *  the legacy free-text label), absent for a role-less agent. */
  roleName?: string;
  /** Annotated by `instance.list`: the role's one-paragraph job description —
   *  only present when a first-class role resolved (legacy labels have none). */
  roleDescription?: string;
  /** Annotated by `instance.list`: the NAMES of the skills launched with, in
   *  `launchedSkillIds` order (ids that no longer resolve are dropped). */
  skillNames?: string[];
  /** Annotated by `instance.list`: skill ids used at the last launch (see
   *  Session.launchedSkillIds — same value, joined in for the Roster). */
  launchedSkillIds?: string[];
  /** Annotated by `instance.list`, live instances only (R-act-1): whether the
   *  backend emitted output within the last `WORKING_WINDOW` (5s). */
  working?: boolean;
  /** Annotated by `instance.list`, live instances only: ISO-8601 UTC of the
   *  last recorded activity. */
  lastActivityAt?: string;
  /** Annotated by `instance.list`, live instances only: the live session id —
   *  maps a `session:output` event (which carries only `sessionId`) back to
   *  this roster row. */
  sessionId?: string;
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
  // The agent's self-written handoff text on a `handoff` snapshot (the
  // strategic-compact loop); absent on `auto`/`manual` markers.
  carriedForward?: string;
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

export interface Tool {
  id: string;
  name: string;
  kind: "builtin" | "plugin" | "mcp";
  icon?: string;
  isCore: boolean;
}
