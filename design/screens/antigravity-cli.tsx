import { useState } from "react";
import {
  AlertTriangle,
  Ban,
  Check,
  ChevronDown,
  CircleHelp,
  Command,
  Folder,
  Library,
  MessageSquare,
  Moon,
  RefreshCw,
  Search,
  Settings2,
  Sparkles,
  Sun,
  Terminal,
  Users,
  Waypoints,
  X,
} from "lucide-react";

export const meta = { title: "Antigravity CLI — Builder and provider labels" };

type Preview = "ready" | "auto" | "missing" | "bypass" | "long";
type Effort = "Auto" | "low" | "medium" | "high";
type ExecutionMode = "Default" | "Accept edits" | "Plan" | "Bypass permissions";

const namedModel = "gemini-3.8-pro-high";
const longModel = "gemini-3.8-pro-experimental-context-extended";

const modeHelp: Record<ExecutionMode, string> = {
  Default: "Pauses for diff review before applying changes.",
  "Accept edits": "Applies file edits automatically. Shell and web actions still ask.",
  Plan: "Starts in planning mode before making changes.",
  "Bypass permissions": "Skips every permission prompt, including shell and web actions.",
};

function Avatar({ name, tone = "indigo" }: { name: string; tone?: "indigo" | "teal" | "magenta" }) {
  return (
    <span
      className="grid h-7 w-7 shrink-0 place-items-center rounded-lg text-[12px] font-bold text-white"
      style={{ backgroundColor: `var(--color-agent-${tone})` }}
      aria-hidden="true"
    >
      {name[0]}
    </span>
  );
}

function ProviderChip({ model, className = "" }: { model: string; className?: string }) {
  const full = model.trim() ? `Antigravity · ${model.trim()}` : "Antigravity";
  return (
    <span
      className={`min-w-0 truncate text-[10px] font-medium tracking-tight text-text-tertiary ${className}`}
      title={model.trim() || "Antigravity"}
      aria-label={full}
    >
      {full}
    </span>
  );
}

function RailButton({ active, label, children }: { active?: boolean; label: string; children: React.ReactNode }) {
  return (
    <button
      type="button"
      aria-label={label}
      className={`grid h-9 w-9 place-items-center rounded-lg transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
        active ? "bg-accent text-white" : "text-text-secondary hover:bg-overlay/[0.06] hover:text-text-primary"
      }`}
    >
      {children}
    </button>
  );
}

function RosterRow({ model, selected = false }: { model: string; selected?: boolean }) {
  return (
    <button
      type="button"
      className={`flex min-h-11 w-full items-start gap-2.5 rounded-lg px-2 py-1.5 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
        selected ? "bg-accent/[0.08] ring-1 ring-accent/20" : "hover:bg-overlay/[0.04]"
      }`}
    >
      <Avatar name="Nova" tone="magenta" />
      <span className="min-w-0 flex-1 leading-tight">
        <span className="flex min-w-0 items-center gap-1.5 text-[12.5px] font-semibold">
          <span className="min-w-0 flex-1 truncate text-text-primary">Nova</span>
          <ProviderChip model={model} className="max-w-[104px] shrink-0" />
        </span>
        <span className="mt-0.5 flex min-w-0 items-center gap-1 text-[10.5px] text-text-muted">
          <span className="truncate">Senior · Implementer</span>
          <span aria-hidden="true">·</span>
          <span className="shrink-0">↪ Aoki</span>
        </span>
      </span>
      <span className="mt-1.5 h-2 w-2 shrink-0 rounded-full bg-live" aria-label="Running" />
    </button>
  );
}

