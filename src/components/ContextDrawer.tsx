import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  PanelRight,
  Bot,
  CheckCircle2,
  Inbox,
  Send,
  Wrench,
  Sparkles,
  Camera,
  Clock,
  Waypoints,
  CornerDownLeft,
  CornerUpRight,
  ChevronDown,
  Trash2,
  Scale,
  Gauge,
  History,
  RotateCcw,
} from "lucide-react";
import { ipc, useMessageInjected, useSessionContext, useSnapshotCreated } from "../ipc";
import type {
  AgentDefinition,
  InterAgentMessage,
  Session,
  Skill,
  Snapshot,
  WorkspaceAgent,
} from "../ipc";
import type { RoutingTarget } from "./RoutingPicker";
import { timeHint } from "../lib/timeHint";
import { computeSkillsStale } from "../lib/skills";
import { DeferredNote } from "./DeferredNote";

// ---------------------------------------------------------------------------
// Right-side Context drawer — shows the ACTIVE agent's configuration. The
// section set adapts to the agent's `type` (cli / chat / orchestrator).
//
// HONESTY NOTE — this drawer renders ONLY data that genuinely exists in
// `AgentDefinition` (+ the live `Session` when available). Sections without a
// real data source yet (Tools/Skills → M5, inter-agent message log → M3,
// snapshots/Session cwd/branch/pid → M4 / not modelled) render an honest
// deferred placeholder, NOT fabricated chips/rows. Each deferred block names
// its milestone, mirroring how `ChatView.tsx` documents its deferred parts.
// ---------------------------------------------------------------------------

interface ContextDrawerProps {
  /** The active agent's full definition (carries name/color/type/config). */
  def: AgentDefinition;
  /** Live status of the active instance. */
  status: WorkspaceAgent["status"];
  /** The active instance id — keys the inbox/outbox message query. */
  instanceId: string;
  /** The workspace routing roster — resolves message counterpart names. */
  roster: RoutingTarget[];
  /** The active session, when one has been spawned (for context meter). */
  session?: Session | null;
  /** Skill ids the live session actually launched with (undefined before any
   *  launch) — drives the Skills section's "restart to apply" drift hint. */
  launchedSkillIds?: string[];
}

// How many recent inbox/outbox rows the Messages log shows.
const MESSAGE_LOG_LIMIT = 6;

// How many recent snapshots the Memory section shows.
const SNAPSHOT_LOG_LIMIT = 6;

function allowedSendersLabel(v: AgentDefinition["allowedSenders"]): string {
  switch (v) {
    case "all":
      return "All agents";
    case "selected":
      return "Selected only";
    case "none":
      return "None";
    default:
      return "—";
  }
}

// Small section header, matching the prototype's uppercase label rows.
function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5 px-0.5 flex items-center justify-between">
      {children}
    </div>
  );
}

