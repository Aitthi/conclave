// Shared `default` scenario dataset — a populated "codeup" workspace rendered
// with deterministic, byte-reproducible data. Every timestamp is a fixed
// literal derived by hand from T0 = 2026-07-05T12:00:00.000Z; there is NO
// Date.now()/Math.random() anywhere here (Global Constraint: screenshots must
// be byte-reproducible). Every value is a fully-typed literal — no `as`-casts,
// so the compiler is the completeness + drift alarm.

import type {
  Workspace,
  WorkspaceAgent,
  AgentDefinition,
  TaskListRow,
  BlackboardEntry,
  BlackboardActivity,
  Artifact,
  MemoryGraphNode,
  MemoryGraphEdge,
  InterAgentMessage,
  Role,
  Skill,
  Provider,
} from "../../ipc/types";

export const WS_ID = "fx-ws-codeup";

// ── Agent instance ids (referenced across messages/tasks/blackboard) ────────
const AG_DETORO = "fx-ag-detoro";
const AG_MELLOW = "fx-ag-mellow";
const AG_TIESTO = "fx-ag-tiesto";
const AG_DEW = "fx-ag-dew";
const AG_ARTA = "fx-ag-arta";

// ── Workspaces ──────────────────────────────────────────────────────────────
export const workspaces: Workspace[] = [
  {
    id: WS_ID,
    name: "codeup",
    folderPath: "/Users/dev/code/codeup",
    color: "#7c6af2",
    createdAt: "2026-07-01T09:00:00.000Z",
  },
];

// ── Agent definitions (agentDef.list) ───────────────────────────────────────
export const agentDefs: AgentDefinition[] = [
  {
    id: "fx-def-detoro",
    name: "Detoro",
    role: "Lead",
    roleId: "lead",
    type: "cli",
    cliKind: "claude-code",
    color: "#7c6af2",
    model: "claude-opus-4-8",
    harnessMode: "own",
    contextWindow: "1m",
    rtkEnabled: true,
    proxyEnabled: true,
    defaultLevel: "principal",
    createdAt: "2026-07-01T09:05:00.000Z",
  },
  {
    id: "fx-def-mellow",
    name: "Mellow",
    role: "Reviewer",
    roleId: "reviewer",
    type: "cli",
    cliKind: "claude-code",
    color: "#3fb27f",
    model: "claude-sonnet-5",
    harnessMode: "own",
    contextWindow: "200k",
    defaultLevel: "senior",
    createdAt: "2026-07-01T09:06:00.000Z",
  },
  {
    id: "fx-def-tiesto",
    name: "Tiesto",
    role: "Implementer",
    roleId: "implementer",
    type: "cli",
    cliKind: "claude-code",
    color: "#e0863b",
    model: "claude-opus-4-8",
    harnessMode: "own",
    contextWindow: "1m",
    rtkEnabled: false,
    proxyEnabled: false,
    defaultLevel: "senior",
    createdAt: "2026-07-01T09:07:00.000Z",
  },
  {
    id: "fx-def-dew",
    name: "Dew",
    role: "Implementer",
    roleId: "implementer",
    type: "cli",
    cliKind: "codex",
    color: "#4a9fd8",
    model: "gpt-5-codex",
    harnessMode: "own",
    contextWindow: "258400",
    defaultLevel: "mid",
    createdAt: "2026-07-01T09:08:00.000Z",
  },
  {
    id: "fx-def-arta",
    name: "Arta",
    role: "Designer",
    roleId: "designer",
    type: "cli",
    cliKind: "claude-code",
    color: "#d857a8",
    model: "claude-opus-4-8",
    harnessMode: "own",
    contextWindow: "200k",
    defaultLevel: "senior",
    createdAt: "2026-07-01T09:09:00.000Z",
  },
];

