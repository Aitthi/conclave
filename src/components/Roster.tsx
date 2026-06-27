import { Waypoints, Terminal, Search, Folder, ChevronDown, Plus, Layers } from "lucide-react";
import type { AgentDefinition, WorkspaceAgent } from "../ipc";

// ---------------------------------------------------------------------------
// Combined view model — real data comes from ipc.call("instance.list", …)
// TODO(M1): replace with ipc call("instance.list", { workspaceId })
// ---------------------------------------------------------------------------

interface RosterEntry {
  def: AgentDefinition;
  instance: WorkspaceAgent;
  /** Human-readable subtitle: role · model / status info */
  meta: string;
}

// Status dot colors mapped from WorkspaceAgent.status
const STATUS_COLOR: Record<WorkspaceAgent["status"], string> = {
  running: "#30d158",
  waiting: "#ff9f0a",
  idle: "#c7c7cc",
};

// Hardcoded placeholder roster matching the approved prototype.
// TODO(M1): replace with ipc call("instance.list", { workspaceId })
const ROSTER: RosterEntry[] = [
  {
    def: {
      id: "def-maestro",
      name: "Maestro",
      role: "Orchestrator",
      type: "orchestrator",
      color: "#5e5ce6",
      harnessMode: "central",
      createdAt: "2026-01-01T00:00:00Z",
    },
    instance: {
      id: "inst-maestro",
      workspaceId: "ws-codeup",
      agentDefId: "def-maestro",
      status: "running",
      addedAt: "2026-01-01T00:00:00Z",
    },
    meta: "Fusion router · 3 models",
  },
  {
    def: {
      id: "def-atlas",
      name: "Atlas",
      role: "Claude Code",
      type: "cli",
      cliKind: "claude-code",
      color: "#ff7a45",
      harnessMode: "own",
      createdAt: "2026-01-01T00:00:00Z",
    },
    instance: {
      id: "inst-atlas",
      workspaceId: "ws-codeup",
      agentDefId: "def-atlas",
      status: "running",
      addedAt: "2026-01-01T00:00:00Z",
    },
    meta: "Claude Code · running tests",
  },
  {
    def: {
      id: "def-vega",
      name: "Vega",
      role: "Codex",
      type: "cli",
      cliKind: "codex",
      color: "#0fa3a3",
      harnessMode: "own",
      createdAt: "2026-01-01T00:00:00Z",
    },
    instance: {
      id: "inst-vega",
      workspaceId: "ws-codeup",
      agentDefId: "def-vega",
      status: "idle",
      addedAt: "2026-01-01T00:00:00Z",
    },
    meta: "Codex · idle",
  },
  {
    def: {
      id: "def-iris",
      name: "Iris",
      role: "Researcher",
      type: "chat",
      color: "#d6409f",
      model: "Opus 4.8",
      harnessMode: "own",
      createdAt: "2026-01-01T00:00:00Z",
    },
    instance: {
      id: "inst-iris",
      workspaceId: "ws-codeup",
      agentDefId: "def-iris",
      status: "running",
      addedAt: "2026-01-01T00:00:00Z",
    },
    meta: "Researcher · Opus 4.8",
  },
  {
    def: {
      id: "def-sol",
      name: "Sol",
      role: "Reviewer",
      type: "chat",
      color: "#ff9f0a",
      harnessMode: "own",
      createdAt: "2026-01-01T00:00:00Z",
    },
    instance: {
      id: "inst-sol",
      workspaceId: "ws-codeup",
      agentDefId: "def-sol",
      status: "waiting",
      addedAt: "2026-01-01T00:00:00Z",
    },
    meta: "Reviewer · your API",
  },
  {
    def: {
      id: "def-echo",
      name: "Echo",
      role: "Summarizer",
      type: "chat",
      color: "#32ade6",
      harnessMode: "own",
      createdAt: "2026-01-01T00:00:00Z",
    },
    instance: {
      id: "inst-echo",
      workspaceId: "ws-codeup",
      agentDefId: "def-echo",
      status: "idle",
      addedAt: "2026-01-01T00:00:00Z",
    },
    meta: "Summarizer · idle",
  },
];

const ORCHESTRATORS = ROSTER.filter((e) => e.def.type === "orchestrator");
const CLI_AGENTS = ROSTER.filter((e) => e.def.type === "cli");
const CHAT_AGENTS = ROSTER.filter((e) => e.def.type === "chat");

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

interface AgentAvatarProps {
  entry: RosterEntry;
  size?: "sm" | "md";
}

function AgentAvatar({ entry, size = "md" }: AgentAvatarProps) {
  const dim = size === "sm" ? "w-6 h-6 rounded-[7px] text-[11px]" : "w-7 h-7 rounded-[8px] text-[12px]";
  const color = entry.def.color ?? "#6e6e73";

  if (entry.def.type === "orchestrator") {
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
      {entry.def.name[0]}
    </div>
  );
}

interface AgentRowProps {
  entry: RosterEntry;
  isSelected: boolean;
  onSelect: () => void;
}

