import { useState } from "react";
import {
  AlertCircle,
  Check,
  ChevronDown,
  CircleDashed,
  Folder,
  LoaderCircle,
  MessageSquare,
  Plus,
  RotateCcw,
  Search,
  Settings2,
  Sparkles,
  Terminal,
  UserPen,
  Users,
  Waypoints,
  X,
} from "lucide-react";

export const meta = { title: "AI drafter — agent and team" };

/*
  CANON: AgentDrafter overlay + entry points (Lane B / task drafter-ui-canon).
  Spec: docs/superpowers/specs/2026-09-04-ai-agent-team-drafter-design.md
  Implemented by: src/components/AgentDrafter.tsx, Library.tsx, Roster.tsx, Builder.tsx.

  Copy deck. Use these strings verbatim (all UI copy English, D11):

  Entry buttons   "Draft with AI"            (Library, beside "New agent")
                  "Build team with AI"       (Roster, beside "Add agent")
  Overlay titles  "Draft an agent" (agent mode) · "Build a team" (team mode)
  Header chips    "opens in Builder" (agent) · "applies to codeup" (team)
  Field labels    "Brief" · "Drafter" · "Notes from the drafter"
  Placeholder     "Describe the job. Example: a team to port our billing service
                   from Node to Rust with tests and a reviewer."
  Buttons         "Cancel" · "Draft" · "Drafting…" · "Retry" · "Back"
                  "Apply N agents" · "Applying…" · "Done" · "Open Builder"
                  "Back to preview" · "Create agent"
  Waiting caption "Closing this panel does not stop the run."
  Timeout note    "The drafter answers within 120 s or times out."

  Error strings (spec "Error handling" table): one line, sentence case, no code
  identifiers in the first line; the detail line is monospace and may be empty:
    no drafter   "Configure a Claude Code or Codex agent first"
                 + "Open Builder" button, Draft disabled
    exit code    "claude exited with code 127"        detail: stderr tail
    timeout      "The drafter did not answer in 120 s" detail: none
    validation   "The draft named a model that is not in the catalogue"
                 detail: "draft.agents[1].model: gpt-4o"
    apply failed "Couldn't add Ferro to codeup."
                 + "1 agent was created before the failure. Created agents stay
                    in the Library."

  Apply statuses (per row, in order): pending → created → added → positioned,
  or failed. The trail renders every step so a half-applied team is readable.
  Completion line: "3 agents created and added to codeup."

  Not drawn on purpose: a cancel control for the running model (D9: v1 has
  none) and any launch-flag field (D10, the Builder owns those).
*/

type Flow = "entry" | "brief" | "waiting" | "error" | "preview" | "applying" | "builder";
type Mode = "agent" | "team";
type BriefVariant = "ready" | "no-drafter";
type ApplyVariant = "running" | "done" | "failed";
type ApplyStep = "pending" | "created" | "added" | "positioned" | "failed";

const identity = {
  aoki: "var(--color-agent-indigo)",
  dew: "var(--color-agent-teal)",
  mellow: "var(--color-agent-amber)",
  nova: "var(--color-agent-blue)",
  ferro: "var(--color-agent-orange)",
  hardwell: "var(--color-agent-magenta)",
};

const drafters = [
  { id: "aoki", name: "Aoki", color: identity.aoki, kind: "claude-code", model: "claude-opus-5" },
  { id: "dew", name: "Dew", color: identity.dew, kind: "claude-code", model: "claude-sonnet-5" },
  { id: "mellow", name: "Mellow", color: identity.mellow, kind: "codex", model: "gpt-5.6-sol" },
];

const SAMPLE_BRIEF =
  "Port the billing service from Node to Rust, module by module, with tests for each module and a reviewer who checks the ported behaviour against the old service.";

const CLAUDE_MODELS = ["claude-fable-5-1", "claude-opus-5", "claude-sonnet-5", "claude-haiku-4-5", "claude-opus-4-8"];
const ROLE_OPTIONS = ["Lead", "Reviewer", "Implementer", "Designer", "Researcher", "New: Rust Porter"];
const LEVELS = ["Junior", "Mid", "Senior", "Principal"];

interface DraftRow {
  key: string;
  name: string;
  color: string;
  role: string;
  newRole: boolean;
  reuse: boolean;
  model: string;
  level: string;
  reports: string;
  rationale: string;
}

const INITIAL_ROWS: DraftRow[] = [
  {
    key: "lead",
    name: "Nova",
    color: identity.nova,
    role: "Lead",
    newRole: false,
    reuse: false,
    model: "claude-opus-5",
    level: "Principal",
    reports: "—",
    rationale: "One lead settles decisions and integrates the two lanes.",
  },
  {
    key: "impl-rust",
    name: "Ferro",
    color: identity.ferro,
    role: "New: Rust Porter",
    newRole: true,
    reuse: false,
    model: "claude-sonnet-5",
    level: "Senior",
    reports: "Nova",
    rationale: "The brief needs a Rust specialist and no catalogue role fits.",
  },
  {
    key: "reviewer",
    name: "Hardwell",
    color: identity.hardwell,
    role: "Reviewer",
    newRole: false,
    reuse: true,
    model: "claude-sonnet-5",
    level: "Senior",
    reports: "Nova",
    rationale: "Reuse the reviewer definition already in the Library.",
  },
];

const libraryDefs = [
  { name: "Aoki", color: identity.aoki, role: "Lead", model: "claude-opus-5", kind: "claude-code" },
  { name: "Hardwell", color: identity.hardwell, role: "Reviewer", model: "claude-sonnet-5", kind: "claude-code" },
  { name: "Dew", color: identity.dew, role: "Implementer", model: "claude-sonnet-5", kind: "claude-code" },
  { name: "Mellow", color: identity.mellow, role: "Researcher", model: "gpt-5.6-sol", kind: "codex" },
];