// ── Agent instances (instance.list) ─────────────────────────────────────────
// Status "idle" with no sessionId so the WorkspacePane renders roster tabs
// without opening a live Terminal (which would fire session.* commands).
export const agents: WorkspaceAgent[] = [
  {
    id: AG_DETORO,
    workspaceId: WS_ID,
    agentDefId: "fx-def-detoro",
    status: "running",
    addedAt: "2026-07-01T09:10:00.000Z",
    name: "Detoro",
    roleName: "Lead",
    roleDescription: "Owns the plan, rules escalations, and integrates lanes.",
    model: "claude-opus-4-8",
    cliKind: "claude-code",
    working: false,
    lastActivityAt: "2026-07-05T11:52:00.000Z",
    level: "principal",
  },
  {
    id: AG_MELLOW,
    workspaceId: WS_ID,
    agentDefId: "fx-def-mellow",
    status: "idle",
    addedAt: "2026-07-01T09:11:00.000Z",
    name: "Mellow",
    roleName: "Reviewer",
    roleDescription: "Reviews deliverables against the plan and the spec.",
    model: "claude-sonnet-5",
    cliKind: "claude-code",
    working: false,
    lastActivityAt: "2026-07-05T11:40:00.000Z",
    level: "senior",
    supervisorAgentId: AG_DETORO,
    supervisorName: "Detoro",
  },
  {
    id: AG_TIESTO,
    workspaceId: WS_ID,
    agentDefId: "fx-def-tiesto",
    status: "running",
    addedAt: "2026-07-01T09:12:00.000Z",
    name: "Tiesto",
    roleName: "Implementer",
    roleDescription: "Turns the plan into verified software, one lane at a time.",
    model: "claude-opus-4-8",
    cliKind: "claude-code",
    working: true,
    lastActivityAt: "2026-07-05T11:58:00.000Z",
    level: "senior",
    supervisorAgentId: AG_DETORO,
    supervisorName: "Detoro",
  },
  {
    id: AG_DEW,
    workspaceId: WS_ID,
    agentDefId: "fx-def-dew",
    status: "idle",
    addedAt: "2026-07-01T09:13:00.000Z",
    name: "Dew",
    roleName: "Implementer",
    roleDescription: "Builds the uishot capture CLI lane.",
    model: "gpt-5-codex",
    cliKind: "codex",
    working: false,
    lastActivityAt: "2026-07-05T11:30:00.000Z",
    level: "mid",
    supervisorAgentId: AG_DETORO,
    supervisorName: "Detoro",
  },
  {
    id: AG_ARTA,
    workspaceId: WS_ID,
    agentDefId: "fx-def-arta",
    status: "idle",
    addedAt: "2026-07-01T09:14:00.000Z",
    name: "Arta",
    roleName: "Designer",
    roleDescription: "Owns the design canon and the design-acceptance gate.",
    model: "claude-opus-4-8",
    cliKind: "claude-code",
    working: false,
    lastActivityAt: "2026-07-05T10:20:00.000Z",
    level: "senior",
    supervisorAgentId: AG_DETORO,
    supervisorName: "Detoro",
  },
];

