import { useState, useEffect } from "react";
import {
  UsersRound,
  Search,
  LayoutGrid,
  List,
  Plus,
  Pencil,
  MessageSquare,
  Terminal,
  Waypoints,
  X,
} from "lucide-react";
import { ipc } from "../ipc";
import type { AgentDefinition, Workspace } from "../ipc";

// ── Types ────────────────────────────────────────────────────────────────────

export interface LibraryProps {
  onClose: () => void;
  /** Pass `def` to open Builder in edit mode; omit for "New agent". */
  onOpenBuilder: (def?: AgentDefinition) => void;
  /** Increment to force the def list to re-fetch (e.g. after Builder saves). */
  refreshKey?: number;
}

type FilterType = "all" | "chat" | "cli" | "orchestrator";

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

const FILTER_ITEMS: { value: FilterType; label: string; Icon: typeof LayoutGrid }[] = [
  { value: "all", label: "All agents", Icon: LayoutGrid },
  { value: "chat", label: "Chat agents", Icon: MessageSquare },
  { value: "cli", label: "CLI agents", Icon: Terminal },
  { value: "orchestrator", label: "Orchestrators", Icon: Waypoints },
];

// ── Helpers ──────────────────────────────────────────────────────────────────

function defColor(def: AgentDefinition): string {
  return def.color ?? TYPE_DEFAULT_COLOR[def.type];
}

// ── AgentAvatar ───────────────────────────────────────────────────────────────

interface AvatarProps {
  def: AgentDefinition;
  size?: "sm" | "lg";
}

function AgentAvatar({ def, size = "sm" }: AvatarProps) {
  const color = defColor(def);
  const letter = def.name.charAt(0).toUpperCase();
  const isOrch = def.type === "orchestrator";
  const cls =
    size === "lg"
      ? "w-12 h-12 rounded-[13px] text-[18px]"
      : "w-10 h-10 rounded-[11px] text-[15px]";
  return (
    <div
      className={`${cls} text-white grid place-items-center font-bold ring-hair shrink-0`}
      style={{ backgroundColor: color }}
    >
      {isOrch ? <Waypoints className="w-5 h-5" /> : letter}
    </div>
  );
}

// ── AgentCard (grid view) ─────────────────────────────────────────────────────

interface AgentCardProps {
  def: AgentDefinition;
  isSelected: boolean;
  onSelect: () => void;
  onEdit: () => void;
  onAddToWs: () => void;
}

function AgentCard({ def, isSelected, onSelect, onEdit, onAddToWs }: AgentCardProps) {
  const inCount = def.inWorkspaces ?? 0;
  const countLabel =
    inCount === 0
      ? "Not in any workspace"
      : `in ${inCount} workspace${inCount !== 1 ? "s" : ""}`;

  return (
    <div
      onClick={onSelect}
      className={`rounded-xl p-3.5 cursor-pointer transition-all ${
        isSelected
          ? "ring-1 ring-[#0a84ff]/30 bg-[#0a84ff]/[0.04]"
          : "ring-hair bg-white hover:ring-black/15"
      }`}
    >
      <div className="flex items-start gap-3">
        <AgentAvatar def={def} />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-1.5">
            <span className="text-[13.5px] font-semibold">{def.name}</span>
            <span className="text-[9.5px] font-medium text-[#86868b] bg-black/[0.05] px-1.5 py-px rounded">
              {TYPE_BADGE[def.type]}
            </span>
          </div>
          <div className="text-[11px] text-[#86868b] truncate">
            {[def.role, def.model].filter(Boolean).join(" · ") || def.type}
          </div>
        </div>
      </div>

      <div className="flex items-center gap-1.5 mt-3">
        <span className="text-[10.5px] text-[#86868b]">{countLabel}</span>
        {/* TODO: show exact workspace color swatches (requires instance.list per workspace) */}
      </div>

      <div className="flex items-center gap-1.5 mt-3">
        <button
          onClick={(e) => {
            e.stopPropagation();
            onEdit();
          }}
          className="flex-1 text-[11.5px] font-medium text-[#3a3a3c] bg-white ring-hair rounded-lg py-1.5 hover:bg-black/[0.02] flex items-center justify-center gap-1"
        >
          <Pencil className="w-3.5 h-3.5" />
          Edit
        </button>
        <button
          onClick={(e) => {
            e.stopPropagation();
            onAddToWs();
          }}
          className={`flex-1 text-[11.5px] font-semibold rounded-lg py-1.5 flex items-center justify-center gap-1 ${
            isSelected
              ? "text-white bg-[#0a84ff] hover:brightness-105"
              : "text-[#0a84ff] bg-[#0a84ff]/10 hover:bg-[#0a84ff]/15"
          }`}
        >
          <Plus className="w-3.5 h-3.5" />
          Add to ws
        </button>
      </div>
    </div>
  );
}