function Roster({ model }: { model: string }) {
  return (
    <aside className="hidden min-h-0 w-[252px] shrink-0 flex-col border-r border-border bg-sidebar md:flex">
      <div className="flex h-11 items-center justify-between border-b border-border px-3">
        <span className="text-[12.5px] font-semibold text-text-primary">codeup</span>
        <button
          type="button"
          aria-label="Workspace settings"
          className="grid h-7 w-7 place-items-center rounded-md text-text-muted hover:bg-overlay/[0.05] hover:text-text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
        >
          <Settings2 className="h-3.5 w-3.5" />
        </button>
      </div>
      <div className="flex items-center gap-2 border-b border-border px-3 py-2">
        <Search className="h-3.5 w-3.5 text-text-tertiary" />
        <span className="text-[11.5px] text-text-muted">Filter agents</span>
        <span className="ml-auto text-[10px] text-text-tertiary">⌘K</span>
      </div>
      <div className="min-h-0 flex-1 overflow-hidden px-1.5 py-2">
        <div className="mb-1 flex items-center justify-between px-2 text-[10px] font-semibold text-text-tertiary">
          <span>Agents</span>
          <span>3</span>
        </div>
        <RosterRow model={model} selected />
        <button type="button" className="mt-0.5 flex min-h-11 w-full items-start gap-2.5 rounded-lg px-2 py-1.5 text-left hover:bg-overlay/[0.04]">
          <Avatar name="Aoki" tone="indigo" />
          <span className="min-w-0 flex-1 leading-tight">
            <span className="flex min-w-0 items-center gap-1.5 text-[12.5px] font-semibold">
              <span className="min-w-0 flex-1 truncate">Aoki</span>
              <span className="shrink-0 text-[10px] font-medium tracking-tight text-text-tertiary">Codex · gpt-5.6-sol</span>
            </span>
            <span className="mt-0.5 block truncate text-[10.5px] text-text-muted">Principal · Lead</span>
          </span>
          <span className="mt-1.5 h-2 w-2 shrink-0 rounded-full bg-live" aria-label="Running" />
        </button>
        <button type="button" className="mt-0.5 flex min-h-11 w-full items-start gap-2.5 rounded-lg px-2 py-1.5 text-left hover:bg-overlay/[0.04]">
          <Avatar name="Dew" tone="teal" />
          <span className="min-w-0 flex-1 leading-tight">
            <span className="flex min-w-0 items-center gap-1.5 text-[12.5px] font-semibold">
              <span className="min-w-0 flex-1 truncate">Dew</span>
              <span className="shrink-0 text-[10px] font-medium tracking-tight text-text-tertiary">Claude · opus-5</span>
            </span>
            <span className="mt-0.5 block truncate text-[10.5px] text-text-muted">Senior · Implementer</span>
          </span>
          <span className="mt-1.5 h-2 w-2 shrink-0 rounded-full bg-text-tertiary" aria-label="Idle" />
        </button>
      </div>
      <div className="border-t border-border p-2.5 text-[10.5px] leading-relaxed text-text-tertiary">
        Provider labels replace the generic terminal glyph.
      </div>
    </aside>
  );
}

