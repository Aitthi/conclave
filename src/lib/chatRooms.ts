import type { InterAgentMessage } from "../ipc";
import { derivePairs, pairKeyOf } from "./chatPairs";

export type RoomKind = "channel" | "dm";

export interface Room {
  key: string;
  kind: RoomKind;
  /** Display title. Fixed "workspace" for the channel; left empty for a DM —
   *  identity resolution needs the live identities map (React state), so
   *  `ChatRail` builds the DM label from `memberIds` via `identityOf` at
   *  render time rather than duplicating that lookup here. */
  title: string;
  memberIds: string[];
  lastAt: string;
  unread: number;
}

const WORKSPACE_ROOM_KEY = "workspace";

/**
 * Count messages in `roomMessages` newer than the room's last-seen
 * watermark. `seenMarker` is an ISO `createdAt` timestamp (interface ruling:
 * a message id is NOT a valid marker here — once the watermarked message
 * ages out of `useWorkspaceChat`'s MESSAGE_LIMIT window, an id-vs-timestamp
 * comparison is garbage, not just imprecise; see Mellow's F-a). A message
 * with the exact same `createdAt` as the marker counts as seen — acceptable,
 * not a fake backlog. Absent entirely falls back to `mountedAt` — a
 * timestamp the caller captures once when the rail mounts — so a room that
 * first appears mid-session still badges its new messages (plan-review F4;
 * without this a fresh pair room reads as fully-read history and never
 * badges).
 */
function countUnread(
  roomMessages: InterAgentMessage[],
  seenMarker: string | undefined,
  mountedAt: string,
): number {
  const threshold = seenMarker ?? mountedAt;
  return roomMessages.filter((m) => m.createdAt > threshold).length;
}

/**
 * Derive Phase-1 rooms (R1: #workspace channel + one DM room per pair —
 * there is no channel/group/thread entity backing this, it's a view over
 * pairwise messages) from a newest-first message window plus a client-side
 * last-seen map. `mountedAt` is the rail's own mount timestamp (ISO), used as
 * the unread baseline for any room with no explicit `lastSeen` entry yet.
 * Pure — no React, no identity resolution.
 */
export function deriveRooms(
  messages: InterAgentMessage[],
  lastSeen: Record<string, string>,
  mountedAt: string,
): Room[] {
  const workspaceRoom: Room = {
    key: WORKSPACE_ROOM_KEY,
    kind: "channel",
    title: "workspace",
    memberIds: [],
    lastAt: messages[0]?.createdAt ?? "",
    unread: countUnread(messages, lastSeen[WORKSPACE_ROOM_KEY], mountedAt),
  };

  const dmRooms: Room[] = derivePairs(messages).map((p) => {
    const pairMessages = messages.filter(
      (m) => pairKeyOf(m.fromInstanceId, m.toInstanceId) === p.key,
    );
    return {
      key: p.key,
      kind: "dm",
      title: "",
      memberIds: [p.aId, p.bId],
      lastAt: p.lastAt,
      unread: countUnread(pairMessages, lastSeen[p.key], mountedAt),
    };
  });

  return [workspaceRoom, ...dmRooms];
}
