import { useState } from "react";
import { Sparkles, X, Hammer, Check, CornerUpRight, Ban, ChevronRight } from "lucide-react";
import { agents, avClass } from "../lib/data";
import { LEVELS, positions, chainUp, wouldCycle, levelOf, type Track } from "../lib/positions";
import { LevelRung, TrackIcon, HumanChip } from "../components/Position";

export const meta = { title: "Edit agent · Position" };

/* Surface 2 of the Position System canon: the Level + Supervisor pickers inside
   the existing agent Builder, extending the role-picker language (ADR 0005),
   not inventing a second visual system.

   The section sits below Role and reads as its continuation:
     • TRACK is read-only. It follows the Role picked above (track = roleName),
       so the member never picks a track twice.
     • LEVEL is a 4-segment control in the same shape as the CLI-kind segmented
       control already in this modal, each segment carrying the rung ladder. A
       "Clear" affordance (mirrors "No role") sets it back to Unranked.
     • SUPERVISOR is a member list. Self and any descendant are disabled (a
       supervisor link that would form a cycle, rejected at write time), and
       "Reports to the human" is the explicit top-of-chain option (null).
     • CHAIN PREVIEW walks supervisor links up to the human, so the human sees
       the escalation path the choice produces before saving.

   Editing Tiësto (Implementer · Senior), who supervises Dew, so the cycle
   exclusion (Dew is disabled) is visible. */

const EDIT_ID = "tiesto";
const TRACK: Track = "Implementer";

// Workspace members that could be picked as a supervisor (everyone but the
// edited member), in roster order.
const CANDIDATES = ["detoro", "mellow", "arta", "guetta", "dew"];

function LevelSegment({
  levelId,
  selId,
  onPick,
}: {
  levelId: string;
  selId: string | null;
  onPick: () => void;
}) {
  const l = levelOf(levelId);
  const on = selId === levelId;
  return (
    <button
      onClick={onPick}
      aria-pressed={on}
      className={`flex-1 flex flex-col items-center gap-1 py-2 rounded-lg transition-colors ${
        on ? "heading" : "dim hover:text-[var(--color-text)]"
      }`}
      style={
        on
          ? { background: "var(--color-center)", boxShadow: "inset 0 0 0 1px color-mix(in srgb, var(--color-accent) 45%, var(--color-border))" }
          : undefined
      }
      title={`${l.name} · level ${l.rung} of ${LEVELS.length}`}
    >
      <LevelRung rung={l.rung} h={13} />
      <span className="text-[11.5px] font-semibold leading-none">{l.name}</span>
    </button>
  );
}

function SupervisorRow({
  id,
  selId,
  onPick,
}: {
  id: string;
  selId: string | null;
  onPick: () => void;
}) {
  const a = agents[id];
  const p = positions[id];
  const cyclic = wouldCycle(EDIT_ID, id);
  const on = selId === id;
  return (
    <button
      disabled={cyclic}
      onClick={onPick}
      aria-pressed={on}
      className={`w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg text-left transition-colors ${
        cyclic ? "opacity-45 cursor-not-allowed" : on ? "bg-accent/[0.09]" : "hover:bg-[var(--color-hover)]"
      }`}
      style={on && !cyclic ? { boxShadow: "inset 0 0 0 1px color-mix(in srgb, var(--color-accent) 40%, transparent)" } : undefined}
      title={cyclic ? "Would form a supervisor cycle" : `Reports to ${a.name}`}
    >
      <span className={`av av-sm ${avClass[a.color]}`}>{a.initials}</span>
      <span className="min-w-0 flex-1">
        <span className="block text-[12.5px] font-semibold heading leading-tight">{a.name}</span>
        <span className="flex items-center gap-1 text-[10.5px] dim leading-tight">
          <TrackIcon track={p.track} size={9} className="faint" />
          {p.track} · {p.levelId ? levelOf(p.levelId).name : "Unranked"}
        </span>
      </span>
      {cyclic ? (
        <span className="inline-flex items-center gap-1 text-[10px] faint shrink-0">
          <Ban size={11} /> cycle
        </span>
      ) : on ? (
        <Check size={14} className="text-accent shrink-0" />
      ) : null}
    </button>
  );
}

