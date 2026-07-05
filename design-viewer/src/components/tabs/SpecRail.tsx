import { ChevronLeft, ChevronRight, FileText } from "lucide-react";
import type { Spec } from "../../lib/types";
import { MONO, useTheme } from "../../lib/theme";
import { alpha } from "../../lib/utils";

// The right rail of the Prototype tab: the spec lives next to the live
// wireframe so the dev sees *why* a screen exists while clicking through it.
// Collapses to a thin vertical "SPEC" strip.
export function SpecRail({
  spec,
  open,
  onToggle,
}: {
  spec: Spec;
  open: boolean;
  onToggle: () => void;
}) {
  const { c } = useTheme();

  if (!open) {
    return (
      <button
        onClick={onToggle}
        className="flex w-[38px] shrink-0 cursor-pointer flex-col items-center gap-2.5 border-l pt-3.5 transition-colors hover:bg-[#1c1c20]"
        style={{ borderColor: c.border, background: c.panel }}
      >
        <ChevronLeft size={16} color={c.dim} />
        <span
          className="text-[10.5px] uppercase tracking-[1.4px]"
          style={{ writingMode: "vertical-rl", fontFamily: MONO, color: c.dim }}
        >
          Spec
        </span>
      </button>
    );
  }

  const chip = (txt: string, col: string, key: React.Key) => (
    <span
      key={key}
      className="inline-block max-w-full whitespace-normal [overflow-wrap:anywhere] rounded-[14px] px-[9px] py-[3px] text-[11px] leading-snug"
      style={{ fontFamily: MONO, border: `1px solid ${alpha(col, 0.4)}`, background: alpha(col, 0.1), color: col }}
    >
      {txt}
    </span>
  );
  const chipRow = (arr: readonly unknown[] | undefined, col: string) => (
    <div className="flex flex-wrap gap-1.5">{(arr || []).map((x, i) => chip(toLabel(x), col, i))}</div>
  );
  const mini = (label: string, body: React.ReactNode) => (
    <div>
      <div
        className="mb-[9px] text-[9.5px] font-medium uppercase tracking-[0.8px]"
        style={{ fontFamily: MONO, color: c.faint }}
      >
        {label}
      </div>
      {body}
    </div>
  );

  return (
    <div
      className="flex w-[312px] shrink-0 flex-col overflow-y-auto overflow-x-hidden border-l"
      style={{ borderColor: c.border, background: c.panel }}
    >
      <div
        className="sticky top-0 z-[1] flex items-center gap-2 border-b px-4 py-[13px]"
        style={{ borderColor: c.border, background: c.panel }}
      >
        <FileText size={14} color={c.accent} />
        <span
          className="text-[10.5px] font-medium uppercase tracking-[0.8px]"
          style={{ fontFamily: MONO, color: c.dim }}
        >
          Spec
        </span>
        <span className="text-[11px]" style={{ color: c.faint }}>
          · why this screen exists
        </span>
        <button
          onClick={onToggle}
          className="ml-auto flex rounded-[5px] p-0.5 transition-opacity hover:opacity-70"
          style={{ color: c.faint }}
        >
          <ChevronRight size={16} />
        </button>
      </div>

      <div className="flex flex-col gap-5 px-4 py-[18px]">
        {mini(
          "Goal",
          <div className="text-[14px] font-medium leading-[1.45]" style={{ color: c.text }}>
            {spec.goal || ""}
          </div>
        )}
        {mini("Users", <UsersBlock users={spec.users} />)}
        {mini(
          "User stories",
          <div className="flex flex-col gap-2.5">
            {(spec.userStories || []).map((st, i) => (
              <div key={i} className="flex gap-2 text-[12px] leading-[1.5]" style={{ color: c.dim }}>
                <ChevronRight size={12} color={c.accent} className="mt-0.5 shrink-0" />
                {toLabel(st)}
              </div>
            ))}
          </div>
        )}
        {mini(
          "Scope",
          <div className="flex flex-col gap-[11px]">
            <div>
              <div className="mb-[7px] text-[9px] tracking-[1px]" style={{ fontFamily: MONO, color: c.green }}>
                IN
              </div>
              {chipRow(spec.scope?.in, c.green)}
            </div>
            <div>
              <div className="mb-[7px] text-[9px] tracking-[1px]" style={{ fontFamily: MONO, color: c.red }}>
                OUT
              </div>
              {chipRow(spec.scope?.out, c.red)}
            </div>
          </div>
        )}
        {mini(
          "Constraints",
          <div className="flex flex-col gap-2">
            {(spec.constraints || []).map((cons, i) => (
              <div key={i} className="flex gap-2 text-[11.5px] leading-[1.45]" style={{ color: c.faint }}>
                <span style={{ color: c.border }}>•</span>
                {toLabel(cons)}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

// The AI writes spec freely, so an array item might be a plain string or a richer
// object (a user as { name, need }, a scope item as { label }, …). Never pass a raw
// object to JSX — React throws "Objects are not valid as a React child" and the rail
// (and, without a boundary, the whole viewer) crashes. Reduce anything to a label.
function toLabel(x: unknown): string {
  if (x == null) return "";
  if (typeof x === "string") return x;
  if (typeof x === "number" || typeof x === "boolean") return String(x);
  if (typeof x === "object") {
    const o = x as Record<string, unknown>;
    for (const k of ["name", "label", "title", "role", "text", "value"]) {
      if (typeof o[k] === "string") return o[k] as string;
    }
  }
  return "";
}

// Users may arrive as ["Host", …] or as [{ name, need }, …] (a common, reasonable
// shape the schema doesn't forbid). Show the richer form — name + the need it implies
// — when any entry carries one; fall back to plain chips otherwise.
function UsersBlock({ users }: { users?: readonly unknown[] }) {
  const { c } = useTheme();
  const rows = (users || []).map((u) => {
    if (u && typeof u === "object") {
      const o = u as Record<string, unknown>;
      const need = o.need ?? o.goal ?? o.desc ?? o.description ?? o.want;
      return { name: toLabel(u), need: typeof need === "string" ? need : undefined };
    }
    return { name: toLabel(u), need: undefined as string | undefined };
  });
  if (!rows.length) return null;

  if (!rows.some((r) => r.need)) {
    return (
      <div className="flex flex-wrap gap-1.5">
        {rows.map((r, i) => (
          <span
            key={i}
            className="inline-block max-w-full whitespace-normal [overflow-wrap:anywhere] rounded-[14px] px-[9px] py-[3px] text-[11px] leading-snug"
            style={{
              fontFamily: MONO,
              border: `1px solid ${alpha(c.accent, 0.4)}`,
              background: alpha(c.accent, 0.1),
              color: c.accent,
            }}
          >
            {r.name}
          </span>
        ))}
      </div>
    );
  }
  return (
    <div className="flex flex-col gap-2.5">
      {rows.map((r, i) => (
        <div key={i}>
          <div className="text-[12px] font-medium leading-[1.4]" style={{ color: c.text }}>
            {r.name}
          </div>
          {r.need && (
            <div className="mt-0.5 text-[11.5px] leading-[1.45]" style={{ color: c.faint }}>
              {r.need}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