function Avatar({ name, color, size = 7 }: { name: string; color: string; size?: 6 | 7 | 10 }) {
  const box = size === 10 ? "h-10 w-10 rounded-[10px] text-[15px]" : size === 6 ? "h-6 w-6 rounded-[7px] text-[11px]" : "h-7 w-7 rounded-[8px] text-[12px]";
  return (
    <span className={`grid shrink-0 place-items-center font-bold text-white ${box}`} style={{ backgroundColor: color }} aria-hidden="true">
      {name[0]}
    </span>
  );
}

function FieldLabel({ children }: { children: React.ReactNode }) {
  return <div className="mb-1.5 text-[10px] font-bold uppercase tracking-[0.08em] text-text-tertiary">{children}</div>;
}

function RailIcon({ active, children, label }: { active?: boolean; children: React.ReactNode; label: string }) {
  return (
    <button
      type="button"
      aria-label={label}
      className={`grid h-10 w-10 place-items-center rounded-[10px] transition-colors ${
        active ? "bg-accent text-white" : "text-text-secondary hover:bg-overlay/[0.06] hover:text-text-primary"
      }`}
    >
      {children}
    </button>
  );
}

function Segmented<T extends string>({
  label,
  value,
  options,
  onChange,
  disabled,
}: {
  label: string;
  value: T;
  options: readonly (readonly [T, string])[];
  onChange: (value: T) => void;
  disabled?: (value: T) => boolean;
}) {
  return (
    <div className="flex shrink-0 items-center gap-1.5" aria-label={label}>
      <span className="text-[9.5px] font-semibold text-text-tertiary">{label}</span>
      <div className="flex items-center rounded-lg bg-canvas p-1">
        {options.map(([option, text]) => (
          <button
            key={option}
            type="button"
            disabled={disabled?.(option)}
            aria-pressed={value === option}
            onClick={() => onChange(option)}
            className={`rounded-md px-2 py-1 text-[10px] font-semibold transition-colors disabled:cursor-not-allowed disabled:opacity-30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
              value === option ? "bg-surface-raised text-text-primary" : "text-text-muted hover:text-text-primary"
            }`}
          >
            {text}
          </button>
        ))}
      </div>
    </div>
  );
}

/** Compact native select styled to the app's fill controls. */
function Field({ value, options, onChange, label }: { value: string; options: string[]; onChange: (v: string) => void; label: string }) {
  return (
    <div className="relative">
      <select
        aria-label={label}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        className="h-7 w-full appearance-none truncate rounded-md bg-fill pl-2 pr-6 text-[11.5px] text-text-primary transition-colors hover:bg-overlay/[0.08] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
      >
        {options.map((option) => (
          <option key={option} value={option}>
            {option}
          </option>
        ))}
      </select>
      <ChevronDown className="pointer-events-none absolute right-1.5 top-1/2 h-3 w-3 -translate-y-1/2 text-text-muted" />
    </div>
  );
}

/* ── Entry points ─────────────────────────────────────────────────────────── */

function RosterSidebar({ onBuildTeam }: { onBuildTeam: () => void }) {
  return (
    <aside className="flex w-[248px] shrink-0 flex-col border-r border-border bg-sidebar">
      <div className="flex h-12 items-center gap-2 border-b border-border px-3.5">
        <span className="grid h-6 w-6 place-items-center rounded-[7px] bg-accent text-[11px] font-bold text-white">C</span>
        <div className="min-w-0 flex-1 leading-tight">
          <div className="truncate text-[12.5px] font-semibold">codeup</div>
          <div className="flex items-center gap-1 truncate font-mono text-[9.5px] text-text-muted">
            <Folder className="h-2.5 w-2.5" /> /Users/dev/code/codeup
          </div>
        </div>
      </div>
      <div className="px-3 pb-2 pt-3">
        <div className="flex h-7 items-center gap-2 rounded-lg bg-overlay/[0.05] px-2.5">
          <Search className="h-3.5 w-3.5 text-text-muted" />
          <span className="text-[12px] text-text-tertiary">Search agents</span>
        </div>
      </div>
      <div className="min-h-0 flex-1 px-2 pb-2">
        <div className="px-2 pb-1 text-[10px] font-bold uppercase tracking-[0.08em] text-text-tertiary">CLI agents</div>
        <div className="space-y-0.5">
          {[
            { name: "Aoki", color: identity.aoki, role: "Lead · principal" },
            { name: "Dew", color: identity.dew, role: "Implementer · senior" },
          ].map((agent) => (
            <div key={agent.name} className="flex min-h-11 items-center gap-2.5 rounded-lg px-2 py-1.5 hover:bg-overlay/[0.04]">
              <Avatar name={agent.name} color={agent.color} />
              <div className="min-w-0 flex-1 leading-tight">
                <div className="flex items-center gap-1.5">
                  <span className="truncate text-[12.5px] font-semibold">{agent.name}</span>
                  <Terminal className="h-3 w-3 shrink-0 text-text-muted" />
                </div>
                <div className="mt-0.5 truncate text-[10.5px] text-text-muted">{agent.role}</div>
              </div>
              <span className="h-2 w-2 shrink-0 rounded-full bg-live" role="img" aria-label="running" />
            </div>
          ))}
        </div>
        <p className="px-2 pt-4 text-[10.5px] leading-relaxed text-text-tertiary">
          Two agents cover the current work. A team for a new brief is drafted, reviewed, then applied here.
        </p>
      </div>
      <div className="space-y-0.5 border-t border-border p-2">
        <button
          type="button"
          className="flex w-full items-center gap-2.5 rounded-lg px-2 py-1.5 text-accent transition-colors hover:bg-accent/10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
        >
          <span className="grid h-7 w-7 shrink-0 place-items-center rounded-[8px] border border-dashed border-accent/50">
            <Plus className="h-[15px] w-[15px]" />
          </span>
          <span className="text-[12.5px] font-semibold">Add agent</span>
        </button>
        <button
          type="button"
          onClick={onBuildTeam}
          className="flex w-full items-center gap-2.5 rounded-lg px-2 py-1.5 text-accent transition-colors hover:bg-accent/10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
        >
          <span className="grid h-7 w-7 shrink-0 place-items-center rounded-[8px] bg-accent/[0.16]">
            <Sparkles className="h-[15px] w-[15px]" />
          </span>
          <span className="text-[12.5px] font-semibold">Build team with AI</span>
        </button>
      </div>
      <div className="border-t border-border p-2 text-[12px] text-text-secondary">
        <div className="flex items-center gap-2 rounded-lg px-2 py-1.5 hover:bg-overlay/[0.04]">
          <MessageSquare className="h-4 w-4" />
          <span className="font-semibold">Chat</span>
        </div>
      </div>
    </aside>
  );
}

