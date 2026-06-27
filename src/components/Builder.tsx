import { useState } from "react";
import { X, Sparkles, Waypoints, MessageSquare, Terminal } from "lucide-react";
import { ipc } from "../ipc";
import type { AgentDefinition } from "../ipc";

// ── Types ────────────────────────────────────────────────────────────────────

export interface BuilderProps {
  onClose: () => void;
  onSaved?: (def: AgentDefinition) => void;
  /** Pre-fill the form for editing an existing definition. */
  initialDef?: AgentDefinition;
}

type AgentType = "cli" | "chat" | "orchestrator";
type CliKind = "claude-code" | "codex" | "custom";
type AllowedSenders = "all" | "selected" | "none";

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

// ── Sub-components ───────────────────────────────────────────────────────────

interface ToggleProps {
  on: boolean;
  onChange: (v: boolean) => void;
}

function Toggle({ on, onChange }: ToggleProps) {
  return (
    <button
      role="switch"
      aria-checked={on}
      onClick={() => onChange(!on)}
      className={`w-9 h-5 rounded-full relative transition-colors ${on ? "bg-[#30d158]" : "bg-black/20"}`}
    >
      <span
        className={`absolute top-0.5 w-4 h-4 rounded-full bg-white shadow-sm transition-transform ${on ? "right-0.5" : "left-0.5"}`}
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
  const [model, setModel] = useState(initialDef?.model ?? "");
  const [autoSubmitInjected, setAutoSubmitInjected] = useState(
    initialDef?.autoSubmitInjected ?? false,
  );
  const [allowedSenders, setAllowedSenders] = useState<AllowedSenders>(
    initialDef?.allowedSenders ?? "all",
  );
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // ── Derived ────────────────────────────────────────────────────────────────
  const letter = name.trim().charAt(0).toUpperCase() || "?";

  // ── Save ───────────────────────────────────────────────────────────────────
  async function handleSave() {
    if (!name.trim()) {
      setError("Name is required");
      return;
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
        autoSubmitInjected,
        allowedSenders,
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
      <div className="w-[560px] max-h-[90vh] bg-white rounded-2xl shadow-2xl flex flex-col overflow-hidden ring-1 ring-black/[0.08]">

        {/* ── Header ── */}
        <div className="h-12 flex items-center justify-between px-5 border-b border-black/[0.06] shrink-0">
          <div className="flex items-center gap-2">
            <Sparkles className="w-4 h-4 text-[#0a84ff]" />
            <span className="text-[13px] font-semibold tracking-tight">
              {isEditing ? "Edit agent" : "New agent"}
            </span>
            <span className="text-[10px] font-medium text-[#86868b] bg-black/[0.04] px-1.5 py-px rounded-md">
              {isEditing ? "update definition" : "saved to Library"}
            </span>
          </div>
          <button
            onClick={onClose}
            className="w-7 h-7 grid place-items-center rounded-md hover:bg-black/[0.05] text-[#6e6e73]"
            aria-label="Close builder"
          >
            <X className="w-[15px] h-[15px]" />
          </button>
        </div>

        {/* ── Scrollable body ── */}
        <div className="flex-1 overflow-y-auto px-6 py-5 space-y-5 min-h-0">

          {/* Identity */}
          <section>
            <div className="text-[10px] font-bold tracking-wider text-[#a1a1a6] uppercase mb-2.5">
              Identity
            </div>
            <div className="flex items-center gap-3 mb-3">
              <div
                className="w-12 h-12 rounded-[12px] text-white grid place-items-center text-[18px] font-bold ring-1 ring-black/[0.06] shrink-0"
                style={{ backgroundColor: color }}
              >
                {letter}
              </div>
              <div className="flex-1 space-y-1">
                <input
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="Agent name"
                  className="w-full text-[14px] font-semibold tracking-tight bg-transparent outline-none border-b border-black/10 focus:border-[#0a84ff] pb-0.5"
                />
                <input
                  value={role}
                  onChange={(e) => setRole(e.target.value)}
                  placeholder="Role (optional)"
                  className="w-full text-[11.5px] text-[#86868b] bg-transparent outline-none"
                />
              </div>
            </div>

            {/* Color swatches */}
            <div className="flex items-center gap-2">
              <span className="text-[11px] text-[#86868b] mr-1">Color</span>
              {COLOR_SWATCHES.map((swatch) => (
                <button
                  key={swatch}
                  onClick={() => setColor(swatch)}
                  className={`w-5 h-5 rounded-full transition-all ${
                    color === swatch
                      ? "ring-2 ring-offset-2"
                      : ""
                  }`}
                  style={{
                    backgroundColor: swatch,
                    // Tailwind ring color lives in this custom property — not `ringColor`.
                    "--tw-ring-color": swatch,
                  } as React.CSSProperties}
                  aria-label={`Color ${swatch}`}
                />
              ))}
            </div>
          </section>

          {/* Type */}
          <section>
            <div className="text-[10px] font-bold tracking-wider text-[#a1a1a6] uppercase mb-2.5">
              Type
            </div>
            <div className="grid grid-cols-3 gap-2">
              {(
                [
                  { value: "cli", label: "CLI agent", Icon: Terminal },
                  { value: "chat", label: "Chat agent", Icon: MessageSquare },
                  { value: "orchestrator", label: "Orchestrator", Icon: Waypoints },
                ] as { value: AgentType; label: string; Icon: typeof Terminal }[]
              ).map(({ value, label, Icon }) => {
                const active = agentType === value;
                return (
                  <button
                    key={value}
                    onClick={() => setAgentType(value)}
                    className={`rounded-xl p-2.5 text-left transition-all ${
                      active
                        ? "ring-1 ring-[#0a84ff]/40 bg-[#0a84ff]/[0.06]"
                        : "ring-1 ring-black/[0.08] bg-white hover:bg-black/[0.02]"
                    }`}
                  >
                    <Icon
                      className={`w-4 h-4 mb-1.5 ${active ? "text-[#0a84ff]" : "text-[#6e6e73]"}`}
                    />
                    <div className="text-[12px] font-semibold">{label}</div>
                  </button>
                );
              })}
            </div>

            {/* CLI Kind (only when type = cli) */}
            {agentType === "cli" && (
              <div className="mt-2.5 rounded-xl ring-1 ring-black/[0.08] bg-white divide-y divide-black/[0.06]">
                {(
                  [
                    { value: "claude-code", label: "Claude Code" },
                    { value: "codex", label: "Codex" },
                    { value: "custom", label: "Custom" },
                  ] as { value: CliKind; label: string }[]
                ).map(({ value, label }) => (
                  <label
                    key={value}
                    className="flex items-center justify-between px-3 py-2 cursor-pointer"
                  >
                    <span className="text-[12.5px]">{label}</span>
                    <input
                      type="radio"
                      name="cliKind"
                      value={value}
                      checked={cliKind === value}
                      onChange={() => setCliKind(value)}
                      className="accent-[#0a84ff]"
                    />
                  </label>
                ))}
              </div>
            )}
          </section>

          {/* Model / API */}
          <section>
            <div className="text-[10px] font-bold tracking-wider text-[#a1a1a6] uppercase mb-2.5">
              Model
            </div>
            <div className="rounded-xl ring-1 ring-black/[0.08] bg-white divide-y divide-black/[0.06]">
              <div className="flex items-center justify-between px-3 py-2.5">
                <span className="text-[12.5px] text-[#6e6e73]">Provider</span>
                {/* TODO(M5): real provider picker wired to provider.upsert */}
                <span className="text-[12.5px] text-[#a1a1a6]">Configure in Settings</span>
              </div>
              <div className="flex items-center justify-between px-3 py-2">
                <span className="text-[12.5px] text-[#6e6e73]">Model</span>
                <input
                  value={model}
                  onChange={(e) => setModel(e.target.value)}
                  placeholder="e.g. claude-opus-4-8"
                  className="text-[12.5px] text-right bg-transparent outline-none w-44 placeholder:text-[#c7c7cc]"
                />
              </div>
            </div>
          </section>

          {/* Tools placeholder (M5) */}
          <section>
            <div className="text-[10px] font-bold tracking-wider text-[#a1a1a6] uppercase mb-2">
              Tools
            </div>
            {/* TODO(M5): tool picker wired to agent_tool join table */}
            <div className="rounded-xl ring-1 ring-black/[0.08] bg-black/[0.02] px-3 py-3 text-[12px] text-[#a1a1a6] text-center">
              Tool selection — available in M5
            </div>
          </section>

          {/* Skills placeholder (M5) */}
          <section>
            <div className="text-[10px] font-bold tracking-wider text-[#a1a1a6] uppercase mb-2">
              Custom skills
            </div>
            {/* TODO(M5): skill picker wired to agent_skill join table */}
            <div className="rounded-xl ring-1 ring-black/[0.08] bg-black/[0.02] px-3 py-3 text-[12px] text-[#a1a1a6] text-center">
              Skill assignment — available in M5
            </div>
          </section>

          {/* Messaging */}
          <section>
            <div className="text-[10px] font-bold tracking-wider text-[#a1a1a6] uppercase mb-2.5">
              Messaging
            </div>
            <div className="rounded-xl ring-1 ring-black/[0.08] bg-white divide-y divide-black/[0.06]">
              <div className="flex items-center justify-between px-3 py-2.5">
                <span className="text-[12.5px] text-[#6e6e73]">Allowed senders</span>
                <select
                  value={allowedSenders}
                  onChange={(e) => setAllowedSenders(e.target.value as AllowedSenders)}
                  className="text-[12.5px] bg-transparent outline-none text-right"
                >
                  <option value="all">All agents</option>
                  <option value="selected">Selected</option>
                  <option value="none">None</option>
                </select>
              </div>
              <div className="flex items-center justify-between px-3 py-2.5">
                <span className="text-[12.5px]">Auto-submit injected messages</span>
                <Toggle on={autoSubmitInjected} onChange={setAutoSubmitInjected} />
              </div>
            </div>
          </section>

          {/* Error */}
          {error && (
            <p className="text-[12px] text-[#ff3b30] px-1">{error}</p>
          )}
        </div>

        {/* ── Footer actions ── */}
        <div className="border-t border-black/[0.07] px-6 py-3 bg-white shrink-0 flex items-center justify-end gap-2">
          <button
            onClick={onClose}
            className="text-[12.5px] font-medium text-[#6e6e73] px-3.5 py-1.5 rounded-lg hover:bg-black/[0.05]"
          >
            Cancel
          </button>
          <button
            onClick={handleSave}
            disabled={saving}
            className="text-[12.5px] font-semibold text-white bg-[#0a84ff] px-4 py-1.5 rounded-lg hover:brightness-105 disabled:opacity-60 flex items-center gap-1.5"
          >
            <Sparkles className="w-3.5 h-3.5" />
            {saving ? "Saving…" : isEditing ? "Save changes" : "Create agent"}
          </button>
        </div>
      </div>
    </div>
  );
}
