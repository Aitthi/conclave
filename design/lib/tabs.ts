// Shared mock data for the Browser canon screens. Real workspace agent names +
// the app's real identity hues, realistic URLs/titles, and every tab state the
// spec enumerates (active / inactive / loading / ended). One source so the hero,
// agent, and empty screens stay consistent.

export type TabStatus = "live" | "loading" | "ended";

export interface Owner {
  kind: "human" | "agent";
  /** display name — agent name, or "You" for the human */
  label: string;
  /** two-letter monogram for the identity chip */
  initials: string;
  /** role suffix (agents only) */
  role?: string;
  /** identity hue token name (agents only); human uses the accent */
  hue?: string;
}

export interface Tab {
  tabId: string;
  owner: Owner;
  /** last-navigated target (never read from a native getter) */
  url: string;
  title: string;
  status: TabStatus;
}

// Human tabs — the only interactive surfaces (D2). Multiple allowed (D4a).
export const humanTabs: Tab[] = [
  {
    tabId: "human-1",
    owner: { kind: "human", label: "You", initials: "DT" },
    url: "github.com/detoro/codeup/pull/48",
    title: "Per-agent browser tabs · Pull Request #48",
    status: "live",
  },
  {
    tabId: "human-2",
    owner: { kind: "human", label: "You", initials: "DT" },
    url: "tauri.app/reference/javascript/webviewwindow",
    title: "WebviewWindow — Tauri",
    status: "live",
  },
];

// Agent tabs — read-only for the human; exactly one per agent (D4a), and an
// ended agent's tab persists read-only with a badge (D4b).
export const agentTabs: Tab[] = [
  {
    tabId: "agent-dew",
    owner: { kind: "agent", label: "Dew", initials: "DE", role: "Backend", hue: "agent-teal" },
    url: "docs.rs/wry/latest/wry/struct.WebViewBuilder.html",
    title: "WebViewBuilder in wry — Rust",
    status: "loading",
  },
  {
    tabId: "agent-guetta",
    owner: { kind: "agent", label: "Guetta", initials: "GU", role: "Reviewer", hue: "agent-orange" },
    url: "developer.apple.com/documentation/webkit/wkwebview",
    title: "WKWebView | Apple Developer Documentation",
    status: "live",
  },
  {
    tabId: "agent-tiesto",
    owner: { kind: "agent", label: "Tiësto", initials: "TI", role: "Frontend", hue: "agent-magenta" },
    url: "react.dev/reference/react/useSyncExternalStore",
    title: "useSyncExternalStore — React",
    status: "live",
  },
  {
    tabId: "agent-mellow",
    owner: { kind: "agent", label: "Mellow", initials: "ME", role: "Research", hue: "agent-blue" },
    url: "stackoverflow.com/questions/25376537/wkwebview-url-nil-crash",
    title: "WKWebView URL nil crash on about:blank",
    status: "ended",
  },
];

export const allTabs: Tab[] = [...humanTabs, ...agentTabs];

// Identity-chip fill as a token reference (never a raw hex in a screen body).
// Human tabs carry the accent; agents carry their real identity hue via CSS var,
// which also sidesteps Tailwind's dynamic-class scan (a template `bg-${hue}` would
// never be generated).
export function hueVar(owner: Owner): string {
  return owner.kind === "human"
    ? "var(--color-accent)"
    : `var(--color-${owner.hue})`;
}
