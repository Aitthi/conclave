// Mock data for the Lane Board + workspace telemetry strip (ADR 0008 · Lane D).
// Shapes track the FROZEN wire contract in docs/2026-07-04-plan-agent-work-system.md
// §Frozen exactly, so the implementer (Tiësto) can swap this file for a real
// `task.list` call without reshaping a single field:
//
//   task  = {id, workspaceId, slug, title, state, ownerAgentId, implementerAgentId,
//            fileBoundary[], designCanon, plan, createdAt, updatedAt}
//   event = task_event(kind: 'note'|'state'|'gate'|'challenge'|'ruling', payload, …)
//     gate payload      = {cmd, exit, sha, tail, cwd}
//     challenge payload = {claim, evidence, proposal, default, deadlineMin?}
//     ruling payload    = {challengeId, text, by}
//
// The data is the real Agent-Work-System program, dogfooded onto its own board.

export type TaskState =
  | "planned"
  | "claimed"
  | "in_progress"
  | "review"
  | "merged"
  | "abandoned";

export interface GateBadge {
  /** command that ran (shown on hover / detail) */
  cmd: string;
  /** process exit code — 0 = green, non-zero = red. Evidence, recorded at run time. */
  exit: number;
  /** `git rev-parse HEAD` captured when the gate ran — the SHA the evidence is true at */
  sha: string;
  label: string; // short human name for the gate (e.g. "cargo test")
}

export interface ChallengeBadge {
  /** open = awaiting a ruling; ruled = a ruling event resolved it */
  status: "open" | "ruled";
  claim: string;
  /** minutes until the stated default fires (open only); absent = advisory */
  deadlineMin?: number;
}

export interface Task {
  id: string;
  slug: string;
  title: string;
  state: TaskState;
  ownerAgentId: string; // who leads / escalation target
  implementerAgentId: string | null; // who claimed it (null until claimed)
  fileBoundary: string[]; // JSON pathspecs the lane may touch
  designCanon: string | null; // bb key / proto path, or null
  gates: GateBadge[]; // derived from task_event(kind='gate')
  challenges: ChallengeBadge[]; // derived from kind='challenge' + 'ruling'
  updatedAt: string; // relative hint for the card foot
}

// Column order + presentation. `abandoned` is off the main flow — surfaced only
// through the filter, never as a sixth always-present column.
export const COLUMNS: { state: TaskState; label: string; accent: string }[] = [
  { state: "planned", label: "Planned", accent: "var(--color-faint)" },
  { state: "claimed", label: "Claimed", accent: "var(--color-accent)" },
  { state: "in_progress", label: "In progress", accent: "var(--color-working)" },
  { state: "review", label: "Review", accent: "var(--color-a-violet)" },
  { state: "merged", label: "Merged", accent: "var(--color-live)" },
];