// ── Tasks (task.list) — five across the lifecycle, with gates + a challenge ──
export const tasks: TaskListRow[] = [
  {
    id: "fx-task-1",
    workspaceId: WS_ID,
    slug: "uishot-fixture-mode",
    title: "Fixture mode: DEV-only IPC seam + scenarios",
    state: "in_progress",
    ownerAgentId: AG_DETORO,
    implementerAgentId: AG_TIESTO,
    fileBoundary: ["src/fixtures", "src/ipc/commands.ts", "src/ipc/events.ts"],
    plan: "Add a DEV-only fixture backend behind the single IPC seam so the real app renders in plain Chrome with deterministic data.",
    createdAt: "2026-07-05T09:15:00.000Z",
    updatedAt: "2026-07-05T11:58:00.000Z",
    eventCount: 7,
    lastGates: [
      {
        cmd: "pnpm build",
        exit: 0,
        sha: "a9f4b01abce7",
        createdAt: "2026-07-05T11:20:00.000Z",
      },
    ],
    challenges: [],
  },
  {
    id: "fx-task-2",
    workspaceId: WS_ID,
    slug: "uishot-cli",
    title: "uishot capture CLI (puppeteer-core)",
    state: "claimed",
    ownerAgentId: AG_DETORO,
    implementerAgentId: AG_DEW,
    fileBoundary: ["scripts/uishot.mjs", "package.json"],
    plan: "Port Arta's headless-snapshot core into scripts/uishot.mjs, driving the fixture URL + readiness sentinel.",
    createdAt: "2026-07-05T09:16:00.000Z",
    updatedAt: "2026-07-05T11:30:00.000Z",
    eventCount: 3,
    lastGates: [],
    challenges: [
      {
        id: "fx-chal-1",
        status: "open",
        claim: "The sentinel selector should be data-attr, not a class",
        deadlineAt: "2026-07-05T14:00:00.000Z",
      },
    ],
  },
  {
    id: "fx-task-3",
    workspaceId: WS_ID,
    slug: "design-shell-remember",
    title: "Design view remembers active screen across reloads",
    state: "review",
    ownerAgentId: AG_DETORO,
    implementerAgentId: AG_MELLOW,
    fileBoundary: ["design-host/src/Shell.tsx"],
    plan: "Persist the active screen to the URL hash + localStorage; restore on mount with precedence hash → localStorage → default.",
    createdAt: "2026-07-05T09:17:00.000Z",
    updatedAt: "2026-07-05T11:05:00.000Z",
    eventCount: 5,
    lastGates: [
      {
        cmd: "node --test",
        exit: 0,
        sha: "b7c2f01d9a34",
        createdAt: "2026-07-05T11:00:00.000Z",
      },
    ],
    challenges: [],
  },
  {
    id: "fx-task-4",
    workspaceId: WS_ID,
    slug: "supervisor-picker-remove",
    title: "Edit-variant Remove supervisor affordance",
    state: "merged",
    ownerAgentId: AG_DETORO,
    implementerAgentId: AG_TIESTO,
    fileBoundary: ["src/components/SupervisorPicker.tsx"],
    plan: "Add a one-click Remove supervisor affordance to the edit variant per the Arta canon.",
    designCanon: ".arta/proto/screens/supervisor-picker.tsx @d2ac161",
    createdAt: "2026-07-04T14:00:00.000Z",
    updatedAt: "2026-07-05T08:30:00.000Z",
    eventCount: 9,
    lastGates: [
      {
        cmd: "pnpm build",
        exit: 0,
        sha: "0e180d1abc12",
        createdAt: "2026-07-05T08:20:00.000Z",
      },
    ],
    challenges: [],
  },
  {
    id: "fx-task-6",
    workspaceId: WS_ID,
    slug: "artifacts-markdown-render",
    title: "Artifacts view renders markdown as formatted markdown",
    state: "merged",
    ownerAgentId: AG_DETORO,
    implementerAgentId: AG_DETORO,
    fileBoundary: ["src/components/Artifacts.tsx", "src/lib/markdown.ts"],
    plan: "Render stored markdown artifacts through the shared markdown renderer instead of a raw <pre> block.",
    createdAt: "2026-07-04T12:00:00.000Z",
    updatedAt: "2026-07-05T06:40:00.000Z",
    eventCount: 6,
    lastGates: [
      {
        cmd: "pnpm uishot",
        exit: 0,
        sha: "c3e56d0a71b8",
        createdAt: "2026-07-05T06:30:00.000Z",
      },
    ],
    challenges: [],
  },
  {
    id: "fx-task-7",
    workspaceId: WS_ID,
    slug: "inapp-browser-ended-detection",
    title: "Flip agent browser tab to ended on terminal exit",
    state: "merged",
    ownerAgentId: AG_TIESTO,
    implementerAgentId: AG_MELLOW,
    fileBoundary: ["src-tauri/src/engine/browser.rs", "src/components/Browser.tsx"],
    plan: "Detect terminal exit and transition the in-app browser tab to the ended state so no stale live view lingers.",
    createdAt: "2026-07-04T13:30:00.000Z",
    updatedAt: "2026-07-05T07:20:00.000Z",
    eventCount: 8,
    lastGates: [
      {
        cmd: "cargo test",
        exit: 0,
        sha: "ee36de2b90c4",
        createdAt: "2026-07-05T07:10:00.000Z",
      },
    ],
    challenges: [],
  },
  {
    id: "fx-task-5",
    workspaceId: WS_ID,
    slug: "welcome-screen",
    title: "Welcome screen + design tokens",
    state: "planned",
    ownerAgentId: AG_DETORO,
    fileBoundary: ["src/components/Welcome.tsx", ".arta/runtime.json"],
    plan: "Implement the welcome screen and define the design-system tokens.",
    createdAt: "2026-07-03T10:00:00.000Z",
    updatedAt: "2026-07-03T10:00:00.000Z",
    eventCount: 1,
    lastGates: [],
    challenges: [],
  },
];

