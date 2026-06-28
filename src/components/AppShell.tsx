import { useEffect, useState } from "react";
import { ipc } from "../ipc";
import type { Workspace, AgentDefinition } from "../ipc";
import { Rail } from "./Rail";
import { Roster } from "./Roster";
import { Builder } from "./Builder";
import { Library } from "./Library";
import { LinkFolder } from "./LinkFolder";
import { Settings } from "./Settings";
import { WorkspacePane } from "./WorkspacePane";
import { Blackboard } from "./Blackboard";

export function AppShell() {
  // Roster selection — propagated to WorkspacePane.focusInstanceId to switch
  // the active agent tab when the user clicks an agent in the Roster sidebar.
  const [selectedId, setSelectedId] = useState<string | null>(null);

  // ── Workspace state ────────────────────────────────────────────────────
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [activeWorkspaceId, setActiveWorkspaceId] = useState<string | null>(null);

  // ── Blackboard state ───────────────────────────────────────────────────
  const [showBlackboard, setShowBlackboard] = useState(false);

  // ── Library state ──────────────────────────────────────────────────────
  const [showLibrary, setShowLibrary] = useState(false);
  /** Incremented after Builder saves so Library re-fetches agentDef.list. */
  const [libraryRefreshKey, setLibraryRefreshKey] = useState(0);

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

  function handleSelectWorkspace(id: string) {
    // Optimistically update selection; handler only validates on the Rust side.
    setActiveWorkspaceId(id);
    // Clear the previous workspace's agent selection so a stale instance id
    // isn't carried into the remounted pane as focus.
    setSelectedId(null);
    // Switching workspace returns to that workspace's agent pane.
    setShowBlackboard(false);
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
        <div className="w-[56px] bg-[#f5f5f7] border-r border-black/[0.06] pointer-events-none" />
        {/* Roster column bg */}
        <div className="w-[266px] bg-[#f5f5f7] border-r border-black/[0.06] pointer-events-none" />
        {/* Main content bg */}
        <div className="flex-1 bg-[#f5f5f7] pointer-events-none" />
      </div>

      {/* ── 3-pane layout ────────────────────────────────────────────── */}
      <div className="flex-1 flex overflow-hidden min-h-0">
        <Rail
          workspaces={workspaces}
          activeWorkspaceId={activeWorkspaceId}
          onSelectWorkspace={handleSelectWorkspace}
          onOpenLibrary={() => {
            setShowBlackboard(false);
            setShowLibrary(true);
          }}
          onOpenLinkFolder={() => setShowLinkFolder(true)}
          onOpenSettings={() => setShowSettings(true)}
        />

        {showLibrary ? (
          /* ── Library view: replaces Roster + main while open ─── */
          <Library
            onClose={() => setShowLibrary(false)}
            onOpenBuilder={(def) => {
              setBuilderInitialDef(def);
              setShowBuilder(true);
            }}
            refreshKey={libraryRefreshKey}
          />
        ) : (
          <>
            <Roster
              workspaceId={activeWorkspaceId}
              workspaceName={activeWorkspace?.name}
              folderPath={activeWorkspace?.folderPath}
              selectedId={selectedId}
              onSelect={(id) => {
                // Selecting an agent returns from the Blackboard to the pane.
                setShowBlackboard(false);
                setSelectedId(id);
              }}
              onAddAgent={() => {
                setBuilderInitialDef(undefined);
                setShowBuilder(true);
              }}
              // Blackboard needs a workspace to scope to — only toggle when one
              // is active (else the view would fall through to "Select a workspace").
              onOpenBlackboard={
                activeWorkspaceId ? () => setShowBlackboard((v) => !v) : undefined
              }
              blackboardOpen={showBlackboard}
            />

            {/* ── Main content: Blackboard screen, else the live agent pane ─── */}
            {showBlackboard && activeWorkspaceId ? (
              <Blackboard
                key={activeWorkspaceId}
                workspaceId={activeWorkspaceId}
                workspaceName={activeWorkspace?.name}
                onClose={() => setShowBlackboard(false)}
              />
            ) : activeWorkspaceId ? (
              // Remount per workspace so the pane refetches its instances.
              <WorkspacePane key={activeWorkspaceId} workspaceId={activeWorkspaceId} focusInstanceId={selectedId} />
            ) : (
              <main className="flex-1 flex flex-col min-w-0 bg-white">
                <div className="flex-1 grid place-items-center text-[13px] text-[#a1a1a6]">
                  Select a workspace to start
                </div>
              </main>
            )}
          </>
        )}
      </div>

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
    </div>
  );
}
