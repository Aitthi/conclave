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
 * Count messages in `roomMessages` (newest-first) newer than the room's
 * last-seen watermark. `seenMarker` may be either the id of the last-seen
 * message (exact — an index cutoff into the newest-first list) or a plain
 * ISO timestamp (fallback for when that message has aged out of the loaded
 * window). Absent entirely means "seen at mount" — never a fake backlog.
 */
function countUnread(roomMessages: InterAgentMessage[], seenMarker: string | undefined): number {
  if (seenMarker === undefined) return 0;
  const idx = roomMessages.findIndex((m) => m.id === seenMarker);
  if (idx !== -1) return idx;
  return roomMessages.filter((m) => m.createdAt > seenMarker).length;
}

/**
 * Derive Phase-1 rooms (R1: #workspace channel + one DM room per pair —
 * there is no channel/group/thread entity backing this, it's a view over
 * pairwise messages) from a newest-first message window plus a client-side
 * last-seen map. Pure — no React, no identity resolution.
 */
export function deriveRooms(
  messages: InterAgentMessage[],
  lastSeen: Record<string, string>,
): Room[] {
  const workspaceRoom: Room = {
    key: WORKSPACE_ROOM_KEY,
    kind: "channel",
    title: "workspace",
    memberIds: [],
    lastAt: messages[0]?.createdAt ?? "",
    unread: countUnread(messages, lastSeen[WORKSPACE_ROOM_KEY]),
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
      unread: countUnread(pairMessages, lastSeen[p.key]),
    };
  });

  return [workspaceRoom, ...dmRooms];
}