function SupervisorPicker({ model }: { model: string }) {
  return (
    <section className="absolute right-4 top-14 z-20 hidden w-[330px] overflow-hidden rounded-xl bg-surface-raised shadow-lg xl:block">
      <div className="flex h-10 items-center justify-between border-b border-border px-3">
        <div>
          <div className="text-[12px] font-semibold text-text-primary">Change supervisor</div>
          <div className="text-[9.5px] text-text-tertiary">Nova reports through this agent</div>
        </div>
        <button type="button" aria-label="Close supervisor picker" className="grid h-7 w-7 place-items-center rounded-md text-text-muted hover:bg-overlay/[0.05]">
          <X className="h-3.5 w-3.5" />
        </button>
      </div>
      <div className="p-1.5">
        <button
          type="button"
          className="flex w-full items-center gap-2.5 rounded-lg bg-accent/[0.09] px-2 py-1.5 text-left ring-1 ring-accent/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
        >
          <Avatar name="Aoki" tone="indigo" />
          <span className="min-w-0 flex-1">
            <span className="block truncate text-[12.5px] font-semibold leading-tight text-text-primary">Aoki</span>
            <span className="flex min-w-0 items-center gap-1 text-[10.5px] leading-tight text-text-tertiary">
              <Waypoints className="h-2.5 w-2.5 shrink-0" />
              <span className="min-w-0 flex-1 truncate">Lead · Principal</span>
              <span className="shrink-0">· Codex · gpt-5.6-sol</span>
            </span>
          </span>
          <Check className="h-3.5 w-3.5 shrink-0 text-accent" />
        </button>
        <button
          type="button"
          className="mt-0.5 flex w-full items-center gap-2.5 rounded-lg px-2 py-1.5 text-left hover:bg-overlay/[0.04] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
        >
          <Avatar name="Nova" tone="magenta" />
          <span className="min-w-0 flex-1">
            <span className="block truncate text-[12.5px] font-semibold leading-tight text-text-primary">Nova</span>
            <span className="flex min-w-0 items-center gap-1 text-[10.5px] leading-tight text-text-tertiary">
              <Waypoints className="h-2.5 w-2.5 shrink-0" />
              <span className="min-w-0 flex-1 truncate">Implementer · Senior</span>
              <span aria-hidden="true">·</span>
              <ProviderChip model={model} className="max-w-[145px] shrink-0" />
            </span>
          </span>
        </button>
        <button
          type="button"
          disabled
          aria-disabled="true"
          className="mt-0.5 flex w-full cursor-not-allowed items-center gap-2.5 rounded-lg px-2 py-1.5 text-left opacity-45"
        >
          <Avatar name="Dew" tone="teal" />
          <span className="min-w-0 flex-1">
            <span className="block truncate text-[12.5px] font-semibold leading-tight text-text-primary">Dew</span>
            <span className="flex items-center gap-1 text-[10.5px] leading-tight text-text-tertiary">
              <Waypoints className="h-2.5 w-2.5 shrink-0" />
              <span className="truncate">Implementer · Senior · Claude · opus-5</span>
            </span>
          </span>
          <span className="inline-flex shrink-0 items-center gap-1 text-[9.5px] text-text-tertiary">
            <Ban className="h-2.5 w-2.5" /> Descendant
          </span>
        </button>
      </div>
    </section>
  );
}

function TypeControl() {
  return (
    <section>
      <div className="mb-2 text-[10px] font-bold uppercase tracking-wider text-text-tertiary">Type</div>
      <div className="grid grid-cols-3 gap-2">
        <button type="button" className="rounded-xl bg-accent/[0.06] p-2 text-left ring-1 ring-accent/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent">
          <Terminal className="mb-1 h-4 w-4 text-accent" />
          <div className="text-[12px] font-semibold text-text-primary">CLI agent</div>
        </button>
        <button type="button" disabled className="cursor-not-allowed rounded-xl bg-overlay/[0.02] p-2 text-left opacity-55 ring-1 ring-overlay/[0.06]">
          <MessageSquare className="mb-1 h-4 w-4 text-text-tertiary" />
          <div className="text-[12px] font-semibold text-text-primary">Chat agent</div>
          <span className="text-[9px] font-semibold uppercase tracking-wide text-text-muted">Soon</span>
        </button>
        <button type="button" disabled className="cursor-not-allowed rounded-xl bg-overlay/[0.02] p-2 text-left opacity-55 ring-1 ring-overlay/[0.06]">
          <Waypoints className="mb-1 h-4 w-4 text-text-tertiary" />
          <div className="text-[12px] font-semibold text-text-primary">Orchestrator</div>
          <span className="text-[9px] font-semibold uppercase tracking-wide text-text-muted">Soon</span>
        </button>
      </div>
      <div role="radiogroup" aria-label="CLI kind" className="mt-2 grid grid-cols-4 gap-1 rounded-xl bg-overlay/[0.04] p-1">
        {[
          ["claude-code", "Claude Code"],
          ["codex", "Codex"],
          ["antigravity", "Antigravity"],
          ["custom", "Custom"],
        ].map(([value, label]) => {
          const selected = value === "antigravity";
          const disabled = value === "custom";
          return (
            <button
              key={value}
              type="button"
              role="radio"
              aria-checked={selected}
              disabled={disabled}
              className={`min-w-0 rounded-lg px-1 py-1.5 text-[11.5px] transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                disabled
                  ? "cursor-not-allowed text-text-tertiary"
                  : selected
                    ? "bg-surface font-semibold text-text-primary shadow-sm"
                    : "text-text-secondary hover:bg-overlay/[0.03]"
              }`}
            >
              <span className="block truncate">{label}</span>
              {disabled && <span className="block text-[8px] font-semibold uppercase tracking-wide">Soon</span>}
            </button>
          );
        })}
      </div>
    </section>
  );
}

