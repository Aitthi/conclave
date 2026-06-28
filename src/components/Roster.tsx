import { useEffect, useState } from "react";
import { Waypoints, Terminal, Search, Folder, Plus, Layers, X } from "lucide-react";
import { ipc, useEvent, EVENT_NAMES } from "../ipc";
import type { AgentDefinition, WorkspaceAgent, SessionStatusEvent } from "../ipc";

// ---------------------------------------------------------------------------
// View model for one agent row — derived from ipc.instance.list ⨝ ipc.agentDef.list.
// NO hardcoded data; the source is always the real DB join.
// ---------------------------------------------------------------------------

interface RosterEntry {
  instanceId: string;
  name: string;
  color: string;
  type: AgentDefinition["type"];
  status: WorkspaceAgent["status"];
  /** Subtitle derived honestly from the def — no fabricated strings. */
  meta: string;
}

// Status dot colors mapped from WorkspaceAgent.status (mirrors WorkspacePane).
const STATUS_COLOR: Record<WorkspaceAgent["status"], string> = {
  running: "#30d158",
  waiting: "#ff9f0a",
  idle: "#c7c7cc",
};

// Derive a subtitle from the definition — never fabricated.
function deriveMeta(def: AgentDefinition): string {
  switch (def.type) {
    case "orchestrator":
      return "Orchestrator";
    case "cli":
      return def.role ?? def.cliKind ?? "CLI";
    case "chat":
      return def.role ?? def.model ?? "Chat";
  }
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

interface AgentAvatarProps {
  entry: RosterEntry;
  size?: "sm" | "md";
}

function AgentAvatar({ entry, size = "md" }: AgentAvatarProps) {
  const dim =
    size === "sm" ? "w-6 h-6 rounded-[7px] text-[11px]" : "w-7 h-7 rounded-[8px] text-[12px]";
  const color = entry.color;

  if (entry.type === "orchestrator") {
    return (
      <div
        className={`${dim} text-white grid place-items-center ring-hair shrink-0`}
        style={{ backgroundColor: color }}
      >
        <Waypoints className="w-[15px] h-[15px]" />
      </div>
    );
  }

  return (
    <div
      className={`${dim} font-bold text-white grid place-items-center ring-hair shrink-0`}
      style={{ backgroundColor: color }}
    >
      {entry.name[0]}
    </div>
  );
}

interface AgentRowProps {
  entry: RosterEntry;
  isSelected: boolean;
  onSelect: () => void;
  onRemove: () => void;
  removing: boolean;
}

function AgentRow({ entry, isSelected, onSelect, onRemove, removing }: AgentRowProps) {
  const isCli = entry.type === "cli";
  const statusColor = STATUS_COLOR[entry.status];
  // Two-step removal so a stray click can't delete an agent: the first click
  // arms the confirm, the second (red) click commits.
  const [confirming, setConfirming] = useState(false);

  return (
    // Not a <button> (it now nests buttons) — a div with a role for the a11y tree.
    <div
      className={`group w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg transition-colors cursor-pointer${
        isSelected ? " bg-[#0a84ff]/10 ring-1 ring-[#0a84ff]/30" : " hover:bg-black/[0.04]"
      }`}
      role="button"
      tabIndex={0}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
      onMouseLeave={() => setConfirming(false)}
    >
      <AgentAvatar entry={entry} />

      <div className="flex-1 text-left leading-tight min-w-0">
        <div className="text-[12.5px] font-semibold flex items-center gap-1.5 truncate">
          {entry.name}
          {isCli && <Terminal className="w-3 h-3 text-[#86868b] shrink-0" />}
        </div>
        <div className="text-[10.5px] text-[#86868b] truncate">{entry.meta}</div>
      </div>

      {confirming ? (
        // Confirm step — commits the removal.
        <button
          onClick={(e) => {
            e.stopPropagation();
            onRemove();
          }}
          disabled={removing}
          className="text-[10.5px] font-semibold text-white bg-[#ff3b30] px-2 py-0.5 rounded-md shrink-0 disabled:opacity-50"
          title="Confirm removal from this workspace"
        >
          {removing ? "Removing…" : "Remove"}
        </button>
      ) : (
        <>
          {/* Status dot — hidden on hover to make room for the remove affordance. */}
          <span
            className="w-2 h-2 rounded-full shrink-0 group-hover:hidden"
            style={{ backgroundColor: statusColor }}
            role="img"
            aria-label={entry.status}
          />
          <button
            onClick={(e) => {
              e.stopPropagation();
              setConfirming(true);
            }}
            className="hidden group-hover:grid w-5 h-5 place-items-center rounded-md text-[#86868b] hover:bg-black/[0.06] hover:text-[#ff3b30] shrink-0"
            title="Remove from workspace"
            aria-label={`Remove ${entry.name} from workspace`}
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Roster
// ---------------------------------------------------------------------------

interface RosterProps {
  workspaceId: string | null;
  workspaceName?: string;
  folderPath?: string;
  selectedId: string | null;
  onSelect: (instanceId: string) => void;
  /** Open the Builder to define a brand-new agent (from inside the picker). */
  onCreateAgent?: () => void;
  /** Bumped externally whenever the workspace's agent set changes (re-fetch). */
  agentsVersion?: number;
  /** Notify the parent that this workspace's agent set changed (add / remove). */
  onAgentsChanged?: () => void;
  /** Open the per-workspace Blackboard screen. Absent → no workspace active. */
  onOpenBlackboard?: () => void;
  /** Whether the Blackboard screen is currently shown (drives active styling). */
  blackboardOpen?: boolean;
}

export function Roster({
  workspaceId,
  workspaceName,
  folderPath,
  selectedId,
  onSelect,
  onCreateAgent,
  agentsVersion,
  onAgentsChanged,
  onOpenBlackboard,
  blackboardOpen,
}: RosterProps) {
  const [entries, setEntries] = useState<RosterEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState(false);
  const [search, setSearch] = useState("");
  // Add-agent picker (choose an existing Library agent to add to this workspace).
  const [showPicker, setShowPicker] = useState(false);
  // Instance id currently being removed (disables its confirm button).
  const [removingId, setRemovingId] = useState<string | null>(null);

  async function handleRemove(instanceId: string) {
    setRemovingId(instanceId);
    try {
      await ipc.instance.remove({ workspaceAgentId: instanceId });
      onAgentsChanged?.();
    } catch (err) {
      if (import.meta.env.DEV) console.error("Roster: instance.remove failed", err);
    } finally {
      setRemovingId(null);
    }
  }

  // Fetch + join instances with their definitions whenever the workspace changes.
  // StrictMode-safe: `active` flag prevents a stale resolve from updating state
  // after a cleanup (same pattern as WorkspacePane).
  useEffect(() => {
    if (workspaceId === null) {
      setEntries([]);
      setLoading(false);
      setLoadError(false);
      return;
    }

    let active = true;
    setEntries([]);
    setLoading(true);
    setLoadError(false);

    Promise.all([ipc.instance.list({ workspaceId }), ipc.agentDef.list()])
      .then(([instances, defs]) => {
        if (!active) return;
        const byId = new Map<string, AgentDefinition>(defs.map((d) => [d.id, d]));
        const rosterEntries: RosterEntry[] = [];
        for (const inst of instances) {
          const def = byId.get(inst.agentDefId);
          if (!def) continue;
          rosterEntries.push({
            instanceId: inst.id,
            name: def.name,
            color: def.color ?? "#6e6e73",
            type: def.type,
            status: inst.status,
            meta: deriveMeta(def),
          });
        }
        setEntries(rosterEntries);
        setLoading(false);
      })
      .catch((err: unknown) => {
        if (!active) return;
        if (import.meta.env.DEV) {
          console.error("Roster: instance.list / agentDef.list failed", err);
        }
        setEntries([]);
        setLoading(false);
        setLoadError(true);
      });

    return () => {
      active = false;
    };
  }, [workspaceId, agentsVersion]);

  // Live status dots. A newly-added agent is "idle" until the WorkspacePane
  // lazily spawns its session, which flips the workspace_agent to "running" and
  // emits `session:status`. The roster fetched its statuses BEFORE that spawn,
  // so without this the dot stays stale at idle until a reload. On any status
  // event, re-read instance.list and patch the dots in place (no loading flash).
  // The event carries only sessionId, so we refetch rather than map directly.
  useEvent<SessionStatusEvent>(EVENT_NAMES.sessionStatus, () => {
    if (workspaceId === null) return;
    ipc.instance
      .list({ workspaceId })
      .then((instances) => {
        const byId = new Map(instances.map((i) => [i.id, i.status]));
        setEntries((prev) =>
          prev.map((e) => {
            const next = byId.get(e.instanceId);
            return next && next !== e.status ? { ...e, status: next } : e;
          }),
        );
      })
      .catch(() => {
        // Transient failure — the dot just stays at its last value.
      });
  });

  // Client-side search filter — case-insensitive match on name and meta.
  const q = search.trim().toLowerCase();
  const filtered = q
    ? entries.filter(
        (e) => e.name.toLowerCase().includes(q) || e.meta.toLowerCase().includes(q),
      )
    : entries;

  const orchestrators = filtered.filter((e) => e.type === "orchestrator");
  const cliAgents = filtered.filter((e) => e.type === "cli");
  const chatAgents = filtered.filter((e) => e.type === "chat");

  // Workspace header: use real name/folderPath; static blue avatar (no fake state).
  const wsLetter = workspaceName ? workspaceName[0].toUpperCase() : "—";

  return (
    <aside className="w-[266px] vibrancy border-r border-black/[0.06] flex flex-col shrink-0">
      {/* Workspace header — real workspaceName + folderPath; plain non-interactive
          display. Doubles as a window drag region (it has no controls): the
          `pointer-events-none` children let clicks fall through to the
          attributed container so dragging / double-click-zoom work here too. */}
      <div
        data-tauri-drag-region
        className="h-12 flex items-center gap-2 px-3.5 border-b border-black/[0.06] shrink-0"
      >
        <div className="w-6 h-6 rounded-[7px] bg-[#0a84ff] text-white grid place-items-center text-[11px] font-bold ring-hair shrink-0 pointer-events-none">
          {wsLetter}
        </div>
        <div className="leading-tight text-left min-w-0 pointer-events-none">
          <div className="text-[12.5px] font-semibold tracking-tight truncate">
            {workspaceName ?? "—"}
          </div>
          {folderPath && (
            <div className="text-[10px] text-[#86868b] truncate flex items-center gap-1">
              <Folder className="w-2.5 h-2.5 shrink-0" />
              <span className="font-mono">{folderPath}</span>
            </div>
          )}
        </div>
      </div>

      {/* Search — functional client-side filter */}
      <div className="px-3 pt-3 pb-2 shrink-0">
        <div className="flex items-center gap-2 bg-black/[0.05] rounded-lg px-2.5 h-7">
          <Search className="w-[13px] h-[13px] text-[#86868b] shrink-0" />
          <input
            type="search"
            aria-label="Search agents"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search agents"
            className="bg-transparent outline-none text-[12px] placeholder:text-[#a1a1a6] w-full"
          />
        </div>
      </div>

      {/* Agent list — state messages or grouped sections */}
      {workspaceId === null ? (
        <div className="flex-1 grid place-items-center text-[12px] text-[#a1a1a6] px-4 text-center">
          Select a workspace
        </div>
      ) : loading ? (
        <div className="flex-1 grid place-items-center text-[12px] text-[#a1a1a6]">
          Loading agents…
        </div>
      ) : loadError ? (
        <div className="flex-1 grid place-items-center text-[12px] text-[#a1a1a6]">
          Failed to load agents
        </div>
      ) : entries.length === 0 ? (
        <div className="flex-1 grid place-items-center text-[12px] text-[#a1a1a6] px-4 text-center">
          No agents in this workspace yet
        </div>
      ) : (
        <div className="flex-1 overflow-y-auto scroll-thin px-2 pb-2 space-y-3">
          {filtered.length === 0 ? (
            <div className="px-2 py-2 text-[12px] text-[#a1a1a6]">No matching agents</div>
          ) : (
            <>
              {orchestrators.length > 0 && (
                <div>
                  <div className="px-2 mb-1 text-[10px] font-bold tracking-wider text-[#a1a1a6] uppercase">
                    Orchestrator
                  </div>
                  {orchestrators.map((entry) => (
                    <AgentRow
                      key={entry.instanceId}
                      entry={entry}
                      isSelected={selectedId === entry.instanceId}
                      onSelect={() => onSelect(entry.instanceId)}
                      onRemove={() => handleRemove(entry.instanceId)}
                      removing={removingId === entry.instanceId}
                    />
                  ))}
                </div>
              )}

              {cliAgents.length > 0 && (
                <div>
                  <div className="px-2 mb-1 text-[10px] font-bold tracking-wider text-[#a1a1a6] uppercase">
                    CLI agents
                  </div>
                  <div className="space-y-0.5">
                    {cliAgents.map((entry) => (
                      <AgentRow
                        key={entry.instanceId}
                        entry={entry}
                        isSelected={selectedId === entry.instanceId}
                        onSelect={() => onSelect(entry.instanceId)}
                        onRemove={() => handleRemove(entry.instanceId)}
                        removing={removingId === entry.instanceId}
                      />
                    ))}
                  </div>
                </div>
              )}

              {chatAgents.length > 0 && (
                <div>
                  <div className="px-2 mb-1 text-[10px] font-bold tracking-wider text-[#a1a1a6] uppercase">
                    Chat agents
                  </div>
                  <div className="space-y-0.5">
                    {chatAgents.map((entry) => (
                      <AgentRow
                        key={entry.instanceId}
                        entry={entry}
                        isSelected={selectedId === entry.instanceId}
                        onSelect={() => onSelect(entry.instanceId)}
                        onRemove={() => handleRemove(entry.instanceId)}
                        removing={removingId === entry.instanceId}
                      />
                    ))}
                  </div>
                </div>
              )}
            </>
          )}
        </div>
      )}

      {/* Footer */}
      <div className="border-t border-black/[0.06] p-2 space-y-0.5 shrink-0">
        <button
          onClick={() => setShowPicker(true)}
          disabled={workspaceId === null}
          className="w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg text-[#0a84ff] hover:bg-[#0a84ff]/10 disabled:opacity-40 disabled:cursor-not-allowed"
        >
          <div className="w-7 h-7 rounded-[8px] border border-dashed border-[#0a84ff]/50 grid place-items-center shrink-0">
            <Plus className="w-[15px] h-[15px]" />
          </div>
          <span className="text-[12.5px] font-semibold">Add agent</span>
        </button>
        <button
          onClick={onOpenBlackboard}
          disabled={!onOpenBlackboard}
          className={`w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg transition-colors disabled:opacity-40 disabled:cursor-not-allowed ${
            blackboardOpen ? "bg-black/[0.06]" : "hover:bg-black/[0.04]"
          }`}
        >
          <div className="w-7 h-7 rounded-[8px] bg-[#1d1d1f] text-white grid place-items-center ring-hair shrink-0">
            <Layers className="w-[14px] h-[14px]" />
          </div>
          <div className="flex-1 text-left leading-tight">
            <div className="text-[12.5px] font-semibold">Blackboard</div>
            <div className="text-[10.5px] text-[#86868b]">Shared key/value store</div>
          </div>
        </button>
      </div>

      {/* Add-agent picker — choose an existing Library agent to add here. */}
      {showPicker && workspaceId !== null && (
        <AddAgentPicker
          workspaceId={workspaceId}
          onClose={() => setShowPicker(false)}
          onAdded={() => {
            setShowPicker(false);
            onAgentsChanged?.();
          }}
          onCreateAgent={
            onCreateAgent
              ? () => {
                  setShowPicker(false);
                  onCreateAgent();
                }
              : undefined
          }
        />
      )}
    </aside>
  );
}

// ---------------------------------------------------------------------------
// AddAgentPicker — modal listing Library agents not yet in this workspace.
// ---------------------------------------------------------------------------

interface AddAgentPickerProps {
  workspaceId: string;
  onClose: () => void;
  onAdded: () => void;
  onCreateAgent?: () => void;
}

function AddAgentPicker({ workspaceId, onClose, onAdded, onCreateAgent }: AddAgentPickerProps) {
  const [available, setAvailable] = useState<AgentDefinition[] | null>(null);
  const [error, setError] = useState(false);
  const [addingId, setAddingId] = useState<string | null>(null);

  // Load all defs ⨝ this workspace's instances → defs NOT already present.
  useEffect(() => {
    let active = true;
    Promise.all([ipc.agentDef.list(), ipc.instance.list({ workspaceId })])
      .then(([defs, instances]) => {
        if (!active) return;
        const present = new Set(instances.map((i) => i.agentDefId));
        setAvailable(defs.filter((d) => !present.has(d.id)));
      })
      .catch(() => {
        if (active) setError(true);
      });
    return () => {
      active = false;
    };
  }, [workspaceId]);

  async function handleAdd(def: AgentDefinition) {
    setAddingId(def.id);
    try {
      await ipc.agentDef.addToWorkspace({ agentDefId: def.id, workspaceIds: [workspaceId] });
      onAdded();
    } catch (err) {
      if (import.meta.env.DEV) console.error("AddAgentPicker: addToWorkspace failed", err);
      setAddingId(null);
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/30"
      onClick={onClose}
    >
      <div
        className="w-[420px] max-h-[70vh] bg-white rounded-2xl shadow-2xl flex flex-col overflow-hidden ring-1 ring-black/[0.08]"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="h-12 flex items-center justify-between px-5 border-b border-black/[0.06] shrink-0">
          <span className="text-[13px] font-semibold tracking-tight">Add agent to workspace</span>
          <button
            onClick={onClose}
            className="w-7 h-7 grid place-items-center rounded-md hover:bg-black/[0.05] text-[#6e6e73]"
            aria-label="Close"
          >
            <X className="w-[15px] h-[15px]" />
          </button>
        </div>

        <div className="flex-1 overflow-y-auto scroll-thin p-2 min-h-0">
          {available === null && !error ? (
            <div className="grid place-items-center py-8 text-[12px] text-[#a1a1a6]">Loading…</div>
          ) : error ? (
            <div className="grid place-items-center py-8 text-[12px] text-[#a1a1a6]">
              Failed to load agents
            </div>
          ) : available && available.length === 0 ? (
            <div className="grid place-items-center py-8 text-[12px] text-[#a1a1a6] px-6 text-center">
              Every agent in your Library is already in this workspace.
            </div>
          ) : (
            available?.map((def) => (
              <button
                key={def.id}
                onClick={() => handleAdd(def)}
                disabled={addingId !== null}
                className="w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg hover:bg-black/[0.04] disabled:opacity-50 text-left"
              >
                <div
                  className="w-7 h-7 rounded-[8px] text-white grid place-items-center text-[12px] font-bold ring-hair shrink-0"
                  style={{ backgroundColor: def.color ?? "#6e6e73" }}
                >
                  {def.type === "orchestrator" ? (
                    <Waypoints className="w-[15px] h-[15px]" />
                  ) : (
                    def.name[0]?.toUpperCase()
                  )}
                </div>
                <div className="flex-1 min-w-0 leading-tight">
                  <div className="text-[12.5px] font-semibold truncate">{def.name}</div>
                  <div className="text-[10.5px] text-[#86868b] truncate">{deriveMeta(def)}</div>
                </div>
                <span className="text-[11px] font-semibold text-[#0a84ff] shrink-0">
                  {addingId === def.id ? "Adding…" : "Add"}
                </span>
              </button>
            ))
          )}
        </div>

        {onCreateAgent && (
          <div className="border-t border-black/[0.06] p-2 shrink-0">
            <button
              onClick={onCreateAgent}
              className="w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg text-[#0a84ff] hover:bg-[#0a84ff]/10"
            >
              <div className="w-7 h-7 rounded-[8px] border border-dashed border-[#0a84ff]/50 grid place-items-center shrink-0">
                <Plus className="w-[15px] h-[15px]" />
              </div>
              <span className="text-[12.5px] font-semibold">Create new agent…</span>
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
