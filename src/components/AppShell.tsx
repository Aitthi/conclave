import { useEffect, useState } from "react";
import { ipc } from "../ipc";
import type { Workspace } from "../ipc";
import { Rail } from "./Rail";
import { Roster } from "./Roster";
import { Builder } from "./Builder";
import { LinkFolder } from "./LinkFolder";

// Minimal info needed for the placeholder main pane header
// (kept local — M1 screens will own their own header)
interface SelectedAgent {
  instanceId: string;
  name: string;
  letter: string;
  color: string;
  type: "cli" | "chat" | "orchestrator";
  status: "running" | "idle" | "waiting";
}

// Agent type label for the placeholder main header
const AGENT_TYPE_LABEL: Record<SelectedAgent["type"], string> = {
  orchestrator: "Orchestrator",
  cli: "CLI agent",
  chat: "Chat agent",
};

// Status label map for the main pane
const STATUS_LABEL: Record<SelectedAgent["status"], string> = {
  running: "running",
  waiting: "waiting",
  idle: "idle",
};

const STATUS_DOT_COLOR: Record<SelectedAgent["status"], string> = {
  running: "#30d158",
  waiting: "#ff9f0a",
  idle: "#c7c7cc",
};

// Reverse-lookup map from instance id → SelectedAgent fields
// Kept in sync with Roster's ROSTER const.
// TODO(M1): derive from the same ipc call("instance.list") data.
const AGENT_META: Record<string, SelectedAgent> = {
  "inst-maestro": {
    instanceId: "inst-maestro",
    name: "Maestro",
    letter: "M",
    color: "#5e5ce6",
    type: "orchestrator",
    status: "running",
  },
  "inst-atlas": {
    instanceId: "inst-atlas",
    name: "Atlas",
    letter: "A",
    color: "#ff7a45",
    type: "cli",
    status: "running",
  },
  "inst-vega": {
    instanceId: "inst-vega",
    name: "Vega",
    letter: "V",
    color: "#0fa3a3",
    type: "cli",
    status: "idle",
  },
  "inst-iris": {
    instanceId: "inst-iris",
    name: "Iris",
    letter: "I",
    color: "#d6409f",
    type: "chat",
    status: "running",
  },
  "inst-sol": {
    instanceId: "inst-sol",
    name: "Sol",
    letter: "S",
    color: "#ff9f0a",
    type: "chat",
    status: "waiting",
  },
  "inst-echo": {
    instanceId: "inst-echo",
    name: "Echo",
    letter: "E",
    color: "#32ade6",
    type: "chat",
    status: "idle",
  },
};

