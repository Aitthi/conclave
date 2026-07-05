import { ArrowLeft, X, Check, Ban, CornerUpRight, UserPlus, Users } from "lucide-react";
import { agents, avClass } from "../lib/data";
import { positions, wouldCycle, levelOf } from "../lib/positions";
import { TrackIcon } from "../components/Position";

export const meta = { title: "Supervisor picker · states" };

/* Lane C canon (supervisor-picker-ui): the SupervisorPicker modal, ONE
   component with two entry points (D4/D5). Ruled canon-by-composition by the
   lead. The modal FRAME is AddAgentPicker's real shell (Roster.tsx:778,
   w-[420px] max-h-[70vh] rounded-2xl bg-surface ring-1, h-12 header, X). The
   BODY is the supervisor-row list already pinned in builder-position.tsx
   (@b2ac739): a "Reports to the human" row, member rows (avatar, name,
   track·level), select = fill highlight + Check, cycle-disabled = opacity-45
   + Ban "cycle".

   This board pins ONLY the three states composition does NOT already cover, so
   Dew builds them to target instead of improvising:

     A · add-flow, step 2. The NEW step-2 chrome the AddAgentPicker shell has no
        version of: a back arrow to the agent list, the retitled header plus
        "Step 2 of 2", the identity strip (who am I picking a supervisor FOR),
        and a footer (AddAgentPicker rows are instant-add, no footer). Human is
        the default selection; no rows are cycle-disabled (the new agent isn't
        in the workspace yet, so it has no descendants).
     B · add-flow, first agent. The empty-members case: adding the FIRST agent,
        the list is ONLY the "Reports to the human" row. It must read as
        COMPLETE, not broken; a helper line says so and Add stays enabled.
     C · roster edit. Chip entry (variant="edit"): single-purpose, no back arrow,
        no step, header "Change supervisor", identity names the current
        supervisor. Self and descendants are cycle-disabled (editing Tiësto
        disables Tiësto[self] and Dew[descendant], per wouldCycle); a member is
        selected to show the fill + Check state.

   FOOTER MODEL (human-ruled, 2d03b21a): the human overruled the no-Skip
   resolution — a visible Skip button is required in the add-flow ("supervisor
   isn't required; give me a Skip"). Human wish outranks the redundancy
   argument; the rest of the challenge ruling stands. Kept coherent by NOT
   pre-selecting the Human row: nothing is selected initially, so Skip and the
   list read as two distinct intents rather than two defaults for one outcome.
     · Skip (secondary, left) = add now with no supervisor (reports to human);
       the bypass for "decide later" — changeable from the roster chip (panel C).
     · Add agent (primary, right) = commit the picked supervisor; DISABLED until
       a row is selected, so it never duplicates Skip.
     · X / back arrow = abort the whole add.
   The Human row stays PRESENT (ruling upheld) as an explicit positive choice,
   just no longer the pre-selection — that is the one amendment vs the prior
   canon. The edit variant (C) has NO Skip (you're changing an existing link,
   not adding) and keeps its current supervisor pre-selected. Its footer is
   Cancel + Confirm PLUS a "Remove supervisor" action in the left slot — an
   AMENDMENT to canon @3fd0f6e, ruled by Arta 2026-07-05 after the human could
   not discover how to take a supervisor out ("Change forces a pick — no way to
   take the supervisor out"). Remove is rendered ONLY when the subject currently
   HAS a supervisor (current != null); it is a ONE-CLICK write of onPick(null) —
   the SAME gesture as add-flow Skip, and symmetric in meaning: "no supervisor,
   one click, reversible from the chip later." That symmetry is deliberate: the
   human anchors on Skip, so Remove is its edit-mode twin, not a new model. The
   Cancel-adjacency misclick (the 99ffd9ed lesson) is defused by DISAMBIGUATION,
   not distance — Remove carries a Ban icon in the rose hue so it never reads as
   a second Cancel, and a misclick is cheap because reports-to-human is
   reversible, not data loss. Order: Cancel · Remove supervisor (Cancel at the
   edge). When current == null the button is HIDDEN (panel D): the agent already
   reports to the human, so there is nothing to remove; the Human row already
   carries that state. The Human row stays unchanged in both cases.

   The row markup below is COPIED from builder-position.tsx's SupervisorRow so
   the proto literally IS the composition: reuse it, do not restyle. Frame
   fidelity target is AddAgentPicker's actual classes, not this token re-skin;
   this board pins layout, states, and copy. Zero host imports, theme.css only. */

