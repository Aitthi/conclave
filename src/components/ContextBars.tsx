import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Bot,
  Camera,
  CheckCircle2,
  ChevronDown,
  Clock,
  Gauge,
  History,
  RotateCcw,
  Scale,
  Send,
  Sparkles,
  Trash2,
  Zap,
} from "lucide-react";
import { ipc, useSessionContext } from "../ipc";
import type { AgentDefinition, Role, Session, Skill, WorkspaceAgent } from "../ipc";
import type { RoutingTarget } from "./RoutingPicker";
import { computeSkillsStale } from "../lib/skills";
import { timeHint } from "../lib/timeHint";
import { DeferredNote } from "./DeferredNote";
import type { SessionSnapshots } from "../lib/useSessionSnapshots";

// ---------------------------------------------------------------------------
// Center-pane top/bottom context bars — slim, click-to-open popovers that
// replace the old ContextDrawer's always-open sections (design: bb design:
// right-rail-chat; plan Task 5/6). Content + guards are MOVED from
// ContextDrawer.tsx, not rewritten (risk ledger: 997 lines of guarded code).
// ---------------------------------------------------------------------------

interface ContextTopBarProps {
  def: AgentDefinition;
  status: WorkspaceAgent["status"];
  instanceId: string;
  /** All routable agents — only consumed by the orchestrator Fusion config
   *  (panel = the workspace's chat agents excluding self). */
  roster: RoutingTarget[];
  session: Session | null;
  /** Gates Resume — lifted from `useSessionSnapshots` (plan-review F2), a
   *  single shared fetcher `WorkspacePane` holds for the active session. */
  hasHandoff: boolean;
  launchedSkillIds?: string[];
}

type TopPopover = "skills" | "config" | null;