function AgentPane() {
  return (
    <main className="flex min-w-0 flex-1 flex-col bg-surface">
      <div className="flex h-12 items-center gap-1 border-b border-border bg-sidebar px-2">
        <div className="flex h-7 items-center gap-2 rounded-md bg-overlay/[0.06] px-3 text-[12.5px] font-semibold">
          <span className="h-2 w-2 rounded-full bg-live" role="img" aria-label="running" />
          <span>Aoki</span>
          <Terminal className="h-3 w-3 text-text-muted" />
        </div>
      </div>
      <div className="flex h-10 items-center gap-2 border-b border-border px-3 text-[11.5px] text-text-secondary">
        <span className="inline-flex items-center gap-1.5 rounded-md px-2 py-1">
          <Users className="h-3.5 w-3.5 text-accent" /> Lead · principal
        </span>
        <span className="h-4 w-px bg-border" />
        <span className="font-mono text-[10.5px] text-text-muted">claude-code · claude-opus-5</span>
      </div>
      <div className="flex-1 px-5 py-4 font-mono text-[12px] leading-relaxed text-text-secondary">
        <p className="text-live">runtime ready</p>
        <p>Two agents cover the billing work. The port to Rust needs people this workspace does not have yet.</p>
        <span className="mt-3 inline-block h-4 w-1.5 bg-text-primary" aria-hidden="true" />
      </div>
      <div className="border-t border-border bg-surface px-4 py-3">
        <div className="rounded-[12px] bg-fill px-3 py-2 text-[12px] text-text-muted">Message Aoki…</div>
      </div>
    </main>
  );
}

/** The real Library is a 440px right sheet over a scrim (Library.tsx:169-176). */
function LibrarySheet({ onDraftAgent, onClose }: { onDraftAgent: () => void; onClose: () => void }) {
  return (
    <div className="absolute inset-0 z-30 flex justify-end" role="presentation">
      <div className="absolute inset-0 bg-black/30" onClick={onClose} />
      <div className="relative flex h-full w-[440px] max-w-full flex-col bg-sidebar shadow-2xl">
        <div className="flex h-12 shrink-0 items-center gap-2 border-b border-border px-4">
          <Users className="h-[15px] w-[15px] shrink-0 text-accent" />
          <span className="text-[13px] font-semibold tracking-[-0.01em]">Agent Library</span>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close Agent Library"
            className="ml-auto grid h-6 w-6 shrink-0 place-items-center rounded-md text-text-muted transition-colors hover:bg-overlay/[0.06] hover:text-text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
        <div className="shrink-0 px-3 pb-2 pt-3">
          <div className="flex h-7 items-center gap-2 rounded-lg bg-overlay/[0.05] px-2.5">
            <Search className="h-[13px] w-[13px] text-text-muted" />
            <span className="text-[12px] text-text-tertiary">Search agents</span>
          </div>
        </div>
        <div className="min-h-0 flex-1 space-y-1.5 overflow-y-auto px-3 pb-3">
          {libraryDefs.map((def) => (
            <div key={def.name} className="flex items-start gap-3 rounded-xl bg-surface p-3">
              <Avatar name={def.name} color={def.color} size={10} />
              <div className="min-w-0 flex-1 leading-tight">
                <div className="flex items-center gap-1.5">
                  <span className="truncate text-[13.5px] font-semibold">{def.name}</span>
                  <span className="rounded bg-overlay/[0.05] px-1.5 py-px text-[9.5px] font-medium text-text-muted">CLI</span>
                </div>
                <div className="mt-0.5 truncate text-[11px] text-text-muted">
                  {def.role} · <span className="font-mono">{def.model}</span>
                </div>
                <div className="mt-1.5 truncate text-[10.5px] text-text-tertiary">{def.kind}</div>
              </div>
            </div>
          ))}
          <p className="px-1 pt-2 text-[10.5px] leading-relaxed text-text-tertiary">
            Definitions are reusable across workspaces. A drafted agent lands here only after you press Create agent in the Builder.
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2 border-t border-border p-2">
          <button
            type="button"
            className="flex flex-1 items-center justify-center gap-1.5 rounded-lg bg-accent px-2 py-2 text-white transition-colors hover:bg-accent-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
          >
            <Plus className="h-4 w-4" />
            <span className="text-[12.5px] font-semibold">New agent</span>
          </button>
          <button
            type="button"
            onClick={onDraftAgent}
            className="flex flex-1 items-center justify-center gap-1.5 rounded-lg bg-overlay/[0.06] px-2 py-2 text-text-primary transition-colors hover:bg-overlay/[0.10] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
          >
            <Sparkles className="h-4 w-4 text-accent" />
            <span className="text-[12.5px] font-semibold">Draft with AI</span>
          </button>
        </div>
      </div>
    </div>
  );
}

