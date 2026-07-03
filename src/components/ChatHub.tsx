import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { MessageSquare, MoveRight, Search, X } from "lucide-react";
import { ipc, useAnyMessageInjected } from "../ipc";
import type { InterAgentMessage } from "../ipc";
import { timeHint } from "../lib/timeHint";
import { ClampText } from "./ClampText";
import { derivePairs, pairKeyOf } from "../lib/chatPairs";

// ---------------------------------------------------------------------------
// Chat Hub — the workspace's inter-agent conversation, center-pane sized
// (spec: docs/superpowers/specs/2026-07-03-chat-hub-design.md). Read-only in
// Phase 1: read + search + per-pair narrowing; composing is a recorded
// follow-up. Renders via AppShell's Blackboard slot pattern.
// ---------------------------------------------------------------------------

interface ChatHubProps {
  workspaceId: string;
  onClose: () => void;
}

// The hub loads the workspace's full recent window (the server clamps to its
// max); per-pair narrowing and search are client-side over this window.
const MESSAGE_LIMIT = 200;

// Auto-scroll only snaps to the newest message when the reader is already
// within this many pixels of the bottom — a background refetch must never
// scroll-jack someone reading older history (same guard as the old drawer
// timeline).
const NEAR_BOTTOM_PX = 40;

interface AgentIdentity {
  name: string;
  color: string;
}

const FALLBACK_IDENTITY: AgentIdentity = { name: "unknown", color: "#8e8e93" };

/** Small colored avatar square, initial-letter, matching Blackboard's. */
function Avatar({ identity, size = 5 }: { identity: AgentIdentity; size?: 4 | 5 }) {
  const cls = size === 5 ? "w-5 h-5 text-[10px] rounded-md" : "w-4 h-4 text-[9px] rounded-[5px]";
  return (
    <span
      className={`${cls} text-white grid place-items-center font-bold shrink-0`}
      style={{ backgroundColor: identity.color }}
    >
      {identity.name.charAt(0).toUpperCase()}
    </span>
  );
}

