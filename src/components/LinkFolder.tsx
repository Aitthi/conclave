import { useEffect, useState } from "react";
import { X, FolderPlus, Folder, Check } from "lucide-react";
import { ipc } from "../ipc";
import type { AgentDefinition, Workspace } from "../ipc";

// ── Types ─────────────────────────────────────────────────────────────────────

export interface LinkFolderProps {
  onClose: () => void;
  onLinked?: (workspace: Workspace) => void;
}

// ── Color swatches (matches Builder's palette) ────────────────────────────────

const COLOR_SWATCHES = [
  "#ff3b30",
  "#0a84ff",
  "#30d158",
  "#ff9f0a",
  "#5e5ce6",
  "#d6409f",
  "#0fa3a3",
];

// ── Native folder picker ──────────────────────────────────────────────────────

async function pickFolder(): Promise<string | null> {
  try {
    // Dynamic import so plain-Vite builds don't crash if Tauri isn't present.
    const { open } = await import("@tauri-apps/plugin-dialog");
    const result = await open({
      directory: true,
      multiple: false,
      title: "Choose a folder",
    });
    if (typeof result === "string") return result;
    return null;
  } catch {
    // Running in plain Vite dev (no Tauri) — return null gracefully.
    return null;
  }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function basename(path: string): string {
  return path.replace(/\\/g, "/").split("/").filter(Boolean).pop() ?? path;
}

// ── Main component ────────────────────────────────────────────────────────────

export function LinkFolder({ onClose, onLinked }: LinkFolderProps) {
  // ── Form state ──────────────────────────────────────────────────────────────
  const [folderPath, setFolderPath] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [color, setColor] = useState(COLOR_SWATCHES[0]);
  const [agentDefs, setAgentDefs] = useState<AgentDefinition[]>([]);
  const [selectedDefIds, setSelectedDefIds] = useState<Set<string>>(new Set());
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Load agent definitions for the multi-select list.
  useEffect(() => {
    ipc.agentDef
      .list()
      .then(setAgentDefs)
      .catch(() => setAgentDefs([]));
  }, []);

  // ── Folder picker ───────────────────────────────────────────────────────────

  async function handleBrowse() {
    const path = await pickFolder();
    if (!path) return;
    setFolderPath(path);
    // Auto-fill name from basename only if the user hasn't edited it yet.
    setName((prev) => (prev === "" ? basename(path) : prev));
  }

  // ── Agent toggle ────────────────────────────────────────────────────────────

  function toggleDef(id: string) {
    setSelectedDefIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  // ── Submit ──────────────────────────────────────────────────────────────────

  async function handleLink() {
    if (!folderPath) {
      setError("Please choose a folder first.");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const result = await ipc.workspace.link({
        folderPath,
        name: name.trim() || basename(folderPath),
        color: color || undefined,
        agentDefIds: [...selectedDefIds],
      });
      onLinked?.(result.workspace);
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  // ── Derived ─────────────────────────────────────────────────────────────────

  const displayPath = folderPath
    ? folderPath.replace(/^\/Users\/[^/]+/, "~")
    : null;

  // ── Render ──────────────────────────────────────────────────────────────────

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30">
      <div className="w-[520px] max-h-[90vh] bg-surface rounded-2xl shadow-2xl flex flex-col overflow-hidden ring-1 ring-overlay/[0.08]">

        {/* ── Header ── */}
        <div className="h-12 flex items-center justify-between px-5 border-b border-overlay/[0.06] shrink-0">
          <div className="flex items-center gap-2">
            <FolderPlus className="w-4 h-4 text-accent" />
            <span className="text-[13px] font-semibold tracking-tight">New workspace</span>
          </div>
          <button
            onClick={onClose}
            className="w-7 h-7 grid place-items-center rounded-md hover:bg-overlay/[0.05] text-text-secondary"
            aria-label="Close"
          >
            <X className="w-[15px] h-[15px]" />
          </button>
        </div>

        {/* ── Scrollable body ── */}
        <div className="flex-1 overflow-y-auto min-h-0">
          <div className="p-6">

            {/* Icon + headline */}
            <div className="text-center mb-5">
              <div className="w-12 h-12 rounded-[14px] bg-accent/10 grid place-items-center mx-auto mb-2.5">
                <FolderPlus className="w-6 h-6 text-accent" />
              </div>
              <div className="text-[16px] font-bold tracking-tight">
                Link a folder as workspace
              </div>
              <div className="text-[12px] text-text-muted mt-0.5">
                Each workspace is bound to one folder with its own roster and blackboard.
              </div>
            </div>

            {/* ── Folder picker ── */}
            <div className="mb-4">
              <div className="text-[11px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
                Folder
              </div>
              <div className="flex items-center gap-2 rounded-lg ring-1 ring-overlay/[0.10] bg-fill-softer px-3 h-10">
                <Folder className="w-4 h-4 text-warning shrink-0" />
                <span className="flex-1 font-mono text-[12.5px] text-text-body truncate">
                  {displayPath ?? (
                    <span className="text-text-tertiary">No folder chosen</span>
                  )}
                </span>
                <button
                  onClick={handleBrowse}
                  className="text-[11.5px] font-semibold text-accent bg-surface ring-1 ring-overlay/[0.08] px-2.5 py-1 rounded-md hover:bg-overlay/[0.02] shrink-0"
                >
                  Browse…
                </button>
              </div>
            </div>

            {/* ── Name + Color ── */}
            <div className="grid grid-cols-[1fr_auto] gap-3 mb-4">
              <div>
                <div className="text-[11px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
                  Name
                </div>
                <input
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder={
                    folderPath ? basename(folderPath) : "Workspace name"
                  }
                  className="w-full rounded-lg ring-1 ring-overlay/[0.10] bg-surface px-3 h-10 text-[13px] outline-none focus:ring-accent/50"
                />
              </div>
              <div>
                <div className="text-[11px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
                  Color
                </div>
                <div className="flex items-center gap-1.5 h-10">
                  {COLOR_SWATCHES.map((swatch) => (
                    <button
                      key={swatch}
                      onClick={() => setColor(swatch)}
                      className={`w-5 h-5 rounded-full transition-all ${
                        color === swatch ? "ring-2 ring-offset-2" : ""
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
                </div>
              </div>
            </div>

            {/* ── Initial agents ── */}
            {agentDefs.length > 0 && (
              <div className="mb-5">
                <div className="text-[11px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5 flex items-center justify-between">
                  <span>Add agents from Library</span>
                  <span className="text-[10px] text-text-muted normal-case tracking-normal font-medium">
                    each gets its own session
                  </span>
                </div>
                <div className="grid grid-cols-2 gap-1.5">
                  {agentDefs.map((def) => {
                    const checked = selectedDefIds.has(def.id);
                    const letter = def.name.charAt(0).toUpperCase();
                    const defColor = def.color ?? "#6e6e73";
                    return (
                      <button
                        key={def.id}
                        onClick={() => toggleDef(def.id)}
                        className={`flex items-center gap-2 rounded-lg px-2.5 py-1.5 text-left transition-all ${
                          checked
                            ? "ring-1 ring-accent/30 bg-accent/[0.05]"
                            : "ring-1 ring-overlay/[0.08] bg-surface hover:bg-overlay/[0.02]"
                        }`}
                      >
                        {/* Checkbox */}
                        <span
                          className={`w-4 h-4 rounded-[5px] shrink-0 grid place-items-center ${
                            checked
                              ? "bg-accent"
                              : "ring-1 ring-overlay/[0.15]"
                          }`}
                        >
                          {checked && (
                            <Check className="w-2.5 h-2.5 text-white" />
                          )}
                        </span>
                        {/* Agent avatar */}
                        <span
                          className="w-5 h-5 rounded-[6px] text-white grid place-items-center text-[10px] font-bold shrink-0"
                          style={{ backgroundColor: defColor }}
                        >
                          {letter}
                        </span>
                        <span className="text-[12px] font-medium truncate">
                          {def.name}
                        </span>
                      </button>
                    );
                  })}
                </div>
              </div>
            )}

            {/* ── Error ── */}
            {error && (
              <p className="text-[12px] text-danger mb-3 px-1">{error}</p>
            )}
          </div>
        </div>

        {/* ── Footer actions ── */}
        <div className="border-t border-overlay/[0.07] px-6 py-3 bg-surface shrink-0 flex items-center gap-2">
          <button
            onClick={onClose}
            className="flex-1 text-[12.5px] font-medium text-text-secondary bg-surface ring-1 ring-overlay/[0.08] rounded-lg py-2.5 hover:bg-overlay/[0.02]"
          >
            Cancel
          </button>
          <button
            onClick={handleLink}
            disabled={saving || !folderPath}
            className="flex-[1.4] text-[12.5px] font-semibold text-white bg-accent rounded-lg py-2.5 hover:brightness-105 disabled:opacity-50 flex items-center justify-center gap-1.5"
          >
            <FolderPlus className="w-4 h-4" />
            {saving ? "Creating…" : "Create workspace"}
          </button>
        </div>
      </div>
    </div>
  );
}
