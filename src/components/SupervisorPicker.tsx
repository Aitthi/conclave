import { useEffect, useState } from "react";
import { ArrowLeft, Ban, Check, CornerUpRight, UserPlus, Users, X } from "lucide-react";
import { TrackIcon } from "./Position";
import { levelOf } from "../lib/positions";

// ---------------------------------------------------------------------------
// SupervisorPicker — one modal, two entry points (plan default-level-
// supervisor-picker, D4/D5). Design canon: Arta proto @18149cc
// (.arta/proto/screens/supervisor-picker.tsx) pins the 3 states composition
// left open — add-flow step-2 chrome, the empty-members case, and the
// roster-edit cycle exclusion. Row/Human-row markup, footer model (NO Skip
// button — the Human row IS the no-supervisor choice), and copy are exact
// per that proto; do not restyle.
//
// Dumb about IPC (D4) — callers own the write. `submitting`/`error` are
// caller-driven so the add-flow's composite addToWorkspace→setPosition
// failure UX (D6) doesn't need to leak into this component's contract.
// ---------------------------------------------------------------------------

export interface SupervisorCandidate {
  id: string;
  name?: string;
  color?: string;
  level?: string;
  roleName?: string;
  /** "Claude · opus-5" — appended to the subtitle so the picker says which
   *  provider a candidate runs on (human request 2026-09-04). Built by the
   *  caller via `lib/providerLabel.providerChip`; absent when unknown. */
  providerChip?: string;
}

export interface SupervisorPickerSubject {
  name: string;
  color?: string;
  /** Caller-composed context line, e.g. "You're adding this agent to the
   *  workspace" (add) or "Currently reports to Detoro" / "Currently reports
   *  to the human" (edit) — this component doesn't resolve supervisor names. */
  sub: string;
}

export interface SupervisorPickerProps {
  subject: SupervisorPickerSubject;
  members: SupervisorCandidate[];
  /** Member ids that can't be picked (self + descendants — the engine's
   *  no-cycle rule). Shown disabled, not hidden, per canon. */
  excludeIds: string[];
  /** The member being edited (edit variant only) — tags its row "self"
   *  instead of "cycle" and pins it at the foot of the list (proto state C). */
  selfId?: string;
  /** The currently-set supervisor id, or null/undefined for "no supervisor". */
  current?: string | null;
  /** Called with the confirmed pick. `null` = no supervisor / reports to Human. */
  onPick: (supervisorId: string | null) => void;
  /** Full dismiss — backdrop click, Escape, header X, or footer Cancel. */
  onClose: () => void;
  variant: "add" | "edit";
  /** Add-flow only: renders a back arrow (returns to the agent list) instead
   *  of the header's default Users glyph. */
  onBack?: () => void;
  /** Add-flow only: the "Step 2 of 2" chip next to the title. */
  step?: string;
  /** Caller-driven: disables rows + the primary button while an async write
   *  is in flight. */
  submitting?: boolean;
  /** Caller-driven inline error banner (D6) — shown without closing the modal. */
  error?: string | null;
}

const TITLES: Record<SupervisorPickerProps["variant"], string> = {
  add: "Choose a supervisor",
  edit: "Change supervisor",
};

function Avatar({ name, color }: { name: string; color?: string }) {
  return (
    <span
      className="w-6 h-6 rounded-[7px] grid place-items-center shrink-0 text-[11px] font-bold text-white"
      style={{ backgroundColor: color ?? "#6e6e73" }}
    >
      {(name[0] ?? "?").toUpperCase()}
    </span>
  );
}

function HumanAvatar() {
  return (
    <span
      className="w-6 h-6 rounded-[7px] grid place-items-center shrink-0"
      style={{
        color: "var(--color-accent)",
        background: "color-mix(in srgb, var(--color-accent) 12%, transparent)",
        border: "1px solid color-mix(in srgb, var(--color-accent) 30%, transparent)",
      }}
    >
      <CornerUpRight className="w-3 h-3" />
    </span>
  );
}

function HumanRow({ selected, onSelect }: { selected: boolean; onSelect: () => void }) {
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-pressed={selected}
      className={`w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg text-left ${
        selected ? "bg-accent/[0.09] ring-1 ring-accent/40" : "hover:bg-overlay/[0.04]"
      }`}
    >
      <HumanAvatar />
      <span className="min-w-0 flex-1">
        <span className="block text-[12.5px] font-semibold leading-tight">Reports to the human</span>
        <span className="block text-[10.5px] text-text-tertiary leading-tight">
          Top of the chain (no supervisor)
        </span>
      </span>
      {selected && <Check className="w-3.5 h-3.5 text-accent shrink-0" />}
    </button>
  );
}

