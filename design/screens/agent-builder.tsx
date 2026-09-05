import { useEffect, useLayoutEffect, useRef, useState } from "react";
import {
  AlertTriangle,
  Check,
  ChevronDown,
  ChevronRight,
  CircleHelp,
  Compass,
  Hammer,
  PenTool,
  Plus,
  RefreshCw,
  Search,
  Settings2,
  ShieldCheck,
  Sparkles,
  Terminal,
  X,
} from "lucide-react";

export const meta = { title: "Agent Builder — New and Edit agent" };

/*
  CANON: agent Builder (New / Edit agent modal) redesign.
  Task: agent-builder-canon. Spec: docs/superpowers/specs/2026-09-05-agent-builder-redesign-design.md
  Implemented by: src/components/Builder.tsx + src/components/builder/*.
  Baseline it replaces: .shots/builder-default.png (560px single column, commit d90b779).

  ── Frame ────────────────────────────────────────────────────────────────────
  Modal 880 wide, max-height 90vh, rounded-2xl, bg-surface, ring-1
  ring-overlay/[0.08], shadow-2xl, over a bg-black/30 backdrop. Three fixed
  bands: header h-11, body (rail + content), footer.
  Header and footer span the full 880; only the CONTENT column scrolls. The
  rail never scrolls with it.

  ── Rail (180px, left) ───────────────────────────────────────────────────────
  bg-fill, border-r border-border, p-2.5, items 4px apart.
  Item: h-7, rounded-lg, px-2.5, gap-2.5, label text-[12px].
  Order = section order: Identity, Role & Level, Runtime, Skills, Position.
  Position appears only when positionEnabled (edit opened from a roster row).

  Dot, 7px, sits left of the label:
    complete    bg-accent (solid)
    incomplete  ring-1 ring-text-tertiary/70, transparent fill (hollow)
    error       bg-danger (solid)
    none        transparent placeholder, keeps labels aligned (Position only,
                which is always valid and therefore carries no dot)

  Active item (scroll-spy): background fill bg-accent/[0.10] + text-accent +
  font-semibold. NO left bar: an accent side-stripe on an active row is the
  banned `side-tab` antipattern and reads as invisible on the dark theme, so
  the active state shifts the FILL instead. Challenge filed on the task
  against the spec's "accent left bar" wording; this is the drawn default.

  Scroll-spy rule (exact): a section is active when its top edge has crossed
  the UPPER THIRD of the scroll container, i.e.
    sectionTop - containerTop <= containerHeight / 3
  and it is the LAST section satisfying that. Clicking a rail item scrolls the
  container to that section's anchor (`[data-builder-section="<id>"]`), smooth.

  ── Content column (700px, px-6 py-5) ────────────────────────────────────────
  Sections are separated by a hairline: every section after the first carries
  `mt-5 border-t border-border pt-5`. Section heading: text-[10px] font-bold
  uppercase tracking-wider text-text-tertiary, with an optional right slot.

  ── Control sizes (reuse these, do not invent new ones) ──────────────────────
  Segmented control (Level, Effort, Permission mode, Context window):
    track  flex rounded-lg bg-overlay/[0.04] p-0.5
    item   rounded-[7px] px-2.5 py-1 text-[11.5px]
    active bg-surface font-semibold text-text-primary shadow-sm
    idle   text-text-secondary hover:text-text-primary
    measured height 28px
  Runtime provider tile (D5, replaces the old Type cards + CLI-kind segment):
    grid   grid grid-cols-3 gap-2
    tile   flex items-center gap-2.5 rounded-xl px-3 py-2.5 ring-1, height 44px
    mark   16px, monochrome currentColor
    name   text-[12.5px] font-semibold
    active ring-accent/40 bg-accent/[0.06], mark text-accent
    idle   ring-overlay/[0.08] bg-surface, mark text-text-secondary
  Settings card (Model / Effort / Permission / Context / rtk):
    rounded-xl bg-surface ring-1 ring-overlay/[0.08], rows divided by
    divide-y divide-overlay/[0.06], each row px-3 py-2.
  Role card: rounded-xl p-2.5 ring-1, mark 17px, name text-[12.5px], one row of
    four (grid-cols-4); a workspace with more builtin roles wraps to a 2nd row.
  Toggle: w-9 h-5 rounded-full, knob 16px; on = bg-live, off = bg-overlay/20.

  ── Provider marks ───────────────────────────────────────────────────────────
  PROVIDER_PATHS below carries the real 24x24 monochrome path data delivered by
  task provider-logos (main 2060481, design/assets/providers/<cliKind>.svg,
  sources MIT lobehub/lobe-icons and Apache-2.0 Untrivial-ai/agent-orchestrator,
  nominative use only). Copy it verbatim into
  src/components/builder/providerLogos.tsx as one map keyed by CliKind; do not
  re-source or re-draw the marks. Muse Spark has NO vector: Detoro ruled a
  lucide Terminal glyph stands in until Meta publishes one, and the reference
  PNG is never embedded. Marks render at 16px in currentColor, so they follow
  the tile's selected/idle colour with no per-brand hue. Only kinds present in
  the CliKind union render as tiles; artboard C draws the five-tile target so
  the two-row grid is specified before opencode and Muse Spark ship.

  ── Copy strings, verbatim ───────────────────────────────────────────────────
  Header title     "New agent" · "Edit agent"
  Header chip      "saved to Library" · "update definition"
  Identity chip    "Drafted by Aoki"          (only while untouched)
  Name placeholder "Agent name"
  Section titles   "Identity" · "Role & Level" · "Runtime" · "Skills" · "Position"
  Role actions     "No role" · "Custom…"
  No-role note     "No role: the agent runs with only the mandatory Collaboration
                    skills, and no job description in its preamble."
  Role attach row  "Attaches"  ·  "mandatory skills only"
  Level label      "Level"
  Level options    "Unranked" "Junior" "Mid" "Senior" "Principal"
  Runtime names    "Claude Code" "Codex" "Antigravity" "opencode" "Muse Spark"
  Runtime caption  "Chat agent and Orchestrator are coming soon."
  Row labels       "Model" "Effort" "Permission mode" "Execution mode"
                   "Context window" "Token filter (rtk)" "Custom args"
                   "Custom environment"
  Effort options   "Auto" "low" "medium" "high"
  Effort hint      "Auto uses the provider default and omits the effort override."
  Permission opts  "Auto" "Bypass"
  Bypass warning   "Skips every permission prompt — use only in workspaces you trust."
  Context options  "200K" "1M"
  1M hint          "Launches as claude-opus-5[1m]."
  rtk hint         "Rewrites shell commands through rtk to compress output and save tokens."
  Advanced         "Advanced"  /  "Custom args, custom environment"
  Args placeholder "e.g. --verbose --mcp-config ./mcp.json"
  Env hint         "Secrets (AUTH_TOKEN / API_KEY / …) are stored in the macOS
                    Keychain, never in the database. Leave a value as •••••••• to
                    keep the stored secret."
  Skills groups    "System skills — always on" · "System skills — optional"
                   "Custom skills"
  agy status       "agy available" · "agy not found" · "Checking agy…"
  agy alert title  "Antigravity CLI is not available"
  agy alert body   "Install the CLI from the Antigravity documentation, then make
                    sure agy is on your login-shell PATH."
  agy alert acts   "Installation guide" · "Check again"
  agy footnote     "Token filtering and sandbox controls are not available for
                    Antigravity in this version."
  Position rows    "Track" · "Level" · "Supervisor" · "Escalation chain"
  Supervisor top   "Top of the chain"
  Footer blockers  "Name required" · "Checking agy…" · "Install agy to continue"
  Footer ready     "Ready to create" · "Ready to save"
  Buttons          "Cancel" · "Create agent" · "Save changes"
                   "Cancel" · "Create role"   (inline role editor)

  ── Readiness (D7) ───────────────────────────────────────────────────────────
  identity  complete when name.trim() is non-empty, else incomplete. Never error.
  role      always complete. No role and Unranked are valid answers.
  runtime   complete unless Antigravity; Antigravity is complete when agy is
            available, error when missing or the check failed, incomplete while
            idle or checking.
  skills    always complete.
  position  no dot, always valid.
  Footer left slot shows the FIRST blocker in this order: "Name required", then
  "Install agy to continue" (missing) or "Checking agy…" (idle/checking), else
  "Ready to create" / "Ready to save". The blocker renders text-text-tertiary,
  except the agy-missing one which renders text-danger. Primary button is
  disabled while any blocker stands.

  ── Position, and the one thing that did change ──────────────────────────────
  D8 keeps Position's CONTENT unchanged, and it is: Track, four Level cards,
  the Supervisor list, the Escalation chain. Only its outer chrome is gone.
  Today those four groups sit inside one rounded panel, which puts a card
  inside a card inside the modal; here they sit directly on the surface with
  their own kicker labels, the way Runtime and Skills already do.
  Position keeps its four LEVEL CARDS while Role & Level above uses the new
  segmented Level control. That is deliberate: Role & Level sets the
  definition's remembered default level, Position sets this workspace
  instance's live level. Raised as a task note; the ruling stands.

  ── Not drawn on purpose ─────────────────────────────────────────────────────
  The colour-swatch popover behind the avatar (moved verbatim, unchanged), the
  Chat agent / Orchestrator "SOON" cards and the Custom CLI tab (D5 removes
  them), and the non-CLI Model/API fallback branch (unreachable after D5).
*/