// ── the row canon (verbatim from builder-position.tsx) ──────────────────────

function SupervisorRow({
  id,
  editId,
  selId,
}: {
  id: string;
  editId: string | null; // the member being edited (add-flow: null → nothing to exclude)
  selId: string | null;
}) {
  const a = agents[id];
  const p = positions[id];
  const cyclic = editId ? id === editId || wouldCycle(editId, id) : false;
  const on = selId === id;
  return (
    <div
      aria-disabled={cyclic}
      className={`w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg text-left ${
        cyclic ? "opacity-45 cursor-not-allowed" : on ? "bg-accent/[0.09]" : "hover:bg-[var(--color-hover)]"
      }`}
      style={on && !cyclic ? { boxShadow: "inset 0 0 0 1px color-mix(in srgb, var(--color-accent) 40%, transparent)" } : undefined}
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
          <Ban size={11} /> {id === editId ? "self" : "cycle"}
        </span>
      ) : on ? (
        <Check size={14} className="text-accent shrink-0" />
      ) : null}
    </div>
  );
}

function HumanRow({ selId }: { selId: string | null }) {
  const on = selId === null;
  return (
    <div
      aria-pressed={on}
      className={`w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg text-left ${
        on ? "bg-accent/[0.09]" : "hover:bg-[var(--color-hover)]"
      }`}
      style={on ? { boxShadow: "inset 0 0 0 1px color-mix(in srgb, var(--color-accent) 40%, transparent)" } : undefined}
    >
      <span className="av av-sm av-human"><CornerUpRight size={12} /></span>
      <span className="min-w-0 flex-1">
        <span className="block text-[12.5px] font-semibold heading leading-tight">Reports to the human</span>
        <span className="block text-[10.5px] dim leading-tight">Top of the chain (no supervisor)</span>
      </span>
      {on && <Check size={14} className="text-accent shrink-0" />}
    </div>
  );
}

// ── the modal frame (= AddAgentPicker shell, Roster.tsx:778) ─────────────────

function Modal({
  title,
  onBack,
  step,
  children,
  footer,
}: {
  title: string;
  onBack?: boolean;
  step?: string;
  children: React.ReactNode;
  footer: React.ReactNode;
}) {
  return (
    <div className="rounded-2xl overflow-hidden flex flex-col" style={{ width: 420, background: "var(--color-center)", border: "1px solid var(--color-border)", boxShadow: "var(--shadow-pop)" }}>
      {/* header — AddAgentPicker's h-12; step 2 gains a back arrow + step chip */}
      <div className="h-12 shrink-0 flex items-center gap-2.5 px-4 border-b" style={{ borderColor: "var(--color-border)" }}>
        {onBack ? (
          <button className="ctx-ibtn -ml-1" aria-label="Back to agent list"><ArrowLeft size={15} /></button>
        ) : (
          <Users size={15} className="text-accent" />
        )}
        <span className="heading text-[0.9rem] font-semibold tracking-tight">{title}</span>
        {step && (
          <span className="text-[0.62rem] font-medium faint px-1.5 py-0.5 rounded-md" style={{ background: "var(--color-app)", border: "1px solid var(--color-border)" }}>
            {step}
          </span>
        )}
        <button className="ml-auto ctx-ibtn" aria-label="Close"><X size={15} /></button>
      </div>
      {children}
      {/* footer — AddAgentPicker has none; the picker step adds one */}
      <div className="flex items-center gap-2 px-4 h-14 shrink-0 border-t" style={{ borderColor: "var(--color-border)", background: "var(--color-sidebar)" }}>
        {footer}
      </div>
    </div>
  );
}

/* Identity strip — "who am I picking a supervisor for", the context the compact
   picker step needs (builder-position carries it as a full Identity section). */
function Subject({ id, sub }: { id: string; sub: React.ReactNode }) {
  const a = agents[id];
  return (
    <div className="flex items-center gap-2.5 px-4 py-3 border-b" style={{ borderColor: "var(--color-border-soft)" }}>
      <span className={`av av-sm ${avClass[a.color]}`}>{a.initials}</span>
      <span className="min-w-0">
        <span className="block text-[12px] heading font-semibold leading-tight">Supervisor for {a.name}</span>
        <span className="block text-[10.5px] dim leading-tight">{sub}</span>
      </span>
    </div>
  );
}

