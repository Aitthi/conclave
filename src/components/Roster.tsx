import { useCallback, useEffect, useRef, useState } from "react";
import {
  Waypoints,
  Terminal,
  Search,
  Folder,
  Plus,
  Layers,
  Network,
  Columns3,
  MessageSquare,
  X,
  Pencil,
  LoaderCircle,
} from "lucide-react";
import { ipc, useEvent, EVENT_NAMES } from "../ipc";
import type {
  AgentDefinition,
  WorkspaceAgent,
  SessionStatusEvent,
  SessionOutputEvent,
} from "../ipc";
import type { RosterChangedEvent } from "../ipc/events";
import { computeSkillsStale } from "../lib/skills";
import { type PositionPerson, PositionLine, ReportsTo } from "./Position";
import { SupervisorPicker, type SupervisorCandidate } from "./SupervisorPicker";
import { descendantsOf } from "../lib/positions";

// A live instance reads as "working" while its backend emitted output within
// this window (R-act-1) — mirrors commands::instance::WORKING_WINDOW (Rust).
// Keep both in sync if the window ever changes.
const WORKING_WINDOW_MS = 5000;

// ---------------------------------------------------------------------------
// View model for one agent row — derived from ipc.instance.list ⨝ ipc.agentDef.list.
// NO hardcoded data; the source is always the real DB join.
// ---------------------------------------------------------------------------

interface RosterEntry {
  instanceId: string;
  name: string;
  color: string;
  type: AgentDefinition["type"];
  status: WorkspaceAgent["status"];
  /** Subtitle derived honestly from the def / enriched roster — no fabricated
   *  strings. Prefers the first-class role name (ADR 0005) over the legacy
   *  free-text label. */
  meta: string;
  /** The role's one-paragraph job description (first-class roles only), shown
   *  on hover of the subtitle. */
  roleDescription?: string;
  /** True when the def's CURRENT full skill id set (builtin + custom) differs
   *  from what the live session actually launched with — see
   *  Session.launchedSkillIds. Only meaningful for `type === "cli"` (the
   *  only type skills apply to in v1). */
  skillsStale: boolean;
  /** R-act-1: whether the live backend emitted output within `WORKING_WINDOW_MS`.
   *  Always `false` for a non-live (idle) instance — `instance.list` only sets
   *  `working` for live rows, so this seeds `false` and the roster never shows
   *  the working animation for an idle agent. */
  working: boolean;
  /** The live session id — lets the `session:output` subscription below map an
   *  event (which carries only `sessionId`) back to this row. `undefined` for
   *  a non-live instance. */
  sessionId?: string;
  level?: string;
  roleName?: string;
  supervisor?: PositionPerson | null;
  /** Raw id (as opposed to `supervisor`'s resolved display identity) — needed
   *  to seed SupervisorPicker's `current` and to walk the chain client-side
   *  (descendantsOf) when the chip opens the edit-variant picker. */
  supervisorAgentId?: string;
}

// Status dot colors mapped from WorkspaceAgent.status (mirrors WorkspacePane).
const STATUS_COLOR: Record<WorkspaceAgent["status"], string> = {
  running: "#30d158",
  waiting: "#ff9f0a",
  idle: "#c7c7cc",
};

