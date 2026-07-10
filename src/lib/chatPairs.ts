import type { InterAgentMessage } from "../ipc";

/** Canonical key for an unordered conversation pair: the two instance ids
 *  joined with '|', lexicographically smaller id first. */
export function pairKeyOf(a: string, b: string): string {
  return a < b ? `${a}|${b}` : `${b}|${a}`;
}

/** Epoch ms of a message `createdAt` for chronological comparison — always
 *  parse-then-compare, never lexicographic string compare (fractional-second
 *  digit widths vary; same rule as chatFeed's `group`). An unparseable
 *  timestamp sorts oldest, so it can never displace a valid newest. */
export function createdAtMs(iso: string): number {
  const t = new Date(iso).getTime();
  return Number.isNaN(t) ? Number.NEGATIVE_INFINITY : t;
}

export interface ChatPair {
  key: string;
  /** Lexicographically first id — renders on the LEFT in the pair view. */
  aId: string;
  bId: string;
  /** createdAt of the pair's chronologically newest loaded message. */
  lastAt: string;
}

/** Distinct conversation pairs in a message window of ANY order, sorted by
 *  most-recent activity. The window is NOT assumed newest-first — the default
 *  fixture deliberately returns a non-chronological shuffle as a regression
 *  tripwire — so `lastAt` is derived by comparing parsed `createdAt`s, never
 *  from array position. Pure — trivially testable if a FE harness lands. */
export function derivePairs(messages: InterAgentMessage[]): ChatPair[] {
  const seen = new Map<string, ChatPair>();
  for (const m of messages) {
    const key = pairKeyOf(m.fromInstanceId, m.toInstanceId);
    const pair = seen.get(key);
    if (!pair) {
      const [aId, bId] = key.split("|") as [string, string];
      seen.set(key, { key, aId, bId, lastAt: m.createdAt });
    } else if (createdAtMs(m.createdAt) > createdAtMs(pair.lastAt)) {
      pair.lastAt = m.createdAt;
    }
  }
  return [...seen.values()].sort((a, b) => createdAtMs(b.lastAt) - createdAtMs(a.lastAt));
}
