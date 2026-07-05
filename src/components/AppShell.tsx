import { useEffect, useState } from "react";
import { ipc } from "../ipc";
import { useEvent } from "../ipc/events";
import { setThemePref } from "../lib/theme";
import type { Workspace, AgentDefinition } from "../ipc";
import { Rail } from "./Rail";
import { Roster } from "./Roster";
import { Builder } from "./Builder";
import { Library } from "./Library";
import { SkillLibrary } from "./SkillLibrary";
import { LinkFolder } from "./LinkFolder";
import { EditWorkspace } from "./EditWorkspace";
import { Settings } from "./Settings";
import { WorkspacePane } from "./WorkspacePane";
import { Blackboard } from "./Blackboard";
import { ChatHub } from "./ChatHub";
import { MemoryGraph } from "./MemoryGraph";
import { LaneBoard } from "./LaneBoard";
import { ArtifactsView } from "./ArtifactsView";

export function AppShell() {
  // Roster selection — propagated to WorkspacePane.focusInstanceId to switch
  // the active agent tab when the user clicks an agent in the Roster sidebar.
  const [selectedId, setSelectedId] = useState<string | null>(null);

  // ── Workspace state ────────────────────────────────────────────────────
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [activeWorkspaceId, setActiveWorkspaceId] = useState<string | null>(null);

  // ── Blackboard state ───────────────────────────────────────────────────
  const [showBlackboard, setShowBlackboard] = useState(false);

  // ── Chat Hub state — shares the center pane with the Blackboard, so
  //    opening one closes the other. ────────────────────────────────────────
  const [showChat, setShowChat] = useState(false);

  // ── Memory graph state — a third center-pane destination, mutually
  //    exclusive with the Blackboard and Chat Hub (same toggle pattern). ─────
  const [showMemory, setShowMemory] = useState(false);

  // ── Lane Board state — a fourth center-pane destination (agent work system,
  //    ADR 0008), mutually exclusive with Blackboard / Chat Hub / Memory. ──────
  const [showLaneBoard, setShowLaneBoard] = useState(false);

  // ── Artifacts state — a fifth center-pane destination, mutually exclusive
  //    with the other full-page workspace views. ──────────────────────────────
  const [showArtifacts, setShowArtifacts] = useState(false);

  // ── Design state — shell-plumbed now so Lane D can render inside the
  //    mounted workspace pane without touching the Rail. ──────────────────────
  const [showDesign, setShowDesign] = useState(false);

  // Bumped whenever the set of agents in the active workspace changes (add via
  // the Roster picker / remove an agent). Both the Roster and the WorkspacePane
  // key/refetch off it so the two views stay in sync without a manual reload.
  const [agentsVersion, setAgentsVersion] = useState(0);

  // ── Library state ──────────────────────────────────────────────────────
  const [showLibrary, setShowLibrary] = useState(false);
  /** Incremented after Builder saves so Library re-fetches agentDef.list. */
  const [libraryRefreshKey, setLibraryRefreshKey] = useState(0);

  // ── Skill Library state ────────────────────────────────────────────────
  const [showSkillLibrary, setShowSkillLibrary] = useState(false);

  // ── Builder state ──────────────────────────────────────────────────────
  const [showBuilder, setShowBuilder] = useState(false);
  /** Set when opening Builder in edit mode from Library. */
  const [builderInitialDef, setBuilderInitialDef] = useState<AgentDefinition | undefined>(
    undefined,
  );

  // ── Settings state ─────────────────────────────────────────────────────
  const [showSettings, setShowSettings] = useState(false);

  // ── LinkFolder state ───────────────────────────────────────────────────
  const [showLinkFolder, setShowLinkFolder] = useState(false);

  // ── EditWorkspace state ────────────────────────────────────────────────
  const [showEditWorkspace, setShowEditWorkspace] = useState(false);

  // Load workspaces from the DB on mount.
  // Falls back to an empty list if Tauri is not present (plain Vite dev).
  useEffect(() => {
    ipc.workspace
      .list()
      .then(setWorkspaces)
      .catch((err: unknown) => {
        // Plain `vite` dev (no Tauri shell) lands here; so does a real backend
        // failure. Surface it in dev rather than silently showing an empty Rail.
        if (import.meta.env.DEV) {
          console.error("AppShell: workspace.list failed", err);
        }
        setWorkspaces([]);
      });
  }, []);

  // Native menu / accelerator events from the Rust menu bar (⌘N, ⌘L, ⌘B, the
  // Appearance submenu). Each carries the clicked item's id.
  useEvent<string>("menu", (id) => {
    switch (id) {
      case "new_agent":
        setBuilderInitialDef(undefined);
        setShowBuilder(true);
        break;
      case "library":
        setShowBlackboard(false);
        setShowLibrary(true);
        break;
      case "toggle_blackboard":
        if (activeWorkspaceId) setShowBlackboard((v) => !v);
        break;
      case "theme_system":
        setThemePref("system");
        break;
      case "theme_light":
        setThemePref("light");
        break;
      case "theme_dark":
        setThemePref("dark");
        break;
    }
  });

  useEffect(() => {
    function onKeyDown(e: KeyboardEvent) {
      if (e.defaultPrevented || activeWorkspaceId == null || e.metaKey === false) return;

      const target = e.target;
      if (
        target instanceof HTMLElement &&
        (target.isContentEditable ||
          target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.tagName === "SELECT")
      ) {
        return;
      }

      const key = e.key.toLowerCase();
      if (key === "d" && !e.shiftKey && !e.altKey && !e.ctrlKey) {
        e.preventDefault();
        setShowArtifacts(false);
        setShowDesign((v) => !v);
        return;
      }
      if (key === "a" && e.shiftKey && !e.altKey && !e.ctrlKey) {
        e.preventDefault();
        setShowBlackboard(false);
        setShowChat(false);
        setShowMemory(false);
        setShowLaneBoard(false);
        setShowArtifacts((v) => !v);
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [activeWorkspaceId]);

  function handleSelectWorkspace(id: string) {
    // Optimistically update selection; handler only validates on the Rust side.
    setActiveWorkspaceId(id);
    // Clear the previous workspace's agent selection so a stale instance id
    // isn't carried into the remounted pane as focus.
    setSelectedId(null);
    // Switching workspace returns to that workspace's agent pane.
    setShowBlackboard(false);
    setShowChat(false);
    setShowMemory(false);
    setShowLaneBoard(false);
    setShowArtifacts(false);
    setShowDesign(false);
    ipc.workspace.use({ workspaceId: id }).catch(() => {
      // Ignore — the workspace just can't be found in the DB (stale id).
    });
  }

  const activeWorkspace = activeWorkspaceId
    ? (workspaces.find((w) => w.id === activeWorkspaceId) ?? null)
    : null;

  return (
    <div className="h-screen w-full flex flex-col overflow-hidden bg-bg-canvas text-text-primary select-none">
      {/*
       * ── Overlay titlebar drag region (28 px) ──────────────────────────
       * Tauri titleBarStyle "Overlay" floats the macOS traffic lights over
       * our content. This 28 px bar is the native-feeling title bar: it drags
       * the window and double-clicks to zoom (Tauri's drag-region handler).
       *
       * `data-tauri-drag-region` only fires when the CLICKED element carries
       * the attribute. The colored column-background children would otherwise
       * sit on top and swallow every click, so they are `pointer-events-none`
       * — that lets the hit-test fall through to this attributed parent.
       */}
      <div
        data-tauri-drag-region
        className="h-7 shrink-0 flex"
        aria-hidden="true"
      >
        {/* One continuous toolbar tint across all columns (macOS unified
            titlebar). The column dividers carry through from the panes below. */}
        {/* Rail column bg */}
        <div className="w-[56px] bg-sidebar border-r border-overlay/[0.06] pointer-events-none" />
        {/* Roster column bg */}
        <div className="w-[266px] bg-sidebar border-r border-overlay/[0.06] pointer-events-none" />
        {/* Main content bg */}
        <div className="flex-1 bg-sidebar pointer-events-none" />
      </div>

      {/* ── 3-pane layout ────────────────────────────────────────────── */}
      <div className="flex-1 flex overflow-hidden min-h-0">
        <Rail
          workspaces={workspaces}
          activeWorkspaceId={activeWorkspaceId}
          artifactsOpen={showArtifacts}
          designOpen={showDesign}
          onSelectWorkspace={handleSelectWorkspace}
          onOpenDesign={() => {
            if (!activeWorkspaceId) return;
            setShowArtifacts(false);
            setShowDesign((v) => !v);
          }}
          onOpenArtifacts={() => {
            if (!activeWorkspaceId) return;
            setShowBlackboard(false);
            setShowChat(false);
            setShowMemory(false);
            setShowLaneBoard(false);
            setShowArtifacts((v) => !v);
          }}
          onOpenLibrary={() => {
            setShowBlackboard(false);
            setShowLibrary(true);
          }}
          onOpenSkillLibrary={() => setShowSkillLibrary(true)}
          onOpenLinkFolder={() => setShowLinkFolder(true)}
          onOpenSettings={() => setShowSettings(true)}
        />

        {/* Roster + main stay mounted; the Library opens as an overlay sheet
            on top so the workspace refreshes live underneath a delete. */}
        <>
            <Roster
              workspaceId={activeWorkspaceId}
              workspaceName={activeWorkspace?.name}
              folderPath={activeWorkspace?.folderPath}
              selectedId={selectedId}
              onSelect={(id) => {
                // Selecting an agent returns from any center-pane screen to the pane.
                setShowBlackboard(false);
                setShowChat(false);
                setShowMemory(false);
                setShowLaneBoard(false);
                setShowArtifacts(false);
                setSelectedId(id);
              }}
              // "Create new agent…" (from inside the picker) still opens the Builder.
              onCreateAgent={() => {
                setBuilderInitialDef(undefined);
                setShowBuilder(true);
              }}
              agentsVersion={agentsVersion}
              onAgentsChanged={() => {
                setAgentsVersion((v) => v + 1);
                // A removed agent may have been the current selection.
                setSelectedId(null);
              }}
              // Blackboard needs a workspace to scope to — only toggle when one
              // is active (else the view would fall through to "Select a workspace").
              onOpenBlackboard={
                activeWorkspaceId
                  ? () => {
                      setShowArtifacts(false);
                      setShowChat(false);
                      setShowMemory(false);
                      setShowLaneBoard(false);
                      setShowBlackboard((v) => !v);
                    }
                  : undefined
              }
              blackboardOpen={showBlackboard}
              onOpenMemory={
                activeWorkspaceId
                  ? () => {
                      setShowArtifacts(false);
                      setShowBlackboard(false);
                      setShowChat(false);
                      setShowLaneBoard(false);
                      setShowMemory((v) => !v);
                    }
                  : undefined
              }
              memoryOpen={showMemory}
              onOpenChat={
                activeWorkspaceId
                  ? () => {
                      setShowArtifacts(false);
                      setShowBlackboard(false);
                      setShowMemory(false);
                      setShowLaneBoard(false);
                      setShowChat((v) => !v);
                    }
                  : undefined
              }
              chatOpen={showChat}
              onOpenLaneBoard={
                activeWorkspaceId
                  ? () => {
                      setShowArtifacts(false);
                      setShowBlackboard(false);
                      setShowChat(false);
                      setShowMemory(false);
                      setShowLaneBoard((v) => !v);
                    }
                  : undefined
              }
              laneBoardOpen={showLaneBoard}
              onEditWorkspace={
                activeWorkspaceId ? () => setShowEditWorkspace(true) : undefined
              }
            />

            {/* ── Main content: Chat Hub / Blackboard screen, else the live agent pane ─── */}
            {showChat && activeWorkspaceId ? (
              <ChatHub
                key={activeWorkspaceId}
                workspaceId={activeWorkspaceId}
                onClose={() => setShowChat(false)}
              />
            ) : showBlackboard && activeWorkspaceId ? (
              <Blackboard
                key={activeWorkspaceId}
                workspaceId={activeWorkspaceId}
                workspaceName={activeWorkspace?.name}
                onClose={() => setShowBlackboard(false)}
              />
            ) : showMemory && activeWorkspaceId ? (
              <MemoryGraph
                key={activeWorkspaceId}
                workspaceId={activeWorkspaceId}
                workspaceName={activeWorkspace?.name}
                onClose={() => setShowMemory(false)}
              />
            ) : showLaneBoard && activeWorkspaceId ? (
              <LaneBoard
                key={activeWorkspaceId}
                workspaceId={activeWorkspaceId}
                workspaceName={activeWorkspace?.name}
                onClose={() => setShowLaneBoard(false)}
              />
            ) : showArtifacts && activeWorkspaceId ? (
              <ArtifactsView
                key={activeWorkspaceId}
                workspaceId={activeWorkspaceId}
                workspaceName={activeWorkspace?.name}
                onClose={() => setShowArtifacts(false)}
              />
            ) : activeWorkspaceId ? (
              // Remount per workspace AND per agents change so the pane refetches
              // its tabs when an agent is added/removed.
              <WorkspacePane
                key={`${activeWorkspaceId}:${agentsVersion}`}
                workspaceId={activeWorkspaceId}
                focusInstanceId={selectedId}
                showDesign={showDesign}
                onOpenChat={() => {
                  setShowBlackboard(false);
                  setShowMemory(false);
                  setShowLaneBoard(false);
                  setShowArtifacts(false);
                  setShowChat(true);
                }}
              />
            ) : (
              <main className="flex-1 flex flex-col min-w-0 bg-surface">
                <div className="flex-1 grid place-items-center text-[13px] text-text-tertiary">
                  Select a workspace to start
                </div>
              </main>
            )}
        </>
      </div>

      {/* ── Agent Library overlay (sheet) ─────────────────────────────── */}
      {showLibrary && (
        <Library
          onClose={() => setShowLibrary(false)}
          onOpenBuilder={(def) => {
            setBuilderInitialDef(def);
            setShowBuilder(true);
          }}
          refreshKey={libraryRefreshKey}
          onAgentsChanged={() => {
            setAgentsVersion((v) => v + 1);
            setSelectedId(null);
          }}
        />
      )}

      {/* ── Skill Library overlay (sheet) ─────────────────────────────── */}
      {showSkillLibrary && <SkillLibrary onClose={() => setShowSkillLibrary(false)} />}

      {/* ── Agent builder overlay ─────────────────────────────────────── */}
      {showBuilder && (
        <Builder
          // Remount per def identity so the once-only useState prefill can't go
          // stale if a different agent is edited while the Builder is open.
          key={builderInitialDef?.id ?? "new"}
          initialDef={builderInitialDef}
          onClose={() => {
            setShowBuilder(false);
            setBuilderInitialDef(undefined);
          }}
          onSaved={() => {
            setShowBuilder(false);
            setBuilderInitialDef(undefined);
            // Bump key so Library re-fetches agentDef.list after a save/edit.
            setLibraryRefreshKey((k) => k + 1);
          }}
        />
      )}

      {/* ── Settings overlay ─────────────────────────────────────────── */}
      {showSettings && (
        <Settings onClose={() => setShowSettings(false)} />
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

      {/* ── Edit-workspace overlay ────────────────────────────────────── */}
      {showEditWorkspace && activeWorkspace && (
        <EditWorkspace
          workspace={activeWorkspace}
          onClose={() => setShowEditWorkspace(false)}
          onSaved={(updated) => {
            setWorkspaces((prev) => prev.map((w) => (w.id === updated.id ? updated : w)));
          }}
          onDeleted={(deletedId) => {
            setWorkspaces((prev) => prev.filter((w) => w.id !== deletedId));
            setActiveWorkspaceId(null);
            setSelectedId(null);
            setShowBlackboard(false);
            setShowChat(false);
            setShowMemory(false);
            setShowLaneBoard(false);
            setShowArtifacts(false);
            setShowDesign(false);
          }}
        />
      )}
    </div>
  );
}
