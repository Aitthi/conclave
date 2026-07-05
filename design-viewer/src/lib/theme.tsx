import { createContext, useContext, useEffect, useMemo, useState } from "react";

// The viewer chrome palette — a Vercel "Geist" design system. The whole UI is
// driven by these tokens, in both dark (default) and light mode. `accent` is the
// one freely themeable value; it's exposed both as a JS object (for inline-styled
// chrome that needs dynamic alpha tints) and as the --color-accent CSS var (so
// Tailwind's accent utility classes track the picker too).
//
// Field names are kept stable across the codebase: the original `bg/panel/card/
// panel2/border/text/dim/faint/...` set maps onto Geist's bg / elevations / fg
// ramp, and the newer `bgElev2/border2/borderStrong/accent2/accentSoft/invBg/
// invFg/...` fields expose the rest of the Geist scale for components that want it.
export type Mode = "dark" | "light";

export interface DarkTokens {
  // surfaces
  bg: string;
  panel: string; // = bg-elev
  card: string; // = bg-elev-2
  panel2: string; // = bg-hover
  bgElev: string;
  bgElev2: string;
  bgHover: string;
  // borders
  border: string;
  borderSoft: string;
  border2: string;
  borderStrong: string;
  // foreground ramp
  text: string; // = fg
  dim: string; // = fg-2
  faint: string; // = fg-3
  // accent
  accent: string;
  accent2: string;
  accentSoft: string;
  // inverted (primary buttons, logo, done-step)
  invBg: string;
  invFg: string;
  // semantic
  green: string;
  amber: string;
  red: string;
  yellow: string;
  blue: string;
  purple: string;
  teal: string;
  // misc
  shadow: string;
  gridDot: string;
}

export interface LightTokens {
  bg: string;
  fg: string;
  mutedFg: string;
  faint: string;
  border: string;
  muted: string;
  primary: string;
  primaryFg: string;
}

// The prototype always renders inside a real device frame on a white "screen",
// independent of the viewer's own theme — these tokens style that device shell.
export const LIGHT: LightTokens = {
  bg: "#ffffff",
  fg: "#171717",
  mutedFg: "#5f5f5f",
  faint: "#969696",
  border: "#ebebeb",
  muted: "#fafafa",
  primary: "#171717",
  primaryFg: "#ffffff",
};

const DARK = (accent: string): DarkTokens => ({
  bg: "#000000",
  panel: "#0a0a0a",
  card: "#141414",
  panel2: "#1c1c1c",
  bgElev: "#0a0a0a",
  bgElev2: "#141414",
  bgHover: "#1c1c1c",
  border: "#242424",
  borderSoft: "#1c1c1c",
  border2: "#2e2e2e",
  borderStrong: "#3a3a3a",
  text: "#ededed",
  dim: "#a0a0a0",
  faint: "#6e6e6e",
  accent,
  accent2: "#52a8ff",
  accentSoft: "rgba(0,112,243,.14)",
  invBg: "#ededed",
  invFg: "#000000",
  green: "#5bbb6e",
  amber: "#f5b14c",
  red: "#ff5f56",
  yellow: "#f5b14c",
  blue: "#52a8ff",
  purple: "#c08cf0",
  teal: "#3ad0c0",
  shadow: "0 1px 2px rgba(0,0,0,.6),0 8px 24px rgba(0,0,0,.45)",
  gridDot: "rgba(255,255,255,.035)",
});

// Light/airy "Linear/Vercel/Figma" redesign — white surfaces on a soft #f7f8f8
// page, hairline #ececec borders, generous whitespace, emerald accent. This is
// the default the viewer ships in; dark stays available behind the theme toggle.
const LIGHTC = (accent: string): DarkTokens => ({
  bg: "#f7f8f8",
  panel: "#ffffff",
  card: "#ffffff",
  panel2: "#f3f4f4",
  bgElev: "#ffffff",
  bgElev2: "#ffffff",
  bgHover: "#f2f3f3",
  border: "#ececec",
  borderSoft: "#f0f0f0",
  border2: "#e6e7e7",
  borderStrong: "#cdd0d2",
  text: "#1f2328",
  dim: "#5e6168",
  faint: "#9a9da3",
  accent,
  accent2: "#0b6b3f",
  accentSoft: "rgba(16,185,129,.10)",
  invBg: "#0f1115",
  invFg: "#ffffff",
  green: "#10b981",
  amber: "#d97706",
  red: "#dc2626",
  yellow: "#c77b00",
  blue: "#0070f3",
  purple: "#7c3aed",
  teal: "#0d9488",
  shadow: "0 1px 2px rgba(15,17,21,.04),0 8px 24px -8px rgba(15,17,21,.10)",
  gridDot: "rgba(15,17,21,.05)",
});

export function tokens(mode: Mode, accent: string): DarkTokens {
  return mode === "light" ? LIGHTC(accent) : DARK(accent);
}

// Kept for any caller that still wants the dark set directly.
export function darkTokens(accent: string): DarkTokens {
  return DARK(accent);
}

export const ACCENT_PRESETS = ["#10b981", "#0070f3", "#7c3aed", "#f59e0b", "#ec4899"];
export const MONO = "var(--font-mono)";
export const SANS = "var(--font-sans)";

interface ThemeCtx {
  c: DarkTokens;
  l: LightTokens;
  mode: Mode;
  accent: string;
  grid: boolean;
  setMode: (m: Mode) => void;
  toggleMode: () => void;
  setAccent: (a: string) => void;
  setGrid: (g: boolean) => void;
}

const Ctx = createContext<ThemeCtx | null>(null);

const readMode = (): Mode => {
  if (typeof localStorage === "undefined") return "light";
  return localStorage.getItem("hs-mode") === "dark" ? "dark" : "light";
};

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const [accent, setAccent] = useState(ACCENT_PRESETS[0]);
  const [grid, setGrid] = useState(true);
  const [mode, setModeState] = useState<Mode>(readMode);

  const setMode = (m: Mode) => {
    setModeState(m);
    try {
      localStorage.setItem("hs-mode", m);
    } catch {
      /* ignore */
    }
  };
  const toggleMode = () => setMode(mode === "dark" ? "light" : "dark");

  useEffect(() => {
    document.documentElement.style.setProperty("--color-accent", accent);
  }, [accent]);

  useEffect(() => {
    const root = document.documentElement;
    root.setAttribute("data-theme", mode);
    root.style.colorScheme = mode;
  }, [mode]);

  const value = useMemo<ThemeCtx>(
    () => ({ c: tokens(mode, accent), l: LIGHT, mode, accent, grid, setMode, toggleMode, setAccent, setGrid }),
    [mode, accent, grid]
  );
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useTheme(): ThemeCtx {
  const v = useContext(Ctx);
  if (!v) throw new Error("useTheme must be used within ThemeProvider");
  return v;
}
