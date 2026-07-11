import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  Search,
  GitCommitHorizontal,
  Check,
  X,
  Swords,
  Gavel,
  Palette,
  FileCode2,
  ArrowRight,
  ArrowUp,
  Clock,
  Crown,
  Filter,
  LoaderCircle,
  Columns3,
  Network,
  Scale,
  ShieldQuestion,
  Plus,
} from "lucide-react";
import { ipc, useTaskChanged, useEvent } from "../ipc";
import type {
  Task,
  TaskListRow,
  TaskLastGate,
  TaskChallengeBadge,
  TaskState,
  TaskEvent,
  WorkspaceAgent,
  SessionContextEvent,
} from "../ipc";
import type { RosterChangedEvent } from "../ipc/events";
import { timeHint } from "../lib/timeHint";
import {
  chainUp,
  lowestCommonSupervisor,
  reportsOf,
  rootMembers,
} from "../lib/positions";
import { PositionLine } from "./Position";
import { mockTaskGet, mockTaskList } from "./laneBoardMock";

/* Lane board + workspace telemetry strip (ADR 0008 · Lane D).
   Fidelity target: .arta/proto/screens/lane-board.tsx @ fa4929b (Arta canon).
   The proto's `var(--color-*)` design tokens are remapped to the real app's
   flipping semantic tokens so the view is theme-aware (the proto was dark-only),
   and the proto's `avClass[...]` avatar palette is replaced by the app's real
   agent-identity join (instance.list + agentDef.list, the same join the
   Roster/MemoryGraph use). Two components, exactly as the canon splits them:
   TelemetryStrip (workspace context meters off the live `session:context` path
   ContextBars already consumes) and the LaneBoard kanban.

   Tasks are read from a local mock until Lane A's `task.list` merges — the
   INTEGRATION seam is `load()` below (one line swaps mock → `ipc.task.list`);
   the `task:changed` bus event already drives the live refresh. */

// ── proto token → app token map (see MemoryGraph.tsx for the same remap) ────
const BORDER = "color-mix(in srgb, var(--color-overlay) 8%, transparent)";
const WORKING = "var(--color-status-working)"; // proto --color-working (amber)
const ROSE = "var(--color-danger)"; // proto --color-rose (compact imminent)
const LIVE = "var(--color-success)"; // proto --color-live (gate pass / merged)
const VIOLET = "#bf5af0"; // proto --color-a-violet (review) — matches MemoryGraph
const SKY = "#32ade6"; // proto --color-a-sky (design-canon glyph)
const FAINT = "var(--color-text-tertiary)";
// A brighter hairline (proto --hair-2) for card-hover borders + dashed empty
// placeholders; still theme-flipping via --color-overlay.
const HAIR2 = "color-mix(in srgb, var(--color-overlay) 11%, transparent)";
// The bounded-lane surface: a very subtle top-down overlay gradient (proto .lane),
// theme-aware — reads as faint light in dark, faint dark in light.
const LANE_BG =
  "linear-gradient(180deg, color-mix(in srgb, var(--color-overlay) 3%, transparent), color-mix(in srgb, var(--color-overlay) 1%, transparent))";
// Soft amber glow ring for the in-progress status dot (proto .dot working).
const WORKING_GLOW = "0 0 0 3px color-mix(in srgb, var(--color-status-working) 15%, transparent)";

// Column order + accent, mirroring the canon. `abandoned` is off the happy path
// — surfaced only through the "All lanes" toggle, never a sixth always-on column.
const COLUMNS: { state: TaskState; label: string; accent: string }[] = [
  { state: "planned", label: "Planned", accent: FAINT },
  { state: "claimed", label: "Claimed", accent: "var(--color-accent)" },
  { state: "in_progress", label: "In progress", accent: WORKING },
  { state: "review", label: "Review", accent: VIOLET },
  { state: "merged", label: "Merged", accent: LIVE },
];
const ABANDONED_COLUMN = { state: "abandoned" as TaskState, label: "Abandoned", accent: FAINT };

function columnAccent(state: TaskState): string {
  return [...COLUMNS, ABANDONED_COLUMN].find((column) => column.state === state)?.accent ?? FAINT;
}

// Context pressure → meter colour. Honest graded signal for a whole swarm: amber
// warns, rose = compact imminent (per-session ContextBars is always accent).
function meterColor(pct: number): string {
  if (pct >= 88) return ROSE;
  if (pct >= 68) return WORKING;
  return "var(--color-accent)";
}

// The gate's human label, derived client-side from the command (no stored label —
// lead ruling). Maps the usual gate commands to a short name.
function gateLabel(cmd: string): string {
  if (/\btsc\b/.test(cmd)) return "tsc";
  if (/clippy/.test(cmd)) return "clippy";
  if (/cargo test/.test(cmd)) return "cargo test";
  if (/cargo build/.test(cmd)) return "cargo build";
  const first = cmd.trim().split(/\s+/).slice(0, 2).join(" ");
  return first || cmd;
}

