import { useEffect, useState } from "react";
import { Waypoints, Terminal, Search, Folder, Plus, Layers } from "lucide-react";
import { ipc } from "../ipc";
import type { AgentDefinition, WorkspaceAgent } from "../ipc";

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
}

function AgentRow({ entry, isSelected, onSelect }: AgentRowProps) {
  const isCli = entry.type === "cli";
  const statusColor = STATUS_COLOR[entry.status];

  return (
    <button
      className={`w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg transition-colors${
        isSelected
          ? " bg-[#0a84ff]/10 ring-1 ring-[#0a84ff]/30"
          : " hover:bg-black/[0.04]"
      }`}
      onClick={onSelect}
    >
      <AgentAvatar entry={entry} />

      <div className="flex-1 text-left leading-tight min-w-0">
        <div className="text-[12.5px] font-semibold flex items-center gap-1.5 truncate">
          {entry.name}
          {isCli && <Terminal className="w-3 h-3 text-[#86868b] shrink-0" />}
        </div>
        <div className="text-[10.5px] text-[#86868b] truncate">{entry.meta}</div>
      </div>

      {/* Status dot */}
      <span
        className="w-2 h-2 rounded-full shrink-0"
        style={{ backgroundColor: statusColor }}
        role="img"
        aria-label={entry.status}
      />
    </button>
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
  onAddAgent?: () => void;
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
  onAddAgent,
  onOpenBlackboard,
  blackboardOpen,
}: RosterProps) {
  const [entries, setEntries] = useState<RosterEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState(false);
  const [search, setSearch] = useState("");

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
  }, [workspaceId]);

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
      {/* Workspace header — real workspaceName + folderPath; plain non-interactive display */}
      <div className="h-12 flex items-center gap-2 px-3.5 border-b border-black/[0.06] shrink-0">
        <div className="w-6 h-6 rounded-[7px] bg-[#0a84ff] text-white grid place-items-center text-[11px] font-bold ring-hair shrink-0">
          {wsLetter}
        </div>
        <div className="leading-tight text-left min-w-0">
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
          onClick={() => onAddAgent?.()}
          disabled={!onAddAgent}
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
    </aside>
  );
}
