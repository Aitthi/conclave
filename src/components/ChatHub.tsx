import { useEffect, useMemo, useRef, useState } from "react";
import { MessageSquare, Search, X } from "lucide-react";
import { timeHint } from "../lib/timeHint";
import { ClampText } from "./ClampText";
import { createdAtMs, derivePairs, pairKeyOf } from "../lib/chatPairs";
import { clockLabel, dayLabel, group } from "../lib/chatFeed";
import { useWorkspaceChat, type AgentIdentity } from "../lib/useWorkspaceChat";

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

// Auto-scroll only snaps to the newest message when the reader is already
// within this many pixels of the bottom — a background refetch must never
// scroll-jack someone reading older history (same guard as the old drawer
// timeline).
const NEAR_BOTTOM_PX = 40;

/** Small colored avatar square, initial-letter, matching Blackboard's. */
function Avatar({ identity, size = 5 }: { identity: AgentIdentity; size?: 4 | 5 | 7 }) {
  const cls =
    size === 7
      ? "w-7 h-7 text-[12px] rounded-[8px]"
      : size === 5
        ? "w-5 h-5 text-[10px] rounded-md"
        : "w-4 h-4 text-[9px] rounded-[5px]";
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
  const { messages, loadError, identityOf } = useWorkspaceChat(workspaceId);

  // ── View state: null = All feed, else a pair key; plus client-side search. ─
  const [selectedPair, setSelectedPair] = useState<string | null>(null);
  const [search, setSearch] = useState("");

  const pairs = useMemo(() => derivePairs(messages), [messages]);
  // A selected pair whose messages aged out of the window (or whose agents
  // left) falls back to All rather than showing a stuck empty pane.
  useEffect(() => {
    if (selectedPair && !pairs.some((p) => p.key === selectedPair)) setSelectedPair(null);
  }, [pairs, selectedPair]);

  // Oldest→newest for reading; narrowed by pair, then by search. Sorted by
  // parsed createdAt — the raw window's array order is untrusted (the default
  // fixture deliberately shuffles it), so a blind reverse() would make
  // `newest` (last element) point at an arbitrary message. Stable sort:
  // createdAt ties keep window order, so the later array element stays newest.
  const visible = useMemo(() => {
    const oldestFirst = [...messages].sort(
      (a, b) => createdAtMs(a.createdAt) - createdAtMs(b.createdAt),
    );
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
  const groups = useMemo(() => group(visible), [visible]);

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
          <div className="flex items-center gap-2 bg-overlay/[0.05] rounded-lg px-2.5 h-7 w-52">
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
        <aside className="w-[240px] border-r border-overlay/[0.06] overflow-y-auto scroll-thin p-2 space-y-0.5 shrink-0">
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
            <span className="ml-auto text-[10px] text-text-tertiary font-mono tabular-nums">
              {messages.length}
            </span>
          </button>
          <div className="px-2 pt-2 pb-1 text-[10px] font-bold tracking-wider text-text-tertiary uppercase">
            Conversations
          </div>
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
            // All view — a feed: sender-group headers, day dividers, the
            // recipient carried by the chip below each bubble (a hub has no
            // "self", so bubble sides would be a lie).
            groups.map((g, gi) => {
              const from = identityOf(g.fromInstanceId);
              const prevGroup = groups[gi - 1];
              const showDivider =
                gi === 0 ||
                (prevGroup &&
                  dayLabel(prevGroup.items[0].createdAt) !== dayLabel(g.items[0].createdAt));
              return (
                <div key={`${g.fromInstanceId}-${g.items[0].id}`}>
                  {showDivider && (
                    <div className="flex items-center gap-2 justify-center py-1">
                      <span className="h-px flex-1 bg-overlay/[0.06]" />
                      <span className="text-[10.5px] text-text-tertiary font-mono tabular-nums">
                        {dayLabel(g.items[0].createdAt)}
                      </span>
                      <span className="h-px flex-1 bg-overlay/[0.06]" />
                    </div>
                  )}
                  <div className="flex gap-2.5">
                    <Avatar identity={from} size={7} />
                    <div className="min-w-0 flex flex-col gap-1 items-start" style={{ maxWidth: "72%" }}>
                      <div className="flex items-baseline gap-2">
                        <span className="text-[12.5px] font-semibold text-text-primary">
                          {from.name}
                        </span>
                        {from.role && <span className="text-[10px] text-text-tertiary">{from.role}</span>}
                        <span className="text-[10px] text-text-tertiary font-mono tabular-nums">
                          {clockLabel(g.items[0].createdAt)}
                        </span>
                      </div>
                      {g.items.map((m) => {
                        const to = identityOf(m.toInstanceId);
                        return (
                          <div key={m.id} className="flex flex-col gap-0.5 items-start self-stretch">
                            <div
                              className="rounded-md border border-overlay/[0.06] bg-surface-raised px-[0.72rem] py-2 text-[0.84rem] leading-[1.5] text-text-primary"
                              title={`→ ${to.name}`}
                            >
                              <ClampText text={m.text} outgoing={false} lines={12} />
                            </div>
                            <div className="flex items-center gap-1 self-stretch">
                              {m.status === "queued" && (
                                <span className="text-[9px] text-warning">queued</span>
                              )}
                              {m.autoSubmitted && (
                                <span className="text-[9px] text-text-tertiary">injected</span>
                              )}
                              <span className="text-[10px] text-text-tertiary font-mono tabular-nums">
                                {clockLabel(m.createdAt)}
                              </span>
                              <span
                                className="ml-auto flex items-center gap-1 text-[10px] text-text-tertiary"
                                title={`→ ${to.name}`}
                              >
                                <Avatar identity={to} size={4} />
                                <span>{to.name}</span>
                              </span>
                            </div>
                          </div>
                        );
                      })}
                    </div>
                  </div>
                </div>
              );
            })
          ) : (
            // Pair view — a conversation: stable side per participant
            // (pair-key order: lexicographically-first id on the left). No
            // recipient chip — the pair implies it.
            groups.map((g, gi) => {
              const onLeft = g.fromInstanceId === leftIdOfPair;
              const from = identityOf(g.fromInstanceId);
              const prevGroup = groups[gi - 1];
              const showDivider =
                gi === 0 ||
                (prevGroup &&
                  dayLabel(prevGroup.items[0].createdAt) !== dayLabel(g.items[0].createdAt));
              return (
                <div key={`${g.fromInstanceId}-${g.items[0].id}`}>
                  {showDivider && (
                    <div className="flex items-center gap-2 justify-center py-1">
                      <span className="h-px flex-1 bg-overlay/[0.06]" />
                      <span className="text-[10.5px] text-text-tertiary font-mono tabular-nums">
                        {dayLabel(g.items[0].createdAt)}
                      </span>
                      <span className="h-px flex-1 bg-overlay/[0.06]" />
                    </div>
                  )}
                  <div className={`flex flex-col gap-1 ${onLeft ? "items-start" : "items-end"}`}>
                    <div className="flex items-center gap-1.5">
                      <Avatar identity={from} size={5} />
                      <span className="text-[11px] font-semibold text-text-primary">{from.name}</span>
                      <span className="text-[10px] text-text-tertiary font-mono tabular-nums">
                        {clockLabel(g.items[0].createdAt)}
                      </span>
                    </div>
                    {g.items.map((m) => (
                      <div
                        key={m.id}
                        className={`flex flex-col gap-0.5 ${onLeft ? "items-start" : "items-end"}`}
                        style={{ maxWidth: "72%" }}
                      >
                        <div className="rounded-md border border-overlay/[0.06] bg-surface-raised px-[0.72rem] py-2 text-[0.84rem] leading-[1.5] text-text-primary">
                          <ClampText text={m.text} outgoing={false} lines={12} />
                        </div>
                        <div className="flex items-center gap-1.5 text-[9px] text-text-tertiary">
                          {m.status === "queued" && <span className="text-warning">queued</span>}
                          {m.autoSubmitted && <span>injected</span>}
                          <span className="text-[10px] font-mono tabular-nums">
                            {clockLabel(m.createdAt)}
                          </span>
                        </div>
                      </div>
                    ))}
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
