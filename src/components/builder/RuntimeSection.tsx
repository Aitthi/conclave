// src/components/builder/RuntimeSection.tsx
//
// Runtime (spec D5 + D6). The Type cards and the CLI-kind segmented control
// collapse into ONE picker: a 3-column grid of provider logo tiles (canon
// rules 17-20). The per-CLI launch config rows move here verbatim, and Custom
// args + Custom environment move under an "Advanced" disclosure that opens by
// default when the edited definition already uses either (canon rules 25-26).

import { useState } from "react";
import {
  AlertTriangle,
  Check,
  ChevronDown,
  ChevronRight,
  CircleHelp,
  RefreshCw,
} from "lucide-react";
import { PROVIDER_NAMES, ProviderMark, RUNTIME_TILES } from "./providerLogos";
import { Section } from "./Section";

// ── Types (moved from Builder.tsx; the shell imports them back) ──────────────

export type CliKind = "claude-code" | "codex" | "antigravity" | "custom";
export type PermissionMode =
  "auto" | "default" | "acceptEdits" | "plan" | "bypassPermissions";
export type CliEffort = "low" | "medium" | "high" | undefined;
export type ClaudeContextWindow = "1m" | "200k";

export type CliAvailability =
  | { state: "idle" | "checking" }
  | { state: "available" | "missing"; installUrl: string }
  | { state: "error"; message: string };

/** The authenticated Antigravity model catalog (`instance.cliModels`). Queried
 *  only once availability says `agy` is there, so `error` here always means the
 *  QUERY failed (auth/network) — never that the CLI is missing. */
export type CliModelCatalog =
  | { state: "idle" | "loading" }
  | { state: "ready"; models: { id: string; label: string }[] }
  | { state: "error" };

export const ANTIGRAVITY_MODE_HELP: Record<
  Exclude<PermissionMode, "auto">,
  string
> = {
  default: "Pauses for diff review before applying changes.",
  acceptEdits:
    "Applies file edits automatically. Shell and web actions still ask.",
  plan: "Starts in planning mode before making changes.",
  bypassPermissions:
    "Skips every permission prompt, including shell and web actions.",
};

/**
 * Sentinel shown for a secret env var already stored in the Keychain. Sending
 * it back unchanged means "keep the stored secret" (must match
 * `SECRET_PLACEHOLDER` in `src-tauri/src/engine/commands/agent.rs`).
 */
export const SECRET_PLACEHOLDER =
  "\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022";

// ── Sub-components ───────────────────────────────────────────────────────────

interface ToggleProps {
  on: boolean;
  onChange: (v: boolean) => void;
  label?: string;
}

export function Toggle({ on, onChange, label }: ToggleProps) {
  return (
    <button
      role="switch"
      aria-checked={on}
      aria-label={label}
      onClick={() => onChange(!on)}
      className={`w-9 h-5 rounded-full relative transition-colors ${on ? "bg-success" : "bg-overlay/20"}`}
    >
      <span
        className={`absolute top-0.5 w-4 h-4 rounded-full bg-surface shadow-sm transition-transform ${on ? "right-0.5" : "left-0.5"}`}
      />
    </button>
  );
}

// ── Section ──────────────────────────────────────────────────────────────────

interface RuntimeSectionProps {
  cliKind: CliKind;
  selectCliKind: (k: CliKind) => void;
  isClaudeCode: boolean;
  isCodex: boolean;
  isAntigravity: boolean;
  showCliConfig: boolean;
  cliAvailability: CliAvailability;
  checkAntigravityAvailability: () => void;
  openAntigravityInstallGuide: () => void;
  modelCatalog: CliModelCatalog;
  loadAntigravityModels: () => void;
  catalogModels: { id: string; label: string }[];
  savedModelUnlisted: boolean;
  model: string;
  setModel: (v: string) => void;
  modelPresets: readonly string[];
  selectModelPreset: (m: string) => void;
  effort: CliEffort;
  setEffort: (v: CliEffort) => void;
  permissionMode: PermissionMode;
  setPermissionMode: (v: PermissionMode) => void;
  setPermissionModeDirty: (v: boolean) => void;
  contextWindow: string;
  setContextWindow: (v: string) => void;
  rtkEnabled: boolean;
  setRtkEnabled: (v: boolean) => void;
  customArgs: string;
  setCustomArgs: (v: string) => void;
  useCustomEnv: boolean;
  setUseCustomEnv: (v: boolean) => void;
  envText: string;
  setEnvText: (v: string) => void;
  /** Read once, as the disclosure's initial state (spec D6). */
  advancedInitiallyOpen: boolean;
}