// ── Blackboard (blackboard.list) ────────────────────────────────────────────
export const blackboardEntries: BlackboardEntry[] = [
  {
    id: "fx-bb-1",
    workspaceId: WS_ID,
    key: "design:supervisor-picker",
    value: { proto: ".arta/proto/screens/supervisor-picker.tsx", sha: "d2ac161" },
    lastWriterId: AG_ARTA,
    updatedAt: "2026-07-05T08:10:00.000Z",
  },
  {
    id: "fx-bb-2",
    workspaceId: WS_ID,
    key: "convention:commit-scope",
    value: "Commit only via `conclave stage commit` in the shared checkout.",
    lastWriterId: AG_DETORO,
    updatedAt: "2026-07-05T09:00:00.000Z",
  },
  {
    id: "fx-bb-3",
    workspaceId: WS_ID,
    key: "anomaly:vite-1420-strictport",
    value: "Dev server on 1420 is strictPort; uishot reuses a running one.",
    lastWriterId: AG_DEW,
    updatedAt: "2026-07-05T10:45:00.000Z",
  },
  {
    id: "fx-bb-4",
    workspaceId: WS_ID,
    key: "config:distill-auto",
    value: { distiller: AG_MELLOW, reviewer: AG_DETORO, cooldownHours: 12 },
    lastWriterId: AG_DETORO,
    updatedAt: "2026-07-04T18:00:00.000Z",
  },
];

export const blackboardActivity: BlackboardActivity[] = [
  {
    id: "fx-bba-1",
    entryId: "fx-bb-1",
    instanceId: AG_TIESTO,
    action: "read",
    at: "2026-07-05T11:50:00.000Z",
  },
  {
    id: "fx-bba-2",
    entryId: "fx-bb-2",
    instanceId: AG_DEW,
    action: "read",
    at: "2026-07-05T10:40:00.000Z",
  },
  {
    id: "fx-bba-3",
    entryId: "fx-bb-1",
    instanceId: AG_ARTA,
    action: "write",
    at: "2026-07-05T08:10:00.000Z",
  },
];

// ── Artifacts (artifact.list) ───────────────────────────────────────────────
export const artifacts: Artifact[] = [
  {
    id: "fx-art-1",
    workspaceId: WS_ID,
    agentId: AG_DETORO,
    title: "uishot Real-Pixel Feedback Loop — plan",
    kind: "markdown",
    content:
      "# uishot Plan\n\nThree lanes (F, U, P) with disjoint file boundaries render the real app headlessly with fixture data.",
    createdAt: "2026-07-05T09:14:00.000Z",
  },
  {
    id: "fx-art-2",
    workspaceId: WS_ID,
    agentId: AG_ARTA,
    title: "Supervisor picker — remove affordance",
    kind: "react",
    content:
      "export function SupervisorPicker() {\n  return <div>edit-variant Remove supervisor</div>;\n}",
    createdAt: "2026-07-05T08:05:00.000Z",
  },
  {
    id: "fx-art-3",
    workspaceId: WS_ID,
    agentId: AG_MELLOW,
    title: "Review notes — design-shell-remember",
    kind: "text",
    content:
      "Functional LAND: hash → localStorage → default precedence verified across a live reload.",
    createdAt: "2026-07-05T11:02:00.000Z",
  },
];