function Btn({ kind, children, disabled }: { kind: "ghost" | "accent"; children: React.ReactNode; disabled?: boolean }) {
  if (kind === "accent")
    return (
      <button
        disabled={disabled}
        className={`inline-flex items-center gap-1.5 text-[12.5px] font-semibold px-3.5 py-1.5 rounded-lg ${disabled ? "opacity-40 cursor-not-allowed" : ""}`}
        style={{ background: "var(--color-accent)", color: "var(--color-accent-ink)" }}
      >
        {children}
      </button>
    );
  return <button className="text-[12.5px] font-medium dim px-3 py-1.5 rounded-lg hover:bg-[var(--color-hover)]">{children}</button>;
}

export default function SupervisorPickerStates() {
  // add-flow candidates = current members (Guetta is being added, not yet present)
  const ADD_MEMBERS = ["detoro", "mellow", "tiesto", "dew", "arta"];
  // edit candidates = everyone but the subject; editing Tiësto shows self + descendant disabled
  const EDIT_MEMBERS = ["detoro", "mellow", "dew", "arta", "guetta"];

  return (
    <div className="min-h-screen w-full px-8 py-8" style={{ background: "var(--color-app)" }}>
      <div className="max-w-[1360px] mx-auto">
        <h1 className="text-[1.15rem] font-semibold heading tracking-tight">Supervisor picker — the states composition doesn't pin</h1>
        <p className="mt-1 text-[0.84rem] dim max-w-[860px] leading-relaxed">
          One modal, two entry points. The frame is{" "}
          <span className="mono text-[0.8rem] heading">AddAgentPicker</span>'s shell, the list is the{" "}
          <span className="mono text-[0.8rem] heading">builder-position</span> supervisor rows. These three panels pin what
          composition leaves open: the <span className="heading">step-2 chrome</span> (back, step, identity, footer), the{" "}
          <span className="heading">empty-members</span> case, and the <span className="heading">edit</span> variant's cycle
          exclusion. Everything else is reused verbatim.
        </p>

        <div className="flex flex-wrap gap-10 mt-8 items-start">
          {/* A — add flow, step 2, members present */}
          <div className="flex flex-col">
            <span className="label faint mb-2">A · Add flow — pick supervisor (members present)</span>
            <Modal
              title="Choose a supervisor"
              onBack
              step="Step 2 of 2"
              footer={
                <>
                  <Btn kind="ghost">Skip</Btn>
                  <span className="ml-auto" />
                  <Btn kind="accent"><UserPlus size={13} /> Add agent</Btn>
                </>
              }
            >
              <Subject id="guetta" sub="You're adding this agent to the workspace" />
              <div className="p-2 flex-1 min-h-0 overflow-y-auto scroll-thin" style={{ maxHeight: 260 }}>
                <div className="rounded-xl p-1" style={{ background: "var(--color-app)", border: "1px solid var(--color-border)" }}>
                  <HumanRow selId="tiesto" />
                  <div className="h-px my-1 mx-2" style={{ background: "var(--color-border-soft)" }} />
                  {ADD_MEMBERS.map((id) => (
                    <SupervisorRow key={id} id={id} editId={null} selId="tiesto" />
                  ))}
                </div>
                <p className="text-[10.5px] faint leading-snug mt-1.5 px-1">
                  Nothing is pre-selected — Add enables once a row is picked (Tiësto here). Skip adds now with no supervisor
                  (reports to the human); it's changeable later from the roster chip. No rows disabled: the new agent has no
                  reports yet, so no pick can cycle.
                </p>
              </div>
            </Modal>
          </div>

          {/* B — add flow, first agent, empty workspace */}
          <div className="flex flex-col">
            <span className="label faint mb-2">B · Add flow — first agent (no other members)</span>
            <Modal
              title="Choose a supervisor"
              onBack
              step="Step 2 of 2"
              footer={
                <>
                  <Btn kind="ghost">Skip</Btn>
                  <span className="ml-auto" />
                  <Btn kind="accent" disabled><UserPlus size={13} /> Add agent</Btn>
                </>
              }
            >
              <Subject id="guetta" sub="You're adding this agent to the workspace" />
              <div className="p-2">
                <div className="rounded-xl p-1" style={{ background: "var(--color-app)", border: "1px solid var(--color-border)" }}>
                  <HumanRow selId="none" />
                </div>
                <p className="text-[10.5px] faint leading-snug mt-1.5 px-1">
                  No other members yet. Nothing selected → Add is disabled; Skip adds now (reports to the human), or pick the
                  row to commit it explicitly. The list is complete, not broken.
                </p>
              </div>
            </Modal>
          </div>

          {/* C — roster edit, chip entry, cycle exclusion */}
          <div className="flex flex-col">
            <span className="label faint mb-2">C · Roster chip — change supervisor (cycle-disabled)</span>
            <Modal
              title="Change supervisor"
              footer={
                <>
                  <Btn kind="ghost">Cancel</Btn>
                  {/* Remove supervisor — edit variant, current != null only.
                      One-click onPick(null), Skip's edit-mode twin. Ban+rose
                      disambiguates it from Cancel (the misclick guard). */}
                  <button className="inline-flex items-center gap-1.5 text-[12.5px] font-medium dim px-3 py-1.5 rounded-lg hover:bg-[var(--color-hover)]">
                    <Ban size={12} style={{ color: "var(--color-rose)" }} /> Remove supervisor
                  </button>
                  <span className="ml-auto" />
                  <Btn kind="accent"><Check size={13} /> Confirm</Btn>
                </>
              }
            >
              <Subject id="tiesto" sub="Currently reports to Detoro" />
              <div className="p-2 flex-1 min-h-0 overflow-y-auto scroll-thin" style={{ maxHeight: 260 }}>
                <div className="rounded-xl p-1" style={{ background: "var(--color-app)", border: "1px solid var(--color-border)" }}>
                  <HumanRow selId="detoro" />
                  <div className="h-px my-1 mx-2" style={{ background: "var(--color-border-soft)" }} />
                  {EDIT_MEMBERS.map((id) => (
                    <SupervisorRow key={id} id={id} editId="tiesto" selId="detoro" />
                  ))}
                  {/* self, rendered disabled at the foot to show the "self" tag */}
                  <SupervisorRow id="tiesto" editId="tiesto" selId="detoro" />
                </div>
                <p className="text-[10.5px] faint leading-snug mt-1.5 px-1">
                  Tiësto (self) and Dew (its report) are disabled — either would loop the chain. Rejected client-side and,
                  as a backstop, at write time. Footer gains <span className="heading">Remove supervisor</span> (has a
                  supervisor → shown): one click writes <span className="mono">onPick(null)</span>, the edit-mode twin of
                  add-flow Skip. Ban + rose keeps it from reading as a second Cancel.
                </p>
              </div>
            </Modal>
          </div>

          {/* D — roster edit, subject already reports to human: no Remove button */}
          <div className="flex flex-col">
            <span className="label faint mb-2">D · Roster chip — already reports to human (no Remove)</span>
            <Modal
              title="Change supervisor"
              footer={
                <>
                  <Btn kind="ghost">Cancel</Btn>
                  {/* current == null → Remove supervisor is HIDDEN; nothing to remove */}
                  <span className="ml-auto" />
                  <Btn kind="accent"><Check size={13} /> Confirm</Btn>
                </>
              }
            >
              <Subject id="mellow" sub="Currently reports to the human" />
              <div className="p-2 flex-1 min-h-0 overflow-y-auto scroll-thin" style={{ maxHeight: 260 }}>
                <div className="rounded-xl p-1" style={{ background: "var(--color-app)", border: "1px solid var(--color-border)" }}>
                  <HumanRow selId={null} />
                  <div className="h-px my-1 mx-2" style={{ background: "var(--color-border-soft)" }} />
                  {["detoro", "tiesto", "dew", "arta", "guetta"].map((id) => (
                    <SupervisorRow key={id} id={id} editId="mellow" selId={null} />
                  ))}
                </div>
                <p className="text-[10.5px] faint leading-snug mt-1.5 px-1">
                  Subject already reports to the human (current == null), so <span className="heading">Remove supervisor</span>
                  {" "}is not rendered — the Human row already carries that state and is pre-selected. Footer is Cancel +
                  Confirm, unchanged. Pick a member and Confirm to give it a supervisor.
                </p>
              </div>
            </Modal>
          </div>
        </div>
      </div>
    </div>
  );
}
