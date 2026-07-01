import { useEffect, useState } from "react";
import { X, Sparkles, Waypoints, MessageSquare, Terminal } from "lucide-react";
import { ipc } from "../ipc";
import type { AgentDefinition, Skill } from "../ipc";

// ── Types ────────────────────────────────────────────────────────────────────

export interface BuilderProps {
  onClose: () => void;
  onSaved?: (def: AgentDefinition) => void;
  /** Pre-fill the form for editing an existing definition. */
  initialDef?: AgentDefinition;
}

type AgentType = "cli" | "chat" | "orchestrator";
type CliKind = "claude-code" | "codex" | "custom";
type PermissionMode = "auto" | "bypassPermissions";
type ContextWindow = "1m" | "200k";

// ── Preset color swatches ────────────────────────────────────────────────────

const COLOR_SWATCHES = [
  "#5e5ce6",
  "#0a84ff",
  "#d6409f",
  "#30d158",
  "#ff9f0a",
  "#0fa3a3",
  "#ff3b30",
];

// ── Claude Code config presets ───────────────────────────────────────────────

/** Quick-fill model presets (the user can still type any value). */
const CLAUDE_MODELS = [
  "claude-opus-4-8",
  "claude-sonnet-5",
  "claude-haiku-4-5",
];

/** Quick-fill Codex model presets (context window is fixed per model: gpt-5.5
 *  / gpt-5.4 = 1M, gpt-5.4-mini = 400K — not separately selectable). */
const CODEX_MODELS = ["gpt-5.5", "gpt-5.4", "gpt-5.4-mini"];

/**
 * Sentinel shown for a secret env var already stored in the Keychain. Sending
 * it back unchanged means "keep the stored secret" (must match
 * `SECRET_PLACEHOLDER` in `src-tauri/src/engine/commands/agent.rs`).
 */
const SECRET_PLACEHOLDER = "••••••••";

/** Starter custom-env JSON, matching Claude Code's settings.json `env` shape. */
const DEFAULT_ENV_TEMPLATE = `{
  "env": {
    "ANTHROPIC_BASE_URL": "https://openrouter.ai/api",
    "ANTHROPIC_AUTH_TOKEN": "sk-or-...",
    "ANTHROPIC_MODEL": "...",
    "ANTHROPIC_SMALL_FAST_MODEL": "..."
  }
}`;

/** Build the env-editor text from a saved definition (or the starter template). */
function buildEnvText(def?: AgentDefinition): string {
  const env: Record<string, string> = { ...(def?.customEnv ?? {}) };
  // Secret values aren't returned — show their NAMES with a masked placeholder.
  for (const k of def?.secretEnvKeys ?? []) env[k] = SECRET_PLACEHOLDER;
  if (Object.keys(env).length === 0) return DEFAULT_ENV_TEMPLATE;
  return JSON.stringify({ env }, null, 2);
}

/**
 * Parse the env-editor text into a flat string→string map. Accepts either a
 * bare object or Claude's `{ "env": { … } }` wrapper. Throws on invalid JSON
 * (the caller surfaces it). Returns undefined when empty.
 */
function parseEnvText(text: string): Record<string, string> | undefined {
  const t = text.trim();
  if (!t) return undefined;
  const parsed: unknown = JSON.parse(t);
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("Custom environment must be a JSON object");
  }
  const obj = parsed as Record<string, unknown>;
  const inner =
    obj.env && typeof obj.env === "object" && !Array.isArray(obj.env)
      ? (obj.env as Record<string, unknown>)
      : obj;
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(inner)) {
    if (typeof v !== "string") {
      throw new Error(`Env value for "${k}" must be a string (got ${typeof v})`);
    }
    out[k] = v;
  }
  return Object.keys(out).length ? out : undefined;
}

// ── Sub-components ───────────────────────────────────────────────────────────

interface ToggleProps {
  on: boolean;
  onChange: (v: boolean) => void;
  label?: string;
}

function Toggle({ on, onChange, label }: ToggleProps) {
  return (
    <button
      role="switch"
      aria-checked={on}
      aria-label={label}
      onClick={() => onChange(!on)}
      className={`w-9 h-5 rounded-full relative transition-colors ${on ? "bg-status-running" : "bg-black/20"}`}
    >
      <span
        className={`absolute top-0.5 w-4 h-4 rounded-full bg-surface shadow-sm transition-transform ${on ? "right-0.5" : "left-0.5"}`}
      />
    </button>
  );
}

