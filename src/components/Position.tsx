import {
  Compass,
  CornerUpRight,
  Hammer,
  Microscope,
  PenTool,
  ShieldCheck,
  User,
  UserPen,
  type LucideIcon,
} from "lucide-react";
import { MAX_RUNG, levelOf } from "../lib/positions";

export interface PositionPerson {
  name: string;
  color?: string;
  initials?: string;
}

function initialsOf(name: string): string {
  return name.trim().slice(0, 1).toUpperCase() || "?";
}

const TRACK_ICONS: Record<string, LucideIcon> = {
  lead: Compass,
  reviewer: ShieldCheck,
  implementer: Hammer,
  designer: PenTool,
  researcher: Microscope,
};

function iconForTrack(track?: string | null): LucideIcon {
  if (!track) return UserPen;
  return TRACK_ICONS[track.trim().toLowerCase()] ?? UserPen;
}

function PersonAvatar({
  person,
  size = "xs",
}: {
  person: PositionPerson;
  size?: "xs" | "sm";
}) {
  const initials = person.initials ?? initialsOf(person.name);
  const dimension = size === "xs" ? "w-4 h-4 rounded-[5px] text-[9px]" : "w-5 h-5 rounded-[6px] text-[10px]";
  return (
    <span
      className={`${dimension} grid place-items-center shrink-0 font-bold text-white`}
      style={{ background: person.color ?? "var(--color-accent)" }}
      title={person.name}
    >
      {initials}
    </span>
  );
}

export function TrackIcon({
  track,
  size = 12,
  className,
}: {
  track?: string | null;
  size?: number;
  className?: string;
}) {
  const Icon = iconForTrack(track);
  return <Icon size={size} strokeWidth={2} className={className} />;
}

export function LevelRung({ rung, h = 11 }: { rung: number; h?: number }) {
  return (
    <span className="inline-flex items-end gap-[2px]" style={{ height: h }} aria-hidden>
      {Array.from({ length: MAX_RUNG }).map((_, index) => (
        <span
          key={index}
          style={{
            width: 2.5,
            height: `${40 + (index / Math.max(1, MAX_RUNG - 1)) * 60}%`,
            borderRadius: 1.5,
            background:
              index < rung ? "var(--color-accent)" : "color-mix(in srgb, var(--color-overlay) 12%, transparent)",
          }}
        />
      ))}
    </span>
  );
}

export function LevelTag({
  levelId,
  compact = false,
}: {
  levelId?: string | null;
  compact?: boolean;
}) {
  if (!levelId) {
    return (
      <span className="inline-flex items-center gap-1.5" title="Unranked — no level set">
        <LevelRung rung={0} h={compact ? 10 : 12} />
        <span
          className="italic font-medium"
          style={{
            fontSize: compact ? "0.66rem" : "0.72rem",
            color: "var(--color-text-tertiary)",
            lineHeight: 1,
          }}
        >
          Unranked
        </span>
      </span>
    );
  }
  const level = levelOf(levelId);
  return (
    <span
      className="inline-flex items-center gap-1.5"
      title={`${level.name} · level ${level.rung} of ${MAX_RUNG}`}
    >
      <LevelRung rung={level.rung} h={compact ? 10 : 12} />
      <span
        className="font-semibold"
        style={{
          fontSize: compact ? "0.66rem" : "0.72rem",
          color: "var(--color-text-secondary)",
          lineHeight: 1,
        }}
      >
        {compact ? level.short : level.name}
      </span>
    </span>
  );
}

export function HumanChip({
  label = true,
  size = "xs",
}: {
  label?: boolean;
  size?: "xs" | "sm";
}) {
  const avatarSize = size === "xs" ? "w-4 h-4 rounded-[5px]" : "w-5 h-5 rounded-[6px]";
  const iconSize = size === "xs" ? 9 : 11;
  return (
    <span className="inline-flex items-center gap-1.5" title="The human — top of the chain">
      <span
        className={`${avatarSize} grid place-items-center shrink-0`}
        style={{
          color: "var(--color-accent)",
          background: "color-mix(in srgb, var(--color-accent) 12%, transparent)",
          border: "1px solid color-mix(in srgb, var(--color-accent) 30%, transparent)",
        }}
      >
        <User size={iconSize} />
      </span>
      {label && (
        <span className="text-[0.66rem] font-medium" style={{ color: "var(--color-text-secondary)" }}>
          Human
        </span>
      )}
    </span>
  );
}

export function ReportsTo({
  supervisor,
  label = false,
}: {
  supervisor?: PositionPerson | null;
  label?: boolean;
}) {
  if (!supervisor) {
    return (
      <span className="inline-flex items-center gap-1" title="Reports to Human">
        <CornerUpRight size={11} style={{ color: "var(--color-text-tertiary)" }} />
        <HumanChip label={label} size="xs" />
      </span>
    );
  }
  return (
    <span className="inline-flex items-center gap-1" title={`Reports to ${supervisor.name}`}>
      <CornerUpRight size={11} style={{ color: "var(--color-text-tertiary)" }} />
      <PersonAvatar person={supervisor} size="xs" />
      {label && (
        <span className="text-[0.66rem] font-medium" style={{ color: "var(--color-text-secondary)" }}>
          {supervisor.name}
        </span>
      )}
    </span>
  );
}

export function PositionLine({
  levelId,
  track,
  compact = false,
  supervisor,
  showReportsTo = false,
  trackTitle,
  className,
}: {
  levelId?: string | null;
  track?: string | null;
  compact?: boolean;
  supervisor?: PositionPerson | null;
  showReportsTo?: boolean;
  trackTitle?: string;
  className?: string;
}) {
  return (
    <div className={`flex items-center gap-1.5 min-w-0 ${className ?? ""}`}>
      <LevelTag levelId={levelId} compact={compact} />
      {track && (
        <>
          <span
            className="shrink-0"
            style={{ color: "var(--color-text-tertiary)", fontSize: compact ? "0.6rem" : "0.66rem" }}
          >
            ·
          </span>
          <span className="inline-flex items-center gap-1 min-w-0">
            <span className="shrink-0" style={{ color: "var(--color-text-tertiary)" }}>
              <TrackIcon track={track} size={compact ? 11 : 12} className="shrink-0" />
            </span>
            <span
              className="truncate"
              title={trackTitle}
              style={{
                fontSize: compact ? "0.66rem" : "0.72rem",
                color: "var(--color-text-secondary)",
                lineHeight: 1,
              }}
            >
              {track}
            </span>
          </span>
        </>
      )}
      {showReportsTo && (
        <span className="ml-auto shrink-0 pl-1">
          <ReportsTo supervisor={supervisor} />
        </span>
      )}
    </div>
  );
}
