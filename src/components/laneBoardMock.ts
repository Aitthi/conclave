// ── Lane Board development mock (ADR 0008, Lane D) ──────────────────────────
// Returns the EXACT frozen wire shape (plan §Frozen > Response shapes) so the
// LaneBoard renders real-looking data before Lane A's `task.*` handlers merge.
// At integration this whole module is deleted and the component swaps to
// `ipc.task.list` / `ipc.task.get`. Nothing here touches the backend.
//
// Contract parity with the lead's frozen rulings (@ 5e3e27e + the badge ruling):
//   • task.list → TaskListRow[]: frozen Task + `eventCount` + `lastGate?`
//     (newest gate only, omitted when none) + `challenges` (always [], deadlineAt
//     ISO) — optional task fields are OMITTED when absent, never null.
//   • task.get  → { task, events } with events sorted `createdAt` DESC, each
//     payload a parsed JSON OBJECT.
// Data mirrors the canon's dogfood set (.arta/proto/lib/laneBoard.ts) so the dev
// render tracks .arta/snapshots/lane-board.png.
import type { Task, TaskListRow, TaskEvent } from "../ipc";

// Fixed timestamps keep the fixture deterministic; `updatedAt`/`createdAt` are
// ISO-8601 UTC (the component runs them through `timeHint`). Only the live
// challenge deadline is relative — a fixed past ISO would render as expired.
const T = (h: number, m = 0): string =>
  `2026-07-04T${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:00Z`;
const inMinutes = (n: number): string => new Date(Date.now() + n * 60_000).toISOString();

const WS = "mock-workspace";

const MOCK_TASKS: TaskListRow[] = [
  // ── planned — owner set, no implementer, no gates ─────────────────────────
  {
    id: "t-aws-b",
    workspaceId: WS,
    slug: "aws-b",
    title: "Watch / notify injection + stall & challenge-default timers",
    state: "planned",
    ownerAgentId: "detoro",
    fileBoundary: ["engine/runtime/task_timer.rs", "engine/commands/task.rs"],
    plan: "After A merges: inject watcher lines, 5-min stall/deadline timers.",
    createdAt: T(13, 56),
    updatedAt: T(13, 56),
    eventCount: 1,
    challenges: [],
  },
  {
    id: "t-r5",
    workspaceId: WS,
    slug: "r5-rebuild",
    title: "Rebuild 0.2.0 (r5) off main — program + memory-graph, then install",
    state: "planned",
    ownerAgentId: "detoro",
    fileBoundary: ["src-tauri/**"],
    plan: "r5 rebuild off main including this program + memory-graph; supersedes held r4.",
    createdAt: T(13, 40),
    updatedAt: T(13, 40),
    eventCount: 0,
    challenges: [],
  },

  // ── claimed — implementer + canon, no gate run yet ────────────────────────
  {
    id: "t-aws-d",
    workspaceId: WS,
    slug: "aws-d",
    title: "Lane board UI + workspace telemetry strip",
    state: "claimed",
    ownerAgentId: "detoro",
    implementerAgentId: "tiesto",
    fileBoundary: ["src/components/LaneBoard.tsx", "src/ipc/events.ts"],
    designCanon: "design:aws-laneboard",
    plan: "Frozen-wire ipc + telemetry strip + Lane Board per Arta canon @ fa4929b.",
    createdAt: T(13, 54),
    updatedAt: T(13, 57),
    eventCount: 4,
    challenges: [],
  },

  // ── in_progress — live gate ledger + a live challenge ─────────────────────
  {
    id: "t-aws-a",
    workspaceId: WS,
    slug: "aws-a",
    title: "Task core — tables, repo, task.* commands, CLI verbs, gate runner",
    state: "in_progress",
    ownerAgentId: "detoro",
    implementerAgentId: "dew",
    fileBoundary: ["engine/migrations/0012_task_system.sql", "engine/repo/task.rs"],
    plan: "Migration 0012 + repo chain-builder + router verbs + gate runner + memory nudge.",
    createdAt: T(13, 53),
    updatedAt: T(14, 0),
    eventCount: 9,
    lastGate: { cmd: "cargo test --lib", exit: 0, sha: "4f2a1c9", createdAt: T(13, 59) },
    challenges: [
      {
        id: "ch-aws-a-1",
        status: "open",
        claim: "State CHECK omits a 'blocked' state — sqlite can't ALTER it in later",
        deadlineAt: inMinutes(42),
      },
    ],
  },
  {
    id: "t-aws-c",
    workspaceId: WS,
    slug: "aws-c",
    title: "Lane manager — conclave lane start/finish + pre-commit scope guard",
    state: "in_progress",
    ownerAgentId: "detoro",
    implementerAgentId: "guetta",
    fileBoundary: ["src-tauri/src/bin/conclave-cli.rs", ".git/hooks/pre-commit"],
    plan: "lane start/finish worktree lifecycle + POSIX pre-commit guard.",
    createdAt: T(13, 55),
    updatedAt: T(13, 58),
    eventCount: 3,
    lastGate: { cmd: "cargo clippy --all-targets -- -D warnings", exit: 101, sha: "9b2e104", createdAt: T(13, 58) },
    challenges: [],
  },

  // ── review — gates green, a challenge that got ruled ──────────────────────
  {
    id: "t-mem-graph",
    workspaceId: WS,
    slug: "memory-graph",
    title: "Memory knowledge-graph view — Obsidian-style force graph",
    state: "review",
    ownerAgentId: "detoro",
    implementerAgentId: "tiesto",
    fileBoundary: ["src/components/MemoryGraph.tsx", "engine/commands/memory.rs"],
    designCanon: "73ac6fa",
    plan: "Force-directed graph over the semantic memory store; ADR 0007.",
    createdAt: T(11, 0),
    updatedAt: T(13, 0),
    eventCount: 11,
    lastGate: { cmd: "npx tsc --noEmit", exit: 0, sha: "dcc3627", createdAt: T(12, 50) },
    challenges: [
      { id: "ch-mem-1", status: "ruled", claim: "ControlPanel group keyed by display name" },
    ],
  },

  // ── merged — shipped, evidence pinned to the merge SHA ────────────────────
  {
    id: "t-rail-chats",
    workspaceId: WS,
    slug: "right-rail-chats",
    title: "Right-rail live agent chat viewer (Slack-style)",
    state: "merged",
    ownerAgentId: "detoro",
    implementerAgentId: "dew",
    fileBoundary: ["src/components/ChatRail.tsx"],
    designCanon: "design:right-rail-chat",
    plan: "Live per-agent chat viewer in the right rail.",
    createdAt: T(9, 0),
    updatedAt: T(10, 30),
    eventCount: 7,
    lastGate: { cmd: "npx tsc --noEmit", exit: 0, sha: "6cdb83f", createdAt: T(10, 25) },
    challenges: [],
  },
  {
    id: "t-role-picker",
    workspaceId: WS,
    slug: "role-picker",
    title: "Role picker — lift role selection into the agent header",
    state: "merged",
    ownerAgentId: "detoro",
    implementerAgentId: "tiesto",
    fileBoundary: ["src/components/RoutingPicker.tsx"],
    designCanon: "design:role-picker",
    plan: "Move role selection into the agent header.",
    createdAt: T(6, 0),
    updatedAt: T(8, 0),
    eventCount: 6,
    lastGate: { cmd: "npx tsc --noEmit", exit: 0, sha: "c848033", createdAt: T(7, 55) },
    challenges: [],
  },
  {
    id: "t-mem-skill",
    workspaceId: WS,
    slug: "memory-skill",
    title: "Semantic memory store + conclave memory verbs",
    state: "merged",
    ownerAgentId: "detoro",
    implementerAgentId: "detoro",
    fileBoundary: ["engine/repo/memory.rs"],
    plan: "Embed + persist durable memories; conclave memory remember/search.",
    createdAt: T(2, 0),
    updatedAt: T(4, 0),
    eventCount: 5,
    lastGate: { cmd: "cargo test --lib", exit: 0, sha: "0313a31", createdAt: T(3, 55) },
    challenges: [],
  },
];