export function AppShell() {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const agent = selectedId ? (AGENT_META[selectedId] ?? null) : null;

  // ── Workspace state ────────────────────────────────────────────────────
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [activeWorkspaceId, setActiveWorkspaceId] = useState<string | null>(null);

  // ── Builder state ──────────────────────────────────────────────────────
  const [showBuilder, setShowBuilder] = useState(false);

  // ── LinkFolder state ───────────────────────────────────────────────────
  const [showLinkFolder, setShowLinkFolder] = useState(false);

  // Load workspaces from the DB on mount.
  // Falls back to an empty list if Tauri is not present (plain Vite dev).
  useEffect(() => {
    ipc.workspace.list().then(setWorkspaces).catch(() => setWorkspaces([]));
  }, []);

  function handleSelectWorkspace(id: string) {
    // Optimistically update selection; handler only validates on the Rust side.
    setActiveWorkspaceId(id);
    ipc.workspace.use({ workspaceId: id }).catch(() => {
      // Ignore — the workspace just can't be found in the DB (stale id).
    });
  }

  return (
    <div className="h-screen w-full flex flex-col overflow-hidden bg-bg-canvas text-text-primary select-none">
      {/*
       * ── Overlay titlebar drag region (28 px) ──────────────────────────
       * Tauri titleBarStyle "Overlay" floats the macOS traffic lights over
       * our content. This 28 px bar acts as the drag handle for the window
       * and is visually split to match each column's background color so
       * nothing looks misaligned behind the traffic lights.
       */}
      <div
        data-tauri-drag-region
        className="h-7 shrink-0 flex"
        aria-hidden="true"
      >
        {/* Rail column bg */}
        <div className="w-[56px] bg-[#ebebed] border-r border-black/[0.06]" />
        {/* Roster column bg */}
        <div className="w-[266px] bg-[#f5f5f7] border-r border-black/[0.06]" />
        {/* Main content bg */}
        <div className="flex-1 bg-white" />
      </div>

      {/* ── 3-pane layout ────────────────────────────────────────────── */}
      <div className="flex-1 flex overflow-hidden min-h-0">
        <Rail
          workspaces={workspaces}
          activeWorkspaceId={activeWorkspaceId}
          onSelectWorkspace={handleSelectWorkspace}
          onOpenBuilder={() => setShowBuilder(true)}
          onOpenLinkFolder={() => setShowLinkFolder(true)}
        />
        <Roster selectedId={selectedId} onSelect={setSelectedId} />

        {/* ── Main content placeholder (M1 screens mount here) ───── */}
        <main className="flex-1 flex flex-col min-w-0 bg-white">
          {/* Mail.app-style column header, h-12, border-b */}
          <div className="h-12 flex items-center justify-between px-4 border-b border-black/[0.06] shrink-0">
            {agent ? (
              <>
                <div className="flex items-center gap-2.5">
                  {/* Agent avatar */}
                  <div
                    className="w-6 h-6 rounded-[7px] text-white grid place-items-center text-[12px] font-bold ring-hair shrink-0"
                    style={{ backgroundColor: agent.color }}
                  >
                    {agent.letter}
                  </div>
                  <div className="text-[13px] font-semibold tracking-tight flex items-center gap-1.5">
                    {agent.name}
                    <span className="text-[10px] font-medium text-[#86868b] bg-black/[0.04] px-1.5 py-px rounded-md">
                      {AGENT_TYPE_LABEL[agent.type]}
                    </span>
                  </div>
                  <span className="flex items-center gap-1 text-[11px] font-medium ml-1" style={{ color: STATUS_DOT_COLOR[agent.status] }}>
                    <span
                      className="w-1.5 h-1.5 rounded-full"
                      style={{ backgroundColor: STATUS_DOT_COLOR[agent.status] }}
                    />
                    {STATUS_LABEL[agent.status]}
                  </span>
                </div>
              </>
            ) : (
              <span className="text-[13px] text-[#a1a1a6] font-medium">
                Select an agent
              </span>
            )}
          </div>

          {/* Empty state / placeholder body */}
          <div className="flex-1 flex flex-col items-center justify-center gap-2 text-[#a1a1a6]">
            {agent ? (
              <>
                <div
                  className="w-10 h-10 rounded-[12px] text-white grid place-items-center text-[18px] font-bold ring-hair"
                  style={{ backgroundColor: agent.color }}
                >
                  {agent.letter}
                </div>
                <p className="text-[13px] font-semibold text-[#6e6e73]">{agent.name}</p>
                <p className="text-[11px] text-[#a1a1a6]">
                  {AGENT_TYPE_LABEL[agent.type]} · {STATUS_LABEL[agent.status]}
                </p>
                <p className="text-[11px] text-[#c7c7cc] mt-2">
                  {/* M1 screen content mounts here */}
                  Terminal / chat UI — coming in M1–M2
                </p>
              </>
            ) : (
              <>
                <p className="text-[14px] font-semibold text-[#6e6e73]">No agent selected</p>
                <p className="text-[12px] text-[#a1a1a6]">
                  Choose an agent from the roster
                </p>
              </>
            )}
          </div>
        </main>
      </div>

      {/* ── Agent builder overlay ─────────────────────────────────────── */}
      {showBuilder && (
        <Builder onClose={() => setShowBuilder(false)} />
      )}

      {/* ── Link-folder overlay ───────────────────────────────────────── */}
      {showLinkFolder && (
        <LinkFolder
          onClose={() => setShowLinkFolder(false)}
          onLinked={(ws) => {
            setWorkspaces((prev) => [...prev, ws]);
            setActiveWorkspaceId(ws.id);
            setShowLinkFolder(false);
          }}
        />
      )}
    </div>
  );
}
