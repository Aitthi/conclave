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
  /** Harness-specific context window config.
   *  Claude Code: "1m" appends the [1m] suffix; "200k" is the standard model.
   *  Codex: decimal token count passed as -c model_context_window=<tokens>. */
  contextWindow?: string;
  /** Token filter (rtk) toggle — `null`/absent means enabled (default ON).
   *  Claude agents only (wire name contract: DB `rtk_enabled`). */
  rtkEnabled?: boolean | null;
  /** Context proxy toggle — `null`/absent means enabled (default ON).
   *  Claude agents only; NULL = ON (rtk-parity). DB `proxy_enabled`. */
  proxyEnabled?: boolean | null;
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
  /** Position System seed (spec: default-level-supervisor-picker plan, D1): the
   *  level a new instance is created with. A SEED, not a live link — once an
   *  instance exists, `WorkspaceAgent.level` is the only value the engine
   *  reads; changing this never retro-writes existing instances (D2). */
  defaultLevel?: "junior" | "mid" | "senior" | "principal" | null;
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
  /** Annotated by `instance.list`: current context usage for the live session,
   *  `None` when no session row is present. */
  contextTokens?: number;
  /** Annotated by `instance.list`: the corresponding context limit for the live
   *  session, `None` when no session row is present. */
  contextLimit?: number;
  /** Annotated by `instance.list`: the agent definition's configured model id
   *  (e.g. "claude-sonnet-5"), absent when unset. */
  model?: string;
  /** Annotated by `instance.list`: the agent definition's CLI harness kind,
   *  absent for a chat/orchestrator agent or an unset `cli` agent. */
  cliKind?: "claude-code" | "codex" | "custom";
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
  /** Position System (spec §5.2): ordered seniority within the agent's track —
   *  one of junior|mid|senior|principal, absent when unset. */
  level?: string;
  /** Position System: the supervising workspace-agent instance id; absent
   *  reports to the human/top. */
  supervisorAgentId?: string;
  /** Position System: resolved display name of `supervisorAgentId`, absent when
   *  no supervisor is set. */
  supervisorName?: string;
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
  /**
   * Chat-parsed rows still carry a `messageId`; workspace-scoped artifacts
   * created via `conclave artifact add` (migration 0014) leave it null, so it
   * is optional now. `html` is likewise retained for the old rows — new
   * artifacts store their body in `content` under a typed `kind`.
   */
  messageId?: string;
  workspaceId?: string;
  agentId?: string;
  title?: string;
  /** markdown | code | html | svg | mermaid | react | text */
  kind?: string;
  filename?: string;
  content?: string;
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

