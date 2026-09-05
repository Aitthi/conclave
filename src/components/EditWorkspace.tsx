import { useEffect, useRef, useState } from "react";
import { Archive, Pencil, RotateCcw, Trash2, X } from "lucide-react";

import { ipc } from "../ipc";
import type { Workspace } from "../ipc";
import "./workspace-manager.css";

const COLOR_SWATCHES = [
  "#ff3b30",
  "#0a84ff",
  "#30d158",
  "#ff9f0a",
  "#5e5ce6",
  "#d6409f",
  "#0fa3a3",
];

type Mutation = "save" | "delete" | "stop" | "archive" | "restore";

export interface EditWorkspaceProps {
  workspace: Workspace;
  onClose: () => void;
  onSaved: (workspace: Workspace) => void;
  onStopped: (workspace: Workspace) => void;
  onArchived: (workspace: Workspace) => void;
  onRestored: (workspace: Workspace) => void;
  onDeleted: (workspaceId: string) => void;
}

function detail(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function EditWorkspace({
  workspace,
  onClose,
  onSaved,
  onStopped,
  onArchived,
  onRestored,
  onDeleted,
}: EditWorkspaceProps) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [name, setName] = useState(workspace.name);
  const [color, setColor] = useState(workspace.color ?? COLOR_SWATCHES[0]);
  const [mutation, setMutation] = useState<Mutation | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [confirmingStop, setConfirmingStop] = useState(false);
  const archived = workspace.archivedAt != null;
  const busy = mutation != null;
  const isCustomColor = !COLOR_SWATCHES.includes(color);

  useEffect(() => {
    const node = dialog.current;
    if (node && !node.open) node.showModal();
  }, []);

  function close() {
    if (busy) return;
    dialog.current?.close();
  }

  async function run<T>(kind: Mutation, operation: () => Promise<T>, done: (value: T) => void) {
    if (busy) return;
    setMutation(kind);
    setError(null);
    try {
      done(await operation());
    } catch (reason) {
      setError(detail(reason));
      setMutation(null);
      if (kind === "delete") setConfirmingDelete(false);
      if (kind === "stop") setConfirmingStop(false);
    }
  }

  function handleSave() {
    const trimmed = name.trim();
    if (!trimmed) {
      setError("Name can’t be empty.");
      return;
    }
    void run(
      "save",
      () => ipc.workspace.update({ workspaceId: workspace.id, name: trimmed, color }),
      (updated) => {
        onSaved(updated);
        dialog.current?.close();
      },
    );
  }

  function handleStop() {
    if (!confirmingStop) {
      setConfirmingStop(true);
      return;
    }
    void run(
      "stop",
      () => ipc.workspace.stop({ workspaceId: workspace.id }),
      (result) => {
        onStopped(result.workspace);
        setMutation(null);
        setConfirmingStop(false);
      },
    );
  }

  function handleArchive() {
    void run(
      "archive",
      () => ipc.workspace.archive({ workspaceId: workspace.id }),
      (updated) => {
        onArchived(updated);
        dialog.current?.close();
      },
    );
  }

  function handleRestore() {
    void run(
      "restore",
      () => ipc.workspace.restore({ workspaceId: workspace.id }),
      (updated) => {
        onRestored(updated);
        dialog.current?.close();
      },
    );
  }

  function handleDelete() {
    if (!confirmingDelete) {
      setConfirmingDelete(true);
      return;
    }
    void run(
      "delete",
      () => ipc.workspace.delete({ workspaceId: workspace.id }),
      () => {
        onDeleted(workspace.id);
        dialog.current?.close();
      },
    );
  }

  return (
    <dialog
      ref={dialog}
      className="workspace-dialog"
      data-workspace-id={workspace.id}
      data-workspace-settings-state={mutation ?? (error ? "error" : "ready")}
      aria-label={archived ? "Archived workspace" : "Edit workspace"}
      onCancel={(event) => {
        if (busy) event.preventDefault();
      }}
      onClose={onClose}
    >
      <div className="workspace-dialog-frame">
        <header>
          <div>
            {archived ? <Archive aria-hidden="true" /> : <Pencil aria-hidden="true" />}
            <h2>{archived ? "Archived workspace" : "Edit workspace"}</h2>
          </div>
          <button type="button" onClick={close} disabled={busy} aria-label="Close workspace settings">
            <X aria-hidden="true" />
          </button>
        </header>

        <div className="workspace-dialog-body">
          <p className="workspace-dialog-path" title={workspace.folderPath}>{workspace.folderPath}</p>

          <label className="workspace-field">
            <span>Name</span>
            <input
              value={name}
              disabled={archived || busy}
              onChange={(event) => setName(event.target.value)}
            />
          </label>

          <fieldset className="workspace-color-field" disabled={archived || busy}>
            <legend>Color</legend>
            <div>
              {COLOR_SWATCHES.map((swatch) => (
                <button
                  key={swatch}
                  type="button"
                  onClick={() => setColor(swatch)}
                  className={color === swatch ? "is-selected" : ""}
                  style={{ backgroundColor: swatch, "--workspace-swatch": swatch } as React.CSSProperties}
                  aria-label={`Color ${swatch}`}
                  aria-pressed={color === swatch}
                />
              ))}
              <label
                className={`workspace-custom-color${isCustomColor ? " is-selected" : ""}`}
                style={isCustomColor ? { background: color, "--workspace-swatch": color } as React.CSSProperties : undefined}
                title="Custom color"
              >
                <input type="color" value={color} onChange={(event) => setColor(event.target.value)} aria-label="Custom color" />
              </label>
            </div>
          </fieldset>

          {archived && (
            <p className="workspace-archived-lock">Restore this workspace before renaming or changing its color.</p>
          )}

          <section className="workspace-archive-section">
            <h3>
              {archived ? <RotateCcw aria-hidden="true" /> : <Archive aria-hidden="true" />}
              {archived ? "Restore workspace" : "Archive workspace"}
            </h3>
            <p>
              {archived
                ? "Restore to edit or open this workspace. It returns stopped and no agents launch."
                : "Hide this workspace from the Rail and normal list. All agents, sessions, tasks, memory, artifacts and project files stay."}
            </p>
            {archived ? (
              <button type="button" data-workspace-action="restore" className="workspace-neutral-button" disabled={busy} onClick={handleRestore}>
                {mutation === "restore" ? "Restoring…" : "Restore workspace"}
              </button>
            ) : workspace.runState === "started" ? (
              <div className="workspace-stop-box">
                <strong>Stop workspace before archiving.</strong>
                <p>This workspace is started. Archiving never stops agents automatically.</p>
                {confirmingStop && (
                  <p role="alert">Stop all live runtimes and their current work? Saved records remain. Archive will still require a separate action.</p>
                )}
                <div>
                  <button type="button" data-workspace-action="stop" className="workspace-neutral-button" disabled={busy} onClick={handleStop}>
                    {mutation === "stop" ? "Stopping…" : confirmingStop ? "Confirm stop" : "Stop workspace"}
                  </button>
                  {confirmingStop && (
                    <button type="button" className="workspace-neutral-button" disabled={busy} onClick={() => setConfirmingStop(false)}>Cancel</button>
                  )}
                  <button type="button" className="workspace-primary-button" disabled>Archive</button>
                </div>
              </div>
            ) : (
              <div>
                <p className="workspace-archive-hint">Workspace stopped. Archive also checks for live or busy work.</p>
                <button type="button" data-workspace-action="archive" className="workspace-neutral-button" disabled={busy} onClick={handleArchive}>
                  {mutation === "archive" ? "Archiving…" : "Archive workspace"}
                </button>
              </div>
            )}
            {error && <p className="workspace-inline-error" role="alert">{error}</p>}
          </section>

          <section className="workspace-delete-section">
            <h3>Permanently delete workspace</h3>
            <p>Removes this workspace and its Conclave records. This cannot be undone.</p>
            {confirmingDelete && (
              <p role="alert">Permanently delete {workspace.name} and its Conclave records?</p>
            )}
            <div>
              <button type="button" data-workspace-action="delete" className="workspace-delete-button" disabled={busy} onClick={handleDelete}>
                <Trash2 aria-hidden="true" />
                {mutation === "delete" ? "Deleting…" : confirmingDelete ? "Confirm permanent delete" : "Delete…"}
              </button>
              {confirmingDelete && (
                <button type="button" className="workspace-neutral-button" disabled={busy} onClick={() => setConfirmingDelete(false)}>Cancel</button>
              )}
            </div>
          </section>
        </div>

        <footer>
          <button type="button" className="workspace-neutral-button" disabled={busy} onClick={close}>Cancel</button>
          {!archived && (
            <button type="button" data-workspace-action="save" className="workspace-primary-button" disabled={busy || !name.trim()} onClick={handleSave}>
              {mutation === "save" ? "Saving…" : "Save changes"}
            </button>
          )}
        </footer>
      </div>
    </dialog>
  );
}
