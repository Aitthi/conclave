// src/components/builder/Section.tsx
//
// One Builder section: the scroll-spy anchor, the uppercase heading, and an
// optional right slot for heading actions (e.g. "No role · Custom…").
// Canon rule 9: sections are separated by a hairline — every section after the
// first carries `mt-5 border-t pt-5`, so the first one passes `first`.

import type { ReactNode } from "react";
import type { SectionId } from "./readiness";
import { SECTION_ATTR } from "./useScrollSpy";

interface SectionProps {
  id: SectionId;
  title: string;
  actions?: ReactNode;
  first?: boolean;
  children: ReactNode;
}

export function Section({ id, title, actions, first = false, children }: SectionProps) {
  return (
    <section
      {...{ [SECTION_ATTR]: id }}
      aria-labelledby={`builder-${id}-heading`}
      className={first ? "" : "mt-5 border-t border-overlay/[0.06] pt-5"}
    >
      <div className="mb-2.5 flex items-center justify-between gap-3">
        <div
          id={`builder-${id}-heading`}
          className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase"
        >
          {title}
        </div>
        {actions}
      </div>
      {children}
    </section>
  );
}
