import { useEffect, useMemo, useRef, useState } from "react";
import { Terminal as TerminalIcon, MessageSquare, Waypoints } from "lucide-react";
import { ipc } from "../ipc";
import type { AgentDefinition, Session, WorkspaceAgent } from "../ipc";
import { Terminal } from "./Terminal";
import { StdinBar } from "./StdinBar";
import { ChatView } from "./ChatView";
import { FusionView } from "./FusionView";
import { ContextDrawer } from "./ContextDrawer";
import type { RoutingTarget } from "./RoutingPicker";

// Re-exported here (the pane builds the roster) for consumers that think of the
// routing roster as a workspace concept; the type itself lives with the picker.
export type { RoutingTarget };

interface WorkspacePaneProps {
  workspaceId: string;
  /** Optional: when set, switches the active tab to the matching instanceId (if
   *  it exists in the loaded tabs). Used to honor Roster selection. The existing
   *  auto-focus-first-tab behavior in the load effect is unaffected — this effect
   *  runs after it and only overrides when the id is a real loaded tab. */
  focusInstanceId?: string | null;
}

// View-model for one agent tab (any type). Carries the full `AgentDefinition`
// so the Context drawer can render the active agent's config without a re-fetch.
interface AgentTab {
  instanceId: string;
  name: string;
  color: string;
  type: AgentDefinition["type"];
  status: WorkspaceAgent["status"];
  def: AgentDefinition;
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
export function WorkspacePane({ workspaceId, focusInstanceId }: WorkspacePaneProps) {
  const [tabs, setTabs] = useState<AgentTab[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState(false);
  const [activeInstanceId, setActiveInstanceId] = useState<string | null>(null);
  // instanceId → sessionId for already-spawned sessions.
  const [sessions, setSessions] = useState<Record<string, string>>({});
  // instanceId → the full Session object (carries context token counts for the
  // Context drawer's live meter). Parallel to `sessions` (which is only the id).
  const [sessionObjs, setSessionObjs] = useState<Record<string, Session>>({});
  // instanceId → spawn error message (claude not installed, etc.).
  const [spawnErrors, setSpawnErrors] = useState<Record<string, string>>({});
  // Instances a spawn has already been kicked off for — kept in a ref so it
  // does NOT drive the spawn effect (including it in deps re-fires in-flight
  // spawns → duplicate spawn calls). The component remounts per workspace
  // (key={workspaceId} at the call site), so this resets naturally; we also
  // clear it in the load effect for safety.
  const spawnAttempted = useRef<Set<string>>(new Set());

  // Latest `focusInstanceId`, mirrored into a ref so the load effect can honor a
  // pending Roster selection for its initial auto-focus WITHOUT listing
  // focusInstanceId as a dep (which would re-fetch on every selection change).
  const focusInstanceIdRef = useRef(focusInstanceId);
  useEffect(() => {
    focusInstanceIdRef.current = focusInstanceId;
  });

  // Load + join instances with their definitions whenever the workspace changes.
  useEffect(() => {
    let active = true;
    setTabs([]);
    setLoading(true);
    setLoadError(false);
    setActiveInstanceId(null);
    setSessions({});
    setSessionObjs({});
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
            def,
          });
        }
        setTabs(agentTabs);
        setLoading(false);
        // Auto-focus: honor a pending Roster selection if it resolves to a real
        // tab, else the first tab. Picking the focused tab up front avoids
        // spawning the first tab's session then immediately switching away.
        const pending = focusInstanceIdRef.current;
        const initial =
          agentTabs.find((t) => t.instanceId === pending)?.instanceId ??
          agentTabs[0]?.instanceId ??
          null;
        if (initial) setActiveInstanceId(initial);
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

  // Honor Roster selection: when focusInstanceId changes (user clicked an agent
  // in the Roster sidebar), switch the active tab to it — but ONLY if the tab is
  // loaded. Guard: if tabs haven't loaded yet (empty array or id not found) the
  // effect is a no-op; it re-fires when tabs arrive and the id then matches.
  // No double-spawn risk: the spawn effect below is guarded by spawnAttempted.
  useEffect(() => {
    if (focusInstanceId == null) return;
    if (tabs.some((t) => t.instanceId === focusInstanceId)) {
      setActiveInstanceId(focusInstanceId);
    }
  }, [focusInstanceId, tabs]);

  // Spawn the active instance's session lazily (once) when it becomes active.
  // The "already attempted" guard lives in a ref, so this effect depends ONLY
  // on activeInstanceId — it never re-fires for an in-flight spawn.
  //
  // NOTE: the result setState calls are intentionally NOT guarded by an
  // `active`/mounted flag. Under React 19 StrictMode the effect runs as
  // mount → cleanup → mount; an `active=false` cleanup would drop the FIRST
  // spawn's result while the SECOND invocation early-returns on the ref guard,
  // leaving the session id never recorded (stuck on "Opening session…").
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
        // Stash the full Session too so the Context drawer can seed its live
        // meter from the spawned session's token counts.
        setSessionObjs((prev) => ({ ...prev, [id]: session }));
      })
      .catch((err: unknown) => {
        // Allow a retry on a later re-select by clearing the attempt mark.
        spawnAttempted.current.delete(id);
        const msg = err instanceof Error ? err.message : String(err);
        setSpawnErrors((prev) => ({ ...prev, [id]: msg }));
      });
  }, [activeInstanceId]);

  // Routing roster — ALL agents (the picker shows self + others; consumers
  // exclude self where needed). Derived straight from the tab view-models.
  // Memoised so its identity is stable across renders. MUST be declared before
  // any early return so the hook order is unconditional (Rules of Hooks).
  const roster: RoutingTarget[] = useMemo(
    () =>
      tabs.map((t) => ({
        instanceId: t.instanceId,
        name: t.name,
        color: t.color,
        type: t.type,
      })),
    [tabs],
  );

  // Loading state: don't flash "no agents" during the initial fetch.
  if (loading) {
    return (
      <main className="flex-1 flex flex-col min-w-0 bg-white">
        <div className="flex-1 grid place-items-center text-[13px] text-[#a1a1a6]">
          Loading…
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
            ? "Failed to load agents"
            : "No agents in this workspace yet"}
        </div>
      </main>
    );
  }

  const activeTab = activeInstanceId
    ? (tabs.find((t) => t.instanceId === activeInstanceId) ?? null)
    : null;
  const activeSessionId = activeInstanceId ? (sessions[activeInstanceId] ?? null) : null;
  const activeSession = activeInstanceId ? (sessionObjs[activeInstanceId] ?? null) : null;
  const activeError = activeInstanceId ? (spawnErrors[activeInstanceId] ?? null) : null;

  return (
    <div className="flex-1 flex min-w-0 bg-white">
      <main className="flex-1 flex flex-col min-w-0">
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
            Select an agent
          </div>
        ) : activeTab.type === "cli" ? (
          // CLI → live terminal + stdin bar.
          activeError ? (
            <div className="flex-1 grid place-items-center bg-[#1e1e1e] text-[13px] text-[#ff453a] px-6 text-center">
              Couldn't open session: {activeError}
            </div>
          ) : activeSessionId ? (
            <>
              <Terminal key={activeSessionId} sessionId={activeSessionId} />
              <StdinBar
                sessionId={activeSessionId}
                instanceId={activeTab.instanceId}
                roster={roster}
              />
            </>
          ) : (
            <>
              <div className="flex-1 grid place-items-center bg-[#1e1e1e] text-[13px] text-[#a1a1a6]">
                Opening session…
              </div>
              <StdinBar sessionId={null} instanceId={activeTab.instanceId} roster={roster} />
            </>
          )
        ) : activeTab.type === "chat" ? (
          // Chat → custom chat UI.
          activeError ? (
            <div className="flex-1 grid place-items-center text-[13px] text-[#ff3b30] px-6 text-center">
              Couldn't open session: {activeError}
            </div>
          ) : activeSessionId ? (
            <ChatView
              key={activeSessionId}
              sessionId={activeSessionId}
              instanceId={activeTab.instanceId}
              roster={roster}
              agentName={activeTab.name}
              agentColor={activeTab.color}
            />
          ) : (
            <div className="flex-1 grid place-items-center text-[13px] text-[#a1a1a6]">
              Opening session…
            </div>
          )
        ) : (
          // Orchestrator → Fusion pipeline UI (drives the real M4.3 backend via
          // the instance id; it doesn't need the placeholder session object).
          <FusionView
            instanceId={activeTab.instanceId}
            def={activeTab.def}
            roster={roster}
          />
        )}
      </main>

      {/* Right Context drawer — renders only when a tab is active. The active
          tab carries the full AgentDefinition (its config is the drawer's data).
          `session` is the spawned Session (M4.1 snapshot manager): it seeds the
          live context meter and scopes the Memory · snapshots section. It is
          `null` until the spawn resolves; the drawer renders the meter/snapshots
          only once a real session exists. */}
      {activeTab !== null && (
        <ContextDrawer
          def={activeTab.def}
          status={activeTab.status}
          instanceId={activeTab.instanceId}
          roster={roster}
          session={activeSession}
        />
      )}
    </div>
  );
}
