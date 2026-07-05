import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Terminal as TerminalIcon, MessageSquare, Waypoints } from "lucide-react";
import { ipc, useEvent, EVENT_NAMES } from "../ipc";
import type { AgentDefinition, Session, WorkspaceAgent, SessionStatusEvent } from "../ipc";
import { Terminal } from "./Terminal";
import { StdinBar } from "./StdinBar";
import { ChatView } from "./ChatView";
import { FusionView } from "./FusionView";
import { ChatRail } from "./ChatRail";
import { DesignView } from "./DesignView";
import { ArtifactsView } from "./ArtifactsView";
import { ContextTopBar, ContextBottomBar } from "./ContextBars";
import { useSessionSnapshots } from "../lib/useSessionSnapshots";
import { getTermTabMode } from "../lib/termMode";
import type { RoutingTarget } from "./RoutingPicker";

// Terminal tab mode, read once — it cannot change within a page lifetime
// (see src/lib/termMode.ts). "remount" mounts only the active cli tab's xterm;
// "keep-alive" mounts every cli tab and toggles visibility.
const termTabMode = getTermTabMode();

// Re-exported here (the pane builds the roster) for consumers that think of the
// routing roster as a workspace concept; the type itself lives with the picker.
export type { RoutingTarget };

interface WorkspacePaneProps {
  workspaceId: string;
  /** For the Design view's header (DesignView has no other way to know it). */
  workspaceName?: string;
  /** Optional: when set, switches the active tab to the matching instanceId (if
   *  it exists in the loaded tabs). Used to honor Roster selection. The existing
   *  auto-focus-first-tab behavior in the load effect is unaffected — this effect
   *  runs after it and only overrides when the id is a real loaded tab. */
  focusInstanceId?: string | null;
  /** Called when the USER clicks a tab in the strip (not on auto-focus or the
   *  honor-focus effect). AppShell wires it to `setSelectedId` so Roster's
   *  selection, the terminal, StdinBar, and the tab strip never desync — the
   *  parent half of the bidirectional focus sync (plan D2). */
  onActiveInstanceChange?: (id: string) => void;
  /** Opens the workspace Chat Hub (threaded down to the Context drawer's
   *  "Open chat" affordance). */
  onOpenChat?: () => void;
  /** Design view (master plan Lane D): renders inside a permanent slot in
   *  THIS component's own tree (never a full-page replace-pane like Chat/
   *  Blackboard/Memory/LaneBoard) so the terminal column beside it never
   *  remounts. Owned by AppShell/Rail's ⌘D toggle. */
  designOpen?: boolean;
  /** Clears `designOpen` in the parent — wired to DesignView's close X. */
  onCloseDesign?: () => void;
  /** Artifacts view: shares the SAME canvas slot as Design (mutually exclusive
   *  with it — see AppShell), so opening Artifacts renders artifacts-left +
   *  terminal-right full-window, the terminal column never remounting. Owned by
   *  AppShell/Rail's ⌘⇧A toggle. */
  artifactsOpen?: boolean;
  /** Clears `artifactsOpen` in the parent — wired to ArtifactsView's close X. */
  onCloseArtifacts?: () => void;
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
  /** Skill ids the live session actually launched with (undefined before any
   *  launch) — feeds the Context drawer's Skills section drift hint. */
  launchedSkillIds?: string[];
}

// Status dot colors (matches Roster / AppShell tokens).
const STATUS_COLOR: Record<WorkspaceAgent["status"], string> = {
  running: "#30d158",
  waiting: "#ff9f0a",
  idle: "#c7c7cc",
};

