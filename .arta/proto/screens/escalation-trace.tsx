import {
  X, Columns3, Swords, ArrowUp, Gavel, Clock, FileCode2, ArrowRight,
  ShieldQuestion, Scale,
} from "lucide-react";
import { agents, avClass } from "../lib/data";
import { positions, chainUp } from "../lib/positions";
import { LevelTag, TrackIcon, HumanChip } from "../components/Position";

export const meta = { title: "Task detail · Escalation trace" };

/* Surface 4 of the Position System canon: when a challenge routes up the
   supervisor chain, how the human SEES that path. It lives on the Lane board
   task detail (opened from a card), so it reuses the board's challenge
   vocabulary (Swords = open, Gavel = ruled) and the position atoms.

   The trace is a vertical stepper in escalation order: the filer at the top,
   then each supervisor the challenge climbs to, then the human as the final
   tiebreaker. The step currently expected to rule is highlighted; a footer
   states what happens if the deadline passes unruled (the filer's stated
   default fires) and where a cross-chain dispute settles (the lowest common
   ancestor, ultimately the human).

   Modelled on task aws-a: filed by Dew (Implementer · Mid), whose chain is
   Dew → Tiësto (sub-lead) → Detoro (lead) → Human. */

const FILER = "dew";
const RULER = "detoro"; // the lead / task owner expected to rule
const DEADLINE_MIN = 42;

const CHALLENGE = {
  claim: "State CHECK omits a 'blocked' state — sqlite can't ALTER it in later",
  evidence: "0012_task_system.sql:14 · CHECK(state IN (…)) has no 'blocked'",
  proposal: "Add 'blocked' to the CHECK now, before any rows exist",
  default: "Ship without 'blocked'; add a follow-up migration if it's ever needed",
};

type StepKind = "filed" | "through" | "ruling" | "tiebreaker";

function StatusChip({ kind }: { kind: StepKind }) {
  const map = {
    filed: { label: "Filed", Icon: Swords, color: "var(--color-working)" },
    through: { label: "Routes through", Icon: ArrowUp, color: "var(--color-faint)" },
    ruling: { label: "Expected to rule", Icon: Gavel, color: "var(--color-accent)" },
    tiebreaker: { label: "Tiebreaker", Icon: Scale, color: "var(--color-faint)" },
  }[kind];
  const { Icon } = map;
  return (
    <span
      className="inline-flex items-center gap-1 pl-1 pr-1.5 h-[19px] rounded-md text-[0.64rem] font-semibold shrink-0"
      style={{
        color: map.color,
        background: `color-mix(in srgb, ${map.color} 12%, transparent)`,
        border: `1px solid color-mix(in srgb, ${map.color} 30%, transparent)`,
      }}
    >
      <Icon size={11} /> {map.label}
    </span>
  );
}

function Step({
  kind,
  id,
  human,
  connectorAbove,
  ruling,
}: {
  kind: StepKind;
  id?: string;
  human?: boolean;
  connectorAbove: boolean;
  ruling?: boolean;
}) {
  return (
    <div className="relative pl-7">
      {/* rail + up-arrow connector to the step above */}
      {connectorAbove && (
        <>
          <span className="absolute left-[13px] -top-2 h-2 w-px" style={{ background: "color-mix(in srgb, var(--color-faint) 55%, transparent)" }} />
          <span className="absolute left-[7px] -top-[1px] grid place-items-center" title="escalates up to">
            <ArrowUp size={13} className="faint" />
          </span>
        </>
      )}
      <div
        className="lane-card rounded-lg px-2.5 py-2 flex items-center gap-2.5"
        style={ruling ? { borderColor: "color-mix(in srgb, var(--color-accent) 45%, var(--color-border))", background: "color-mix(in srgb, var(--color-accent) 6%, var(--color-raised))" } : undefined}
      >
        {human ? (
          <span className="av av-md av-human"><Scale size={13} /></span>
        ) : (
          <span className={`av av-md ${avClass[agents[id!].color]}`}>{agents[id!].initials}</span>
        )}
        <div className="min-w-0 flex-1">
          <div className="text-[0.82rem] font-semibold heading truncate">{human ? "Human" : agents[id!].name}</div>
          {human ? (
            <div className="text-[0.66rem] dim">Top of the chain · settles cross-chain disputes</div>
          ) : (
            <div className="flex items-center gap-1.5 mt-0.5">
              <LevelTag levelId={positions[id!].levelId} compact />
              <span className="faint text-[0.6rem]">·</span>
              <TrackIcon track={positions[id!].track} size={11} className="faint" />
              <span className="text-[0.66rem] dim">{positions[id!].track}</span>
            </div>
          )}
        </div>
        <StatusChip kind={kind} />
      </div>
    </div>
  );
}