// ── Vocabulary ───────────────────────────────────────────────────────────────

type SectionId = "identity" | "role" | "runtime" | "skills" | "position";
type Readiness = "complete" | "incomplete" | "error" | "none";
type RuntimeKind = "claude-code" | "codex" | "antigravity" | "opencode" | "muse-spark";
type Level = "Unranked" | "Junior" | "Mid" | "Senior" | "Principal";
type Effort = "Auto" | "low" | "medium" | "high";
type ExecMode = "Default" | "Accept edits" | "Plan" | "Bypass permissions";
type PreviewId = "new-empty" | "new-filled" | "edit-advanced" | "edit-position" | "edit-antigravity";

const LEVELS: Level[] = ["Unranked", "Junior", "Mid", "Senior", "Principal"];
const EFFORTS: Effort[] = ["Auto", "low", "medium", "high"];
const CLAUDE_PRESETS = ["fable-5-1", "opus-5", "sonnet-5", "haiku-4-5", "opus-4-8"];

const EXEC_HELP: Record<ExecMode, string> = {
  Default: "Pauses for diff review before applying changes.",
  "Accept edits": "Applies file edits automatically. Shell and web actions still ask.",
  Plan: "Starts in planning mode before making changes.",
  "Bypass permissions": "Skips every permission prompt, including shell and web actions.",
};

const ROLES = [
  { id: "lead", name: "Lead", Icon: Compass, tagline: "Settles & delegates work", attaches: ["Leadership", "Comms Protocol"] },
  { id: "reviewer", name: "Reviewer", Icon: ShieldCheck, tagline: "Grills work with evidence", attaches: ["Implementer"] },
  { id: "implementer", name: "Implementer", Icon: Hammer, tagline: "Builds the recorded plan", attaches: ["Implementer", "Memory"] },
  { id: "designer", name: "Designer", Icon: PenTool, tagline: "Designs on the canvas", attaches: ["Design Canvas", "Design Craft"] },
];

const OPTIONAL_SKILLS = ["Implementer", "Leadership", "Memory", "Strategic Compact"];
const CUSTOM_SKILLS = ["Arta canvas", "Lane hygiene"];

// Placeholder marks. Real logos: design/assets/providers/<cliKind>.svg (task
// provider-logos), inlined into src/components/builder/providerLogos.tsx.
const RUNTIMES: { kind: RuntimeKind; name: string; planned?: boolean }[] = [
  { kind: "claude-code", name: "Claude Code" },
  { kind: "codex", name: "Codex" },
  { kind: "antigravity", name: "Antigravity" },
  { kind: "opencode", name: "opencode", planned: true },
  { kind: "muse-spark", name: "Muse Spark", planned: true },
];

const PROVIDER_PATHS: Record<Exclude<RuntimeKind, "muse-spark">, string> = {
  "claude-code":
    "M20.998 10.949H24v3.102h-3v3.028h-1.487V20H18v-2.921h-1.487V20H15v-2.921H9V20H7.488v-2.921H6V20H4.487v-2.921H3V14.05H0V10.95h3V5h17.998v5.949zM6 10.949h1.488V8.102H6v2.847zm10.51 0H18V8.102h-1.49v2.847z",
  codex:
    "M8.086.457a6.105 6.105 0 013.046-.415c1.333.153 2.521.72 3.564 1.7a.117.117 0 00.107.029c1.408-.346 2.762-.224 4.061.366l.063.03.154.076c1.357.703 2.33 1.77 2.918 3.198.278.679.418 1.388.421 2.126a5.655 5.655 0 01-.18 1.631.167.167 0 00.04.155 5.982 5.982 0 011.578 2.891c.385 1.901-.01 3.615-1.183 5.14l-.182.22a6.063 6.063 0 01-2.934 1.851.162.162 0 00-.108.102c-.255.736-.511 1.364-.987 1.992-1.199 1.582-2.962 2.462-4.948 2.451-1.583-.008-2.986-.587-4.21-1.736a.145.145 0 00-.14-.032c-.518.167-1.04.191-1.604.185a5.924 5.924 0 01-2.595-.622 6.058 6.058 0 01-2.146-1.781c-.203-.269-.404-.522-.551-.821a7.74 7.74 0 01-.495-1.283 6.11 6.11 0 01-.017-3.064.166.166 0 00.008-.074.115.115 0 00-.037-.064 5.958 5.958 0 01-1.38-2.202 5.196 5.196 0 01-.333-1.589 6.915 6.915 0 01.188-2.132c.45-1.484 1.309-2.648 2.577-3.493.282-.188.55-.334.802-.438.286-.12.573-.22.861-.304a.129.129 0 00.087-.087A6.016 6.016 0 015.635 2.31C6.315 1.464 7.132.846 8.086.457zm-.804 7.85a.848.848 0 00-1.473.842l1.694 2.965-1.688 2.848a.849.849 0 001.46.864l1.94-3.272a.849.849 0 00.007-.854l-1.94-3.393zm5.446 6.24a.849.849 0 000 1.695h4.848a.849.849 0 000-1.696h-4.848z",
  antigravity:
    "M21.751 22.607c1.34 1.005 3.35.335 1.508-1.508C17.73 15.74 18.904 1 12.037 1 5.17 1 6.342 15.74.815 21.1c-2.01 2.009.167 2.511 1.507 1.506 5.192-3.517 4.857-9.714 9.715-9.714 4.857 0 4.522 6.197 9.714 9.715z",
  opencode: "M16 6H8v12h8V6zm4 16H4V2h16v20z",
};

function ProviderMark({ kind, className = "" }: { kind: RuntimeKind; className?: string }) {
  // Muse Spark ships no vector (ruled 2026-09-05: lucide Terminal until Meta
  // publishes one; the reference PNG is never embedded).
  if (kind === "muse-spark") return <Terminal className={`h-4 w-4 shrink-0 ${className}`} aria-hidden="true" />;
  return (
    <svg
      viewBox="0 0 24 24"
      fill="currentColor"
      fillRule="evenodd"
      clipRule="evenodd"
      aria-hidden="true"
      className={`h-4 w-4 shrink-0 ${className}`}
    >
      <path d={PROVIDER_PATHS[kind]} />
    </svg>
  );
}

// ── Preview configurations (fixed literal example data) ──────────────────────

interface BuilderConfig {
  mode: "new" | "edit";
  name: string;
  letter: string;
  colorVar: string;
  draftedBy: string | null;
  roleId: string | null;
  level: Level;
  runtime: RuntimeKind;
  model: string;
  effort: Effort;
  bypass: boolean;
  execMode: ExecMode;
  contextWindow: "200K" | "1M";
  rtk: boolean;
  advancedOpen: boolean;
  customArgs: string;
  customEnv: boolean;
  skills: string[];
  agyMissing: boolean;
  position: boolean;
  focus: SectionId;
}

