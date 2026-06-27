import { useEffect, useRef, useState } from "react";
import { ipc } from "../ipc";
import type { AgentDefinition, WorkspaceAgent } from "../ipc";
import { Terminal } from "./Terminal";
import { StdinBar } from "./StdinBar";

interface TerminalPaneProps {
  workspaceId: string;
}

// View-model for one CLI tab.
interface CliTab {
  instanceId: string;
  name: string;
  color: string;
  letter: string;
  status: WorkspaceAgent["status"];
}

// Status dot colors (matches Roster / AppShell tokens).
const STATUS_COLOR: Record<WorkspaceAgent["status"], string> = {
  running: "#30d158",
  waiting: "#ff9f0a",
  idle: "#c7c7cc",
};

/**
 * The terminal workspace pane: a tab strip of the workspace's CLI agents and a
 * live xterm.js terminal + stdin bar for the focused one.
 *
 * Tabs are derived by joining `instance.list` with `agentDef.list` and keeping
 * only `cli`-type agents (chat agents get a chat UI in a later task). Selecting
 * a tab lazily spawns its session (idempotent server-side; we also memoize the
 * instance→session mapping so re-selecting never re-spawns).
 */
export function TerminalPane({ workspaceId }: TerminalPaneProps) {
  const [tabs, setTabs] = useState<CliTab[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState(false);
  const [activeInstanceId, setActiveInstanceId] = useState<string | null>(null);
  // instanceId → sessionId for already-spawned sessions.
  const [sessions, setSessions] = useState<Record<string, string>>({});
  // instanceId → spawn error message (claude not installed, etc.).
  const [spawnErrors, setSpawnErrors] = useState<Record<string, string>>({});
  // Instances a spawn has already been kicked off for — kept in a ref so it
  // does NOT drive the spawn effect (including it in deps re-fires in-flight
  // spawns → duplicate spawn calls). The component remounts per workspace
  // (key={workspaceId} at the call site), so this resets naturally; we also
  // clear it in the load effect for safety.
  const spawnAttempted = useRef<Set<string>>(new Set());

  // Load + join instances with their definitions whenever the workspace changes.
  useEffect(() => {
    let active = true;
    setTabs([]);
    setLoading(true);
    setLoadError(false);
    setActiveInstanceId(null);
    setSessions({});
    setSpawnErrors({});
    spawnAttempted.current = new Set();

    Promise.all([ipc.instance.list({ workspaceId }), ipc.agentDef.list()])
      .then(([instances, defs]) => {
        if (!active) return;
        const byId = new Map<string, AgentDefinition>(defs.map((d) => [d.id, d]));
        const cliTabs: CliTab[] = [];
        for (const inst of instances) {
          const def = byId.get(inst.agentDefId);
          if (!def || def.type !== "cli") continue;
          cliTabs.push({
            instanceId: inst.id,
            name: def.name,
            color: def.color ?? "#6e6e73",
            letter: def.name[0]?.toUpperCase() ?? "?",
            status: inst.status,
          });
        }
        setTabs(cliTabs);
        setLoading(false);
        // Auto-focus the first CLI tab if there is one.
        if (cliTabs.length > 0) setActiveInstanceId(cliTabs[0].instanceId);
      })
      .catch((err: unknown) => {
        // A real backend failure (DB down, command missing) is distinct from an
        // empty workspace — surface it instead of masquerading as "no agents".
        // Plain `vite` dev without the Tauri shell also lands here.
        if (import.meta.env.DEV) {
          console.error("TerminalPane: instance.list / agentDef.list failed", err);
        }
        if (active) {
          setTabs([]);
          setLoading(false);
          setLoadError(true);
        }
      });

    return () => {
      active = false;
    };
  }, [workspaceId]);

  // Spawn the active instance's session lazily (once) when it becomes active.
  // The "already attempted" guard lives in a ref, so this effect depends ONLY
  // on activeInstanceId — it never re-fires for an in-flight spawn.
  useEffect(() => {
    if (activeInstanceId === null) return;
    const id = activeInstanceId;
    if (spawnAttempted.current.has(id)) return;
    spawnAttempted.current.add(id);

    let active = true;
    ipc.instance
      .spawn({ workspaceAgentId: id })
      .then((session) => {
        if (active) setSessions((prev) => ({ ...prev, [id]: session.id }));
      })
      .catch((err: unknown) => {
        // Allow a retry on a later re-select by clearing the attempt mark.
        spawnAttempted.current.delete(id);
        const msg = err instanceof Error ? err.message : String(err);
        if (active) setSpawnErrors((prev) => ({ ...prev, [id]: msg }));
      });

    return () => {
      active = false;
    };
  }, [activeInstanceId]);

  // Loading state: don't flash "no CLI agents" during the initial fetch.
  if (loading) {
    return (
      <main className="flex-1 flex flex-col min-w-0 bg-white">
        <div className="flex-1 grid place-items-center text-[13px] text-[#a1a1a6]">
          กำลังโหลด…
        </div>
      </main>
    );
  }

  // Empty / load-error state.
  if (tabs.length === 0) {
    return (
      <main className="flex-1 flex flex-col min-w-0 bg-white">
        <div className="flex-1 grid place-items-center text-[13px] text-[#a1a1a6] px-6 text-center">
          {loadError
            ? "โหลดรายชื่อ agent ไม่สำเร็จ"
            : "ยังไม่มี CLI agent ใน workspace นี้"}
        </div>
      </main>
    );
  }

  const activeSessionId = activeInstanceId ? (sessions[activeInstanceId] ?? null) : null;
  const activeError = activeInstanceId ? (spawnErrors[activeInstanceId] ?? null) : null;

  return (
    <main className="flex-1 flex flex-col min-w-0 bg-white">
      {/* Tab strip — one tab per CLI instance */}
      <div className="h-10 flex items-stretch gap-1 px-2 border-b border-black/[0.06] shrink-0 overflow-x-auto scroll-thin">
        {tabs.map((tab) => {
          const isActive = tab.instanceId === activeInstanceId;
          return (
            <button
              key={tab.instanceId}
              onClick={() => setActiveInstanceId(tab.instanceId)}
              className={`flex items-center gap-2 px-3 my-1.5 rounded-md text-[13px] transition-colors${
                isActive ? " bg-black/[0.06] font-semibold" : " hover:bg-black/[0.04] text-[#6e6e73]"
              }`}
            >
              <span
                className="w-2 h-2 rounded-full shrink-0"
                style={{ backgroundColor: STATUS_COLOR[tab.status] }}
              />
              <span className="truncate max-w-[140px]">{tab.name}</span>
            </button>
          );
        })}
      </div>

      {/* Active terminal / spawn-error / spawning state */}
      {activeError ? (
        <div className="flex-1 grid place-items-center bg-[#1e1e1e] text-[13px] text-[#ff453a] px-6 text-center">
          ไม่สามารถเปิด session: {activeError}
        </div>
      ) : activeSessionId ? (
        <Terminal key={activeSessionId} sessionId={activeSessionId} />
      ) : (
        <div className="flex-1 grid place-items-center bg-[#1e1e1e] text-[13px] text-[#a1a1a6]">
          กำลังเปิด session…
        </div>
      )}

      <StdinBar sessionId={activeSessionId} />
    </main>
  );
}