// ── DefinitionDetail (right-hand drawer) ──────────────────────────────────────

interface DetailProps {
  def: AgentDefinition;
  workspaces: Workspace[];
  onEdit: () => void;
  onClose: () => void;
  onRefresh: () => void;
}

function DefinitionDetail({ def, workspaces, onEdit, onClose, onRefresh }: DetailProps) {
  const [selectedWsIds, setSelectedWsIds] = useState<Set<string>>(new Set());
  const [adding, setAdding] = useState(false);
  const [addError, setAddError] = useState<string | null>(null);

  // Reset workspace selection when the viewed definition changes.
  useEffect(() => {
    setSelectedWsIds(new Set());
    setAddError(null);
  }, [def.id]);

  function toggleWs(id: string) {
    setSelectedWsIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  async function handleAdd() {
    if (selectedWsIds.size === 0) return;
    setAdding(true);
    setAddError(null);
    try {
      await ipc.agentDef.addToWorkspace({
        agentDefId: def.id,
        workspaceIds: [...selectedWsIds],
      });
      setSelectedWsIds(new Set());
      onRefresh(); // re-fetch defs so inWorkspaces count updates
    } catch (e) {
      setAddError(e instanceof Error ? e.message : String(e));
    } finally {
      setAdding(false);
    }
  }

  const inCount = def.inWorkspaces ?? 0;

  return (
    <aside className="w-[306px] border-l border-black/[0.06] flex flex-col shrink-0 bg-[#f5f5f7]">
      {/* Header */}
      <div className="h-12 flex items-center justify-between px-4 border-b border-black/[0.06] shrink-0">
        <span className="text-[12px] font-semibold text-[#6e6e73] tracking-tight truncate">
          Definition · {def.name}
        </span>
        <button
          onClick={onClose}
          className="w-6 h-6 grid place-items-center rounded-md hover:bg-black/[0.06] text-[#86868b] shrink-0 ml-1"
          aria-label="Close detail panel"
        >
          <X className="w-3.5 h-3.5" />
        </button>
      </div>

      <div className="flex-1 overflow-y-auto p-3 space-y-4">
        {/* Identity */}
        <div className="flex items-center gap-3">
          <AgentAvatar def={def} size="lg" />
          <div className="leading-tight min-w-0">
            <div className="text-[15px] font-semibold tracking-tight">{def.name}</div>
            <div className="text-[11px] text-[#86868b] truncate">
              {[def.role, def.model].filter(Boolean).join(" · ") || def.type}
            </div>
          </div>
        </div>

        {/* Configuration */}
        <div>
          <div className="text-[10px] font-bold tracking-wider text-[#a1a1a6] uppercase mb-1.5 px-0.5">
            Configuration
          </div>
          <div className="rounded-xl ring-hair bg-white divide-y divide-black/[0.05]">
            <div className="flex items-center justify-between px-2.5 py-1.5">
              <span className="text-[11.5px] text-[#6e6e73]">Type</span>
              <span className="text-[11.5px] font-medium">{TYPE_BADGE[def.type]}</span>
            </div>
            {def.cliKind && (
              <div className="flex items-center justify-between px-2.5 py-1.5">
                <span className="text-[11.5px] text-[#6e6e73]">CLI</span>
                <span className="text-[11.5px] font-medium">{def.cliKind}</span>
              </div>
            )}
            {def.model && (
              <div className="flex items-center justify-between px-2.5 py-1.5">
                <span className="text-[11.5px] text-[#6e6e73]">Model</span>
                <span className="text-[11.5px] font-medium truncate max-w-[160px]">
                  {def.model}
                </span>
              </div>
            )}
            <div className="flex items-center justify-between px-2.5 py-1.5">
              <span className="text-[11.5px] text-[#6e6e73]">Harness</span>
              <span className="text-[11.5px] font-medium">
                {def.harnessMode === "own" ? "Own" : "Central"}
              </span>
            </div>
            {def.allowedSenders && (
              <div className="flex items-center justify-between px-2.5 py-1.5">
                <span className="text-[11.5px] text-[#6e6e73]">Messaging</span>
                <span className="text-[11.5px] font-medium">{def.allowedSenders}</span>
              </div>
            )}
          </div>
        </div>

        {/* Workspace count */}
        <div className="text-[11px] text-[#86868b] px-0.5">
          In{" "}
          <span className="font-medium text-[#3a3a3c]">
            {inCount} workspace{inCount !== 1 ? "s" : ""}
          </span>
          {/* TODO: list exact workspace names (requires instance.list per workspace, M2 follow-up) */}
        </div>

        {/* Add to workspaces */}
        <div>
          <div className="text-[10px] font-bold tracking-wider text-[#a1a1a6] uppercase mb-1.5 px-0.5">
            Add to workspace
          </div>
          {workspaces.length === 0 ? (
            <p className="text-[11px] text-[#a1a1a6] px-0.5">
              No workspaces — link a folder first
            </p>
          ) : (
            <div className="space-y-1">
              {workspaces.map((ws) => {
                const wsColor = ws.color ?? "#6e6e73";
                return (
                  <label
                    key={ws.id}
                    className="flex items-center gap-2 rounded-lg ring-hair bg-white px-2.5 py-2 cursor-pointer hover:bg-black/[0.02]"
                  >
                    <div
                      className="w-4 h-4 rounded-[4px] shrink-0"
                      style={{ backgroundColor: wsColor }}
                    />
                    <span className="flex-1 text-[12px] font-medium">{ws.name}</span>
                    <input
                      type="checkbox"
                      checked={selectedWsIds.has(ws.id)}
                      onChange={() => toggleWs(ws.id)}
                      className="accent-[#0a84ff] w-3.5 h-3.5"
                    />
                  </label>
                );
              })}

              {addError && (
                <p className="text-[11px] text-[#ff3b30] px-0.5 mt-1">{addError}</p>
              )}

              <button
                onClick={handleAdd}
                disabled={selectedWsIds.size === 0 || adding}
                className="w-full mt-1 text-[11.5px] font-semibold text-white bg-[#0a84ff] rounded-lg py-1.5 hover:brightness-105 disabled:opacity-40 flex items-center justify-center gap-1"
              >
                <Plus className="w-3.5 h-3.5" />
                {adding ? "Adding…" : "Add to selected"}
              </button>
            </div>
          )}
        </div>

        {/* Bottom action buttons */}
        <div className="flex items-center gap-2 pt-1">
          <button
            onClick={onEdit}
            className="flex-1 text-[12px] font-medium text-[#3a3a3c] bg-white ring-hair rounded-lg py-2 hover:bg-black/[0.02] flex items-center justify-center gap-1.5"
          >
            <Pencil className="w-3.5 h-3.5" />
            Edit definition
          </button>
          <button
            onClick={() => setSelectedWsIds(new Set(workspaces.map((ws) => ws.id)))}
            className="flex-1 text-[12px] font-semibold text-white bg-[#0a84ff] rounded-lg py-2 hover:brightness-105 flex items-center justify-center gap-1.5"
          >
            <Plus className="w-3.5 h-3.5" />
            Add to ws
          </button>
        </div>
      </div>
    </aside>
  );
}

// ── Library ───────────────────────────────────────────────────────────────────

export function Library({ onClose, onOpenBuilder, refreshKey }: LibraryProps) {
  const [defs, setDefs] = useState<AgentDefinition[]>([]);
  const [loadError, setLoadError] = useState(false);
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [filter, setFilter] = useState<FilterType>("all");
  const [search, setSearch] = useState("");
  const [selectedDefId, setSelectedDefId] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<"grid" | "list">("grid");

  async function loadDefs() {
    try {
      const result = await ipc.agentDef.list();
      setDefs(result);
      setLoadError(false);
    } catch (err: unknown) {
      if (import.meta.env.DEV) {
        console.error("Library: agentDef.list failed", err);
      }
      setDefs([]);
      setLoadError(true);
    }
  }

  // Re-fetch when refreshKey changes (Builder saved a def) or on initial mount.
  useEffect(() => {
    loadDefs();
  }, [refreshKey]);

  // Workspace list is only needed for the detail drawer; load once.
  useEffect(() => {
    ipc.workspace.list().then(setWorkspaces).catch(() => setWorkspaces([]));
  }, []);

  const filteredDefs = defs.filter((def) => {
    if (filter !== "all" && def.type !== filter) return false;
    if (search.trim()) {
      const q = search.toLowerCase();
      return (
        def.name.toLowerCase().includes(q) ||
        (def.role ?? "").toLowerCase().includes(q) ||
        (def.model ?? "").toLowerCase().includes(q)
      );
    }
    return true;
  });

  const selectedDef =
    selectedDefId ? (defs.find((d) => d.id === selectedDefId) ?? null) : null;

  const counts: Record<FilterType, number> = {
    all: defs.length,
    chat: defs.filter((d) => d.type === "chat").length,
    cli: defs.filter((d) => d.type === "cli").length,
    orchestrator: defs.filter((d) => d.type === "orchestrator").length,
  };

  const filterLabel: Record<FilterType, string> = {
    all: "All agents",
    chat: "Chat agents",
    cli: "CLI agents",
    orchestrator: "Orchestrators",
  };

  return (
    // Library fills the space to the right of the Rail (Roster + main content replaced).
    <div className="flex flex-1 min-w-0 overflow-hidden">

      {/* ── Category / nav sidebar ─────────────────────────────────────── */}
      <aside className="w-[230px] bg-[#f5f5f7] border-r border-black/[0.06] flex flex-col shrink-0">
        {/* Header */}
        <div className="h-12 flex items-center gap-2 px-4 border-b border-black/[0.06] shrink-0">
          <UsersRound className="w-[15px] h-[15px] text-[#0a84ff] shrink-0" />
          <span className="text-[13px] font-semibold tracking-tight">Agent Library</span>
          <button
            onClick={onClose}
            className="ml-auto w-6 h-6 grid place-items-center rounded-md hover:bg-black/[0.06] text-[#86868b] shrink-0"
            aria-label="Close Agent Library"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </div>

        {/* Search */}
        <div className="px-3 pt-3 pb-2">
          <div className="flex items-center gap-2 bg-black/[0.05] rounded-lg px-2.5 h-7">
            <Search className="w-[13px] h-[13px] text-[#86868b] shrink-0" />
            <input
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Search definitions"
              className="bg-transparent outline-none text-[12px] placeholder:text-[#a1a1a6] w-full"
            />
          </div>
        </div>

        {/* Type filters */}
        <div className="flex-1 overflow-y-auto px-2 pb-2 space-y-0.5">
          {FILTER_ITEMS.map(({ value, label, Icon }) => {
            const active = filter === value;
            return (
              <button
                key={value}
                onClick={() => setFilter(value)}
                className={`w-full flex items-center gap-2.5 px-2.5 py-1.5 rounded-lg text-left ${
                  active
                    ? "bg-[#0a84ff]/10 ring-1 ring-[#0a84ff]/30"
                    : "hover:bg-black/[0.04]"
                }`}
              >
                <Icon
                  className={`w-4 h-4 shrink-0 ${active ? "text-[#0a84ff]" : "text-[#6e6e73]"}`}
                />
                <span className={`text-[12.5px] ${active ? "font-semibold" : ""}`}>
                  {label}
                </span>
                <span className="ml-auto text-[10.5px] text-[#86868b]">{counts[value]}</span>
              </button>
            );
          })}

          {/* Workspace list (informational; filter-by-workspace is a future enhancement) */}
          {workspaces.length > 0 && (
            <>
              <div className="px-2.5 pt-3 pb-1 text-[10px] font-bold tracking-wider text-[#a1a1a6] uppercase">
                Workspaces
              </div>
              {workspaces.map((ws) => {
                const wsColor = ws.color ?? "#6e6e73";
                return (
                  <div
                    key={ws.id}
                    className="w-full flex items-center gap-2.5 px-2.5 py-1.5 rounded-lg text-[#3a3a3c]"
                  >
                    <div
                      className="w-4 h-4 rounded-[5px] shrink-0"
                      style={{ backgroundColor: wsColor }}
                    />
                    <span className="text-[12.5px]">{ws.name}</span>
                  </div>
                );
              })}
            </>
          )}
        </div>

        {/* New agent */}
        <div className="border-t border-black/[0.06] p-2 shrink-0">
          <button
            onClick={() => onOpenBuilder()}
            className="w-full flex items-center justify-center gap-1.5 px-2 py-2 rounded-lg bg-[#0a84ff] text-white hover:brightness-105"
          >
            <Plus className="w-4 h-4" />
            <span className="text-[12.5px] font-semibold">New agent</span>
          </button>
        </div>
      </aside>

      {/* ── Agent grid / list ──────────────────────────────────────────── */}
      <main className="flex-1 flex flex-col min-w-0 bg-white">
        {/* Column header */}
        <div className="h-12 flex items-center justify-between px-5 border-b border-black/[0.06] shrink-0">
          <div className="flex items-baseline gap-2">
            <span className="text-[13px] font-semibold tracking-tight">
              {filterLabel[filter]}
            </span>
            <span className="text-[11px] text-[#86868b]">
              Global definitions · add to any workspace
            </span>
          </div>
          <div className="flex items-center gap-1 text-[#6e6e73]">
            <button
              onClick={() => setViewMode("grid")}
              className={`w-7 h-7 grid place-items-center rounded-md ${
                viewMode === "grid" ? "bg-black/[0.05]" : "hover:bg-black/[0.05]"
              }`}
              aria-label="Grid view"
            >
              <LayoutGrid className="w-[15px] h-[15px]" />
            </button>
            <button
              onClick={() => setViewMode("list")}
              className={`w-7 h-7 grid place-items-center rounded-md ${
                viewMode === "list" ? "bg-black/[0.05]" : "hover:bg-black/[0.05]"
              }`}
              aria-label="List view"
            >
              <List className="w-[15px] h-[15px]" />
            </button>
          </div>
        </div>

        <div className="flex-1 overflow-y-auto p-5">
          {filteredDefs.length === 0 ? (
            /* ── Empty / error state ── */
            <div className="flex flex-col items-center justify-center h-full gap-3 text-center">
              <UsersRound className="w-10 h-10 text-[#c7c7cc]" />
              <p className="text-[14px] font-semibold text-[#6e6e73]">
                {loadError
                  ? "Failed to load agents"
                  : defs.length === 0
                    ? "No agents yet — create one"
                    : "No matching agents"}
              </p>
              <p className="text-[12px] text-[#a1a1a6]">
                {loadError
                  ? "Check the app is running correctly and try again"
                  : defs.length === 0
                    ? "Define agents once and deploy to any workspace"
                    : "Try a different filter or search term"}
              </p>
              {!loadError && defs.length === 0 && (
                <button
                  onClick={() => onOpenBuilder()}
                  className="mt-2 flex items-center gap-1.5 px-3.5 py-1.5 rounded-lg bg-[#0a84ff] text-white text-[12.5px] font-semibold hover:brightness-105"
                >
                  <Plus className="w-3.5 h-3.5" />
                  Create agent
                </button>
              )}
            </div>
          ) : viewMode === "grid" ? (
            /* ── Grid view ── */
            <div className="grid grid-cols-2 gap-3 max-w-[760px]">
              {filteredDefs.map((def) => (
                <AgentCard
                  key={def.id}
                  def={def}
                  isSelected={selectedDefId === def.id}
                  onSelect={() =>
                    setSelectedDefId(def.id === selectedDefId ? null : def.id)
                  }
                  onEdit={() => onOpenBuilder(def)}
                  onAddToWs={() => setSelectedDefId(def.id)}
                />
              ))}
            </div>
          ) : (
            /* ── List view ── */
            <div className="space-y-1.5 max-w-[760px]">
              {filteredDefs.map((def) => {
                const inCount = def.inWorkspaces ?? 0;
                const countLabel =
                  inCount === 0
                    ? "Not in any workspace"
                    : `in ${inCount} workspace${inCount !== 1 ? "s" : ""}`;
                const isSelected = selectedDefId === def.id;
                return (
                  <div
                    key={def.id}
                    onClick={() =>
                      setSelectedDefId(def.id === selectedDefId ? null : def.id)
                    }
                    className={`flex items-center gap-3 rounded-xl px-3.5 py-2.5 cursor-pointer ${
                      isSelected
                        ? "ring-1 ring-[#0a84ff]/30 bg-[#0a84ff]/[0.04]"
                        : "ring-hair bg-white hover:ring-black/15"
                    }`}
                  >
                    <AgentAvatar def={def} />
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-1.5">
                        <span className="text-[13.5px] font-semibold">{def.name}</span>
                        <span className="text-[9.5px] font-medium text-[#86868b] bg-black/[0.05] px-1.5 py-px rounded">
                          {TYPE_BADGE[def.type]}
                        </span>
                      </div>
                      <div className="text-[11px] text-[#86868b] truncate">
                        {[def.role, def.model].filter(Boolean).join(" · ") || def.type}
                      </div>
                    </div>
                    <span className="text-[10.5px] text-[#86868b] shrink-0">
                      {countLabel}
                    </span>
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        onOpenBuilder(def);
                      }}
                      className="w-6 h-6 grid place-items-center rounded-md hover:bg-black/[0.06] text-[#86868b] shrink-0"
                      aria-label={`Edit ${def.name}`}
                    >
                      <Pencil className="w-3.5 h-3.5" />
                    </button>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </main>

      {/* ── Right detail / instances drawer ───────────────────────────── */}
      {selectedDef && (
        <DefinitionDetail
          def={selectedDef}
          workspaces={workspaces}
          onEdit={() => onOpenBuilder(selectedDef)}
          onClose={() => setSelectedDefId(null)}
          onRefresh={loadDefs}
        />
      )}
    </div>
  );
}