/* ── Overlay chrome ───────────────────────────────────────────────────────── */

function Panel({
  width,
  title,
  chip,
  onClose,
  children,
  footer,
}: {
  width: string;
  title: string;
  chip: string;
  onClose: () => void;
  children: React.ReactNode;
  footer: React.ReactNode;
}) {
  return (
    <div className="absolute inset-0 z-40 grid place-items-center bg-black/45 px-6" role="presentation">
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className={`flex max-h-[92%] max-w-full flex-col overflow-hidden rounded-[14px] bg-surface shadow-2xl ${width}`}
      >
        <div className="flex h-11 shrink-0 items-center justify-between border-b border-border px-4">
          <div className="flex items-center gap-2">
            <Sparkles className="h-4 w-4 text-accent" />
            <span className="text-[13px] font-semibold tracking-[-0.01em]">{title}</span>
            <span className="rounded-md bg-overlay/[0.05] px-1.5 py-px text-[10px] font-medium text-text-muted">{chip}</span>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close drafter"
            className="grid h-7 w-7 place-items-center rounded-md text-text-secondary transition-colors hover:bg-overlay/[0.06] hover:text-text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
          >
            <X className="h-[15px] w-[15px]" />
          </button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">{children}</div>
        <div className="flex shrink-0 items-center gap-2 border-t border-border bg-surface px-5 py-2.5">{footer}</div>
      </div>
    </div>
  );
}

function GhostButton({ children, onClick }: { children: React.ReactNode; onClick?: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="rounded-lg px-3.5 py-1.5 text-[12.5px] font-medium text-text-secondary transition-colors hover:bg-overlay/[0.05] hover:text-text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
    >
      {children}
    </button>
  );
}

/** `busy` keeps the accent fill (work is running); a plain `disabled` drops to
 *  the fill surface so an unavailable primary never reads as pressable. */
function PrimaryButton({
  children,
  onClick,
  disabled,
  busy,
  icon,
}: {
  children: React.ReactNode;
  onClick?: () => void;
  disabled?: boolean;
  busy?: boolean;
  icon: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled || busy}
      className={`flex items-center gap-1.5 rounded-lg px-4 py-1.5 text-[12.5px] font-semibold transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
        busy
          ? "cursor-wait bg-accent/70 text-white"
          : "bg-accent text-white hover:bg-accent-hover disabled:cursor-not-allowed disabled:bg-fill disabled:text-text-tertiary"
      }`}
    >
      {icon}
      {children}
    </button>
  );
}

/* ── Drafter body: brief / waiting / error ────────────────────────────────── */

function BriefBody({
  mode,
  flow,
  variant,
  brief,
  setBrief,
  drafterId,
  setDrafterId,
  elapsed,
}: {
  mode: Mode;
  flow: Flow;
  variant: BriefVariant;
  brief: string;
  setBrief: (value: string) => void;
  drafterId: string;
  setDrafterId: (value: string) => void;
  elapsed: string;
}) {
  const drafter = drafters.find((d) => d.id === drafterId) ?? drafters[0];
  const waiting = flow === "waiting";
  const failed = flow === "error";

  return (
    <div className="space-y-4">
      <section>
        <FieldLabel>Brief</FieldLabel>
        <textarea
          aria-label="Brief"
          value={brief}
          readOnly={waiting}
          onChange={(event) => setBrief(event.target.value)}
          rows={4}
          placeholder="Describe the job. Example: a team to port our billing service from Node to Rust with tests and a reviewer."
          className={`w-full resize-none rounded-lg bg-fill px-3 py-2.5 text-[12.5px] leading-relaxed text-text-primary placeholder:text-text-tertiary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
            waiting ? "text-text-secondary" : ""
          }`}
        />
        <p className="mt-1.5 px-0.5 text-[10.5px] text-text-tertiary">
          {mode === "agent"
            ? "One agent is drafted and opened in the Builder. Nothing is saved until you press Create agent."
            : "Agents, levels and reporting lines are proposed for codeup. You review and edit every row before anything is created."}
        </p>
      </section>

      {variant === "no-drafter" ? (
        <section>
          <FieldLabel>Drafter</FieldLabel>
          <div className="flex items-center gap-2.5 rounded-lg bg-waiting/[0.12] px-3 py-2.5 text-[12px] font-semibold text-waiting" role="alert">
            <AlertCircle className="h-4 w-4 shrink-0" />
            <span className="min-w-0 flex-1">Configure a Claude Code or Codex agent first</span>
            <button
              type="button"
              className="shrink-0 rounded-md bg-waiting px-2.5 py-1 text-[11.5px] font-semibold text-canvas transition-[filter] hover:brightness-110 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
            >
              Open Builder
            </button>
          </div>
          <p className="mt-1.5 px-0.5 text-[10.5px] leading-relaxed text-text-tertiary">
            Drafting runs through one of your CLI agent definitions and reuses its model, environment and credentials.
          </p>
        </section>
      ) : (
        <section>
          <FieldLabel>Drafter</FieldLabel>
          <div className="space-y-1">
            {drafters.map((candidate) => {
              const selected = candidate.id === drafter.id;
              return (
                <button
                  key={candidate.id}
                  type="button"
                  disabled={waiting}
                  aria-pressed={selected}
                  onClick={() => setDrafterId(candidate.id)}
                  className={`flex w-full items-center gap-2.5 rounded-lg px-2 py-1.5 text-left transition-colors disabled:cursor-not-allowed focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                    selected ? "bg-accent/[0.12]" : "hover:bg-overlay/[0.05]"
                  } ${waiting && !selected ? "opacity-45" : ""}`}
                >
                  <Avatar name={candidate.name} color={candidate.color} size={6} />
                  <span className="min-w-0 flex-1 truncate text-[12.5px] font-semibold">{candidate.name}</span>
                  <span className="shrink-0 font-mono text-[10.5px] text-text-muted">
                    {candidate.kind} · {candidate.model}
                  </span>
                  <span
                    className={`grid h-4 w-4 shrink-0 place-items-center rounded-full ${
                      selected ? "bg-accent text-white" : "bg-overlay/[0.10] text-transparent"
                    }`}
                    aria-hidden="true"
                  >
                    <Check className="h-2.5 w-2.5" />
                  </span>
                </button>
              );
            })}
          </div>
        </section>
      )}

      {waiting && (
        <section className="flex items-center gap-3 rounded-lg bg-fill px-3 py-2.5">
          <LoaderCircle className="h-4 w-4 shrink-0 animate-spin text-accent motion-reduce:animate-none" />
          <div className="min-w-0 flex-1 leading-tight">
            <p className="text-[12.5px] font-semibold">Drafting with {drafter.name}</p>
            <p className="mt-0.5 font-mono text-[10.5px] text-text-muted">
              {drafter.kind} · {drafter.model}
            </p>
          </div>
          <span className="shrink-0 font-mono text-[13px] tabular-nums text-text-secondary">{elapsed}</span>
        </section>
      )}

      {failed && (
        <section>
          <div className="flex items-start gap-2.5 rounded-lg bg-danger/[0.12] px-3 py-2.5 text-[12px] leading-relaxed text-danger" role="alert">
            <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <div className="min-w-0">
              <p className="font-semibold">The draft named a model that is not in the catalogue</p>
              <p className="mt-1 font-mono text-[10.5px]">draft.agents[1].model: gpt-4o</p>
            </div>
          </div>
          <p className="mt-1.5 px-0.5 text-[10.5px] text-text-tertiary">Retry sends the same brief again. Nothing was created.</p>
        </section>
      )}
    </div>
  );
}