// Whole minutes until an ISO deadline, clamped at 0 (never a negative countdown).
function minutesUntil(iso: string): number {
  return Math.max(0, Math.round((new Date(iso).getTime() - Date.now()) / 60_000));
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function readString(record: Record<string, unknown>, key: string): string | undefined {
  const value = record[key];
  return typeof value === "string" ? value : undefined;
}

function challengePayloadOf(event: TaskEvent | undefined): ChallengePayload | null {
  if (!event || event.kind !== "challenge" || !isRecord(event.payload)) return null;
  return {
    claim: readString(event.payload, "claim"),
    evidence: readString(event.payload, "evidence"),
    proposal: readString(event.payload, "proposal"),
    default: readString(event.payload, "default"),
  };
}

// ── agent identity (roster join) ────────────────────────────────────────────
interface Identity {
  name: string;
  color: string;
  initials: string;
}
function initialsOf(name: string): string {
  return name.trim().slice(0, 1).toUpperCase() || "?";
}

interface TaskDetailState {
  task: Task;
  events: TaskEvent[];
}

interface ChallengePayload {
  claim?: string;
  evidence?: string;
  proposal?: string;
  default?: string;
}

// ── telemetry view model ────────────────────────────────────────────────────
interface AgentTelemetry {
  instanceId: string;
  ident: Identity;
  working: boolean;
  /** live token reading from `session:context`; undefined until the first event. */
  ctx?: { tokens: number; limit: number; estimated: boolean };
}

export interface LaneBoardProps {
  workspaceId: string;
  workspaceName?: string;
  onClose?: () => void;
}

export function LaneBoard({ workspaceId, workspaceName, onClose }: LaneBoardProps) {
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  // ── tasks — the real `task.list` over the UDS wire (Lane A) ────────────────
  const [tasks, setTasks] = useState<TaskListRow[]>([]);
  const [selectedSlug, setSelectedSlug] = useState<string | null>(null);
  const [taskDetail, setTaskDetail] = useState<TaskDetailState | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [viewMode, setViewMode] = useState<"board" | "org">("board");
  const load = useCallback(() => {
    ipc.task
      .list({ workspaceId })
      .then((rows) => {
        if (mounted.current) setTasks(rows);
      })
      .catch((err: unknown) => {
        // Plain `vite` dev (no Tauri backend) lands here — fall back to the mock
        // so the board still renders for pure-frontend work. A real backend error
        // in a Tauri build surfaces in the console and leaves the board empty
        // (honest — no faked tasks in production).
        if (import.meta.env.DEV) {
          console.error("LaneBoard: task.list failed — using dev mock", err);
          if (mounted.current) setTasks(mockTaskList());
        }
      });
  }, [workspaceId]);
  useEffect(load, [load]);
  const loadTaskDetail = useCallback(
    (slug: string) => {
      setDetailLoading(true);
      ipc.task
        .get({ workspaceId, slug })
        .then((detail) => {
          if (mounted.current) setTaskDetail(detail);
        })
        .catch((err: unknown) => {
          if (import.meta.env.DEV) {
            console.error("LaneBoard: task.get failed", err);
            try {
              if (mounted.current) setTaskDetail(mockTaskGet(slug));
            } catch (mockErr) {
              console.error("LaneBoard: mock task.get failed", mockErr);
              if (mounted.current) setTaskDetail(null);
            }
          } else if (mounted.current) {
            setTaskDetail(null);
          }
        })
        .finally(() => {
          if (mounted.current) setDetailLoading(false);
        });
    },
    [workspaceId],
  );
  useEffect(() => {
    if (!selectedSlug) {
      setTaskDetail(null);
      setDetailLoading(false);
      return;
    }
    loadTaskDetail(selectedSlug);
  }, [loadTaskDetail, selectedSlug]);
  // Live refresh: every mutating `task.*` handler emits `task:changed` (Lane A).
  useTaskChanged(workspaceId, () => {
    load();
    if (selectedSlug) loadTaskDetail(selectedSlug);
  });

  // ── agent identity map (instance.list + agentDef.list, like MemoryGraph) ───
  const [instances, setInstances] = useState<WorkspaceAgent[]>([]);
  const [identityById, setIdentityById] = useState<Record<string, Identity>>({});
  const seq = useRef(0);
  const loadRoster = useCallback(() => {
    const mine = ++seq.current;
    Promise.all([ipc.instance.list({ workspaceId }), ipc.agentDef.list()])
      .then(([insts, defs]) => {
        if (mine !== seq.current || !mounted.current) return;
        const defById = new Map(defs.map((d) => [d.id, d]));
        const ident: Record<string, Identity> = {};
        for (const inst of insts) {
          const def = defById.get(inst.agentDefId);
          const name = inst.name ?? def?.name ?? inst.id;
          ident[inst.id] = {
            name,
            color: def?.color ?? "#6e6e73",
            initials: initialsOf(name),
          };
        }
        setInstances(insts);
        setIdentityById(ident);
      })
      .catch((err: unknown) => {
        if (import.meta.env.DEV) console.error("LaneBoard: roster load failed", err);
      });
  }, [workspaceId]);
  // Refresh on mount + a modest cadence so the `working` marks stay live (the
  // Roster's own working signal; ADR R-act-1). Live token counts layer on top
  // via `session:context` below.
  useEffect(() => {
    loadRoster();
    const t = setInterval(loadRoster, 4_000);
    return () => clearInterval(t);
  }, [loadRoster]);
  useEvent<RosterChangedEvent>("roster:changed", (payload) => {
    if (payload.workspaceId === workspaceId) loadRoster();
  });

  const resolve = useCallback(
    (id: string | undefined): Identity | null =>
      id ? (identityById[id] ?? { name: id, color: "#6e6e73", initials: initialsOf(id) }) : null,
    [identityById],
  );

  // ── live context meters — reuse the SAME `session:context` path ContextBars
  //    consumes, aggregated across every session in the workspace. ────────────
  const [ctxBySession, setCtxBySession] = useState<
    Record<string, { tokens: number; limit: number; estimated: boolean }>
  >({});
  useEvent<SessionContextEvent>("session:context", (e) => {
    setCtxBySession((prev) => ({
      ...prev,
      [e.sessionId]: { tokens: e.contextTokens, limit: e.contextLimit, estimated: e.estimated },
    }));
  });

  const telemetry = useMemo<AgentTelemetry[]>(() => {
    return instances
      .filter((i) => i.status === "running")
      .map((i) => ({
        instanceId: i.id,
        ident: resolve(i.id)!,
        working: i.working ?? false,
        ctx: i.sessionId ? ctxBySession[i.sessionId] : undefined,
      }));
  }, [instances, ctxBySession, resolve]);

  // ── header controls ────────────────────────────────────────────────────────
  const [query, setQuery] = useState("");
  const [showAll, setShowAll] = useState(false); // reveal the Abandoned column
  const q = query.trim().toLowerCase();

  const columns = showAll ? [...COLUMNS, ABANDONED_COLUMN] : COLUMNS;
  const visibleTasks = useMemo(
    () => (q ? tasks.filter((t) => (t.slug + " " + t.title).toLowerCase().includes(q)) : tasks),
    [tasks, q],
  );

  const total = tasks.length;
  const openChallenges = useMemo(
    () =>
      tasks.reduce(
        (n, t) => n + t.challenges.filter((c) => c.status === "open").length,
        0,
      ),
    [tasks],
  );
  const selectedTask = useMemo(
    () => tasks.find((task) => task.slug === selectedSlug) ?? null,
    [tasks, selectedSlug],
  );

  useEffect(() => {
    if (selectedSlug && !tasks.some((task) => task.slug === selectedSlug)) {
      setSelectedSlug(null);
    }
  }, [selectedSlug, tasks]);

  return (
    <main
      className="flex-1 min-w-0 relative overflow-hidden"
      style={{ background: "var(--color-bg-canvas)", color: "var(--color-text-body)" }}
    >
      {/* floating blurred header — mirrors the MemoryGraph.tsx mount template
          (center pane, absolute top bar, onClose X returns to the agent pane). */}
      <div
        className="absolute top-0 left-0 right-0 h-12 z-20 flex items-center gap-3 px-4 border-b border-overlay/[0.06]"
        style={{
          background: "color-mix(in srgb, var(--color-bg-canvas) 78%, transparent)",
          backdropFilter: "blur(8px)",
        }}
      >
        <span
          className="w-6 h-6 rounded-[7px] grid place-items-center shrink-0 bg-surface-raised text-text-primary ring-hair"
        >
          <Columns3 size={13} />
        </span>
        <div className="leading-tight">
          <div className="text-[0.84rem] font-semibold tracking-tight text-text-primary">Lane board</div>
          <div className="text-[0.64rem] -mt-0.5 text-text-tertiary">
            {workspaceName ? `${workspaceName} · agent work system` : "agent work system"}
          </div>
        </div>
        <HeaderSegment
          value={viewMode}
          onChange={setViewMode}
          options={[
            { value: "board", label: "Board", icon: <Columns3 size={12} /> },
            { value: "org", label: "Org", icon: <Network size={12} /> },
          ]}
        />
        <HeaderPill>
          <Columns3 size={11} style={{ color: "var(--color-accent)" }} />
          {total} {total === 1 ? "task" : "tasks"}
        </HeaderPill>
        {openChallenges > 0 && (
          <HeaderPill>
            <Swords size={11} style={{ color: WORKING }} />
            <span style={{ color: WORKING }}>{openChallenges} open</span>
          </HeaderPill>
        )}
        <div className="ml-auto flex items-center gap-2">
          <div
            className="flex items-center gap-2 rounded-md px-2.5 h-7 bg-bg-canvas"
            style={{ border: `1px solid ${BORDER}` }}
          >
            <Search size={12} className="text-text-tertiary shrink-0" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Filter"
              className="bg-transparent outline-none text-[0.72rem] w-24 text-text-body placeholder:text-text-tertiary"
            />
            {query && (
              <button
                onClick={() => setQuery("")}
                className="text-text-tertiary shrink-0"
                aria-label="Clear filter"
              >
                <X size={12} />
              </button>
            )}
          </div>
          <button
            onClick={() => setShowAll((v) => !v)}
            title={showAll ? "Hide abandoned lane" : "Show all lanes incl. abandoned"}
            className="inline-flex items-center gap-1.5 rounded-md px-2.5 h-7 text-[0.72rem] bg-bg-canvas transition-colors"
            style={{
              border: `1px solid ${BORDER}`,
              color: showAll ? "var(--color-text-primary)" : "var(--color-text-secondary)",
            }}
          >
            <Filter size={12} /> All lanes
          </button>
          {onClose && (
            <button
              onClick={onClose}
              title="Close board"
              aria-label="Close board"
              className="w-7 h-7 grid place-items-center rounded-md text-text-muted hover:bg-overlay/[0.06] hover:text-text-primary transition-colors"
            >
              <X size={15} />
            </button>
          )}
        </div>
      </div>

      {/* content sits below the floating header */}
      <div className="absolute inset-0 pt-12 flex flex-col min-h-0">
        <TelemetryStrip telemetry={telemetry} />
        {viewMode === "board" ? (
          <div className="flex-1 min-h-0 flex">
            <div className="flex-1 min-h-0 overflow-x-auto scroll-thin">
              <div className="h-full flex gap-3.5 px-5 pt-3 pb-4">
                {columns.map((c) => (
                  <Column
                    key={c.state}
                    col={c}
                    tasks={visibleTasks}
                    resolve={resolve}
                    selectedSlug={selectedSlug}
                    onOpenTask={setSelectedSlug}
                  />
                ))}
              </div>
            </div>
            <TaskDetailPane
              row={selectedTask}
              detail={taskDetail}
              loading={detailLoading}
              roster={instances}
              resolve={resolve}
              onClose={() => setSelectedSlug(null)}
            />
          </div>
        ) : (
          <OrgChartPane
            roster={instances}
            resolve={resolve}
            query={q}
          />
        )}
      </div>
    </main>
  );
}

// ── Telemetry strip ──────────────────────────────────────────────────────────

function Meter({ t }: { t: AgentTelemetry }) {
  const pct = t.ctx ? Math.min(100, Math.round((t.ctx.tokens / t.ctx.limit) * 100)) : null;
  const color = pct != null ? meterColor(pct) : FAINT;
  return (
    <div
      className="flex items-center gap-2 shrink-0"
      title={
        t.ctx
          ? `${t.ident.name} — ${t.ctx.tokens.toLocaleString()} / ${t.ctx.limit.toLocaleString()} tokens${
              t.ctx.estimated ? " (estimate)" : ""
            }`
          : `${t.ident.name} — no context reading yet`
      }
    >
      <Avatar ident={t.ident} />
      <span className="text-[0.72rem] font-medium text-text-primary">{t.ident.name}</span>
      {t.working && (
        <LoaderCircle size={10} className="animate-spin shrink-0" style={{ color: WORKING }} />
      )}
      <span
        className="w-16 h-1.5 rounded-full overflow-hidden shrink-0"
        style={{ background: BORDER }}
      >
        {pct != null && (
          <span
            className="block h-full rounded-full"
            style={{ width: `${pct}%`, background: color }}
          />
        )}
      </span>
      <span
        className="font-mono text-[0.66rem] tabular-nums w-8 text-right"
        style={{ color }}
      >
        {pct != null ? `${pct}%` : "—"}
      </span>
    </div>
  );
}

function TelemetryStrip({ telemetry }: { telemetry: AgentTelemetry[] }) {
  const working = telemetry.filter((t) => t.working).length;
  const peak = telemetry.reduce((mx, t) => {
    const p = t.ctx ? Math.round((t.ctx.tokens / t.ctx.limit) * 100) : 0;
    return Math.max(mx, p);
  }, 0);
  return (
    <div
      className="shrink-0 flex items-center gap-5 px-5 h-14 border-b overflow-x-auto scroll-thin"
      style={{ borderColor: BORDER, background: "var(--color-sidebar)" }}
    >
      <div className="flex flex-col shrink-0 pr-1">
        <span className="text-[0.62rem] tracking-[0.09em] uppercase font-semibold text-text-tertiary">
          Context
        </span>
        <span className="text-[0.7rem] leading-tight text-text-tertiary">
          {telemetry.length} live · <span style={{ color: WORKING }}>{working} working</span>
          {peak >= 88 && (
            <>
              {" · "}
              <span style={{ color: ROSE }}>peak {peak}%</span>
            </>
          )}
        </span>
      </div>
      <span className="w-px h-7 shrink-0" style={{ background: BORDER }} />
      {telemetry.length === 0 ? (
        <span className="text-[0.72rem] text-text-tertiary">No live agents</span>
      ) : (
        <div className="flex items-center gap-4">
          {telemetry.map((t) => (
            <Meter key={t.instanceId} t={t} />
          ))}
        </div>
      )}
    </div>
  );
}

function HeaderSegment<T extends string>({
  value,
  onChange,
  options,
}: {
  value: T;
  onChange: (value: T) => void;
  options: Array<{ value: T; label: string; icon: ReactNode }>;
}) {
  return (
    <div className="ml-1 flex gap-1 rounded-lg p-1 bg-bg-canvas" style={{ border: `1px solid ${BORDER}` }}>
      {options.map((option) => {
        const active = option.value === value;
        return (
          <button
            key={option.value}
            onClick={() => onChange(option.value)}
            className={`inline-flex items-center gap-1.5 h-7 px-2.5 rounded-md text-[0.72rem] font-medium transition-colors ${
              active ? "text-text-primary" : "text-text-secondary hover:text-text-primary"
            }`}
            style={
              active
                ? {
                    background: "var(--color-surface-raised)",
                    boxShadow: "inset 0 0 0 1px color-mix(in srgb, var(--color-overlay) 10%, transparent)",
                  }
                : undefined
            }
          >
            <span style={{ color: active ? "var(--color-accent)" : FAINT }}>{option.icon}</span>
            {option.label}
          </button>
        );
      })}
    </div>
  );
}

function trackLabelOf(agent: WorkspaceAgent): string {
  if (agent.roleName) return agent.roleName;
  if (agent.cliKind === "claude-code") return "CLI";
  if (agent.cliKind === "codex") return "Codex";
  if (agent.cliKind === "custom") return "Custom";
  return "Agent";
}

function activityLabel(agent: WorkspaceAgent): string {
  if (agent.working) return "working";
  if (agent.lastActivityAt) return timeHint(agent.lastActivityAt);
  return agent.status;
}

const STATUS_DOT: Record<WorkspaceAgent["status"], string> = {
  running: LIVE,
  waiting: WORKING,
  idle: FAINT,
};

type Guide = "line" | "space";

interface OrgRow {
  id: string;
  guides: Guide[];
  elbow: "tee" | "ell";
}

function OrgChartPane({
  roster,
  resolve,
  query,
}: {
  roster: WorkspaceAgent[];
  resolve: (id: string | undefined) => Identity | null;
  query: string;
}) {
  const instanceById = useMemo(() => new Map(roster.map((agent) => [agent.id, agent])), [roster]);
  const includeSet = useMemo(() => {
    if (!query) return null;
    const next = new Set<string>();
    for (const agent of roster) {
      const ident = resolve(agent.id);
      const haystack = `${ident?.name ?? agent.id} ${trackLabelOf(agent)}`.toLowerCase();
      if (!haystack.includes(query)) continue;
      for (const id of chainUp(agent.id, roster)) next.add(id);
    }
    return next;
  }, [query, resolve, roster]);
  const childrenOf = useCallback(
    (id: string) =>
      reportsOf(id, roster)
        .filter((childId) => includeSet == null || includeSet.has(childId))
        .sort((left, right) => {
          const leftName = resolve(left)?.name ?? left;
          const rightName = resolve(right)?.name ?? right;
          return leftName.localeCompare(rightName);
        }),
    [includeSet, resolve, roster],
  );
  const rootIds = useMemo(
    () =>
      rootMembers(roster)
        .filter((id) => includeSet == null || includeSet.has(id))
        .sort((left, right) => {
          const leftName = resolve(left)?.name ?? left;
          const rightName = resolve(right)?.name ?? right;
          return leftName.localeCompare(rightName);
        }),
    [includeSet, resolve, roster],
  );
  const rows = useMemo(() => {
    const next: OrgRow[] = [];
    const visit = (ids: string[], guides: Guide[]) => {
      ids.forEach((id, index) => {
        const isLast = index === ids.length - 1;
        next.push({ id, guides, elbow: isLast ? "ell" : "tee" });
        visit(childrenOf(id), [...guides, isLast ? "space" : "line"]);
      });
    };
    visit(rootIds, []);
    return next;
  }, [childrenOf, rootIds]);
  const visibleCount = includeSet?.size ?? roster.length;

  if (roster.length === 0) {
    return (
      <div className="flex-1 overflow-y-auto scroll-thin px-6 py-6">
        <div
          className="max-w-[560px] mx-auto rounded-xl px-4 py-5 text-[0.74rem] text-text-tertiary"
          style={{ border: `1px dashed ${BORDER}` }}
        >
          Org view appears once this workspace has roster data.
        </div>
      </div>
    );
  }

  return (
    <div className="flex-1 overflow-y-auto scroll-thin px-6 py-6">
      <div className="max-w-[720px] mx-auto">
        <p className="text-[0.72rem] text-text-tertiary leading-relaxed mb-4">
          The supervisor chain of this workspace. The rails show the path that challenges, escalations, and stall alerts climb.
        </p>

        <div
          className="rounded-lg px-3 py-2.5 flex items-center gap-2.5 bg-surface-raised border border-overlay/[0.08]"
          style={{ borderColor: "color-mix(in srgb, var(--color-accent) 26%, var(--color-overlay) 8%)" }}
        >
          <span
            className="w-8 h-8 rounded-[8px] grid place-items-center shrink-0"
            style={{
              color: "var(--color-accent)",
              background: "color-mix(in srgb, var(--color-accent) 12%, transparent)",
              border: "1px solid color-mix(in srgb, var(--color-accent) 30%, transparent)",
            }}
          >
            <Crown size={14} />
          </span>
          <div className="min-w-0 flex-1">
            <div className="text-[0.82rem] font-semibold text-text-primary">Human</div>
            <div className="text-[0.66rem] text-text-secondary">Top of the chain · final tiebreaker</div>
          </div>
          <span className="text-[0.62rem] font-mono" style={{ color: FAINT }}>
            {visibleCount} shown
          </span>
        </div>

        <div className="mt-2 flex flex-col gap-1.5">
          {rows.length === 0 ? (
            <div
              className="rounded-lg px-3 py-4 text-[0.72rem] text-text-tertiary"
              style={{ border: `1px dashed ${BORDER}` }}
            >
              No members match this filter.
            </div>
          ) : (
            rows.map((row) => {
              const agent = instanceById.get(row.id);
              if (!agent) return null;
              return (
                <div key={row.id} className="flex items-stretch">
                  <GuideColumns guides={row.guides} elbow={row.elbow} />
                  <div className="flex-1 min-w-0">
                    <OrgNodeRow
                      agent={agent}
                      ident={resolve(row.id)}
                      reportCount={reportsOf(row.id, roster).length}
                    />
                  </div>
                </div>
              );
            })
          )}
        </div>
      </div>
    </div>
  );
}

function GuideColumns({
  guides,
  elbow,
}: {
  guides: Guide[];
  elbow: "tee" | "ell";
}) {
  return (
    <>
      {guides.map((guide, index) => (
        <span key={index} className="w-6 shrink-0 relative self-stretch" aria-hidden>
          {guide === "line" && (
            <span
              className="absolute top-0 bottom-0 w-px"
              style={{ left: "50%", background: "color-mix(in srgb, var(--color-text-tertiary) 55%, transparent)" }}
            />
          )}
        </span>
      ))}
      <span className="w-6 shrink-0 relative self-stretch" aria-hidden>
        <span
          className="absolute top-0 w-px"
          style={{ left: "50%", height: "50%", background: "color-mix(in srgb, var(--color-text-tertiary) 55%, transparent)" }}
        />
        {elbow === "tee" && (
          <span
            className="absolute bottom-0 w-px"
            style={{ left: "50%", height: "50%", background: "color-mix(in srgb, var(--color-text-tertiary) 55%, transparent)" }}
          />
        )}
        <span
          className="absolute h-px"
          style={{ left: "50%", right: 0, top: "50%", background: "color-mix(in srgb, var(--color-text-tertiary) 55%, transparent)" }}
        />
      </span>
    </>
  );
}

function OrgNodeRow({
  agent,
  ident,
  reportCount,
}: {
  agent: WorkspaceAgent;
  ident: Identity | null;
  reportCount: number;
}) {
  return (
    <div className="rounded-lg px-3 py-2.5 flex items-center gap-2.5 bg-surface-raised border border-overlay/[0.08] min-w-0">
      <Avatar ident={ident ?? undefined} dashed={!ident} />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2 min-w-0">
          <span className="text-[0.8rem] font-semibold text-text-primary truncate">
            {ident?.name ?? agent.id}
          </span>
          {agent.working && (
            <LoaderCircle size={11} className="shrink-0 animate-spin" style={{ color: WORKING }} />
          )}
        </div>
        <PositionLine
          levelId={agent.level}
          track={trackLabelOf(agent)}
          compact
          className="mt-0.5"
        />
      </div>
      <div className="flex flex-col items-end gap-0.5 shrink-0">
        {reportCount > 0 && (
          <span className="text-[0.6rem] font-mono" style={{ color: FAINT }}>
            {reportCount} report{reportCount === 1 ? "" : "s"}
          </span>
        )}
        <span className="inline-flex items-center gap-1 text-[0.62rem]" style={{ color: FAINT }}>
          <span className="w-1.5 h-1.5 rounded-full" style={{ background: STATUS_DOT[agent.status] }} />
          {activityLabel(agent)}
        </span>
      </div>
    </div>
  );
}

function TaskDetailPane({
  row,
  detail,
  loading,
  roster,
  resolve,
  onClose,
}: {
  row: TaskListRow | null;
  detail: TaskDetailState | null;
  loading: boolean;
  roster: WorkspaceAgent[];
  resolve: (id: string | undefined) => Identity | null;
  onClose: () => void;
}) {
  if (!row && !loading) return null;

  const openChallenge = row?.challenges.find((challenge) => challenge.status === "open") ?? null;
  const challengeEvent =
    openChallenge != null ? detail?.events.find((event) => event.id === openChallenge.id) : undefined;
  const payload = challengePayloadOf(challengeEvent);
  const filerId = challengeEvent?.actorAgentId ?? row?.implementerAgentId;
  const ownerId = row?.ownerAgentId;
  const expectedRulerId =
    filerId && ownerId ? lowestCommonSupervisor(filerId, ownerId, roster) : ownerId ?? null;
  const route =
    filerId != null
      ? (() => {
          const fullChain = chainUp(filerId, roster);
          if (expectedRulerId == null) return fullChain;
          const index = fullChain.indexOf(expectedRulerId);
          return index >= 0 ? fullChain.slice(0, index + 1) : fullChain;
        })()
      : [];
  const owner = resolve(row?.ownerAgentId);
  const implementer = resolve(row?.implementerAgentId);

  return (
    <aside
      className="w-[392px] shrink-0 border-l bg-sidebar overflow-y-auto scroll-thin"
      style={{ borderColor: BORDER }}
    >
      <div className="sticky top-0 z-10 px-4 py-3 bg-sidebar border-b" style={{ borderColor: BORDER }}>
        <div className="flex items-center gap-2">
          <span className="font-mono text-[0.66rem]" style={{ color: FAINT }}>
            {row?.slug ?? "task"}
          </span>
          <span
            className="inline-flex items-center gap-1 h-5 px-1.5 rounded-md text-[0.62rem] font-medium"
            style={{
              color: row ? columnAccent(row.state) : FAINT,
              background: row
                ? `color-mix(in srgb, ${columnAccent(row.state)} 12%, transparent)`
                : "transparent",
              border: row
                ? `1px solid color-mix(in srgb, ${columnAccent(row.state)} 28%, transparent)`
                : `1px solid ${BORDER}`,
            }}
          >
            {row?.state.replace("_", " ") ?? "detail"}
          </span>
          <button
            onClick={onClose}
            className="ml-auto w-7 h-7 grid place-items-center rounded-md text-text-muted hover:bg-overlay/[0.06] hover:text-text-primary"
            aria-label="Close task detail"
          >
            <X size={14} />
          </button>
        </div>
        <div className="mt-2 text-[0.86rem] font-semibold leading-snug text-text-primary">
          {row?.title ?? "Task detail"}
        </div>
        {row && (
          <div className="mt-2 flex items-center gap-2.5">
            <AgentPips t={row} resolve={resolve} />
            <span className="text-[0.62rem]" style={{ color: FAINT }}>
              owner {owner?.name ?? "—"} · implementer {implementer?.name ?? "—"}
            </span>
          </div>
        )}
      </div>

      <div className="px-4 py-4 space-y-4">
        {loading && (
          <div className="flex items-center gap-2 text-[0.72rem]" style={{ color: FAINT }}>
            <LoaderCircle size={13} className="animate-spin" />
            Loading task detail…
          </div>
        )}

        {!loading && openChallenge == null && (
          <div
            className="rounded-xl px-3.5 py-4 text-[0.72rem] text-text-tertiary"
            style={{ border: `1px dashed ${BORDER}` }}
          >
            No open challenges on this task.
          </div>
        )}

        {!loading && openChallenge != null && row && (
          <>
            <div
              className="rounded-xl p-3.5"
              style={{
                background: "var(--color-surface-raised)",
                border: `1px solid color-mix(in srgb, ${WORKING} 24%, transparent)`,
              }}
            >
              <div className="flex items-center gap-2 mb-2.5">
                <Swords size={14} style={{ color: WORKING }} />
                <span className="text-[0.8rem] font-semibold text-text-primary">Open challenge</span>
                {openChallenge.deadlineAt && (
                  <span className="ml-auto inline-flex items-center gap-1 text-[0.66rem] font-mono" style={{ color: WORKING }}>
                    <Clock size={11} /> {minutesUntil(openChallenge.deadlineAt)}m
                  </span>
                )}
              </div>
              <p className="text-[0.8rem] leading-snug text-text-primary">
                {payload?.claim ?? openChallenge.claim}
              </p>
              <div className="mt-3 grid gap-2">
                {payload?.evidence && (
                  <DetailField icon={<FileCode2 size={12} />} label="Evidence" value={payload.evidence} mono />
                )}
                {payload?.proposal && (
                  <DetailField icon={<ShieldQuestion size={12} />} label="Proposal" value={payload.proposal} />
                )}
                {payload?.default && (
                  <DetailField icon={<Gavel size={12} />} label="Default if unruled" value={payload.default} />
                )}
              </div>
            </div>

            <div>
              <div className="flex items-center gap-2 mb-3">
                <span className="text-[0.64rem] font-semibold tracking-[0.08em] uppercase" style={{ color: FAINT }}>
                  Escalation trace
                </span>
                <span className="text-[0.66rem]" style={{ color: FAINT }}>
                  routes up the supervisor chain
                </span>
              </div>
              {route.length === 0 && !expectedRulerId ? (
                <div
                  className="rounded-xl px-3.5 py-4 text-[0.72rem] text-text-tertiary"
                  style={{ border: `1px dashed ${BORDER}` }}
                >
                  Challenge routing appears once the roster chain and task owner are both present.
                </div>
              ) : (
                <div className="flex flex-col gap-3">
                  {route.map((id, index) => (
                    <EscalationStep
                      key={id}
                      connectorAbove={index > 0}
                      ident={resolve(id)}
                      agent={roster.find((candidate) => candidate.id === id)}
                      kind={
                        index === 0
                          ? "filed"
                          : id === expectedRulerId
                            ? "ruling"
                            : "through"
                      }
                    />
                  ))}
                  {expectedRulerId == null && (
                    <EscalationStep
                      connectorAbove={route.length > 0}
                      human
                      kind="ruling"
                    />
                  )}
                  {expectedRulerId != null && (
                    <EscalationStep
                      connectorAbove
                      human
                      kind="tiebreaker"
                    />
                  )}
                </div>
              )}
            </div>
          </>
        )}
      </div>
    </aside>
  );
}

function EscalationStep({
  kind,
  ident,
  agent,
  human,
  connectorAbove,
}: {
  kind: "filed" | "through" | "ruling" | "tiebreaker";
  ident?: Identity | null;
  agent?: WorkspaceAgent;
  human?: boolean;
  connectorAbove: boolean;
}) {
  const stepMeta = {
    filed: { label: "Filed", icon: <Swords size={11} />, color: WORKING },
    through: { label: "Routes through", icon: <ArrowUp size={11} />, color: FAINT },
    ruling: { label: "Expected to rule", icon: <Gavel size={11} />, color: "var(--color-accent)" },
    tiebreaker: { label: "Tiebreaker", icon: <Scale size={11} />, color: FAINT },
  }[kind];

  return (
    <div className="relative pl-7">
      {connectorAbove && (
        <>
          <span
            className="absolute left-[13px] -top-2 h-2 w-px"
            style={{ background: "color-mix(in srgb, var(--color-text-tertiary) 55%, transparent)" }}
          />
          <span className="absolute left-[7px] -top-[1px] grid place-items-center">
            <ArrowUp size={13} style={{ color: FAINT }} />
          </span>
        </>
      )}
      <div
        className="rounded-lg px-3 py-2.5 flex items-center gap-2.5 bg-surface-raised border border-overlay/[0.08]"
        style={
          kind === "ruling"
            ? {
                borderColor: "color-mix(in srgb, var(--color-accent) 42%, transparent)",
                background: "color-mix(in srgb, var(--color-accent) 6%, var(--color-surface-raised))",
              }
            : undefined
        }
      >
        {human ? (
          <span
            className="w-[18px] h-[18px] rounded-[5px] grid place-items-center shrink-0"
            style={{
              color: "var(--color-accent)",
              background: "color-mix(in srgb, var(--color-accent) 12%, transparent)",
              border: "1px solid color-mix(in srgb, var(--color-accent) 30%, transparent)",
            }}
          >
            <Scale size={11} />
          </span>
        ) : (
          <Avatar ident={ident ?? undefined} dashed={!ident} />
        )}
        <div className="min-w-0 flex-1">
          <div className="text-[0.8rem] font-semibold text-text-primary">
            {human ? "Human" : ident?.name ?? agent?.id ?? "Unknown"}
          </div>
          {human ? (
            <div className="text-[0.66rem] text-text-secondary">Top of the chain · final tiebreaker</div>
          ) : (
            <PositionLine
              levelId={agent?.level}
              track={agent ? trackLabelOf(agent) : "Agent"}
              compact
              className="mt-0.5"
            />
          )}
        </div>
        <span
          className="inline-flex items-center gap-1 h-[19px] px-1.5 rounded-md text-[0.64rem] font-semibold shrink-0"
          style={{
            color: stepMeta.color,
            background: `color-mix(in srgb, ${stepMeta.color} 12%, transparent)`,
            border: `1px solid color-mix(in srgb, ${stepMeta.color} 28%, transparent)`,
          }}
        >
          {stepMeta.icon}
          {stepMeta.label}
        </span>
      </div>
    </div>
  );
}

function DetailField({
  icon,
  label,
  value,
  mono,
}: {
  icon: ReactNode;
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <div className="flex items-start gap-2">
      <span className="w-4 grid place-items-center pt-0.5 shrink-0" style={{ color: FAINT }}>
        {icon}
      </span>
      <div className="min-w-0">
        <div className="text-[0.6rem] font-semibold tracking-[0.08em] uppercase" style={{ color: FAINT }}>
          {label}
        </div>
        <div className={`text-[0.74rem] leading-snug text-text-secondary ${mono ? "font-mono" : ""}`}>
          {value}
        </div>
      </div>
    </div>
  );
}

// ── card badges ──────────────────────────────────────────────────────────────

function GateChip({ g }: { g: TaskLastGate }) {
  const ok = g.exit === 0;
  const c = ok ? LIVE : ROSE;
  return (
    <span
      className="inline-flex items-center gap-1 pl-1 pr-1.5 h-[18px] rounded-md text-[0.64rem] font-medium"
      style={{
        color: c,
        background: `color-mix(in srgb, ${c} 12%, transparent)`,
        border: `1px solid color-mix(in srgb, ${c} 28%, transparent)`,
      }}
      title={`${g.cmd} → exit ${g.exit} @ ${g.sha} · ${timeHint(g.createdAt)}`}
    >
      {ok ? <Check size={11} /> : <X size={11} />}
      <span>{gateLabel(g.cmd)}</span>
      <span className="font-mono flex items-center gap-0.5" style={{ color: FAINT }}>
        <GitCommitHorizontal size={10} />
        {g.sha.slice(0, 6)}
      </span>
    </span>
  );
}

function ChallengeChip({ c }: { c: TaskChallengeBadge }) {
  const open = c.status === "open";
  const col = open ? WORKING : FAINT;
  const mins = open && c.deadlineAt != null ? minutesUntil(c.deadlineAt) : null;
  return (
    <span
      className="inline-flex items-center gap-1 pl-1 pr-1.5 h-[18px] rounded-md text-[0.64rem] font-medium"
      style={{
        color: col,
        background: open ? `color-mix(in srgb, ${WORKING} 12%, transparent)` : "transparent",
        border: `1px solid color-mix(in srgb, ${col} ${open ? "30" : "22"}%, transparent)`,
      }}
      title={c.claim}
    >
      {open ? <Swords size={11} /> : <Gavel size={11} />}
      <span>{open ? "challenge" : "ruled"}</span>
      {mins != null && <span className="font-mono opacity-70">{mins}m</span>}
    </span>
  );
}

function Avatar({ ident, dashed = false }: { ident?: Identity; dashed?: boolean }) {
  if (dashed || !ident) {
    return (
      <span
        className="w-[18px] h-[18px] rounded-[5px] grid place-items-center text-[9px] shrink-0"
        style={{ background: "transparent", color: FAINT, border: `1px dashed ${BORDER}` }}
      >
        ·
      </span>
    );
  }
  return (
    <span
      className="w-[18px] h-[18px] rounded-[5px] grid place-items-center text-[9px] font-bold text-white shrink-0"
      style={{ background: ident.color }}
      title={ident.name}
    >
      {ident.initials}
    </span>
  );
}

function AgentPips({ t, resolve }: { t: TaskListRow; resolve: (id: string | undefined) => Identity | null }) {
  const owner = resolve(t.ownerAgentId);
  const impl = resolve(t.implementerAgentId);
  return (
    <div
      className="flex items-center gap-1.5"
      title={`owner ${owner?.name ?? "—"}${impl ? ` · implementer ${impl.name}` : " · unclaimed"}`}
    >
      <Avatar ident={owner ?? undefined} dashed={!owner} />
      <ArrowRight size={11} className="shrink-0" style={{ color: FAINT }} />
      <Avatar ident={impl ?? undefined} dashed={!impl} />
    </div>
  );
}

const clamp2: React.CSSProperties = {
  display: "-webkit-box",
  WebkitLineClamp: 2,
  WebkitBoxOrient: "vertical",
  overflow: "hidden",
};

function Card({
  t,
  resolve,
  selected,
  onOpenTask,
}: {
  t: TaskListRow;
  resolve: (id: string | undefined) => Identity | null;
  selected: boolean;
  onOpenTask: (slug: string) => void;
}) {
  const [hover, setHover] = useState(false);
  const edge = columnAccent(t.state);
  const border = selected
    ? "color-mix(in srgb, var(--color-accent) 38%, transparent)"
    : hover
      ? HAIR2
      : BORDER;
  const background = selected
    ? "color-mix(in srgb, var(--color-accent) 6%, var(--color-surface-raised))"
    : hover
      ? "var(--color-fill-soft)"
      : "var(--color-surface-raised)";
  const hasBadges = t.lastGates.length > 0 || t.challenges.length > 0;
  return (
    <div
      onClick={() => onOpenTask(t.slug)}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      className="relative rounded-[11px] pl-3.5 pr-3 py-2.5 cursor-pointer overflow-hidden transition-[transform,background-color,border-color] duration-150 ease-out"
      style={{
        background,
        border: `1px solid ${border}`,
        transform: hover && !selected ? "translateY(-1px)" : undefined,
      }}
    >
      {/* status-colored left edge (spec D2) */}
      <span
        className="absolute left-0 top-0 bottom-0 w-[2.5px]"
        style={{ background: edge, opacity: 0.7 }}
        aria-hidden
      />

      <div className="flex items-center gap-2 mb-1.5">
        <span className="font-mono text-[0.64rem] truncate" style={{ color: FAINT }}>
          {t.slug}
        </span>
        {t.designCanon && (
          <span
            className="ml-auto inline-flex items-center gap-1 text-[0.6rem] shrink-0"
            title={`design canon · ${t.designCanon}`}
          >
            <Palette size={11} style={{ color: `color-mix(in srgb, ${SKY} 80%, ${FAINT})` }} />
          </span>
        )}
      </div>

      <div
        className="text-[0.8rem] font-[550] tracking-[-0.005em] leading-[1.34] text-text-primary"
        style={clamp2}
      >
        {t.title}
      </div>

      {hasBadges && (
        <div className="flex flex-wrap gap-1.5 mt-2.5">
          {t.lastGates.map((g) => (
            <GateChip key={g.cmd} g={g} />
          ))}
          {t.challenges.map((c) => (
            <ChallengeChip key={c.id} c={c} />
          ))}
        </div>
      )}

      <div
        className="flex items-center gap-2.5 mt-2.5 pt-2.5"
        style={{ borderTop: `1px solid ${BORDER}` }}
      >
        <AgentPips t={t} resolve={resolve} />
        <span
          className="inline-flex items-center gap-1 text-[0.62rem] tabular-nums"
          style={{ color: FAINT }}
          title={t.fileBoundary.join("\n")}
        >
          <FileCode2 size={11} />
          {t.fileBoundary.length}
        </span>
        <span className="ml-auto text-[0.62rem] font-mono tabular-nums" style={{ color: FAINT }}>
          {timeHint(t.updatedAt)}
        </span>
      </div>
    </div>
  );
}

// ── board column ─────────────────────────────────────────────────────────────

function Column({
  col,
  tasks,
  resolve,
  selectedSlug,
  onOpenTask,
}: {
  col: { state: TaskState; label: string; accent: string };
  tasks: TaskListRow[];
  resolve: (id: string | undefined) => Identity | null;
  selectedSlug: string | null;
  onOpenTask: (slug: string) => void;
}) {
  const items = tasks.filter((t) => t.state === col.state);
  const working = col.state === "in_progress";
  return (
    <section
      className="w-[276px] shrink-0 flex flex-col min-h-0 rounded-[14px]"
      style={{ background: LANE_BG, border: `1px solid ${BORDER}` }}
    >
      <div className="flex items-center gap-2.5 px-3.5 pt-3 pb-2.5 shrink-0">
        <span
          className="w-2 h-2 rounded-full shrink-0"
          style={{ background: col.accent, boxShadow: working ? WORKING_GLOW : undefined }}
        />
        <span className="text-[0.76rem] font-semibold tracking-[0.01em] text-text-primary">
          {col.label}
        </span>
        <span
          className="font-mono text-[0.62rem] tabular-nums rounded-full px-2 py-px min-w-[20px] text-center"
          style={{ color: FAINT, background: "color-mix(in srgb, var(--color-overlay) 5%, transparent)" }}
        >
          {items.length}
        </span>
        <button
          className="ml-auto w-[22px] h-[22px] grid place-items-center rounded-md text-text-tertiary hover:bg-overlay/[0.06] hover:text-text-secondary transition-colors"
          aria-label={`Add task to ${col.label}`}
        >
          <Plus size={13} />
        </button>
      </div>
      <div className="flex-1 overflow-y-auto scroll-thin flex flex-col gap-2 px-2.5 pb-2.5 min-h-0">
        {items.length === 0 ? (
          <div
            className="mx-1 mt-1 rounded-[10px] px-3 py-5 text-center text-[0.7rem] text-text-tertiary"
            style={{ border: `1px dashed ${HAIR2}` }}
          >
            No tasks
          </div>
        ) : (
          items.map((t) => (
            <Card
              key={t.id}
              t={t}
              resolve={resolve}
              selected={selectedSlug === t.slug}
              onOpenTask={onOpenTask}
            />
          ))
        )}
      </div>
    </section>
  );
}

function HeaderPill({ children }: { children: ReactNode }) {
  return (
    <span
      className="inline-flex items-center gap-1.5 h-6 px-2 rounded-md text-[0.68rem] font-medium text-text-secondary shrink-0 bg-bg-canvas"
      style={{ border: `1px solid ${BORDER}` }}
    >
      {children}
    </span>
  );
}
