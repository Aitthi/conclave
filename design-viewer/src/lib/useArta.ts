import { useCallback, useEffect, useRef, useState } from "react";
import type { ArtaState } from "./types";
import { nowLabel } from "./utils";

export interface ChangeEntry {
  kind: string;
  label: string;
  id?: string;
  at: string;
}

export interface Project {
  id: string;
  name: string;
}

interface ArtaLive {
  data: ArtaState | null;
  error: string | null;
  updatedAt: string;
  flashing: boolean;
  /** Recent edits the AI made, newest first — for the "what changed" feed. */
  changes: ChangeEntry[];
  /** Replace state locally (the Edit-state drawer) and flash, like the design. */
  applyLocal: (next: ArtaState) => void;
  /** Projects this one viewer can show (home first); >1 only with multiple canvases. */
  projects: Project[];
  /** The project currently shown — persisted in localStorage, falls back to the first. */
  activeProject: string;
  selectProject: (id: string) => void;
  /** The project this viewer was launched for — always present, can't be removed. */
  home: string;
  /** Drop a project from the switcher (unregister only; the .arta/ files are untouched). */
  deleteProject: (id: string) => void;
}

const STORAGE_KEY = "arta-project";

// The active project id, mirrored outside React so the fire-and-forget reporters
// below (runtime / feedback / snapshot) tag their writes with the right project —
// they run from places that don't have the hook's state.
let activeProjectId = "";

// Pick the project to show: the one saved in localStorage if it still exists,
// otherwise the first project in the list (per the agreed fallback). Exported so the
// eval gate can lock this rule.
export function resolveActive(list: Project[], stored: string | null): string {
  if (stored && list.some((p) => p.id === stored)) return stored;
  return list[0]?.id ?? "";
}

// A one-shot ?project=<id> deep link is a handoff from the agent's start-viewer URL,
// not durable state. Capture it ONCE at module load — not inside the effect — because
// the viewer runs as a dev server under React StrictMode, which mounts and double-fires
// effects; reading-and-stripping in the effect would clear the param on the first run
// and leave the surviving run with nothing (the param would be lost to localStorage).
// Stripping is deferred until the pick is actually applied (stripProjectParam below).
const URL_PROJECT = (() => {
  try {
    return new URLSearchParams(window.location.search).get("project");
  } catch {
    return null;
  }
})();

// Drop ?project from the address bar so a reload falls back to the persisted pick.
function stripProjectParam(): void {
  try {
    const url = new URL(window.location.href);
    if (!url.searchParams.has("project")) return;
    url.searchParams.delete("project");
    window.history.replaceState(null, "", url.pathname + url.search + url.hash);
  } catch {
    /* ignore */
  }
}