// ── Memory graph (memory.graph) ─────────────────────────────────────────────
export const memoryGraph: { nodes: MemoryGraphNode[]; edges: MemoryGraphEdge[] } = {
  nodes: [
    {
      id: "fx-mem-1",
      text: "Conclave core uses sqlx(sqlite) + chain-builder, not rusqlite.",
      sourceKind: "agent",
      sourceId: AG_DETORO,
      createdAt: "2026-07-02T10:00:00.000Z",
      updatedAt: "2026-07-02T10:00:00.000Z",
    },
    {
      id: "fx-mem-2",
      text: "App UI copy is English; replies to the user stay Thai.",
      sourceKind: "agent",
      sourceId: AG_DETORO,
      createdAt: "2026-07-02T10:05:00.000Z",
      updatedAt: "2026-07-02T10:05:00.000Z",
    },
    {
      id: "fx-mem-3",
      text: "Shared-tree commit needs a pathspec, else it sweeps the whole index.",
      sourceKind: "agent",
      sourceId: AG_TIESTO,
      createdAt: "2026-07-03T14:00:00.000Z",
      updatedAt: "2026-07-03T14:00:00.000Z",
    },
    {
      id: "fx-mem-4",
      text: "Conclave task state transitions are single-step (claimed → in_progress → review).",
      sourceKind: "agent",
      sourceId: AG_TIESTO,
      createdAt: "2026-07-03T14:10:00.000Z",
      updatedAt: "2026-07-03T14:10:00.000Z",
    },
    {
      id: "fx-mem-5",
      text: "Gate results must be recorded via `task gate`, not narrated as prose.",
      sourceKind: "agent",
      sourceId: AG_MELLOW,
      createdAt: "2026-07-04T09:00:00.000Z",
      updatedAt: "2026-07-04T09:00:00.000Z",
    },
    {
      id: "fx-mem-6",
      text: "StdinBar composer width budget: RoutingPicker + attach + send eat 230-320px.",
      sourceKind: "agent",
      sourceId: AG_ARTA,
      createdAt: "2026-07-04T11:00:00.000Z",
      updatedAt: "2026-07-04T11:00:00.000Z",
    },
    {
      id: "fx-mem-7",
      text: "Human prefers a layout change over shrinking text when space is tight.",
      sourceKind: "manual",
      sourceId: null,
      createdAt: "2026-07-04T11:30:00.000Z",
      updatedAt: "2026-07-04T11:30:00.000Z",
    },
    {
      id: "fx-mem-8",
      text: "Boundary the defining file, not the importer, when cutting a lane.",
      sourceKind: "agent",
      sourceId: AG_DETORO,
      createdAt: "2026-07-04T16:00:00.000Z",
      updatedAt: "2026-07-04T16:00:00.000Z",
    },
  ],
  edges: [
    { a: "fx-mem-3", b: "fx-mem-4", rel: "related", score: 0.82 },
    { a: "fx-mem-3", b: "fx-mem-5", rel: "related", score: 0.71 },
    { a: "fx-mem-4", b: "fx-mem-5", rel: "related", score: 0.68 },
    { a: "fx-mem-6", b: "fx-mem-7", rel: "wiki" },
    { a: "fx-mem-1", b: "fx-mem-8", rel: "related", score: 0.6 },
    { a: "fx-mem-2", b: "fx-mem-7", rel: "related", score: 0.55 },
    { a: "fx-mem-5", b: "fx-mem-8", rel: "related", score: 0.64 },
    { a: "fx-mem-1", b: "fx-mem-3", rel: "related", score: 0.58 },
    { a: "fx-mem-6", b: "fx-mem-2", rel: "related", score: 0.52 },
    { a: "fx-mem-4", b: "fx-mem-8", rel: "wiki" },
  ],
};