function CandidateRow({
  candidate,
  selected,
  disabled,
  tag,
  onSelect,
}: {
  candidate: SupervisorCandidate;
  selected: boolean;
  disabled: boolean;
  tag: "self" | "cycle";
  onSelect: () => void;
}) {
  const name = candidate.name ?? candidate.id;
  return (
    <button
      type="button"
      onClick={() => !disabled && onSelect()}
      disabled={disabled}
      aria-disabled={disabled}
      className={`w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg text-left ${
        disabled
          ? "opacity-45 cursor-not-allowed"
          : selected
            ? "bg-accent/[0.09] ring-1 ring-accent/40"
            : "hover:bg-overlay/[0.04]"
      }`}
    >
      <Avatar name={name} color={candidate.color} />
      <span className="min-w-0 flex-1">
        <span className="block text-[12.5px] font-semibold leading-tight truncate">{name}</span>
        <span className="flex items-center gap-1 text-[10.5px] text-text-tertiary leading-tight">
          <TrackIcon track={candidate.roleName} size={9} className="shrink-0" />
          <span className="truncate">
            {candidate.roleName ?? "Agent"} · {candidate.level ? levelOf(candidate.level).name : "Unranked"}
            {candidate.providerChip ? ` · ${candidate.providerChip}` : ""}
          </span>
        </span>
      </span>
      {disabled ? (
        <span className="inline-flex items-center gap-1 text-[10px] text-text-tertiary shrink-0">
          <Ban className="w-[11px] h-[11px]" /> {tag}
        </span>
      ) : selected ? (
        <Check className="w-3.5 h-3.5 text-accent shrink-0" />
      ) : null}
    </button>
  );
}