export function ContextDrawer({
  def,
  status,
  instanceId,
  roster,
  session,
  launchedSkillIds,
}: ContextDrawerProps) {
  // Simplest collapse affordance: internal open/closed state. The header
  // `panel-right` button toggles it; collapsed → a thin strip to reopen.
  // Scope: this state is workspace-scoped — it persists across tab switches
  // (drawer visibility is a user preference, not per-agent) and resets only
  // when the pane remounts (WorkspacePane is keyed by workspaceId).
  const [open, setOpen] = useState(true);

  // Recent inbox/outbox for this instance — REAL data from `message.list`.
  const [messages, setMessages] = useState<InterAgentMessage[]>([]);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  // Fetch on mount / when the active instance changes, and on each injection.
  // `refetch` is memoised per instanceId so the effect deps stay honest (no
  // exhaustive-deps trap). A monotonic seq guard drops stale results: if two
  // injections fire in quick succession, only the latest list response wins,
  // even if an earlier (slower) request settles after it.
  const seq = useRef(0);
  const refetch = useCallback(() => {
    const mine = ++seq.current;
    ipc.message
      .list({ instanceId, limit: MESSAGE_LOG_LIMIT })
      .then((rows) => {
        if (mounted.current && mine === seq.current) setMessages(rows);
      })
      .catch((err: unknown) => {
        // Non-Tauri dev context (plain `vite`) lands here — surface in dev only.
        if (import.meta.env.DEV) {
          console.error("ContextDrawer: message.list failed", err);
        }
      });
  }, [instanceId]);
  useEffect(() => {
    refetch();
  }, [refetch]);

  // Refetch when an injection involving this instance fires (in OR out).
  useMessageInjected(instanceId, () => refetch());

  // ── Skills (REAL — the agent's effective skill set) ───────────────────────
  // `def.skillIds` is annotated by `agentDef.list` (mandatory builtins +
  // selected optional builtins + attached custom, in launch order); the skill
  // catalog is fetched once to resolve ids → names/kind. Only `cli` agents
  // consume skills at launch (the spawn path injects them via the sidecar), so
  // the section renders real chips for cli and an honest note otherwise.
  const [skillCatalog, setSkillCatalog] = useState<Skill[] | null>(null);
  useEffect(() => {
    ipc.skill
      .list()
      .then((rows) => {
        if (mounted.current) setSkillCatalog(rows);
      })
      .catch((err: unknown) => {
        if (import.meta.env.DEV) {
          console.error("ContextDrawer: skill.list failed", err);
        }
      });
  }, []);
  const skillsById = useMemo(
    () => new Map((skillCatalog ?? []).map((s) => [s.id, s])),
    [skillCatalog],
  );
  const effectiveSkillIds = def.skillIds ?? [];
  // Drift: the live session launched with a DIFFERENT set than the def now
  // carries — a restart re-applies the current set (same basis as the Roster's
  // stale badge).
  const skillsStale = computeSkillsStale(def, launchedSkillIds);

  // ── Session (restart · resume) ─────────────────────────────────────────────
  // Restart is save-gated and destructive (kills the process), so it sits
  // behind a Yes/No confirm like Compact. Resume is non-destructive (types the
  // resume prompt into the live terminal) and only enabled once a handoff
  // snapshot exists to resume from.
  const [restartConfirming, setRestartConfirming] = useState(false);
  // null = not restarting; "saving" = waiting for the agent's handoff save;
  // "respawning" = process killed (or was dead), fresh spawn booting.
  const [restartPhase, setRestartPhase] = useState<"saving" | "respawning" | null>(null);
  const [resumeBusy, setResumeBusy] = useState(false);
  const [sessionError, setSessionError] = useState<string | null>(null);

  const doRestart = useCallback(() => {
    setRestartConfirming(false);
    setSessionError(null);
    ipc.instance
      .restart({ workspaceAgentId: instanceId })
      .then((res) => {
        if (mounted.current) setRestartPhase(res.phase);
      })
      .catch((err: unknown) => {
        if (import.meta.env.DEV) {
          console.error("ContextDrawer: instance.restart failed", err);
        }
        if (mounted.current) setSessionError("Couldn't restart this agent");
      });
  }, [instanceId]);

  const doResume = useCallback(() => {
    setSessionError(null);
    setResumeBusy(true);
    ipc.snapshot
      .resume({ instanceId })
      .catch((err: unknown) => {
        if (import.meta.env.DEV) {
          console.error("ContextDrawer: snapshot.resume failed", err);
        }
        if (mounted.current) setSessionError("Couldn't resume this agent");
      })
      .finally(() => {
        // Brief lockout — the prompt was typed; there's no completion event to
        // wait on (the agent simply starts working in its terminal).
        setTimeout(() => {
          if (mounted.current) setResumeBusy(false);
        }, 3_000);
      });
  }, [instanceId]);

  // Progress tracking for the restart loop, driven by the instance status prop:
  // the save phase ends when the backend kills the process (status → idle), and
  // the respawn phase ends when the fresh spawn reports running.
  useEffect(() => {
    if (restartPhase === "saving" && status === "idle") setRestartPhase("respawning");
    else if (restartPhase === "respawning" && status === "running") setRestartPhase(null);
  }, [status, restartPhase]);

  // Failsafe: an agent that never saves its handoff (ignores the prompt, arm
  // expires server-side) would leave "Restarting…" pulsing forever — time it
  // out well past the backend's TTL window and surface an honest error.
  useEffect(() => {
    if (restartPhase === null) return;
    const t = setTimeout(() => {
      if (mounted.current) {
        setRestartPhase(null);
        setSessionError("Restart didn't complete — check the agent's terminal");
      }
    }, 240_000);
    return () => clearTimeout(t);
  }, [restartPhase]);

  // ── Live context meter ─────────────────────────────────────────────────────
  // Seed from the spawned `session` prop (a fresh session reports 0 tokens), then
  // track live `session:context` estimates. HONESTY: this is an ESTIMATE derived
  // from streamed output bytes, not exact provider usage — labelled as such below.
  const sessionId = session?.id ?? "";
  const [ctx, setCtx] = useState<{ tokens: number; limit: number; estimated: boolean } | null>(
    session?.contextTokens != null && session?.contextLimit != null
      ? { tokens: session.contextTokens, limit: session.contextLimit, estimated: true }
      : null,
  );
  useSessionContext(sessionId, (e) =>
    setCtx({ tokens: e.contextTokens, limit: e.contextLimit, estimated: e.estimated }),
  );
  // Reset the meter (and any transient snapshot UI) when the focused session
  // changes. ContextDrawer is NOT keyed per instance (its open/closed state is a
  // workspace-scoped preference that must persist across tab switches), so these
  // `useState` values would otherwise carry the PREVIOUS session's estimate —
  // `ctx` wins over the `session` prop in the meter fallback, so without this the
  // meter shows a stale token count until the new session happens to emit
  // `session:context` (which an idle session never does). Keyed by id, not the
  // whole `session` object, to avoid resetting on unrelated prop identity churn.
  useEffect(() => {
    setCtx(
      session?.contextTokens != null && session?.contextLimit != null
        ? { tokens: session.contextTokens, limit: session.contextLimit, estimated: true }
        : null,
    );
    setSnapshotError(false);
    setSnapshotBusy(false);
    setConfirming(false);
    setCompacting(false);
    setSelectedSnap(null);
    setRowBusy(null);
    setHasHandoff(false);
    setRestartConfirming(false);
    setRestartPhase(null);
    setResumeBusy(false);
    setSessionError(null);
    // `session` is intentionally read but excluded from deps — we re-seed only on
    // identity change of the session id, not on every new session object.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId]);

  // ── Snapshots (Memory section) ─────────────────────────────────────────────
  // Recent snapshots for this session, newest-first. Same refetch discipline as
  // the message log: a per-session useCallback + a monotonic seq guard so a stale
  // response can't overwrite a newer one. Refetches on session change AND on each
  // `snapshot:created` (so an auto-compact snapshot appears live).
  const [snapshots, setSnapshots] = useState<Snapshot[]>([]);
  // Whether ANY handoff snapshot exists for this session (computed from the
  // FULL list, before the display slice) — gates the Session section's Resume.
  const [hasHandoff, setHasHandoff] = useState(false);
  const [snapshotError, setSnapshotError] = useState(false);
  const [snapshotBusy, setSnapshotBusy] = useState(false);
  // Compact-loop UI: a Yes/No confirm gate (the loop CLEARS the agent, so we
  // never fire it on a stray click), and a transient "compacting" note while the
  // backend drives save → clear → restore in the agent's terminal.
  const [confirming, setConfirming] = useState(false);
  const [compacting, setCompacting] = useState(false);
  // Inline snapshot detail: the id of the expanded row (click a row to view its
  // saved content + per-row actions), and the id of a row with a delete/send in
  // flight (to disable its buttons).
  const [selectedSnap, setSelectedSnap] = useState<string | null>(null);
  const [rowBusy, setRowBusy] = useState<string | null>(null);
  const snapSeq = useRef(0);
  const refetchSnapshots = useCallback(() => {
    if (!sessionId) {
      setSnapshots([]);
      return;
    }
    const mine = ++snapSeq.current;
    ipc.snapshot
      .list({ sessionId })
      .then((rows) => {
        if (mounted.current && mine === snapSeq.current) {
          setSnapshots(rows.slice(0, SNAPSHOT_LOG_LIMIT));
          setHasHandoff(rows.some((r) => r.type === "handoff"));
          setSnapshotError(false); // a successful list dismisses a stale create-error
        }
      })
      .catch((err: unknown) => {
        if (import.meta.env.DEV) {
          console.error("ContextDrawer: snapshot.list failed", err);
        }
      });
  }, [sessionId]);
  useEffect(() => {
    refetchSnapshots();
  }, [refetchSnapshots]);
  useSnapshotCreated(sessionId, () => {
    // A snapshot landing means the compact loop's handoff was written — clear the
    // transient "compacting" note (the /clear + restore now play out in the term).
    setCompacting(false);
    refetchSnapshots();
  });

  // "Snapshot now" — create a manual snapshot, then refetch. Errors are surfaced
  // honestly (dev console + a tiny inline note), never silently swallowed.
  const doSnapshot = useCallback(() => {
    if (!sessionId) return;
    setSnapshotBusy(true);
    setSnapshotError(false);
    ipc.snapshot
      .create({ sessionId, type: "manual" })
      .then(() => {
        if (mounted.current) refetchSnapshots();
      })
      .catch((err: unknown) => {
        if (import.meta.env.DEV) {
          console.error("ContextDrawer: snapshot.create failed", err);
        }
        if (mounted.current) setSnapshotError(true);
      })
      .finally(() => {
        if (mounted.current) setSnapshotBusy(false);
      });
  }, [sessionId, refetchSnapshots]);

  // "Compact" — the strategic-compact loop for a live CLI agent. The backend
  // injects a "save your handoff" prompt, waits for the agent to write its
  // handoff snapshot, then `/clear`s and injects a "restore" prompt. The call
  // returns immediately ("compacting"); the loop then plays out in the agent's
  // own terminal, and the new handoff snapshot arrives via `snapshot:created`.
  const doCompact = useCallback(() => {
    setConfirming(false);
    setSnapshotError(false);
    setCompacting(true);
    // `snapshot.compact` returns IMMEDIATELY (the loop then plays out in the
    // agent's terminal), so there's nothing to refetch here — the new handoff
    // snapshot arrives via `snapshot:created`, which clears `compacting`. We only
    // handle the call itself failing (e.g. the agent isn't live).
    ipc.snapshot.compact({ instanceId }).catch((err: unknown) => {
      if (import.meta.env.DEV) {
        console.error("ContextDrawer: snapshot.compact failed", err);
      }
      if (mounted.current) {
        setSnapshotError(true);
        setCompacting(false);
      }
    });
  }, [instanceId]);

  // Failsafe: if the agent never writes its handoff (ignores the prompt, is
  // killed, the loop aborts server-side), no `snapshot:created` ever fires and
  // the "Compacting…" note would pulse forever. Time it out a bit beyond the
  // backend's own ~90s wait and surface an honest error instead of a stuck UI.
  useEffect(() => {
    if (!compacting) return;
    const t = setTimeout(() => {
      if (mounted.current) {
        setCompacting(false);
        setSnapshotError(true);
      }
    }, 120_000);
    return () => clearTimeout(t);
  }, [compacting]);

  // Row action — delete a snapshot, then refetch. Collapses the row if it was
  // expanded. Errors surface via the inline note (never swallowed).
  const doDeleteSnapshot = useCallback(
    (snapshotId: string) => {
      setRowBusy(snapshotId);
      setSnapshotError(false);
      ipc.snapshot
        .delete({ snapshotId })
        .then(() => {
          if (!mounted.current) return;
          setSelectedSnap((cur) => (cur === snapshotId ? null : cur));
          refetchSnapshots();
        })
        .catch((err: unknown) => {
          if (import.meta.env.DEV) {
            console.error("ContextDrawer: snapshot.delete failed", err);
          }
          if (mounted.current) setSnapshotError(true);
        })
        .finally(() => {
          if (mounted.current) setRowBusy(null);
        });
    },
    [refetchSnapshots],
  );

  // Row action — submit a snapshot's content into the live agent's terminal.
  const doSendSnapshot = useCallback(
    (snapshotId: string) => {
      setRowBusy(snapshotId);
      setSnapshotError(false);
      ipc.snapshot
        .send({ instanceId, snapshotId })
        .catch((err: unknown) => {
          if (import.meta.env.DEV) {
            console.error("ContextDrawer: snapshot.send failed", err);
          }
          if (mounted.current) setSnapshotError(true);
        })
        .finally(() => {
          if (mounted.current) setRowBusy(null);
        });
    },
    [instanceId],
  );

  // Resolve a counterpart instance id → display name via the roster.
  const nameOf = (id: string): string =>
    roster.find((t) => t.instanceId === id)?.name ?? id;

  // Fusion panel (orchestrator branch) — the workspace's chat agents excluding
  // self, capped at 8 (the same derivation the M4.3 backend does). Memoised so
  // the drawer's frequent re-renders (message/snapshot/context events) don't
  // re-filter the roster each time. `costMult` = panel + judge + synth.
  const fusionPanel = useMemo(
    () => roster.filter((t) => t.type === "chat" && t.instanceId !== instanceId).slice(0, 8),
    [roster, instanceId],
  );
  const fusionCostMult = fusionPanel.length + 2;

  if (!open) {
    return (
      <aside className="w-9 vibrancy border-l border-overlay/[0.06] flex flex-col items-center shrink-0">
        <button
          title="Show Context"
          onClick={() => setOpen(true)}
          className="w-7 h-7 mt-2.5 grid place-items-center rounded-md hover:bg-overlay/[0.05] text-text-secondary"
        >
          <PanelRight className="w-[15px] h-[15px]" />
        </button>
      </aside>
    );
  }

  // Live meter values: prefer the streamed `ctx` estimate, fall back to the
  // session prop's counts. Render only when a real session reports a usable
  // limit — we never fabricate a meter.
  const meterTokens = ctx?.tokens ?? session?.contextTokens ?? null;
  const meterLimit = ctx?.limit ?? session?.contextLimit ?? null;
  const meterEstimated = ctx?.estimated ?? true;
  // The meter is a CHAT-only concept. For chat agents we own the provider loop,
  // so the streamed-text estimate is a genuine (if rough) proxy. CLI agents
  // (claude-code/codex) manage their own context internally and display it
  // themselves — we have no honest source, and the old byte estimate visibly
  // disagreed with the child's own `/context`, so we don't show a meter for
  // them. Orchestrators have no streaming window at all. Gating on `type` here
  // also covers any stale CLI session row that still carries a pre-fix estimate.
  const showMeter =
    def.type === "chat" &&
    session != null &&
    meterTokens != null &&
    meterLimit != null &&
    meterLimit > 0;

  return (
      <aside className="w-[306px] vibrancy border-l border-overlay/[0.06] flex flex-col shrink-0">
      {/* Header — also a window drag region. The "Context" label is
          `pointer-events-none` so clicks fall through to the attributed
          container (drag / double-click-zoom); the collapse button keeps its
          own click. */}
      <div
        data-tauri-drag-region
        className="h-12 flex items-center justify-between px-4 border-b border-overlay/[0.06] shrink-0"
      >
        <span className="text-[12px] font-semibold text-text-secondary tracking-tight pointer-events-none">
          Context
        </span>
        <button
          title="Hide Context"
          onClick={() => setOpen(false)}
          className="w-7 h-7 grid place-items-center rounded-md hover:bg-overlay/[0.05] text-text-secondary"
        >
          <PanelRight className="w-[15px] h-[15px]" />
        </button>
      </div>

      <div className="flex-1 overflow-y-auto scroll-thin p-3 space-y-4">
        {def.type === "orchestrator" ? (
          // Orchestrator (Fusion) config — REAL/derived values (M4.4). The panel
          // is derived from the workspace's chat agents (the same derivation the
          // M4.3 backend does); the judge is the orchestrator's own model; the
          // cost is an honest derived estimate. Tuning options aren't wired into
          // the pipeline yet → an honest deferred note, NOT fake toggles.
          <>
            {/* Panel agents — derived from the workspace's chat agents. */}
            <div>
              <SectionLabel>
                <span>Panel agents</span>
                <span className="normal-case tracking-normal text-[10.5px] font-medium text-text-muted">
                  {fusionPanel.length} / 8
                </span>
              </SectionLabel>
              {fusionPanel.length === 0 ? (
                <DeferredNote>No chat agents in this workspace to fuse.</DeferredNote>
              ) : (
                <div className="rounded-xl ring-hair bg-surface p-2.5 space-y-1.5">
                  {fusionPanel.map((t) => (
                    <div key={t.instanceId} className="flex items-center gap-2">
                      <span
                        className="w-5 h-5 rounded-md text-white grid place-items-center text-[10px] font-bold shrink-0"
                        style={{ backgroundColor: t.color }}
                      >
                        {t.name[0]?.toUpperCase() ?? "A"}
                      </span>
                      <span className="text-[12px] font-medium truncate flex-1 min-w-0">
                        {t.name}
                      </span>
                    </div>
                  ))}
                </div>
              )}
            </div>

            {/* Judge — the orchestrator's own model (matches the M4.3 backend). */}
            <div>
              <SectionLabel>Judge</SectionLabel>
              <div className="rounded-xl ring-hair bg-surface px-2.5 py-2 flex items-center gap-2">
                <div className="w-6 h-6 rounded-md bg-agent-maestro grid place-items-center text-white shrink-0">
                  <Scale className="w-3.5 h-3.5" />
                </div>
                <div className="leading-tight flex-1 min-w-0">
                  <div className="text-[12px] font-medium truncate">{def.model ?? "—"}</div>
                  <div className="text-[10.5px] text-text-muted truncate">structured JSON</div>
                </div>
              </div>
            </div>

            {/* Est. cost — honest derived estimate (panel + judge + synth). */}
            <div>
              <SectionLabel>Est. cost</SectionLabel>
              <div className="rounded-xl ring-hair bg-surface px-2.5 py-2 flex items-center gap-2">
                <div className="w-6 h-6 rounded-md bg-accent grid place-items-center text-white shrink-0">
                  <Gauge className="w-3.5 h-3.5" />
                </div>
                <div className="leading-tight flex-1 min-w-0">
                  <div className="text-[12px] font-medium">~{fusionCostMult}×</div>
                  <div className="text-[10.5px] text-text-muted truncate">
                    panel runs in parallel; judge + synthesize add 2 stages
                  </div>
                </div>
              </div>
            </div>

            {/* Tuning — not wired into the M4.3 pipeline yet. */}
            <div>
              <SectionLabel>Tuning</SectionLabel>
              <DeferredNote>Panel/judge tuning isn't wired yet.</DeferredNote>
            </div>
          </>
        ) : (
          <>
            {/* Model · API — REAL (model + providerId), chat-focused */}
            {def.type === "chat" && (
              <div>
                <SectionLabel>Model · API</SectionLabel>
                {def.model ? (
                  <div className="rounded-xl ring-hair bg-surface px-2.5 py-2 flex items-center gap-2">
                    <div className="w-6 h-6 rounded-md bg-[#10a37f] grid place-items-center text-white shrink-0">
                      <Bot className="w-3.5 h-3.5" />
                    </div>
                    <div className="leading-tight flex-1 min-w-0">
                      <div className="text-[12px] font-medium truncate">
                        {def.model}
                        {def.role ? ` · ${def.role}` : ""}
                      </div>
                      <div className="text-[10.5px] text-text-muted truncate">
                        {def.providerId ? `provider · ${def.providerId}` : "—"}
                      </div>
                    </div>
                    {/* Static "configured" affordance — only because `model` is set. */}
                    <CheckCircle2 className="w-4 h-4 text-success shrink-0" />
                  </div>
                ) : (
                  <DeferredNote>No model configured</DeferredNote>
                )}
              </div>
            )}

            {/* Tools — DEFERRED to M5 (no tool join tables yet) */}
            <div>
              <SectionLabel>
                {def.type === "cli" ? "Tools · permissions" : "Tools · plugins"}
              </SectionLabel>
              <DeferredNote>
                <span className="flex items-center gap-1.5">
                  <Wrench className="w-3.5 h-3.5" />
                  Not connected yet — coming in M5
                </span>
              </DeferredNote>
            </div>

            {/* Skills — REAL (the def's effective launch set, annotated by
                `agentDef.list`; names resolved via the skill catalog). Only cli
                agents consume skills at launch, so chat gets an honest note. */}
            <div>
              <SectionLabel>
                <span>Skills</span>
                {def.type === "cli" && effectiveSkillIds.length > 0 && (
                  <span className="normal-case tracking-normal text-[10.5px] font-medium text-text-muted">
                    {effectiveSkillIds.length}
                  </span>
                )}
              </SectionLabel>
              {def.type !== "cli" ? (
                <DeferredNote>
                  <span className="flex items-center gap-1.5">
                    <Sparkles className="w-3.5 h-3.5" />
                    Skills are injected for CLI agents only
                  </span>
                </DeferredNote>
              ) : effectiveSkillIds.length === 0 ? (
                <DeferredNote>
                  <span className="flex items-center gap-1.5">
                    <Sparkles className="w-3.5 h-3.5" />
                    No skills attached
                  </span>
                </DeferredNote>
              ) : (
                <div className="rounded-xl ring-hair bg-surface p-2.5 space-y-1.5">
                  {effectiveSkillIds.map((id) => {
                    const sk = skillsById.get(id);
                    return (
                      <div key={id} className="flex items-center gap-2">
                        <Sparkles className="w-3.5 h-3.5 text-accent shrink-0" />
                        <span
                          className="text-[12px] font-medium truncate flex-1 min-w-0"
                          title={sk?.description ?? undefined}
                        >
                          {sk?.name ?? id}
                        </span>
                        {sk && (
                          <span className="text-[9.5px] text-text-tertiary uppercase tracking-wider shrink-0">
                            {sk.kind === "builtin"
                              ? sk.mandatory
                                ? "builtin · always"
                                : "builtin"
                              : "custom"}
                          </span>
                        )}
                      </div>
                    );
                  })}
                  {skillsStale && (
                    <div className="text-[10.5px] text-warning leading-snug pt-1.5 mt-0.5 border-t border-overlay/[0.06]">
                      Launched with a different skill set — Restart · resume applies the
                      current one.
                    </div>
                  )}
                </div>
              )}
            </div>

            {/* Session — restart · resume controls for a live CLI agent. Resume
                types the "read your last handoff" prompt into the terminal
                (non-destructive); Restart is the save-gated kill → respawn →
                resume loop, so it sits behind a confirm like Compact. */}
            {def.type === "cli" && session && (
              <div>
                <SectionLabel>Session</SectionLabel>
                <div className="rounded-xl ring-hair bg-surface p-2.5 space-y-1.5 text-[11.5px]">
                  {restartConfirming ? (
                    <>
                      <div className="text-[10.5px] text-text-secondary leading-snug">
                        The agent saves a handoff, its process is killed and relaunched,
                        then it resumes from that handoff. Continue?
                      </div>
                      <div className="flex items-center gap-3">
                        <button
                          onClick={doRestart}
                          className="text-[11px] font-medium text-accent hover:underline flex items-center gap-1"
                        >
                          <RotateCcw className="w-3 h-3" />
                          Restart
                        </button>
                        <button
                          onClick={() => setRestartConfirming(false)}
                          className="text-[11px] text-text-tertiary hover:text-text-secondary"
                        >
                          Cancel
                        </button>
                      </div>
                    </>
                  ) : restartPhase !== null ? (
                    <div className="text-[10.5px] text-text-secondary leading-snug flex items-start gap-1.5">
                      <RotateCcw className="w-3 h-3 animate-pulse shrink-0 mt-0.5" />
                      {restartPhase === "saving"
                        ? "Restarting — the agent is saving its handoff, then its process relaunches. Watch the terminal."
                        : "Restarting — respawning the process. Watch the terminal."}
                    </div>
                  ) : (
                    <div className="flex items-center gap-3">
                      <button
                        onClick={doResume}
                        disabled={!hasHandoff || resumeBusy || status !== "running"}
                        title={
                          hasHandoff
                            ? "Ask the agent to reload its last handoff and continue"
                            : "No handoff snapshot to resume from yet"
                        }
                        className="text-[11px] font-medium text-accent hover:underline disabled:opacity-40 disabled:hover:no-underline flex items-center gap-1"
                      >
                        <History className="w-3 h-3" />
                        {resumeBusy ? "Resuming…" : "Resume last handoff"}
                      </button>
                      <button
                        onClick={() => setRestartConfirming(true)}
                        title="Save a handoff, restart the process, resume from it"
                        className="ml-auto text-[11px] font-medium text-text-secondary hover:text-text-primary flex items-center gap-1"
                      >
                        <RotateCcw className="w-3 h-3" />
                        Restart · resume
                      </button>
                    </div>
                  )}
                  {sessionError && (
                    <div className="text-[10.5px] text-danger">{sessionError}</div>
                  )}
                </div>
              </div>
            )}

            {/* Memory · snapshots — REAL (M4.1 snapshot manager). Shown for both
                cli and chat agents, but only once a real session exists (we never
                fabricate rows). "Snapshot now" creates a manual snapshot; the list
                shows the newest few. Auto-compact snapshots appear live via
                `snapshot:created`. */}
            {session && (
              <div>
                <SectionLabel>
                  <span>Memory · snapshots</span>
                  {confirming ? (
                    // Yes/No gate — the compact loop CLEARS the agent, so confirm.
                    <span className="normal-case tracking-normal text-[10.5px] font-medium flex items-center gap-2">
                      <button
                        onClick={doCompact}
                        className="text-accent hover:underline flex items-center gap-1"
                      >
                        <Camera className="w-3 h-3" />
                        Compact
                      </button>
                      <button
                        onClick={() => setConfirming(false)}
                        className="text-text-tertiary hover:text-text-secondary"
                      >
                        Cancel
                      </button>
                    </span>
                  ) : compacting ? (
                    <span className="normal-case tracking-normal text-[10.5px] font-medium text-text-tertiary flex items-center gap-1">
                      <Camera className="w-3 h-3 animate-pulse" />
                      Compacting…
                    </span>
                  ) : (
                    <button
                      onClick={def.type === "cli" ? () => setConfirming(true) : doSnapshot}
                      disabled={snapshotBusy}
                      title={
                        def.type === "cli"
                          ? "Compact: save a handoff, clear the agent, then restore from it"
                          : "Create a manual snapshot"
                      }
                      className="normal-case tracking-normal text-[10.5px] font-medium text-accent hover:underline disabled:opacity-40 disabled:hover:no-underline flex items-center gap-1"
                    >
                      <Camera className="w-3 h-3" />
                      {snapshotBusy ? "Saving…" : def.type === "cli" ? "Compact now" : "Snapshot now"}
                    </button>
                  )}
                </SectionLabel>
                <div className="rounded-xl ring-hair bg-surface p-2.5 space-y-1 text-[11.5px]">
                  {confirming && (
                    <div className="text-[10.5px] text-text-secondary leading-snug pb-0.5">
                      The agent will summarize its work, then be cleared and resume from
                      that handoff. Continue?
                    </div>
                  )}
                  {compacting && (
                    <div className="text-[10.5px] text-text-secondary leading-snug pb-0.5">
                      Asking the agent to save its handoff, then clearing &amp; restoring —
                      watch its terminal.
                    </div>
                  )}
                  {snapshotError && (
                    <div className="text-[10.5px] text-danger">Couldn't compact this agent</div>
                  )}
                  {snapshots.length === 0 ? (
                    <div className="text-[10.5px] text-text-tertiary py-0.5">No snapshots yet</div>
                  ) : (
                    snapshots.map((s) => {
                      const isOpen = selectedSnap === s.id;
                      const busy = rowBusy === s.id;
                      const content =
                        typeof s.carriedForward === "string" ? s.carriedForward : null;
                      return (
                        <div key={s.id} className="rounded-md">
                          {/* Click a row to expand its saved content + actions. */}
                          <button
                            onClick={() =>
                              setSelectedSnap((cur) => (cur === s.id ? null : s.id))
                            }
                            className="w-full flex items-center gap-1.5 py-0.5 px-0.5 rounded-md hover:bg-overlay/[0.05] text-left"
                          >
                            {s.type === "auto" ? (
                              <Clock className="w-3 h-3 shrink-0 text-warning" />
                            ) : (
                              <Camera className="w-3 h-3 shrink-0 text-accent" />
                            )}
                            <span className="font-medium shrink-0">{s.type}</span>
                            {s.tokens != null && (
                              <span className="text-text-secondary truncate min-w-0">
                                ~{s.tokens.toLocaleString()} tok
                              </span>
                            )}
                            <span className="text-[10px] text-text-tertiary shrink-0 ml-auto">
                              {timeHint(s.createdAt)}
                            </span>
                            <ChevronDown
                              className={`w-3 h-3 shrink-0 text-text-tertiary transition-transform${
                                isOpen ? " rotate-180" : ""
                              }`}
                            />
                          </button>
                          {isOpen && (
                            <div className="px-0.5 pb-1 pt-0.5 space-y-1.5">
                              {/* View — the saved content (handoff text), or an honest
                                  note for a marker snapshot that has none. */}
                              <div className="text-[10.5px] leading-snug text-text-secondary whitespace-pre-wrap break-words max-h-40 overflow-y-auto scroll-thin rounded-md bg-overlay/[0.04] p-1.5">
                                {content ?? (
                                  <span className="italic text-text-tertiary">
                                    Marker only — no saved content.
                                  </span>
                                )}
                              </div>
                              <div className="flex items-center gap-3">
                                {def.type === "cli" && (
                                  <button
                                    onClick={() => doSendSnapshot(s.id)}
                                    disabled={busy}
                                    title="Type this snapshot's content into the agent's terminal"
                                    className="text-[10.5px] font-medium text-accent hover:underline disabled:opacity-40 flex items-center gap-1"
                                  >
                                    <Send className="w-3 h-3" />
                                    Send to agent
                                  </button>
                                )}
                                <button
                                  onClick={() => doDeleteSnapshot(s.id)}
                                  disabled={busy}
                                  className="text-[10.5px] font-medium text-danger hover:underline disabled:opacity-40 flex items-center gap-1 ml-auto"
                                >
                                  <Trash2 className="w-3 h-3" />
                                  Delete
                                </button>
                              </div>
                            </div>
                          )}
                        </div>
                      );
                    })
                  )}
                </div>
              </div>
            )}

            {/* Messages — routing POLICY (REAL config from AgentDefinition) + a
                REAL recent inbox/outbox log from `message.list`. */}
            <div>
              <SectionLabel>Messages</SectionLabel>
              <div className="rounded-xl ring-hair bg-surface p-2.5 space-y-1.5 text-[11.5px]">
                <div className="flex items-center justify-between">
                  <span className="text-text-secondary flex items-center gap-1.5">
                    <Inbox className="w-3.5 h-3.5" />
                    Accepts from
                  </span>
                  <span className="font-medium">{allowedSendersLabel(def.allowedSenders)}</span>
                </div>
                <div className="flex items-center justify-between">
                  <span className="text-text-secondary flex items-center gap-1.5">
                    <Send className="w-3.5 h-3.5" />
                    Auto-submit on inject
                  </span>
                  <span className="font-medium">{def.autoSubmitInjected ? "On" : "Off"}</span>
                </div>

                {/* Recent inbox/outbox — newest first. Honest empty state. */}
                <div className="pt-1.5 mt-0.5 border-t border-overlay/[0.06] space-y-1">
                  {messages.length === 0 ? (
                    <div className="text-[10.5px] text-text-tertiary py-0.5">No messages yet</div>
                  ) : (
                    messages.map((m) => {
                      const inbound = m.toInstanceId === instanceId;
                      const counterpart = inbound ? m.fromInstanceId : m.toInstanceId;
                      return (
                        <div key={m.id} className="flex items-center gap-1.5">
                          {inbound ? (
                            <CornerDownLeft className="w-3 h-3 shrink-0 text-success" />
                          ) : (
                            <CornerUpRight className="w-3 h-3 shrink-0 text-accent" />
                          )}
                          <span className="text-text-secondary shrink-0">
                            {inbound ? "from" : "to"}
                          </span>
                          <span className="font-medium truncate flex-1 min-w-0">
                            {nameOf(counterpart)}
                          </span>
                          {m.status === "queued" && (
                            <span className="text-[10px] text-warning shrink-0">queued</span>
                          )}
                          <span className="text-[10px] text-text-tertiary shrink-0">
                            {timeHint(m.createdAt)}
                          </span>
                        </div>
                      );
                    })
                  )}
                </div>
              </div>
            </div>
          </>
        )}

        {/* Context meter — LIVE from `session:context` estimates (falls back to
            the session prop). HONESTY: the count is an ESTIMATE (streamed-byte
            derived, ≈4 chars/token), NOT exact provider usage — labelled below.
            Rendered only when a real session reports a usable limit. */}
        {showMeter && meterTokens != null && meterLimit != null && (
          <div>
            <SectionLabel>Context</SectionLabel>
            <div className="rounded-xl ring-hair bg-surface p-2.5 space-y-1.5 text-[11.5px]">
              <div className="flex items-center justify-between text-text-secondary">
                <span className="flex items-center gap-1">
                  tokens
                  {meterEstimated && (
                    <span className="text-[9.5px] text-text-tertiary font-normal">estimate</span>
                  )}
                </span>
                <span className="font-mono">
                  {meterTokens.toLocaleString()} / {meterLimit.toLocaleString()}
                </span>
              </div>
              <div className="h-1.5 rounded-full bg-overlay/[0.06] overflow-hidden">
                <div
                  className="h-full bg-accent"
                  style={{
                    width: `${Math.min(100, Math.round((meterTokens / meterLimit) * 100))}%`,
                  }}
                />
              </div>
            </div>
          </div>
        )}
      </div>

      {/* Footer — agent identity (real: name/color/status). */}
      <div className="border-t border-overlay/[0.06] px-3 py-2 shrink-0 flex items-center gap-2">
        <div
          className="w-6 h-6 rounded-[7px] text-white grid place-items-center text-[11px] font-bold ring-hair shrink-0"
          style={{ backgroundColor: def.color ?? "#6e6e73" }}
        >
          {def.type === "orchestrator" ? (
            <Waypoints className="w-[14px] h-[14px]" />
          ) : (
            (def.name[0]?.toUpperCase() ?? "A")
          )}
        </div>
        <div className="leading-tight flex-1 min-w-0">
          <div className="text-[12px] font-semibold truncate">{def.name}</div>
          <div className="text-[10.5px] text-text-muted truncate">{status}</div>
        </div>
      </div>
      </aside>
  );
}