function AgentRow({ entry, isSelected, onSelect }: AgentRowProps) {
  const isCli = entry.def.type === "cli";
  const statusColor = STATUS_COLOR[entry.instance.status];

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
          {entry.def.name}
          {isCli && <Terminal className="w-3 h-3 text-[#86868b] shrink-0" />}
        </div>
        <div className="text-[10.5px] text-[#86868b] truncate">{entry.meta}</div>
      </div>

      {/* Status dot */}
      <span
        className="w-2 h-2 rounded-full shrink-0"
        style={{ backgroundColor: statusColor }}
      />
    </button>
  );
}

// ---------------------------------------------------------------------------
// Roster
// ---------------------------------------------------------------------------

interface RosterProps {
  selectedId: string | null;
  onSelect: (instanceId: string) => void;
  /** Open the per-workspace Blackboard screen. Absent → no workspace active. */
  onOpenBlackboard?: () => void;
  /** Whether the Blackboard screen is currently shown (drives active styling). */
  blackboardOpen?: boolean;
}

export function Roster({ selectedId, onSelect, onOpenBlackboard, blackboardOpen }: RosterProps) {
  return (
    <aside className="w-[266px] vibrancy border-r border-black/[0.06] flex flex-col shrink-0">
      {/* Workspace header */}
      <div className="h-12 flex items-center justify-between gap-2 px-3.5 border-b border-black/[0.06] shrink-0">
        <button className="flex items-center gap-2 min-w-0">
          <div className="w-6 h-6 rounded-[7px] bg-[#0a84ff] text-white grid place-items-center text-[11px] font-bold ring-hair shrink-0">
            C
          </div>
          <div className="leading-tight text-left min-w-0">
            <div className="text-[12.5px] font-semibold tracking-tight truncate">codeup</div>
            <div className="text-[10px] text-[#86868b] truncate flex items-center gap-1">
              <Folder className="w-2.5 h-2.5 shrink-0" />
              <span className="font-mono">~/code/codeup</span>
            </div>
          </div>
        </button>
        <button
          title="Switch workspace"
          className="w-6 h-6 grid place-items-center rounded-md hover:bg-black/[0.06] text-[#86868b] shrink-0"
        >
          <ChevronDown className="w-3.5 h-3.5" />
        </button>
      </div>

      {/* Search */}
      <div className="px-3 pt-3 pb-2">
        <div className="flex items-center gap-2 bg-black/[0.05] rounded-lg px-2.5 h-7">
          <Search className="w-[13px] h-[13px] text-[#86868b] shrink-0" />
          {/* TODO(M1): wire to roster filter */}
          <input
            readOnly
            placeholder="Search agents"
            className="bg-transparent outline-none text-[12px] placeholder:text-[#a1a1a6] w-full"
          />
        </div>
      </div>

      {/* Agent list */}
      <div className="flex-1 overflow-y-auto scroll-thin px-2 pb-2 space-y-3">
        {/* Orchestrator */}
        <div>
          <div className="px-2 mb-1 text-[10px] font-bold tracking-wider text-[#a1a1a6] uppercase">
            Orchestrator
          </div>
          {ORCHESTRATORS.map((entry) => (
            <AgentRow
              key={entry.instance.id}
              entry={entry}
              isSelected={selectedId === entry.instance.id}
              onSelect={() => onSelect(entry.instance.id)}
            />
          ))}
        </div>

        {/* CLI agents */}
        <div>
          <div className="px-2 mb-1 text-[10px] font-bold tracking-wider text-[#a1a1a6] uppercase">
            CLI agents
          </div>
          <div className="space-y-0.5">
            {CLI_AGENTS.map((entry) => (
              <AgentRow
                key={entry.instance.id}
                entry={entry}
                isSelected={selectedId === entry.instance.id}
                onSelect={() => onSelect(entry.instance.id)}
              />
            ))}
          </div>
        </div>

        {/* Chat agents */}
        <div>
          <div className="px-2 mb-1 text-[10px] font-bold tracking-wider text-[#a1a1a6] uppercase">
            Chat agents
          </div>
          <div className="space-y-0.5">
            {CHAT_AGENTS.map((entry) => (
              <AgentRow
                key={entry.instance.id}
                entry={entry}
                isSelected={selectedId === entry.instance.id}
                onSelect={() => onSelect(entry.instance.id)}
              />
            ))}
          </div>
        </div>
      </div>

      {/* Footer */}
      <div className="border-t border-black/[0.06] p-2 space-y-0.5 shrink-0">
        <button className="w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg text-[#0a84ff] hover:bg-[#0a84ff]/10">
          <div className="w-7 h-7 rounded-[8px] border border-dashed border-[#0a84ff]/50 grid place-items-center shrink-0">
            <Plus className="w-[15px] h-[15px]" />
          </div>
          <span className="text-[12.5px] font-semibold">Add agent</span>
        </button>
        <button
          onClick={onOpenBlackboard}
          disabled={!onOpenBlackboard}
          className={`w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg transition-colors disabled:opacity-40 ${
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
