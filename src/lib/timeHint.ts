// Compact relative-ish time hint from an ISO datetime string ("just now" /
// "5m" / "3h" / "2d"). Best-effort, never throws — falls back to the raw
// string when it can't be parsed. Shared by ContextDrawer and Blackboard.
export function timeHint(iso: string): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso;
  const diffSec = Math.max(0, Math.floor((Date.now() - t) / 1000));
  if (diffSec < 60) return "just now";
  if (diffSec < 3600) return `${Math.floor(diffSec / 60)}m`;
  if (diffSec < 86400) return `${Math.floor(diffSec / 3600)}h`;
  return `${Math.floor(diffSec / 86400)}d`;
}
