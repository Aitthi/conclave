import { useEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";

// ---------------------------------------------------------------------------
// Native file-drop → composer path insertion.
//
// Dragging a file onto a browser drop zone yields an HTML5 `File` whose real
// filesystem path is hidden for security — useless for a CLI that wants a path.
// Tauri's OS-level drag-drop (`dragDropEnabled`, on by default) DOES carry the
// absolute paths, but only via one window-level event, not per-element DOM
// events. So we keep a registry of drop targets and a SINGLE subscription that
// hit-tests the drop position against them — mirroring how Terminal.app inserts
// the shell-quoted path of a file you drag into it.
// ---------------------------------------------------------------------------

interface DropTarget {
  el: HTMLElement;
  onPaths: (paths: string[]) => void;
  setOver: (over: boolean) => void;
}

const targets = new Set<DropTarget>();
let subscribed = false;
let currentOver: DropTarget | null = null;

/**
 * Quote a path for a POSIX shell (zsh) the way a drag-into-terminal would: a
 * path made only of safe characters is inserted bare; anything else is
 * single-quoted (with embedded single quotes escaped as '\''). zsh — and CLIs
 * like Claude Code that read the line — accept this verbatim.
 */
export function shellQuotePath(p: string): string {
  if (/^[A-Za-z0-9_@%+=:,./-]+$/.test(p)) return p;
  return "'" + p.replace(/'/g, "'\\''") + "'";
}

// Tauri reports the drop point in PHYSICAL pixels; the DOM hit-test wants CSS
// pixels. On a 2× Retina display those differ by devicePixelRatio.
function toCssPoint(x: number, y: number): { x: number; y: number } {
  const r = window.devicePixelRatio || 1;
  return { x: x / r, y: y / r };
}

function findTarget(x: number, y: number): DropTarget | null {
  let el = document.elementFromPoint(x, y) as HTMLElement | null;
  while (el) {
    for (const t of targets) {
      if (t.el === el) return t;
    }
    el = el.parentElement;
  }
  return null;
}

function clearOver() {
  currentOver?.setOver(false);
  currentOver = null;
}

// Lazily attach the one window-level subscription the first time a target
// registers. Silently no-ops outside a Tauri webview (plain `vite` dev).
function ensureSubscribed() {
  if (subscribed) return;
  subscribed = true;
  getCurrentWebview()
    .onDragDropEvent((event) => {
      const p = event.payload;
      if (p.type === "over") {
        const { x, y } = toCssPoint(p.position.x, p.position.y);
        const t = findTarget(x, y);
        if (t !== currentOver) {
          currentOver?.setOver(false);
          currentOver = t;
          t?.setOver(true);
        }
      } else if (p.type === "drop") {
        const { x, y } = toCssPoint(p.position.x, p.position.y);
        const t = findTarget(x, y) ?? currentOver;
        clearOver();
        if (t && p.paths.length > 0) t.onPaths(p.paths);
      } else {
        // "leave" / cancel
        clearOver();
      }
    })
    .catch(() => {
      // Not a Tauri webview — allow a later retry if one ever mounts.
      subscribed = false;
    });
}

/**
 * Register `el` (via the returned ref) as a native file-drop target. When files
 * are dropped onto it, `onPaths` receives their absolute paths. `isOver` tracks
 * whether a drag is currently hovering this target, for a highlight.
 */
export function useFileDrop<T extends HTMLElement = HTMLDivElement>(
  onPaths: (paths: string[]) => void,
): { ref: React.RefObject<T | null>; isOver: boolean } {
  const ref = useRef<T | null>(null);
  const [isOver, setIsOver] = useState(false);
  // Keep the latest callback without re-registering the target each render.
  const cbRef = useRef(onPaths);
  cbRef.current = onPaths;

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const target: DropTarget = {
      el,
      onPaths: (paths) => cbRef.current(paths),
      setOver: setIsOver,
    };
    targets.add(target);
    ensureSubscribed();
    return () => {
      targets.delete(target);
      if (currentOver === target) currentOver = null;
    };
  }, []);

  return { ref, isOver };
}
