import { useEffect, useRef, useState } from "react";
import { EVENT_NAMES, ipc } from "../ipc";
import { useEvent, type WorkspaceChangedEvent } from "../ipc/events";
import { setThemePref } from "../lib/theme";
import type { Workspace, AgentDefinition, DraftMode } from "../ipc";
import { Rail } from "./Rail";
import { Roster } from "./Roster";
import { Builder } from "./Builder";
import { Library } from "./Library";
import { AgentDrafter } from "./AgentDrafter";
import { SkillLibrary } from "./SkillLibrary";
import { LinkFolder } from "./LinkFolder";
import { EditWorkspace } from "./EditWorkspace";
import { Settings } from "./Settings";
import { WorkspacePane, type WorkspaceStartBatch } from "./WorkspacePane";
import { Blackboard } from "./Blackboard";
import { ChatHub } from "./ChatHub";
import { MemoryGraph } from "./MemoryGraph";
import { LaneBoard } from "./LaneBoard";
import { InAppBrowserView } from "./InAppBrowserView";

/** Synchronous fixture-mode check (mirrors src/fixtures/mode.ts) — kept inline
 *  so prod builds never statically import the fixture module. The
 *  `import.meta.env.DEV` short-circuit makes the whole expression dead-code-
 *  eliminate in a production build. */
function fixtureActive(): boolean {
  return (
    import.meta.env.DEV &&
    !!new URLSearchParams(window.location.search).get("fixture")
  );
}

