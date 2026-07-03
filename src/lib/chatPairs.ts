import type { InterAgentMessage } from "../ipc";

/** Canonical key for an unordered conversation pair: the two instance ids
 *  joined with '|', lexicographically smaller id first. */
export function pairKeyOf(a: string, b: string): string {
  return a < b ? `${a}|${b}` : `${b}|${a}`;
}

export interface ChatPair {
  key: string;
  /** Lexicographically first id — renders on the LEFT in the pair view. */
  aId: string;
  bId: string;
  /** createdAt of the pair's newest loaded message. */
  lastAt: string;
}

/** Distinct conversation pairs in a newest-first message window, ordered by
 *  most-recent activity. Pure — trivially testable if a FE harness lands. */
export function derivePairs(messages: InterAgentMessage[]): ChatPair[] {
  const seen = new Map<string, ChatPair>();
  for (const m of messages) {
    const key = pairKeyOf(m.fromInstanceId, m.toInstanceId);
    if (!seen.has(key)) {
      // First sighting in a newest-first list IS the pair's newest message,
      // so insertion order is already most-recent-first.
      const [aId, bId] = key.split("|") as [string, string];
      seen.set(key, { key, aId, bId, lastAt: m.createdAt });
    }
  }
  return [...seen.values()];
}