export default function EscalationTrace() {
  // Chain from the filer up to the lead, then the human tiebreaker on top.
  const chain = chainUp(FILER); // [dew, tiesto, detoro]
  const owner = agents[RULER];
  const impl = agents[FILER];

  return (
    <div className="relative h-screen w-full overflow-hidden" style={{ background: "var(--color-app)" }}>
      {/* floating blurred header — same chrome as the Lane board */}
      <div
        className="absolute top-0 left-0 right-0 h-12 z-20 flex items-center gap-3 px-4 border-b"
        style={{ borderColor: "var(--color-border)", background: "color-mix(in srgb, var(--color-app) 78%, transparent)", backdropFilter: "blur(8px)" }}
      >
        <span className="w-6 h-6 rounded-[7px] grid place-items-center shrink-0" style={{ background: "var(--color-raised)", color: "var(--color-heading)", boxShadow: "inset 0 0 0 1px var(--color-border)" }}>
          <Columns3 size={13} />
        </span>
        <div className="leading-tight">
          <div className="text-[0.84rem] font-semibold tracking-tight heading">Lane board</div>
          <div className="text-[0.64rem] -mt-0.5 faint">codeup · task detail</div>
        </div>
        <span className="ml-2 inline-flex items-center gap-1.5 h-6 px-2 rounded-md text-[0.68rem] font-medium shrink-0" style={{ background: "var(--color-app)", border: "1px solid var(--color-border)", color: "var(--color-working)" }}>
          <Swords size={11} /> 1 open challenge
        </span>
        <button className="ml-auto w-7 h-7 grid place-items-center rounded-md faint ctx-ibtn" title="Close detail">
          <X size={15} />
        </button>
      </div>

      {/* content */}
      <div className="absolute inset-0 pt-12 overflow-y-auto scroll-thin">
        <div className="max-w-[560px] mx-auto px-6 py-6">
          {/* task header */}
          <div className="flex items-center gap-2 mb-1">
            <span className="mono text-[0.66rem] faint">aws-a</span>
            <span className="inline-flex items-center gap-1 text-[0.62rem] font-medium px-1.5 py-0.5 rounded-md" style={{ color: "var(--color-working)", background: "color-mix(in srgb, var(--color-working) 12%, transparent)", border: "1px solid color-mix(in srgb, var(--color-working) 28%, transparent)" }}>
              in progress
            </span>
            <span className="ml-auto inline-flex items-center gap-1.5 text-[0.64rem] faint" title={`owner ${owner.name} · implementer ${impl.name}`}>
              <span className={`av av-xs ${avClass[owner.color]}`}>{owner.initials}</span>
              <ArrowRight size={11} className="faint" />
              <span className={`av av-xs ${avClass[impl.color]}`}>{impl.initials}</span>
            </span>
          </div>
          <h1 className="heading text-[0.98rem] font-semibold tracking-tight leading-snug">
            Task core — tables, repo, task.* commands, CLI verbs, gate runner
          </h1>

          {/* open challenge */}
          <div className="mt-4 rounded-xl p-3.5" style={{ background: "var(--color-sidebar)", border: "1px solid color-mix(in srgb, var(--color-working) 26%, var(--color-border))" }}>
            <div className="flex items-center gap-2 mb-2.5">
              <Swords size={14} style={{ color: "var(--color-working)" }} />
              <span className="text-[0.8rem] font-semibold heading">Open challenge</span>
              <span className="ml-auto inline-flex items-center gap-1 text-[0.66rem] num" style={{ color: "var(--color-working)" }}>
                <Clock size={11} /> {DEADLINE_MIN}m to default
              </span>
            </div>
            <p className="text-[0.8rem] heading leading-snug mb-2.5">{CHALLENGE.claim}</p>
            <div className="grid grid-cols-1 gap-2">
              <Field icon={<FileCode2 size={12} />} label="Evidence" value={CHALLENGE.evidence} mono />
              <Field icon={<ShieldQuestion size={12} />} label="Proposal" value={CHALLENGE.proposal} />
              <Field icon={<Gavel size={12} />} label="Default if unruled" value={CHALLENGE.default} />
            </div>
          </div>

          {/* escalation trace */}
          <div className="mt-5">
            <div className="flex items-center gap-2 mb-3">
              <span className="label faint" style={{ paddingBottom: 0 }}>Escalation trace</span>
              <span className="text-[0.66rem] faint">routes up the supervisor chain</span>
            </div>

            <div className="flex flex-col gap-3">
              {/* filer */}
              <Step kind="filed" id={chain[0]} connectorAbove={false} />
              {/* middle supervisors the challenge climbs through */}
              {chain.slice(1, -1).map((id) => (
                <Step key={id} kind="through" id={id} connectorAbove />
              ))}
              {/* the lead / owner expected to rule */}
              <Step kind="ruling" id={chain[chain.length - 1]} connectorAbove ruling />
              {/* human tiebreaker */}
              <Step kind="tiebreaker" human connectorAbove />
            </div>

            {/* what happens next */}
            <div className="mt-4 rounded-xl p-3 text-[0.72rem] leading-relaxed" style={{ background: "var(--color-app)", border: "1px dashed var(--color-border)" }}>
              <p className="dim">
                <span className="heading font-medium">If unruled in {DEADLINE_MIN}m,</span> the filer's stated default
                applies automatically and both parties are notified — the loop can't silently stall.
              </p>
              <p className="dim mt-1.5 flex items-center gap-1.5 flex-wrap">
                <span className="heading font-medium">Cross-chain disputes</span> settle at the lowest common supervisor;
                if there is none, the <HumanChip label /> rules.
              </p>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

function Field({
  icon,
  label,
  value,
  mono,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <div className="flex items-start gap-2">
      <span className="w-4 grid place-items-center pt-0.5 faint shrink-0">{icon}</span>
      <div className="min-w-0">
        <div className="label faint" style={{ paddingBottom: 0, fontSize: "0.6rem" }}>{label}</div>
        <div className={`text-[0.74rem] dim leading-snug ${mono ? "mono" : ""}`}>{value}</div>
      </div>
    </div>
  );
}
