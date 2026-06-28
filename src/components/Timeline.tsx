import { useCallback, useEffect, useRef, useState } from "react";
import {
  Zap,
  Bookmark,
  ArrowLeftRight,
  Gauge,
  Camera,
  Radio,
  Flag,
  BookOpen,
  GitFork,
  RotateCcw,
  X,
  Waypoints,
} from "lucide-react";
import { ipc, useSessionContext, useSnapshotCreated } from "../ipc";
import type { AgentDefinition, Session, Snapshot } from "../ipc";
import { timeHint } from "../lib/timeHint";
import { DeferredNote } from "./DeferredNote";

// ---------------------------------------------------------------------------
// Timeline — a per-agent memory-timeline screen over the REAL snapshot chain
// (M4.2). A self-contained full-screen overlay for ONE spawned session.
//
// HONESTY NOTE — every snapshot node comes from real `snapshot.list` data; the
// only synthetic nodes are the "Now · live context" header (the real live
// meter) and a "Session start" marker (from `session.startedAt`) — honest
// structural markers, not fabricated snapshots. The context meter is an
// ESTIMATE (streamed-byte derived), labelled as such. `Read` is functional
// (view a snapshot's real detail); `Fork`, `Restore`, "Diff vs previous" and
// "Carried forward" all need persisted conversation history that does NOT exist
// yet, so they render as honestly DISABLED / deferred, never fake affordances.
// ---------------------------------------------------------------------------

interface TimelineProps {
  def: AgentDefinition;
  session: Session; // a real spawned session (has id, startedAt, contextTokens/Limit)
  onClose: () => void;
}

// The deferred reason shared by Fork / Restore / Diff / Carried-forward — all
// blocked on the same missing capability.
const DEFERRED_REASON = "Captured once conversation history is persisted.";

// Per-type visual identity (dot color + icon + default title + pill label).
type SnapType = Snapshot["type"];
const TYPE_META: Record<
  SnapType,
  { color: string; Icon: typeof Zap; title: string; label: string }
> = {
  auto: { color: "#ff9f0a", Icon: Zap, title: "Auto compact", label: "auto" },
  manual: { color: "#0a84ff", Icon: Bookmark, title: "Manual snapshot", label: "manual" },
  handoff: { color: "#5e5ce6", Icon: ArrowLeftRight, title: "Handoff", label: "handoff" },
};

function typeMeta(type: SnapType) {
  return TYPE_META[type] ?? TYPE_META.manual;
}

// The display title of a snapshot: its label, or a sensible per-type default.
function snapTitle(s: Snapshot): string {
  return s.label?.trim() ? s.label : typeMeta(s.type).title;
}

// A small colored type pill.
function TypePill({ type }: { type: SnapType }) {
  const { color, label } = typeMeta(type);
  return (
    <span
      className="text-[10px] font-semibold px-1.5 py-px rounded-md capitalize"
      style={{ backgroundColor: `${color}1f`, color }}
    >
      {label}
    </span>
  );
}