const PREVIEWS: Record<PreviewId, { label: string; note: string; config: BuilderConfig }> = {
  "new-empty": {
    label: "New · empty",
    note: "Nothing typed yet. Identity is the only incomplete section, and it is the only thing the footer talks about.",
    config: {
      mode: "new",
      name: "",
      letter: "?",
      colorVar: "var(--color-agent-indigo)",
      draftedBy: null,
      roleId: null,
      level: "Unranked",
      runtime: "claude-code",
      model: "",
      effort: "Auto",
      bypass: false,
      execMode: "Default",
      contextWindow: "200K",
      rtk: true,
      advancedOpen: false,
      customArgs: "",
      customEnv: false,
      skills: [],
      agyMissing: false,
      position: false,
      focus: "identity",
    },
  },
  "new-filled": {
    label: "New · filled",
    note: "Nova, Implementer, Senior, Claude Code on sonnet-5 with Bypass on. Scrolled to Runtime so the rail highlight sits there.",
    config: {
      mode: "new",
      name: "Nova",
      letter: "N",
      colorVar: "var(--color-agent-blue)",
      draftedBy: "Aoki",
      roleId: "implementer",
      level: "Senior",
      runtime: "claude-code",
      model: "claude-sonnet-5",
      effort: "Auto",
      bypass: true,
      execMode: "Default",
      contextWindow: "200K",
      rtk: true,
      advancedOpen: false,
      customArgs: "",
      customEnv: false,
      skills: ["Implementer", "Memory"],
      agyMissing: false,
      position: false,
      focus: "runtime",
    },
  },
  "edit-advanced": {
    label: "Edit · Advanced",
    note: "Editing Tiësto. Advanced opened itself because the saved definition already carries custom args. Skills follows directly below.",
    config: {
      mode: "edit",
      name: "Tiësto",
      letter: "T",
      colorVar: "var(--color-agent-magenta)",
      draftedBy: null,
      roleId: "implementer",
      level: "Principal",
      runtime: "claude-code",
      model: "claude-opus-5",
      effort: "Auto",
      bypass: true,
      execMode: "Default",
      contextWindow: "1M",
      rtk: true,
      advancedOpen: true,
      customArgs: "--verbose --mcp-config ./mcp.json",
      customEnv: true,
      skills: ["Implementer", "Memory", "Arta canvas"],
      agyMissing: false,
      position: true,
      focus: "runtime-advanced" as SectionId,
    },
  },
  "edit-position": {
    label: "Edit · Position",
    note: "Same agent, scrolled to the fifth rail item. Position is edit-only and carries no readiness dot.",
    config: {
      mode: "edit",
      name: "Tiësto",
      letter: "T",
      colorVar: "var(--color-agent-magenta)",
      draftedBy: null,
      roleId: "implementer",
      level: "Principal",
      runtime: "claude-code",
      model: "claude-opus-5",
      effort: "Auto",
      bypass: true,
      execMode: "Default",
      contextWindow: "1M",
      rtk: true,
      advancedOpen: true,
      customArgs: "--verbose --mcp-config ./mcp.json",
      customEnv: true,
      skills: ["Implementer", "Memory", "Arta canvas"],
      agyMissing: false,
      position: true,
      focus: "position",
    },
  },
  "edit-antigravity": {
    label: "Edit · agy missing",
    note: "Orbit runs on Antigravity and the binary is gone. Runtime dot turns danger, the footer says what to do, the primary button is dead.",
    config: {
      mode: "edit",
      name: "Orbit",
      letter: "O",
      colorVar: "var(--color-agent-orange)",
      draftedBy: null,
      roleId: "implementer",
      level: "Mid",
      runtime: "antigravity",
      model: "",
      effort: "Auto",
      bypass: false,
      execMode: "Default",
      contextWindow: "200K",
      rtk: true,
      advancedOpen: false,
      customArgs: "",
      customEnv: false,
      skills: ["Implementer"],
      agyMissing: true,
      position: true,
      focus: "runtime",
    },
  },
};

// ── Derived state (mirrors src/components/builder/readiness.ts) ───────────────

function sectionReadiness(c: BuilderConfig): Record<SectionId, Readiness> {
  return {
    identity: c.name.trim().length > 0 ? "complete" : "incomplete",
    role: "complete",
    runtime: c.runtime === "antigravity" && c.agyMissing ? "error" : "complete",
    skills: "complete",
    position: "none",
  };
}

function firstBlocker(c: BuilderConfig): string | null {
  if (c.name.trim().length === 0) return "Name required";
  if (c.runtime === "antigravity" && c.agyMissing) return "Install agy to continue";
  return null;
}

// ── Small shared parts ───────────────────────────────────────────────────────

function Dot({ state }: { state: Readiness }) {
  if (state === "none") return <span className="h-[7px] w-[7px] shrink-0" aria-hidden="true" />;
  const look =
    state === "complete"
      ? "bg-accent"
      : state === "error"
        ? "bg-danger"
        : "bg-transparent ring-1 ring-text-tertiary/70";
  return <span className={`h-[7px] w-[7px] shrink-0 rounded-full ${look}`} aria-hidden="true" />;
}

function Segmented<T extends string>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: readonly T[];
  onChange: (next: T) => void;
  label: string;
}) {
  return (
    <div role="radiogroup" aria-label={label} className="flex rounded-lg bg-overlay/[0.04] p-0.5">
      {options.map((option) => (
        <button
          key={option}
          type="button"
          role="radio"
          aria-checked={value === option}
          onClick={() => onChange(option)}
          className={`rounded-[7px] px-2.5 py-1 text-[11.5px] transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
            value === option
              ? "bg-surface font-semibold text-text-primary shadow-sm"
              : "text-text-secondary hover:text-text-primary"
          }`}
        >
          {option}
        </button>
      ))}
    </div>
  );
}

function Toggle({ on, onChange, label }: { on: boolean; onChange: (next: boolean) => void; label: string }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      onClick={() => onChange(!on)}
      className={`relative h-5 w-9 shrink-0 rounded-full transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${on ? "bg-live" : "bg-overlay/20"}`}
    >
      <span className={`absolute top-0.5 h-4 w-4 rounded-full bg-surface shadow-sm transition-[left,right] ${on ? "right-0.5" : "left-0.5"}`} />
    </button>
  );
}

function SettingsCard({ children }: { children: React.ReactNode }) {
  return (
    <div className="overflow-hidden rounded-xl bg-surface ring-1 ring-overlay/[0.08] divide-y divide-overlay/[0.06]">
      {children}
    </div>
  );
}

function Kicker({ children }: { children: React.ReactNode }) {
  return <div className="text-[10px] font-bold uppercase tracking-wider text-text-tertiary">{children}</div>;
}

function Section({
  id,
  title,
  actions,
  first = false,
  children,
}: {
  id: string;
  title: string;
  actions?: React.ReactNode;
  first?: boolean;
  children: React.ReactNode;
}) {
  return (
    <section data-builder-section={id} className={first ? "" : "mt-5 border-t border-border pt-5"}>
      <div className="mb-2.5 flex items-center justify-between gap-3">
        <Kicker>{title}</Kicker>
        {actions}
      </div>
      {children}
    </section>
  );
}

// ── Sections ─────────────────────────────────────────────────────────────────

