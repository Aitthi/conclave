// Shared visual atoms for the Position System surfaces. Kept in one place so the
// roster card, the Builder pickers, the org chart and the escalation trace all
// speak ONE vocabulary, so a level reads the same everywhere it appears.
// Tokens only (theme.css), lucide icons, zero Arta imports.

import { Compass, ShieldCheck, Hammer, PenTool, Microscope, CornerUpRight, User } from "lucide-react";
import { LEVELS, MAX_RUNG, levelOf, type Track } from "../lib/positions";
import { agents, avClass } from "../lib/data";

// Track → the same icon the role picker uses, so track identity is stable
// across the whole app.
const TRACK_ICON: Record<Track, typeof Compass> = {
  Lead: Compass,
  Reviewer: ShieldCheck,
  Implementer: Hammer,
  Designer: PenTool,
  Researcher: Microscope,
};

export function TrackIcon({ track, size = 12, className }: { track: Track; size?: number; className?: string }) {
  const Icon = TRACK_ICON[track];
  return <Icon size={size} strokeWidth={2} className={className} />;
}

/* Level as a levels.fyi-style ladder: MAX_RUNG ascending bars, filled to the
   member's rung in accent, the rest quiet border. Reads seniority at a glance
   without a colour code to learn (taller filled stack = more senior). */
export function LevelRung({ rung, h = 11 }: { rung: number; h?: number }) {
  return (
    <span className="inline-flex items-end gap-[2px]" style={{ height: h }} aria-hidden>
      {Array.from({ length: MAX_RUNG }).map((_, i) => (
        <span
          key={i}
          style={{
            width: 2.5,
            height: `${(40 + (i / (MAX_RUNG - 1)) * 60)}%`,
            borderRadius: 1.5,
            background: i < rung ? "var(--color-accent)" : "var(--color-border)",
          }}
        />
      ))}
    </span>
  );
}

/* Level tag: rung ladder + level name. `compact` uses the short form for the
   tight 266px roster subtitle. `levelId` NULL = UNRANKED — the honest state for
   legacy members with no level set (backward-compat): empty ladder, faint word,
   never a fake "Junior". */
export function LevelTag({ levelId, compact = false }: { levelId: string | null; compact?: boolean }) {
  if (!levelId) {
    return (
      <span className="inline-flex items-center gap-1.5" title="Unranked — no level set">
        <LevelRung rung={0} h={compact ? 10 : 12} />
        <span
          className="font-medium leading-none italic"
          style={{ fontSize: compact ? "0.66rem" : "0.72rem", color: "var(--color-faint)" }}
        >
          Unranked
        </span>
      </span>
    );
  }
  const l = levelOf(levelId);
  return (
    <span
      className="inline-flex items-center gap-1.5"
      title={`${l.name} · level ${l.rung} of ${MAX_RUNG}`}
    >
      <LevelRung rung={l.rung} h={compact ? 10 : 12} />
      <span
        className="font-semibold leading-none"
        style={{ fontSize: compact ? "0.66rem" : "0.72rem", color: "var(--color-dim)" }}
      >
        {compact ? l.short : l.name}
      </span>
    </span>
  );
}

/* The human at the top of every chain. One consistent mark, the av-human chip
   (transparent, accent hairline), so "not an agent, it's the human" reads the
   same in the roster, the picker preview and the org crown. */
export function HumanChip({ label = true, size = "xs" }: { label?: boolean; size?: "xs" | "sm" }) {
  return (
    <span className="inline-flex items-center gap-1.5" title="The human — top of the chain">
      <span className={`av av-${size} av-human`}>
        <User size={size === "xs" ? 9 : 11} />
      </span>
      {label && <span className="text-[0.66rem] dim font-medium">Human</span>}
    </span>
  );
}

/* "Reports to X" as a quiet up-affordance for the roster subtitle: a corner-up
   arrow + the supervisor's avatar (or the human chip at the top of the chain). */
export function ReportsTo({ supervisorId }: { supervisorId: string | null }) {
  if (!supervisorId) {
    return (
      <span className="inline-flex items-center gap-1" title="Reports to the human (top of chain)">
        <CornerUpRight size={11} className="faint" />
        <span className="av av-xs av-human">
          <User size={9} />
        </span>
      </span>
    );
  }
  const s = agents[supervisorId];
  return (
    <span className="inline-flex items-center gap-1" title={`Reports to ${s.name}`}>
      <CornerUpRight size={11} className="faint" />
      <span className={`av av-xs ${avClass[s.color]}`}>{s.initials}</span>
    </span>
  );
}

export { LEVELS, levelOf, MAX_RUNG };
