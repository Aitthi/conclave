import { useState, useEffect } from "react";
import { UsersRound, Search, Plus, Pencil, Trash2, Waypoints, X, Sparkles } from "lucide-react";
import { ipc } from "../ipc";
import type { AgentDefinition } from "../ipc";

// ── Types ────────────────────────────────────────────────────────────────────

export interface LibraryProps {
  onClose: () => void;
  /** Pass `def` to open Builder in edit mode; omit for "New agent". */
  onOpenBuilder: (def?: AgentDefinition) => void;
  /** Open the AI drafter in agent mode ("Draft with AI"). */
  onOpenDrafter?: () => void;
  /** Increment to force the def list to re-fetch (e.g. after Builder saves). */
  refreshKey?: number;
  /** Notify the parent that workspace agent sets may have changed (a delete
   *  removes the def's instances from their workspaces). */
  onAgentsChanged?: () => void;
}

// ── Constants ─────────────────────────────────────────────────────────────────

const TYPE_BADGE: Record<AgentDefinition["type"], string> = {
  cli: "CLI",
  chat: "Chat",
  orchestrator: "Orchestrator",
};

const TYPE_DEFAULT_COLOR: Record<AgentDefinition["type"], string> = {
  cli: "#ff7a45",
  chat: "#0a84ff",
  orchestrator: "#5e5ce6",
};

function defColor(def: AgentDefinition): string {
  return def.color ?? TYPE_DEFAULT_COLOR[def.type];
}

// ── AgentAvatar ───────────────────────────────────────────────────────────────

function AgentAvatar({ def }: { def: AgentDefinition }) {
  const color = defColor(def);
  return (
    <div
      className="w-10 h-10 rounded-[11px] text-[15px] text-white grid place-items-center font-bold ring-hair shrink-0"
      style={{ backgroundColor: color }}
    >
      {def.type === "orchestrator" ? (
        <Waypoints className="w-5 h-5" />
      ) : (
        def.name.charAt(0).toUpperCase()
      )}
    </div>
  );
}

// ── AgentCard ─────────────────────────────────────────────────────────────────

interface AgentCardProps {
  def: AgentDefinition;
  onEdit: () => void;
  onDelete: () => void;
  deleting: boolean;
}

function AgentCard({ def, onEdit, onDelete, deleting }: AgentCardProps) {
  // Two-step delete so a stray click can't wipe a definition (and all its
  // workspace instances): the first click arms the red confirm.
  const [confirming, setConfirming] = useState(false);
  const inCount = def.inWorkspaces ?? 0;
  const countLabel =
    inCount === 0 ? "Not in any workspace" : `in ${inCount} workspace${inCount !== 1 ? "s" : ""}`;

  return (
    <div className="rounded-xl p-3.5 ring-hair bg-surface">
      <div className="flex items-start gap-3">
        <AgentAvatar def={def} />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-1.5">
            <span className="text-[13.5px] font-semibold">{def.name}</span>
            <span className="text-[9.5px] font-medium text-text-muted bg-overlay/[0.05] px-1.5 py-px rounded">
              {TYPE_BADGE[def.type]}
            </span>
          </div>
          <div className="text-[11px] text-text-muted truncate">
            {[def.role, def.model].filter(Boolean).join(" · ") || def.type}
          </div>
          <div className="text-[10.5px] text-text-muted mt-1">{countLabel}</div>
        </div>
      </div>

      <div className="flex items-center gap-1.5 mt-3">
        <button
          onClick={onEdit}
          className="flex-1 text-[11.5px] font-medium text-text-body bg-surface ring-hair rounded-lg py-1.5 hover:bg-overlay/[0.02] flex items-center justify-center gap-1"
        >
          <Pencil className="w-3.5 h-3.5" />
          Edit
        </button>
        {confirming ? (
          <button
            onClick={onDelete}
            disabled={deleting}
            onMouseLeave={() => setConfirming(false)}
            className="flex-1 text-[11.5px] font-semibold text-white bg-danger rounded-lg py-1.5 hover:brightness-105 disabled:opacity-50 flex items-center justify-center gap-1"
          >
            <Trash2 className="w-3.5 h-3.5" />
            {deleting ? "Deleting…" : "Confirm"}
          </button>
        ) : (
          <button
            onClick={() => setConfirming(true)}
            className="flex-1 text-[11.5px] font-medium text-danger bg-danger/[0.06] rounded-lg py-1.5 hover:bg-danger/10 flex items-center justify-center gap-1"
          >
            <Trash2 className="w-3.5 h-3.5" />
            Delete
          </button>
        )}
      </div>
    </div>
  );
}

// ── Library (right-side Sheet) ─────────────────────────────────────────────────