export function ChatHub({ workspaceId, onClose }: ChatHubProps) {
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  // ── Identities: instanceId → { name, color } (instance.list ⨝ agentDef.list,
  //    the same join WorkspacePane/Blackboard do). ─────────────────────────────
  const [agents, setAgents] = useState<Map<string, AgentIdentity>>(new Map());
  useEffect(() => {
    let active = true;
    Promise.all([ipc.instance.list({ workspaceId }), ipc.agentDef.list()])
      .then(([instances, defs]) => {
        if (!active) return;
        const defsById = new Map(defs.map((d) => [d.id, d]));
        const m = new Map<string, AgentIdentity>();
        for (const inst of instances) {
          const def = defsById.get(inst.agentDefId);
          if (def) m.set(inst.id, { name: def.name, color: def.color ?? "#6e6e73" });
        }
        setAgents(m);
      })
      .catch((err: unknown) => {
        if (import.meta.env.DEV) console.error("ChatHub: identity load failed", err);
      });
    return () => {
      active = false;
    };
  }, [workspaceId]);
  const identityOf = useCallback(
    (id: string): AgentIdentity => agents.get(id) ?? FALLBACK_IDENTITY,
    [agents],
  );

  // ── Messages — REAL data from message.listForWorkspace, newest-first from
  //    the API; seq-guarded so a stale response can't overwrite a newer one. ──
  const [messages, setMessages] = useState<InterAgentMessage[]>([]);
  const [loadError, setLoadError] = useState(false);
  const seq = useRef(0);
  const refetch = useCallback(() => {
    const mine = ++seq.current;
    ipc.message
      .listForWorkspace({ workspaceId, limit: MESSAGE_LIMIT })
      .then((rows) => {
        if (mounted.current && mine === seq.current) {
          setMessages(rows);
          setLoadError(false);
        }
      })
      .catch((err: unknown) => {
        if (import.meta.env.DEV) console.error("ChatHub: listForWorkspace failed", err);
        if (mounted.current && mine === seq.current) setLoadError(true);
      });
  }, [workspaceId]);
  useEffect(() => {
    refetch();
  }, [refetch]);
  // Any injection anywhere → refetch (workspace-scoped server-side; a
  // cross-workspace event costs one cheap guarded refetch).
  useAnyMessageInjected(() => refetch());

  // ── View state: null = All feed, else a pair key; plus client-side search. ─
  const [selectedPair, setSelectedPair] = useState<string | null>(null);
  const [search, setSearch] = useState("");

  const pairs = useMemo(() => derivePairs(messages), [messages]);
  // A selected pair whose messages aged out of the window (or whose agents
  // left) falls back to All rather than showing a stuck empty pane.
  useEffect(() => {
    if (selectedPair && !pairs.some((p) => p.key === selectedPair)) setSelectedPair(null);
  }, [pairs, selectedPair]);

  // Oldest→newest for reading; narrowed by pair, then by search.
  const visible = useMemo(() => {
    const oldestFirst = [...messages].reverse();
    const inPair = selectedPair
      ? oldestFirst.filter((m) => pairKeyOf(m.fromInstanceId, m.toInstanceId) === selectedPair)
      : oldestFirst;
    const q = search.trim().toLowerCase();
    if (!q) return inPair;
    return inPair.filter(
      (m) =>
        m.text.toLowerCase().includes(q) ||
        identityOf(m.fromInstanceId).name.toLowerCase().includes(q) ||
        identityOf(m.toInstanceId).name.toLowerCase().includes(q),
    );
  }, [messages, selectedPair, search, identityOf]);

  // ── Auto-scroll (near-bottom guarded, ported from the drawer timeline). ────
  const timelineRef = useRef<HTMLDivElement | null>(null);
  const forceScrollRef = useRef(true);
  const lastMsgIdRef = useRef<string | null>(null);
  const atBottomRef = useRef(true);
  const onTimelineScroll = () => {
    const el = timelineRef.current;
    if (el) atBottomRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < NEAR_BOTTOM_PX;
  };
  // Switching conversation (or search) is a fresh read — snap to newest.
  useEffect(() => {
    forceScrollRef.current = true;
  }, [selectedPair, search]);
  useEffect(() => {
    const el = timelineRef.current;
    if (!el) return;
    const newest = visible[visible.length - 1];
    const newestId = newest?.id ?? null;
    const isNew = newestId !== lastMsgIdRef.current;
    if (forceScrollRef.current || (isNew && atBottomRef.current)) {
      el.scrollTop = el.scrollHeight;
      atBottomRef.current = true;
    }
    lastMsgIdRef.current = newestId;
    forceScrollRef.current = false;
  }, [visible]);

  // Pair view: the lexicographically-first id (pair key order) sits LEFT.
  const leftIdOfPair = selectedPair ? selectedPair.split("|")[0] : null;

  const pairLabel = (aId: string, bId: string) =>
    `${identityOf(aId).name} · ${identityOf(bId).name}`;

  return (
    <main className="flex-1 flex flex-col min-w-0 bg-surface">
      {/* Header — mirrors the Blackboard screen's. */}
      <div className="h-12 flex items-center justify-between px-5 border-b border-overlay/[0.06] shrink-0">
        <div className="flex items-center gap-2.5">
          <div className="w-6 h-6 rounded-[7px] bg-ink text-on-ink grid place-items-center ring-hair shrink-0">
            <MessageSquare className="w-[13px] h-[13px]" />
          </div>
          <div className="text-[13px] font-semibold tracking-tight flex items-center gap-1.5">
            Chat
            <span className="text-[10px] font-medium text-text-muted bg-overlay/[0.04] px-1.5 py-px rounded-md">
              read-only
            </span>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <div className="flex items-center gap-2 bg-overlay/[0.05] rounded-lg px-2.5 h-7 w-44">
            <Search className="w-[13px] h-[13px] text-text-muted shrink-0" />
            <input
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Search messages"
              className="bg-transparent outline-none text-[12px] placeholder:text-text-tertiary w-full"
            />
          </div>
          <button
            title="Close Chat"
            onClick={onClose}
            className="w-7 h-7 grid place-items-center rounded-md hover:bg-overlay/[0.05] text-text-secondary"
          >
            <X className="w-[15px] h-[15px]" />
          </button>
        </div>
      </div>

      <div className="flex-1 flex min-h-0">
        {/* ── Sidebar: All + conversation pairs, most-recent first. ── */}
        <aside className="w-[220px] border-r border-overlay/[0.06] overflow-y-auto scroll-thin p-2 space-y-0.5 shrink-0">
          <button
            onClick={() => setSelectedPair(null)}
            className={`w-full flex items-center gap-2 px-2 py-1.5 rounded-lg text-left transition-colors ${
              selectedPair === null ? "bg-overlay/[0.06]" : "hover:bg-overlay/[0.04]"
            }`}
          >
            <div className="w-5 h-5 rounded-md bg-ink text-on-ink grid place-items-center shrink-0">
              <MessageSquare className="w-3 h-3" />
            </div>
            <span className="text-[12px] font-semibold">All</span>
          </button>
          {pairs.map((p) => (
            <button
              key={p.key}
              onClick={() => setSelectedPair(p.key)}
              className={`w-full flex items-center gap-2 px-2 py-1.5 rounded-lg text-left transition-colors ${
                selectedPair === p.key ? "bg-overlay/[0.06]" : "hover:bg-overlay/[0.04]"
              }`}
            >
              <span className="flex -space-x-1 shrink-0">
                <Avatar identity={identityOf(p.aId)} size={4} />
                <Avatar identity={identityOf(p.bId)} size={4} />
              </span>
              <span className="text-[12px] font-medium truncate flex-1 min-w-0">
                {pairLabel(p.aId, p.bId)}
              </span>
              <span className="text-[9.5px] text-text-tertiary shrink-0">
                {timeHint(p.lastAt)}
              </span>
            </button>
          ))}
        </aside>

        {/* ── Timeline. ── */}
        <div
          ref={timelineRef}
          onScroll={onTimelineScroll}
          className="flex-1 overflow-y-auto scroll-thin px-5 py-4 space-y-3 min-w-0"
        >
          {loadError ? (
            <div className="text-[11.5px] text-danger py-4 text-center">
              Couldn't load messages
            </div>
          ) : visible.length === 0 ? (
            <div className="text-[11.5px] text-text-tertiary py-4 text-center">
              {search.trim() ? "No messages match the search" : "No messages yet"}
            </div>
          ) : selectedPair === null ? (
            // All view — a feed: every row left-aligned with sender → recipient
            // chrome (a hub has no "self", so bubble sides would be a lie).
            visible.map((m) => {
              const from = identityOf(m.fromInstanceId);
              const to = identityOf(m.toInstanceId);
              return (
                <div key={m.id} className="flex gap-2.5">
                  <Avatar identity={from} />
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-1.5 text-[10.5px] mb-0.5">
                      <span className="font-semibold text-text-secondary">{from.name}</span>
                      <MoveRight className="w-3 h-3 text-text-tertiary shrink-0" />
                      <span className="font-semibold text-text-secondary">{to.name}</span>
                      <span className="text-[9.5px] text-text-tertiary ml-auto shrink-0">
                        {timeHint(m.createdAt)}
                      </span>
                      {m.status === "queued" && (
                        <span className="text-[9.5px] text-warning shrink-0">queued</span>
                      )}
                    </div>
                    <div
                      className="rounded-xl bg-overlay/[0.05] px-3 py-2 text-[12px] text-text-primary"
                      style={{ borderLeft: `2px solid ${from.color}` }}
                    >
                      <ClampText text={m.text} outgoing={false} lines={12} />
                    </div>
                  </div>
                </div>
              );
            })
          ) : (
            // Pair view — a conversation: stable side per participant
            // (pair-key order: lexicographically-first id on the left).
            visible.map((m) => {
              const onLeft = m.fromInstanceId === leftIdOfPair;
              const from = identityOf(m.fromInstanceId);
              return (
                <div
                  key={m.id}
                  className={`flex flex-col ${onLeft ? "items-start" : "items-end"}`}
                >
                  <div className="flex items-center gap-1 mb-0.5 px-0.5 max-w-[72%]">
                    <span
                      className="w-1.5 h-1.5 rounded-full shrink-0"
                      style={{ backgroundColor: from.color }}
                    />
                    <span className="text-[10px] font-semibold text-text-secondary truncate">
                      {from.name}
                    </span>
                  </div>
                  <div
                    className={`max-w-[72%] rounded-2xl bg-overlay/[0.05] px-3 py-2 text-[12px] text-text-primary ${
                      onLeft ? "rounded-bl-md" : "rounded-br-md"
                    }`}
                    style={
                      onLeft
                        ? { borderLeft: `2px solid ${from.color}` }
                        : { borderRight: `2px solid ${from.color}` }
                    }
                  >
                    <ClampText text={m.text} outgoing={false} lines={12} />
                  </div>
                  <div
                    className={`flex items-center gap-1.5 mt-0.5 px-0.5 text-[9px] text-text-tertiary ${
                      onLeft ? "" : "flex-row-reverse"
                    }`}
                  >
                    <span>{timeHint(m.createdAt)}</span>
                    {m.status === "queued" && <span className="text-warning">queued</span>}
                    {m.autoSubmitted && <span>injected</span>}
                  </div>
                </div>
              );
            })
          )}
        </div>
      </div>
    </main>
  );
}
