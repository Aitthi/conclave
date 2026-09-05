// src/components/builder/useScrollSpy.ts
//
// Highlights the rail item whose section is in view (spec D2). Canon rule 7:
// active = the LAST section whose top has crossed containerHeight/3, with two
// clamps — scrollTop <= 4 forces the first section, at-bottom forces the last.
// jumpTo() smooth-scrolls a section to the top of the container.
//
// The scroll container MUST be positioned (`relative`): jumpTo reads
// `offsetTop`, which is measured against the nearest positioned ancestor.

import { useCallback, useEffect, useState, type RefObject } from "react";

export const SECTION_ATTR = "data-builder-section";

/** Distance kept above a jumped-to section so its heading isn't flush. */
const JUMP_OFFSET = 12;

export function useScrollSpy(
  containerRef: RefObject<HTMLElement | null>,
  ids: string[],
): { activeId: string; jumpTo: (id: string) => void } {
  const [activeId, setActiveId] = useState<string>(ids[0] ?? "");
  // ids is rebuilt each render (positionEnabled toggles one entry), so the
  // effect keys off a primitive rather than the array identity.
  const idKey = ids.join("|");

  useEffect(() => {
    const root = containerRef.current;
    if (!root) return;
    const order = idKey ? idKey.split("|") : [];
    const sections = order
      .map((id) => root.querySelector<HTMLElement>(`[${SECTION_ATTR}="${id}"]`))
      .filter((el): el is HTMLElement => el !== null);
    if (sections.length === 0) return;

    const recompute = () => {
      if (root.scrollTop <= 4) {
        setActiveId(order[0]);
        return;
      }
      if (root.scrollTop >= root.scrollHeight - root.clientHeight - 4) {
        setActiveId(order[order.length - 1]);
        return;
      }
      const rootTop = root.getBoundingClientRect().top;
      const threshold = root.clientHeight / 3;
      let current = order[0];
      for (const el of sections) {
        if (el.getBoundingClientRect().top - rootTop <= threshold) {
          current = el.getAttribute(SECTION_ATTR) ?? current;
        }
      }
      setActiveId(current);
    };

    recompute();
    // The scroll listener is the load-bearing one (it also works in a hidden
    // webview, where rAF never ticks); the observer only catches sections that
    // move because content above them grew — e.g. Advanced expanding.
    const observer = new IntersectionObserver(recompute, {
      root,
      threshold: [0, 0.25, 0.5, 0.75, 1],
    });
    sections.forEach((el) => observer.observe(el));
    root.addEventListener("scroll", recompute, { passive: true });
    return () => {
      observer.disconnect();
      root.removeEventListener("scroll", recompute);
    };
  }, [containerRef, idKey]);

  const jumpTo = useCallback(
    (id: string) => {
      const root = containerRef.current;
      const el = root?.querySelector<HTMLElement>(`[${SECTION_ATTR}="${id}"]`);
      if (!root || !el) return;
      root.scrollTo({
        top: Math.max(0, el.offsetTop - JUMP_OFFSET),
        behavior: "smooth",
      });
      setActiveId(id);
    },
    [containerRef],
  );

  return { activeId, jumpTo };
}