export function Timeline({ def, session, onClose }: TimelineProps) {
  const sessionId = session.id;

  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  // Escape closes the overlay — the only keyboard exit for a full-screen view
  // with no exposed backdrop (WCAG 2.1.2, no keyboard trap). Add/remove is
  // idempotent across StrictMode's double-mount.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // ── Live context meter (estimate) ──────────────────────────────────────────
  // Seed from the session prop, then track live `session:context` estimates.
  // Same idiom as ContextDrawer: an ESTIMATE, never exact provider usage.
  const [ctx, setCtx] = useState<{ tokens: number; limit: number; estimated: boolean } | null>(
    session.contextTokens != null && session.contextLimit != null
      ? { tokens: session.contextTokens, limit: session.contextLimit, estimated: true }
      : null,
  );
  useSessionContext(sessionId, (e) =>
    setCtx({ tokens: e.contextTokens, limit: e.contextLimit, estimated: e.estimated }),
  );

  // The estimate is a CHAT-only concept (same rule as ContextDrawer): only chat
  // backends have a self-owned provider loop to estimate from. CLI agents
  // (claude-code/codex) track and display their own context — Conclave has no
  // honest source, and a fresh CLI session still carries the seeded
  // `0 / 200_000` columns (or a stale pre-fix estimate), so rendering them here
  // would reintroduce the very meter the backend fix removed. For non-chat
  // agents we null the meter out and fall back to "No context estimate yet".
  const trackContext = def.type === "chat";
  const meterTokens = trackContext ? (ctx?.tokens ?? session.contextTokens ?? null) : null;
  const meterLimit = trackContext ? (ctx?.limit ?? session.contextLimit ?? null) : null;
  const meterPct =
    meterTokens != null && meterLimit != null && meterLimit > 0
      ? Math.min(100, Math.round((meterTokens / meterLimit) * 100))
      : null;

  // ── Snapshots (real, newest-first) ─────────────────────────────────────────
  // Same refetch discipline as Blackboard/ContextDrawer: a per-session
  // useCallback + a monotonic seq guard so a stale response can't overwrite a
  // newer one. Refetches on mount / session change AND on each `snapshot:created`.
  const [snapshots, setSnapshots] = useState<Snapshot[]>([]);
  const [snapshotError, setSnapshotError] = useState(false);
  const [snapshotBusy, setSnapshotBusy] = useState(false);
  // `loaded` gates the empty state so "No snapshots yet" can't flash before the
  // first list settles (a session with snapshots would briefly read as empty).
  const [loaded, setLoaded] = useState(false);
  // Selected snapshot: `selectedId` drives the highlight; `detail` holds the
  // row shown in the aside, fetched fresh via `snapshot.read` (canonical by-id
  // detail), seeded optimistically from the list row to avoid a flash.
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [detail, setDetail] = useState<Snapshot | null>(null);
  const snapSeq = useRef(0);
  const readSeq = useRef(0);
  const refetchSnapshots = useCallback(() => {
    const mine = ++snapSeq.current;
    ipc.snapshot
      .list({ sessionId })
      .then((rows) => {
        if (mounted.current && mine === snapSeq.current) {
          setSnapshots(rows);
          setSnapshotError(false); // a successful list dismisses a stale create-error
          setLoaded(true);
        }
      })
      .catch((err: unknown) => {
        // Non-Tauri dev context (plain `vite`) lands here — surface in dev only.
        if (import.meta.env.DEV) {
          console.error("Timeline: snapshot.list failed", err);
        }
        if (mounted.current && mine === snapSeq.current) setLoaded(true);
      });
  }, [sessionId]);
  useEffect(() => {
    refetchSnapshots();
  }, [refetchSnapshots]);
  useSnapshotCreated(sessionId, () => refetchSnapshots());

  // Drop selection + reset the load gate when the focused session changes (the
  // parent can swap the `session` prop while the overlay is open). Without this
  // `selectedId`/`detail` would hold the previous session's snapshot.
  useEffect(() => {
    setSelectedId(null);
    setDetail(null);
    setLoaded(false);
  }, [sessionId]);

  // Open a snapshot's detail: highlight it, seed the aside from the list row
  // (instant), then fetch the canonical row via `snapshot.read`. A seq guard
  // drops a stale read if the user clicks another node before it resolves.
  const openDetail = useCallback((s: Snapshot) => {
    setSelectedId(s.id);
    setDetail(s);
    const mine = ++readSeq.current;
    ipc.snapshot
      .read({ snapshotId: s.id })
      .then((full) => {
        if (mounted.current && mine === readSeq.current) setDetail(full);
      })
      .catch((err: unknown) => {
        if (import.meta.env.DEV) {
          console.error("Timeline: snapshot.read failed", err);
        }
      });
  }, []);

  const clearDetail = useCallback(() => {
    setSelectedId(null);
    setDetail(null);
  }, []);

  // "Snapshot now" — create a manual snapshot, then refetch. Errors surfaced
  // honestly (dev console + an inline note), never silently swallowed.
  const doSnapshot = useCallback(() => {
    setSnapshotBusy(true);
    setSnapshotError(false);
    ipc.snapshot
      .create({ sessionId, type: "manual" })
      .then(() => {
        if (mounted.current) refetchSnapshots();
      })
      .catch((err: unknown) => {
        if (import.meta.env.DEV) {
          console.error("Timeline: snapshot.create failed", err);
        }
        if (mounted.current) setSnapshotError(true);
      })
      .finally(() => {
        if (mounted.current) setSnapshotBusy(false);
      });
  }, [sessionId, refetchSnapshots]);

  return (
    <div className="fixed inset-0 z-50 flex flex-col bg-sidebar">
      {/* ── Header bar ─────────────────────────────────────────────────── */}
      <div className="h-12 flex items-center justify-between px-4 border-b border-overlay/[0.06] bg-surface shrink-0">
        <div className="flex items-center gap-2.5 min-w-0">
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
          <span className="text-[13px] font-semibold tracking-tight truncate">{def.name}</span>
          <span className="text-[10px] font-medium text-text-muted bg-overlay/[0.04] px-1.5 py-px rounded-md shrink-0">
            Memory timeline
          </span>
        </div>

        <div className="flex items-center gap-3 shrink-0">
          {/* Live context meter — ESTIMATE, labelled. */}
          {meterPct != null && (
            <div className="flex items-center gap-1.5 text-[11px] text-text-secondary">
              <Gauge className="w-3.5 h-3.5 text-text-muted" />
              <span>
                context {meterPct}%
                <span className="ml-1 text-[9.5px] text-text-tertiary">estimate</span>
              </span>
              <div className="h-1.5 w-20 rounded-full bg-overlay/[0.06] overflow-hidden">
                <div className="h-full bg-accent" style={{ width: `${meterPct}%` }} />
              </div>
            </div>
          )}

          {snapshotError && (
            <span className="text-[10.5px] text-danger">Couldn't save snapshot</span>
          )}

          <button
            onClick={doSnapshot}
            disabled={snapshotBusy}
            title="Create a manual snapshot"
            className="text-[12px] font-semibold text-white bg-accent px-3 h-7 rounded-lg hover:brightness-105 disabled:opacity-40 flex items-center gap-1.5"
          >
            <Camera className="w-3.5 h-3.5" />
            {snapshotBusy ? "Saving…" : "Snapshot now"}
          </button>
          <button
            onClick={onClose}
            className="w-7 h-7 grid place-items-center rounded-md hover:bg-overlay/[0.06] text-text-muted shrink-0"
            aria-label="Close timeline"
          >
            <X className="w-[15px] h-[15px]" />
          </button>
        </div>
      </div>

      {/* ── Body: center timeline + right detail aside ─────────────────── */}
      <div className="flex-1 flex min-h-0">
        {/* Center — the vertical timeline */}
        <div className="flex-1 overflow-y-auto scroll-thin min-w-0">
          <div className="max-w-[640px] mx-auto px-6 py-6">
            {/* Now · live context (synthetic structural marker — real meter). */}
            <TimelineRow
              dot={
                <span className="relative grid place-items-center">
                  <Radio className="w-3.5 h-3.5 text-success" />
                </span>
              }
              connector
            >
              <div className="rounded-xl ring-hair bg-surface px-3 py-2.5">
                <div className="flex items-center justify-between">
                  <span className="text-[12.5px] font-semibold">Now · live context</span>
                  <span className="text-[10px] font-medium text-success bg-success/[0.12] px-1.5 py-px rounded-md">
                    live
                  </span>
                </div>
                <div className="mt-0.5 text-[11.5px] text-text-secondary">
                  {meterTokens != null && meterLimit != null ? (
                    <>
                      ~{meterTokens.toLocaleString()} / {meterLimit.toLocaleString()} tokens
                      {meterPct != null ? ` · ${meterPct}%` : ""}
                      <span className="ml-1 text-[10px] text-text-tertiary">estimate</span>
                    </>
                  ) : (
                    <span className="text-text-tertiary">No context estimate yet</span>
                  )}
                </div>
              </div>
            </TimelineRow>

            {/* Snapshot nodes (real, newest-first). The empty state is gated on
                `loaded` so it can't flash before the first list settles. */}
            {snapshots.length === 0 ? (
              <TimelineRow
                dot={<span className="w-2.5 h-2.5 rounded-full bg-overlay/[0.12]" />}
                connector
              >
                <DeferredNote>
                  {loaded
                    ? "No snapshots yet — capture one with Snapshot now."
                    : "Loading…"}
                </DeferredNote>
              </TimelineRow>
            ) : (
              snapshots.map((s) => {
                const meta = typeMeta(s.type);
                const isSelected = s.id === selectedId;
                return (
                  <TimelineRow
                    key={s.id}
                    connector
                    dot={
                      <span
                        className="grid place-items-center w-4 h-4 rounded-full"
                        style={{ backgroundColor: `${meta.color}1f` }}
                      >
                        <meta.Icon className="w-3 h-3" style={{ color: meta.color }} />
                      </span>
                    }
                  >
                    <div
                      className={`rounded-xl bg-surface px-3 py-2.5 ${
                        isSelected
                          ? "ring-2 ring-accent/50"
                          : "ring-hair hover:bg-overlay/[0.01]"
                      }`}
                    >
                      <div className="flex items-center gap-2">
                        <span className="text-[12.5px] font-semibold truncate">
                          {snapTitle(s)}
                        </span>
                        <TypePill type={s.type} />
                        <span className="ml-auto text-[10.5px] text-text-tertiary shrink-0 flex items-center gap-1.5">
                          <span>{timeHint(s.createdAt)}</span>
                          {s.tokens != null && (
                            <span className="font-mono">~{s.tokens.toLocaleString()} tok</span>
                          )}
                        </span>
                      </div>
                      {s.summary && (
                        <div className="mt-1 text-[11.5px] text-text-secondary">{s.summary}</div>
                      )}
                      <div className="mt-1.5">
                        <button
                          onClick={() => openDetail(s)}
                          className="text-[11px] font-medium text-accent hover:underline flex items-center gap-1"
                        >
                          <BookOpen className="w-3 h-3" />
                          Read
                        </button>
                      </div>
                    </div>
                  </TimelineRow>
                );
              })
            )}

            {/* Session start (synthetic structural marker — real startedAt). */}
            <TimelineRow dot={<Flag className="w-3.5 h-3.5 text-text-muted" />}>
              <div className="rounded-xl ring-hair bg-surface px-3 py-2.5">
                <div className="flex items-center justify-between">
                  <span className="text-[12.5px] font-semibold">Session start</span>
                  <span className="text-[10.5px] text-text-tertiary">
                    {timeHint(session.startedAt)}
                  </span>
                </div>
              </div>
            </TimelineRow>
          </div>
        </div>

        {/* Right — snapshot detail aside */}
        <aside className="w-[306px] vibrancy border-l border-overlay/[0.06] flex flex-col shrink-0">
          <div className="h-12 flex items-center px-4 border-b border-overlay/[0.06] shrink-0">
            <span className="text-[12px] font-semibold text-text-secondary tracking-tight">
              Snapshot detail
            </span>
            {detail && (
              <>
                <span className="ml-2">
                  <TypePill type={detail.type} />
                </span>
                <button
                  onClick={clearDetail}
                  className="ml-auto w-6 h-6 grid place-items-center rounded-md hover:bg-overlay/[0.06] text-text-muted"
                  aria-label="Close snapshot detail"
                  title="Close detail"
                >
                  <X className="w-3.5 h-3.5" />
                </button>
              </>
            )}
          </div>

          <div className="flex-1 overflow-y-auto scroll-thin p-3 space-y-4">
            {!detail ? (
              <DeferredNote>Select a snapshot to view its detail.</DeferredNote>
            ) : (
              <>
                {/* Title + meta line (omit null parts). */}
                <div>
                  <div className="text-[13px] font-semibold tracking-tight">
                    {snapTitle(detail)}
                  </div>
                  <div className="mt-0.5 text-[11px] text-text-muted">
                    {[
                      timeHint(detail.createdAt),
                      detail.tokens != null
                        ? `~${detail.tokens.toLocaleString()} tokens`
                        : null,
                      detail.triggerPct != null ? `trigger ${detail.triggerPct}%` : null,
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </div>
                </div>

                {/* Summary — REAL when present (deterministic honest string). */}
                {detail.summary && (
                  <div className="rounded-xl ring-hair bg-surface px-2.5 py-2 text-[11.5px] text-text-body">
                    {detail.summary}
                  </div>
                )}

                {/* Carried forward — DEFERRED (always null until history persisted). */}
                <div>
                  <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5 px-0.5">
                    Carried forward
                  </div>
                  <DeferredNote>{DEFERRED_REASON}</DeferredNote>
                </div>

                {/* Diff vs previous — only when a previous snapshot exists. */}
                {detail.prevSnapshotId && (
                  <div>
                    <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5 px-0.5">
                      Diff vs previous
                    </div>
                    <DeferredNote>{DEFERRED_REASON}</DeferredNote>
                  </div>
                )}

                {/* Actions — Read is the current view; Fork/Restore are deferred. */}
                <div className="flex items-center gap-2 pt-1">
                  <button
                    disabled
                    title="You're viewing this snapshot's detail"
                    className="text-[11.5px] font-medium text-text-secondary bg-overlay/[0.05] px-2.5 h-7 rounded-lg flex items-center gap-1.5 cursor-default"
                  >
                    <BookOpen className="w-3.5 h-3.5" />
                    Viewing
                  </button>
                  <button
                    disabled
                    title="Fork needs persisted conversation history (deferred)"
                    className="text-[11.5px] font-medium text-text-tertiary ring-hair px-2.5 h-7 rounded-lg flex items-center gap-1.5 opacity-50 cursor-not-allowed"
                  >
                    <GitFork className="w-3.5 h-3.5" />
                    Fork
                  </button>
                  <button
                    disabled
                    title="Restore needs persisted conversation history (deferred)"
                    className="text-[11.5px] font-medium text-text-tertiary ring-hair px-2.5 h-7 rounded-lg flex items-center gap-1.5 opacity-50 cursor-not-allowed"
                  >
                    <RotateCcw className="w-3.5 h-3.5" />
                    Restore
                  </button>
                </div>
                <p className="text-[10.5px] text-text-tertiary leading-snug px-0.5">
                  Fork and Restore need persisted conversation history — deferred.
                </p>
              </>
            )}
          </div>
        </aside>
      </div>
    </div>
  );
}

// One row of the vertical timeline: a left rail with the dot (and an optional
// connector line down to the next row) + the node content on the right.
function TimelineRow({
  dot,
  connector = false,
  children,
}: {
  dot: React.ReactNode;
  connector?: boolean;
  children: React.ReactNode;
}) {
  return (
    <div className="flex gap-3">
      <div className="flex flex-col items-center w-4 shrink-0">
        <div className="h-5 grid place-items-center">{dot}</div>
        {connector && <div className="w-px flex-1 bg-overlay/[0.1] min-h-4" />}
      </div>
      <div className={`flex-1 min-w-0 ${connector ? "pb-3" : ""}`}>{children}</div>
    </div>
  );
}