// ── Main component ───────────────────────────────────────────────────────────

export function Builder({ onClose, onSaved, initialDef }: BuilderProps) {
  const isEditing = Boolean(initialDef);

  // ── Form state (lazy-initialised from initialDef when editing) ─────────────
  const [name, setName] = useState(initialDef?.name ?? "");
  const [role, setRole] = useState(initialDef?.role ?? "");
  const [agentType, setAgentType] = useState<AgentType>(initialDef?.type ?? "cli");
  const [cliKind, setCliKind] = useState<CliKind>(initialDef?.cliKind ?? "claude-code");
  const [color, setColor] = useState(initialDef?.color ?? COLOR_SWATCHES[0]);
  // Color picker popover (anchored to the avatar).
  const [showColors, setShowColors] = useState(false);
  const [model, setModel] = useState(initialDef?.model ?? "");
  // ── Claude Code launch config ──────────────────────────────────────────────
  const [permissionMode, setPermissionMode] = useState<PermissionMode>(
    initialDef?.permissionMode ?? "auto",
  );
  const [contextWindow, setContextWindow] = useState<ContextWindow>(
    initialDef?.contextWindow ?? "200k",
  );
  const [customArgs, setCustomArgs] = useState(initialDef?.customArgs ?? "");
  // Custom env is opt-in so the starter template isn't saved by accident.
  const [useCustomEnv, setUseCustomEnv] = useState(
    Object.keys(initialDef?.customEnv ?? {}).length > 0 ||
      (initialDef?.secretEnvKeys?.length ?? 0) > 0,
  );
  const [envText, setEnvText] = useState(() => buildEnvText(initialDef));
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // ── Skills ─────────────────────────────────────────────────────────────────
  const [allSkills, setAllSkills] = useState<Skill[]>([]);
  // initialDef?.skillIds may now also contain builtin ids (agentDef.list
  // annotates the FULL builtin+custom set) — harmless here since the custom
  // checklist below only ever tests membership against `kind === "custom"`
  // skills, so a builtin id in this state simply never matches any checkbox.
  const [skillIds, setSkillIds] = useState<string[]>(initialDef?.skillIds ?? []);

  useEffect(() => {
    ipc.skill
      .list()
      .then(setAllSkills)
      .catch((err: unknown) => {
        if (import.meta.env.DEV) console.error("Builder: skill.list failed", err);
      });
  }, []);

  // ── Derived ────────────────────────────────────────────────────────────────
  const letter = name.trim().charAt(0).toUpperCase() || "?";
  // CLI launch config applies to claude-code + codex (custom isn't wired yet).
  const isClaudeCode = agentType === "cli" && cliKind === "claude-code";
  const isCodex = agentType === "cli" && cliKind === "codex";
  const showCliConfig = isClaudeCode || isCodex;
  const modelPresets = isCodex ? CODEX_MODELS : CLAUDE_MODELS;

  // ── Save ───────────────────────────────────────────────────────────────────
  async function handleSave() {
    if (!name.trim()) {
      setError("Name is required");
      return;
    }
    // Parse the custom env up front so a JSON error is reported before saving.
    // Claude Code only — Codex doesn't use ANTHROPIC_* env config.
    let customEnv: Record<string, string> | undefined;
    if (isClaudeCode && useCustomEnv) {
      try {
        customEnv = parseEnvText(envText);
      } catch (e) {
        setError(e instanceof Error ? e.message : "Invalid custom environment JSON");
        return;
      }
    }

    setSaving(true);
    setError(null);
    try {
      const def = await ipc.agentDef.save({
        // Pass `id` when editing so the backend upserts rather than inserts.
        id: initialDef?.id,
        name: name.trim(),
        type: agentType,
        role: role.trim() || undefined,
        cliKind: agentType === "cli" ? cliKind : undefined,
        color: color || undefined,
        model: model.trim() || undefined,
        // Harness is no longer a per-agent setting: every agent uses the shared
        // central Conclave harness + shared blackboard. Kept in the payload as
        // fixed constants so the backend contract is unchanged (no migration).
        harnessMode: "central",
        shareBlackboard: true,
        // Messaging is no longer a per-agent setting: every agent can message
        // every other agent, and injected messages always auto-submit (the
        // backend already behaves this way). Sent as fixed constants.
        autoSubmitInjected: true,
        allowedSenders: "all",
        // CLI launch config (claude-code + codex; omitted for other kinds).
        // contextWindow is Claude-only ([1m] suffix); Codex has no equivalent.
        permissionMode: showCliConfig ? permissionMode : undefined,
        contextWindow: isClaudeCode ? contextWindow : undefined,
        customArgs: showCliConfig && customArgs.trim() ? customArgs.trim() : undefined,
        customEnv,
        // Skills are cli-only in v1 — omit for other types so a chat/orchestrator
        // save never sends a stale list.
        skillIds: agentType === "cli" ? skillIds : undefined,
      });
      onSaved?.(def);
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  // ── Render ─────────────────────────────────────────────────────────────────
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30">
      <div className="w-[560px] max-h-[90vh] bg-surface rounded-2xl shadow-2xl flex flex-col overflow-hidden ring-1 ring-overlay/[0.08]">

        {/* ── Header ── */}
        <div className="h-11 flex items-center justify-between px-4 border-b border-overlay/[0.06] shrink-0">
          <div className="flex items-center gap-2">
            <Sparkles className="w-4 h-4 text-accent" />
            <span className="text-[13px] font-semibold tracking-tight">
              {isEditing ? "Edit agent" : "New agent"}
            </span>
            <span className="text-[10px] font-medium text-text-muted bg-overlay/[0.04] px-1.5 py-px rounded-md">
              {isEditing ? "update definition" : "saved to Library"}
            </span>
          </div>
          <button
            onClick={onClose}
            className="w-7 h-7 grid place-items-center rounded-md hover:bg-overlay/[0.05] text-text-secondary"
            aria-label="Close builder"
          >
            <X className="w-[15px] h-[15px]" />
          </button>
        </div>

        {/* ── Scrollable body ── */}
        <div className="flex-1 overflow-y-auto px-5 py-4 space-y-3.5 min-h-0">

          {/* Identity */}
          <section>
            <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-2">
              Identity
            </div>
            <div className="flex items-center gap-2.5">
              {/* Avatar doubles as the color picker — click to choose. */}
              <div className="relative shrink-0">
                <button
                  type="button"
                  onClick={() => setShowColors((v) => !v)}
                  className="w-10 h-10 rounded-[10px] text-white grid place-items-center text-[15px] font-bold ring-1 ring-overlay/[0.06] hover:brightness-105"
                  style={{ backgroundColor: color }}
                  title="Change color"
                  aria-label="Change color"
                >
                  {letter}
                </button>
                {showColors && (
                  <>
                    {/* Click-away backdrop. */}
                    <div className="fixed inset-0 z-10" onClick={() => setShowColors(false)} />
                    <div className="absolute z-20 top-full left-0 mt-1.5 flex items-center gap-1.5 bg-surface rounded-xl ring-1 ring-overlay/[0.1] shadow-lg p-2">
                      {COLOR_SWATCHES.map((swatch) => (
                        <button
                          key={swatch}
                          onClick={() => {
                            setColor(swatch);
                            setShowColors(false);
                          }}
                          className={`w-[18px] h-[18px] rounded-full transition-all ${
                            color === swatch ? "ring-2 ring-offset-1" : "hover:scale-110"
                          }`}
                          style={
                            {
                              backgroundColor: swatch,
                              "--tw-ring-color": swatch,
                            } as React.CSSProperties
                          }
                          aria-label={`Color ${swatch}`}
                        />
                      ))}
                      {/* Custom color — opens the OS color picker. The popover
                          stays open so the avatar preview updates live. */}
                      <label
                        className="w-[18px] h-[18px] rounded-full cursor-pointer ring-1 ring-overlay/15 relative overflow-hidden shrink-0"
                        title="Custom color"
                        style={{
                          background:
                            "conic-gradient(red, yellow, lime, aqua, blue, magenta, red)",
                        }}
                      >
                        <input
                          type="color"
                          value={color}
                          onChange={(e) => setColor(e.target.value)}
                          className="absolute inset-0 opacity-0 cursor-pointer"
                          aria-label="Custom color"
                        />
                      </label>
                    </div>
                  </>
                )}
              </div>
              <div className="flex-1 space-y-1">
                <input
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="Agent name"
                  className="w-full text-[14px] font-semibold tracking-tight bg-transparent outline-none border-b border-overlay/10 focus:border-accent pb-0.5"
                />
                <input
                  value={role}
                  onChange={(e) => setRole(e.target.value)}
                  placeholder="Role (optional)"
                  className="w-full text-[11.5px] text-text-muted bg-transparent outline-none"
                />
              </div>
            </div>

          </section>

          {/* Type */}
          <section>
            <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-2">
              Type
            </div>
            <div className="grid grid-cols-3 gap-2">
              {(
                [
                  { value: "cli", label: "CLI agent", Icon: Terminal, soon: false },
                  { value: "chat", label: "Chat agent", Icon: MessageSquare, soon: true },
                  { value: "orchestrator", label: "Orchestrator", Icon: Waypoints, soon: true },
                ] as { value: AgentType; label: string; Icon: typeof Terminal; soon: boolean }[]
              ).map(({ value, label, Icon, soon }) => {
                const active = agentType === value;
                // Chat / Orchestrator aren't ready yet — shown disabled with a
                // "Soon" badge so only CLI agents can be created for now.
                return (
                  <button
                    key={value}
                    onClick={() => !soon && setAgentType(value)}
                    disabled={soon}
                    aria-disabled={soon}
                    className={`relative rounded-xl p-2 text-left transition-all ${
                      soon
                        ? "ring-1 ring-overlay/[0.06] bg-overlay/[0.02] opacity-60 cursor-not-allowed"
                        : active
                          ? "ring-1 ring-accent/40 bg-accent/[0.06]"
                          : "ring-1 ring-overlay/[0.08] bg-surface hover:bg-overlay/[0.02]"
                    }`}
                  >
                    {soon && (
                      <span className="absolute top-1.5 right-1.5 text-[9px] font-bold tracking-wide text-text-muted bg-overlay/[0.06] px-1.5 py-px rounded-full uppercase">
                        Soon
                      </span>
                    )}
                    <Icon
                      className={`w-4 h-4 mb-1 ${
                        soon ? "text-text-tertiary" : active ? "text-accent" : "text-text-secondary"
                      }`}
                    />
                    <div className="text-[12px] font-semibold">{label}</div>
                  </button>
                );
              })}
            </div>

            {/* CLI Kind (only when type = cli) */}
            {agentType === "cli" && (
              // Segmented control (matches the Permission/Context pickers). Custom
              // CLI isn't wired in the backend yet, so it's shown disabled.
              <div
                role="radiogroup"
                aria-label="CLI kind"
                className="mt-2 flex gap-1 rounded-xl bg-overlay/[0.04] p-1"
              >
                {(
                  [
                    { value: "claude-code", label: "Claude Code", soon: false },
                    { value: "codex", label: "Codex", soon: false },
                    { value: "custom", label: "Custom", soon: true },
                  ] as { value: CliKind; label: string; soon: boolean }[]
                ).map(({ value, label, soon }) => (
                  <button
                    key={value}
                    role="radio"
                    aria-checked={cliKind === value}
                    disabled={soon}
                    onClick={() => !soon && setCliKind(value)}
                    className={`flex-1 flex items-center justify-center gap-1 text-[12.5px] py-1.5 rounded-lg transition-colors ${
                      soon
                        ? "text-text-tertiary cursor-not-allowed"
                        : cliKind === value
                          ? "bg-surface shadow-sm font-semibold"
                          : "text-text-secondary hover:bg-overlay/[0.03]"
                    }`}
                  >
                    {label}
                    {soon && (
                      <span className="text-[8.5px] font-bold tracking-wide text-text-muted bg-overlay/[0.06] px-1 py-px rounded uppercase">
                        Soon
                      </span>
                    )}
                  </button>
                ))}
              </div>
            )}
          </section>

          {/* Model / API — for claude-code / codex the model lives in the CLI
              config section below (with its presets), so it's hidden here to
              avoid two disconnected model fields. */}
          {!showCliConfig && (
            <section>
              <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-2">
                Model
              </div>
              <div className="rounded-xl ring-1 ring-overlay/[0.08] bg-surface divide-y divide-overlay/[0.06]">
                <div className="flex items-center justify-between px-3 py-2">
                  <span className="text-[12.5px] text-text-secondary">Provider</span>
                  {/* TODO(M5): real provider picker wired to provider.upsert */}
                  <span className="text-[12.5px] text-text-tertiary">Configure in Settings</span>
                </div>
                <div className="flex items-center justify-between px-3 py-2">
                  <span className="text-[12.5px] text-text-secondary">Model</span>
                  <input
                    value={model}
                    onChange={(e) => setModel(e.target.value)}
                    placeholder="e.g. claude-opus-4-8"
                    className="text-[12.5px] text-right bg-transparent outline-none w-44 placeholder:text-text-quaternary"
                  />
                </div>
              </div>
            </section>
          )}

          {/* CLI launch config — for claude-code + codex */}
          {showCliConfig && (
            <section>
              <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-2">
                {isCodex ? "Codex" : "Claude Code"}
              </div>
              <div className="rounded-xl ring-1 ring-overlay/[0.08] bg-surface divide-y divide-overlay/[0.06]">
                {/* Model — field + quick-presets together so picking a preset
                    visibly fills the same field. */}
                <div className="px-3 py-2">
                  <div className="flex items-center justify-between gap-3">
                    <span className="text-[12.5px] text-text-secondary shrink-0">Model</span>
                    <input
                      value={model}
                      onChange={(e) => setModel(e.target.value)}
                      placeholder={isCodex ? "gpt-5.5" : "claude-opus-4-8"}
                      className="text-[12.5px] font-mono text-right bg-transparent outline-none flex-1 placeholder:text-text-quaternary"
                    />
                  </div>
                  <div className="flex flex-wrap gap-1.5 mt-2">
                    {modelPresets.map((m) => (
                      <button
                        key={m}
                        onClick={() => setModel(m)}
                        className={`text-[11px] font-mono px-2 py-0.5 rounded-md ring-1 transition-colors ${
                          model === m
                            ? "ring-accent/40 bg-accent/[0.08] text-accent"
                            : "ring-overlay/[0.08] text-text-secondary hover:bg-overlay/[0.03]"
                        }`}
                      >
                        {m.replace("claude-", "")}
                      </button>
                    ))}
                  </div>
                </div>

                {/* Permission mode */}
                <div className="px-3 py-2">
                  <div className="flex items-center justify-between">
                    <span className="text-[12.5px] text-text-secondary">Permission mode</span>
                    <div
                      role="radiogroup"
                      aria-label="Permission mode"
                      className="flex rounded-lg bg-overlay/[0.04] p-0.5"
                    >
                      {(
                        [
                          { value: "auto", label: "Auto" },
                          { value: "bypassPermissions", label: "Bypass" },
                        ] as { value: PermissionMode; label: string }[]
                      ).map(({ value, label }) => (
                        <button
                          key={value}
                          role="radio"
                          aria-checked={permissionMode === value}
                          onClick={() => setPermissionMode(value)}
                          className={`text-[12px] px-2.5 py-1 rounded-[7px] transition-colors ${
                            permissionMode === value
                              ? "bg-surface shadow-sm font-semibold"
                              : "text-text-secondary"
                          }`}
                          title={
                            isCodex
                              ? value === "auto"
                                ? "codex --ask-for-approval never"
                                : "codex --yolo"
                              : `claude --permission-mode ${value}`
                          }
                        >
                          {label}
                        </button>
                      ))}
                    </div>
                  </div>
                  {permissionMode === "bypassPermissions" && (
                    <p className="text-[10.5px] text-warning mt-1.5">
                      Skips every permission prompt — use only in workspaces you trust.
                    </p>
                  )}
                </div>

                {/* Context window — Claude only (the [1m] suffix is
                    Claude-specific; Codex has no equivalent flag). */}
                {isClaudeCode && (
                <div className="px-3 py-2">
                  <div className="flex items-center justify-between">
                    <span className="text-[12.5px] text-text-secondary">Context window</span>
                    <div
                      role="radiogroup"
                      aria-label="Context window"
                      className="flex rounded-lg bg-overlay/[0.04] p-0.5"
                    >
                      {(
                        [
                          { value: "200k", label: "200K" },
                          { value: "1m", label: "1M" },
                        ] as { value: ContextWindow; label: string }[]
                      ).map(({ value, label }) => (
                        <button
                          key={value}
                          role="radio"
                          aria-checked={contextWindow === value}
                          onClick={() => setContextWindow(value)}
                          className={`text-[12px] px-2.5 py-1 rounded-[7px] transition-colors ${
                            contextWindow === value
                              ? "bg-surface shadow-sm font-semibold"
                              : "text-text-secondary"
                          }`}
                        >
                          {label}
                        </button>
                      ))}
                    </div>
                  </div>
                  {contextWindow === "1m" && (
                    <p className="text-[10.5px] text-text-tertiary mt-1.5">
                      Launches as <span className="font-mono">{model || "model"}[1m]</span>.
                    </p>
                  )}
                </div>
                )}

                {/* Custom args */}
                <div className="flex items-center justify-between px-3 py-2.5 gap-3">
                  <span className="text-[12.5px] text-text-secondary shrink-0">Custom args</span>
                  <input
                    value={customArgs}
                    onChange={(e) => setCustomArgs(e.target.value)}
                    placeholder="e.g. --verbose --mcp-config ./mcp.json"
                    className="text-[12px] font-mono text-right bg-transparent outline-none flex-1 placeholder:text-text-quaternary"
                  />
                </div>

                {/* Custom env (opt-in) — Claude Code only. Codex is configured
                    via its own config.toml / -c flags, not ANTHROPIC_* env. */}
                {isClaudeCode && (
                <div className="px-3 py-2">
                  <div className="flex items-center justify-between">
                    <span className="text-[12.5px] text-text-secondary">Custom environment</span>
                    <Toggle
                      on={useCustomEnv}
                      onChange={setUseCustomEnv}
                      label="Use custom environment"
                    />
                  </div>
                  {useCustomEnv && (
                    <>
                      <textarea
                        value={envText}
                        onChange={(e) => setEnvText(e.target.value)}
                        spellCheck={false}
                        rows={8}
                        className="mt-2 w-full rounded-lg ring-1 ring-overlay/[0.1] bg-fill-softer focus:ring-accent/50 outline-none px-2.5 py-2 text-[11.5px] font-mono leading-relaxed resize-y"
                      />
                      <p className="text-[10.5px] text-text-tertiary mt-1.5">
                        Secrets (AUTH_TOKEN / API_KEY / …) are stored in the macOS Keychain, never
                        in the database. Leave a value as{" "}
                        <span className="font-mono">{SECRET_PLACEHOLDER}</span> to keep the stored
                        secret.
                      </p>
                    </>
                  )}
                </div>
                )}
              </div>
            </section>
          )}

          {/* Skills — for claude-code + codex */}
          {showCliConfig && (
            <section>
              <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-2">
                Skills
              </div>
              <div className="rounded-xl ring-1 ring-overlay/[0.08] bg-surface divide-y divide-overlay/[0.06]">
                {allSkills.filter((s) => s.kind === "builtin").length > 0 && (
                  <div className="px-3 py-2">
                    <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
                      System skills — always on
                    </div>
                    <div className="flex flex-wrap gap-1.5">
                      {allSkills
                        .filter((s) => s.kind === "builtin")
                        .map((s) => (
                          <span
                            key={s.id}
                            className="text-[11px] font-medium px-2 py-0.5 rounded-md ring-1 ring-overlay/[0.08] text-text-secondary"
                          >
                            {s.name}
                          </span>
                        ))}
                    </div>
                  </div>
                )}
                <div className="px-3 py-2">
                  <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
                    Custom skills
                  </div>
                  {allSkills.filter((s) => s.kind === "custom").length === 0 ? (
                    <p className="text-[11.5px] text-text-tertiary">
                      No custom skills yet — create one in the Skill Library.
                    </p>
                  ) : (
                    <div className="space-y-1">
                      {allSkills
                        .filter((s) => s.kind === "custom")
                        .map((s) => {
                          const checked = skillIds.includes(s.id);
                          return (
                            <label
                              key={s.id}
                              className="flex items-center gap-2 text-[12.5px] text-text-secondary cursor-pointer"
                            >
                              <input
                                type="checkbox"
                                checked={checked}
                                onChange={(e) =>
                                  setSkillIds((prev) =>
                                    e.target.checked ? [...prev, s.id] : prev.filter((id) => id !== s.id),
                                  )
                                }
                              />
                              {s.name}
                            </label>
                          );
                        })}
                    </div>
                  )}
                </div>
              </div>
            </section>
          )}

          {/* Error */}
          {error && (
            <p className="text-[12px] text-danger px-1">{error}</p>
          )}
        </div>

        {/* ── Footer actions ── */}
        <div className="border-t border-overlay/[0.07] px-5 py-2.5 bg-surface shrink-0 flex items-center justify-end gap-2">
          <button
            onClick={onClose}
            className="text-[12.5px] font-medium text-text-secondary px-3.5 py-1.5 rounded-lg hover:bg-overlay/[0.05]"
          >
            Cancel
          </button>
          <button
            onClick={handleSave}
            disabled={saving}
            className="text-[12.5px] font-semibold text-white bg-accent px-4 py-1.5 rounded-lg hover:brightness-105 disabled:opacity-60 flex items-center gap-1.5"
          >
            <Sparkles className="w-3.5 h-3.5" />
            {saving ? "Saving…" : isEditing ? "Save changes" : "Create agent"}
          </button>
        </div>
      </div>
    </div>
  );
}