// Small per-type glyph so cli vs chat vs orchestrator is distinguishable.
function TypeGlyph({ type }: { type: AgentDefinition["type"] }) {
  const cls = "w-3 h-3 text-text-muted shrink-0";
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
export function WorkspacePane({
  workspaceId,
  workspaceName,
  focusInstanceId,
  onActiveInstanceChange,
  onOpenChat,
  designOpen = false,
  onCloseDesign,
  artifactsOpen = false,
  onCloseArtifacts,
}: WorkspacePaneProps) {
  // The canvas slot is OPEN when either full-window view wants it. Design and
  // Artifacts are mutually exclusive (AppShell clears one when opening the
  // other), so at most one renders inside the slot at a time.
  const slotOpen = designOpen || artifactsOpen;
  const [tabs, setTabs] = useState<AgentTab[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState(false);
  const [activeInstanceId, setActiveInstanceId] = useState<string | null>(null);
  // instanceId → sessionId for already-spawned sessions.
  const [sessions, setSessions] = useState<Record<string, string>>({});
  // instanceId → the full Session object (carries context token counts for
  // ContextTopBar/ContextBottomBar). Parallel to `sessions` (id only).
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
            launchedSkillIds: inst.launchedSkillIds,
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

  // Last focusInstanceId this effect actually APPLIED. `tabs` gets a fresh array
  // identity on every `session:status` event (the status-patch setState below
  // maps a new array), so this effect re-fires constantly — without this guard
  // it would re-apply the SAME (possibly stale) focus every time, snapping the
  // active tab back after the user picked another one. Guarding on the applied
  // value makes each focusInstanceId take effect at most once (plan D2).
  const lastAppliedFocus = useRef<string | null>(null);

  // Honor Roster selection: when focusInstanceId changes (user clicked an agent
  // in the Roster sidebar), switch the active tab to it — but ONLY if the tab is
  // loaded. Guard: if tabs haven't loaded yet (empty array or id not found) the
  // effect is a no-op; it re-fires when tabs arrive and the id then matches (so
  // `tabs` stays in the deps for that late-load case).
  // No double-spawn risk: the spawn effect below is guarded by spawnAttempted.
  useEffect(() => {
    if (focusInstanceId == null) return;
    if (focusInstanceId === lastAppliedFocus.current) return;
    if (tabs.some((t) => t.instanceId === focusInstanceId)) {
      setActiveInstanceId(focusInstanceId);
      lastAppliedFocus.current = focusInstanceId;
    }
  }, [focusInstanceId, tabs]);

  // Spawn one instance's session, at most once (the attempt is marked in a ref).
  //
  // NOTE: the result setState calls are intentionally NOT guarded by an
  // `active`/mounted flag. Under React 19 StrictMode the effect runs as
  // mount → cleanup → mount; an `active=false` cleanup would drop the FIRST
  // spawn's result while the SECOND invocation early-returns on the ref guard,
  // leaving the session id never recorded (stuck on "Opening session…").
  // setState on an unmounted component is a no-op in React 18+, so resolving
  // unconditionally is safe and avoids that StrictMode trap.
  const spawnInstance = useCallback((id: string) => {
    if (spawnAttempted.current.has(id)) return;
    spawnAttempted.current.add(id);

    ipc.instance
      .spawn({ workspaceAgentId: id })
      .then((session) => {
        setSessions((prev) => ({ ...prev, [id]: session.id }));
        setSessionObjs((prev) => ({ ...prev, [id]: session }));
      })
      .catch((err: unknown) => {
        // Allow a retry on a later re-select by clearing the attempt mark.
        spawnAttempted.current.delete(id);
        const msg = err instanceof Error ? err.message : String(err);
        setSpawnErrors((prev) => ({ ...prev, [id]: msg }));
      });
  }, []);

  // Eager-spawn EVERY agent in the workspace as soon as the roster loads — so an
  // agent can RECEIVE `conclave tell` / injected messages even if a human never
  // clicked its tab. Previously only the active tab spawned, so an unopened agent
  // stayed offline and messages to it merely queued. Idempotent via the ref guard.
  useEffect(() => {
    for (const t of tabs) spawnInstance(t.instanceId);
  }, [tabs, spawnInstance]);

  // Fallback: also spawn whatever becomes active (e.g. a tab added after load).
  useEffect(() => {
    if (activeInstanceId !== null) spawnInstance(activeInstanceId);
  }, [activeInstanceId, spawnInstance]);

  // Live status dots on the tab strip. Spawning a session flips the
  // workspace_agent to "running" and emits `session:status`, but the tabs were
  // fetched at load with the pre-spawn status — so without this the tab dot
  // stays stale at idle. On any status event, re-read instance.list and patch
  // the tabs' status in place (the event carries only sessionId, so we refetch).
  useEvent<SessionStatusEvent>(EVENT_NAMES.sessionStatus, () => {
    ipc.instance
      .list({ workspaceId })
      .then((instances) => {
        const byId = new Map(instances.map((i) => [i.id, i]));
        setTabs((prev) =>
          prev.map((t) => {
            const next = byId.get(t.instanceId);
            if (!next) return t;
            // Patch status AND launchedSkillIds: a (re)spawn records a fresh
            // launch snapshot, and the Context drawer's Skills drift hint must
            // clear/appear accordingly. Identity-guarded to avoid re-renders on
            // unchanged rows.
            const skillsChanged =
              JSON.stringify(next.launchedSkillIds) !== JSON.stringify(t.launchedSkillIds);
            if (next.status === t.status && !skillsChanged) return t;
            return { ...t, status: next.status, launchedSkillIds: next.launchedSkillIds };
          }),
        );
      })
      .catch(() => {
        // Transient failure — the dot just stays at its last value.
      });
  });

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

  // instanceId → status, for the rail's room-chip live dots (R2). Memoized
  // like `roster` so unrelated WorkspacePane state changes don't re-render
  // the always-mounted rail (plan-review F5).
  const statuses: Record<string, WorkspaceAgent["status"]> = useMemo(
    () => Object.fromEntries(tabs.map((t) => [t.instanceId, t.status])),
    [tabs],
  );

  // Active-tab derivations, needed by the hooks below — MUST be declared
  // before any early return (Rules of Hooks).
  const activeTab = activeInstanceId
    ? (tabs.find((t) => t.instanceId === activeInstanceId) ?? null)
    : null;
  const activeSessionId = activeInstanceId ? (sessions[activeInstanceId] ?? null) : null;
  const activeSession = activeInstanceId ? (sessionObjs[activeInstanceId] ?? null) : null;

  // Single shared snapshot fetcher for the active session (plan-review F2) —
  // both ContextTopBar's Resume gate and ContextBottomBar's popover (Task 6)
  // consume THIS instance, never a second one of their own.
  const sessionSnapshots = useSessionSnapshots(activeSessionId ?? "");

  // Loading / empty / load-error states also carry the design slot — no
  // Terminal exists in either (nothing to protect from remounting), but
  // Design is still useful in a workspace with zero agents, so it shouldn't
  // disappear just because the tab strip hasn't loaded yet.
  if (loading || tabs.length === 0) {
    return (
      <div className="flex-1 flex min-w-0 bg-surface">
        <div className={slotOpen ? "flex-1 flex flex-col min-w-0 border-r border-overlay/[0.06]" : "w-0 shrink-0 overflow-hidden"}>
          {designOpen ? (
            <DesignView
              workspaceId={workspaceId}
              workspaceName={workspaceName}
              onClose={() => onCloseDesign?.()}
            />
          ) : artifactsOpen ? (
            <ArtifactsView
              workspaceId={workspaceId}
              workspaceName={workspaceName}
              onClose={() => onCloseArtifacts?.()}
            />
          ) : null}
        </div>
        <main className={slotOpen ? "w-[420px] min-w-[360px] shrink-0 flex flex-col" : "flex-1 flex flex-col min-w-0"}>
          <div className="flex-1 grid place-items-center text-[13px] text-text-tertiary px-6 text-center">
            {loading
              ? "Loading…"
              : loadError
                ? "Failed to load agents"
                : "No agents in this workspace yet"}
          </div>
        </main>
      </div>
    );
  }

  const activeError = activeInstanceId ? (spawnErrors[activeInstanceId] ?? null) : null;

  return (
    <div className="flex-1 flex min-w-0 bg-surface">
      {/* Canvas slot (master plan Lane D + canvas-slot-artifacts) — ALWAYS
          mounted in this exact tree position, in EVERY mode (Design open,
          Artifacts open, or neither). Only this div's className and its child
          (DesignView | ArtifactsView | nothing) change when the slot toggles;
          `<main>` below and `<ChatRail>` at the end are the SAME two siblings
          either way, just with different classNames — the terminal's ancestor
          chain never changes shape, so it never remounts (the one hard
          constraint of this lane). The slot's OWN child (DesignView/iframe or
          ArtifactsView) mounting/unmounting on toggle is fine — only the SLOT
          div, `<main>`, and the ChatRail wrapper must stay stable siblings. */}
      <div className={slotOpen ? "flex-1 flex flex-col min-w-0 border-r border-overlay/[0.06]" : "w-0 shrink-0 overflow-hidden"}>
        {designOpen ? (
          <DesignView
            workspaceId={workspaceId}
            workspaceName={workspaceName}
            onClose={() => onCloseDesign?.()}
          />
        ) : artifactsOpen ? (
          <ArtifactsView
            workspaceId={workspaceId}
            workspaceName={workspaceName}
            onClose={() => onCloseArtifacts?.()}
          />
        ) : null}
      </div>
      <main className={slotOpen ? "w-[420px] min-w-[360px] shrink-0 flex flex-col" : "flex-1 flex flex-col min-w-0"}>
        {/* Tab strip — one tab per instance, with a per-type glyph. 48px tall to
            line up with the Roster and Context headers so the three column
            dividers form one continuous macOS-style toolbar rule. The strip is
            also a window drag region: the tab buttons keep their own clicks
            (they don't carry the drag attribute), while the empty space drags
            the window / double-clicks to zoom. */}
        <div
          data-tauri-drag-region
          className="h-12 flex items-center gap-1 px-2 border-b border-overlay/[0.06] shrink-0 overflow-x-auto scroll-thin bg-sidebar"
        >
          {tabs.map((tab) => {
            const isActive = tab.instanceId === activeInstanceId;
            return (
              <button
                key={tab.instanceId}
                onClick={() => {
                  // User-driven focus change: update local state AND report up so
                  // the parent's Roster selection stays in sync (plan D2). Only
                  // here — never in auto-focus or the honor-focus effect, which
                  // apply values the parent already knows.
                  setActiveInstanceId(tab.instanceId);
                  onActiveInstanceChange?.(tab.instanceId);
                }}
                className={`flex items-center gap-2 px-3 h-7 rounded-md text-[13px] transition-colors${
                  isActive ? " bg-overlay/[0.06] font-semibold" : " hover:bg-overlay/[0.04] text-text-secondary"
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

        {/* Center top context bar — Skills popover + Session (Resume/Restart)
            + Config popover (R5), for the ACTIVE agent only. */}
        {activeTab && (
          <ContextTopBar
            def={activeTab.def}
            status={activeTab.status}
            instanceId={activeTab.instanceId}
            roster={roster}
            session={activeSession}
            hasHandoff={sessionSnapshots.hasHandoff}
            launchedSkillIds={activeTab.launchedSkillIds}
          />
        )}

        {/* CLI terminals — rendered per the terminal tab mode (src/lib/termMode.ts):
            • "keep-alive" (pre-remount behavior): every cli tab keeps its xterm
              MOUNTED and we only toggle visibility (`hidden`). A tab switch never
              remounts, so scrollback and the app's alt-screen / mouse modes survive
              — this is what kept wheel-scroll alive after a switch (a fresh xterm
              forgets the app is mid-TUI and the wheel goes dead). Mirrors VSCode's
              TerminalInstance, which reparents one long-lived xterm and flips a
              `.active` class rather than recreating it.
            • "remount" (default): only the ACTIVE tab's xterm is mounted; inactive
              tabs unmount and remount on return, restoring their pre-remount buffer
              from a serialize snapshot above a divider (see Terminal.tsx). The live
              TUI frame below is re-drawn by the mount jiggle's SIGWINCH. */}
        {tabs
          .filter((t) => t.type === "cli")
          .filter((t) => termTabMode === "keep-alive" || t.instanceId === activeInstanceId)
          .map((tab) => {
            const isActive = tab.instanceId === activeInstanceId;
            const sid = sessions[tab.instanceId] ?? null;
            const err = spawnErrors[tab.instanceId] ?? null;
            return (
              <div
                key={tab.instanceId}
                className={isActive ? "flex-1 flex flex-col min-h-0" : "hidden"}
              >
                {err ? (
                  <div className="flex-1 grid place-items-center bg-[#1e1e1e] text-[13px] text-danger px-6 text-center">
                    Couldn't open session: {err}
                  </div>
                ) : sid ? (
                  <>
                    {/* keyed by sessionId so a RESPAWN (new session) remounts, but a
                        tab switch (same sessionId) does NOT. */}
                    <Terminal key={sid} sessionId={sid} />
                    {/* Bottom context bar — ONLY for the active tab (guarantees a
                        single live snapshot/meter consumer even though every cli
                        tab's div is always mounted). */}
                    {isActive && (
                      <ContextBottomBar
                        def={tab.def}
                        instanceId={tab.instanceId}
                        session={activeSession}
                        snapshots={sessionSnapshots}
                      />
                    )}
                    <StdinBar sessionId={sid} instanceId={tab.instanceId} roster={roster} />
                  </>
                ) : (
                  <>
                    <div className="flex-1 grid place-items-center bg-[#1e1e1e] text-[13px] text-text-tertiary">
                      Opening session…
                    </div>
                    <StdinBar sessionId={null} instanceId={tab.instanceId} roster={roster} />
                  </>
                )}
              </div>
            );
          })}

        {/* Non-cli active body — chat / orchestrator / nothing selected. (cli is
            handled by the always-mounted layer above.) */}
        {activeTab === null ? (
          <div className="flex-1 grid place-items-center text-[13px] text-text-tertiary">
            Select an agent
          </div>
        ) : activeTab.type === "chat" ? (
          activeError ? (
            <div className="flex-1 grid place-items-center text-[13px] text-danger px-6 text-center">
              Couldn't open session: {activeError}
            </div>
          ) : activeSessionId ? (
            <>
              <ContextBottomBar
                def={activeTab.def}
                instanceId={activeTab.instanceId}
                session={activeSession}
                snapshots={sessionSnapshots}
              />
              <ChatView
                key={activeSessionId}
                sessionId={activeSessionId}
                instanceId={activeTab.instanceId}
                roster={roster}
                agentName={activeTab.name}
                agentColor={activeTab.color}
              />
            </>
          ) : (
            <div className="flex-1 grid place-items-center text-[13px] text-text-tertiary">
              Opening session…
            </div>
          )
        ) : activeTab.type === "orchestrator" ? (
          <>
            <ContextBottomBar
              def={activeTab.def}
              instanceId={activeTab.instanceId}
              session={activeSession}
              snapshots={sessionSnapshots}
            />
            {/* Orchestrator → Fusion pipeline UI (drives the real M4.3 backend
                via the instance id; it doesn't need the placeholder session
                object itself). */}
            <FusionView instanceId={activeTab.instanceId} def={activeTab.def} roster={roster} />
          </>
        ) : null}
      </main>

      {/* Right Chats rail — workspace-scoped, NOT per-tab: mounted
          unconditionally so switching tabs (or having none active) never
          resets its room selection or unread state. In any full-window slot
          mode (Design OR Artifacts) it's visually collapsed (judgment call —
          three full columns plus a narrowed terminal is too cramped) via a
          WRAPPING div's className, never by conditionally rendering ChatRail
          itself — it stays mounted, same "never unmount a sibling" discipline
          as the canvas slot above. `display: contents` when not collapsed
          makes the wrapper transparent to the flex layout, so it changes
          nothing when the slot is closed. `inert` + `aria-hidden` when
          collapsed pull its clipped-but-mounted focusables out of the tab
          order and the a11y tree (CSS clipping hides pixels only — Armin F1
          / plan D5). */}
      <div
        inert={slotOpen}
        aria-hidden={slotOpen || undefined}
        className={slotOpen ? "w-0 shrink-0 overflow-hidden" : "contents"}
      >
        <ChatRail workspaceId={workspaceId} roster={roster} statuses={statuses} onOpenChat={onOpenChat} />
      </div>
    </div>
  );
}
