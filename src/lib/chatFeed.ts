import type { InterAgentMessage } from "../ipc";

// Absolute clock time for a group header (proto chats.tsx:105-109 — the
// group's own HH:MM, day context comes from the divider, not a per-message
// relative stamp).
export function clockLabel(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", hour12: false });
}

// Day-boundary divider label — honest (a merged feed can span more than
// "today"), not the proto's always-"Today" literal.
export function dayLabel(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const today = new Date();
  if (d.toDateString() === today.toDateString()) return "Today";
  const yesterday = new Date(today);
  yesterday.setDate(yesterday.getDate() - 1);
  if (d.toDateString() === yesterday.toDateString()) return "Yesterday";
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

// Consecutive same-sender grouping, matching the proto's group() — a run of
// messages from the same instance shares one avatar/name header, each message
// keeps its own recipient + timestamp line. Input order is NOT trusted: the
// backend/fixture array is not guaranteed chronological, so the feed sorts by
// parsed createdAt (never lexicographic string compare — fractional-second
// widths vary) before grouping.
export interface MsgGroup {
  fromInstanceId: string;
  items: InterAgentMessage[];
}
export function group(messages: InterAgentMessage[]): MsgGroup[] {
  const sorted = [...messages].sort((a, b) => {
    const ta = new Date(a.createdAt).getTime();
    const tb = new Date(b.createdAt).getTime();
    return (Number.isNaN(ta) ? 0 : ta) - (Number.isNaN(tb) ? 0 : tb);
  });
  const out: MsgGroup[] = [];
  for (const m of sorted) {
    const last = out[out.length - 1];
    if (last && last.fromInstanceId === m.fromInstanceId) last.items.push(m);
    else out.push({ fromInstanceId: m.fromInstanceId, items: [m] });
  }
  return out;
}
