import { invoke } from "@tauri-apps/api/core";

// ───────────────────────────────────────────────────────────────────────────
// Theme manager
//
// Three preferences: "system" (follow macOS), "light", "dark". The effective
// theme toggles a `.dark` class on <html>, which flips every `--c-*` token in
// app.css. Preference is persisted to localStorage and applied synchronously at
// boot (before React renders) so there's no light-on-dark flash.
// ───────────────────────────────────────────────────────────────────────────

export type ThemePref = "system" | "light" | "dark";

const STORAGE_KEY = "conclave.theme";

const listeners = new Set<(pref: ThemePref) => void>();

function readPref(): ThemePref {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    if (v === "light" || v === "dark" || v === "system") return v;
  } catch {
    // localStorage can throw in locked-down contexts — fall through to default.
  }
  return "system";
}

let currentPref: ThemePref = readPref();

function systemPrefersDark(): boolean {
  return (
    typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-color-scheme: dark)").matches === true
  );
}

/** Resolve the preference to an effective light/dark and apply it to <html>. */
function apply(pref: ThemePref): void {
  if (typeof document === "undefined") return;
  const dark = pref === "dark" || (pref === "system" && systemPrefersDark());
  const root = document.documentElement;
  root.classList.toggle("dark", dark);
  // Native form controls / scrollbars follow the OS hint.
  root.style.colorScheme = dark ? "dark" : "light";
}

export function getThemePref(): ThemePref {
  return currentPref;
}

export function setThemePref(pref: ThemePref): void {
  currentPref = pref;
  try {
    localStorage.setItem(STORAGE_KEY, pref);
  } catch {
    // Persisting is best-effort; the in-memory pref still drives this session.
  }
  apply(pref);
  for (const fn of listeners) fn(pref);
}

/** Subscribe to preference changes (e.g. so a Settings control stays in sync).
 *  Returns an unsubscribe function. */
export function subscribeTheme(fn: (pref: ThemePref) => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

let initialized = false;

/** Apply the persisted theme and start tracking system changes. Idempotent —
 *  the system-change listener is wired at most once. */
export function initTheme(): void {
  apply(currentPref);
  if (initialized) return;
  initialized = true;
  // Re-apply when the OS appearance flips while we're in "system" mode.
  window
    .matchMedia?.("(prefers-color-scheme: dark)")
    .addEventListener("change", () => {
      if (currentPref === "system") apply("system");
    });
}

/** Pull the live macOS system accent color from the Rust host and drive it into
 *  `--mac-accent`. No-ops (keeping the Apple-blue default) outside Tauri or when
 *  the host can't read the accent. */
export function initSystemAccent(): void {
  invoke<string | null>("system_accent")
    .then((hex) => {
      if (hex && /^#[0-9a-fA-F]{6}$/.test(hex)) {
        document.documentElement.style.setProperty("--mac-accent", hex);
      }
    })
    .catch(() => {
      // Non-Tauri context or read failure — the CSS fallback (#0a84ff) stands.
    });
}