export function AppShell() {
  // Roster selection — propagated to WorkspacePane.focusInstanceId to switch
  // the active agent tab when the user clicks an agent in the Roster sidebar.
  const [selectedId, setSelectedId] = useState<string | null>(null);

  // ── Workspace state ────────────────────────────────────────────────────
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [activeWorkspaceId, setActiveWorkspaceId] = useState<string | null>(null);
  const activeWorkspaceIdRef = useRef<string | null>(null);
  const [workspaceLifecyclePhase, setWorkspaceLifecyclePhase] = useState<
    "idle" | "starting" | "stopping"
  >("idle");
  const workspaceLifecyclePhaseRef = useRef<"idle" | "starting" | "stopping">("idle");
  const workspaceLifecycleGenerationRef = useRef(0);
  const [workspaceLifecycleError, setWorkspaceLifecycleError] = useState<string | null>(null);
  const [workspaceStartBatch, setWorkspaceStartBatch] = useState<WorkspaceStartBatch | null>(
    null,
  );
  const [lifecycleAnnouncement, setLifecycleAnnouncement] = useState("");
  const [focusWorkspaceStart, setFocusWorkspaceStart] = useState(false);

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

  // ── Browser state — an in-app browser control surface (runtime::browser).
  //    A center-pane destination, mutually exclusive with the other full-page
  //    workspace views (same toggle pattern as Blackboard/Memory/LaneBoard). ──
  const [showBrowser, setShowBrowser] = useState(false);

  // Whether an agent-driven browser is currently open — polled so the Rail can
  // show a dot even while the human is on another tab.
  const [browserActive, setBrowserActive] = useState(false);

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

  // ── AI drafter state ───────────────────────────────────────────────────
  /** Open overlay + which mode it runs in; null = closed. */
  const [showDrafter, setShowDrafter] = useState<{ mode: DraftMode } | null>(null);
  /** Bumped per accepted draft so the Builder remounts for each one — an
   *  id-less draft has no id to key on. */
  const [draftSeq, setDraftSeq] = useState(0);
  /** Drafter name for the Builder's "Drafted by" chip. */
  const [builderDraftedBy, setBuilderDraftedBy] = useState<string | undefined>(undefined);

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

  // ── Fixture-mode boot flag (DEV-only) — true once the initial workspace
  //    fetch has settled, gating the readiness sentinel below. ────────────────
  const [booted, setBooted] = useState(false);
  // Fixture mode (DEV-only): `#view=builder-edit` waits for the roster.
  const [pendingBuilderEdit, setPendingBuilderEdit] = useState(false);

  useEffect(() => {
    activeWorkspaceIdRef.current = activeWorkspaceId;
  }, [activeWorkspaceId]);

  // Load workspaces from the DB on mount.
  // Falls back to an empty list if Tauri is not present (plain Vite dev).
  useEffect(() => {
    ipc.workspace
      .list()
      .then((ws) => {
        setWorkspaces(ws);
        // Fixture mode (DEV-only): auto-select the first workspace so the routed
        // view renders with data — a headless shot has no human to click a
        // workspace. Set directly (not via handleSelectWorkspace) so the
        // hash-routed center-screen flags are NOT cleared.
        if (fixtureActive() && ws.length > 0) {
          setActiveWorkspaceId(ws[0].id);
        }
        setBooted(true);
      })
      .catch((err: unknown) => {
        // Plain `vite` dev (no Tauri shell) lands here; so does a real backend
        // failure. Surface it in dev rather than silently showing an empty Rail.
        if (import.meta.env.DEV) {
          console.error("AppShell: workspace.list failed", err);
        }
        setWorkspaces([]);
        setBooted(true);
      });
  }, []);

  // Workspace lifecycle events can originate from a CLI or another window. The
  // start command emits its own `started` event before its agent batch finishes,
  // so ignore the active workspace's event while our own transition is in flight;
  // the command result below carries the authoritative batch outcome.
  useEvent<WorkspaceChangedEvent>(EVENT_NAMES.workspaceChanged, (payload) => {
    if (
      payload.workspaceId === activeWorkspaceId &&
      workspaceLifecyclePhaseRef.current !== "idle"
    ) {
      return;
    }
    setWorkspaces((prev) =>
      prev.map((workspace) =>
        workspace.id === payload.workspaceId
          ? { ...workspace, runState: payload.runState }
          : workspace,
      ),
    );
    if (payload.workspaceId === activeWorkspaceId) {
      setFocusWorkspaceStart(false);
      setWorkspaceLifecycleError(null);
      setWorkspaceStartBatch(null);
      setAgentsVersion((version) => version + 1);
      setLifecycleAnnouncement(
        payload.runState === "started" ? "Workspace started." : "Workspace stopped.",
      );
    }
  });

  // Fixture mode (DEV-only): route the initial view from the URL hash so a
  // headless capture (scripts/uishot.mjs) can open any screen directly via
  // `#view=<id>`. Set directly (not via handleSelectWorkspace) so the boot
  // effect's workspace auto-select doesn't clobber it. No-op outside ?fixture=.
  useEffect(() => {
    if (!fixtureActive()) return;
    const view = /view=([a-z-]+)/.exec(window.location.hash)?.[1] ?? "home";
    const open: Record<string, () => void> = {
      home: () => {},
      laneboard: () => setShowLaneBoard(true),
      memory: () => setShowMemory(true),
      artifacts: () => setShowArtifacts(true),
      blackboard: () => setShowBlackboard(true),
      chat: () => setShowChat(true),
      library: () => setShowLibrary(true),
      builder: () => setShowBuilder(true),
      // Edit mode with Position (spec D11): the Builder needs a definition AND
      // the workspace agent that instantiates it, neither of which exists yet
      // at boot — so this only arms the flag and the effect below opens it.
      "builder-edit": () => setPendingBuilderEdit(true),
      drafter: () => setShowDrafter({ mode: "team" }),
      settings: () => setShowSettings(true),
      browser: () => setShowBrowser(true),
    };
    (open[view] ?? open.home)();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Fixture mode (DEV-only): open the Builder in EDIT mode on the first
  // definition that has a live workspace agent, so `pnpm uishot builder-edit`
  // renders the Position section without any browser interaction (spec D11).
  useEffect(() => {
    if (!pendingBuilderEdit || !activeWorkspaceId) return;
    let active = true;
    void Promise.all([ipc.agentDef.list(), ipc.instance.list({ workspaceId: activeWorkspaceId })])
      .then(([defs, instances]) => {
        if (!active) return;
        // positionEnabled needs the pair to agree, so pick the instance first.
        const instance = instances.find((a) => defs.some((d) => d.id === a.agentDefId));
        const def = defs.find((d) => d.id === instance?.agentDefId);
        if (!instance || !def) return;
        setBuilderInitialDef(def);
        setSelectedId(instance.id);
        setShowBuilder(true);
      })
      .catch((err: unknown) => {
        if (import.meta.env.DEV) console.error("AppShell: builder-edit view failed", err);
      })
      .finally(() => {
        if (active) setPendingBuilderEdit(false);
      });
    return () => {
      active = false;
    };
  }, [activeWorkspaceId, pendingBuilderEdit]);

  // Fixture mode (DEV-only): set the readiness sentinel once boot data has
  // landed and the routed view has had its first real paint, so uishot knows
  // when to shoot. Double-rAF defers past the paint. No-op outside ?fixture=.
  useEffect(() => {
    if (!booted || pendingBuilderEdit || !fixtureActive()) return;
    let raf2 = 0;
    const raf1 = requestAnimationFrame(() => {
      raf2 = requestAnimationFrame(() => {
        document.body.dataset.conclaveReady = "1";
      });
    });
    return () => {
      cancelAnimationFrame(raf1);
      if (raf2) cancelAnimationFrame(raf2);
    };
  }, [booted, pendingBuilderEdit]);

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
        if (activeWorkspaceId) {
          setShowBrowser(false);
          setShowBlackboard((v) => !v);
        }
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
    if (!activeWorkspaceId) {
      setBrowserActive(false);
      return;
    }
    let alive = true;
    const check = () => {
      ipc.browser
        .status()
        .then((st) => {
          if (alive) setBrowserActive(st.tabs.length > 0);
        })
        .catch(() => {});
    };
    check();
    const id = window.setInterval(check, 4000);
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, [activeWorkspaceId]);

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
        // ⌘⇧A mirrors ⌘D now that Artifacts shares the canvas slot with Design
        // (plan D3/D4): toggle it, clear the OTHER slot flag, and leave the
        // center-screen flags alone so Artifacts stays latent behind them.
        e.preventDefault();
        setShowDesign(false);
        setShowArtifacts((v) => !v);
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [activeWorkspaceId]);

  function handleSelectWorkspace(id: string) {
    // Optimistically update selection; handler only validates on the Rust side.
    setActiveWorkspaceId(id);
    activeWorkspaceIdRef.current = id;
    workspaceLifecycleGenerationRef.current += 1;
    // Clear the previous workspace's agent selection so a stale instance id
    // isn't carried into the remounted pane as focus.
    setSelectedId(null);
    workspaceLifecyclePhaseRef.current = "idle";
    setWorkspaceLifecyclePhase("idle");
    setWorkspaceLifecycleError(null);
    setWorkspaceStartBatch(null);
    setFocusWorkspaceStart(false);
    // Switching workspace returns to that workspace's agent pane.
    setShowBlackboard(false);
    setShowChat(false);
    setShowMemory(false);
    setShowLaneBoard(false);
    setShowBrowser(false);
    setShowArtifacts(false);
    setShowDesign(false);
    ipc.workspace.use({ workspaceId: id }).catch(() => {
      // Ignore — the workspace just can't be found in the DB (stale id).
    });
  }

  const activeWorkspace = activeWorkspaceId
    ? (workspaces.find((w) => w.id === activeWorkspaceId) ?? null)
    : null;

  async function handleStartWorkspace() {
    const workspaceId = activeWorkspaceId;
    if (!workspaceId || workspaceLifecyclePhaseRef.current !== "idle") return;

    workspaceLifecyclePhaseRef.current = "starting";
    const generation = ++workspaceLifecycleGenerationRef.current;
    setWorkspaceLifecyclePhase("starting");
    setWorkspaceLifecycleError(null);
    setWorkspaceStartBatch(null);
    setFocusWorkspaceStart(false);
    try {
      const result = await ipc.workspace.start({ workspaceId });
      setWorkspaces((prev) =>
        prev.map((workspace) => (workspace.id === workspaceId ? result.workspace : workspace)),
      );
      if (
        activeWorkspaceIdRef.current === workspaceId &&
        workspaceLifecycleGenerationRef.current === generation
      ) {
        setWorkspaceStartBatch({
          readyAgentIds: result.readyAgentIds,
          skippedStoppedAgentIds: result.skippedStoppedAgentIds,
          failures: result.failures,
        });
        setAgentsVersion((version) => version + 1);
        setLifecycleAnnouncement(
          result.failures.length === 0
            ? "Workspace started."
            : `Workspace started with ${result.failures.length} agent ${result.failures.length === 1 ? "failure" : "failures"}.`,
        );
      }
    } catch (error) {
      if (
        activeWorkspaceIdRef.current === workspaceId &&
        workspaceLifecycleGenerationRef.current === generation
      ) {
        const detail = error instanceof Error ? error.message : String(error);
        setWorkspaceLifecycleError(`Couldn’t start workspace: ${detail}`);
      }
    } finally {
      if (
        workspaceLifecycleGenerationRef.current === generation &&
        workspaceLifecyclePhaseRef.current === "starting"
      ) {
        workspaceLifecyclePhaseRef.current = "idle";
        setWorkspaceLifecyclePhase("idle");
      }
    }
  }

  async function handleStopWorkspace() {
    const workspaceId = activeWorkspaceId;
    if (!workspaceId || workspaceLifecyclePhaseRef.current !== "idle") return;

    workspaceLifecyclePhaseRef.current = "stopping";
    const generation = ++workspaceLifecycleGenerationRef.current;
    setWorkspaceLifecyclePhase("stopping");
    setWorkspaceLifecycleError(null);
    setFocusWorkspaceStart(false);
    try {
      const result = await ipc.workspace.stop({ workspaceId });
      setWorkspaces((prev) =>
        prev.map((workspace) => (workspace.id === workspaceId ? result.workspace : workspace)),
      );
      if (
        activeWorkspaceIdRef.current === workspaceId &&
        workspaceLifecycleGenerationRef.current === generation
      ) {
        setWorkspaceStartBatch(null);
        setFocusWorkspaceStart(true);
        setAgentsVersion((version) => version + 1);
        setLifecycleAnnouncement("Workspace stopped.");
      }
    } catch (error) {
      if (
        activeWorkspaceIdRef.current === workspaceId &&
        workspaceLifecycleGenerationRef.current === generation
      ) {
        const detail = error instanceof Error ? error.message : String(error);
        setWorkspaceLifecycleError(
          `Couldn’t stop workspace: ${detail}. One or more agents may still be running.`,
        );
      }
    } finally {
      if (
        workspaceLifecycleGenerationRef.current === generation &&
        workspaceLifecyclePhaseRef.current === "stopping"
      ) {
        workspaceLifecyclePhaseRef.current = "idle";
        setWorkspaceLifecyclePhase("idle");
      }
    }
  }

  function handleAgentLifecycleChanged(
    instanceId: string,
    availability: "active" | "stopped",
  ) {
    setWorkspaceStartBatch((batch) => {
      if (!batch) return batch;
      return {
        readyAgentIds:
          availability === "active"
            ? Array.from(new Set([...batch.readyAgentIds, instanceId]))
            : batch.readyAgentIds.filter((id) => id !== instanceId),
        skippedStoppedAgentIds:
          availability === "stopped"
            ? Array.from(new Set([...batch.skippedStoppedAgentIds, instanceId]))
            : batch.skippedStoppedAgentIds.filter((id) => id !== instanceId),
        failures: batch.failures.filter(
          (failure) => failure.workspaceAgentId !== instanceId,
        ),
      };
    });
    setAgentsVersion((version) => version + 1);
    setLifecycleAnnouncement(
      availability === "active" ? "Agent resumed." : "Agent stopped.",
    );
  }

  // Center-pane destinations that REPLACE the live WorkspacePane (each renders
  // full-page instead of it). This is the ONE canonical list — adding a new
  // center screen means adding its flag HERE, and both `workspacePaneVisible`
  // (which drives the WorkspacePane render branch below) and `slotFullWindow`
  // update together, so the two can't silently diverge (Armin rot-guard).
  // NOTE: showArtifacts is NOT here — Artifacts moved into the canvas slot
  // (like showDesign), so it renders INSIDE the WorkspacePane, not instead of
  // it (plan D3).
  const centerScreenOpen =
    showChat || showBlackboard || showMemory || showLaneBoard || showBrowser;

  // The live WorkspacePane (agent pane + the always-mounted Design slot) is the
  // visible center content exactly when a workspace is active and no center
  // screen is up. Used BOTH as the WorkspacePane render condition and as the
  // gate for full-window Design mode.
  const workspacePaneVisible = !!activeWorkspaceId && !centerScreenOpen;

  // Full-window slot mode (human ruling D3): while a canvas-slot view (Design OR
  // Artifacts) is OPEN and actually on screen, hide the Rail + Roster columns so
  // the window becomes canvas-left + agent-terminal-right. The slot flag alone
  // is not enough — each is latent and stays true behind a center-pane screen,
  // where the slot content is NOT rendered (it lives inside the WorkspacePane
  // branch); hiding the sidebars then would strand the user in a full-screen
  // center view with no navigation. So gate on `workspacePaneVisible` — the
  // exact condition under which the WorkspacePane branch (and thus the slot)
  // renders.
  const slotFullWindow = (showDesign || showArtifacts) && workspacePaneVisible;

  // Full-window Browser (human request 2026-07-11): the in-app Browser is a
  // center screen (it REPLACES the WorkspacePane, so workspacePaneVisible is
  // false while it's up — it can't ride slotFullWindow). Unlike the other
  // center screens it carries its OWN nav rail with a "Close Browser" button,
  // so collapsing the app Rail + Roster leaves the user a way back and never
  // strands them. Gate on an active workspace to mirror the other slot flags.
  //
  // INVARIANT (Mellow, review of 620482b): this does NOT also require the other
  // center-screen flags to be false, unlike slotFullWindow (which gates on
  // workspacePaneVisible). Safe today because every setter of showChat/
  // showBlackboard/showMemory/showLaneBoard is reached from the Roster (inert
  // while collapsed) or a menu path that clears showBrowser first — so browser
  // can never be latent behind a peer center screen. If you add a NEW entry
  // point to those four flags that does NOT clear showBrowser, add
  // `&& !centerScreenOpen`-style exclusivity here (browser is last in the
  // render chain, so a peer flag would win the branch while this stays true).
  const browserFullWindow = showBrowser && !!activeWorkspaceId;

  // Sidebars (Rail + Roster) collapse to 0 width for canvas-slot full-window
  // (Design / Artifacts) OR full-window Browser — the single predicate every
  // collapse site below reads, so the two modes can't drift apart.
  const sidebarsCollapsed = slotFullWindow || browserFullWindow;

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
          className={`${sidebarsCollapsed ? "w-0 overflow-hidden" : "w-[56px] border-r border-overlay/[0.06]"} bg-sidebar pointer-events-none`}
        />
        {/* Roster column bg */}
        <div
          className={`${sidebarsCollapsed ? "w-0 overflow-hidden" : "w-[266px] border-r border-overlay/[0.06]"} bg-sidebar pointer-events-none`}
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
          inert={sidebarsCollapsed}
          aria-hidden={sidebarsCollapsed || undefined}
          className={sidebarsCollapsed ? "w-0 shrink-0 overflow-hidden" : "contents"}
        >
          <Rail
            workspaces={workspaces}
            activeWorkspaceId={activeWorkspaceId}
            artifactsOpen={showArtifacts}
            designOpen={showDesign}
            browserOpen={showBrowser}
            browserActive={browserActive}
            onSelectWorkspace={handleSelectWorkspace}
            onOpenBrowser={() => {
              if (!activeWorkspaceId) return;
              // Browser is a center screen — clear the other center screens so
              // it actually shows (they precede it in the render order).
              setShowChat(false);
              setShowBlackboard(false);
              setShowMemory(false);
              setShowLaneBoard(false);
              setShowBrowser((v) => !v);
            }}
            onOpenDesign={() => {
              if (!activeWorkspaceId) return;
              setShowArtifacts(false);
              setShowDesign((v) => !v);
            }}
            onOpenArtifacts={() => {
              // Mirror of onOpenDesign: Artifacts shares the canvas slot, so
              // toggle it and clear the OTHER slot flag only (D3/D4).
              if (!activeWorkspaceId) return;
              setShowDesign(false);
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
              inert={sidebarsCollapsed}
              aria-hidden={sidebarsCollapsed || undefined}
              className={sidebarsCollapsed ? "w-0 shrink-0 overflow-hidden" : "contents"}
            >
            <Roster
              workspaceId={activeWorkspaceId}
              workspaceName={activeWorkspace?.name}
              folderPath={activeWorkspace?.folderPath}
              workspaceRunState={activeWorkspace?.runState}
              workspaceLifecyclePhase={workspaceLifecyclePhase}
              workspaceLifecycleError={workspaceLifecycleError}
              onStartWorkspace={() => void handleStartWorkspace()}
              onStopWorkspace={() => void handleStopWorkspace()}
              onAgentLifecycleChanged={handleAgentLifecycleChanged}
              selectedId={selectedId}
              onSelect={(id) => {
                // Selecting an agent returns from any center-pane screen to the
                // pane. Does NOT clear showArtifacts (nor showDesign): both are
                // canvas-slot flags now, so a click just returns to whichever
                // slot view was latent — mirroring design's latency (D4).
                setShowBlackboard(false);
                setShowChat(false);
                setShowMemory(false);
                setShowLaneBoard(false);
                setSelectedId(id);
              }}
              // "Create new agent…" (from inside the picker) still opens the Builder.
              onCreateAgent={() => {
                setBuilderInitialDef(undefined);
                setShowBuilder(true);
              }}
              onBuildTeam={() => setShowDrafter({ mode: "team" })}
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
                      // Do NOT clear showArtifacts: it's a canvas-slot flag now
                      // (like showDesign), latent behind center screens (D4).
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
            ) : showBrowser && activeWorkspaceId ? (
              <InAppBrowserView
                key={activeWorkspaceId}
                workspaceId={activeWorkspaceId}
                workspaceName={activeWorkspace?.name}
                onClose={() => setShowBrowser(false)}
              />
            ) : workspacePaneVisible ? (
              // Remount per workspace AND per agents change so the pane refetches
              // its tabs when an agent is added/removed. `workspacePaneVisible` is
              // the shared predicate (also gating slotFullWindow); reaching this
              // arm with a workspace active already implies no center screen is up,
              // so it is equivalent to the former `activeWorkspaceId` guard.
              <WorkspacePane
                key={`${activeWorkspaceId}:${agentsVersion}`}
                workspaceId={activeWorkspaceId}
                workspaceName={activeWorkspace?.name}
                workspaceRunState={activeWorkspace?.runState ?? "stopped"}
                workspaceLifecyclePhase={workspaceLifecyclePhase}
                workspaceLifecycleError={workspaceLifecycleError}
                workspaceStartBatch={workspaceStartBatch}
                focusStartAction={focusWorkspaceStart}
                onStartWorkspace={() => void handleStartWorkspace()}
                onAgentLifecycleChanged={handleAgentLifecycleChanged}
                onStartRetryComplete={(readyAgentIds, failures) => {
                  setWorkspaceStartBatch((batch) =>
                    batch
                      ? {
                          ...batch,
                          readyAgentIds: Array.from(
                            new Set([...batch.readyAgentIds, ...readyAgentIds]),
                          ),
                          failures,
                        }
                      : batch,
                  );
                }}
                focusInstanceId={selectedId}
                onActiveInstanceChange={(id) => setSelectedId(id)}
                designOpen={showDesign}
                onCloseDesign={() => setShowDesign(false)}
                artifactsOpen={showArtifacts}
                onCloseArtifacts={() => setShowArtifacts(false)}
                onOpenChat={() => {
                  // Do NOT clear showArtifacts (nor showDesign): both are canvas-
                  // slot flags, latent behind the ChatHub center screen — same
                  // latency as Design (Mellow F1, ruled; plan D4 guard case).
                  setShowBlackboard(false);
                  setShowMemory(false);
                  setShowLaneBoard(false);
                  setShowBrowser(false);
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
          onOpenDrafter={() => setShowDrafter({ mode: "agent" })}
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
          key={builderInitialDef?.id || `draft-${draftSeq}`}
          initialDef={builderInitialDef}
          draftedBy={builderDraftedBy}
          workspaceId={activeWorkspaceId ?? undefined}
          workspaceAgentId={selectedId ?? undefined}
          onClose={() => {
            setShowBuilder(false);
            setBuilderInitialDef(undefined);
            setBuilderDraftedBy(undefined);
          }}
          onSaved={() => {
            setShowBuilder(false);
            setBuilderInitialDef(undefined);
            setBuilderDraftedBy(undefined);
            // Bump key so Library re-fetches agentDef.list after a save/edit.
            setLibraryRefreshKey((k) => k + 1);
          }}
        />
      )}

      {/* ── AI drafter overlay ───────────────────────────────────────── */}
      {showDrafter && (
        <AgentDrafter
          mode={showDrafter.mode}
          workspaceId={activeWorkspaceId ?? undefined}
          workspaceName={activeWorkspace?.name}
          onClose={() => setShowDrafter(null)}
          onDraftAgent={(def, by) => {
            setBuilderInitialDef(def);
            setBuilderDraftedBy(by);
            setDraftSeq((n) => n + 1);
            setShowDrafter(null);
            setShowBuilder(true);
          }}
          onApplyFinished={(created) => {
            setShowDrafter(null);
            // A partial apply still created definitions, so refresh either way.
            if (created > 0) {
              setLibraryRefreshKey((k) => k + 1);
              setAgentsVersion((v) => v + 1);
            }
          }}
          onOpenBuilder={() => {
            setShowDrafter(null);
            setBuilderInitialDef(undefined);
            setBuilderDraftedBy(undefined);
            setShowBuilder(true);
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
            activeWorkspaceIdRef.current = null;
            workspaceLifecycleGenerationRef.current += 1;
            setSelectedId(null);
            setShowBlackboard(false);
            setShowChat(false);
            setShowMemory(false);
            setShowLaneBoard(false);
            setShowBrowser(false);
            setShowArtifacts(false);
            setShowDesign(false);
          }}
        />
      )}
      <div className="sr-only" aria-live="polite">
        {lifecycleAnnouncement}
      </div>
    </div>
  );
}
