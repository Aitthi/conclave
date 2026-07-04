// ── Lane Board development mock (ADR 0008, Lane D) ──────────────────────────
// Returns the EXACT frozen wire shape (plan §Frozen > Response shapes) so the
// LaneBoard renders real-looking data before Lane A's `task.*` handlers merge.
// At integration this whole module is deleted and the component swaps to
// `ipc.task.list` / `ipc.task.get`. Nothing here touches the backend.
//
// Contract parity with the lead's frozen ruling @ 5e3e27e:
//   • `task.list` → TaskListRow[] (frozen Task + `eventCount`)
//   • `task.get`  → { task, events } with events sorted `createdAt` DESC
//   • every TaskEvent.payload is a parsed JSON OBJECT, never a JSON string
import type { Task, TaskListRow, TaskEvent } from "../ipc";

// Fixed timestamps — no `Date.now()` so the fixture is deterministic across
// renders (and matches the workspace's real ISO-8601 UTC string format).
const T = (h: number, m = 0): string =>
  `2026-07-04T${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:00Z`;

const WS = "mock-workspace";

const MOCK_TASKS: TaskListRow[] = [
  {
    id: "task-aws-a",
    workspaceId: WS,
    slug: "aws-a",
    title: "task core — tables + repo + task.* commands + CLI + gate runner",
    state: "merged",
    ownerAgentId: "lead",
    implementerAgentId: "dew",
    fileBoundary: ["src-tauri/src/engine/repo/task.rs", "src-tauri/src/engine/commands/task.rs"],
    designCanon: null,
    plan: "Migration 0012 + repo chain-builder + router verbs + gate runner + memory nudge.",
    createdAt: T(13, 53),
    updatedAt: T(15, 10),
    eventCount: 12,
  },
  {
    id: "task-aws-d",
    workspaceId: WS,
    slug: "aws-d",
    title: "LaneBoard UI + telemetry strip + task.list/get ipc + task:changed",
    state: "in_progress",
    ownerAgentId: "lead",
    implementerAgentId: "tiesto",
    fileBoundary: ["src/components/LaneBoard.tsx", "src/ipc/"],
    designCanon: ".arta/proto/screens/lane-board.tsx @ <pending>",
    plan: "Frozen-wire ipc + telemetry strip aggregating session:context + Lane Board per Arta canon.",
    createdAt: T(13, 54),
    updatedAt: T(14, 5),
    eventCount: 4,
  },
  {
    id: "task-aws-c",
    workspaceId: WS,
    slug: "aws-c",
    title: "lane manager (start/finish) + pre-commit scope guard",
    state: "claimed",
    ownerAgentId: "lead",
    implementerAgentId: "guetta",
    fileBoundary: ["src-tauri/src/bin/conclave-cli.rs"],
    designCanon: null,
    plan: "lane start/finish worktree lifecycle + POSIX pre-commit guard blocking foreign paths.",
    createdAt: T(13, 55),
    updatedAt: T(13, 55),
    eventCount: 1,
  },
  {
    id: "task-aws-b",
    workspaceId: WS,
    slug: "aws-b",
    title: "watch/notify injection + stall timer + challenge-default timer",
    state: "planned",
    ownerAgentId: "lead",
    implementerAgentId: null,
    fileBoundary: ["src-tauri/src/engine/runtime/task_timer.rs"],
    designCanon: null,
    plan: "After A merges: inject watcher lines, 5-min stall/deadline timers.",
    createdAt: T(13, 56),
    updatedAt: T(13, 56),
    eventCount: 0,
  },
];

const MOCK_EVENTS: Record<string, TaskEvent[]> = {
  "aws-a": [
    {
      id: "ev-a-4",
      taskId: "task-aws-a",
      kind: "state",
      actorAgentId: "lead",
      payload: { from: "review", to: "merged" },
      createdAt: T(15, 10),
    },
    {
      id: "ev-a-3",
      taskId: "task-aws-a",
      kind: "gate",
      actorAgentId: "dew",
      payload: {
        cmd: "cargo test --lib",
        exit: 0,
        sha: "abc1234",
        tail: "test result: ok. 475 passed; 0 failed; 9 ignored",
        cwd: ".claude/worktrees/aws-a",
      },
      createdAt: T(14, 58),
    },
    {
      id: "ev-a-2",
      taskId: "task-aws-a",
      kind: "note",
      actorAgentId: "dew",
      payload: { text: "Repo CRUD + state-machine transitions green." },
      createdAt: T(14, 20),
    },
    {
      id: "ev-a-1",
      taskId: "task-aws-a",
      kind: "state",
      actorAgentId: "dew",
      payload: { from: "planned", to: "claimed" },
      createdAt: T(13, 53),
    },
  ],
  "aws-d": [
    {
      id: "ev-d-4",
      taskId: "task-aws-d",
      kind: "note",
      actorAgentId: "tiesto",
      payload: { text: "ipc scaffolding done; mock built; waiting on design canon." },
      createdAt: T(14, 5),
    },
    {
      id: "ev-d-3",
      taskId: "task-aws-d",
      kind: "ruling",
      actorAgentId: "lead",
      payload: { challengeId: "ev-d-2", text: "Response shapes frozen verbatim.", by: "lead" },
      createdAt: T(14, 2),
    },
    {
      id: "ev-d-2",
      taskId: "task-aws-d",
      kind: "challenge",
      actorAgentId: "tiesto",
      payload: {
        claim: "task.list/get response shapes are underspecified across the A↔D wire.",
        evidence: "plan §Frozen enumerates task fields but not eventCount / get envelope.",
        proposal: "TaskListRow=Task+eventCount; task.get={task,events}.",
        default: "proceed on proposed shapes; redo if overruled.",
      },
      createdAt: T(14, 0),
    },
    {
      id: "ev-d-1",
      taskId: "task-aws-d",
      kind: "state",
      actorAgentId: "tiesto",
      payload: { from: "planned", to: "in_progress" },
      createdAt: T(13, 54),
    },
  ],
  "aws-c": [
    {
      id: "ev-c-1",
      taskId: "task-aws-c",
      kind: "state",
      actorAgentId: "guetta",
      payload: { from: "planned", to: "claimed" },
      createdAt: T(13, 55),
    },
  ],
  "aws-b": [],
};

/** Mock of `ipc.task.list` — returns a fresh copy so callers can sort/filter
 *  without mutating the fixture. Honors the optional `state` filter. */
export function mockTaskList(state?: Task["state"]): TaskListRow[] {
  const rows = MOCK_TASKS.map((t) => ({ ...t }));
  return state ? rows.filter((t) => t.state === state) : rows;
}

/** Mock of `ipc.task.get` — task + its events sorted `createdAt` DESC (newest
 *  first), matching the lead's ruling. Throws NotFound-style on an unknown slug
 *  to mirror the backend's error surface. */
export function mockTaskGet(slug: string): { task: Task; events: TaskEvent[] } {
  const task = MOCK_TASKS.find((t) => t.slug === slug);
  if (!task) throw new Error(`task '${slug}' not found`);
  const events = (MOCK_EVENTS[slug] ?? [])
    .map((e) => ({ ...e }))
    .sort((a, b) => (a.createdAt < b.createdAt ? 1 : a.createdAt > b.createdAt ? -1 : 0));
  const { eventCount: _eventCount, ...bare } = task;
  return { task: bare, events };
}