/* ── Drafter body: team preview ──────────────────────────────────────────── */

const COLUMNS = "grid-cols-[176px_148px_150px_92px_124px_1fr]";

function PreviewBody({
  rows,
  setRows,
  drafterId,
}: {
  rows: DraftRow[];
  setRows: (rows: DraftRow[]) => void;
  drafterId: string;
}) {
  const drafter = drafters.find((d) => d.id === drafterId) ?? drafters[0];
  const update = (key: string, patch: Partial<DraftRow>) => setRows(rows.map((row) => (row.key === key ? { ...row, ...patch } : row)));
  const reportsFor = (key: string) => ["—", ...rows.filter((row) => row.key !== key).map((row) => row.name)];

  return (
    <div className="space-y-3.5">
      <section className="rounded-lg bg-fill px-3 py-2.5">
        <FieldLabel>Notes from the drafter</FieldLabel>
        <p className="text-[12px] leading-relaxed text-text-secondary">
          Assumed the port keeps the public HTTP contract. Added a reviewer because the brief asks for tests, and reused Hardwell instead of
          creating a second reviewer.
        </p>
        <p className="mt-2 flex items-center gap-1.5 font-mono text-[10px] text-text-muted">
          <Avatar name={drafter.name} color={drafter.color} size={6} />
          {drafter.name} · {drafter.model} · 21 s
        </p>
      </section>

      <section>
        <div className={`grid ${COLUMNS} gap-2 border-b border-border pb-1.5 text-[9.5px] font-bold uppercase tracking-[0.08em] text-text-tertiary`}>
          <span>Name</span>
          <span>Role</span>
          <span>Model</span>
          <span>Level</span>
          <span>Reports to</span>
          <span>Rationale</span>
        </div>
        <div className="divide-y divide-border">
          {rows.map((row) => (
            <div key={row.key} className={`grid ${COLUMNS} items-center gap-2 py-2`}>
              <div className="flex min-w-0 items-center gap-2">
                <Avatar name={row.name} color={row.color} size={6} />
                {row.reuse ? (
                  <span className="min-w-0 flex-1 truncate text-[12px] font-semibold text-text-secondary">{row.name}</span>
                ) : (
                  <input
                    aria-label={`Name for ${row.key}`}
                    value={row.name}
                    onChange={(event) => update(row.key, { name: event.target.value })}
                    className="min-w-0 flex-1 rounded-md bg-transparent px-1 py-0.5 text-[12px] font-semibold text-text-primary transition-colors hover:bg-overlay/[0.06] focus-visible:bg-overlay/[0.06] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
                  />
                )}
                {row.reuse && (
                  <span className="inline-flex shrink-0 items-center gap-1 rounded-md bg-accent/[0.14] px-1.5 py-0.5 text-[10px] font-semibold text-accent" title="Reuses an existing definition">
                    <Check className="h-2.5 w-2.5" />
                    Reuse
                  </span>
                )}
              </div>

              {row.reuse ? (
                <span className="truncate text-[11.5px] text-text-secondary">{row.role}</span>
              ) : (
                <Field label={`Role for ${row.key}`} value={row.role} options={ROLE_OPTIONS} onChange={(role) => update(row.key, { role, newRole: role.startsWith("New:") })} />
              )}

              {row.reuse ? (
                <span className="truncate font-mono text-[10.5px] text-text-muted">{row.model}</span>
              ) : (
                <Field label={`Model for ${row.key}`} value={row.model} options={CLAUDE_MODELS} onChange={(model) => update(row.key, { model })} />
              )}

              <Field label={`Level for ${row.key}`} value={row.level} options={LEVELS} onChange={(level) => update(row.key, { level })} />
              <Field label={`Supervisor for ${row.key}`} value={row.reports} options={reportsFor(row.key)} onChange={(reports) => update(row.key, { reports })} />

              <p className="line-clamp-2 text-[11px] leading-snug text-text-tertiary" title={row.rationale}>
                {row.rationale}
              </p>
            </div>
          ))}
        </div>
        <p className="pt-2 text-[10.5px] text-text-tertiary">
          {rows.filter((row) => row.newRole).length > 0 && "Rust Porter is created as a custom role. "}
          Every value here is checked against the live catalogue before it is applied.
        </p>
      </section>
    </div>
  );
}