export const tasks: Task[] = [
  // ── planned — backlog, owner set, no implementer yet ──────────────────────
  {
    id: "t-aws-b",
    slug: "aws-b",
    title: "Watch / notify injection + stall & challenge-default timers",
    state: "planned",
    ownerAgentId: "detoro",
    implementerAgentId: null,
    fileBoundary: ["engine/runtime/task_timer.rs", "engine/commands/task.rs"],
    designCanon: null,
    gates: [],
    challenges: [],
    updatedAt: "8m",
  },
  {
    id: "t-r5",
    slug: "r5-rebuild",
    title: "Rebuild 0.2.0 (r5) off main — program + memory-graph, then install",
    state: "planned",
    ownerAgentId: "detoro",
    implementerAgentId: null,
    fileBoundary: ["src-tauri/**"],
    designCanon: null,
    gates: [],
    challenges: [],
    updatedAt: "20m",
  },

  // ── claimed — implementer set, canon attached, no gate run yet ────────────
  {
    id: "t-aws-d",
    slug: "aws-d",
    title: "Lane board UI + workspace telemetry strip",
    state: "claimed",
    ownerAgentId: "detoro",
    implementerAgentId: "tiesto",
    fileBoundary: ["src/components/LaneBoard.tsx", "src/ipc/events.ts"],
    designCanon: "design:aws-laneboard",
    gates: [],
    challenges: [],
    updatedAt: "3m",
  },

  // ── in_progress — live work, gate ledger + a live challenge ───────────────
  {
    id: "t-aws-a",
    slug: "aws-a",
    title: "Task core — tables, repo, task.* commands, CLI verbs, gate runner",
    state: "in_progress",
    ownerAgentId: "detoro",
    implementerAgentId: "dew",
    fileBoundary: ["engine/migrations/0012_task_system.sql", "engine/repo/task.rs"],
    designCanon: null,
    gates: [
      { cmd: "cargo test --lib", exit: 0, sha: "4f2a1c9", label: "cargo test" },
      { cmd: "cargo clippy --all-targets -- -D warnings", exit: 0, sha: "4f2a1c9", label: "clippy" },
    ],
    challenges: [
      {
        status: "open",
        claim: "State CHECK omits a 'blocked' state — sqlite can't ALTER it in later",
        deadlineMin: 42,
      },
    ],
    updatedAt: "just now",
  },
  {
    id: "t-aws-c",
    slug: "aws-c",
    title: "Lane manager — conclave lane start/finish + pre-commit scope guard",
    state: "in_progress",
    ownerAgentId: "detoro",
    implementerAgentId: "guetta",
    fileBoundary: ["src-tauri/src/bin/conclave-cli.rs", ".git/hooks/pre-commit"],
    designCanon: null,
    gates: [
      { cmd: "cargo clippy --all-targets -- -D warnings", exit: 101, sha: "9b2e104", label: "clippy" },
    ],
    challenges: [],
    updatedAt: "6m",
  },

  // ── review — gates green, a challenge that got ruled ──────────────────────
  {
    id: "t-mem-graph",
    slug: "memory-graph",
    title: "Memory knowledge-graph view — Obsidian-style force graph",
    state: "review",
    ownerAgentId: "detoro",
    implementerAgentId: "tiesto",
    fileBoundary: ["src/components/MemoryGraph.tsx", "engine/commands/memory.rs"],
    designCanon: "73ac6fa",
    gates: [
      { cmd: "cargo test --lib", exit: 0, sha: "dcc3627", label: "cargo test" },
      { cmd: "npx tsc --noEmit", exit: 0, sha: "dcc3627", label: "tsc" },
    ],
    challenges: [{ status: "ruled", claim: "ControlPanel group keyed by display name" }],
    updatedAt: "1h",
  },

  // ── merged — shipped, evidence pinned to the merge SHA ────────────────────
  {
    id: "t-rail-chats",
    slug: "right-rail-chats",
    title: "Right-rail live agent chat viewer (Slack-style)",
    state: "merged",
    ownerAgentId: "detoro",
    implementerAgentId: "dew",
    fileBoundary: ["src/components/ChatRail.tsx"],
    designCanon: "design:right-rail-chat",
    gates: [{ cmd: "npx tsc --noEmit", exit: 0, sha: "6cdb83f", label: "tsc" }],
    challenges: [],
    updatedAt: "5h",
  },
  {
    id: "t-role-picker",
    slug: "role-picker",
    title: "Role picker — lift role selection into the agent header",
    state: "merged",
    ownerAgentId: "detoro",
    implementerAgentId: "tiesto",
    fileBoundary: ["src/components/RoutingPicker.tsx"],
    designCanon: "design:role-picker",
    gates: [{ cmd: "npx tsc --noEmit", exit: 0, sha: "c848033", label: "tsc" }],
    challenges: [],
    updatedAt: "8h",
  },
  {
    id: "t-mem-skill",
    slug: "memory-skill",
    title: "Semantic memory store + conclave memory verbs",
    state: "merged",
    ownerAgentId: "detoro",
    implementerAgentId: "detoro",
    fileBoundary: ["engine/repo/memory.rs"],
    designCanon: null,
    gates: [{ cmd: "cargo test --lib", exit: 0, sha: "0313a31", label: "cargo test" }],
    challenges: [],
    updatedAt: "1d",
  },
];

// ── Workspace telemetry — per-agent context meters aggregated across the
//    workspace. Feeds off the SAME `session:context` events ContextBars already
//    consumes per session (plan: reuse that data path, don't invent a new one).
//    `working` mirrors the Roster working-state (PTY output within ~5s).
export interface AgentTelemetry {
  agentId: string;
  /** live token count for this agent's session */
  tokens: number;
  /** the model's context window */
  limit: number;
  /** honest flag — the count is an estimate between exact syncs */
  estimated: boolean;
  working: boolean;
  /** no live session → meter is dashed/empty, not a lie about 0% */
  live: boolean;
}

export const telemetry: AgentTelemetry[] = [
  { agentId: "detoro", tokens: 82_000, limit: 200_000, estimated: true, working: true, live: true },
  { agentId: "mellow", tokens: 23_000, limit: 200_000, estimated: false, working: false, live: true },
  { agentId: "dew", tokens: 156_000, limit: 200_000, estimated: true, working: true, live: true },
  { agentId: "guetta", tokens: 124_000, limit: 200_000, estimated: true, working: true, live: true },
  { agentId: "tiesto", tokens: 178_000, limit: 200_000, estimated: true, working: false, live: true },
  { agentId: "arta", tokens: 61_000, limit: 200_000, estimated: false, working: false, live: true },
];

// Context pressure → meter colour. Honest signal: amber warns, rose = compact
// imminent (the agent is about to lose its context). Mirrors nothing in the app
// yet — the per-session ContextBars meter is always accent; aggregating many
// agents is exactly where a graded scale earns its place.
export function meterColor(pct: number): string {
  if (pct >= 88) return "var(--color-rose)";
  if (pct >= 68) return "var(--color-working)";
  return "var(--color-accent)";
}
