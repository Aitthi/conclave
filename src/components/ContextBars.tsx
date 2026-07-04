import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Bot,
  CheckCircle2,
  ChevronDown,
  Gauge,
  History,
  RotateCcw,
  Scale,
  Sparkles,
} from "lucide-react";
import { ipc } from "../ipc";
import type { AgentDefinition, Role, Session, Skill, WorkspaceAgent } from "../ipc";
import type { RoutingTarget } from "./RoutingPicker";
import { computeSkillsStale } from "../lib/skills";
import { DeferredNote } from "./DeferredNote";

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