function AntigravitySettings({
  model,
  setModel,
  effort,
  setEffort,
  mode,
  setMode,
  missing,
}: {
  model: string;
  setModel: (model: string) => void;
  effort: Effort;
  setEffort: (effort: Effort) => void;
  mode: ExecutionMode;
  setMode: (mode: ExecutionMode) => void;
  missing: boolean;
}) {
  const bypass = mode === "Bypass permissions";
  return (
    <section>
      <div className="mb-2 flex items-center justify-between">
        <div className="text-[10px] font-bold uppercase tracking-wider text-text-tertiary">Antigravity</div>
        <span className={`inline-flex items-center gap-1 text-[10px] font-medium ${missing ? "text-danger" : "text-text-muted"}`}>
          {missing ? <AlertTriangle className="h-3 w-3" /> : <Check className="h-3 w-3 text-live" />}
          {missing ? "agy not found" : "agy available"}
        </span>
      </div>

      {missing && (
        <div role="alert" className="mb-2 rounded-xl bg-danger/[0.09] px-3 py-2.5 text-danger">
          <div className="flex items-start gap-2">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
            <div className="min-w-0 flex-1">
              <div className="text-[11.5px] font-semibold">Antigravity CLI is not available</div>
              <p className="mt-0.5 text-[10.5px] leading-relaxed text-danger">
                Install the CLI from the Antigravity documentation, then make sure <span className="font-mono">agy</span> is on your login-shell PATH.
              </p>
              <div className="mt-2 flex items-center gap-2">
                <button type="button" className="inline-flex h-7 items-center gap-1.5 rounded-md bg-danger px-2.5 text-[10.5px] font-semibold text-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-danger">
                  <CircleHelp className="h-3 w-3" /> Installation guide
                </button>
                <button type="button" className="inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-[10.5px] font-semibold text-danger hover:bg-danger/[0.08] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-danger">
                  <RefreshCw className="h-3 w-3" /> Check again
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      <div className="overflow-hidden rounded-xl bg-surface ring-1 ring-overlay/[0.08]">
        <div className="border-b border-border px-3 py-2.5">
          <div className="flex items-center justify-between gap-3">
            <label htmlFor="agy-model" className="shrink-0 text-[12.5px] text-text-secondary">Model</label>
            <input
              id="agy-model"
              value={model}
              onChange={(event) => setModel(event.target.value)}
              placeholder="Auto (authenticated default)"
              className="min-w-0 flex-1 bg-transparent text-right font-mono text-[12px] text-text-primary outline-none placeholder:text-text-tertiary focus-visible:rounded-sm focus-visible:ring-2 focus-visible:ring-accent"
            />
          </div>
          <p className="mt-1.5 text-[10px] leading-relaxed text-text-tertiary">
            Leave blank for Antigravity to choose from your authenticated models.
          </p>
        </div>

        <div className="border-b border-border px-3 py-2.5">
          <div className="flex items-center justify-between gap-3">
            <span className="text-[12.5px] text-text-secondary">Effort</span>
            <div role="radiogroup" aria-label="Effort" className="flex rounded-lg bg-overlay/[0.04] p-0.5">
              {(["Auto", "low", "medium", "high"] as Effort[]).map((value) => (
                <button
                  key={value}
                  type="button"
                  role="radio"
                  aria-checked={effort === value}
                  onClick={() => setEffort(value)}
                  className={`rounded-[7px] px-2.5 py-1 text-[11.5px] transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                    effort === value ? "bg-surface font-semibold text-text-primary shadow-sm" : "text-text-secondary hover:text-text-primary"
                  }`}
                >
                  {value}
                </button>
              ))}
            </div>
          </div>
          <p className="mt-1.5 text-[10px] leading-relaxed text-text-tertiary">Auto omits the effort flag.</p>
        </div>

        <div className="px-3 py-2.5">
          <div className="flex items-center justify-between gap-3">
            <label htmlFor="agy-mode" className="shrink-0 text-[12.5px] text-text-secondary">Execution mode</label>
            <div className="relative min-w-[190px]">
              <select
                id="agy-mode"
                value={mode}
                onChange={(event) => setMode(event.target.value as ExecutionMode)}
                className={`h-7 w-full appearance-none rounded-lg bg-overlay/[0.04] pl-2.5 pr-7 text-[11.5px] font-medium outline-none focus-visible:ring-2 focus-visible:ring-accent ${bypass ? "text-danger" : "text-text-primary"}`}
              >
                <option>Default</option>
                <option>Accept edits</option>
                <option>Plan</option>
                <option>Bypass permissions</option>
              </select>
              <ChevronDown className="pointer-events-none absolute right-2 top-2 h-3 w-3 text-text-muted" />
            </div>
          </div>
          <p className={`mt-1.5 text-[10px] leading-relaxed ${bypass ? "text-danger" : "text-text-tertiary"}`}>
            {modeHelp[mode]}
          </p>
          {bypass && (
            <div className="mt-2 flex items-start gap-2 rounded-lg bg-danger/[0.09] px-2.5 py-2 text-[10.5px] leading-relaxed text-danger">
              <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
              <span><strong>Use only in workspaces you trust.</strong> This disables Antigravity permission checks.</span>
            </div>
          )}
        </div>
      </div>
      <p className="mt-2 text-[10px] leading-relaxed text-text-tertiary">
        Token filtering and sandbox controls are not available for Antigravity in this version.
      </p>
    </section>
  );
}

function BuilderPanel({ preview, model, setModel }: { preview: Preview; model: string; setModel: (model: string) => void }) {
  const [effort, setEffort] = useState<Effort>("Auto");
  const [mode, setMode] = useState<ExecutionMode>(preview === "bypass" ? "Bypass permissions" : "Default");
  const missing = preview === "missing";

  return (
    <div className="relative z-10 flex h-[calc(100%-32px)] max-h-[760px] w-[536px] max-w-[calc(100%-32px)] flex-col overflow-hidden rounded-2xl bg-surface shadow-popover">
      <div className="flex h-11 shrink-0 items-center justify-between border-b border-border px-4">
        <div className="flex items-center gap-2">
          <Sparkles className="h-4 w-4 text-accent" />
          <span className="text-[13px] font-semibold tracking-tight text-text-primary">New agent</span>
          <span className="rounded-md bg-overlay/[0.04] px-1.5 py-px text-[10px] font-medium text-text-muted">saved to Library</span>
        </div>
        <button type="button" aria-label="Close Builder" className="grid h-7 w-7 place-items-center rounded-md text-text-secondary hover:bg-overlay/[0.05] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent">
          <X className="h-[15px] w-[15px]" />
        </button>
      </div>

      <div className="min-h-0 flex-1 space-y-3.5 overflow-y-auto px-5 py-4">
        <section>
          <div className="mb-2 text-[10px] font-bold uppercase tracking-wider text-text-tertiary">Identity</div>
          <div className="flex items-center gap-2.5">
            <span className="grid h-10 w-10 shrink-0 place-items-center rounded-[10px] text-[15px] font-bold text-white" style={{ backgroundColor: "var(--color-agent-magenta)" }}>N</span>
            <div className="min-w-0 flex-1 space-y-1">
              <input defaultValue="Nova" aria-label="Agent name" className="w-full border-b border-overlay/10 bg-transparent pb-0.5 text-[14px] font-semibold tracking-tight text-text-primary outline-none focus:border-accent" />
              <div className="truncate text-[11.5px] text-text-muted">Implementer · Senior</div>
            </div>
          </div>
        </section>

        <TypeControl />
        <AntigravitySettings
          model={model}
          setModel={setModel}
          effort={effort}
          setEffort={setEffort}
          mode={mode}
          setMode={setMode}
          missing={missing}
        />

        <section>
          <div className="mb-2 text-[10px] font-bold uppercase tracking-wider text-text-tertiary">Custom arguments</div>
          <div className="flex items-center rounded-xl bg-surface px-3 py-2 ring-1 ring-overlay/[0.08]">
            <Command className="mr-2 h-3.5 w-3.5 text-text-tertiary" />
            <input aria-label="Custom arguments" placeholder="Optional arguments appended after Conclave settings" className="min-w-0 flex-1 bg-transparent font-mono text-[11px] text-text-primary outline-none placeholder:text-text-tertiary focus-visible:rounded-sm focus-visible:ring-2 focus-visible:ring-accent" />
          </div>
        </section>
      </div>

      <div className="flex shrink-0 items-center justify-between border-t border-border bg-surface px-5 py-2.5">
        <span className="text-[10px] text-text-tertiary">Model: {model.trim() ? "named" : "Auto"}</span>
        <div className="flex items-center gap-2">
          <button type="button" className="rounded-lg px-3.5 py-1.5 text-[12.5px] font-medium text-text-secondary hover:bg-overlay/[0.05] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent">Cancel</button>
          <button
            type="button"
            disabled={missing}
            className="inline-flex items-center gap-1.5 rounded-lg bg-accent px-4 py-1.5 text-[12.5px] font-semibold text-white hover:bg-accent-hover disabled:cursor-not-allowed disabled:opacity-45 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-offset-2 focus-visible:ring-offset-surface"
          >
            <Sparkles className="h-3.5 w-3.5" />
            {missing ? "Install agy to continue" : "Create agent"}
          </button>
        </div>
      </div>
    </div>
  );
}

function ScenarioToolbar({ preview, setPreview, dark, setDark }: { preview: Preview; setPreview: (preview: Preview) => void; dark: boolean; setDark: (dark: boolean) => void }) {
  const items: { id: Preview; label: string }[] = [
    { id: "ready", label: "Named model" },
    { id: "auto", label: "Auto model" },
    { id: "missing", label: "Missing agy" },
    { id: "bypass", label: "Bypass" },
    { id: "long", label: "Long label" },
  ];
  return (
    <div className="flex min-h-11 shrink-0 items-center gap-3 overflow-x-auto border-b border-border bg-surface px-3">
      <div className="flex shrink-0 items-center gap-2 text-[11px] font-semibold text-text-secondary">
        <Terminal className="h-3.5 w-3.5 text-accent" /> Antigravity canon
      </div>
      <div role="radiogroup" aria-label="Preview state" className="flex shrink-0 rounded-lg bg-overlay/[0.04] p-0.5">
        {items.map((item) => (
          <button
            key={item.id}
            type="button"
            role="radio"
            aria-checked={preview === item.id}
            onClick={() => setPreview(item.id)}
            className={`rounded-[7px] px-2.5 py-1 text-[10.5px] transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
              preview === item.id ? "bg-surface font-semibold text-text-primary shadow-sm" : "text-text-secondary hover:text-text-primary"
            }`}
          >
            {item.label}
          </button>
        ))}
      </div>
      <div className="ml-auto flex shrink-0 items-center gap-2">
        <span className="hidden text-[10px] text-text-tertiary lg:inline">Builder · Roster · Supervisor picker</span>
        <button
          type="button"
          onClick={() => setDark(!dark)}
          aria-label={dark ? "Preview light appearance" : "Preview dark appearance"}
          className="grid h-7 w-7 place-items-center rounded-md text-text-secondary hover:bg-overlay/[0.05] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
        >
          {dark ? <Sun className="h-3.5 w-3.5" /> : <Moon className="h-3.5 w-3.5" />}
        </button>
      </div>
    </div>
  );
}

export default function AntigravityCliScreen() {
  const initialPreview = (() => {
    if (typeof window === "undefined") return "ready" as Preview;
    const value = new URLSearchParams(window.location.search).get("state");
    return (["ready", "auto", "missing", "bypass", "long"] as Preview[]).includes(value as Preview)
      ? (value as Preview)
      : "ready";
  })();
  const [preview, setPreviewState] = useState<Preview>(initialPreview);
  const [dark, setDark] = useState(() =>
    typeof window !== "undefined" && new URLSearchParams(window.location.search).get("theme") === "dark",
  );
  const [model, setModel] = useState(() => {
    if (initialPreview === "auto") return "";
    if (initialPreview === "long") return longModel;
    return namedModel;
  });

  function setPreview(next: Preview) {
    setPreviewState(next);
    if (next === "auto") setModel("");
    else if (next === "long") setModel(longModel);
    else setModel(namedModel);
  }

  return (
    <div className={dark ? "dark h-screen overflow-hidden bg-canvas font-sans text-text-primary" : "h-screen overflow-hidden bg-canvas font-sans text-text-primary"}>
      <ScenarioToolbar preview={preview} setPreview={setPreview} dark={dark} setDark={setDark} />

      <div className="flex h-[calc(100vh-44px)] min-h-0 flex-col bg-canvas">
        <div className="flex h-10 shrink-0 items-center border-b border-border bg-surface px-3">
          <div className="mr-4 flex gap-1.5" aria-hidden="true">
            <span className="h-2.5 w-2.5 rounded-full bg-danger" />
            <span className="h-2.5 w-2.5 rounded-full bg-waiting" />
            <span className="h-2.5 w-2.5 rounded-full bg-live" />
          </div>
          <Folder className="h-3.5 w-3.5 text-text-muted" />
          <span className="ml-1.5 text-[11.5px] font-semibold text-text-secondary">codeup</span>
          <span className="mx-1.5 text-text-tertiary">/</span>
          <span className="text-[11.5px] text-text-muted">Agent Library</span>
          <span className="ml-auto rounded-md bg-fill px-2 py-0.5 text-[9.5px] font-medium text-text-secondary md:hidden">Nova · Antigravity</span>
        </div>

        <div className="flex min-h-0 flex-1">
          <nav className="hidden w-[50px] shrink-0 flex-col items-center gap-1 border-r border-border bg-sidebar py-2 sm:flex">
            <RailButton label="Agents"><Users className="h-4 w-4" /></RailButton>
            <RailButton label="Chats"><MessageSquare className="h-4 w-4" /></RailButton>
            <RailButton active label="Library"><Library className="h-4 w-4" /></RailButton>
            <RailButton label="Terminal"><Terminal className="h-4 w-4" /></RailButton>
            <div className="flex-1" />
            <RailButton label="Settings"><Settings2 className="h-4 w-4" /></RailButton>
          </nav>

          <Roster model={model} />

          <main className="relative flex min-w-0 flex-1 items-center justify-center overflow-hidden bg-canvas p-4 xl:pr-[360px]">
            <div className="pointer-events-none absolute inset-0 opacity-70" aria-hidden="true">
              <div className="flex h-12 items-center border-b border-border px-5">
                <div>
                  <div className="text-[13px] font-semibold text-text-primary">Agent Library</div>
                  <div className="text-[10px] text-text-muted">Definitions available to every workspace</div>
                </div>
                <div className="ml-auto h-7 w-28 rounded-lg bg-accent/[0.12]" />
              </div>
              <div className="grid grid-cols-2 gap-3 p-5 xl:grid-cols-3">
                {["Aoki", "Dew", "Nova", "Hardwell", "Marty", "Dabin"].map((name, index) => (
                  <div key={name} className="h-24 rounded-xl bg-surface ring-1 ring-overlay/[0.06]">
                    <div className="flex items-center gap-2 p-3">
                      <span className="h-8 w-8 rounded-lg bg-fill" />
                      <div className="space-y-2">
                        <div className="h-2.5 w-20 rounded bg-fill" />
                        <div className={`h-2 rounded bg-fill ${index % 2 ? "w-14" : "w-24"}`} />
                      </div>
                    </div>
                  </div>
                ))}
              </div>
            </div>

            <div className="absolute inset-0 bg-overlay/[0.18]" aria-hidden="true" />
            <BuilderPanel key={preview} preview={preview} model={model} setModel={setModel} />
            <SupervisorPicker model={model} />
          </main>
        </div>
      </div>
    </div>
  );
}