// A couple of tasks carry event detail for the `task.get` mock; the board itself
// derives its badges from the list, so this exists only for ipc-surface parity.
const MOCK_EVENTS: Record<string, TaskEvent[]> = {
  "aws-a": [
    { id: "ev-a-3", taskId: "t-aws-a", kind: "gate", actorAgentId: "dew", payload: { cmd: "cargo test --lib", exit: 0, sha: "4f2a1c9", tail: "test result: ok. 475 passed; 0 failed", cwd: ".claude/worktrees/aws-a" }, createdAt: T(13, 59) },
    { id: "ev-a-2", taskId: "t-aws-a", kind: "challenge", actorAgentId: "dew", payload: { claim: "State CHECK omits a 'blocked' state", evidence: "migration CHECK", proposal: "add blocked", default: "ship without", deadlineMin: 42 }, createdAt: T(13, 57) },
    { id: "ev-a-1", taskId: "t-aws-a", kind: "state", actorAgentId: "dew", payload: { from: "planned", to: "claimed" }, createdAt: T(13, 53) },
  ],
  "aws-d": [
    { id: "ev-d-2", taskId: "t-aws-d", kind: "ruling", actorAgentId: "detoro", payload: { challengeId: "ev-d-1", text: "Response shapes frozen verbatim.", by: "detoro" }, createdAt: T(13, 57) },
    { id: "ev-d-1", taskId: "t-aws-d", kind: "state", actorAgentId: "tiesto", payload: { from: "planned", to: "claimed" }, createdAt: T(13, 54) },
  ],
};

/** Mock of `ipc.task.list` — a fresh copy so callers can sort/filter without
 *  mutating the fixture. Honors the optional `state` filter. */
export function mockTaskList(state?: Task["state"]): TaskListRow[] {
  const rows = MOCK_TASKS.map((t) => ({ ...t }));
  return state ? rows.filter((t) => t.state === state) : rows;
}

/** Mock of `ipc.task.get` — task + events sorted `createdAt` DESC (newest
 *  first). Throws NotFound-style on an unknown slug, mirroring the backend. */
export function mockTaskGet(slug: string): { task: Task; events: TaskEvent[] } {
  const row = MOCK_TASKS.find((t) => t.slug === slug);
  if (!row) throw new Error(`task '${slug}' not found`);
  const events = (MOCK_EVENTS[slug] ?? [])
    .map((e) => ({ ...e }))
    .sort((a, b) => (a.createdAt < b.createdAt ? 1 : a.createdAt > b.createdAt ? -1 : 0));
  // Strip the list-only derived fields to yield the bare frozen Task shape.
  const { eventCount: _c, lastGate: _g, challenges: _ch, ...task } = row;
  return { task, events };
}