function IdentitySection({ config, name, setName }: { config: BuilderConfig; name: string; setName: (v: string) => void }) {
  const untouched = name === config.name;
  return (
    <Section
      id="identity"
      title="Identity"
      first
      actions={
        config.draftedBy && untouched ? (
          <span className="inline-flex items-center gap-1 text-[11px] text-text-tertiary">
            <Sparkles className="h-3 w-3" />
            Drafted by {config.draftedBy}
          </span>
        ) : null
      }
    >
      <div className="flex items-center gap-2.5">
        <button
          type="button"
          title="Change color"
          aria-label="Change color"
          className="grid h-10 w-10 shrink-0 place-items-center rounded-[10px] text-[15px] font-bold text-white ring-1 ring-overlay/[0.06] hover:brightness-105 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
          style={{ backgroundColor: config.colorVar }}
        >
          {name.trim() ? name.trim()[0].toUpperCase() : config.letter}
        </button>
        <div className="min-w-0 flex-1 space-y-1">
          <input
            value={name}
            onChange={(event) => setName(event.target.value)}
            placeholder="Agent name"
            aria-label="Agent name"
            className="w-full border-b border-overlay/10 bg-transparent pb-0.5 text-[14px] font-semibold tracking-tight text-text-primary outline-none placeholder:text-text-tertiary focus:border-accent"
          />
          <div className="truncate text-[11.5px] text-text-muted">
            {config.roleId ? `${ROLES.find((r) => r.id === config.roleId)?.name} · ${config.level}` : "No role"}
          </div>
        </div>
      </div>
    </Section>
  );
}

function RoleLevelBody({
  roleId,
  setRoleId,
  level,
  setLevel,
  customOpen,
  setCustomOpen,
}: {
  roleId: string | null;
  setRoleId: (id: string | null) => void;
  level: Level;
  setLevel: (l: Level) => void;
  customOpen: boolean;
  setCustomOpen: (v: boolean) => void;
}) {
  const selected = ROLES.find((r) => r.id === roleId) ?? null;
  return (
    <>
      <div className="grid grid-cols-4 gap-2">
        {ROLES.map(({ id, name, Icon }) => {
          const active = roleId === id && !customOpen;
          return (
            <button
              key={id}
              type="button"
              aria-pressed={active}
              onClick={() => {
                setRoleId(id);
                setCustomOpen(false);
              }}
              className={`rounded-xl p-2.5 text-left transition-colors ring-1 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                active ? "ring-accent/40 bg-accent/[0.06]" : "ring-overlay/[0.08] bg-surface hover:bg-overlay/[0.02]"
              }`}
            >
              <Icon className={`mb-1.5 h-[17px] w-[17px] ${active ? "text-accent" : "text-text-secondary"}`} />
              <div className="text-[12.5px] font-semibold leading-tight text-text-primary">{name}</div>
            </button>
          );
        })}
      </div>

      {!customOpen && selected && (
        <div className="mt-2.5">
          <p className="text-[11.5px] text-text-secondary">{selected.tagline}</p>
          <div className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1.5">
            <Kicker>Attaches</Kicker>
            {selected.attaches.map((skill) => (
              <span key={skill} className="rounded-md bg-accent/[0.08] px-2 py-0.5 text-[11px] font-medium text-accent ring-1 ring-accent/40">
                {skill}
              </span>
            ))}
            <span className="text-[10.5px] text-text-tertiary">+ Collaboration, always on</span>
          </div>
        </div>
      )}

      {!customOpen && !selected && (
        <p className="mt-2.5 text-[11.5px] leading-relaxed text-text-tertiary">
          No role: the agent runs with only the mandatory Collaboration skills, and no job description in its preamble.
        </p>
      )}

      {customOpen && <CustomRoleEditor onCancel={() => setCustomOpen(false)} />}

      <div className="mt-3.5 flex items-center justify-between gap-3">
        <span className="text-[12.5px] text-text-secondary">Level</span>
        <Segmented label="Level" value={level} options={LEVELS} onChange={setLevel} />
      </div>
    </>
  );
}

function CustomRoleEditor({ onCancel }: { onCancel: () => void }) {
  const [checked, setChecked] = useState<string[]>(["Implementer"]);
  return (
    <div className="mt-2.5 space-y-2.5 rounded-xl bg-surface p-3 ring-1 ring-overlay/[0.08]">
      <input
        defaultValue="Rust Porter"
        placeholder="Role name"
        aria-label="Role name"
        className="w-full border-b border-overlay/10 bg-transparent pb-0.5 text-[12.5px] font-semibold text-text-primary outline-none focus:border-accent"
      />
      <textarea
        defaultValue="Ports one Node module at a time to Rust, keeps the old behaviour observable, and hands each module to the reviewer with a diff of what changed."
        placeholder="One-paragraph job description (baked into the agent's preamble)"
        rows={3}
        className="w-full resize-none rounded-lg bg-transparent px-2 py-1.5 text-[12px] leading-relaxed text-text-secondary outline-none ring-1 ring-overlay/[0.08] focus:ring-accent"
      />
      <div>
        <Kicker>Default skills</Kicker>
        <div className="mt-1.5 space-y-1">
          {OPTIONAL_SKILLS.map((skill) => (
            <label key={skill} className="flex cursor-pointer items-center gap-2 text-[12px] text-text-secondary">
              <input
                type="checkbox"
                checked={checked.includes(skill)}
                onChange={(event) =>
                  setChecked((prev) => (event.target.checked ? [...prev, skill] : prev.filter((s) => s !== skill)))
                }
              />
              {skill}
            </label>
          ))}
        </div>
      </div>
      <div className="flex items-center justify-end gap-2 pt-0.5">
        <button
          type="button"
          onClick={onCancel}
          className="rounded-lg px-3 py-1 text-[12px] font-medium text-text-secondary hover:bg-overlay/[0.05] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
        >
          Cancel
        </button>
        <button
          type="button"
          className="rounded-lg bg-accent px-3 py-1 text-[12px] font-semibold text-white hover:bg-accent-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
        >
          Create role
        </button>
      </div>
    </div>
  );
}

function RuntimeTiles({
  value,
  onChange,
  kinds,
}: {
  value: RuntimeKind;
  onChange: (kind: RuntimeKind) => void;
  kinds: RuntimeKind[];
}) {
  return (
    <div role="radiogroup" aria-label="Runtime" className="grid grid-cols-3 gap-2">
      {RUNTIMES.filter((runtime) => kinds.includes(runtime.kind)).map(({ kind, name }) => {
        const active = value === kind;
        return (
          <button
            key={kind}
            type="button"
            role="radio"
            aria-checked={active}
            onClick={() => onChange(kind)}
            className={`flex items-center gap-2.5 rounded-xl px-3 py-2.5 text-left transition-colors ring-1 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
              active ? "ring-accent/40 bg-accent/[0.06]" : "ring-overlay/[0.08] bg-surface hover:bg-overlay/[0.02]"
            }`}
          >
            <ProviderMark kind={kind} className={active ? "text-accent" : "text-text-secondary"} />
            <span className="min-w-0 truncate text-[12.5px] font-semibold text-text-primary">{name}</span>
          </button>
        );
      })}
    </div>
  );
}

function AgyStatus({ missing }: { missing: boolean }) {
  return (
    <span className={`inline-flex items-center gap-1 text-[10px] font-medium ${missing ? "text-danger" : "text-text-muted"}`}>
      {missing ? <AlertTriangle className="h-3 w-3" /> : <Check className="h-3 w-3 text-live" />}
      {missing ? "agy not found" : "agy available"}
    </span>
  );
}

