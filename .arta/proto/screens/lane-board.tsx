import {
  Search, GitCommitHorizontal, Check, X, Swords, Gavel,
  Palette, FileCode2, ArrowRight, Filter, LoaderCircle, Columns3,
} from "lucide-react";

// Open challenges across the board: the ops signal worth surfacing in the header.
const openChallenges = () =>
  tasks.reduce((n, t) => n + t.challenges.filter((c) => c.status === "open").length, 0);
import { agents, avClass } from "../lib/data";
import {
  COLUMNS, tasks, telemetry, meterColor,
  type Task, type GateBadge as Gate, type ChallengeBadge as Challenge, type AgentTelemetry,
} from "../lib/laneBoard";

export const meta = { title: "Lane board · Agent Work System" };

/* Lane board + workspace telemetry strip (ADR 0008 · Lane D canon for Tiësto).
   TWO components the implementer extracts cleanly:

   1. TelemetryStrip: a workspace-level bar aggregating every agent's context
      meter. It reads the SAME `session:context` events ContextBars already
      consumes per session (plan: reuse that data path). Per-session the app's
      meter is always accent; aggregating a whole swarm is exactly where a graded
      scale earns its keep (amber warns, rose = compact imminent). Working agents
      carry the Roster amber working mark.

   2. LaneBoard: tasks as a Kanban by state (planned → claimed → in_progress →
      review → merged). Each card is the frozen task wire shape; the gate ledger
      and challenges are derived from task_event rows. A gate badge is EVIDENCE:
      pass/fail plus the `git rev-parse HEAD` SHA it was true at, so a shared-tree
      "all green" claim can be checked against the commit that produced it.

   Zero Arta imports; react tokens only; lifts into src/components/ unmodified. */

// ── Telemetry strip ────────────────────────────────────────────────────────

function Meter({ t }: { t: AgentTelemetry }) {
  const a = agents[t.agentId];
  const pct = Math.min(100, Math.round((t.tokens / t.limit) * 100));
  const color = meterColor(pct);
  return (
    <div
      className="flex items-center gap-2 shrink-0"
      title={`${a.name} — ${t.tokens.toLocaleString()} / ${t.limit.toLocaleString()} tokens${
        t.estimated ? " (estimate)" : ""
      }`}
    >
      <span className={`av av-xs ${avClass[a.color]}`}>{a.initials}</span>
      <span className="text-[0.72rem] heading font-medium">{a.name}</span>
      {t.working && (
        <LoaderCircle size={10} className="rwork-ico shrink-0" style={{ color: "var(--color-working)" }} />
      )}
      <span className="w-16 h-1.5 rounded-full overflow-hidden shrink-0" style={{ background: "var(--color-border)" }}>
        <span className="block h-full rounded-full" style={{ width: `${pct}%`, background: color }} />
      </span>
      <span className="num text-[0.66rem] tabular-nums w-8 text-right" style={{ color }}>
        {pct}%
      </span>
    </div>
  );
}

function TelemetryStrip() {
  const working = telemetry.filter((t) => t.working).length;
  const peak = Math.max(...telemetry.map((t) => Math.round((t.tokens / t.limit) * 100)));
  return (
    <div
      className="shrink-0 flex items-center gap-5 px-5 h-14 border-b overflow-x-auto scroll-thin"
      style={{ borderColor: "var(--color-border)", background: "var(--color-sidebar)" }}
    >
      <div className="flex flex-col shrink-0 pr-1">
        <span className="label" style={{ paddingBottom: 0 }}>Context</span>
        <span className="text-[0.7rem] dim leading-tight">
          {telemetry.length} live · <span style={{ color: "var(--color-working)" }}>{working} working</span>
          {peak >= 88 && <> · <span style={{ color: "var(--color-rose)" }}>peak {peak}%</span></>}
        </span>
      </div>
      <span className="w-px h-7 shrink-0" style={{ background: "var(--color-border)" }} />
      <div className="flex items-center gap-4">
        {telemetry.map((t) => <Meter key={t.agentId} t={t} />)}
      </div>
    </div>
  );
}

// ── Card badges ────────────────────────────────────────────────────────────

