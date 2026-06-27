// Compact relative-ish time hint from an ISO datetime string ("เมื่อสักครู่" /
// "5 น." / "3 ชม." / "2 วัน"). Best-effort, never throws — falls back to the raw
// string when it can't be parsed. Shared by ContextDrawer and Blackboard.
export function timeHint(iso: string): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso;
  const diffSec = Math.max(0, Math.floor((Date.now() - t) / 1000));
  if (diffSec < 60) return "เมื่อสักครู่";
  if (diffSec < 3600) return `${Math.floor(diffSec / 60)} น.`;
  if (diffSec < 86400) return `${Math.floor(diffSec / 3600)} ชม.`;
  return `${Math.floor(diffSec / 86400)} วัน`;
}