export function ContextTopBar({
  def,
  status,
  instanceId,
  roster,
  session,
  hasHandoff,
  launchedSkillIds,
}: ContextTopBarProps) {
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  // ── Skills (REAL — the agent's effective skill set), moved verbatim from
  //    ContextDrawer.tsx. ────────────────────────────────────────────────────
  const [skillCatalog, setSkillCatalog] = useState<Skill[] | null>(null);
  useEffect(() => {
    ipc.skill
      .list()
      .then((rows) => {
        if (mounted.current) setSkillCatalog(rows);
      })
      .catch((err: unknown) => {
        if (import.meta.env.DEV) {
          console.error("ContextTopBar: skill.list failed", err);
        }
      });
  }, []);
  const skillsById = useMemo(
    () => new Map((skillCatalog ?? []).map((s) => [s.id, s])),
    [skillCatalog],
  );

  // ── Role (ADR 0005), moved verbatim. ───────────────────────────────────────
  const [roleCatalog, setRoleCatalog] = useState<Role[] | null>(null);
  useEffect(() => {
    ipc.role
      .list()
      .then((rows) => {
        if (mounted.current) setRoleCatalog(rows);
      })
      .catch((err: unknown) => {
        if (import.meta.env.DEV) {
          console.error("ContextTopBar: role.list failed", err);
        }
      });
  }, []);
  const resolvedRole = def.roleId
    ? (roleCatalog ?? []).find((r) => r.id === def.roleId)
    : undefined;
  const roleName = resolvedRole?.name ?? def.role;
  const roleDescription = resolvedRole?.description;

  const effectiveSkillIds = def.skillIds ?? [];
  const skillsStale = computeSkillsStale(def, launchedSkillIds);

  // ── Session (restart · resume), moved verbatim. ────────────────────────────
  const [restartConfirming, setRestartConfirming] = useState(false);
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
          console.error("ContextTopBar: instance.restart failed", err);
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
          console.error("ContextTopBar: snapshot.resume failed", err);
        }
        if (mounted.current) setSessionError("Couldn't resume this agent");
      })
      .finally(() => {
        setTimeout(() => {
          if (mounted.current) setResumeBusy(false);
        }, 3_000);
      });
  }, [instanceId]);

  useEffect(() => {
    if (restartPhase === "saving" && status === "idle") setRestartPhase("respawning");
    else if (restartPhase === "respawning" && status === "running") setRestartPhase(null);
  }, [status, restartPhase]);

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

  // ── Fusion panel (orchestrator Config popover), moved verbatim. ───────────
  const fusionPanel = useMemo(
    () => roster.filter((t) => t.type === "chat" && t.instanceId !== instanceId).slice(0, 8),
    [roster, instanceId],
  );
  const fusionCostMult = fusionPanel.length + 2;

  // ── Popovers: mutually exclusive, close on outside click. ─────────────────
  const [openPopover, setOpenPopover] = useState<TopPopover>(null);
  const barRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!openPopover) return;
    const onDocClick = (e: MouseEvent) => {
      if (barRef.current && !barRef.current.contains(e.target as Node)) setOpenPopover(null);
    };
    document.addEventListener("mousedown", onDocClick);
    return () => document.removeEventListener("mousedown", onDocClick);
  }, [openPopover]);

  const showConfig = def.type === "chat" || def.type === "orchestrator";
  const showSession = def.type === "cli" && session != null;

  return (
    <div
      ref={barRef}
      className="relative h-9 shrink-0 border-b border-overlay/[0.06] flex items-center gap-1.5 px-3"
    >
      {/* Skills trigger. */}
      <button
        onClick={() => setOpenPopover((v) => (v === "skills" ? null : "skills"))}
        className={`flex items-center gap-1.5 px-2 h-6 rounded-md text-[11.5px] font-medium transition-colors ${
          openPopover === "skills"
            ? "bg-overlay/[0.07] text-text-primary"
            : "text-text-secondary hover:bg-overlay/[0.04]"
        }`}
      >
        <Sparkles className="w-[13px] h-[13px] text-accent" />
        Skills
        {def.type === "cli" && effectiveSkillIds.length > 0 && (
          <span className="text-[10px] text-text-tertiary">{effectiveSkillIds.length}</span>
        )}
        <ChevronDown
          className={`w-3 h-3 text-text-tertiary transition-transform${
            openPopover === "skills" ? " rotate-180" : ""
          }`}
        />
      </button>

      <span className="w-px h-3.5 bg-overlay/[0.1]" />

      {/* Session — resume/restart icon buttons (cli only, live session). */}
      {showSession && (
        <>
          <button
            onClick={doResume}
            disabled={!hasHandoff || resumeBusy || status !== "running"}
            title={
              hasHandoff
                ? "Ask the agent to reload its last handoff and continue"
                : "No handoff snapshot to resume from yet"
            }
            className="w-6 h-6 grid place-items-center rounded-md hover:bg-overlay/[0.05] text-text-secondary disabled:opacity-40"
          >
            <History className="w-[14px] h-[14px]" />
          </button>
          <button
            onClick={() => setRestartConfirming((v) => !v)}
            disabled={restartPhase !== null}
            title="Save a handoff, restart the process, resume from it"
            className={`w-6 h-6 grid place-items-center rounded-md hover:bg-overlay/[0.05] text-text-secondary disabled:opacity-40${
              restartPhase !== null ? " animate-pulse" : ""
            }`}
          >
            <RotateCcw className="w-[14px] h-[14px]" />
          </button>
          {(restartConfirming || restartPhase !== null || sessionError) && (
            <div className="absolute top-full left-3 mt-1 z-10 w-64 rounded-lg ring-hair bg-surface shadow-lg p-2.5 text-[10.5px] text-text-secondary leading-snug space-y-1.5">
              {restartConfirming ? (
                <>
                  <div>
                    The agent saves a handoff, its process is killed and relaunched, then it
                    resumes from that handoff. Continue?
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
                <div className="flex items-start gap-1.5">
                  <RotateCcw className="w-3 h-3 animate-pulse shrink-0 mt-0.5" />
                  {restartPhase === "saving"
                    ? "Restarting — the agent is saving its handoff, then its process relaunches. Watch the terminal."
                    : "Restarting — respawning the process. Watch the terminal."}
                </div>
              ) : null}
              {sessionError && <div className="text-danger">{sessionError}</div>}
            </div>
          )}
        </>
      )}

      {/* Config popover (R5) — Model·API (chat) or Fusion (orchestrator). */}
      {showConfig && (
        <button
          onClick={() => setOpenPopover((v) => (v === "config" ? null : "config"))}
          className={`ml-auto flex items-center gap-1.5 px-2 h-6 rounded-md text-[11.5px] font-medium transition-colors ${
            openPopover === "config"
              ? "bg-overlay/[0.07] text-text-primary"
              : "text-text-secondary hover:bg-overlay/[0.04]"
          }`}
        >
          Config
          <ChevronDown
            className={`w-3 h-3 text-text-tertiary transition-transform${
              openPopover === "config" ? " rotate-180" : ""
            }`}
          />
        </button>
      )}

      {/* Skills popover content. */}
      {openPopover === "skills" && (
        <div className="absolute top-full left-3 mt-1 z-10 w-72 rounded-lg ring-hair bg-surface shadow-lg p-2.5 space-y-2 text-[11.5px]">
          {roleName && (
            <div className="pb-1.5 border-b border-overlay/[0.06]">
              <span
                className="inline-flex items-center px-1.5 py-0.5 rounded-full bg-overlay/[0.06] text-[10.5px] font-medium truncate max-w-full"
                title={roleDescription}
              >
                {roleName}
              </span>
            </div>
          )}
          <div className="flex items-center justify-between text-[10px] font-bold tracking-wider text-text-tertiary uppercase">
            <span>Skills</span>
            <span>{effectiveSkillIds.length}</span>
          </div>
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
            <div className="space-y-1.5 max-h-64 overflow-y-auto scroll-thin">
              {effectiveSkillIds.map((id) => {
                const sk = skillsById.get(id);
                return (
                  <div key={id} className="flex items-center gap-2">
                    <Sparkles className="w-3.5 h-3.5 text-accent shrink-0" />
                    <span
                      className="font-medium truncate flex-1 min-w-0"
                      title={sk?.description ?? undefined}
                    >
                      {sk?.name ?? id}
                    </span>
                    {sk && (
                      <span className="text-[9.5px] text-text-tertiary uppercase tracking-wider shrink-0">
                        {sk.kind === "builtin" ? (sk.mandatory ? "builtin · always" : "builtin") : "custom"}
                      </span>
                    )}
                  </div>
                );
              })}
              {skillsStale && (
                <div className="text-[10.5px] text-warning leading-snug pt-1.5 mt-0.5 border-t border-overlay/[0.06]">
                  Launched with a different skill set — Restart · resume applies the current one.
                </div>
              )}
            </div>
          )}
        </div>
      )}

      {/* Config popover content. */}
      {openPopover === "config" && (
        <div className="absolute top-full right-3 mt-1 z-10 w-72 rounded-lg ring-hair bg-surface shadow-lg p-2.5 space-y-3 text-[11.5px]">
          {def.type === "chat" ? (
            <div>
              <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
                Model · API
              </div>
              {def.model ? (
                <div className="flex items-center gap-2">
                  <div className="w-6 h-6 rounded-md bg-[#10a37f] grid place-items-center text-white shrink-0">
                    <Bot className="w-3.5 h-3.5" />
                  </div>
                  <div className="leading-tight flex-1 min-w-0">
                    <div className="font-medium truncate">
                      {def.model}
                      {def.role ? ` · ${def.role}` : ""}
                    </div>
                    <div className="text-[10.5px] text-text-muted truncate">
                      {def.providerId ? `provider · ${def.providerId}` : "—"}
                    </div>
                  </div>
                  <CheckCircle2 className="w-4 h-4 text-success shrink-0" />
                </div>
              ) : (
                <DeferredNote>No model configured</DeferredNote>
              )}
            </div>
          ) : (
            <>
              <div>
                <div className="flex items-center justify-between text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
                  <span>Panel agents</span>
                  <span className="normal-case tracking-normal font-medium text-text-muted">
                    {fusionPanel.length} / 8
                  </span>
                </div>
                {fusionPanel.length === 0 ? (
                  <DeferredNote>No chat agents in this workspace to fuse.</DeferredNote>
                ) : (
                  <div className="space-y-1.5 max-h-40 overflow-y-auto scroll-thin">
                    {fusionPanel.map((t) => (
                      <div key={t.instanceId} className="flex items-center gap-2">
                        <span
                          className="w-5 h-5 rounded-md text-white grid place-items-center text-[10px] font-bold shrink-0"
                          style={{ backgroundColor: t.color }}
                        >
                          {t.name[0]?.toUpperCase() ?? "A"}
                        </span>
                        <span className="font-medium truncate flex-1 min-w-0">{t.name}</span>
                      </div>
                    ))}
                  </div>
                )}
              </div>
              <div>
                <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
                  Judge
                </div>
                <div className="flex items-center gap-2">
                  <div className="w-6 h-6 rounded-md bg-agent-maestro grid place-items-center text-white shrink-0">
                    <Scale className="w-3.5 h-3.5" />
                  </div>
                  <div className="leading-tight flex-1 min-w-0">
                    <div className="font-medium truncate">{def.model ?? "—"}</div>
                    <div className="text-[10.5px] text-text-muted truncate">structured JSON</div>
                  </div>
                </div>
              </div>
              <div>
                <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
                  Est. cost
                </div>
                <div className="flex items-center gap-2">
                  <div className="w-6 h-6 rounded-md bg-accent grid place-items-center text-white shrink-0">
                    <Gauge className="w-3.5 h-3.5" />
                  </div>
                  <div className="leading-tight flex-1 min-w-0">
                    <div className="font-medium">~{fusionCostMult}×</div>
                    <div className="text-[10.5px] text-text-muted truncate">
                      panel runs in parallel; judge + synthesize add 2 stages
                    </div>
                  </div>
                </div>
              </div>
              <div>
                <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
                  Tuning
                </div>
                <DeferredNote>Panel/judge tuning isn't wired yet.</DeferredNote>
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}

interface ContextBottomBarProps {
  def: AgentDefinition;
  instanceId: string;
  session: Session | null;
  /** The SAME `useSessionSnapshots` instance ContextTopBar's Resume gate
   *  reads (plan-review F2) — this bar must not create a second fetcher. */
  snapshots: SessionSnapshots;
}

type BottomPopover = "snapshots" | null;

export function ContextBottomBar({ def, instanceId, session, snapshots }: ContextBottomBarProps) {
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const sessionId = session?.id ?? "";

  // ── Snapshot actions (Memory section), moved verbatim — local UI state only;
  //    the fetched list/hasHandoff/error live in the shared `snapshots` prop. ─
  const [snapshotBusy, setSnapshotBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [compacting, setCompacting] = useState(false);
  const [selectedSnap, setSelectedSnap] = useState<string | null>(null);
  const [rowBusy, setRowBusy] = useState<string | null>(null);

  // Reset transient UI on a session switch — mirrors the original drawer's
  // combined reset effect for this slice of state.
  useEffect(() => {
    setSnapshotBusy(false);
    setConfirming(false);
    setCompacting(false);
    setSelectedSnap(null);
    setRowBusy(null);
  }, [sessionId]);

  const doSnapshot = useCallback(() => {
    if (!sessionId) return;
    setSnapshotBusy(true);
    snapshots.setSnapshotError(false);
    ipc.snapshot
      .create({ sessionId, type: "manual" })
      .then(() => {
        if (mounted.current) snapshots.refetch();
      })
      .catch((err: unknown) => {
        if (import.meta.env.DEV) {
          console.error("ContextBottomBar: snapshot.create failed", err);
        }
        if (mounted.current) snapshots.setSnapshotError(true);
      })
      .finally(() => {
        if (mounted.current) setSnapshotBusy(false);
      });
  }, [sessionId, snapshots]);

  const doCompact = useCallback(() => {
    setConfirming(false);
    snapshots.setSnapshotError(false);
    setCompacting(true);
    ipc.snapshot.compact({ instanceId }).catch((err: unknown) => {
      if (import.meta.env.DEV) {
        console.error("ContextBottomBar: snapshot.compact failed", err);
      }
      if (mounted.current) {
        snapshots.setSnapshotError(true);
        setCompacting(false);
      }
    });
  }, [instanceId, snapshots]);

  // Failsafe: if the agent never writes its handoff, no `snapshot:created`
  // ever fires and "Compacting…" would pulse forever.
  useEffect(() => {
    if (!compacting) return;
    const t = setTimeout(() => {
      if (mounted.current) {
        setCompacting(false);
        snapshots.setSnapshotError(true);
      }
    }, 120_000);
    return () => clearTimeout(t);
  }, [compacting, snapshots]);

  const doDeleteSnapshot = useCallback(
    (snapshotId: string) => {
      setRowBusy(snapshotId);
      snapshots.setSnapshotError(false);
      ipc.snapshot
        .delete({ snapshotId })
        .then(() => {
          if (!mounted.current) return;
          setSelectedSnap((cur) => (cur === snapshotId ? null : cur));
          snapshots.refetch();
        })
        .catch((err: unknown) => {
          if (import.meta.env.DEV) {
            console.error("ContextBottomBar: snapshot.delete failed", err);
          }
          if (mounted.current) snapshots.setSnapshotError(true);
        })
        .finally(() => {
          if (mounted.current) setRowBusy(null);
        });
    },
    [snapshots],
  );

  const doSendSnapshot = useCallback(
    (snapshotId: string) => {
      setRowBusy(snapshotId);
      snapshots.setSnapshotError(false);
      ipc.snapshot
        .send({ instanceId, snapshotId })
        .catch((err: unknown) => {
          if (import.meta.env.DEV) {
            console.error("ContextBottomBar: snapshot.send failed", err);
          }
          if (mounted.current) snapshots.setSnapshotError(true);
        })
        .finally(() => {
          if (mounted.current) setRowBusy(null);
        });
    },
    [instanceId, snapshots],
  );

  // ── Context meter (R4 compact chip), moved verbatim. ───────────────────────
  const [ctx, setCtx] = useState<{ tokens: number; limit: number; estimated: boolean } | null>(
    session?.contextTokens != null && session?.contextLimit != null
      ? { tokens: session.contextTokens, limit: session.contextLimit, estimated: true }
      : null,
  );
  useSessionContext(sessionId, (e) =>
    setCtx({ tokens: e.contextTokens, limit: e.contextLimit, estimated: e.estimated }),
  );
  useEffect(() => {
    setCtx(
      session?.contextTokens != null && session?.contextLimit != null
        ? { tokens: session.contextTokens, limit: session.contextLimit, estimated: true }
        : null,
    );
    // `session` is intentionally read but excluded from deps — re-seed only on
    // identity change of the session id, matching the original drawer.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId]);

  const meterTokens = ctx?.tokens ?? session?.contextTokens ?? null;
  const meterLimit = ctx?.limit ?? session?.contextLimit ?? null;
  const meterEstimated = ctx?.estimated ?? true;
  const showMeter =
    def.type === "chat" && session != null && meterTokens != null && meterLimit != null && meterLimit > 0;

  // ── Popover (mutual exclusion is trivial here — only one panel exists). ───
  const [openPopover, setOpenPopover] = useState<BottomPopover>(null);
  const barRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!openPopover) return;
    const onDocClick = (e: MouseEvent) => {
      if (barRef.current && !barRef.current.contains(e.target as Node)) setOpenPopover(null);
    };
    document.addEventListener("mousedown", onDocClick);
    return () => document.removeEventListener("mousedown", onDocClick);
  }, [openPopover]);

  if (!session) return null;

  const lastHandoff = snapshots.snapshots.find((s) => s.type === "handoff");

  const onCompactClick = () => {
    if (def.type === "cli") {
      if (!confirming && !compacting) {
        setConfirming(true);
        setOpenPopover("snapshots");
      }
    } else {
      doSnapshot();
    }
  };

  return (
    <div
      ref={barRef}
      className="relative h-9 shrink-0 border-t border-overlay/[0.06] flex items-center gap-1.5 px-3"
    >
      {/* Snapshots trigger. */}
      <button
        onClick={() => setOpenPopover((v) => (v === "snapshots" ? null : "snapshots"))}
        className={`flex items-center gap-1.5 px-2 h-6 rounded-md text-[11.5px] font-medium transition-colors shrink-0 ${
          openPopover === "snapshots"
            ? "bg-overlay/[0.07] text-text-primary"
            : "text-text-secondary hover:bg-overlay/[0.04]"
        }`}
      >
        <Camera className="w-[13px] h-[13px] text-text-tertiary" />
        Snapshots
        <span className="text-[10px] text-text-tertiary">{snapshots.snapshots.length}</span>
        <ChevronDown
          className={`w-3 h-3 text-text-tertiary transition-transform${
            openPopover === "snapshots" ? " rotate-180" : ""
          }`}
        />
      </button>

      {/* Inline summary — honest: only when a real handoff exists. */}
      <span className="text-[10.5px] text-text-tertiary truncate min-w-0">
        {lastHandoff
          ? `last handoff ${lastHandoff.tokens != null ? `~${lastHandoff.tokens.toLocaleString()} tok · ` : ""}${timeHint(lastHandoff.createdAt)}`
          : "No handoff yet"}
      </span>

      <span className="w-px h-3.5 bg-overlay/[0.1] shrink-0" />

      <button
        onClick={onCompactClick}
        disabled={snapshotBusy || compacting}
        title={def.type === "cli" ? "Compact: save a handoff, clear the agent, then restore from it" : "Create a manual snapshot"}
        className={`w-6 h-6 grid place-items-center rounded-md hover:bg-overlay/[0.05] text-accent disabled:opacity-40 shrink-0${
          compacting ? " animate-pulse" : ""
        }`}
      >
        <Zap className="w-[14px] h-[14px]" />
      </button>

      {/* Context meter — compact chip (R4), honest-labelled estimate. */}
      {showMeter && meterTokens != null && meterLimit != null && (
        <span
          className="ml-auto flex items-center gap-1.5 shrink-0 text-[10px] text-text-tertiary"
          title={`${meterTokens.toLocaleString()} / ${meterLimit.toLocaleString()} tokens${meterEstimated ? " (estimate)" : ""}`}
        >
          <span className="w-10 h-1 rounded-full bg-overlay/[0.08] overflow-hidden">
            <span
              className="block h-full bg-accent"
              style={{ width: `${Math.min(100, Math.round((meterTokens / meterLimit) * 100))}%` }}
            />
          </span>
          <span className="font-mono">{Math.round((meterTokens / meterLimit) * 100)}%</span>
        </span>
      )}

      {/* Snapshots popover — opens UPWARD (bottom bar). */}
      {openPopover === "snapshots" && (
        <div className="absolute bottom-full left-3 mb-1 z-10 w-80 rounded-lg ring-hair bg-surface shadow-lg p-2.5 space-y-2 text-[11.5px]">
          <div className="flex items-center justify-between text-[10px] font-bold tracking-wider text-text-tertiary uppercase">
            <span>Memory · snapshots</span>
            <span>{snapshots.snapshots.length}</span>
          </div>

          {confirming && (
            <div className="text-[10.5px] text-text-secondary leading-snug pb-0.5 space-y-1">
              <div>
                The agent will summarize its work, then be cleared and resume from that handoff.
                Continue?
              </div>
              <div className="flex items-center gap-3">
                <button
                  onClick={doCompact}
                  className="text-[11px] font-medium text-accent hover:underline flex items-center gap-1"
                >
                  <Camera className="w-3 h-3" />
                  Compact
                </button>
                <button
                  onClick={() => setConfirming(false)}
                  className="text-[11px] text-text-tertiary hover:text-text-secondary"
                >
                  Cancel
                </button>
              </div>
            </div>
          )}
          {compacting && (
            <div className="text-[10.5px] text-text-secondary leading-snug pb-0.5">
              Asking the agent to save its handoff, then clearing &amp; restoring — watch its
              terminal.
            </div>
          )}
          {snapshots.snapshotError && (
            <div className="text-[10.5px] text-danger">Couldn't compact this agent</div>
          )}

          {snapshots.snapshots.length === 0 ? (
            <div className="text-[10.5px] text-text-tertiary py-0.5">No snapshots yet</div>
          ) : (
            <div className="space-y-1 max-h-72 overflow-y-auto scroll-thin">
              {snapshots.snapshots.map((s) => {
                const isOpen = selectedSnap === s.id;
                const busy = rowBusy === s.id;
                const content = typeof s.carriedForward === "string" ? s.carriedForward : null;
                return (
                  <div key={s.id} className="rounded-md">
                    <button
                      onClick={() => setSelectedSnap((cur) => (cur === s.id ? null : s.id))}
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
              })}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