function GateChip({ g }: { g: Gate }) {
  const ok = g.exit === 0;
  const c = ok ? "var(--color-live)" : "var(--color-rose)";
  return (
    <span
      className="inline-flex items-center gap-1 pl-1 pr-1.5 h-[18px] rounded-md text-[0.64rem] font-medium"
      style={{ color: c, background: `color-mix(in srgb, ${c} 12%, transparent)`, border: `1px solid color-mix(in srgb, ${c} 28%, transparent)` }}
      title={`${g.cmd} → exit ${g.exit} @ ${g.sha}`}
    >
      {ok ? <Check size={11} /> : <X size={11} />}
      <span>{g.label}</span>
      <span className="num opacity-70 flex items-center gap-0.5">
        <GitCommitHorizontal size={10} />{g.sha.slice(0, 6)}
      </span>
    </span>
  );
}

function ChallengeChip({ c }: { c: Challenge }) {
  const open = c.status === "open";
  const col = open ? "var(--color-working)" : "var(--color-faint)";
  return (
    <span
      className="inline-flex items-center gap-1 pl-1 pr-1.5 h-[18px] rounded-md text-[0.64rem] font-medium"
      style={{
        color: col,
        background: open ? "color-mix(in srgb, var(--color-working) 12%, transparent)" : "transparent",
        border: `1px solid color-mix(in srgb, ${col} ${open ? "30" : "22"}%, transparent)`,
      }}
      title={c.claim}
    >
      {open ? <Swords size={11} /> : <Gavel size={11} />}
      <span>{open ? "challenge" : "ruled"}</span>
      {open && c.deadlineMin != null && <span className="num opacity-70">{c.deadlineMin}m</span>}
    </span>
  );
}

function AgentPips({ t }: { t: Task }) {
  const owner = agents[t.ownerAgentId];
  const impl = t.implementerAgentId ? agents[t.implementerAgentId] : null;
  return (
    <div className="flex items-center gap-1.5" title={`owner ${owner.name}${impl ? ` · implementer ${impl.name}` : " · unclaimed"}`}>
      <span className={`av av-xs ${avClass[owner.color]}`}>{owner.initials}</span>
      <ArrowRight size={11} className="faint shrink-0" />
      {impl ? (
        <span className={`av av-xs ${avClass[impl.color]}`}>{impl.initials}</span>
      ) : (
        <span
          className="av av-xs"
          style={{ background: "transparent", color: "var(--color-faint)", border: "1px dashed var(--color-border)" }}
        >
          ·
        </span>
      )}
    </div>
  );
}

function Card({ t }: { t: Task }) {
  return (
    <div className="lane-card rounded-lg p-2.5 cursor-pointer">
      <div className="flex items-center gap-2 mb-1">
        <span className="mono text-[0.64rem] faint truncate">{t.slug}</span>
        {t.designCanon && (
          <span className="ml-auto inline-flex items-center gap-1 text-[0.6rem] faint shrink-0" title={`design canon · ${t.designCanon}`}>
            <Palette size={11} style={{ color: "color-mix(in srgb, var(--color-a-sky) 80%, var(--color-faint))" }} />
          </span>
        )}
      </div>

      <div className="text-[0.79rem] heading font-medium leading-snug mb-2 clamp-2">{t.title}</div>

      {(t.gates.length > 0 || t.challenges.length > 0) && (
        <div className="flex flex-wrap gap-1 mb-2">
          {t.gates.map((g, i) => <GateChip key={i} g={g} />)}
          {t.challenges.map((c, i) => <ChallengeChip key={i} c={c} />)}
        </div>
      )}

      <div className="flex items-center gap-2.5">
        <AgentPips t={t} />
        <span className="inline-flex items-center gap-1 text-[0.62rem] faint" title={t.fileBoundary.join("\n")}>
          <FileCode2 size={11} />{t.fileBoundary.length}
        </span>
        <span className="ml-auto text-[0.62rem] faint num">{t.updatedAt}</span>
      </div>
    </div>
  );
}

// ── Board ──────────────────────────────────────────────────────────────────

