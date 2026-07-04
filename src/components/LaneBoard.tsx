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
  Filter,
  LoaderCircle,
  Columns3,
} from "lucide-react";
import { ipc, useTaskChanged, useEvent } from "../ipc";
import type {
  TaskListRow,
  TaskLastGate,
  TaskChallengeBadge,
  TaskState,
  WorkspaceAgent,
  SessionContextEvent,
} from "../ipc";
import { timeHint } from "../lib/timeHint";
import { mockTaskList } from "./laneBoardMock";

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

// ── agent identity (roster join) ────────────────────────────────────────────
interface Identity {
  name: string;
  color: string;
  initials: string;
}
function initialsOf(name: string): string {
  return name.trim().slice(0, 1).toUpperCase() || "?";
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

  // ── tasks (mock until Lane A merges — see INTEGRATION seam) ────────────────
  const [tasks, setTasks] = useState<TaskListRow[]>([]);
  const load = useCallback(() => {
    // INTEGRATION: swap the next line for
    //   ipc.task.list({ workspaceId }).then((rows) => { if (mounted.current) setTasks(rows); }).catch(...)
    // — the frozen wire shape is identical, so nothing downstream changes.
    void workspaceId;
    setTasks(mockTaskList());
  }, [workspaceId]);
  useEffect(load, [load]);
  // Live refresh: every mutating `task.*` handler emits `task:changed` (Lane A).
  useTaskChanged(workspaceId, load);

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

        {/* kanban board */}
        <div className="flex-1 min-h-0 overflow-x-auto scroll-thin">
          <div className="h-full flex gap-4 px-5 pt-3">
            {columns.map((c) => (
              <Column key={c.state} col={c} tasks={visibleTasks} resolve={resolve} />
            ))}
          </div>
        </div>
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
      <span className="font-mono opacity-70 flex items-center gap-0.5">
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

function Card({ t, resolve }: { t: TaskListRow; resolve: (id: string | undefined) => Identity | null }) {
  return (
    <div
      className="rounded-lg p-2.5 cursor-pointer bg-surface-raised transition-colors hover:bg-fill-soft"
      style={{ border: `1px solid ${BORDER}` }}
    >
      <div className="flex items-center gap-2 mb-1">
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

      <div className="text-[0.79rem] font-medium leading-snug mb-2 text-text-primary" style={clamp2}>
        {t.title}
      </div>

      {(t.lastGate || t.challenges.length > 0) && (
        <div className="flex flex-wrap gap-1 mb-2">
          {t.lastGate && <GateChip g={t.lastGate} />}
          {t.challenges.map((c) => (
            <ChallengeChip key={c.id} c={c} />
          ))}
        </div>
      )}

      <div className="flex items-center gap-2.5">
        <AgentPips t={t} resolve={resolve} />
        <span
          className="inline-flex items-center gap-1 text-[0.62rem]"
          style={{ color: FAINT }}
          title={t.fileBoundary.join("\n")}
        >
          <FileCode2 size={11} />
          {t.fileBoundary.length}
        </span>
        <span className="ml-auto text-[0.62rem] font-mono" style={{ color: FAINT }}>
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
}: {
  col: { state: TaskState; label: string; accent: string };
  tasks: TaskListRow[];
  resolve: (id: string | undefined) => Identity | null;
}) {
  const items = tasks.filter((t) => t.state === col.state);
  return (
    <section className="w-[266px] shrink-0 flex flex-col min-h-0">
      <div className="flex items-center gap-2 px-1.5 py-2 shrink-0">
        <span className="w-2 h-2 rounded-full shrink-0" style={{ background: col.accent }} />
        <span className="text-[0.74rem] font-semibold text-text-primary">{col.label}</span>
        <span className="font-mono text-[0.64rem]" style={{ color: FAINT }}>
          {items.length}
        </span>
      </div>
      <div className="flex-1 overflow-y-auto scroll-thin flex flex-col gap-2 pb-4 pr-0.5">
        {items.length === 0 ? (
          <div
            className="rounded-lg text-[0.68rem] px-3 py-4 text-center"
            style={{ color: FAINT, border: `1px dashed ${BORDER}` }}
          >
            none
          </div>
        ) : (
          items.map((t) => <Card key={t.id} t={t} resolve={resolve} />)
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