/* ── Drafter body: apply ledger ──────────────────────────────────────────── */

const STEP_ORDER: ApplyStep[] = ["created", "added", "positioned"];
const STEP_LABEL: Record<string, string> = { created: "Created", added: "Added", positioned: "Positioned" };

/** `reached` = completed steps; `inFlight` marks the single row the executor is
 *  on right now, so a queued row never shows a spinner. */
function stepsFor(variant: ApplyVariant, index: number): { reached: number; state: ApplyStep; inFlight: boolean } {
  if (variant === "done") return { reached: 3, state: "positioned", inFlight: false };
  if (variant === "failed") {
    if (index === 0) return { reached: 3, state: "positioned", inFlight: false };
    if (index === 1) return { reached: 1, state: "failed", inFlight: false };
    return { reached: 0, state: "pending", inFlight: false };
  }
  if (index === 0) return { reached: 3, state: "positioned", inFlight: false };
  if (index === 1) return { reached: 2, state: "added", inFlight: true };
  return { reached: 0, state: "pending", inFlight: false };
}

function ApplyBody({ rows, variant }: { rows: DraftRow[]; variant: ApplyVariant }) {
  return (
    <div className="space-y-3">
      <div className="divide-y divide-border">
        {rows.map((row, index) => {
          const { reached, state, inFlight } = stepsFor(variant, index);
          const failed = state === "failed";
          return (
            <div key={row.key} className="flex items-center gap-3 py-2.5">
              <Avatar name={row.name} color={row.color} />
              <div className="min-w-0 w-[150px] leading-tight">
                <div className="truncate text-[12.5px] font-semibold">{row.name}</div>
                <div className="mt-0.5 truncate text-[10.5px] text-text-muted">
                  {row.role} · {row.level.toLowerCase()}
                </div>
              </div>
              <div className="flex min-w-0 flex-1 items-center gap-1.5">
                {STEP_ORDER.map((step, stepIndex) => {
                  const done = stepIndex < reached;
                  const active = inFlight && stepIndex === reached;
                  const broken = failed && stepIndex === reached;
                  return (
                    <span
                      key={step}
                      className={`inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[10.5px] font-semibold ${
                        broken
                          ? "bg-danger/[0.14] text-danger"
                          : done
                            ? "bg-live/[0.14] text-live"
                            : active
                              ? "bg-waiting/[0.14] text-waiting"
                              : "bg-overlay/[0.05] text-text-tertiary"
                      }`}
                    >
                      {broken ? (
                        <X className="h-3 w-3" />
                      ) : done ? (
                        <Check className="h-3 w-3" />
                      ) : active ? (
                        <LoaderCircle className="h-3 w-3 animate-spin motion-reduce:animate-none" />
                      ) : (
                        <CircleDashed className="h-3 w-3" />
                      )}
                      {STEP_LABEL[step]}
                    </span>
                  );
                })}
              </div>
              <span className={`shrink-0 text-[11px] font-semibold ${failed ? "text-danger" : state === "pending" ? "text-text-tertiary" : "text-text-secondary"}`}>
                {failed ? "Failed" : state === "pending" ? "Pending" : STEP_LABEL[state]}
              </span>
            </div>
          );
        })}
      </div>

      {variant === "done" && (
        <p className="flex items-center gap-2 rounded-lg bg-live/[0.10] px-3 py-2.5 text-[12.5px] font-semibold text-live">
          <Check className="h-4 w-4 shrink-0" />3 agents created and added to codeup.
        </p>
      )}
      {variant === "running" && (
        <p className="px-0.5 text-[11px] text-text-tertiary">Steps run in reporting order so every supervisor exists before its reports.</p>
      )}
      {variant === "failed" && (
        <div>
          <p className="flex items-center gap-2 rounded-lg bg-danger/[0.12] px-3 py-2.5 text-[12.5px] font-semibold text-danger" role="alert">
            <AlertCircle className="h-4 w-4 shrink-0" />
            Couldn&rsquo;t add Ferro to codeup.
          </p>
          <p className="mt-1.5 px-0.5 text-[11px] leading-relaxed text-text-tertiary">
            1 agent was created before the failure. Created agents stay in the Library, so you can fix the cause and apply the rest.
          </p>
        </div>
      )}
    </div>
  );
}

/* ── Builder with a draft ────────────────────────────────────────────────── */