export function Library({ onClose, onOpenBuilder, onOpenDrafter, refreshKey, onAgentsChanged }: LibraryProps) {
  const [defs, setDefs] = useState<AgentDefinition[]>([]);
  const [loadError, setLoadError] = useState(false);
  const [search, setSearch] = useState("");
  const [deletingId, setDeletingId] = useState<string | null>(null);

  async function loadDefs() {
    try {
      setDefs(await ipc.agentDef.list());
      setLoadError(false);
    } catch (err: unknown) {
      if (import.meta.env.DEV) console.error("Library: agentDef.list failed", err);
      setDefs([]);
      setLoadError(true);
    }
  }

  // Re-fetch on mount and whenever the Builder saved a def (refreshKey bump).
  useEffect(() => {
    loadDefs();
  }, [refreshKey]);

  async function handleDelete(id: string) {
    setDeletingId(id);
    try {
      await ipc.agentDef.delete({ id });
      await loadDefs();
      // The def's instances were removed from their workspaces — refresh those.
      onAgentsChanged?.();
    } catch (err: unknown) {
      if (import.meta.env.DEV) console.error("Library: agentDef.delete failed", err);
      setDeletingId(null);
    }
  }

  const q = search.trim().toLowerCase();
  const filteredDefs = q
    ? defs.filter(
        (def) =>
          def.name.toLowerCase().includes(q) ||
          (def.role ?? "").toLowerCase().includes(q) ||
          (def.model ?? "").toLowerCase().includes(q),
      )
    : defs;

  return (
    <div className="fixed inset-0 z-40 flex justify-end">
      {/* Scrim — click to dismiss. */}
      <div className="absolute inset-0 bg-black/30" onClick={onClose} />

      {/* Sheet panel. */}
      <div className="relative w-[440px] max-w-full h-full bg-sidebar shadow-2xl flex flex-col ring-1 ring-overlay/[0.08]">
        {/* Header */}
        <div className="h-12 flex items-center gap-2 px-4 border-b border-overlay/[0.06] shrink-0">
          <UsersRound className="w-[15px] h-[15px] text-accent shrink-0" />
          <span className="text-[13px] font-semibold tracking-tight">Agent Library</span>
          <button
            onClick={onClose}
            className="ml-auto w-6 h-6 grid place-items-center rounded-md hover:bg-overlay/[0.06] text-text-muted shrink-0"
            aria-label="Close Agent Library"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </div>

        {/* Search */}
        <div className="px-3 pt-3 pb-2 shrink-0">
          <div className="flex items-center gap-2 bg-overlay/[0.05] rounded-lg px-2.5 h-7">
            <Search className="w-[13px] h-[13px] text-text-muted shrink-0" />
            <input
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Search agents"
              className="bg-transparent outline-none text-[12px] placeholder:text-text-tertiary w-full"
            />
          </div>
        </div>

        {/* List */}
        <div className="flex-1 overflow-y-auto scroll-thin px-3 pb-3 space-y-2">
          {filteredDefs.length === 0 ? (
            <div className="flex flex-col items-center justify-center h-full gap-2 text-center px-6">
              <UsersRound className="w-9 h-9 text-text-quaternary" />
              <p className="text-[13px] font-semibold text-text-secondary">
                {loadError
                  ? "Failed to load agents"
                  : defs.length === 0
                    ? "No agents yet"
                    : "No matching agents"}
              </p>
              <p className="text-[11.5px] text-text-tertiary">
                {loadError
                  ? "Check the app is running and try again"
                  : defs.length === 0
                    ? "Create one to add it to any workspace"
                    : "Try a different search term"}
              </p>
            </div>
          ) : (
            filteredDefs.map((def) => (
              <AgentCard
                key={def.id}
                def={def}
                onEdit={() => onOpenBuilder(def)}
                onDelete={() => handleDelete(def.id)}
                deleting={deletingId === def.id}
              />
            ))
          )}
        </div>

        {/* New agent · Draft with AI */}
        <div className="border-t border-overlay/[0.06] p-2 shrink-0 flex items-center gap-2">
          <button
            onClick={() => onOpenBuilder()}
            className="flex-1 flex items-center justify-center gap-1.5 px-2 py-2 rounded-lg bg-accent text-white hover:brightness-105"
          >
            <Plus className="w-4 h-4" />
            <span className="text-[12.5px] font-semibold">New agent</span>
          </button>
          <button
            onClick={onOpenDrafter}
            disabled={!onOpenDrafter}
            className="flex-1 flex items-center justify-center gap-1.5 px-2 py-2 rounded-lg bg-overlay/[0.06] text-text-primary hover:bg-overlay/[0.10] disabled:opacity-40 disabled:cursor-not-allowed"
          >
            <Sparkles className="w-4 h-4 text-accent" />
            <span className="text-[12.5px] font-semibold">Draft with AI</span>
          </button>
        </div>
      </div>
    </div>
  );
}
