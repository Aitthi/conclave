// src/components/builder/BuilderRail.tsx
//
// Left rail of the Builder (spec D1/D7). One row per section: readiness dot,
// label, accent fill tint when active. Geometry per canon
// design/screens/agent-builder.tsx @ b5d3d20 (checklist rules 3-6).

import type { Readiness, SectionId } from "./readiness";
import { SECTION_LABELS } from "./readiness";

interface BuilderRailProps {
  items: SectionId[];
  readiness: Record<SectionId, Readiness>;
  activeId: string;
  onJump: (id: SectionId) => void;
}

/** Canon rule 5: 7px dot. Position carries a transparent one so every label
 *  stays on the same left edge — it is always valid and shows no state. */
function Dot({ state }: { state: Readiness | "none" }) {
  if (state === "none") return <span className="h-[7px] w-[7px] shrink-0" aria-hidden="true" />;
  const look =
    state === "complete"
      ? "bg-accent"
      : state === "error"
        ? "bg-danger"
        : "bg-transparent ring-1 ring-text-tertiary/70";
  return <span className={`h-[7px] w-[7px] shrink-0 rounded-full ${look}`} aria-hidden="true" />;
}

export function BuilderRail({ items, readiness, activeId, onJump }: BuilderRailProps) {
  return (
    <nav
      aria-label="Builder sections"
      className="w-[180px] shrink-0 space-y-1 border-r border-overlay/[0.06] bg-fill-soft p-2.5"
    >
      {items.map((id) => {
        const active = id === activeId;
        const state: Readiness | "none" = id === "position" ? "none" : readiness[id];
        return (
          <button
            key={id}
            type="button"
            onClick={() => onJump(id)}
            aria-current={active ? "true" : undefined}
            data-readiness={state}
            // Canon rule 6: active = fill tint only. No border-left bar — that
            // is the side-tab antipattern gated by slop-detect and it reads as
            // invisible on dark (Arta challenge 69718db4, ruled accepted).
            className={`flex h-7 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-[12px] transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
              active
                ? "bg-accent/[0.10] font-semibold text-accent"
                : "text-text-secondary hover:bg-overlay/[0.04] hover:text-text-primary"
            }`}
          >
            <Dot state={state} />
            <span className="min-w-0 truncate">{SECTION_LABELS[id]}</span>
          </button>
        );
      })}
    </nav>
  );
}