function BuilderPanel({ onClose }: { onClose: () => void }) {
  const [name, setName] = useState("Ferro");
  const [touched, setTouched] = useState(false);
  const [model, setModel] = useState("claude-sonnet-5");
  const [level, setLevel] = useState("Senior");

  return (
    <Panel
      width="w-[560px]"
      title="New agent"
      chip="saved to Library"
      onClose={onClose}
      footer={
        <>
          <span className="mr-auto text-[10.5px] text-text-tertiary">Rust Porter is created as a custom role on save.</span>
          <GhostButton onClick={onClose}>Cancel</GhostButton>
          <PrimaryButton icon={<Sparkles className="h-3.5 w-3.5" />}>Create agent</PrimaryButton>
        </>
      }
    >
      <div className="space-y-4">
        <section>
          <div className="mb-2 flex items-center gap-2">
            <FieldLabel>Identity</FieldLabel>
            {!touched && (
              <span className="mb-1.5 inline-flex items-center gap-1 text-[11px] text-text-tertiary">
                <Sparkles className="h-3 w-3" />
                Drafted by Aoki
              </span>
            )}
          </div>
          <div className="flex items-center gap-2.5">
            <button
              type="button"
              aria-label="Change color"
              className="grid h-10 w-10 shrink-0 place-items-center rounded-[10px] text-[15px] font-bold text-white transition-[filter] hover:brightness-110"
              style={{ backgroundColor: identity.ferro }}
            >
              {name[0] ?? "?"}
            </button>
            <input
              aria-label="Agent name"
              value={name}
              onChange={(event) => {
                setName(event.target.value);
                setTouched(true);
              }}
              className="h-9 min-w-0 flex-1 rounded-lg bg-fill px-3 text-[13px] font-semibold text-text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
            />
          </div>
        </section>

        <section>
          <FieldLabel>Role</FieldLabel>
          <div className="flex items-center gap-2.5 rounded-lg bg-accent/[0.12] px-3 py-2.5">
            <span className="grid h-8 w-8 shrink-0 place-items-center rounded-[9px] bg-accent text-white">
              <UserPen className="h-4 w-4" />
            </span>
            <div className="min-w-0 flex-1 leading-tight">
              <div className="text-[12.5px] font-semibold">Rust Porter</div>
              <div className="mt-0.5 text-[10.5px] text-text-secondary">Custom role · drafted from your brief</div>
            </div>
            <Check className="h-4 w-4 shrink-0 text-accent" />
          </div>
          <p className="mt-2 rounded-lg bg-fill px-3 py-2 text-[11.5px] leading-relaxed text-text-secondary">
            You port Node services to idiomatic Rust, module by module, keeping behaviour identical and covering each module with tests before
            moving on.
          </p>
        </section>

        <section className="grid grid-cols-2 gap-3">
          <div>
            <FieldLabel>Model</FieldLabel>
            <Field label="Model" value={model} options={CLAUDE_MODELS} onChange={setModel} />
          </div>
          <div>
            <FieldLabel>Default level</FieldLabel>
            <Field label="Default level" value={level} options={LEVELS} onChange={setLevel} />
          </div>
        </section>

        <section>
          <FieldLabel>Launch</FieldLabel>
          <div className="flex items-center gap-2 rounded-lg bg-fill px-3 py-2 text-[11.5px] text-text-secondary">
            <Terminal className="h-3.5 w-3.5 shrink-0 text-text-muted" />
            <span className="font-mono">claude-code · own harness · auto permissions</span>
          </div>
          <p className="mt-1.5 px-0.5 text-[10.5px] text-text-tertiary">Launch flags keep Builder defaults; the drafter never sets them.</p>
        </section>
      </div>
    </Panel>
  );
}

/* ── Screen ──────────────────────────────────────────────────────────────── */