function ChainPreview({ supervisorId }: { supervisorId: string | null }) {
  // Chain from the edited member up to the human, using the *pending* supervisor.
  const upper = supervisorId ? chainUp(supervisorId) : [];
  const nodes = [EDIT_ID, ...upper];
  return (
    <div className="flex items-center gap-1 flex-wrap">
      {nodes.map((id, i) => {
        const a = agents[id];
        return (
          <span key={id} className="inline-flex items-center gap-1">
            {i > 0 && <ChevronRight size={12} className="faint shrink-0" />}
            <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded-md" style={{ background: "var(--color-app)", border: "1px solid var(--color-border)" }}>
              <span className={`av av-xs ${avClass[a.color]}`}>{a.initials}</span>
              <span className="text-[11px] heading font-medium">{a.name}</span>
            </span>
          </span>
        );
      })}
      <ChevronRight size={12} className="faint shrink-0" />
      <span className="px-1.5 py-0.5 rounded-md" style={{ background: "var(--color-app)", border: "1px solid var(--color-border)" }}>
        <HumanChip size="xs" />
      </span>
    </div>
  );
}

export default function BuilderPosition() {
  const [levelId, setLevelId] = useState<string | null>("senior");
  const [supId, setSupId] = useState<string | null>("detoro");

  return (
    <div className="min-h-screen w-full py-10 px-6 overflow-y-auto scroll-thin" style={{ background: "var(--color-app)" }}>
      <div style={{ maxWidth: 420, width: "100%", margin: "0 auto" }}>
        {/* caption */}
        <div className="mb-5">
          <h1 className="heading text-[1.02rem] font-semibold tracking-tight">Position, below the role picker</h1>
          <p className="dim text-[0.8rem] leading-relaxed mt-1">
            A new <span className="heading font-medium">Position</span> section continues the Role section in the same
            modal: track follows the role, level is a segmented control, and the supervisor picker previews the escalation
            chain it builds. Click a level or a supervisor and watch the chain update.
          </p>
        </div>

        {/* the Edit-agent modal */}
        <div className="rounded-2xl overflow-hidden" style={{ background: "var(--color-center)", border: "1px solid var(--color-border)", boxShadow: "var(--shadow-pop)" }}>
          {/* header */}
          <div className="flex items-center gap-2.5 px-4 h-12 border-b" style={{ borderColor: "var(--color-border)" }}>
            <Sparkles size={15} className="text-accent" />
            <span className="heading text-[0.9rem] font-semibold tracking-tight">Edit agent</span>
            <span className="text-[0.62rem] font-medium faint px-1.5 py-0.5 rounded-md" style={{ background: "var(--color-app)", border: "1px solid var(--color-border)" }}>
              workspace membership
            </span>
            <button className="ml-auto ctx-ibtn" aria-label="Close">
              <X size={15} />
            </button>
          </div>

          <div className="p-4 space-y-5">
            {/* Identity */}
            <section>
              <div className="label faint mb-2">Identity</div>
              <div className="flex items-center gap-3">
                <span className={`av av-lg ${avClass[agents[EDIT_ID].color]}`}>{agents[EDIT_ID].initials}</span>
                <div className="min-w-0">
                  <div className="heading text-[0.9rem] font-semibold tracking-tight">{agents[EDIT_ID].name}</div>
                  <div className="flex items-center gap-1.5 text-[0.72rem] dim">
                    <LevelRung rung={levelId ? levelOf(levelId).rung : 0} h={10} />
                    {levelId ? levelOf(levelId).name : "Unranked"} · {TRACK}
                  </div>
                </div>
              </div>
            </section>

            {/* Role recap: where track comes from */}
            <section>
              <div className="label faint mb-2">Role</div>
              <div className="flex items-center gap-2.5 rounded-xl p-2.5" style={{ background: "var(--color-app)", border: "1px solid var(--color-border)" }}>
                <span className="w-8 h-8 rounded-lg grid place-items-center shrink-0" style={{ background: "color-mix(in srgb, var(--color-accent) 10%, transparent)" }}>
                  <Hammer size={16} className="text-accent" />
                </span>
                <div className="min-w-0 flex-1">
                  <div className="text-[12.5px] font-semibold heading leading-tight">Implementer</div>
                  <div className="text-[11px] faint leading-snug">Picked in the Role cards above</div>
                </div>
                <span className="pill pill-accent">Role = track</span>
              </div>
            </section>

            {/* Position: the new section */}
            <section>
              <div className="flex items-center justify-between mb-2">
                <span className="label faint">Position</span>
              </div>

              {/* Track — read-only, follows Role */}
              <div className="flex items-center justify-between rounded-xl px-3 py-2 mb-2.5" style={{ background: "var(--color-app)", border: "1px solid var(--color-border)" }}>
                <span className="text-[11.5px] dim">Track</span>
                <span className="inline-flex items-center gap-1.5 text-[12px] heading font-medium">
                  <TrackIcon track={TRACK} size={13} className="text-accent" />
                  {TRACK}
                  <span className="text-[10px] faint font-normal ml-1">follows role</span>
                </span>
              </div>

              {/* Level — segmented control + Clear */}
              <div className="flex items-center justify-between mb-1.5">
                <span className="text-[11.5px] dim">Level</span>
                <button
                  onClick={() => setLevelId(null)}
                  className={`text-[11px] font-medium transition-colors ${levelId === null ? "text-accent" : "faint hover:text-[var(--color-text)]"}`}
                >
                  Clear (Unranked)
                </button>
              </div>
              <div className="flex gap-1 rounded-xl p-1 mb-4" style={{ background: "var(--color-app)" }}>
                {LEVELS.map((l) => (
                  <LevelSegment key={l.id} levelId={l.id} selId={levelId} onPick={() => setLevelId(l.id)} />
                ))}
              </div>

              {/* Supervisor — member list with cycle exclusion */}
              <div className="flex items-center justify-between mb-1.5">
                <span className="text-[11.5px] dim">Supervisor</span>
                <span className="text-[10.5px] faint">escalations route up to</span>
              </div>
              <div className="rounded-xl p-1 mb-1" style={{ background: "var(--color-app)", border: "1px solid var(--color-border)" }}>
                {/* human = null supervisor, top of chain */}
                <button
                  onClick={() => setSupId(null)}
                  aria-pressed={supId === null}
                  className={`w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg text-left transition-colors ${supId === null ? "bg-accent/[0.09]" : "hover:bg-[var(--color-hover)]"}`}
                  style={supId === null ? { boxShadow: "inset 0 0 0 1px color-mix(in srgb, var(--color-accent) 40%, transparent)" } : undefined}
                >
                  <span className="av av-sm av-human"><CornerUpRight size={12} /></span>
                  <span className="min-w-0 flex-1">
                    <span className="block text-[12.5px] font-semibold heading leading-tight">Reports to the human</span>
                    <span className="block text-[10.5px] dim leading-tight">Top of the chain (no supervisor)</span>
                  </span>
                  {supId === null && <Check size={14} className="text-accent shrink-0" />}
                </button>
                <div className="h-px my-1 mx-2" style={{ background: "var(--color-border-soft)" }} />
                {CANDIDATES.map((id) => (
                  <SupervisorRow key={id} id={id} selId={supId} onPick={() => setSupId(id)} />
                ))}
              </div>
              <div className="text-[10.5px] faint leading-snug mb-3 px-1">
                Dew is disabled: it already reports to Tiësto, so picking it would form a cycle (rejected at write time).
              </div>

              {/* Chain preview */}
              <div className="rounded-xl p-3" style={{ background: "var(--color-app)", border: "1px solid var(--color-border)" }}>
                <div className="label faint mb-2">Escalation chain</div>
                <ChainPreview supervisorId={supId} />
              </div>
            </section>
          </div>

          {/* footer */}
          <div className="flex items-center justify-end gap-2 px-4 h-14 border-t" style={{ borderColor: "var(--color-border)", background: "var(--color-sidebar)" }}>
            <button className="text-[12.5px] font-medium dim px-3 py-1.5 rounded-lg hover:bg-[var(--color-hover)]">Cancel</button>
            <button className="flex items-center gap-1.5 text-[12.5px] font-semibold px-3.5 py-1.5 rounded-lg" style={{ background: "var(--color-accent)", color: "var(--color-accent-ink)" }}>
              <Sparkles size={13} />
              Save changes
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