function Column({ state, label, accent }: (typeof COLUMNS)[number]) {
  const items = tasks.filter((t) => t.state === state);
  return (
    <section className="w-[266px] shrink-0 flex flex-col min-h-0">
      <div className="flex items-center gap-2 px-1.5 py-2 shrink-0">
        <span className="w-2 h-2 rounded-full shrink-0" style={{ background: accent }} />
        <span className="text-[0.74rem] font-semibold heading">{label}</span>
        <span className="num text-[0.64rem] faint">{items.length}</span>
      </div>
      <div className="flex-1 overflow-y-auto scroll-thin flex flex-col gap-2 pb-4 pr-0.5">
        {items.length === 0 ? (
          <div
            className="rounded-lg text-[0.68rem] faint px-3 py-4 text-center"
            style={{ border: "1px dashed var(--color-border)" }}
          >
            none
          </div>
        ) : (
          items.map((t) => <Card key={t.id} t={t} />)
        )}
      </div>
    </section>
  );
}

function HeaderPill({ children }: { children: React.ReactNode }) {
  return (
    <span
      className="inline-flex items-center gap-1.5 h-6 px-2 rounded-md text-[0.68rem] font-medium dim shrink-0"
      style={{ background: "var(--color-app)", border: "1px solid var(--color-border)" }}
    >
      {children}
    </span>
  );
}

export default function LaneBoard() {
  const total = tasks.length;
  const open = openChallenges();
  return (
    <div className="relative h-screen w-full overflow-hidden" style={{ background: "var(--color-app)" }}>
      {/* floating blurred header — mount template mirrors MemoryGraph.tsx
          (center pane, absolute top bar, onClose X returns to the agent pane) */}
      <div
        className="absolute top-0 left-0 right-0 h-12 z-20 flex items-center gap-3 px-4 border-b"
        style={{
          borderColor: "var(--color-border)",
          background: "color-mix(in srgb, var(--color-app) 78%, transparent)",
          backdropFilter: "blur(8px)",
        }}
      >
        <span className="w-6 h-6 rounded-[7px] grid place-items-center shrink-0" style={{ background: "var(--color-raised)", color: "var(--color-heading)", boxShadow: "inset 0 0 0 1px var(--color-border)" }}>
          <Columns3 size={13} />
        </span>
        <div className="leading-tight">
          <div className="text-[0.84rem] font-semibold tracking-tight heading">Lane board</div>
          <div className="text-[0.64rem] -mt-0.5 faint">codeup · agent work system</div>
        </div>
        <HeaderPill><Columns3 size={11} style={{ color: "var(--color-accent)" }} />{total} tasks</HeaderPill>
        {open > 0 && (
          <HeaderPill>
            <Swords size={11} style={{ color: "var(--color-working)" }} />
            <span style={{ color: "var(--color-working)" }}>{open} open</span>
          </HeaderPill>
        )}
        <div className="ml-auto flex items-center gap-2">
          <div className="flex items-center gap-2 rounded-md px-2.5 h-7" style={{ background: "var(--color-app)", border: "1px solid var(--color-border)" }}>
            <Search size={12} className="faint shrink-0" />
            <span className="text-[0.72rem] faint">Filter</span>
          </div>
          <button className="inline-flex items-center gap-1.5 rounded-md px-2.5 h-7 text-[0.72rem] dim" style={{ background: "var(--color-app)", border: "1px solid var(--color-border)" }}>
            <Filter size={12} /> All lanes
          </button>
          <button className="w-7 h-7 grid place-items-center rounded-md faint ctx-ibtn" title="Close board">
            <X size={15} />
          </button>
        </div>
      </div>

      {/* content sits below the floating header */}
      <div className="absolute inset-0 pt-12 flex flex-col min-h-0">
        {/* workspace telemetry strip */}
        <TelemetryStrip />

        {/* kanban board */}
        <div className="flex-1 min-h-0 overflow-x-auto scroll-thin">
          <div className="h-full flex gap-4 px-5 pt-3">
            {COLUMNS.map((c) => <Column key={c.state} {...c} />)}
          </div>
        </div>
      </div>
    </div>
  );
}
