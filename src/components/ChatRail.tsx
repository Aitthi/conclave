import { useEffect, useMemo, useRef, useState } from "react";
import {
  Eye,
  Hash,
  MoveRight,
  PanelRightClose,
  PanelRightOpen,
  Maximize2,
  Users,
} from "lucide-react";
import type { InterAgentMessage, WorkspaceAgent } from "../ipc";
import { timeHint } from "../lib/timeHint";
import { ClampText } from "./ClampText";
import { pairKeyOf } from "../lib/chatPairs";
import { deriveRooms, type Room } from "../lib/chatRooms";
import { useWorkspaceChat, type AgentIdentity } from "../lib/useWorkspaceChat";
import type { RoutingTarget } from "./RoutingPicker";

// ---------------------------------------------------------------------------
// ChatRail — the right rail's read-only, real-time viewer of agent traffic
// (design: bb design:right-rail-chat; plan: docs/superpowers/plans/2026-07-04
// -right-rail-chats.md Task 3). Replaces ContextDrawer. R8 (human directive):
// agents only — the human never appears here (their direct sends never enter
// InterAgentMessage). DM rooms render EXACTLY like the channel, just filtered
// — Slack-style, no "mine"/two-sided pair-view semantics (plan guard 932f2b4).
// ---------------------------------------------------------------------------

interface ChatRailProps {
  workspaceId: string;
  roster: RoutingTarget[];
  statuses: Record<string, WorkspaceAgent["status"]>;
  onOpenChat?: () => void;
}

// Auto-scroll only snaps to the newest message when the reader is already
// within this many pixels of the bottom — a background refetch must never
// scroll-jack someone reading older history (ported from the Chat Hub).
const NEAR_BOTTOM_PX = 40;

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

/** Room chip avatar: a hash glyph for the channel, a stacked pair for a DM. */
function RoomAvatar({ room, identityOf }: { room: Room; identityOf: (id: string) => AgentIdentity }) {
  if (room.kind === "channel") {
    return (
      <span className="w-5 h-5 rounded-md bg-ink text-on-ink grid place-items-center shrink-0">
        <Hash className="w-3 h-3" />
      </span>
    );
  }
  return (
    <span className="flex -space-x-1 shrink-0">
      <Avatar identity={identityOf(room.memberIds[0] ?? "")} size={4} />
      <Avatar identity={identityOf(room.memberIds[1] ?? "")} size={4} />
    </span>
  );
}

function roomTitle(room: Room, identityOf: (id: string) => AgentIdentity): string {
  if (room.kind === "channel") return "workspace";
  return room.memberIds.map((id) => identityOf(id).name).join(" · ");
}

function roomIsLive(room: Room, statuses: Record<string, WorkspaceAgent["status"]>): boolean {
  if (room.kind === "channel") return Object.values(statuses).some((s) => s === "running");
  return room.memberIds.some((id) => statuses[id] === "running");
}