// Derive a subtitle from the definition — never fabricated.
function deriveMeta(def: AgentDefinition): string {
  switch (def.type) {
    case "orchestrator":
      return "Orchestrator";
    case "cli":
      return def.role ?? def.cliKind ?? "CLI";
    case "chat":
      return def.role ?? def.model ?? "Chat";
  }
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

interface AgentAvatarProps {
  entry: RosterEntry;
  size?: "sm" | "md";
}

function AgentAvatar({ entry, size = "md" }: AgentAvatarProps) {
  const dim =
    size === "sm" ? "w-6 h-6 rounded-[7px] text-[11px]" : "w-7 h-7 rounded-[8px] text-[12px]";
  const color = entry.color;

  if (entry.type === "orchestrator") {
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
      {entry.name[0]}
    </div>
  );
}

// Amber "working" sub-line (R-act-1, plan:working-indicator-restyle, round 2 —
// supersedes the F2 green-halo dot). Rendered for every row; the grid-rows
// slot only opens while `working` is true, so idle rows stay compact and the
// slide-in is animated rather than an instant layout jump.
function WorkLine({ working }: { working: boolean }) {
  return (
    <div className={`roster-working-slot${working ? " is-working" : ""}`} aria-hidden={!working}>
      <div className="roster-working">
        <div className="roster-working-content">
          <LoaderCircle className="w-[11px] h-[11px] roster-working-icon" />
          <span className="roster-working-label">working</span>
        </div>
      </div>
    </div>
  );
}

interface AgentRowProps {
  entry: RosterEntry;
  isSelected: boolean;
  onSelect: () => void;
  onRemove: () => void;
  removing: boolean;
  onEditSupervisor: () => void;
}

function AgentRow({
  entry,
  isSelected,
  onSelect,
  onRemove,
  removing,
  onEditSupervisor,
}: AgentRowProps) {
  const isCli = entry.type === "cli";
  const statusColor = STATUS_COLOR[entry.status];
  // Two-step removal so a stray click can't delete an agent: the first click
  // arms the confirm, the second (red) click commits.
  const [confirming, setConfirming] = useState(false);

  return (
    // Not a <button> (it now nests buttons) — a div with a role for the a11y tree.
    <div
      className={`group w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg transition-colors cursor-pointer${
        isSelected ? " bg-accent/10 ring-1 ring-accent/30" : " hover:bg-overlay/[0.04]"
      }`}
      role="button"
      tabIndex={0}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
      onMouseLeave={() => setConfirming(false)}
    >
      <AgentAvatar entry={entry} />

      <div className="flex-1 text-left leading-tight min-w-0">
        <div className="text-[12.5px] font-semibold flex items-center gap-1.5 truncate">
          {entry.name}
          {isCli && <Terminal className="w-3 h-3 text-text-muted shrink-0" />}
        </div>
        <div className="flex items-center min-w-0 mt-0.5">
          <PositionLine
            levelId={entry.level}
            track={entry.meta}
            compact
            trackTitle={entry.roleDescription}
            className="flex-1 min-w-0"
          />
          {/* Reports-to chip as a button (plan supervisor-picker-ui, Lane C
              step 5) — stopPropagation so it doesn't also select/deselect the
              row; ReportsTo itself is Position.tsx's unmodified export, just
              wrapped here for interactivity. */}
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              onEditSupervisor();
            }}
            // The row's onKeyDown listens for Enter/Space directly (not via
            // onClick), so it fires from the raw keydown bubbling BEFORE the
            // button's own Enter/Space→click synthesis — stopping only
            // onClick's propagation isn't enough (Armin, review).
            onKeyDown={(e) => e.stopPropagation()}
            onKeyUp={(e) => e.stopPropagation()}
            className="ml-auto shrink-0 pl-1 rounded-md hover:bg-overlay/[0.06] focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-accent/50"
            aria-label={`Change supervisor for ${entry.name}`}
          >
            <ReportsTo supervisor={entry.supervisor} />
          </button>
        </div>
        <WorkLine working={entry.working} />
      </div>

      {confirming ? (
        // Confirm step — commits the removal.
        <button
          onClick={(e) => {
            e.stopPropagation();
            onRemove();
          }}
          disabled={removing}
          className="text-[10.5px] font-semibold text-white bg-danger px-2 py-0.5 rounded-md shrink-0 disabled:opacity-50 self-start"
          title="Confirm removal from this workspace"
        >
          {removing ? "Removing…" : "Remove"}
        </button>
      ) : (
        <>
          {/* Status dot — unchanged by working (R-act-1 now reads via the amber
              WorkLine sub-line above, not this dot); still hidden on hover to
              make room for the remove affordance. `self-start` keeps it pinned
              to the name line instead of stretching to the row's full height
              when the WorkLine slot opens. */}
          <span
            className="w-2 h-2 rounded-full shrink-0 group-hover:hidden self-start mt-0.5"
            style={{ backgroundColor: statusColor }}
            role="img"
            aria-label={entry.status}
          />
          {entry.skillsStale && (
            <span
              className="text-[9px] font-semibold text-warning bg-warning/[0.1] px-1.5 py-px rounded-md shrink-0 self-start"
              title="This agent's skills changed since it last launched — restart to apply"
            >
              Restart to apply
            </span>
          )}
          <button
            onClick={(e) => {
              e.stopPropagation();
              setConfirming(true);
            }}
            className="hidden group-hover:grid w-5 h-5 place-items-center rounded-md text-text-muted hover:bg-overlay/[0.06] hover:text-danger shrink-0 self-start"
            title="Remove from workspace"
            aria-label={`Remove ${entry.name} from workspace`}
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Roster
// ---------------------------------------------------------------------------

interface RosterProps {
  workspaceId: string | null;
  workspaceName?: string;
  folderPath?: string;
  selectedId: string | null;
  onSelect: (instanceId: string) => void;
  /** Open the Builder to define a brand-new agent (from inside the picker). */
  onCreateAgent?: () => void;
  /** Bumped externally whenever the workspace's agent set changes (re-fetch). */
  agentsVersion?: number;
  /** Notify the parent that this workspace's agent set changed (add / remove). */
  onAgentsChanged?: () => void;
  /** Open the per-workspace Blackboard screen. Absent → no workspace active. */
  onOpenBlackboard?: () => void;
  /** Whether the Blackboard screen is currently shown (drives active styling). */
  blackboardOpen?: boolean;
  /** Open the Memory knowledge-graph screen. Absent → no workspace active. */
  onOpenMemory?: () => void;
  /** Whether the Memory screen is currently shown (drives active styling). */
  memoryOpen?: boolean;
  /** Open/toggle the workspace Chat Hub (undefined → disabled). */
  onOpenChat?: () => void;
  /** Whether the Chat Hub is currently open (highlights the toggle). */
  chatOpen?: boolean;
  /** Open the Lane Board (agent work system) screen. Absent → no workspace active. */
  onOpenLaneBoard?: () => void;
  /** Whether the Lane Board is currently shown (drives active styling). */
  laneBoardOpen?: boolean;
  /** Open the rename/recolor/delete modal for the active workspace. */
  onEditWorkspace?: () => void;
}

export function Roster({
  workspaceId,
  workspaceName,
  folderPath,
  selectedId,
  onSelect,
  onCreateAgent,
  agentsVersion,
  onAgentsChanged,
  onOpenBlackboard,
  blackboardOpen,
  onOpenMemory,
  memoryOpen,
  onOpenChat,
  chatOpen,
  onOpenLaneBoard,
  laneBoardOpen,
  onEditWorkspace,
}: RosterProps) {
  const [entries, setEntries] = useState<RosterEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState(false);
  const [search, setSearch] = useState("");
  // Add-agent picker (choose an existing Library agent to add to this workspace).
  const [showPicker, setShowPicker] = useState(false);
  // Roster chip entry (plan supervisor-picker-ui, Lane C step 5): the
  // instance whose reports-to chip was clicked, or null when the picker is
  // closed.
  const [editingSupervisorFor, setEditingSupervisorFor] = useState<string | null>(null);
  const [supervisorSubmitting, setSupervisorSubmitting] = useState(false);
  const [supervisorError, setSupervisorError] = useState<string | null>(null);
  // Instance id currently being removed (disables its confirm button).
  const [removingId, setRemovingId] = useState<string | null>(null);
  const loadSeq = useRef(0);

  // Roster-chip edit (D6-adjacent: clearing an EXISTING supervisor link is
  // the action, so unlike the add-flow this always calls setPosition, even
  // for a null pick — see Detoro's ruling on the add-flow/edit-flow split).
  async function handleSetSupervisor(workspaceAgentId: string, supervisorAgentId: string | null) {
    if (workspaceId === null) return;
    setSupervisorSubmitting(true);
    setSupervisorError(null);
    try {
      await ipc.instance.setPosition({ workspaceId, workspaceAgentId, supervisorAgentId });
      setEditingSupervisorFor(null);
    } catch (err) {
      setSupervisorError(err instanceof Error ? err.message : String(err));
    } finally {
      setSupervisorSubmitting(false);
    }
  }

  async function handleRemove(instanceId: string) {
    setRemovingId(instanceId);
    try {
      await ipc.instance.remove({ workspaceAgentId: instanceId });
      onAgentsChanged?.();
    } catch (err) {
      if (import.meta.env.DEV) console.error("Roster: instance.remove failed", err);
    } finally {
      setRemovingId(null);
    }
  }

  const loadEntries = useCallback(
    async (withLoading: boolean) => {
      const mine = ++loadSeq.current;
      if (workspaceId === null) {
        setEntries([]);
        setLoading(false);
        setLoadError(false);
        return;
      }

      if (withLoading) {
        setEntries([]);
        setLoading(true);
      }
      setLoadError(false);

      try {
        const [instances, defs] = await Promise.all([
          ipc.instance.list({ workspaceId }),
          ipc.agentDef.list(),
        ]);
        if (mine !== loadSeq.current) return;

        const byId = new Map<string, AgentDefinition>(defs.map((def) => [def.id, def]));
        const identityByInstanceId = new Map<string, PositionPerson>();
        for (const inst of instances) {
          const def = byId.get(inst.agentDefId);
          identityByInstanceId.set(inst.id, {
            name: inst.name ?? def?.name ?? inst.id,
            color: def?.color ?? "#6e6e73",
          });
        }
        const rosterEntries: RosterEntry[] = [];
        for (const inst of instances) {
          const def = byId.get(inst.agentDefId);
          if (!def) continue;
          rosterEntries.push({
            instanceId: inst.id,
            name: inst.name ?? def.name,
            color: def.color ?? "#6e6e73",
            type: def.type,
            status: inst.status,
            // The enriched roster's resolved role name wins over the legacy
            // free-text label; deriveMeta covers the role-less case.
            meta: inst.roleName ?? deriveMeta(def),
            roleDescription: inst.roleDescription,
            skillsStale: computeSkillsStale(def, inst.launchedSkillIds),
            working: inst.working ?? false,
            sessionId: inst.sessionId,
            level: inst.level,
            roleName: inst.roleName,
            supervisor:
              inst.supervisorAgentId != null
                ? (identityByInstanceId.get(inst.supervisorAgentId) ?? {
                    name: inst.supervisorName ?? inst.supervisorAgentId,
                  })
                : null,
            supervisorAgentId: inst.supervisorAgentId,
          });
        }
        setEntries(rosterEntries);
        setLoading(false);
      } catch (err: unknown) {
        if (mine !== loadSeq.current) return;
        if (import.meta.env.DEV) {
          console.error("Roster: instance.list / agentDef.list failed", err);
        }
        if (withLoading) setEntries([]);
        setLoading(false);
        setLoadError(true);
      }
    },
    [workspaceId],
  );

  // Fetch + join instances with their definitions whenever the workspace changes.
  useEffect(() => {
    void loadEntries(true);
  }, [loadEntries, agentsVersion]);

  // Position writes emit `roster:changed { workspaceId }`; refetch the full
  // roster so level/supervisor chains stay fresh across every surface.
  useEvent<RosterChangedEvent>(EVENT_NAMES.rosterChanged, (payload) => {
    if (payload.workspaceId !== workspaceId) return;
    void loadEntries(false);
  });

  // Live status dots. A newly-added agent is "idle" until the WorkspacePane
  // lazily spawns its session, which flips the workspace_agent to "running" and
  // emits `session:status`. The roster fetched its statuses BEFORE that spawn,
  // so without this the dot stays stale at idle until a reload. On any status
  // event, re-read instance.list and patch the dots in place (no loading flash).
  // The event carries only sessionId, so we refetch rather than map directly.
  useEvent<SessionStatusEvent>(EVENT_NAMES.sessionStatus, () => {
    if (workspaceId === null) return;
    ipc.instance
      .list({ workspaceId })
      .then((instances) => {
        const byId = new Map(instances.map((i) => [i.id, i]));
        setEntries((prev) =>
          prev.map((e) => {
            const next = byId.get(e.instanceId);
            if (!next) return e;
            const working = next.working ?? false;
            if (next.status === e.status && working === e.working && next.sessionId === e.sessionId) {
              return e;
            }
            return { ...e, status: next.status, working, sessionId: next.sessionId };
          }),
        );
      })
      .catch(() => {
        // Transient failure — the dot just stays at its last value.
      });
  });

  // Live working state (R-act-1, pull-model — R-act-2): every streamed chunk
  // marks its instance working and re-arms a per-instance WORKING_WINDOW_MS
  // timeout that flips it back to quiet. Timeouts live outside React state
  // (a plain ref) since they're a side effect, not something to render.
  const workingTimeoutsRef = useRef<Map<string, number>>(new Map());
  useEvent<SessionOutputEvent>(EVENT_NAMES.sessionOutput, (payload) => {
    // bb plan:working-false-positive: the engine flags a chunk it judged to be
    // our own echo (resize-provoked repaint, keystroke echo) rather than
    // genuine agent activity via `activity: false`. Terminal still renders it
    // (the pane must show the echo); Roster must not, or an idle agent lights
    // up "working" from its own terminal noise.
    if (!payload.activity) return;
    const entry = entries.find((e) => e.sessionId === payload.sessionId);
    if (!entry) return; // not a live instance we're tracking right now

    const timeouts = workingTimeoutsRef.current;
    const prevTimeout = timeouts.get(entry.instanceId);
    if (prevTimeout !== undefined) window.clearTimeout(prevTimeout);
    timeouts.set(
      entry.instanceId,
      window.setTimeout(() => {
        setEntries((cur) =>
          cur.map((e) => (e.instanceId === entry.instanceId ? { ...e, working: false } : e)),
        );
        timeouts.delete(entry.instanceId);
      }, WORKING_WINDOW_MS),
    );

    // Already working — the timer above is re-armed either way; skip the
    // setState so an actively-streaming instance doesn't churn re-renders.
    if (entry.working) return;
    setEntries((cur) =>
      cur.map((e) => (e.instanceId === entry.instanceId ? { ...e, working: true } : e)),
    );
  });

  // Clear every pending working-timeout on workspace switch (stale
  // instanceIds from the old workspace must not fire into the new one's
  // entries) and on unmount.
  useEffect(() => {
    const timeouts = workingTimeoutsRef.current;
    return () => {
      timeouts.forEach((id) => window.clearTimeout(id));
      timeouts.clear();
    };
  }, [workspaceId]);

  // Client-side search filter — case-insensitive match on name and meta.
  const q = search.trim().toLowerCase();
  const filtered = q
    ? entries.filter(
        (e) => e.name.toLowerCase().includes(q) || e.meta.toLowerCase().includes(q),
      )
    : entries;

  const orchestrators = filtered.filter((e) => e.type === "orchestrator");
  const cliAgents = filtered.filter((e) => e.type === "cli");
  const chatAgents = filtered.filter((e) => e.type === "chat");

  // Workspace header: use real name/folderPath; static blue avatar (no fake state).
  const wsLetter = workspaceName ? workspaceName[0].toUpperCase() : "—";

  return (
    <aside className="w-[266px] vibrancy border-r border-overlay/[0.06] flex flex-col shrink-0">
      {/* Workspace header — real workspaceName + folderPath. Doubles as a window
          drag region: the name/avatar are `pointer-events-none` so clicks fall
          through to the attributed container (dragging / double-click-zoom),
          while the edit button is a REAL interactive element (no drag attribute
          of its own) that keeps its own clicks, same pattern as the tab strip
          buttons in WorkspacePane. */}
      <div
        data-tauri-drag-region
        className="h-12 flex items-center gap-2 px-3.5 border-b border-overlay/[0.06] shrink-0"
      >
        <div className="w-6 h-6 rounded-[7px] bg-accent text-white grid place-items-center text-[11px] font-bold ring-hair shrink-0 pointer-events-none">
          {wsLetter}
        </div>
        <div className="flex-1 leading-tight text-left min-w-0 pointer-events-none">
          <div className="text-[12.5px] font-semibold tracking-tight truncate">
            {workspaceName ?? "—"}
          </div>
          {folderPath && (
            <div className="text-[10px] text-text-muted truncate flex items-center gap-1">
              <Folder className="w-2.5 h-2.5 shrink-0" />
              <span className="font-mono">{folderPath}</span>
            </div>
          )}
        </div>
        {onEditWorkspace && (
          <button
            onClick={onEditWorkspace}
            className="w-6 h-6 grid place-items-center rounded-md text-text-muted hover:bg-overlay/[0.06] hover:text-text-primary shrink-0"
            title="Edit workspace"
            aria-label="Edit workspace"
          >
            <Pencil className="w-3 h-3" />
          </button>
        )}
      </div>

      {/* Search — functional client-side filter */}
      <div className="px-3 pt-3 pb-2 shrink-0">
        <div className="flex items-center gap-2 bg-overlay/[0.05] rounded-lg px-2.5 h-7">
          <Search className="w-[13px] h-[13px] text-text-muted shrink-0" />
          <input
            type="search"
            aria-label="Search agents"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search agents"
            className="bg-transparent outline-none text-[12px] placeholder:text-text-tertiary w-full"
          />
        </div>
      </div>

      {/* Agent list — state messages or grouped sections */}
      {workspaceId === null ? (
        <div className="flex-1 grid place-items-center text-[12px] text-text-tertiary px-4 text-center">
          Select a workspace
        </div>
      ) : loading ? (
        <div className="flex-1 grid place-items-center text-[12px] text-text-tertiary">
          Loading agents…
        </div>
      ) : loadError ? (
        <div className="flex-1 grid place-items-center text-[12px] text-text-tertiary">
          Failed to load agents
        </div>
      ) : entries.length === 0 ? (
        <div className="flex-1 grid place-items-center text-[12px] text-text-tertiary px-4 text-center">
          No agents in this workspace yet
        </div>
      ) : (
        <div className="flex-1 overflow-y-auto scroll-thin px-2 pb-2 space-y-3">
          {filtered.length === 0 ? (
            <div className="px-2 py-2 text-[12px] text-text-tertiary">No matching agents</div>
          ) : (
            <>
              {orchestrators.length > 0 && (
                <div>
                  <div className="px-2 mb-1 text-[10px] font-bold tracking-wider text-text-tertiary uppercase">
                    Orchestrator
                  </div>
                  {orchestrators.map((entry) => (
                    <AgentRow
                      key={entry.instanceId}
                      entry={entry}
                      isSelected={selectedId === entry.instanceId}
                      onSelect={() => onSelect(entry.instanceId)}
                      onRemove={() => handleRemove(entry.instanceId)}
                      removing={removingId === entry.instanceId}
                      onEditSupervisor={() => setEditingSupervisorFor(entry.instanceId)}
                    />
                  ))}
                </div>
              )}

              {cliAgents.length > 0 && (
                <div>
                  <div className="px-2 mb-1 text-[10px] font-bold tracking-wider text-text-tertiary uppercase">
                    CLI agents
                  </div>
                  <div className="space-y-0.5">
                    {cliAgents.map((entry) => (
                      <AgentRow
                        key={entry.instanceId}
                        entry={entry}
                        isSelected={selectedId === entry.instanceId}
                        onSelect={() => onSelect(entry.instanceId)}
                        onRemove={() => handleRemove(entry.instanceId)}
                        removing={removingId === entry.instanceId}
                        onEditSupervisor={() => setEditingSupervisorFor(entry.instanceId)}
                      />
                    ))}
                  </div>
                </div>
              )}

              {chatAgents.length > 0 && (
                <div>
                  <div className="px-2 mb-1 text-[10px] font-bold tracking-wider text-text-tertiary uppercase">
                    Chat agents
                  </div>
                  <div className="space-y-0.5">
                    {chatAgents.map((entry) => (
                      <AgentRow
                        key={entry.instanceId}
                        entry={entry}
                        isSelected={selectedId === entry.instanceId}
                        onSelect={() => onSelect(entry.instanceId)}
                        onRemove={() => handleRemove(entry.instanceId)}
                        removing={removingId === entry.instanceId}
                        onEditSupervisor={() => setEditingSupervisorFor(entry.instanceId)}
                      />
                    ))}
                  </div>
                </div>
              )}
            </>
          )}
        </div>
      )}

      {/* Footer */}
      <div className="border-t border-overlay/[0.06] p-2 space-y-0.5 shrink-0">
        <button
          onClick={() => setShowPicker(true)}
          disabled={workspaceId === null}
          className="w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg text-accent hover:bg-accent/10 disabled:opacity-40 disabled:cursor-not-allowed"
        >
          <div className="w-7 h-7 rounded-[8px] border border-dashed border-accent/50 grid place-items-center shrink-0">
            <Plus className="w-[15px] h-[15px]" />
          </div>
          <span className="text-[12.5px] font-semibold">Add agent</span>
        </button>
        <button
          onClick={onOpenBlackboard}
          disabled={!onOpenBlackboard}
          className={`w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg transition-colors disabled:opacity-40 disabled:cursor-not-allowed ${
            blackboardOpen ? "bg-overlay/[0.06]" : "hover:bg-overlay/[0.04]"
          }`}
        >
          <div className="w-7 h-7 rounded-[8px] bg-ink text-on-ink grid place-items-center ring-hair shrink-0">
            <Layers className="w-[14px] h-[14px]" />
          </div>
          <div className="flex-1 text-left leading-tight">
            <div className="text-[12.5px] font-semibold">Blackboard</div>
            <div className="text-[10.5px] text-text-muted">Shared key/value store</div>
          </div>
        </button>
        <button
          onClick={onOpenMemory}
          disabled={!onOpenMemory}
          className={`w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg transition-colors disabled:opacity-40 disabled:cursor-not-allowed ${
            memoryOpen ? "bg-overlay/[0.06]" : "hover:bg-overlay/[0.04]"
          }`}
        >
          <div className="w-7 h-7 rounded-[8px] bg-ink text-on-ink grid place-items-center ring-hair shrink-0">
            <Network className="w-[14px] h-[14px]" />
          </div>
          <div className="flex-1 text-left leading-tight">
            <div className="text-[12.5px] font-semibold">Memory</div>
            <div className="text-[10.5px] text-text-muted">Knowledge graph</div>
          </div>
        </button>
        <button
          onClick={onOpenChat}
          disabled={!onOpenChat}
          className={`w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg transition-colors disabled:opacity-40 disabled:cursor-not-allowed ${
            chatOpen ? "bg-overlay/[0.06]" : "hover:bg-overlay/[0.04]"
          }`}
        >
          <div className="w-7 h-7 rounded-[8px] bg-ink text-on-ink grid place-items-center ring-hair shrink-0">
            <MessageSquare className="w-[14px] h-[14px]" />
          </div>
          <div className="flex-1 text-left leading-tight">
            <div className="text-[12.5px] font-semibold">Chat</div>
            <div className="text-[10.5px] text-text-muted">Agent conversations</div>
          </div>
        </button>
        <button
          onClick={onOpenLaneBoard}
          disabled={!onOpenLaneBoard}
          className={`w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg transition-colors disabled:opacity-40 disabled:cursor-not-allowed ${
            laneBoardOpen ? "bg-overlay/[0.06]" : "hover:bg-overlay/[0.04]"
          }`}
        >
          <div className="w-7 h-7 rounded-[8px] bg-ink text-on-ink grid place-items-center ring-hair shrink-0">
            <Columns3 className="w-[14px] h-[14px]" />
          </div>
          <div className="flex-1 text-left leading-tight">
            <div className="text-[12.5px] font-semibold">Lane board</div>
            <div className="text-[10.5px] text-text-muted">Agent work system</div>
          </div>
        </button>
      </div>

      {/* Add-agent picker — choose an existing Library agent to add here. */}
      {showPicker && workspaceId !== null && (
        <AddAgentPicker
          workspaceId={workspaceId}
          onClose={() => setShowPicker(false)}
          onAdded={() => {
            setShowPicker(false);
            onAgentsChanged?.();
          }}
          onCreateAgent={
            onCreateAgent
              ? () => {
                  setShowPicker(false);
                  onCreateAgent();
                }
              : undefined
          }
        />
      )}

      {/* Roster-chip supervisor edit (plan supervisor-picker-ui, Lane C step 5). */}
      {editingSupervisorFor !== null &&
        workspaceId !== null &&
        (() => {
          const editingEntry = entries.find((e) => e.instanceId === editingSupervisorFor);
          if (!editingEntry) return null;
          const members: SupervisorCandidate[] = entries.map((e) => ({
            id: e.instanceId,
            name: e.name,
            color: e.color,
            level: e.level,
            roleName: e.roleName,
          }));
          const excludeIds = [
            editingSupervisorFor,
            ...descendantsOf(editingSupervisorFor, entries.map((e) => ({
              id: e.instanceId,
              supervisorAgentId: e.supervisorAgentId,
            }))),
          ];
          return (
            <SupervisorPicker
              variant="edit"
              subject={{
                name: editingEntry.name,
                color: editingEntry.color,
                sub: editingEntry.supervisor
                  ? `Currently reports to ${editingEntry.supervisor.name}`
                  : "Currently reports to the human",
              }}
              members={members}
              excludeIds={excludeIds}
              selfId={editingSupervisorFor}
              current={editingEntry.supervisorAgentId ?? null}
              submitting={supervisorSubmitting}
              error={supervisorError}
              onPick={(supervisorId) => void handleSetSupervisor(editingSupervisorFor, supervisorId)}
              onClose={() => {
                setEditingSupervisorFor(null);
                setSupervisorError(null);
              }}
            />
          );
        })()}
    </aside>
  );
}

// ---------------------------------------------------------------------------
// AddAgentPicker — modal listing Library agents not yet in this workspace.
// ---------------------------------------------------------------------------

interface AddAgentPickerProps {
  workspaceId: string;
  onClose: () => void;
  onAdded: () => void;
  onCreateAgent?: () => void;
}

function AddAgentPicker({ workspaceId, onClose, onAdded, onCreateAgent }: AddAgentPickerProps) {
  const [available, setAvailable] = useState<AgentDefinition[] | null>(null);
  const [members, setMembers] = useState<WorkspaceAgent[]>([]);
  const [error, setError] = useState(false);
  // Step 2 (plan supervisor-picker-ui, Lane C step 4): picking an agent
  // doesn't call the API immediately — it swaps to the supervisor step in
  // the SAME modal position/size, `pendingDef` carrying the choice through.
  const [pendingDef, setPendingDef] = useState<AgentDefinition | null>(null);
  const [submitting, setSubmitting] = useState(false);
  // D6: addToWorkspace succeeding but setPosition failing must surface
  // inline and NOT be silently swallowed — but also must not close the modal
  // out from under the message, so `onAdded` fires only once the user
  // dismisses this banner via Done, not the instant the write fails.
  const [partialFailure, setPartialFailure] = useState<string | null>(null);

  // Load all defs ⨝ this workspace's instances → defs NOT already present.
  // The instances double as SupervisorPicker's member list for step 2 — no
  // second round-trip when the user picks an agent.
  useEffect(() => {
    let active = true;
    Promise.all([ipc.agentDef.list(), ipc.instance.list({ workspaceId })])
      .then(([defs, instances]) => {
        if (!active) return;
        const present = new Set(instances.map((i) => i.agentDefId));
        setAvailable(defs.filter((d) => !present.has(d.id)));
        setMembers(instances);
      })
      .catch(() => {
        if (active) setError(true);
      });
    return () => {
      active = false;
    };
  }, [workspaceId]);

  // D6: add is NEVER rolled back once addToWorkspace succeeds — a setPosition
  // failure only changes how the flow ENDS, never whether the agent stays in
  // the workspace.
  async function handleConfirmAdd(supervisorId: string | null) {
    if (!pendingDef) return;
    setSubmitting(true);
    try {
      const created = await ipc.agentDef.addToWorkspace({
        agentDefId: pendingDef.id,
        workspaceIds: [workspaceId],
      });
      // addToWorkspace can return instances for MULTIPLE workspaces — take
      // the row for THIS one, never assume [0].
      const mine = created.find((wa) => wa.workspaceId === workspaceId);
      if (!mine) {
        throw new Error("Agent was added, but no instance came back for this workspace");
      }
      // Ruled: a null pick needs no write — a fresh instance already has no
      // supervisor, so calling setPosition would be a no-op with an extra
      // failure surface for nothing.
      if (supervisorId !== null) {
        try {
          await ipc.instance.setPosition({
            workspaceId,
            workspaceAgentId: mine.id,
            supervisorAgentId: supervisorId,
          });
        } catch (err) {
          const detail = err instanceof Error ? err.message : String(err);
          setPartialFailure(
            `Added, but setting the supervisor failed — set it from the roster chip or Builder. (${detail})`,
          );
          setSubmitting(false);
          return;
        }
      }
      onAdded();
    } catch (err) {
      if (import.meta.env.DEV) console.error("AddAgentPicker: add flow failed", err);
      setSubmitting(false);
      // Failed before or during addToWorkspace itself — nothing was added,
      // so back out to the agent list rather than stranding the user on a
      // supervisor step for an agent that was never added.
      setPendingDef(null);
    }
  }

  const step2Members: SupervisorCandidate[] = members.map((m) => ({
    id: m.id,
    name: m.name,
    color: undefined,
    level: m.level,
    roleName: m.roleName,
  }));

  if (pendingDef) {
    if (partialFailure) {
      return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30">
          <div className="w-[420px] bg-surface rounded-2xl shadow-2xl flex flex-col overflow-hidden ring-1 ring-overlay/[0.08]">
            <div className="h-12 flex items-center px-5 border-b border-overlay/[0.06] shrink-0">
              <span className="text-[13px] font-semibold tracking-tight">
                {pendingDef.name} added
              </span>
            </div>
            <div className="p-4 text-[12px] text-danger">{partialFailure}</div>
            <div className="border-t border-overlay/[0.06] p-2 flex justify-end shrink-0">
              <button
                onClick={onAdded}
                className="text-[12.5px] font-semibold text-white bg-accent px-3.5 py-1.5 rounded-lg hover:brightness-105"
              >
                Done
              </button>
            </div>
          </div>
        </div>
      );
    }
    return (
      <SupervisorPicker
        variant="add"
        step="Step 2 of 2"
        subject={{
          name: pendingDef.name,
          color: pendingDef.color,
          sub: "You're adding this agent to the workspace",
        }}
        members={step2Members}
        excludeIds={[]}
        current={null}
        submitting={submitting}
        onBack={() => setPendingDef(null)}
        onClose={onClose}
        onPick={(supervisorId) => void handleConfirmAdd(supervisorId)}
      />
    );
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/30"
      onClick={onClose}
    >
      <div
        className="w-[420px] max-h-[70vh] bg-surface rounded-2xl shadow-2xl flex flex-col overflow-hidden ring-1 ring-overlay/[0.08]"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="h-12 flex items-center justify-between px-5 border-b border-overlay/[0.06] shrink-0">
          <span className="text-[13px] font-semibold tracking-tight">Add agent to workspace</span>
          <button
            onClick={onClose}
            className="w-7 h-7 grid place-items-center rounded-md hover:bg-overlay/[0.05] text-text-secondary"
            aria-label="Close"
          >
            <X className="w-[15px] h-[15px]" />
          </button>
        </div>

        <div className="flex-1 overflow-y-auto scroll-thin p-2 min-h-0">
          {available === null && !error ? (
            <div className="grid place-items-center py-8 text-[12px] text-text-tertiary">Loading…</div>
          ) : error ? (
            <div className="grid place-items-center py-8 text-[12px] text-text-tertiary">
              Failed to load agents
            </div>
          ) : available && available.length === 0 ? (
            <div className="grid place-items-center py-8 text-[12px] text-text-tertiary px-6 text-center">
              Every agent in your Library is already in this workspace.
            </div>
          ) : (
            available?.map((def) => (
              <button
                key={def.id}
                onClick={() => setPendingDef(def)}
                className="w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg hover:bg-overlay/[0.04] disabled:opacity-50 text-left"
              >
                <div
                  className="w-7 h-7 rounded-[8px] text-white grid place-items-center text-[12px] font-bold ring-hair shrink-0"
                  style={{ backgroundColor: def.color ?? "#6e6e73" }}
                >
                  {def.type === "orchestrator" ? (
                    <Waypoints className="w-[15px] h-[15px]" />
                  ) : (
                    def.name[0]?.toUpperCase()
                  )}
                </div>
                <div className="flex-1 min-w-0 leading-tight">
                  <div className="text-[12.5px] font-semibold truncate">{def.name}</div>
                  <div className="text-[10.5px] text-text-muted truncate">{deriveMeta(def)}</div>
                </div>
                <span className="text-[11px] font-semibold text-accent shrink-0">Add</span>
              </button>
            ))
          )}
        </div>

        {onCreateAgent && (
          <div className="border-t border-overlay/[0.06] p-2 shrink-0">
            <button
              onClick={onCreateAgent}
              className="w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg text-accent hover:bg-accent/10"
            >
              <div className="w-7 h-7 rounded-[8px] border border-dashed border-accent/50 grid place-items-center shrink-0">
                <Plus className="w-[15px] h-[15px]" />
              </div>
              <span className="text-[12.5px] font-semibold">Create new agent…</span>
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
