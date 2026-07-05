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

  // Center-pane destinations that REPLACE the live WorkspacePane (each renders
  // full-page instead of it). This is the ONE canonical list — adding a new
  // center screen means adding its flag HERE, and both `workspacePaneVisible`
  // (which drives the WorkspacePane render branch below) and `designFullWindow`
  // update together, so the two can't silently diverge (Armin rot-guard).
  const centerScreenOpen =
    showChat || showBlackboard || showMemory || showLaneBoard || showArtifacts;

  // The live WorkspacePane (agent pane + the always-mounted Design slot) is the
  // visible center content exactly when a workspace is active and no center
  // screen is up. Used BOTH as the WorkspacePane render condition and as the
  // gate for full-window Design mode.
  const workspacePaneVisible = !!activeWorkspaceId && !centerScreenOpen;

  // Full-window Design mode (human ruling D3): while the Design view is OPEN and
  // actually on screen, hide the Rail + Roster columns so the window becomes
  // canvas-left + agent-terminal-right. `showDesign` alone is not enough — it is
  // a latent flag that stays true behind a center-pane screen, where DesignView
  // is NOT rendered (it lives inside the WorkspacePane branch); hiding the
  // sidebars then would strand the user in a full-screen center view with no
  // navigation. So gate on `workspacePaneVisible` — the exact condition under
  // which the WorkspacePane branch (and thus DesignView) renders.
  const designFullWindow = showDesign && workspacePaneVisible;

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
            titlebar). The column dividers carry through from the panes below.
            In full-window Design mode the Rail + Roster column bgs collapse to
            0 width (their dividers gone with them), leaving a single seamless
            strip over the canvas + terminal below. */}
        {/* Rail column bg */}
        <div
          className={`${designFullWindow ? "w-0 overflow-hidden" : "w-[56px] border-r border-overlay/[0.06]"} bg-sidebar pointer-events-none`}
        />
        {/* Roster column bg */}
        <div
          className={`${designFullWindow ? "w-0 overflow-hidden" : "w-[266px] border-r border-overlay/[0.06]"} bg-sidebar pointer-events-none`}
        />
        {/* Main content bg */}
        <div className="flex-1 bg-sidebar pointer-events-none" />
      </div>

      {/* ── 3-pane layout ────────────────────────────────────────────── */}
      <div className="flex-1 flex overflow-hidden min-h-0">
        {/* Rail — collapsed to 0 width in full-window Design mode (D3), never
            unmounted: `contents` makes the wrapper transparent to flex so the
            Rail's own w-[56px] applies normally; collapsing swaps to a 0-width
            clip. Keeping it mounted preserves its state and — with Roster below
            — keeps WorkspacePane's position in the tree unchanged, so the
            terminal never remounts. `inert` + `aria-hidden` when collapsed pull
            the clipped-but-mounted nav out of the tab order and the a11y tree —
            CSS clipping hides pixels only, leaving focusables tabbable (Armin
            F1). */}
        <div
          inert={designFullWindow}
          aria-hidden={designFullWindow || undefined}
          className={designFullWindow ? "w-0 shrink-0 overflow-hidden" : "contents"}
        >
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
        </div>

        {/* Roster + main stay mounted; the Library opens as an overlay sheet
            on top so the workspace refreshes live underneath a delete. */}
        <>
            {/* Roster — collapsed to 0 width in full-window Design mode (D3),
                never unmounted (same `contents`/clip pattern as the Rail above).
                It must NOT be conditionally removed: keeping it in the tree holds
                the center-content branch (WorkspacePane) at a stable position, so
                toggling Design never remounts the terminal. `inert` +
                `aria-hidden` when collapsed remove its clipped-but-mounted
                focusables from the tab order + a11y tree (Armin F1). */}
            <div
              inert={designFullWindow}
              aria-hidden={designFullWindow || undefined}
              className={designFullWindow ? "w-0 shrink-0 overflow-hidden" : "contents"}
            >
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
            </div>

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
            ) : workspacePaneVisible ? (
              // Remount per workspace AND per agents change so the pane refetches
              // its tabs when an agent is added/removed. `workspacePaneVisible` is
              // the shared predicate (also gating designFullWindow); reaching this
              // arm with a workspace active already implies no center screen is up,
              // so it is equivalent to the former `activeWorkspaceId` guard.
              <WorkspacePane
                key={`${activeWorkspaceId}:${agentsVersion}`}
                workspaceId={activeWorkspaceId}
                workspaceName={activeWorkspace?.name}
                focusInstanceId={selectedId}
                designOpen={showDesign}
                onCloseDesign={() => setShowDesign(false)}
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
