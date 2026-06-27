import { useCallback, useEffect, useRef, useState } from "react";
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
} from "lucide-react";
import { ipc, useMessageInjected, useSessionContext, useSnapshotCreated } from "../ipc";
import type {
  AgentDefinition,
  InterAgentMessage,
  Session,
  Snapshot,
  WorkspaceAgent,
} from "../ipc";
import type { RoutingTarget } from "./RoutingPicker";
import { timeHint } from "../lib/timeHint";

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
    <div className="text-[10px] font-bold tracking-wider text-[#a1a1a6] uppercase mb-1.5 px-0.5 flex items-center justify-between">
      {children}
    </div>
  );
}

// Honest empty/deferred state — a muted line naming the milestone.
function DeferredNote({ children }: { children: React.ReactNode }) {
  return (
    <div className="rounded-xl ring-hair bg-white px-2.5 py-2 text-[11.5px] text-[#a1a1a6]">
      {children}
    </div>
  );
}

export function ContextDrawer({ def, status, instanceId, roster, session }: ContextDrawerProps) {
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
  const [snapshotError, setSnapshotError] = useState(false);
  const [snapshotBusy, setSnapshotBusy] = useState(false);
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
  useSnapshotCreated(sessionId, () => refetchSnapshots());

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

  // Resolve a counterpart instance id → display name via the roster.
  const nameOf = (id: string): string =>
    roster.find((t) => t.instanceId === id)?.name ?? id;

  if (!open) {
    return (
      <aside className="w-9 vibrancy border-l border-black/[0.06] flex flex-col items-center shrink-0">
        <button
          title="Show Context"
          onClick={() => setOpen(true)}
          className="w-7 h-7 mt-2.5 grid place-items-center rounded-md hover:bg-black/[0.05] text-[#6e6e73]"
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
  const showMeter = session != null && meterTokens != null && meterLimit != null && meterLimit > 0;

  return (
    <aside className="w-[306px] vibrancy border-l border-black/[0.06] flex flex-col shrink-0">
      {/* Header */}
      <div className="h-12 flex items-center justify-between px-4 border-b border-black/[0.06] shrink-0">
        <span className="text-[12px] font-semibold text-[#6e6e73] tracking-tight">Context</span>
        <button
          title="Hide Context"
          onClick={() => setOpen(false)}
          className="w-7 h-7 grid place-items-center rounded-md hover:bg-black/[0.05] text-[#6e6e73]"
        >
          <PanelRight className="w-[15px] h-[15px]" />
        </button>
      </div>

      <div className="flex-1 overflow-y-auto scroll-thin p-3 space-y-4">
        {def.type === "orchestrator" ? (
          // Orchestrator (Fusion) config is M4 — no honest source yet.
          <div>
            <SectionLabel>Fusion</SectionLabel>
            <DeferredNote>Fusion settings coming in M4</DeferredNote>
          </div>
        ) : (
          <>
            {/* Model · API — REAL (model + providerId), chat-focused */}
            {def.type === "chat" && (
              <div>
                <SectionLabel>Model · API</SectionLabel>
                {def.model ? (
                  <div className="rounded-xl ring-hair bg-white px-2.5 py-2 flex items-center gap-2">
                    <div className="w-6 h-6 rounded-md bg-[#10a37f] grid place-items-center text-white shrink-0">
                      <Bot className="w-3.5 h-3.5" />
                    </div>
                    <div className="leading-tight flex-1 min-w-0">
                      <div className="text-[12px] font-medium truncate">
                        {def.model}
                        {def.role ? ` · ${def.role}` : ""}
                      </div>
                      <div className="text-[10.5px] text-[#86868b] truncate">
                        {def.providerId ? `provider · ${def.providerId}` : "—"}
                      </div>
                    </div>
                    {/* Static "configured" affordance — only because `model` is set. */}
                    <CheckCircle2 className="w-4 h-4 text-[#30a14e] shrink-0" />
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

            {/* Skills — DEFERRED to M5 (no skill join tables yet) */}
            <div>
              <SectionLabel>Skills</SectionLabel>
              <DeferredNote>
                <span className="flex items-center gap-1.5">
                  <Sparkles className="w-3.5 h-3.5" />
                  Not configured yet — coming in M5
                </span>
              </DeferredNote>
            </div>

            {/* Memory · snapshots — REAL (M4.1 snapshot manager). Shown for both
                cli and chat agents, but only once a real session exists (we never
                fabricate rows). "Snapshot now" creates a manual snapshot; the list
                shows the newest few. Auto-compact snapshots appear live via
                `snapshot:created`. */}
            {session && (
              <div>
                <SectionLabel>
                  <span>Memory · snapshots</span>
                  <button
                    onClick={doSnapshot}
                    disabled={snapshotBusy}
                    title="Create a manual snapshot"
                    className="normal-case tracking-normal text-[10.5px] font-medium text-[#0a84ff] hover:underline disabled:opacity-40 disabled:hover:no-underline flex items-center gap-1"
                  >
                    <Camera className="w-3 h-3" />
                    {snapshotBusy ? "Saving…" : "Snapshot now"}
                  </button>
                </SectionLabel>
                <div className="rounded-xl ring-hair bg-white p-2.5 space-y-1 text-[11.5px]">
                  {snapshotError && (
                    <div className="text-[10.5px] text-[#ff3b30]">Couldn't save snapshot</div>
                  )}
                  {snapshots.length === 0 ? (
                    <div className="text-[10.5px] text-[#a1a1a6] py-0.5">No snapshots yet</div>
                  ) : (
                    snapshots.map((s) => (
                      <div key={s.id} className="flex items-center gap-1.5">
                        {s.type === "auto" ? (
                          <Clock className="w-3 h-3 shrink-0 text-[#ff9f0a]" />
                        ) : (
                          <Camera className="w-3 h-3 shrink-0 text-[#0a84ff]" />
                        )}
                        <span className="font-medium shrink-0">{s.type}</span>
                        {s.tokens != null && (
                          <span className="text-[#6e6e73] truncate flex-1 min-w-0">
                            ~{s.tokens.toLocaleString()} tok
                          </span>
                        )}
                        <span className="text-[10px] text-[#a1a1a6] shrink-0 ml-auto">
                          {timeHint(s.createdAt)}
                        </span>
                      </div>
                    ))
                  )}
                </div>
              </div>
            )}

            {/* Messages — routing POLICY (REAL config from AgentDefinition) + a
                REAL recent inbox/outbox log from `message.list`. */}
            <div>
              <SectionLabel>Messages</SectionLabel>
              <div className="rounded-xl ring-hair bg-white p-2.5 space-y-1.5 text-[11.5px]">
                <div className="flex items-center justify-between">
                  <span className="text-[#6e6e73] flex items-center gap-1.5">
                    <Inbox className="w-3.5 h-3.5" />
                    Accepts from
                  </span>
                  <span className="font-medium">{allowedSendersLabel(def.allowedSenders)}</span>
                </div>
                <div className="flex items-center justify-between">
                  <span className="text-[#6e6e73] flex items-center gap-1.5">
                    <Send className="w-3.5 h-3.5" />
                    Auto-submit on inject
                  </span>
                  <span className="font-medium">{def.autoSubmitInjected ? "On" : "Off"}</span>
                </div>

                {/* Recent inbox/outbox — newest first. Honest empty state. */}
                <div className="pt-1.5 mt-0.5 border-t border-black/[0.06] space-y-1">
                  {messages.length === 0 ? (
                    <div className="text-[10.5px] text-[#a1a1a6] py-0.5">No messages yet</div>
                  ) : (
                    messages.map((m) => {
                      const inbound = m.toInstanceId === instanceId;
                      const counterpart = inbound ? m.fromInstanceId : m.toInstanceId;
                      return (
                        <div key={m.id} className="flex items-center gap-1.5">
                          {inbound ? (
                            <CornerDownLeft className="w-3 h-3 shrink-0 text-[#30a14e]" />
                          ) : (
                            <CornerUpRight className="w-3 h-3 shrink-0 text-[#0a84ff]" />
                          )}
                          <span className="text-[#6e6e73] shrink-0">
                            {inbound ? "from" : "to"}
                          </span>
                          <span className="font-medium truncate flex-1 min-w-0">
                            {nameOf(counterpart)}
                          </span>
                          {m.status === "queued" && (
                            <span className="text-[10px] text-[#ff9f0a] shrink-0">queued</span>
                          )}
                          <span className="text-[10px] text-[#a1a1a6] shrink-0">
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
            <div className="rounded-xl ring-hair bg-white p-2.5 space-y-1.5 text-[11.5px]">
              <div className="flex items-center justify-between text-[#6e6e73]">
                <span className="flex items-center gap-1">
                  tokens
                  {meterEstimated && (
                    <span className="text-[9.5px] text-[#a1a1a6] font-normal">estimate</span>
                  )}
                </span>
                <span className="font-mono">
                  {meterTokens.toLocaleString()} / {meterLimit.toLocaleString()}
                </span>
              </div>
              <div className="h-1.5 rounded-full bg-black/[0.06] overflow-hidden">
                <div
                  className="h-full bg-[#0a84ff]"
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
      <div className="border-t border-black/[0.06] px-3 py-2 shrink-0 flex items-center gap-2">
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
          <div className="text-[10.5px] text-[#86868b] truncate">{status}</div>
        </div>
      </div>
    </aside>
  );
}
