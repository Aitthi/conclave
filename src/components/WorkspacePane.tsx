import { useEffect, useRef, useState } from "react";
import { Terminal as TerminalIcon, MessageSquare, Waypoints } from "lucide-react";
import { ipc } from "../ipc";
import type { AgentDefinition, WorkspaceAgent } from "../ipc";
import { Terminal } from "./Terminal";
import { StdinBar } from "./StdinBar";
import { ChatView } from "./ChatView";

interface WorkspacePaneProps {
  workspaceId: string;
}

// View-model for one agent tab (any type).
interface AgentTab {
  instanceId: string;
  name: string;
  color: string;
  type: AgentDefinition["type"];
  status: WorkspaceAgent["status"];
}

// Status dot colors (matches Roster / AppShell tokens).
const STATUS_COLOR: Record<WorkspaceAgent["status"], string> = {
  running: "#30d158",
  waiting: "#ff9f0a",
  idle: "#c7c7cc",
};

// Small per-type glyph so cli vs chat vs orchestrator is distinguishable.
function TypeGlyph({ type }: { type: AgentDefinition["type"] }) {
  const cls = "w-3 h-3 text-[#86868b] shrink-0";
  switch (type) {
    case "cli":
      return <TerminalIcon className={cls} />;
    case "chat":
      return <MessageSquare className={cls} />;
    case "orchestrator":
      return <Waypoints className={cls} />;
  }
}

/**
 * The main workspace pane: a tab strip of ALL the workspace's agents and the
 * body for the focused one, dispatched by agent type (cli → terminal, chat →
 * chat UI, orchestrator → placeholder).
 *
 * Tabs are derived by joining `instance.list` with `agentDef.list`. Selecting a
 * tab lazily spawns its session (idempotent server-side; we also memoize the
 * instance→session mapping so re-selecting never re-spawns).
 */
export function WorkspacePane({ workspaceId }: WorkspacePaneProps) {
  const [tabs, setTabs] = useState<AgentTab[]>([]);
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
        const agentTabs: AgentTab[] = [];
        for (const inst of instances) {
          const def = byId.get(inst.agentDefId);
          if (!def) continue;
          agentTabs.push({
            instanceId: inst.id,
            name: def.name,
            color: def.color ?? "#6e6e73",
            type: def.type,
            status: inst.status,
          });
        }
        setTabs(agentTabs);
        setLoading(false);
        // Auto-focus the first tab if there is one.
        if (agentTabs.length > 0) setActiveInstanceId(agentTabs[0].instanceId);
      })
      .catch((err: unknown) => {
        // A real backend failure (DB down, command missing) is distinct from an
        // empty workspace — surface it instead of masquerading as "no agents".
        // Plain `vite` dev without the Tauri shell also lands here.
        if (import.meta.env.DEV) {
          console.error("WorkspacePane: instance.list / agentDef.list failed", err);
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
  //
  // NOTE: the result setState calls are intentionally NOT guarded by an
  // `active`/mounted flag. Under React 19 StrictMode the effect runs as
  // mount → cleanup → mount; an `active=false` cleanup would drop the FIRST
  // spawn's result while the SECOND invocation early-returns on the ref guard,
  // leaving the session id never recorded (stuck on "กำลังเปิด session…").
  // setState on an unmounted component is a no-op in React 18+, so resolving
  // unconditionally is safe and avoids that StrictMode trap.
  useEffect(() => {
    if (activeInstanceId === null) return;
    const id = activeInstanceId;
    if (spawnAttempted.current.has(id)) return;
    spawnAttempted.current.add(id);

    ipc.instance
      .spawn({ workspaceAgentId: id })
      .then((session) => {
        setSessions((prev) => ({ ...prev, [id]: session.id }));
      })
      .catch((err: unknown) => {
        // Allow a retry on a later re-select by clearing the attempt mark.
        spawnAttempted.current.delete(id);
        const msg = err instanceof Error ? err.message : String(err);
        setSpawnErrors((prev) => ({ ...prev, [id]: msg }));
      });
  }, [activeInstanceId]);

  // Loading state: don't flash "no agents" during the initial fetch.
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
            : "ยังไม่มี agent ใน workspace นี้"}
        </div>
      </main>
    );
  }

  const activeTab = activeInstanceId
    ? (tabs.find((t) => t.instanceId === activeInstanceId) ?? null)
    : null;
  const activeSessionId = activeInstanceId ? (sessions[activeInstanceId] ?? null) : null;
  const activeError = activeInstanceId ? (spawnErrors[activeInstanceId] ?? null) : null;

  return (
    <main className="flex-1 flex flex-col min-w-0 bg-white">
      {/* Tab strip — one tab per instance, with a per-type glyph */}
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
              <TypeGlyph type={tab.type} />
            </button>
          );
        })}
      </div>

      {/* Active body — dispatched by the focused tab's agent type */}
      {activeTab === null ? (
        <div className="flex-1 grid place-items-center text-[13px] text-[#a1a1a6]">
          เลือก agent
        </div>
      ) : activeTab.type === "cli" ? (
        // CLI → live terminal + stdin bar.
        activeError ? (
          <div className="flex-1 grid place-items-center bg-[#1e1e1e] text-[13px] text-[#ff453a] px-6 text-center">
            ไม่สามารถเปิด session: {activeError}
          </div>
        ) : activeSessionId ? (
          <>
            <Terminal key={activeSessionId} sessionId={activeSessionId} />
            <StdinBar sessionId={activeSessionId} />
          </>
        ) : (
          <>
            <div className="flex-1 grid place-items-center bg-[#1e1e1e] text-[13px] text-[#a1a1a6]">
              กำลังเปิด session…
            </div>
            <StdinBar sessionId={null} />
          </>
        )
      ) : activeTab.type === "chat" ? (
        // Chat → custom chat UI.
        activeError ? (
          <div className="flex-1 grid place-items-center text-[13px] text-[#ff3b30] px-6 text-center">
            ไม่สามารถเปิด session: {activeError}
          </div>
        ) : activeSessionId ? (
          <ChatView
            key={activeSessionId}
            sessionId={activeSessionId}
            agentName={activeTab.name}
            agentColor={activeTab.color}
          />
        ) : (
          <div className="flex-1 grid place-items-center text-[13px] text-[#a1a1a6]">
            กำลังเปิด session…
          </div>
        )
      ) : (
        // Orchestrator / unknown → placeholder.
        <div className="flex-1 grid place-items-center text-[13px] text-[#a1a1a6] px-6 text-center">
          Orchestrator · Fusion — มาใน M4
        </div>
      )}
    </main>
  );
}