// ── Memory knowledge-graph (memory.graph) ──────────────────────────────────
// One durable memory chunk from the workspace semantic store. Node identity is
// the chunk UUID (never a slug). `sourceKind:"agent"` carries the authoring
// instance id in `sourceId`; `"manual"` chunks have a null `sourceId` and render
// in the neutral "Shared" group. Edges are derived backend-side at query time
// (ADR 0007) — never stored.
export interface MemoryGraphNode {
  /** Chunk UUID — the node's stable identity across every keyed map. */
  id: string;
  text: string;
  sourceKind: "manual" | "agent";
  /** Authoring instance id when `sourceKind === "agent"`, else null. */
  sourceId: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface MemoryGraphEdge {
  a: string;
  b: string;
  /** "wiki" = a shared explicit `[[token]]`; "related" = embedding proximity. */
  rel: "wiki" | "related";
  /** Cosine similarity for `related` edges; absent for `wiki`. */
  score?: number;
}

// ── Agent Work System (task.*) — ADR 0008 ───────────────────────────────────
// A claimable unit of work with a lifecycle state machine, a file boundary, an
// optional design canon, and an append-only event log. Fields are the EXACT
// frozen wire shape (plan §Frozen interfaces); Lane A's `task.*` handlers emit
// this camelCase shape, Lane D codes against it.
export type TaskState =
  | "planned"
  | "claimed"
  | "in_progress"
  | "review"
  | "merged"
  | "abandoned";

export interface Task {
  /** UUID — the task's stable identity. */
  id: string;
  workspaceId: string;
  /** Short human key, unique within the workspace (the `<slug>` CLI arg). */
  slug: string;
  title: string;
  state: TaskState;
  /** The lead who owns escalations for this task. Optional: Lane A omits the
   *  field entirely when absent (never serialises `null`) — lead ruling. */
  ownerAgentId?: string;
  /** The agent who claimed and builds it. Omitted until claimed. */
  implementerAgentId?: string;
  /** Path prefixes this task's lane owns (the frozen `file_boundary` JSON array). */
  fileBoundary: string[];
  /** Free-text pointer to the design canon (proto path / SHA). Omitted for
   *  non-UI work. */
  designCanon?: string;
  /** The task's written plan text. */
  plan: string;
  createdAt: string;
  updatedAt: string;
}

/** The newest gate result PER DISTINCT command on a task — `task.list` carries
 *  the latest of each `cmd` (cap 6), so a card shows `cargo test` + `clippy`
 *  side by side like the canon, while rows stay bounded (not O(events)); full
 *  gate history lives in `task.get` (lead ruling on the badge escalation, amended
 *  from a single `lastGate` per Arta's F1). Derived from `task_event(kind=
 *  'gate')` payloads `{cmd,exit,sha,tail,cwd}`. EVIDENCE: `sha` pins the commit
 *  it ran at, so a shared-tree "all green" can be checked against the commit
 *  that produced it. The human-facing label is derived client-side from `cmd`
 *  (no stored `label`). */
export interface TaskLastGate {
  cmd: string;
  /** process exit code — 0 = green, non-zero = red. */
  exit: number;
  /** `git rev-parse HEAD` captured when the gate ran. */
  sha: string;
  createdAt: string;
}

/** A challenge summary derived from `task_event(kind='challenge')` and its
 *  resolving `kind='ruling'` — `open` = awaiting a ruling, `ruled` = resolved.
 *  `deadlineAt` is an ISO timestamp (absent = advisory); the client computes
 *  minutes-remaining live, since a server-sent countdown is stale on arrival
 *  (lead ruling). */
export interface TaskChallengeBadge {
  /** the challenge event's id — stable render key + the ruling's target. */
  id: string;
  status: "open" | "ruled";
  claim: string;
  deadlineAt?: string;
}

/** A `task.list` row — the frozen task shape plus the count of its logged events
 *  and the derived badges the board renders per card. `eventCount` is the
 *  lead-ruled field @ 5e3e27e; `lastGates`/`challenges` are the lead's ruling on
 *  Lane D's badge escalation (amended @ 066e199): the newest gate per distinct
 *  cmd (cap 6) and the challenge set — both ALWAYS present (`[]` when none).
 *  Lane A derives both during list. */
export interface TaskListRow extends Task {
  eventCount: number;
  lastGates: TaskLastGate[];
  challenges: TaskChallengeBadge[];
}

export type TaskEventKind = "note" | "state" | "gate" | "challenge" | "ruling";

/** One append-only entry in a task's event log (the frozen `task_event` row,
 *  camelCase). `payload` is opaque JSON whose shape varies by `kind` (e.g. a
 *  `gate` event carries `{cmd,exit,sha,tail,cwd}`); callers narrow per kind. */
export interface TaskEvent {
  id: string;
  taskId: string;
  kind: TaskEventKind;
  /** The agent who produced the event. Omitted for system-generated entries. */
  actorAgentId?: string;
  payload: unknown;
  createdAt: string;
}

/** Result of `design.ensure`/`design.status` — the design-canvas viewer
 *  sidecar's state for one workspace. `url`/`port` are present only when
 *  `running` is true; the frontend must never hardcode the port (the sidecar
 *  can bump to the next free one). */
export interface DesignInfo {
  projectId: string;
  running: boolean;
  url?: string;
  port?: number;
}

// ── In-app browser agent tools (runtime::browser) ─────────────────────────
// Mirrors the Rust `#[serde(rename_all = "camelCase")]` result structs. `ok`
// is false (with a `message`) when there is no browser open to report on.
export interface BrowserStatus {
  ok: boolean;
  url?: string;
  title?: string;
  message?: string;
}

export interface BrowserActionResult {
  ok: boolean;
  url?: string;
  message?: string;
}

export interface BrowserSnapshotLink {
  text: string;
  href: string;
  selector: string;
}

export interface BrowserSnapshotInput {
  selector: string;
  type?: string;
  name?: string;
}

export interface BrowserSnapshotButton {
  text: string;
  selector: string;
}

export interface BrowserSnapshot {
  url: string;
  title: string;
  text: string;
  headings: string[];
  links: BrowserSnapshotLink[];
  inputs: BrowserSnapshotInput[];
  buttons: BrowserSnapshotButton[];
}

// Viewport rectangle the Browser tab reserves for the native webview overlay
// (logical pixels). Mirrors the Rust `Bounds` struct.
export interface BrowserBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

// Result of browser.screenshot — the written PNG path + capture dims. Mirrors
// the Rust `BrowserShot` struct.
export interface BrowserShot {
  path: string;
  width: number;
  height: number;
}