export function RuntimeSection({
  cliKind,
  selectCliKind,
  isClaudeCode,
  isCodex,
  isAntigravity,
  showCliConfig,
  cliAvailability,
  checkAntigravityAvailability,
  openAntigravityInstallGuide,
  modelCatalog,
  loadAntigravityModels,
  catalogModels,
  savedModelUnlisted,
  model,
  setModel,
  modelPresets,
  selectModelPreset,
  effort,
  setEffort,
  permissionMode,
  setPermissionMode,
  setPermissionModeDirty,
  contextWindow,
  setContextWindow,
  rtkEnabled,
  setRtkEnabled,
  customArgs,
  setCustomArgs,
  useCustomEnv,
  setUseCustomEnv,
  envText,
  setEnvText,
  advancedInitiallyOpen,
}: RuntimeSectionProps) {
  const [advancedOpen, setAdvancedOpen] = useState(advancedInitiallyOpen);

  return (
    <Section
      id="runtime"
      title="Runtime"
      actions={
        isAntigravity && (
          <span
            className={`inline-flex items-center gap-1 text-[10px] font-medium ${
              cliAvailability.state === "missing" ||
              cliAvailability.state === "error"
                ? "text-danger"
                : "text-text-muted"
            }`}
          >
            {cliAvailability.state === "checking" ? (
              <RefreshCw className="h-3 w-3 animate-spin motion-reduce:animate-none" />
            ) : cliAvailability.state === "available" ? (
              <Check className="h-3 w-3 text-success" />
            ) : cliAvailability.state === "missing" ||
              cliAvailability.state === "error" ? (
              <AlertTriangle className="h-3 w-3" />
            ) : null}
            {cliAvailability.state === "checking"
              ? "Checking agy…"
              : cliAvailability.state === "available"
                ? "agy available"
                : cliAvailability.state === "missing"
                  ? "agy not found"
                  : cliAvailability.state === "error"
                    ? "Check failed"
                    : ""}
          </span>
        )
      }
    >
      {/* Provider tiles (D5) — only the kinds the backend can launch today.
          opencode and Muse Spark appear the day the CliKind union carries
          them; nothing is rendered as a disabled placeholder. */}
      <div
        role="radiogroup"
        aria-label="Runtime"
        className="grid grid-cols-3 gap-2"
      >
        {RUNTIME_TILES.map((kind) => {
          const active = cliKind === (kind as CliKind);
          return (
            <button
              key={kind}
              type="button"
              role="radio"
              aria-checked={active}
              onClick={() => selectCliKind(kind as CliKind)}
              className={`flex items-center gap-2.5 rounded-xl px-3 py-2.5 text-left transition-colors ring-1 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                active
                  ? "ring-accent/40 bg-accent/[0.06]"
                  : "ring-overlay/[0.08] bg-surface hover:bg-overlay/[0.02]"
              }`}
            >
              <ProviderMark
                kind={kind}
                className={active ? "text-accent" : "text-text-secondary"}
              />
              <span className="min-w-0 truncate text-[12.5px] font-semibold">
                {PROVIDER_NAMES[kind]}
              </span>
            </button>
          );
        })}
      </div>
      <p className="mb-3 mt-2 text-[10.5px] text-text-tertiary">
        Chat agent and Orchestrator are coming soon.
      </p>

      {isAntigravity && cliAvailability.state === "missing" && (
        <div
          role="alert"
          className="mb-2 rounded-xl bg-danger/[0.09] px-3 py-2.5 text-danger"
        >
          <div className="flex items-start gap-2">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
            <div className="min-w-0 flex-1">
              <div className="text-[11.5px] font-semibold">
                Antigravity CLI is not available
              </div>
              <p className="mt-0.5 text-[10.5px] leading-relaxed">
                Install the CLI from the Antigravity documentation, then make
                sure <span className="font-mono">agy</span> is on your
                login-shell PATH.
              </p>
              <div className="mt-2 flex flex-wrap items-center gap-2">
                <button
                  type="button"
                  onClick={() => void openAntigravityInstallGuide()}
                  title={cliAvailability.installUrl}
                  aria-label="Open Antigravity installation guide"
                  className="inline-flex h-7 items-center gap-1.5 rounded-md bg-danger px-2.5 text-[10.5px] font-semibold text-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-danger"
                >
                  <CircleHelp className="h-3 w-3" /> Installation guide
                </button>
                <button
                  type="button"
                  onClick={() => void checkAntigravityAvailability()}
                  className="inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-[10.5px] font-semibold text-danger hover:bg-danger/[0.08] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-danger"
                >
                  <RefreshCw className="h-3 w-3" /> Check again
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {isAntigravity && cliAvailability.state === "error" && (
        <div
          role="alert"
          className="mb-2 rounded-xl bg-warning/[0.09] px-3 py-2.5 text-warning"
        >
          <div className="flex items-start gap-2">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
            <div className="min-w-0 flex-1">
              <div className="text-[11.5px] font-semibold">
                Couldn’t check Antigravity CLI
              </div>
              <p className="mt-0.5 text-[10.5px] leading-relaxed">
                Conclave couldn’t query your login shell. Check its
                configuration, then try again.
              </p>
              <button
                type="button"
                onClick={() => void checkAntigravityAvailability()}
                title={cliAvailability.message}
                className="mt-2 inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-[10.5px] font-semibold hover:bg-warning/[0.08] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-warning"
              >
                <RefreshCw className="h-3 w-3" /> Check again
              </button>
            </div>
          </div>
        </div>
      )}

      <div className="rounded-xl ring-1 ring-overlay/[0.08] bg-surface divide-y divide-overlay/[0.06]">
        {/* Model — field + quick-presets together so picking a preset
            visibly fills the same field. */}
        <div className="px-3 py-2">
          <div className="flex items-center justify-between gap-3">
            <label
              htmlFor="cli-model"
              className="text-[12.5px] text-text-secondary shrink-0"
            >
              Model
            </label>
            {isAntigravity ? (
              // Antigravity's models are discovered from the user's own
              // authenticated CLI, so there is nothing to type and no
              // list to hardcode — same native select as Execution mode.
              // Fixed width, not min-width: a native select sizes
              // itself to its WIDEST option, so one long model id would
              // otherwise stretch this control across the row. 240px is
              // the smallest width that shows "Auto (authenticated
              // default)" (158px of text) whole; longer labels clip, and
              // the exact id stays reachable through the option title
              // and the hint line below.
              <div className="relative w-[240px] shrink-0">
                <select
                  id="cli-model"
                  value={model}
                  disabled={modelCatalog.state === "loading"}
                  onChange={(event) => setModel(event.target.value)}
                  title={model || "Auto (authenticated default)"}
                  className="h-7 w-full appearance-none rounded-lg bg-overlay/[0.04] pl-2.5 pr-7 text-[11.5px] font-medium text-text-primary outline-none focus-visible:ring-2 focus-visible:ring-accent disabled:text-text-muted"
                >
                  <option value="">Auto (authenticated default)</option>
                  {savedModelUnlisted && (
                    <option value={model} title={model}>
                      {modelCatalog.state === "ready"
                        ? `${model} (unavailable)`
                        : model}
                    </option>
                  )}
                  {catalogModels.map((entry) => (
                    <option key={entry.id} value={entry.id} title={entry.id}>
                      {entry.label}
                    </option>
                  ))}
                </select>
                <ChevronDown className="pointer-events-none absolute right-2 top-2 h-3 w-3 text-text-muted" />
              </div>
            ) : (
              <input
                id="cli-model"
                value={model}
                onChange={(e) => setModel(e.target.value)}
                placeholder={isCodex ? "gpt-5.5" : "claude-opus-4-8"}
                className="min-w-0 flex-1 bg-transparent text-right font-mono text-[12.5px] outline-none placeholder:text-text-tertiary focus-visible:rounded-sm focus-visible:ring-2 focus-visible:ring-accent"
              />
            )}
          </div>
          {isAntigravity ? (
            <div className="mt-1.5 text-[10px] leading-relaxed text-text-tertiary">
              {modelCatalog.state === "loading" ? (
                <span className="inline-flex items-center gap-1.5">
                  <RefreshCw className="h-3 w-3 animate-spin motion-reduce:animate-none" />
                  Loading your authenticated models…
                </span>
              ) : modelCatalog.state === "error" ? (
                <span className="inline-flex flex-wrap items-center gap-1.5 text-warning">
                  Couldn’t load your Antigravity models.
                  <button
                    type="button"
                    onClick={() => void loadAntigravityModels()}
                    className="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 font-semibold hover:bg-warning/[0.08] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-warning"
                  >
                    <RefreshCw className="h-3 w-3" /> Retry
                  </button>
                </span>
              ) : savedModelUnlisted ? (
                <span className="text-warning">
                  <span className="font-mono">{model}</span> isn’t in your
                  authenticated models. It is kept until you pick another.
                </span>
              ) : model ? (
                <>
                  Launches as <span className="font-mono">{model}</span>.
                </>
              ) : (
                "Auto lets Antigravity choose from your authenticated models."
              )}
            </div>
          ) : (
            <div className="flex flex-wrap gap-1.5 mt-2">
              {modelPresets.map((m) => (
                <button
                  key={m}
                  onClick={() => selectModelPreset(m)}
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
          )}
        </div>

        {showCliConfig && (
          <div className="px-3 py-2">
            <div className="flex items-center justify-between gap-3">
              <span className="text-[12.5px] text-text-secondary">Effort</span>
              <div
                role="radiogroup"
                aria-label="Effort"
                className="flex rounded-lg bg-overlay/[0.04] p-0.5"
              >
                {(
                  [
                    { value: undefined, label: "Auto" },
                    { value: "low", label: "low" },
                    { value: "medium", label: "medium" },
                    { value: "high", label: "high" },
                  ] as { value: CliEffort; label: string }[]
                ).map(({ value, label }) => (
                  <button
                    key={label}
                    type="button"
                    role="radio"
                    aria-checked={effort === value}
                    onClick={() => setEffort(value)}
                    className={`rounded-[7px] px-2.5 py-1 text-[11.5px] transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                      effort === value
                        ? "bg-surface font-semibold text-text-primary shadow-sm"
                        : "text-text-secondary hover:text-text-primary"
                    }`}
                  >
                    {label}
                  </button>
                ))}
              </div>
            </div>
            <p className="mt-1.5 text-[10px] leading-relaxed text-text-tertiary">
              Auto uses the provider default and omits the effort override.
            </p>
          </div>
        )}

        {/* Permission mode */}
        {isAntigravity ? (
          <div className="px-3 py-2">
            <div className="flex items-center justify-between gap-3">
              <label
                htmlFor="antigravity-execution-mode"
                className="shrink-0 text-[12.5px] text-text-secondary"
              >
                Execution mode
              </label>
              <div className="relative min-w-[190px]">
                <select
                  id="antigravity-execution-mode"
                  value={permissionMode === "auto" ? "default" : permissionMode}
                  onChange={(event) => {
                    setPermissionMode(event.target.value as PermissionMode);
                    setPermissionModeDirty(true);
                  }}
                  className={`h-7 w-full appearance-none rounded-lg bg-overlay/[0.04] pl-2.5 pr-7 text-[11.5px] font-medium outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                    permissionMode === "bypassPermissions"
                      ? "text-danger"
                      : "text-text-primary"
                  }`}
                >
                  <option value="default">Default</option>
                  <option value="acceptEdits">Accept edits</option>
                  <option value="plan">Plan</option>
                  <option value="bypassPermissions">Bypass permissions</option>
                </select>
                <ChevronDown className="pointer-events-none absolute right-2 top-2 h-3 w-3 text-text-muted" />
              </div>
            </div>
            <p
              className={`mt-1.5 text-[10px] leading-relaxed ${
                permissionMode === "bypassPermissions"
                  ? "text-danger"
                  : "text-text-tertiary"
              }`}
            >
              {
                ANTIGRAVITY_MODE_HELP[
                  permissionMode === "auto" ? "default" : permissionMode
                ]
              }
            </p>
            {permissionMode === "bypassPermissions" && (
              <div className="mt-2 flex items-start gap-2 rounded-lg bg-danger/[0.09] px-2.5 py-2 text-[10.5px] leading-relaxed text-danger">
                <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                <span>
                  <strong>Use only in workspaces you trust.</strong> This
                  disables Antigravity permission checks.
                </span>
              </div>
            )}
          </div>
        ) : (
          <div className="px-3 py-2">
            <div className="flex items-center justify-between">
              <span className="text-[12.5px] text-text-secondary">
                Permission mode
              </span>
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
                    onClick={() => {
                      setPermissionMode(value);
                      setPermissionModeDirty(true);
                    }}
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
                Skips every permission prompt — use only in workspaces you
                trust.
              </p>
            )}
          </div>
        )}

        {/* Context window — Claude's [1m] suffix remains a segmented
            choice; Codex uses a numeric model_context_window override. */}
        {isClaudeCode && (
          <div className="px-3 py-2">
            <div className="flex items-center justify-between">
              <span className="text-[12.5px] text-text-secondary">
                Context window
              </span>
              <div
                role="radiogroup"
                aria-label="Context window"
                className="flex rounded-lg bg-overlay/[0.04] p-0.5"
              >
                {(
                  [
                    { value: "200k", label: "200K" },
                    { value: "1m", label: "1M" },
                  ] as { value: ClaudeContextWindow; label: string }[]
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
                Launches as{" "}
                <span className="font-mono">{model || "model"}[1m]</span>.
              </p>
            )}
          </div>
        )}

        {isCodex && (
          <div className="px-3 py-2">
            <div className="flex items-center justify-between">
              <span className="text-[12.5px] text-text-secondary">
                Context window
              </span>
              <span className="text-[12px] text-text-tertiary">Auto</span>
            </div>
            <p className="text-[10.5px] text-text-tertiary mt-1.5">
              Derived from the model — no manual override.
            </p>
          </div>
        )}

        {/* Token filter (rtk) — Claude Code + Codex; absent/null = ON. */}
        {(isClaudeCode || isCodex) && (
          <div className="px-3 py-2">
            <div className="flex items-center justify-between">
              <span className="text-[12.5px] text-text-secondary">
                Token filter (rtk)
              </span>
              <Toggle
                on={rtkEnabled}
                onChange={setRtkEnabled}
                label="Token filter (rtk)"
              />
            </div>
            <p className="text-[10.5px] text-text-tertiary mt-1.5">
              Rewrites shell commands through rtk to compress output and save
              tokens.
            </p>
          </div>
        )}
      </div>

      {isAntigravity && (
        <p className="mt-2 text-[10px] leading-relaxed text-text-tertiary">
          Token filtering and sandbox controls are not available for Antigravity
          in this version.
        </p>
      )}

      {/* Advanced (D6) — collapsed unless the definition already uses it. */}
      <div className="mt-2.5">
        <button
          type="button"
          aria-expanded={advancedOpen}
          aria-controls="builder-advanced"
          onClick={() => setAdvancedOpen((open) => !open)}
          className="flex w-full items-center gap-1.5 rounded-lg px-1 py-1.5 text-left transition-colors hover:bg-overlay/[0.03] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
        >
          <ChevronRight
            className={`h-3.5 w-3.5 text-text-tertiary transition-transform ${advancedOpen ? "rotate-90" : ""}`}
          />
          <span className="text-[12px] font-semibold">Advanced</span>
          <span className="text-[11px] text-text-tertiary">
            {isClaudeCode ? "Custom args, custom environment" : "Custom args"}
          </span>
        </button>

        {advancedOpen && (
          <div
            id="builder-advanced"
            className="mt-2 rounded-xl ring-1 ring-overlay/[0.08] bg-surface divide-y divide-overlay/[0.06]"
          >
            {/* Custom args */}
            <div className="flex items-center justify-between px-3 py-2.5 gap-3">
              <span className="text-[12.5px] text-text-secondary shrink-0">
                Custom args
              </span>
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
                  <span className="text-[12.5px] text-text-secondary">
                    Custom environment
                  </span>
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
                      Secrets (AUTH_TOKEN / API_KEY / …) are stored in the macOS
                      Keychain, never in the database. Leave a value as{" "}
                      <span className="font-mono">{SECRET_PLACEHOLDER}</span> to
                      keep the stored secret.
                    </p>
                  </>
                )}
              </div>
            )}
          </div>
        )}
      </div>
    </Section>
  );
}