function AgyMissingAlert() {
  return (
    <div role="alert" className="mb-2 rounded-xl bg-danger/[0.09] px-3 py-2.5 text-danger">
      <div className="flex items-start gap-2">
        <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
        <div className="min-w-0 flex-1">
          <div className="text-[11.5px] font-semibold">Antigravity CLI is not available</div>
          <p className="mt-0.5 text-[10.5px] leading-relaxed">
            Install the CLI from the Antigravity documentation, then make sure <span className="font-mono">agy</span> is on your
            login-shell PATH.
          </p>
          <div className="mt-2 flex flex-wrap items-center gap-2">
            <button
              type="button"
              className="inline-flex h-7 items-center gap-1.5 rounded-md bg-danger px-2.5 text-[10.5px] font-semibold text-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-danger"
            >
              <CircleHelp className="h-3 w-3" /> Installation guide
            </button>
            <button
              type="button"
              className="inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-[10.5px] font-semibold text-danger hover:bg-danger/[0.08] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-danger"
            >
              <RefreshCw className="h-3 w-3" /> Check again
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function RuntimeSection({ config }: { config: BuilderConfig }) {
  const [runtime, setRuntime] = useState<RuntimeKind>(config.runtime);
  const [model, setModel] = useState(config.model);
  const [effort, setEffort] = useState<Effort>(config.effort);
  const [permission, setPermission] = useState<"Auto" | "Bypass">(config.bypass ? "Bypass" : "Auto");
  const [execMode, setExecMode] = useState<ExecMode>(config.execMode);
  const [contextWindow, setContextWindow] = useState<"200K" | "1M">(config.contextWindow);
  const [rtk, setRtk] = useState(config.rtk);
  const [advanced, setAdvanced] = useState(config.advancedOpen);
  const [customEnv, setCustomEnv] = useState(config.customEnv);

  const antigravity = runtime === "antigravity";
  const claude = runtime === "claude-code";
  const missing = antigravity && config.agyMissing;

  return (
    <Section id="runtime" title="Runtime" actions={antigravity ? <AgyStatus missing={missing} /> : null}>
      <RuntimeTiles value={runtime} onChange={setRuntime} kinds={["claude-code", "codex", "antigravity"]} />
      <p className="mb-3 mt-2 text-[10.5px] text-text-tertiary">Chat agent and Orchestrator are coming soon.</p>

      {missing && <AgyMissingAlert />}

      <SettingsCard>
        <div className="px-3 py-2">
          <div className="flex items-center justify-between gap-3">
            <label htmlFor="builder-model" className="shrink-0 text-[12.5px] text-text-secondary">
              Model
            </label>
            {antigravity ? (
              <div className="relative w-[240px] shrink-0">
                <select
                  id="builder-model"
                  value={model}
                  onChange={(event) => setModel(event.target.value)}
                  className="h-7 w-full appearance-none rounded-lg bg-overlay/[0.04] pl-2.5 pr-7 text-[11.5px] font-medium text-text-primary outline-none focus-visible:ring-2 focus-visible:ring-accent"
                >
                  <option value="">Auto (authenticated default)</option>
                  <option value="gemini-3.8-pro-high">gemini-3.8-pro-high</option>
                  <option value="gemini-3.8-flash">gemini-3.8-flash</option>
                </select>
                <ChevronDown className="pointer-events-none absolute right-2 top-2 h-3 w-3 text-text-muted" />
              </div>
            ) : (
              <input
                id="builder-model"
                value={model}
                onChange={(event) => setModel(event.target.value)}
                placeholder={runtime === "codex" ? "gpt-5.5" : "claude-opus-4-8"}
                className="min-w-0 flex-1 bg-transparent text-right font-mono text-[12.5px] text-text-primary outline-none placeholder:text-text-tertiary focus-visible:rounded-sm focus-visible:ring-2 focus-visible:ring-accent"
              />
            )}
          </div>
          {antigravity ? (
            <p className="mt-1.5 text-[10px] leading-relaxed text-text-tertiary">
              Auto lets Antigravity choose from your authenticated models.
            </p>
          ) : (
            <div className="mt-2 flex flex-wrap gap-1.5">
              {CLAUDE_PRESETS.map((preset) => {
                const full = `claude-${preset}`;
                const active = model === full;
                return (
                  <button
                    key={preset}
                    type="button"
                    onClick={() => setModel(full)}
                    className={`rounded-md px-2 py-0.5 font-mono text-[11px] transition-colors ring-1 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                      active ? "ring-accent/40 bg-accent/[0.08] text-accent" : "ring-overlay/[0.08] text-text-secondary hover:bg-overlay/[0.03]"
                    }`}
                  >
                    {preset}
                  </button>
                );
              })}
            </div>
          )}
        </div>

        <div className="px-3 py-2">
          <div className="flex items-center justify-between gap-3">
            <span className="text-[12.5px] text-text-secondary">Effort</span>
            <Segmented label="Effort" value={effort} options={EFFORTS} onChange={setEffort} />
          </div>
          <p className="mt-1.5 text-[10px] leading-relaxed text-text-tertiary">
            Auto uses the provider default and omits the effort override.
          </p>
        </div>

        {antigravity ? (
          <div className="px-3 py-2">
            <div className="flex items-center justify-between gap-3">
              <label htmlFor="builder-exec" className="shrink-0 text-[12.5px] text-text-secondary">
                Execution mode
              </label>
              <div className="relative min-w-[190px]">
                <select
                  id="builder-exec"
                  value={execMode}
                  onChange={(event) => setExecMode(event.target.value as ExecMode)}
                  className={`h-7 w-full appearance-none rounded-lg bg-overlay/[0.04] pl-2.5 pr-7 text-[11.5px] font-medium outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                    execMode === "Bypass permissions" ? "text-danger" : "text-text-primary"
                  }`}
                >
                  <option>Default</option>
                  <option>Accept edits</option>
                  <option>Plan</option>
                  <option>Bypass permissions</option>
                </select>
                <ChevronDown className="pointer-events-none absolute right-2 top-2 h-3 w-3 text-text-muted" />
              </div>
            </div>
            <p
              className={`mt-1.5 text-[10px] leading-relaxed ${execMode === "Bypass permissions" ? "text-danger" : "text-text-tertiary"}`}
            >
              {EXEC_HELP[execMode]}
            </p>
          </div>
        ) : (
          <div className="px-3 py-2">
            <div className="flex items-center justify-between gap-3">
              <span className="text-[12.5px] text-text-secondary">Permission mode</span>
              <Segmented label="Permission mode" value={permission} options={["Auto", "Bypass"] as const} onChange={setPermission} />
            </div>
            {permission === "Bypass" && (
              <p className="mt-1.5 text-[10.5px] text-waiting">
                Skips every permission prompt — use only in workspaces you trust.
              </p>
            )}
          </div>
        )}

        {claude && (
          <div className="px-3 py-2">
            <div className="flex items-center justify-between gap-3">
              <span className="text-[12.5px] text-text-secondary">Context window</span>
              <Segmented label="Context window" value={contextWindow} options={["200K", "1M"] as const} onChange={setContextWindow} />
            </div>
            {contextWindow === "1M" && (
              <p className="mt-1.5 text-[10.5px] text-text-tertiary">
                Launches as <span className="font-mono">{model || "model"}[1m]</span>.
              </p>
            )}
          </div>
        )}

        {!antigravity && (
          <div className="px-3 py-2">
            <div className="flex items-center justify-between gap-3">
              <span className="text-[12.5px] text-text-secondary">Token filter (rtk)</span>
              <Toggle on={rtk} onChange={setRtk} label="Token filter (rtk)" />
            </div>
            <p className="mt-1.5 text-[10.5px] text-text-tertiary">
              Rewrites shell commands through rtk to compress output and save tokens.
            </p>
          </div>
        )}
      </SettingsCard>

      {antigravity && (
        <p className="mt-2 text-[10px] leading-relaxed text-text-tertiary">
          Token filtering and sandbox controls are not available for Antigravity in this version.
        </p>
      )}

      <div data-builder-section="runtime-advanced" className="mt-2.5">
        <button
          type="button"
          aria-expanded={advanced}
          onClick={() => setAdvanced((open) => !open)}
          className="flex w-full items-center gap-1.5 rounded-lg px-1 py-1.5 text-left transition-colors hover:bg-overlay/[0.03] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
        >
          <ChevronRight className={`h-3.5 w-3.5 text-text-tertiary transition-transform ${advanced ? "rotate-90" : ""}`} />
          <span className="text-[12px] font-semibold text-text-primary">Advanced</span>
          <span className="text-[11px] text-text-tertiary">Custom args, custom environment</span>
        </button>

        {advanced && (
          <div className="mt-2">
            <SettingsCard>
              <div className="flex items-center justify-between gap-3 px-3 py-2.5">
                <span className="shrink-0 text-[12.5px] text-text-secondary">Custom args</span>
                <input
                  defaultValue={config.customArgs}
                  placeholder="e.g. --verbose --mcp-config ./mcp.json"
                  aria-label="Custom args"
                  className="min-w-0 flex-1 bg-transparent text-right font-mono text-[12px] text-text-primary outline-none placeholder:text-text-tertiary focus-visible:rounded-sm focus-visible:ring-2 focus-visible:ring-accent"
                />
              </div>
              {claude && (
                <div className="px-3 py-2">
                  <div className="flex items-center justify-between gap-3">
                    <span className="text-[12.5px] text-text-secondary">Custom environment</span>
                    <Toggle on={customEnv} onChange={setCustomEnv} label="Use custom environment" />
                  </div>
                  {customEnv && (
                    <>
                      <textarea
                        spellCheck={false}
                        rows={6}
                        defaultValue={
                          '{\n  "env": {\n    "ANTHROPIC_BASE_URL": "https://openrouter.ai/api",\n    "ANTHROPIC_AUTH_TOKEN": "••••••••"\n  }\n}'
                        }
                        aria-label="Custom environment"
                        className="mt-2 w-full resize-y rounded-lg bg-fill-soft px-2.5 py-2 font-mono text-[11.5px] leading-relaxed text-text-primary outline-none ring-1 ring-overlay/[0.1] focus:ring-accent/50"
                      />
                      <p className="mt-1.5 text-[10.5px] leading-relaxed text-text-tertiary">
                        Secrets (AUTH_TOKEN / API_KEY / …) are stored in the macOS Keychain, never in the database. Leave a value as{" "}
                        <span className="font-mono">••••••••</span> to keep the stored secret.
                      </p>
                    </>
                  )}
                </div>
              )}
            </SettingsCard>
          </div>
        )}
      </div>
    </Section>
  );
}

function SkillsSection({ config }: { config: BuilderConfig }) {
  const [picked, setPicked] = useState<string[]>(config.skills);
  const toggle = (skill: string, on: boolean) =>
    setPicked((prev) => (on ? [...prev, skill] : prev.filter((s) => s !== skill)));

  return (
    <Section id="skills" title="Skills">
      <SettingsCard>
        <div className="px-3 py-2">
          <Kicker>System skills — always on</Kicker>
          <div className="mt-1.5 flex flex-wrap gap-1.5">
            <span className="rounded-md px-2 py-0.5 text-[11px] font-medium text-text-secondary ring-1 ring-overlay/[0.08]">
              Collaboration
            </span>
          </div>
        </div>
        <div className="px-3 py-2">
          <Kicker>System skills — optional</Kicker>
          <div className="mt-1.5 space-y-1">
            {OPTIONAL_SKILLS.map((skill) => (
              <label key={skill} className="flex cursor-pointer items-center gap-2 text-[12.5px] text-text-secondary">
                <input type="checkbox" checked={picked.includes(skill)} onChange={(event) => toggle(skill, event.target.checked)} />
                {skill}
              </label>
            ))}
          </div>
        </div>
        <div className="px-3 py-2">
          <Kicker>Custom skills</Kicker>
          <div className="mt-1.5 space-y-1">
            {CUSTOM_SKILLS.map((skill) => (
              <label key={skill} className="flex cursor-pointer items-center gap-2 text-[12.5px] text-text-secondary">
                <input type="checkbox" checked={picked.includes(skill)} onChange={(event) => toggle(skill, event.target.checked)} />
                {skill}
              </label>
            ))}
          </div>
        </div>
      </SettingsCard>
    </Section>
  );
}

function PositionSection({ config }: { config: BuilderConfig }) {
  const [level, setLevel] = useState<Level>(config.level);
  const [supervisor, setSupervisor] = useState("detoro");
  const roster = [
    { id: "detoro", name: "Detoro", line: "Principal · Lead" },
    { id: "mellow", name: "Mellow", line: "Senior · Reviewer" },
    { id: "dew", name: "Dew", line: "Senior · Implementer" },
  ];
  const chain = supervisor === "detoro" ? ["Detoro (P4)"] : supervisor === "mellow" ? ["Mellow (P3)", "Detoro (P4)"] : ["Dew (P3)", "Detoro (P4)"];

  return (
    <Section id="position" title="Position">
      <div className="space-y-3.5">
        <div>
          <Kicker>Track</Kicker>
          <div className="mt-1.5 rounded-lg bg-overlay/[0.04] px-3 py-2 text-[11.5px] text-text-secondary">
            <span className="font-medium text-text-primary">{level}</span> · Implementer · reports to{" "}
            <span className="font-medium text-text-primary">{roster.find((r) => r.id === supervisor)?.name}</span>
          </div>
        </div>

        <div>
          <Kicker>Level</Kicker>
          <div className="mt-1.5 grid grid-cols-4 gap-2">
            {(["Junior", "Mid", "Senior", "Principal"] as Level[]).map((option, index) => {
              const active = level === option;
              return (
                <button
                  key={option}
                  type="button"
                  onClick={() => setLevel(option)}
                  className={`rounded-xl px-2.5 py-2 text-left transition-colors ring-1 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                    active ? "ring-accent/40 bg-accent/[0.06]" : "ring-overlay/[0.08] bg-surface hover:bg-overlay/[0.02]"
                  }`}
                >
                  <div className="text-[11.5px] font-semibold leading-tight text-text-primary">{option}</div>
                  <div className="mt-1 text-[11px] text-text-tertiary">rung {index + 1}</div>
                </button>
              );
            })}
          </div>
        </div>

        <div>
          <Kicker>Supervisor</Kicker>
          <div className="mt-1.5 space-y-1.5">
            <button
              type="button"
              onClick={() => setSupervisor("human")}
              className={`w-full rounded-lg px-2.5 py-2 text-left transition-colors ring-1 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                supervisor === "human" ? "ring-accent/40 bg-accent/[0.06]" : "ring-overlay/[0.08] bg-surface hover:bg-overlay/[0.02]"
              }`}
            >
              <span className="flex items-center gap-2">
                <span className="rounded-md bg-overlay/[0.06] px-1.5 py-px text-[10px] font-semibold text-text-secondary">Human</span>
                <span className="text-[11px] text-text-tertiary">Top of the chain</span>
              </span>
            </button>
            {roster.map((agent) => {
              const active = supervisor === agent.id;
              return (
                <button
                  key={agent.id}
                  type="button"
                  onClick={() => setSupervisor(agent.id)}
                  className={`w-full rounded-lg px-2.5 py-2 text-left transition-colors ring-1 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                    active ? "ring-accent/40 bg-accent/[0.06]" : "ring-overlay/[0.08] bg-surface hover:bg-overlay/[0.02]"
                  }`}
                >
                  <div className="text-[12px] font-semibold leading-tight text-text-primary">{agent.name}</div>
                  <div className="mt-1 text-[11px] text-text-tertiary">{agent.line}</div>
                </button>
              );
            })}
          </div>
        </div>

        <div>
          <Kicker>Escalation chain</Kicker>
          <div className="mt-1.5 flex flex-wrap items-center gap-1.5 rounded-lg bg-overlay/[0.04] px-3 py-2 text-[11.5px] text-text-secondary">
            <span className="font-medium text-text-primary">{config.name}</span>
            {chain.map((step) => (
              <span key={step} className="inline-flex items-center gap-1.5">
                <span className="text-text-tertiary">→</span>
                <span className="text-text-primary">{step}</span>
              </span>
            ))}
            <span className="text-text-tertiary">→</span>
            <span className="rounded-md bg-overlay/[0.06] px-1.5 py-px text-[10px] font-semibold text-text-secondary">Human</span>
          </div>
        </div>
      </div>
    </Section>
  );
}

// ── The modal ────────────────────────────────────────────────────────────────

const RAIL_LABELS: Record<SectionId, string> = {
  identity: "Identity",
  role: "Role & Level",
  runtime: "Runtime",
  skills: "Skills",
  position: "Position",
};

function BuilderModal({ preview }: { preview: PreviewId }) {
  const config = PREVIEWS[preview].config;
  const [name, setName] = useState(config.name);
  const [roleId, setRoleId] = useState<string | null>(config.roleId);
  const [level, setLevel] = useState<Level>(config.level);
  const [customOpen, setCustomOpen] = useState(false);
  const [active, setActive] = useState<SectionId>("identity");
  const scrollRef = useRef<HTMLDivElement>(null);

  const ids: SectionId[] = config.position
    ? ["identity", "role", "runtime", "skills", "position"]
    : ["identity", "role", "runtime", "skills"];

  const live = { ...config, name };
  const readiness = sectionReadiness(live);
  const blocker = firstBlocker(live);
  const agyBlocked = blocker === "Install agy to continue";
  const primaryLabel = agyBlocked ? "Install agy to continue" : config.mode === "edit" ? "Save changes" : "Create agent";

  // Scroll-spy: the last section whose top has crossed the container's upper third.
  useEffect(() => {
    const container = scrollRef.current;
    if (!container) return;
    const compute = () => {
      const atTop = container.scrollTop <= 4;
      const atBottom = container.scrollTop >= container.scrollHeight - container.clientHeight - 4;
      if (atTop) return setActive(ids[0]);
      if (atBottom) return setActive(ids[ids.length - 1]);
      const top = container.getBoundingClientRect().top;
      const threshold = container.clientHeight / 3;
      let current: SectionId = ids[0];
      for (const id of ids) {
        const node = container.querySelector(`[data-builder-section="${id}"]`);
        if (!node) continue;
        if (node.getBoundingClientRect().top - top <= threshold) current = id;
      }
      setActive(current);
    };
    compute();
    container.addEventListener("scroll", compute, { passive: true });
    return () => container.removeEventListener("scroll", compute);
  }, [ids.join("|")]);

  // Each preview opens at its own section so the rail highlight is the state.
  useLayoutEffect(() => {
    const container = scrollRef.current;
    if (!container) return;
    const node = container.querySelector<HTMLElement>(`[data-builder-section="${config.focus}"]`);
    container.scrollTop = config.focus === "identity" ? 0 : node ? Math.max(0, node.offsetTop - 12) : 0;
  }, [preview, config.focus]);

  const jump = (id: SectionId) => {
    const container = scrollRef.current;
    const node = container?.querySelector<HTMLElement>(`[data-builder-section="${id}"]`);
    if (container && node) container.scrollTo({ top: Math.max(0, node.offsetTop - 12), behavior: "smooth" });
  };

  return (
    <div className="relative z-10 flex h-[700px] max-h-[90vh] w-[880px] shrink-0 flex-col overflow-hidden rounded-2xl bg-surface shadow-2xl ring-1 ring-overlay/[0.08]">
      <div className="flex h-11 shrink-0 items-center justify-between border-b border-overlay/[0.06] px-4">
        <div className="flex items-center gap-2">
          <Sparkles className="h-4 w-4 text-accent" />
          <span className="text-[13px] font-semibold tracking-tight text-text-primary">
            {config.mode === "edit" ? "Edit agent" : "New agent"}
          </span>
          <span className="rounded-md bg-overlay/[0.04] px-1.5 py-px text-[10px] font-medium text-text-muted">
            {config.mode === "edit" ? "update definition" : "saved to Library"}
          </span>
        </div>
        <button
          type="button"
          aria-label="Close builder"
          className="grid h-7 w-7 place-items-center rounded-md text-text-secondary hover:bg-overlay/[0.05] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
        >
          <X className="h-[15px] w-[15px]" />
        </button>
      </div>

      <div className="flex min-h-0 flex-1">
        <nav aria-label="Builder sections" className="w-[180px] shrink-0 space-y-1 border-r border-border bg-fill p-2.5">
          {ids.map((id) => {
            const isActive = active === id;
            return (
              <button
                key={id}
                type="button"
                onClick={() => jump(id)}
                aria-current={isActive ? "true" : undefined}
                className={`flex h-7 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-[12px] transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                  isActive
                    ? "bg-accent/[0.10] font-semibold text-accent"
                    : "text-text-secondary hover:bg-overlay/[0.04] hover:text-text-primary"
                }`}
              >
                <Dot state={readiness[id]} />
                <span className="min-w-0 truncate">{RAIL_LABELS[id]}</span>
              </button>
            );
          })}
        </nav>

        <div ref={scrollRef} className="relative min-h-0 flex-1 overflow-y-auto px-6 py-5">
          <IdentitySection config={config} name={name} setName={setName} />

          <Section
            id="role"
            title="Role & Level"
            actions={
              <span className="flex items-center gap-2 text-[11px] font-medium">
                <button
                  type="button"
                  onClick={() => {
                    setRoleId(null);
                    setCustomOpen(false);
                  }}
                  className={`transition-colors ${roleId === null && !customOpen ? "text-accent" : "text-text-tertiary hover:text-text-secondary"}`}
                >
                  No role
                </button>
                <span className="text-text-tertiary" aria-hidden="true">
                  ·
                </span>
                <button
                  type="button"
                  onClick={() => {
                    setCustomOpen(true);
                    setRoleId(null);
                  }}
                  className={`inline-flex items-center gap-1 transition-colors ${customOpen ? "text-accent" : "text-text-tertiary hover:text-text-secondary"}`}
                >
                  <Plus className="h-3 w-3" />
                  Custom…
                </button>
              </span>
            }
          >
            <RoleLevelBody
              roleId={roleId}
              setRoleId={setRoleId}
              level={level}
              setLevel={setLevel}
              customOpen={customOpen}
              setCustomOpen={setCustomOpen}
            />
          </Section>

          <RuntimeSection config={config} />
          <SkillsSection config={config} />
          {config.position && <PositionSection config={config} />}
          <div className="h-8" />
        </div>
      </div>

      <div className="flex shrink-0 items-center justify-between gap-3 border-t border-overlay/[0.07] bg-surface px-5 py-2.5">
        <span className={`text-[11.5px] ${agyBlocked ? "text-danger" : "text-text-tertiary"}`}>
          {blocker ?? (config.mode === "edit" ? "Ready to save" : "Ready to create")}
        </span>
        <div className="flex items-center gap-2">
          <button
            type="button"
            className="rounded-lg px-3.5 py-1.5 text-[12.5px] font-medium text-text-secondary hover:bg-overlay/[0.05] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={blocker !== null}
            className="inline-flex items-center gap-1.5 rounded-lg bg-accent px-4 py-1.5 text-[12.5px] font-semibold text-white hover:bg-accent-hover disabled:cursor-not-allowed disabled:opacity-45 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
          >
            <Sparkles className="h-3.5 w-3.5" />
            {primaryLabel}
          </button>
        </div>
      </div>
    </div>
  );
}

// ── App-shell backdrop (context only, not canon) ─────────────────────────────

function ShellBackdrop() {
  const rows = [
    { name: "Detoro", line: "Principal · Lead", tone: "indigo" },
    { name: "Mellow", line: "Senior · Reviewer", tone: "amber" },
    { name: "Tiësto", line: "Principal · Implementer", tone: "magenta" },
    { name: "Dew", line: "Senior · Implementer", tone: "teal" },
  ];
  return (
    <div aria-hidden="true" className="absolute inset-0 flex select-none overflow-hidden">
      <div className="flex w-11 shrink-0 flex-col items-center gap-2 bg-sidebar py-3">
        <span className="h-7 w-7 rounded-lg bg-accent/70" />
        {[0, 1, 2].map((i) => (
          <span key={i} className="h-7 w-7 rounded-lg bg-overlay/[0.06]" />
        ))}
      </div>
      <div className="flex w-[220px] shrink-0 flex-col border-r border-border bg-sidebar">
        <div className="flex h-11 items-center justify-between border-b border-border px-3">
          <span className="text-[12.5px] font-semibold text-text-primary">codeup</span>
          <Settings2 className="h-3.5 w-3.5 text-text-muted" />
        </div>
        <div className="flex items-center gap-2 border-b border-border px-3 py-2">
          <Search className="h-3.5 w-3.5 text-text-tertiary" />
          <span className="text-[11.5px] text-text-muted">Search agents</span>
        </div>
        <div className="space-y-1 p-2">
          {rows.map((row) => (
            <div key={row.name} className="flex items-center gap-2.5 rounded-lg px-2 py-1.5">
              <span
                className="grid h-7 w-7 shrink-0 place-items-center rounded-lg text-[12px] font-bold text-white"
                style={{ backgroundColor: `var(--color-agent-${row.tone})` }}
              >
                {row.name[0]}
              </span>
              <span className="min-w-0 flex-1 leading-tight">
                <span className="block truncate text-[12.5px] font-semibold text-text-primary">{row.name}</span>
                <span className="block truncate text-[10.5px] text-text-muted">{row.line}</span>
              </span>
              <span className="h-2 w-2 shrink-0 rounded-full bg-live" />
            </div>
          ))}
        </div>
      </div>
      <div className="min-w-0 flex-1 bg-canvas">
        <div className="flex h-11 items-center gap-2 border-b border-border px-4">
          <span className="rounded-md bg-overlay/[0.06] px-2 py-1 text-[11px] text-text-secondary">Detoro</span>
          <span className="rounded-md px-2 py-1 text-[11px] text-text-tertiary">Tiësto</span>
          <span className="rounded-md px-2 py-1 text-[11px] text-text-tertiary">Orbit</span>
        </div>
        <div className="space-y-2 p-4">
          {[88, 64, 76, 40, 58].map((width, i) => (
            <span key={i} className="block h-2.5 rounded bg-overlay/[0.05]" style={{ width: `${width}%` }} />
          ))}
        </div>
      </div>
    </div>
  );
}

// ── Artboards ────────────────────────────────────────────────────────────────

function Artboard({ label, caption, children }: { label: string; caption: string; children: React.ReactNode }) {
  return (
    <figure className="m-0">
      <figcaption className="mb-2 flex items-baseline gap-2">
        <span className="font-mono text-[10px] font-semibold uppercase tracking-wider text-text-secondary">{label}</span>
        <span className="text-[10.5px] text-text-tertiary">{caption}</span>
      </figcaption>
      {children}
    </figure>
  );
}

function RoleCustomArtboard() {
  const [roleId, setRoleId] = useState<string | null>(null);
  const [level, setLevel] = useState<Level>("Senior");
  const [customOpen, setCustomOpen] = useState(true);
  return (
    <div className="w-[700px] overflow-hidden rounded-2xl bg-surface ring-1 ring-overlay/[0.08]">
      <div className="px-6 py-5">
        <Section
          id="role-artboard"
          title="Role & Level"
          first
          actions={
            <span className="flex items-center gap-2 text-[11px] font-medium">
              <button
                type="button"
                onClick={() => {
                  setRoleId(null);
                  setCustomOpen(false);
                }}
                className={`transition-colors ${roleId === null && !customOpen ? "text-accent" : "text-text-tertiary hover:text-text-secondary"}`}
              >
                No role
              </button>
              <span className="text-text-tertiary" aria-hidden="true">
                ·
              </span>
              <button
                type="button"
                onClick={() => setCustomOpen(true)}
                className={`inline-flex items-center gap-1 transition-colors ${customOpen ? "text-accent" : "text-text-tertiary hover:text-text-secondary"}`}
              >
                <Plus className="h-3 w-3" />
                Custom…
              </button>
            </span>
          }
        >
          <RoleLevelBody
            roleId={roleId}
            setRoleId={setRoleId}
            level={level}
            setLevel={setLevel}
            customOpen={customOpen}
            setCustomOpen={setCustomOpen}
          />
        </Section>
      </div>
    </div>
  );
}

function FiveTileArtboard() {
  const [runtime, setRuntime] = useState<RuntimeKind>("opencode");
  return (
    <div className="w-[700px] overflow-hidden rounded-2xl bg-surface ring-1 ring-overlay/[0.08]">
      <div className="px-6 py-5">
        <Kicker>Runtime</Kicker>
        <div className="mt-2.5">
          <RuntimeTiles
            value={runtime}
            onChange={setRuntime}
            kinds={["claude-code", "codex", "antigravity", "opencode", "muse-spark"]}
          />
        </div>
        <p className="mt-2 text-[10.5px] text-text-tertiary">Chat agent and Orchestrator are coming soon.</p>
        <p className="mt-3 text-[10.5px] leading-relaxed text-text-tertiary">
          Five kinds fill the 3-column grid over two rows; the trailing cell stays empty rather than stretching the last tile.
          opencode and Muse Spark render only once the CliKind union carries them, so this artboard is the target shape, not
          today's state.
        </p>
      </div>
    </div>
  );
}

// ── Canvas ───────────────────────────────────────────────────────────────────

const PREVIEW_ORDER: PreviewId[] = ["new-empty", "new-filled", "edit-advanced", "edit-position", "edit-antigravity"];

function readQuery(key: string): string | null {
  if (typeof window === "undefined") return null;
  return new URLSearchParams(window.location.search).get(key);
}

export default function AgentBuilderCanon() {
  const [preview, setPreview] = useState<PreviewId>(() => {
    const requested = readQuery("state");
    return PREVIEW_ORDER.includes(requested as PreviewId) ? (requested as PreviewId) : "new-filled";
  });
  const [dark, setDark] = useState(() => readQuery("theme") === "dark");

  const current = PREVIEWS[preview];

  return (
    <div className={`${dark ? "dark " : ""}min-h-screen bg-canvas font-sans text-text-primary antialiased`}>
      <div className="flex min-h-screen flex-col gap-3 p-3">
        <header className="flex shrink-0 flex-wrap items-center gap-4 rounded-[12px] bg-sidebar px-4 py-3 ring-1 ring-border">
          <div className="flex min-w-0 items-center gap-3">
            <span className="grid h-9 w-9 shrink-0 place-items-center rounded-[10px] bg-accent/[0.14] text-accent ring-1 ring-accent/25">
              <Sparkles className="h-[18px] w-[18px]" />
            </span>
            <div className="min-w-0">
              <h1 className="truncate text-[15px] font-semibold tracking-[-0.015em]">Agent Builder</h1>
              <p className="mt-0.5 truncate text-[10.5px] text-text-muted">
                880px modal, section rail, scroll-spy. Canon for task agent-builder-canon.
              </p>
            </div>
          </div>

          <div className="ml-auto flex min-w-0 flex-wrap items-center gap-3" aria-label="Canon state controls">
            <div className="flex items-center gap-2">
              <span className="text-[10px] font-bold uppercase tracking-wider text-text-tertiary">State</span>
              <div role="radiogroup" aria-label="Preview state" className="flex rounded-lg bg-overlay/[0.04] p-0.5">
                {PREVIEW_ORDER.map((id) => (
                  <button
                    key={id}
                    type="button"
                    role="radio"
                    aria-checked={preview === id}
                    onClick={() => setPreview(id)}
                    className={`rounded-[7px] px-2.5 py-1 text-[11.5px] transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                      preview === id ? "bg-surface font-semibold text-text-primary shadow-sm" : "text-text-secondary hover:text-text-primary"
                    }`}
                  >
                    {PREVIEWS[id].label}
                  </button>
                ))}
              </div>
            </div>

            <span className="h-6 w-px shrink-0 bg-border" />

            <div className="flex items-center gap-2">
              <span className="text-[10px] font-bold uppercase tracking-wider text-text-tertiary">Theme</span>
              <Segmented
                label="Theme"
                value={dark ? "Dark" : "Light"}
                options={["Light", "Dark"] as const}
                onChange={(next) => setDark(next === "Dark")}
              />
            </div>
          </div>
        </header>

        <p className="shrink-0 px-1 text-[11.5px] text-text-secondary">{current.note}</p>

        <div className="min-h-0 flex-1 overflow-auto p-1 pb-6">
          <div className="flex w-[1180px] flex-col gap-6">
            <Artboard label="A · modal" caption="880 × ≤90vh over the app shell">
              <div className="relative flex h-[760px] w-[1180px] shrink-0 items-center justify-center overflow-hidden rounded-2xl bg-canvas ring-1 ring-border">
                <ShellBackdrop />
                <div className="absolute inset-0 bg-black/30" aria-hidden="true" />
                <BuilderModal key={preview} preview={preview} />
              </div>
            </Artboard>

            <Artboard label="B · role editor" caption="Role & Level with Custom… open">
              <RoleCustomArtboard />
            </Artboard>

            <Artboard label="C · five runtimes" caption="Target grid once opencode and Muse Spark land">
              <FiveTileArtboard />
            </Artboard>
          </div>
        </div>
      </div>
    </div>
  );
}
