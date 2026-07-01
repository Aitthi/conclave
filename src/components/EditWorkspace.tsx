import { useState } from "react";
import { X, Pencil, Trash2 } from "lucide-react";
import { ipc } from "../ipc";
import type { Workspace } from "../ipc";

// ── Color swatches (matches LinkFolder / Builder palette) ─────────────────────

const COLOR_SWATCHES = [
  "#ff3b30",
  "#0a84ff",
  "#30d158",
  "#ff9f0a",
  "#5e5ce6",
  "#d6409f",
  "#0fa3a3",
];

export interface EditWorkspaceProps {
  workspace: Workspace;
  onClose: () => void;
  /** The workspace was renamed/recolored — carries the updated row. */
  onSaved: (workspace: Workspace) => void;
  /** The workspace was deleted — carries the deleted id explicitly (rather than
   *  the caller re-deriving "the active workspace") so a future change to
   *  active-workspace tracking can't make this delete the wrong one. */
  onDeleted: (workspaceId: string) => void;
}

/**
 * Rename, recolor, or delete a workspace. Opened from the Roster's workspace
 * header (pencil icon). `folderPath` is not editable here — a workspace stays
 * bound to the folder it was linked from.
 */
export function EditWorkspace({ workspace, onClose, onSaved, onDeleted }: EditWorkspaceProps) {
  const [name, setName] = useState(workspace.name);
  const [color, setColor] = useState(workspace.color ?? COLOR_SWATCHES[0]);
  const isCustomColor = !COLOR_SWATCHES.includes(color);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Two-step delete so a stray click can't destroy a workspace: the first
  // click arms the confirm, the second (red) click commits.
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);

  // Neither mutation may run while the other is in flight — save and delete
  // both target the same row, and a race (e.g. two quick clicks) could apply
  // an update to an already-deleted workspace.
  const busy = saving || deleting;

  async function handleSave() {
    if (busy) return;
    const trimmed = name.trim();
    if (!trimmed) {
      setError("Name can't be empty.");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const updated = await ipc.workspace.update({
        workspaceId: workspace.id,
        name: trimmed,
        color,
      });
      onSaved(updated);
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  async function handleDelete() {
    if (busy) return;
    setDeleting(true);
    setError(null);
    try {
      await ipc.workspace.delete({ workspaceId: workspace.id });
      onDeleted(workspace.id);
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setDeleting(false);
      setConfirmingDelete(false);
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30">
      <div className="w-[420px] bg-surface rounded-2xl shadow-2xl flex flex-col overflow-hidden ring-1 ring-overlay/[0.08]">
        {/* ── Header ── */}
        <div className="h-12 flex items-center justify-between px-5 border-b border-overlay/[0.06] shrink-0">
          <div className="flex items-center gap-2">
            <Pencil className="w-4 h-4 text-accent" />
            <span className="text-[13px] font-semibold tracking-tight">Edit workspace</span>
          </div>
          <button
            onClick={onClose}
            disabled={busy}
            className="w-7 h-7 grid place-items-center rounded-md hover:bg-overlay/[0.05] text-text-secondary disabled:opacity-50"
            aria-label="Close"
          >
            <X className="w-[15px] h-[15px]" />
          </button>
        </div>

        {/* ── Body ── */}
        <div className="p-6">
          <div className="mb-4">
            <div className="text-[11px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
              Folder
            </div>
            <div className="text-[12.5px] font-mono text-text-muted truncate">
              {workspace.folderPath}
            </div>
          </div>

          <div className="mb-5">
            <div className="text-[11px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
              Name
            </div>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              className="w-full rounded-lg ring-1 ring-overlay/[0.10] bg-fill-softer px-3 h-10 text-[13px] outline-none focus:ring-accent/50"
            />
          </div>

          <div className="mb-5">
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
              {/* Custom color — native OS color panel via a fully transparent
                  <input type="color"> overlaid on a swatch-styled label. Shows
                  a rainbow ring when the current color isn't one of the presets
                  above (so the swatch itself doubles as the "active" indicator,
                  same as the preset buttons), otherwise a neutral conic
                  gradient signals "pick a custom color". */}
              <label
                className={`relative w-5 h-5 rounded-full shrink-0 cursor-pointer overflow-hidden transition-all ${
                  isCustomColor ? "ring-2 ring-offset-2" : "ring-1 ring-overlay/[0.15]"
                }`}
                style={
                  isCustomColor
                    ? ({ backgroundColor: color, "--tw-ring-color": color } as React.CSSProperties)
                    : {
                        background:
                          "conic-gradient(from 0deg, #ff3b30, #ff9f0a, #30d158, #0a84ff, #5e5ce6, #d6409f, #ff3b30)",
                      }
                }
                title="Custom color"
              >
                <input
                  type="color"
                  value={color}
                  onChange={(e) => setColor(e.target.value)}
                  className="absolute inset-0 w-full h-full opacity-0 cursor-pointer"
                  aria-label="Custom color"
                />
              </label>
            </div>
          </div>

          {error && <p className="text-[12px] text-danger mb-3">{error}</p>}

          {/* ── Danger zone ── */}
          <div className="pt-4 border-t border-overlay/[0.06] flex items-center justify-between">
            <div>
              <div className="text-[12.5px] font-semibold">Delete workspace</div>
              <div className="text-[10.5px] text-text-muted">
                Removes all agents, sessions, and blackboard data for this workspace.
              </div>
            </div>
            <button
              onClick={confirmingDelete ? handleDelete : () => setConfirmingDelete(true)}
              disabled={busy}
              className="flex items-center gap-1.5 text-[11.5px] font-semibold text-white bg-danger px-3 py-1.5 rounded-md shrink-0 disabled:opacity-50"
            >
              <Trash2 className="w-3.5 h-3.5" />
              {deleting ? "Deleting…" : confirmingDelete ? "Confirm delete" : "Delete"}
            </button>
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
            onClick={handleSave}
            disabled={saving}
            className="flex-[1.4] text-[12.5px] font-semibold text-white bg-accent rounded-lg py-2.5 hover:brightness-105 disabled:opacity-50"
          >
            {saving ? "Saving…" : "Save changes"}
          </button>
        </div>
      </div>
    </div>
  );
}