// The client half of the live loop: fetch the project list + the active project's
// state, then listen for the events the Vite plugin fires whenever a watched .arta/
// changes on disk (i.e. whenever the AI writes to it). Events are tagged with a
// project id; we apply only the active project's. Each applied update flashes the
// canvas cyan — the visible "the agent just wrote in" signal.
export function useArta(): ArtaLive {
  const [data, setData] = useState<ArtaState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [updatedAt, setUpdatedAt] = useState<string>(nowLabel());
  const [flashing, setFlashing] = useState(false);
  const [changes, setChanges] = useState<ChangeEntry[]>([]);
  const [projects, setProjects] = useState<Project[]>([]);
  const [activeProject, setActiveProject] = useState<string>("");
  const [home, setHome] = useState<string>("");
  const flashTimer = useRef<number | undefined>(undefined);

  const flash = useCallback(() => {
    setFlashing(true);
    window.clearTimeout(flashTimer.current);
    flashTimer.current = window.setTimeout(() => setFlashing(false), 720);
  }, []);

  const ingest = useCallback(
    (parsed: ArtaState | null) => {
      try {
        if (!parsed || !parsed.meta) throw new Error('missing "meta"');
        setData(parsed);
        setError(null);
        setUpdatedAt(nowLabel());
        flash();
      } catch (e) {
        setError(`state.json is invalid — ${(e as Error).message}`);
      }
    },
    [flash]
  );

  // Load one project's assembled state into the canvas (clears the stale view first).
  const loadState = useCallback((id: string) => {
    setChanges([]);
    fetch(`/__arta/state?project=${encodeURIComponent(id)}`)
      .then((r) => r.json())
      .then((res: { ok: boolean; state: ArtaState | null; error?: string }) => {
        if (res.ok && res.state) {
          setData(res.state);
          setError(null);
          setUpdatedAt(nowLabel());
        } else if (res.ok) {
          setData(null);
        } else {
          setError(res.error || "could not read state.json");
        }
      })
      .catch(() => setError("dev server unreachable"));
  }, []);

  // Switch the shown project: remember it and load it. Ignored for an unknown id.
  const selectProject = useCallback(
    (id: string) => {
      if (id === activeProjectId) return;
      activeProjectId = id;
      setActiveProject(id);
      try {
        localStorage.setItem(STORAGE_KEY, id);
      } catch {
        /* private mode — fine, just won't persist */
      }
      loadState(id);
    },
    [loadState]
  );

  // Drop a project from the switcher. Unregisters server-side (the .arta/ files stay);
  // the home project can't be removed. If the active one is dropped, fall back to the
  // first that remains so the canvas never goes blank.
  const deleteProject = useCallback(
    (id: string) => {
      if (id === home) return;
      setProjects((prev) => {
        const next = prev.filter((p) => p.id !== id);
        if (id === activeProjectId) {
          const fallback = resolveActive(next, null);
          if (fallback) selectProject(fallback);
        }
        return next;
      });
      fetch(`/__arta/projects?project=${encodeURIComponent(id)}`, { method: "DELETE" }).catch(() => {});
    },
    [home, selectProject]
  );

  const applyActive = useCallback(
    (list: Project[]) => {
      setProjects(list);
      const stored = (() => {
        try {
          return localStorage.getItem(STORAGE_KEY);
        } catch {
          return null;
        }
      })();
      const next = resolveActive(list, stored || activeProjectId);
      if (next && next !== activeProjectId) {
        activeProjectId = next;
        setActiveProject(next);
        loadState(next);
      } else if (next && !activeProjectId) {
        activeProjectId = next;
        setActiveProject(next);
      }
    },
    [loadState]
  );

  useEffect(() => {
    let alive = true;
    fetch("/__arta/projects")
      .then((r) => r.json())
      .then((res: { ok: boolean; projects?: Project[]; home?: string }) => {
        if (!alive) return;
        const list = res.projects || [];
        setHome(res.home || "");
        // Precedence: a valid ?project deep link wins, else the persisted pick, else first.
        const stored = (() => { try { return localStorage.getItem(STORAGE_KEY); } catch { return null; } })();
        const active = URL_PROJECT && list.some((p) => p.id === URL_PROJECT) ? URL_PROJECT : resolveActive(list, stored);
        activeProjectId = active;
        setProjects(list);
        setActiveProject(active);
        if (active) {
          try { localStorage.setItem(STORAGE_KEY, active); } catch { /* private mode */ }
          loadState(active);
        }
        // The deep link is consumed now — clear it so a reload uses the persisted pick.
        if (URL_PROJECT) stripProjectParam();
      })
      .catch(() => alive && setError("dev server unreachable"));

    const asObj = (raw: unknown) => (typeof raw === "string" ? JSON.parse(raw) : raw);
    if (import.meta.hot) {
      import.meta.hot.on("arta:update", (raw: unknown) => {
        try {
          const env = asObj(raw) as { project: string; state: ArtaState };
          if (env.project === activeProjectId) ingest(env.state);
        } catch {
          /* ignore malformed push */
        }
      });
      import.meta.hot.on("arta:change", (raw: unknown) => {
        try {
          const c = asObj(raw) as ChangeEntry & { project: string };
          if (c.project === activeProjectId)
            setChanges((prev) => [{ kind: c.kind, label: c.label, id: c.id, at: nowLabel() }, ...prev].slice(0, 30));
        } catch {
          /* ignore */
        }
      });
      import.meta.hot.on("arta:projects", (raw: unknown) => {
        try {
          applyActive(asObj(raw) as Project[]);
        } catch {
          /* ignore */
        }
      });
    }
    return () => {
      alive = false;
      window.clearTimeout(flashTimer.current);
    };
  }, [ingest, loadState, applyActive]);

  const applyLocal = useCallback(
    (next: ArtaState) => {
      setData(next);
      setError(null);
      setUpdatedAt(nowLabel());
      flash();
    },
    [flash]
  );

  return { data, error, updatedAt, flashing, changes, applyLocal, projects, activeProject, selectProject, home, deleteProject };
}

// Fire-and-forget reporters so the MCP server can see what the dev is doing. Each is
// tagged with the active project so the write lands in that project's .arta/.
const tag = (path: string) => (activeProjectId ? `${path}?project=${encodeURIComponent(activeProjectId)}` : path);

export function reportRuntime(body: Record<string, unknown>): void {
  fetch(tag("/__arta/runtime"), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  }).catch(() => {});
}

export function sendFeedback(body: Record<string, unknown>): Promise<boolean> {
  return fetch(tag("/__arta/feedback"), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  })
    .then((r) => r.ok)
    .catch(() => false);
}

export function reportSnapshot(screen: string, dataUrl: string, full = false): void {
  fetch(tag("/__arta/snapshot"), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ screen, dataUrl, full }),
  }).catch(() => {});
}