export function SupervisorPicker({
  subject,
  members,
  excludeIds,
  selfId,
  current,
  onPick,
  onClose,
  variant,
  onBack,
  step,
  submitting = false,
  error = null,
}: SupervisorPickerProps) {
  // Human-ruled footer (proto @3fd0f6e, supersedes @18149cc): the add variant
  // starts with NOTHING selected (`undefined`, distinct from `null` = the
  // Human row explicitly picked) so Skip and the row list read as two
  // separate intents rather than two defaults for one outcome. The edit
  // variant is unchanged — it always has a real current value to pre-select.
  const [draft, setDraft] = useState<string | null | undefined>(
    variant === "edit" ? (current ?? null) : undefined,
  );
  const nothingPicked = variant === "add" && draft === undefined;

  // Escape closes the whole modal (matches Settings.tsx / ArtifactView.tsx
  // convention) — not just a step-back, even in the add-flow's embedded step.
  useEffect(() => {
    function handleKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [onClose]);

  const excluded = new Set(excludeIds);
  // Self is pinned at the foot of the list (proto state C) — every other
  // candidate (including cycle-disabled descendants) sorts alphabetically
  // above it.
  const others = members
    .filter((m) => m.id !== selfId)
    .sort((left, right) => (left.name ?? left.id).localeCompare(right.name ?? right.id));
  const selfMember = selfId ? members.find((m) => m.id === selfId) : undefined;
  const orderedMembers = selfMember ? [...others, selfMember] : others;

  const helperText =
    variant === "edit"
      ? "Self and its reports are disabled — either would loop the reporting chain. Rejected client-side and, as a backstop, at write time."
      : members.length === 0
        ? "No other members yet. Nothing selected → Add is disabled; Skip adds now (reports to the human), or pick the row to commit it explicitly. The list is complete, not broken."
        : "Nothing is pre-selected — Add enables once a row is picked. Skip adds now with no supervisor (reports to the human); it's changeable later from the roster chip. No rows disabled: the new agent has no reports yet, so no pick can cycle.";

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/30"
      onClick={onClose}
    >
      <div
        className="w-[420px] max-h-[70vh] bg-surface rounded-2xl shadow-2xl flex flex-col overflow-hidden ring-1 ring-overlay/[0.08]"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="h-12 flex items-center gap-2.5 px-4 border-b border-overlay/[0.06] shrink-0">
          {onBack ? (
            <button
              type="button"
              onClick={onBack}
              className="w-7 h-7 -ml-1 grid place-items-center rounded-md hover:bg-overlay/[0.05] text-text-secondary shrink-0"
              aria-label="Back to agent list"
            >
              <ArrowLeft className="w-[15px] h-[15px]" />
            </button>
          ) : (
            <Users className="w-[15px] h-[15px] text-accent shrink-0" />
          )}
          <span className="text-[13px] font-semibold tracking-tight truncate">
            {TITLES[variant]}
          </span>
          {step && (
            <span className="text-[10px] font-medium text-text-tertiary px-1.5 py-0.5 rounded-md bg-overlay/[0.05] ring-1 ring-overlay/[0.08] shrink-0">
              {step}
            </span>
          )}
          <button
            onClick={onClose}
            className="ml-auto w-7 h-7 grid place-items-center rounded-md hover:bg-overlay/[0.05] text-text-secondary shrink-0"
            aria-label="Close"
          >
            <X className="w-[15px] h-[15px]" />
          </button>
        </div>

        <div className="flex items-center gap-2.5 px-4 py-3 border-b border-overlay/[0.06] shrink-0">
          <Avatar name={subject.name} color={subject.color} />
          <span className="min-w-0">
            <span className="block text-[12px] font-semibold leading-tight truncate">
              Supervisor for {subject.name}
            </span>
            <span className="block text-[10.5px] text-text-tertiary leading-tight">{subject.sub}</span>
          </span>
        </div>

        <div className="flex-1 overflow-y-auto scroll-thin p-2 min-h-0">
          <div className="rounded-xl p-1 bg-overlay/[0.02] ring-1 ring-overlay/[0.08]">
            <HumanRow selected={draft === null} onSelect={() => setDraft(null)} />
            {orderedMembers.length > 0 && (
              <div className="h-px my-1 mx-2 bg-overlay/[0.08]" />
            )}
            {orderedMembers.map((member) => (
              <CandidateRow
                key={member.id}
                candidate={member}
                selected={draft === member.id}
                disabled={submitting || excluded.has(member.id)}
                tag={member.id === selfId ? "self" : "cycle"}
                onSelect={() => setDraft(member.id)}
              />
            ))}
          </div>
          <p className="text-[10.5px] text-text-tertiary leading-snug mt-1.5 px-1">{helperText}</p>
        </div>

        {error && <div className="px-4 pt-2 text-[11px] text-danger shrink-0">{error}</div>}

        <div className="flex items-center gap-2 px-4 h-14 shrink-0 border-t border-overlay/[0.06]">
          {/* Add-flow footer is Skip + Add agent ONLY (canon @3fd0f6e, Arta
              design pass) — a Cancel ghost button next to Skip would be a
              misclick trap (same style, opposite effects: Cancel drops the
              whole add, Skip completes it with no supervisor). The X and
              back arrow already cover "abort" for that variant. Edit keeps
              Cancel + Confirm — there's no Skip to collide with there. */}
          {variant === "edit" && (
            <button
              type="button"
              onClick={onClose}
              disabled={submitting}
              className="text-[12.5px] font-medium text-text-secondary px-3 py-1.5 rounded-lg hover:bg-overlay/[0.05] disabled:opacity-50"
            >
              Cancel
            </button>
          )}
          {/* Remove supervisor — edit variant, current != null only (Arta
              proto @d2ac161, amends @3fd0f6e). One-click onPick(null), the
              edit-mode twin of add-flow Skip. Ban+rose (mapped to
              --color-danger per LaneBoard.tsx precedent) disambiguates it
              from the adjacent Cancel — disambiguation, not distance. */}
          {variant === "edit" && current != null && (
            <button
              type="button"
              onClick={() => onPick(null)}
              disabled={submitting}
              className="inline-flex items-center gap-1.5 text-[12.5px] font-medium text-text-secondary px-3 py-1.5 rounded-lg hover:bg-overlay/[0.05] disabled:opacity-50"
            >
              <Ban className="w-3 h-3" style={{ color: "var(--color-danger)" }} />
              Remove supervisor
            </button>
          )}
          {variant === "add" && (
            // Human-ruled (2d03b21a, proto @3fd0f6e): Skip adds with no
            // supervisor REGARDLESS of the current row selection — a bypass
            // distinct from picking the Human row then confirming.
            <button
              type="button"
              onClick={() => onPick(null)}
              disabled={submitting}
              className="text-[12.5px] font-medium text-text-secondary px-3 py-1.5 rounded-lg hover:bg-overlay/[0.05] disabled:opacity-50"
            >
              Skip
            </button>
          )}
          <button
            type="button"
            onClick={() => {
              if (draft !== undefined) onPick(draft);
            }}
            disabled={submitting || nothingPicked}
            className="ml-auto inline-flex items-center gap-1.5 text-[12.5px] font-semibold text-white bg-accent px-3.5 py-1.5 rounded-lg hover:brightness-105 disabled:opacity-40 disabled:cursor-not-allowed"
          >
            {variant === "add" ? (
              <>
                <UserPlus className="w-[13px] h-[13px]" />
                {submitting ? "Adding…" : "Add agent"}
              </>
            ) : (
              <>
                <Check className="w-[13px] h-[13px]" />
                {submitting ? "Saving…" : "Confirm"}
              </>
            )}
          </button>
        </div>
      </div>
    </div>
  );
}