// Day-boundary divider label — honest (a merged feed can span more than
// "today"), not the proto's always-"Today" literal.
function dayLabel(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const today = new Date();
  if (d.toDateString() === today.toDateString()) return "Today";
  const yesterday = new Date(today);
  yesterday.setDate(yesterday.getDate() - 1);
  if (d.toDateString() === yesterday.toDateString()) return "Yesterday";
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

// Consecutive same-sender grouping (oldest-first input), matching the proto's
// group() — a run of messages from the same instance shares one avatar/name
// header, each message keeps its own recipient + timestamp line.
interface MsgGroup {
  fromInstanceId: string;
  items: InterAgentMessage[];
}
function group(messages: InterAgentMessage[]): MsgGroup[] {
  const out: MsgGroup[] = [];
  for (const m of messages) {
    const last = out[out.length - 1];
    if (last && last.fromInstanceId === m.fromInstanceId) last.items.push(m);
    else out.push({ fromInstanceId: m.fromInstanceId, items: [m] });
  }
  return out;
}

export function ChatRail({ workspaceId, roster, statuses, onOpenChat }: ChatRailProps) {
  const [open, setOpen] = useState(true);
  const { messages, loadError, identityOf } = useWorkspaceChat(workspaceId);

  // ── Rooms + selection (client-side, workspace-scoped in-memory state — R3). ─
  // Captured once, at mount — the unread baseline for any room with no
  // explicit lastSeen entry yet (F4: a pair room born mid-session badges its
  // new messages instead of reading as pre-existing, already-read history).
  const mountedAtRef = useRef<string>(new Date().toISOString());
  const [lastSeen, setLastSeen] = useState<Record<string, string>>({});
  const rooms = useMemo(
    () => deriveRooms(messages, lastSeen, mountedAtRef.current),
    [messages, lastSeen],
  );
  const [selectedKey, setSelectedKey] = useState<string>("workspace");
  // A selected DM room whose messages aged out of the window falls back to
  // #workspace rather than showing a stuck, unreachable chip.
  useEffect(() => {
    if (!rooms.some((r) => r.key === selectedKey)) setSelectedKey("workspace");
  }, [rooms, selectedKey]);

  const roomMessages = useMemo(() => {
    if (selectedKey === "workspace") return messages;
    return messages.filter((m) => pairKeyOf(m.fromInstanceId, m.toInstanceId) === selectedKey);
  }, [messages, selectedKey]);
  const oldestFirst = useMemo(() => [...roomMessages].reverse(), [roomMessages]);
  const groups = useMemo(() => group(oldestFirst), [oldestFirst]);

  const activeRoom = rooms.find((r) => r.key === selectedKey) ?? rooms[0];
  const totalUnread = rooms.reduce((n, r) => n + r.unread, 0);

  // ── Auto-scroll (near-bottom guarded) + continuous mark-seen while reading
  //    at the bottom, exactly the Chat Hub's ported pattern. ──────────────────
  const streamRef = useRef<HTMLDivElement | null>(null);
  const forceScrollRef = useRef(true);
  const lastMsgIdRef = useRef<string | null>(null);
  const atBottomRef = useRef(true);
  const onStreamScroll = () => {
    const el = streamRef.current;
    if (el) atBottomRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < NEAR_BOTTOM_PX;
  };
  // Switching rooms is a fresh read — snap to newest and mark it seen.
  useEffect(() => {
    forceScrollRef.current = true;
  }, [selectedKey]);
  useEffect(() => {
    const el = streamRef.current;
    const newest = oldestFirst[oldestFirst.length - 1];
    const newestId = newest?.id ?? null;
    const isNew = newestId !== lastMsgIdRef.current;
    const shouldSnap = forceScrollRef.current || (isNew && atBottomRef.current);
    if (el && shouldSnap) {
      el.scrollTop = el.scrollHeight;
      atBottomRef.current = true;
    }
    // Accrue "seen" only while genuinely caught up — a background refetch the
    // reader has scrolled away from must not silently clear their unread badge.
    if (shouldSnap && newestId) {
      setLastSeen((prev) =>
        prev[selectedKey] === newestId ? prev : { ...prev, [selectedKey]: newestId },
      );
    }
    lastMsgIdRef.current = newestId;
    forceScrollRef.current = false;
  }, [oldestFirst, selectedKey]);

  if (!open) {
    return (
      <aside className="w-9 vibrancy border-l border-overlay/[0.06] flex flex-col items-center shrink-0">
        <button
          title="Show Chats"
          onClick={() => setOpen(true)}
          className="w-7 h-7 mt-2.5 grid place-items-center rounded-md hover:bg-overlay/[0.05] text-text-secondary relative"
        >
          <PanelRightOpen className="w-[15px] h-[15px]" />
          {totalUnread > 0 && (
            <span className="absolute top-1 right-1 w-1.5 h-1.5 rounded-full bg-accent" />
          )}
        </button>
      </aside>
    );
  }

  return (
    <aside className="w-[300px] vibrancy border-l border-overlay/[0.06] flex flex-col shrink-0">
      {/* Header. */}
      <div className="h-12 flex items-center gap-2 px-3.5 border-b border-overlay/[0.06] shrink-0">
        <span className="text-[13px] font-semibold tracking-tight">Chats</span>
        {totalUnread > 0 && (
          <span className="text-[10px] text-text-tertiary">{totalUnread} unread</span>
        )}
        <div className="ml-auto flex items-center gap-0.5 text-text-secondary">
          <button
            title="Open in Chat Hub"
            onClick={onOpenChat}
            className="w-7 h-7 grid place-items-center rounded-md hover:bg-overlay/[0.05]"
          >
            <Maximize2 className="w-[14px] h-[14px]" />
          </button>
          <button
            title="Collapse"
            onClick={() => setOpen(false)}
            className="w-7 h-7 grid place-items-center rounded-md hover:bg-overlay/[0.05]"
          >
            <PanelRightClose className="w-4 h-4" />
          </button>
        </div>
      </div>

      {/* Room switcher — horizontal, tap to switch which room you observe. */}
      <div className="flex items-center gap-1.5 overflow-x-auto scroll-thin px-2.5 py-2 shrink-0">
        {rooms.map((r) => {
          const on = r.key === selectedKey;
          const live = roomIsLive(r, statuses);
          return (
            <button
              key={r.key}
              onClick={() => setSelectedKey(r.key)}
              className={`relative inline-flex items-center gap-1.5 shrink-0 pl-1 pr-2.5 py-1 rounded-full border transition-colors ${
                on
                  ? "border-accent/40 bg-accent/[0.12]"
                  : "border-overlay/[0.08] bg-overlay/[0.02] hover:bg-overlay/[0.05]"
              }`}
            >
              <RoomAvatar room={r} identityOf={identityOf} />
              <span
                className={`text-[11.5px] font-medium truncate max-w-[92px] ${
                  on ? "text-text-primary" : "text-text-secondary"
                }`}
              >
                {roomTitle(r, identityOf)}
              </span>
              {r.unread > 0 ? (
                <span className="min-w-[15px] h-[15px] px-1 rounded-full bg-accent text-white text-[9px] font-bold grid place-items-center">
                  {r.unread}
                </span>
              ) : (
                live && <span className="w-1.5 h-1.5 rounded-full" style={{ backgroundColor: "#30d158" }} />
              )}
            </button>
          );
        })}
      </div>

      {/* Thin active-room meta line. */}
      {activeRoom && (
        <div className="flex items-center gap-1.5 px-3.5 pb-1.5 pt-0.5 shrink-0 text-[11px] text-text-tertiary">
          <Users className="w-[11px] h-[11px]" />
          <span className="truncate">
            <span className="font-semibold text-text-secondary">
              {activeRoom.kind === "channel" ? "#workspace" : roomTitle(activeRoom, identityOf)}
            </span>
            {" · "}
            {activeRoom.kind === "channel"
              ? `${roster.length} members`
              : `${activeRoom.memberIds.length} members`}
            {" · "}
            {(activeRoom.kind === "channel"
              ? roster.filter((t) => statuses[t.instanceId] === "running").length
              : activeRoom.memberIds.filter((id) => statuses[id] === "running").length)}{" "}
            live now
          </span>
        </div>
      )}

      {/* Message stream. */}
      <div
        ref={streamRef}
        onScroll={onStreamScroll}
        className="flex-1 overflow-y-auto scroll-thin px-3 py-3 space-y-2.5 min-h-0"
      >
        {loadError ? (
          <div className="text-[11px] text-danger py-4 text-center">Couldn't load messages</div>
        ) : oldestFirst.length === 0 ? (
          <div className="text-[11px] text-text-tertiary py-4 text-center">No messages yet</div>
        ) : (
          groups.map((g, gi) => {
            const from = identityOf(g.fromInstanceId);
            const prevGroup = groups[gi - 1];
            const showDivider =
              gi === 0 ||
              (prevGroup && dayLabel(prevGroup.items[0].createdAt) !== dayLabel(g.items[0].createdAt));
            return (
              <div key={`${g.fromInstanceId}-${g.items[0].id}`}>
                {showDivider && (
                  <div className="flex items-center gap-2 justify-center py-1">
                    <span className="h-px flex-1 bg-overlay/[0.06]" />
                    <span className="text-[9.5px] text-text-tertiary">
                      {dayLabel(g.items[0].createdAt)}
                    </span>
                    <span className="h-px flex-1 bg-overlay/[0.06]" />
                  </div>
                )}
                <div className="flex gap-2">
                  <Avatar identity={from} />
                  <div className="flex-1 min-w-0 space-y-1">
                    <div className="flex items-center gap-1.5 text-[10.5px]">
                      <span className="font-semibold text-text-secondary">{from.name}</span>
                    </div>
                    {g.items.map((m) => {
                      const to = identityOf(m.toInstanceId);
                      return (
                        <div key={m.id}>
                          <div className="flex items-center gap-1.5 text-[9.5px] text-text-tertiary mb-0.5">
                            {selectedKey === "workspace" && (
                              <>
                                <MoveRight className="w-2.5 h-2.5 shrink-0" />
                                <span>{to.name}</span>
                              </>
                            )}
                            <span className={selectedKey === "workspace" ? "ml-auto" : ""}>
                              {timeHint(m.createdAt)}
                            </span>
                            {m.status === "queued" && <span className="text-warning">queued</span>}
                          </div>
                          <div
                            className="rounded-xl bg-overlay/[0.05] px-3 py-2 text-[12px] text-text-primary"
                            style={{ borderLeft: `2px solid ${from.color}` }}
                          >
                            <ClampText text={m.text} outgoing={false} lines={12} />
                          </div>
                        </div>
                      );
                    })}
                  </div>
                </div>
              </div>
            );
          })
        )}
      </div>

      {/* Read-only footer — this rail observes, it doesn't send. */}
      <div className="shrink-0 px-3.5 py-2 border-t border-overlay/[0.06] flex items-center gap-1.5 text-[10.5px] text-text-tertiary">
        <Eye className="w-3 h-3 shrink-0" />
        <span>Read-only live view — agents reply from their own terminal.</span>
      </div>
    </aside>
  );
}