export default function AgentDrafter() {
  const [flow, setFlow] = useState<Flow>("preview");
  const [mode, setMode] = useState<Mode>("team");
  const [briefVariant, setBriefVariant] = useState<BriefVariant>("ready");
  const [applyVariant, setApplyVariant] = useState<ApplyVariant>("running");
  const [brief, setBrief] = useState("");
  const [drafterId, setDrafterId] = useState("aoki");
  const [rows, setRows] = useState<DraftRow[]>(INITIAL_ROWS);

  const drafter = drafters.find((d) => d.id === drafterId) ?? drafters[0];
  const teamTitle = mode === "agent" ? "Draft an agent" : "Build a team";
  const teamChip = mode === "agent" ? "opens in Builder" : "applies to codeup";
  const close = () => setFlow("entry");

  const variantGroup =
    flow === "brief" ? (
      <Segmented
        label="Drafter"
        value={briefVariant}
        onChange={setBriefVariant}
        options={
          [
            ["ready", "Configured"],
            ["no-drafter", "None"],
          ] as const
        }
      />
    ) : flow === "applying" ? (
      <Segmented
        label="Apply"
        value={applyVariant}
        onChange={setApplyVariant}
        options={
          [
            ["running", "Running"],
            ["done", "Done"],
            ["failed", "Failed"],
          ] as const
        }
      />
    ) : null;

  return (
    <div className="dark h-screen overflow-hidden bg-canvas font-sans text-text-primary antialiased">
      <div className="flex h-full min-h-0 flex-col gap-3 p-3">
        <header className="flex min-h-[64px] shrink-0 items-center gap-5 rounded-[12px] bg-sidebar px-4 ring-1 ring-border">
          <div className="flex min-w-0 items-center gap-3">
            <div className="grid h-9 w-9 shrink-0 place-items-center rounded-[10px] bg-accent/[0.14] text-accent ring-1 ring-accent/25">
              <Sparkles className="h-[18px] w-[18px]" />
            </div>
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <h1 className="truncate text-[15px] font-semibold tracking-[-0.015em]">AI drafter</h1>
                <span className="rounded-full bg-fill px-2 py-0.5 text-[9.5px] font-semibold text-text-muted">Overlay canon</span>
              </div>
              <p className="mt-0.5 truncate text-[10.5px] text-text-muted">A brief becomes a proposal you edit, then apply</p>
            </div>
          </div>

          <div className="ml-auto flex min-w-0 items-center gap-3 overflow-x-auto" aria-label="Canon state controls">
            <Segmented
              label="Flow"
              value={flow}
              onChange={(next) => {
                setFlow(next);
                if (next === "brief") setBrief("");
                if (next === "waiting" || next === "error") setBrief(SAMPLE_BRIEF);
                if (next === "preview" || next === "applying") setMode("team");
                if (next === "builder") setMode("agent");
              }}
              options={
                [
                  ["entry", "Entry"],
                  ["brief", "Brief"],
                  ["waiting", "Waiting"],
                  ["error", "Error"],
                  ["preview", "Preview"],
                  ["applying", "Applying"],
                  ["builder", "Builder"],
                ] as const
              }
            />

            <span className="h-6 w-px shrink-0 bg-border" />

            <Segmented
              label="Mode"
              value={mode}
              onChange={setMode}
              disabled={(value) => (value === "agent" ? flow === "preview" || flow === "applying" : flow === "builder")}
              options={
                [
                  ["agent", "Agent"],
                  ["team", "Team"],
                ] as const
              }
            />

            {variantGroup && (
              <>
                <span className="h-6 w-px shrink-0 bg-border" />
                {variantGroup}
              </>
            )}
          </div>
        </header>

        <div className="relative min-h-0 flex-1 overflow-hidden rounded-[14px] bg-surface ring-1 ring-border">
          <div className="flex h-full min-h-0 w-full">
            <nav className="flex w-[54px] shrink-0 flex-col items-center gap-2 border-r border-border bg-canvas px-1.5 py-3">
              <RailIcon label="Workspaces">
                <Waypoints className="h-4 w-4" />
              </RailIcon>
              <RailIcon active label="codeup workspace">
                <span className="text-[12px] font-bold">C</span>
              </RailIcon>
              <RailIcon label="Library">
                <Users className="h-4 w-4" />
              </RailIcon>
              <RailIcon label="Chat">
                <MessageSquare className="h-4 w-4" />
              </RailIcon>
              <div className="flex-1" />
              <RailIcon label="Settings">
                <Settings2 className="h-4 w-4" />
              </RailIcon>
            </nav>
            <RosterSidebar
              onBuildTeam={() => {
                setMode("team");
                setFlow("brief");
              }}
            />
            <AgentPane />
          </div>

          {(flow === "entry" || mode === "agent") && flow !== "builder" && (
            <LibrarySheet
              onDraftAgent={() => {
                setMode("agent");
                setFlow("brief");
              }}
              onClose={() => undefined}
            />
          )}

          {(flow === "brief" || flow === "waiting" || flow === "error") && (
            <Panel
              width="w-[620px]"
              title={teamTitle}
              chip={teamChip}
              onClose={close}
              footer={
                <>
                  {flow === "waiting" && (
                    <span className="mr-auto text-[10.5px] text-text-tertiary">Closing this panel does not stop the run.</span>
                  )}
                  {flow !== "waiting" && (
                    <span className="mr-auto text-[10.5px] text-text-tertiary">The drafter answers within 120 s or times out.</span>
                  )}
                  <GhostButton onClick={close}>Cancel</GhostButton>
                  {flow === "error" ? (
                    <PrimaryButton icon={<RotateCcw className="h-3.5 w-3.5" />} onClick={() => setFlow("waiting")}>
                      Retry
                    </PrimaryButton>
                  ) : (
                    <PrimaryButton
                      icon={
                        flow === "waiting" ? (
                          <LoaderCircle className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none" />
                        ) : (
                          <Sparkles className="h-3.5 w-3.5" />
                        )
                      }
                      busy={flow === "waiting"}
                      disabled={briefVariant === "no-drafter" || brief.trim().length === 0}
                      onClick={() => setFlow("waiting")}
                    >
                      {flow === "waiting" ? "Drafting…" : "Draft"}
                    </PrimaryButton>
                  )}
                </>
              }
            >
              <BriefBody
                mode={mode}
                flow={flow}
                variant={briefVariant}
                brief={brief}
                setBrief={setBrief}
                drafterId={drafterId}
                setDrafterId={setDrafterId}
                elapsed="0:18"
              />
            </Panel>
          )}

          {flow === "preview" && (
            <Panel
              width="w-[1020px]"
              title="Build a team"
              chip="applies to codeup"
              onClose={close}
              footer={
                <>
                  <span className="mr-auto text-[11px] text-text-tertiary">
                    {rows.length} agents · 1 reuses an existing definition · drafted by {drafter.name}
                  </span>
                  <GhostButton onClick={() => setFlow("brief")}>Back</GhostButton>
                  <PrimaryButton
                    icon={<Check className="h-3.5 w-3.5" />}
                    onClick={() => {
                      setApplyVariant("running");
                      setFlow("applying");
                    }}
                  >
                    Apply {rows.length} agents
                  </PrimaryButton>
                </>
              }
            >
              <PreviewBody rows={rows} setRows={setRows} drafterId={drafterId} />
            </Panel>
          )}

          {flow === "applying" && (
            <Panel
              width="w-[1020px]"
              title="Build a team"
              chip="applying to codeup"
              onClose={close}
              footer={
                <>
                  <span className="mr-auto text-[11px] text-text-tertiary">
                    {applyVariant === "done" ? "3 of 3 applied" : applyVariant === "failed" ? "1 of 3 applied" : "2 of 3 applied"}
                  </span>
                  {applyVariant !== "running" && <GhostButton onClick={() => setFlow("preview")}>Back to preview</GhostButton>}
                  <PrimaryButton
                    icon={applyVariant === "running" ? <LoaderCircle className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none" /> : <Check className="h-3.5 w-3.5" />}
                    busy={applyVariant === "running"}
                    onClick={close}
                  >
                    {applyVariant === "running" ? "Applying…" : applyVariant === "failed" ? "Close" : "Done"}
                  </PrimaryButton>
                </>
              }
            >
              <ApplyBody rows={rows} variant={applyVariant} />
            </Panel>
          )}

          {flow === "builder" && <BuilderPanel onClose={close} />}
        </div>

        <footer className="flex h-8 shrink-0 items-center gap-4 px-1 text-[9.5px] text-text-tertiary">
          <span className="inline-flex items-center gap-1.5">
            <Sparkles className="h-3 w-3" /> Flow: {flow} · mode: {mode}
          </span>
          <span className="inline-flex items-center gap-1.5">
            <Terminal className="h-3 w-3" /> Drafter: {drafter.name} · {drafter.kind} · {drafter.model}
          </span>
          <span className="ml-auto">Implementation checks: drafter/default · drafter/empty · library</span>
        </footer>
      </div>
    </div>
  );
}
