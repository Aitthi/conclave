// Arta-owned platform guarantees for a proto screen document — ported from the old
// srcdoc-iframe runtime (`src/components/proto/FreeformDevice.tsx`'s `RUNTIME` string,
// lines 42-172) and the theme boot half of `HEAD_LIBS` (`src/lib/screenDoc.ts:94`), now
// as real TS running in the proto entry's own module (no `data-*`/mock-store/CDN-script
// model — screens are real React and navigate via react-router-dom).
//
// Deliberately NOT ported: the mock store (`window.__STORE__`, `data-bind`/`data-show`/
// `data-inc`/`data-dec`/`data-set`, `mutate()`, `render()`), the `data-to` nav
// interception (real screens use `<Link>`/`useNavigate` — routing is React Router's job
// now), the raw-href warning sweep, the lucide-icon-name warning sweep (screens import
// `lucide-react` components directly, no `<i data-lucide>` placeholders), `markNav()`.

export interface AnnotateTarget {
  tag: string;
  text: string;
  selector: string;
}

// postMessage helper — every shell→parent message carries this envelope.
function up(msg: Record<string, unknown>): void {
  parent.postMessage(Object.assign({ source: "arta-frame" }, msg), "*");
}

// Single implementation of the error-message shape, shared by the global error/
// rejection/console.error forwarding below AND by Shell's ScreenHost ErrorBoundary
// (which catches a screen's render-time throw — a different path than window.onerror,
// since React swallows render errors before they'd otherwise reach it).
export function reportError(message: string): void {
  up({ type: "error", message });
}

// ---- error forwarding — verbatim logic from FreeformDevice.tsx:81-83 ----------------
window.addEventListener("error", (e) => {
  reportError(e.message + (e.filename ? ` @ ${e.filename}:${e.lineno}` : ""));
});
window.addEventListener("unhandledrejection", (e) => {
  const reason = e.reason as { message?: string } | undefined;
  reportError(`unhandled rejection: ${reason?.message ?? e.reason}`);
});
const _consoleError = console.error;
console.error = (...args: unknown[]) => {
  try {
    reportError(args.map(String).join(" "));
  } catch {
    /* never let forwarding itself throw */
  }
  _consoleError.apply(console, args);
};

// ---- annotate mode — verbatim logic from FreeformDevice.tsx:118-130 -----------------
let annotate = false;

function describe(el: Element): AnnotateTarget {
  let sel = el.tagName.toLowerCase();
  if (el.id) sel += `#${el.id}`;
  else if (typeof el.className === "string" && el.className.trim()) {
    const c0 = el.className.trim().split(/\s+/)[0];
    if (c0) sel += `.${c0}`;
  }
  return { tag: el.tagName.toLowerCase(), text: (el.textContent || "").trim().slice(0, 80), selector: sel };
}

// Capture-phase so annotate clicks pre-empt any navigation the screen would otherwise do.
document.addEventListener(
  "click",
  (e) => {
    if (!annotate) return;
    e.preventDefault();
    e.stopPropagation();
    up({ type: "annotate", target: describe(e.target as Element) });
  },
  true
);

// ---- theme boot + toggle — port of the HEAD_LIBS theme IIFE (screenDoc.ts:94) -------
// Runs synchronously at module load (not deferred) so the initial theme is applied
// before first paint — no flash of the wrong theme.
const THEME_KEY = "arta-theme";
const root = document.documentElement;

function getSavedTheme(): string | null {
  try {
    return localStorage.getItem(THEME_KEY);
  } catch {
    return null;
  }
}
function saveTheme(v: string): void {
  try {
    localStorage.setItem(THEME_KEY, v);
  } catch {
    /* storage unavailable — theme just won't persist across reloads */
  }
}
function setTheme(t: string): void {
  const dark = t === "dark";
  root.classList.toggle("dark", dark);
  root.setAttribute("data-theme", t);
  root.style.colorScheme = dark ? "dark" : "light";
}
setTheme(getSavedTheme() || (window.matchMedia("(prefers-color-scheme:dark)").matches ? "dark" : "light"));

document.addEventListener("click", (e) => {
  const target = e.target as Element | null;
  const btn = target?.closest?.("[data-theme-toggle]");
  if (!btn) return;
  e.preventDefault();
  const next = root.classList.contains("dark") ? "light" : "dark";
  setTheme(next);
  saveTheme(next);
});

// ---- shell↔parent wiring -------------------------------------------------------------

// Sets up the parent→shell half of the postMessage contract. `nav` dispatches to the
// router; `annotate` is fully self-contained here (a CSS-class + click-interception
// concern), mirroring the old RUNTIME's own `message` listener (FreeformDevice.tsx:157-161).
//
// Guarded to wire the listener at most once: Shell calls this from a `useEffect` in
// React StrictMode, which double-invokes effects in dev (mount → cleanup → mount) —
// without this guard that would attach two listeners and double-handle every message.
let bridgeInitialized = false;
export function initBridge(handlers: { onNav(to: string): void }): void {
  if (bridgeInitialized) return;
  bridgeInitialized = true;
  window.addEventListener("message", (e: MessageEvent) => {
    const d = e.data;
    if (!d || d.source !== "arta-parent") return;
    if (d.type === "nav" && typeof d.to === "string") {
      handlers.onNav(d.to);
    } else if (d.type === "annotate") {
      annotate = !!d.on;
      document.body.classList.toggle("arta-annotate", annotate);
    }
  });
}

// Signals the headless-ready contract: `document.body.dataset.artaReady` for
// screenshot/export tooling, plus a `ready` postMessage for the live viewer.
export function reportReady(screen: string): void {
  document.body.dataset.artaReady = "1";
  up({ type: "ready", screen });
}