// ── Inter-agent messages (message.listForWorkspace) — deliberately NOT ──────
// chronological: entry order here (and the id-sort shuffle in default.ts's
// message.listForWorkspace handler) is a standing regression tripwire, so
// consumers MUST order by parsed createdAt. Never reorder these entries.
export const messages: InterAgentMessage[] = [
  {
    id: "fx-msg-10",
    fromInstanceId: AG_TIESTO,
    toInstanceId: AG_DETORO,
    text: "F1 landed at a9f4b01 — fixture seam behind ipc call(). Build green.",
    status: "delivered",
    createdAt: "2026-07-05T11:58:00.000Z",
  },
  {
    id: "fx-msg-9",
    fromInstanceId: AG_DETORO,
    toInstanceId: AG_TIESTO,
    text: "New lane for you: uishot-fixture-mode (Lane F). Claim + read the plan.",
    status: "delivered",
    createdAt: "2026-07-05T09:20:00.000Z",
  },
  {
    id: "fx-msg-8",
    fromInstanceId: AG_DEW,
    toInstanceId: AG_DETORO,
    text: "Challenge filed on uishot-cli: sentinel should be a data-attr selector.",
    status: "delivered",
    createdAt: "2026-07-05T11:25:00.000Z",
  },
  {
    id: "fx-msg-7",
    fromInstanceId: AG_MELLOW,
    toInstanceId: AG_DETORO,
    text: "design-shell-remember: functional LAND, moved to review.",
    status: "delivered",
    createdAt: "2026-07-05T11:05:00.000Z",
  },
  {
    id: "fx-msg-6",
    fromInstanceId: AG_ARTA,
    toInstanceId: AG_MELLOW,
    text: "Proto pinned @d2ac161 — rose maps to the danger token, not a raw hex.",
    status: "delivered",
    createdAt: "2026-07-05T08:12:00.000Z",
  },
  {
    id: "fx-msg-5",
    fromInstanceId: AG_DETORO,
    toInstanceId: AG_DEW,
    text: "Consume the URL + sentinel contract exactly as written; challenge first if you must vary it.",
    status: "delivered",
    createdAt: "2026-07-05T09:22:00.000Z",
  },
  {
    id: "fx-msg-4",
    fromInstanceId: AG_TIESTO,
    toInstanceId: AG_DETORO,
    text: "Reading order done — claiming uishot-fixture-mode now.",
    status: "delivered",
    createdAt: "2026-07-05T09:35:00.000Z",
  },
  {
    id: "fx-msg-3",
    fromInstanceId: AG_DETORO,
    toInstanceId: AG_ARTA,
    text: "Need the supervisor-picker proto pinned before Tiesto builds the remove affordance.",
    status: "delivered",
    createdAt: "2026-07-04T13:40:00.000Z",
  },
];

// ── Roles (role.list) ───────────────────────────────────────────────────────
export const roles: Role[] = [
  {
    id: "lead",
    name: "Lead",
    description: "Owns the plan, settles decisions, rules escalations, integrates lanes.",
    skillIds: ["leadership", "collaboration"],
    kind: "builtin",
  },
  {
    id: "reviewer",
    name: "Reviewer",
    description: "Grills deliverables against the plan and the spec before they land.",
    skillIds: ["collaboration"],
    kind: "builtin",
  },
  {
    id: "implementer",
    name: "Implementer",
    description: "Turns a recorded plan into working, verified software.",
    skillIds: ["implementer", "collaboration"],
    kind: "builtin",
  },
  {
    id: "designer",
    name: "Designer",
    description: "Owns the design canon and the design-acceptance gate.",
    skillIds: ["collaboration"],
    kind: "builtin",
  },
];

// ── Skills (skill.list) ─────────────────────────────────────────────────────
export const skills: Skill[] = [
  {
    id: "collaboration",
    name: "Collaboration",
    description: "Multi-agent etiquette: claiming, replying, escalation.",
    content: "# Collaboration\n\nYou share this workspace with other agents and one human.",
    kind: "builtin",
    mandatory: true,
    attachedTo: 5,
  },
  {
    id: "implementer",
    name: "Implementer",
    description: "Turn a lead's plan into verified software.",
    content: "# Implementer\n\nClaim before touching anything; verify before you claim done.",
    kind: "builtin",
    mandatory: false,
    attachedTo: 2,
  },
  {
    id: "leadership",
    name: "Leadership",
    description: "Decide early, write it down, delegate and stay out.",
    content: "# Leadership\n\nMake decisions cheap for everyone else.",
    kind: "builtin",
    mandatory: false,
    attachedTo: 1,
  },
  {
    id: "fx-skill-custom-1",
    name: "Arta canvas",
    description: "Drive the shared live design canvas.",
    content: "# Arta\n\nBuild a shared, live design canvas the dev watches in the viewer.",
    kind: "custom",
    mandatory: true,
    attachedTo: 1,
  },
];

// ── Providers (provider.list) ───────────────────────────────────────────────
export const providers: Provider[] = [
  {
    id: "fx-prov-anthropic",
    name: "anthropic",
    maskedKey: "sk-ant-…4f2a",
    storedIn: "keychain",
    status: "connected",
  },
  {
    id: "fx-prov-openai",
    name: "openai",
    maskedKey: "sk-…9c1b",
    storedIn: "keychain",
    status: "connected",
  },
];
