import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  Eye,
  Globe,
  Loader2,
  Lock,
  Plus,
  RotateCw,
  X,
} from "lucide-react";
import { ipc } from "../ipc";
import type { BrowserOwner, BrowserTab } from "../ipc";

// ── In-app browser: ownership-first side rail ────────────────────────────────
// Per-agent tabs keyed by owner (D1): the human's own tabs are interactive, an
// agent's tab is a read-only window into its live page. Each agent's page runs
// in its own native child webview (runtime::browser), overlaid on the empty
// `regionRef` rectangle below the chrome bar for whichever tab is active — the
// rest keep running hidden in the background. In fixture mode (no Tauri) that
// region renders empty — accepted, like PTY panes. Layout follows the Lane-0
// design canon (design/screens/browser.tsx, pinned design:inapp-browser).

export interface InAppBrowserViewProps {
  workspaceId: string;
  workspaceName?: string;
  onClose?: () => void;
}

// Identity-chip hue. BrowserOwner (the fixed IPC contract) carries no color —
// unlike the design canon's mock data, which hand-assigns one — so agent tabs
// derive a deterministic hue from `owner.id` over the app's real 6-color
// identity palette (`--color-agent-*`, src/styles/app.css). Literal hex, not
// `var(--color-agent-*)`: those tokens live inside app.css's `@theme inline`
// block, which Tailwind v4 tree-shakes out of the compiled CSS when no
// generated utility class references them (only a `bg-agent-*` class would
// count — a JS `style={{backgroundColor: "var(--color-agent-vega)"}}` is
// invisible to Tailwind's static scanner). The design canon hit this exact
// pitfall and worked around it with a plain `:root` block; here the simplest
// boundary-safe fix is the literal value, "stable across themes" per
// app.css's own comment on this palette. Human tabs use the accent (which
// IS a flipping @theme token, but is safe here since `<button>`s and other
// utility classes elsewhere already reference it, keeping it in the bundle).
const AGENT_HUE_HEX = ["#0fa3a3", "#ff7a45", "#d6409f", "#32ade6", "#ff9f0a", "#5e5ce6"] as const;

function hueVar(owner: BrowserOwner): string {
  if (owner.kind === "human") return "var(--color-accent)";
  let h = 0;
  for (let i = 0; i < owner.id.length; i++) h = (h * 31 + owner.id.charCodeAt(i)) >>> 0;
  return AGENT_HUE_HEX[h % AGENT_HUE_HEX.length];
}

// Two-letter monogram from a display name — BrowserOwner has no `initials`
// field (unlike the canon's mock Owner), so it's derived from `label`.
function initialsOf(label: string): string {
  const parts = label.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "?";
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return (parts[0][0] + parts[1][0]).toUpperCase();
}

type TabStatus = "live" | "loading" | "ended";

function statusOf(tab: BrowserTab): TabStatus {
  if (tab.ended) return "ended";
  if (tab.loading) return "loading";
  return "live";
}

function SectionLabel({
  children,
  lock,
  action,
}: {
  children: React.ReactNode;
  lock?: boolean;
  action?: React.ReactNode;
}) {
  return (
    <div className="flex items-center gap-1 px-2 pb-1 pt-3">
      <span className="text-[10px] font-semibold uppercase tracking-[0.06em] text-text-tertiary">
        {children}
      </span>
      {lock && <Lock className="h-2.5 w-2.5 text-text-tertiary" />}
      {action && <span className="ml-auto">{action}</span>}
    </div>
  );
}

// One session row in the rail. Identity leads (avatar chip + name) — the page
// is secondary, the inversion that makes "who owns this tab" legible at a
// glance (D3).
function TabRow({
  tab,
  active,
  onSelect,
  onClose,
}: {
  tab: BrowserTab;
  active: boolean;
  onSelect: (tabId: string) => void;
  // Present ⇒ the row is closable: a ✕ fades in on hover, replacing the
  // right-side status indicator.
  onClose?: (tabId: string) => void;
}) {
  const status = statusOf(tab);
  const ended = status === "ended";
  const forceClose = tab.owner.kind === "agent" && !ended;

  return (
    <div className="group relative">
      <button
        type="button"
        onClick={() => onSelect(tab.tabId)}
        aria-current={active ? "true" : undefined}
        className={`flex w-full items-center gap-2.5 rounded-md px-2 py-2 text-left transition-colors ${
          active ? "bg-accent/[0.12]" : "hover:bg-overlay/[0.05]"
        }`}
      >
      <span className="relative shrink-0">
        <span
          className={`grid h-8 w-8 place-items-center rounded-md text-[11px] font-semibold text-white ${
            ended ? "opacity-40 grayscale" : ""
          }`}
          style={{ backgroundColor: hueVar(tab.owner) }}
        >
          {initialsOf(tab.owner.label)}
        </span>
        {status === "live" && (
          <span className="absolute -bottom-0.5 -right-0.5 h-2.5 w-2.5 rounded-full border-2 border-sidebar bg-status-running" />
        )}
      </span>

      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-1.5">
          <span
            className={`truncate text-[13px] font-medium ${
              ended ? "text-text-muted" : "text-text-primary"
            }`}
          >
            {tab.owner.label}
          </span>
        </span>
        <span
          className={`mt-0.5 block truncate text-[12px] ${
            ended ? "text-text-tertiary" : "text-text-secondary"
          }`}
        >
          {ended ? tab.url : tab.title || tab.url || "New tab"}
        </span>
      </span>

      <span
        className={`shrink-0 self-start pt-0.5 ${
          onClose ? "transition-opacity group-hover:opacity-0" : ""
        }`}
      >
        {status === "loading" && (
          <Loader2 className="h-3.5 w-3.5 animate-spin text-status-waiting" />
        )}
        {ended && (
          <span className="rounded-full bg-fill-soft px-1.5 py-0.5 text-[10px] font-medium text-text-tertiary">
            Ended
          </span>
        )}
      </span>
      </button>

      {/* Close ✕ — fades in over the status slot on row hover / keyboard
          focus. stopPropagation so closing never also selects the row. */}
      {onClose && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onClose(tab.tabId);
          }}
          title={forceClose ? "Force close — the agent is still using this tab" : "Close tab"}
          aria-label={
            forceClose
              ? `Force close ${tab.owner.label}'s tab`
              : `Close ${tab.owner.label}'s tab`
          }
          className={`absolute right-2 top-1/2 grid h-6 w-6 -translate-y-1/2 place-items-center rounded-md text-text-tertiary opacity-0 transition-opacity focus-visible:opacity-100 group-hover:opacity-100 ${
            forceClose
              ? "hover:bg-danger/[0.08] hover:text-danger"
              : "hover:bg-overlay/[0.08] hover:text-text-primary"
          }`}
        >
          <X className="h-3.5 w-3.5" />
        </button>
      )}
    </div>
  );
}

function NavBtn({
  children,
  disabled,
  onClick,
}: {
  children: React.ReactNode;
  disabled?: boolean;
  onClick?: () => void;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className={`grid h-7 w-7 place-items-center rounded-md transition-colors ${
        disabled
          ? "text-text-tertiary/50"
          : "text-text-secondary hover:bg-overlay/[0.06] hover:text-text-primary"
      }`}
    >
      {children}
    </button>
  );
}

export function InAppBrowserView({ workspaceName, onClose }: InAppBrowserViewProps) {
  const [tabs, setTabs] = useState<BrowserTab[]>([]);
  const [activeTabId, setActiveTabId] = useState<string | undefined>(undefined);
  const [urlInput, setUrlInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mounted = useRef(true);
  const regionRef = useRef<HTMLDivElement | null>(null);
  const rafRef = useRef<number | null>(null);

  const humanTabs = tabs.filter((t) => t.owner.kind === "human");
  const agentTabs = tabs.filter((t) => t.owner.kind === "agent");
  const activeTab =
    tabs.find((t) => t.tabId === activeTabId) ?? humanTabs[0] ?? tabs[0] ?? null;

  // Reset the URL bar to the newly-active tab's url — but only when the ACTIVE
  // TAB changes, never on every poll tick, or in-progress typing would be
  // clobbered out from under the human.
  useEffect(() => {
    setUrlInput(activeTab?.url ?? "");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeTab?.tabId]);

  // Measure the reserved region and push its viewport rect to the native
  // overlay. Debounced to one animation frame so a burst of resize callbacks
  // collapses into a single setBounds.
  const syncBounds = useCallback(() => {
    if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
    rafRef.current = requestAnimationFrame(() => {
      const el = regionRef.current;
      if (!el) return;
      const r = el.getBoundingClientRect();
      void ipc.browser
        .setBounds({ x: r.left, y: r.top, width: r.width, height: r.height })
        .catch(() => {});
    });
  }, []);

  const applyState = useCallback((st: { tabs: BrowserTab[]; activeTabId?: string }) => {
    if (!mounted.current) return;
    setTabs(st.tabs);
    setActiveTabId(st.activeTabId);
  }, []);

  const refresh = useCallback(async () => {
    try {
      const st = await ipc.browser.status();
      applyState(st);
    } catch (err) {
      if (import.meta.env.DEV) console.error("InAppBrowserView: status failed", err);
    }
  }, [applyState]);

  // On mount: show the overlay, position it, load the tab list. On unmount
  // (tab switch away from Browser): hide the overlay (never close) so agents
  // keep driving their pages in the background.
  useEffect(() => {
    mounted.current = true;
    void ipc.browser.setVisible({ visible: true }).catch(() => {});
    syncBounds();
    void refresh();

    const poll = window.setInterval(() => {
      void refresh();
    }, 2000);

    const el = regionRef.current;
    const ro = el ? new ResizeObserver(() => syncBounds()) : null;
    if (el && ro) ro.observe(el);
    window.addEventListener("resize", syncBounds);

    return () => {
      mounted.current = false;
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
      window.clearInterval(poll);
      window.removeEventListener("resize", syncBounds);
      ro?.disconnect();
      void ipc.browser.setVisible({ visible: false }).catch(() => {});
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const doSelect = useCallback(
    async (tabId: string) => {
      if (tabId === activeTabId) return;
      try {
        const st = await ipc.browser.setActive({ tabId });
        applyState(st);
        syncBounds();
      } catch (err) {
        if (import.meta.env.DEV) console.error("InAppBrowserView: setActive failed", err);
      }
    },
    [activeTabId, applyState, syncBounds],
  );

  const doNewTab = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      const { tabId } = await ipc.browser.newTab();
      const st = await ipc.browser.setActive({ tabId });
      applyState(st);
      syncBounds();
    } catch (err) {
      if (import.meta.env.DEV) console.error("InAppBrowserView: newTab failed", err);
      if (mounted.current) setError("Couldn't open a new tab");
    } finally {
      if (mounted.current) setBusy(false);
    }
  }, [busy, applyState, syncBounds]);

  const doGoto = useCallback(async () => {
    const target = urlInput.trim();
    if (!target || busy || !activeTab || activeTab.owner.kind !== "human") return;
    setBusy(true);
    setError(null);
    try {
      const st = await ipc.browser.goto({ tabId: activeTab.tabId, url: target });
      applyState(st);
    } catch (err) {
      if (import.meta.env.DEV) console.error("InAppBrowserView: goto failed", err);
      if (mounted.current) setError("Couldn't open that URL");
    } finally {
      if (mounted.current) setBusy(false);
    }
  }, [urlInput, busy, activeTab, applyState]);

  const doClose = useCallback(
    async (tabId: string) => {
      if (busy) return;
      setBusy(true);
      setError(null);
      try {
        const st = await ipc.browser.close({ tabId });
        applyState(st);
      } catch (err) {
        if (import.meta.env.DEV) console.error("InAppBrowserView: close failed", err);
        if (mounted.current) setError("Couldn't close the tab");
      } finally {
        if (mounted.current) setBusy(false);
      }
    },
    [busy, applyState],
  );

  const doCloseAllAgents = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      for (const tab of agentTabs) {
        const st = await ipc.browser.close({ tabId: tab.tabId });
        applyState(st);
      }
    } catch (err) {
      if (import.meta.env.DEV) console.error("InAppBrowserView: close failed", err);
      if (mounted.current) setError("Couldn't close the tab");
    } finally {
      if (mounted.current) setBusy(false);
    }
  }, [busy, agentTabs, applyState]);

  const liveAgents = agentTabs.filter((t) => !t.ended).length;
  const isEmpty = tabs.length === 0;

  return (
    <div className="flex flex-1 min-h-0 bg-bg-canvas">
      {/* rail — the vertical, Arc-like owner list (D3) */}
      <aside
        className="flex w-[264px] shrink-0 flex-col border-r border-overlay/[0.06] bg-sidebar"
        title={workspaceName ? `Browser · ${workspaceName}` : "Browser"}
      >
        <div className="flex items-center gap-2 px-3 pb-2 pt-3.5">
          <Globe className="h-[18px] w-[18px] shrink-0 text-accent" />
          <span className="text-[13px] font-semibold text-text-primary">Browser</span>
          <button
            type="button"
            onClick={() => void doNewTab()}
            disabled={busy}
            aria-label="New tab"
            className="ml-auto grid h-7 w-7 place-items-center rounded-md text-text-secondary transition-colors hover:bg-overlay/[0.06] hover:text-text-primary disabled:opacity-40"
          >
            <Plus className="h-4 w-4" />
          </button>
          {onClose && (
            <button
              type="button"
              onClick={onClose}
              title="Close"
              aria-label="Close Browser"
              className="grid h-7 w-7 place-items-center rounded-md text-text-secondary transition-colors hover:bg-overlay/[0.06] hover:text-text-primary"
            >
              <X className="h-4 w-4" />
            </button>
          )}
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
          {isEmpty ? (
            <p className="px-2 pt-2 text-[12px] leading-relaxed text-text-tertiary">
              No open tabs. Agents' sessions appear here as they browse.
            </p>
          ) : (
            <>
              {error && (
                <p
                  role="alert"
                  className="mx-2 mt-2 rounded-md bg-danger/[0.08] px-2 py-1.5 text-[11px] leading-relaxed text-danger"
                >
                  {error}
                </p>
              )}

              {humanTabs.length > 0 && (
                <>
                  <SectionLabel>You</SectionLabel>
                  <div className="flex flex-col gap-0.5">
                    {humanTabs.map((t) => (
                      <TabRow
                        key={t.tabId}
                        tab={t}
                        active={t.tabId === activeTab?.tabId}
                        onSelect={(id) => void doSelect(id)}
                        onClose={(id) => void doClose(id)}
                      />
                    ))}
                  </div>
                </>
              )}

              {agentTabs.length > 0 && (
                <>
                  <SectionLabel
                    lock
                    action={
                      <button
                        type="button"
                        onClick={() => void doCloseAllAgents()}
                        disabled={busy}
                        aria-label="Close all agent tabs"
                        className="rounded px-1.5 py-0.5 text-[10px] font-medium normal-case tracking-normal text-text-tertiary transition-colors hover:bg-overlay/[0.06] hover:text-text-primary disabled:opacity-40"
                      >
                        Close all
                      </button>
                    }
                  >
                    Agents · {liveAgents} live
                  </SectionLabel>
                  <div className="flex flex-col gap-0.5">
                    {agentTabs.map((t) => (
                      <TabRow
                        key={t.tabId}
                        tab={t}
                        active={t.tabId === activeTab?.tabId}
                        onSelect={(id) => void doSelect(id)}
                        onClose={(id) => void doClose(id)}
                      />
                    ))}
                  </div>
                </>
              )}
            </>
          )}
        </div>
      </aside>

      {/* detail pane */}
      {isEmpty || !activeTab ? (
        <main className="grid min-w-0 flex-1 place-items-center bg-surface">
          <div className="flex max-w-sm flex-col items-center px-8 text-center">
            <div className="grid h-14 w-14 place-items-center rounded-lg bg-fill-soft text-text-tertiary">
              <Globe className="h-7 w-7" />
            </div>
            <h1 className="mt-5 text-[19px] font-semibold tracking-tight text-text-primary">
              No open tabs
            </h1>
            <p className="mt-2 text-[13px] leading-relaxed text-text-secondary">
              Open a page to browse it yourself. When an agent starts browsing, its
              session shows up in the rail as a read-only tab you can close at any time.
            </p>
            {error && <p className="mt-2 text-[12px] text-danger">{error}</p>}
            <button
              type="button"
              onClick={() => void doNewTab()}
              disabled={busy}
              className="mt-6 inline-flex items-center gap-1.5 rounded-md bg-accent px-3.5 py-2 text-[13px] font-medium text-white transition-colors hover:bg-accent-hover disabled:opacity-40"
            >
              <Plus className="h-4 w-4" />
              New tab
            </button>
          </div>
        </main>
      ) : (
        <main className="flex min-w-0 flex-1 flex-col bg-surface">
          {/* chrome bar — human tabs steer, agent tabs are observed (D2) */}
          <div className="flex h-12 shrink-0 items-center gap-2 border-b border-overlay/[0.06] bg-surface px-3">
            {activeTab.owner.kind === "human" ? (
              <>
                <div className="flex items-center gap-0.5">
                  <NavBtn>
                    <ArrowLeft className="h-4 w-4" />
                  </NavBtn>
                  <NavBtn disabled>
                    <ArrowRight className="h-4 w-4" />
                  </NavBtn>
                  <NavBtn disabled={busy} onClick={() => void doGoto()}>
                    <RotateCw className={`h-4 w-4${busy ? " animate-spin" : ""}`} />
                  </NavBtn>
                </div>
                <div className="flex h-8 min-w-0 flex-1 items-center gap-2 rounded-md bg-fill-soft px-2.5 text-[12px] text-text-primary">
                  <span className="h-3.5 w-3.5 shrink-0 rounded-full bg-status-running/90" />
                  <input
                    value={urlInput}
                    onChange={(e) => setUrlInput(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") void doGoto();
                    }}
                    placeholder="Enter a URL (example.com)…"
                    spellCheck={false}
                    className="min-w-0 flex-1 truncate bg-transparent font-mono outline-none placeholder:text-text-tertiary"
                  />
                </div>
                <button
                  type="button"
                  onClick={() => void doClose(activeTab.tabId)}
                  disabled={busy}
                  aria-label="Close tab"
                  className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-text-secondary transition-colors hover:bg-overlay/[0.06] hover:text-text-primary disabled:opacity-40"
                >
                  <X className="h-4 w-4" />
                </button>
              </>
            ) : (
              <>
                <div className="flex items-center gap-0.5">
                  <NavBtn disabled>
                    <ArrowLeft className="h-4 w-4" />
                  </NavBtn>
                  <NavBtn disabled>
                    <ArrowRight className="h-4 w-4" />
                  </NavBtn>
                  <NavBtn disabled>
                    <RotateCw className="h-4 w-4" />
                  </NavBtn>
                </div>
                <div className="flex h-8 min-w-0 flex-1 items-center gap-2 rounded-md bg-fill-soft px-2.5 text-[12px] text-text-secondary">
                  <Lock className="h-3.5 w-3.5 shrink-0 text-text-tertiary" />
                  <span className="truncate font-mono">{activeTab.url}</span>
                </div>
                <span
                  className={`flex shrink-0 items-center gap-1.5 rounded-full py-1 pl-1.5 pr-2.5 text-[11px] font-medium ${
                    activeTab.ended
                      ? "bg-fill-soft text-text-tertiary"
                      : "bg-overlay/[0.05] text-text-secondary"
                  }`}
                >
                  {activeTab.ended ? (
                    "Session ended"
                  ) : (
                    <>
                      <Eye className="h-3 w-3" />
                      Observing
                      <span
                        className="ml-0.5 h-3.5 w-3.5 rounded-[0.3rem] text-center text-[8px] font-bold leading-[0.875rem] text-white"
                        style={{ backgroundColor: hueVar(activeTab.owner) }}
                      >
                        {initialsOf(activeTab.owner.label).slice(0, 1)}
                      </span>
                    </>
                  )}
                </span>
              </>
            )}
          </div>

          {/* Reserved region — the native webview overlay is positioned over
              this rectangle by syncBounds(). Stays empty in the DOM on
              purpose; only the ended-session marker is real DOM content. */}
          <div ref={regionRef} className="relative min-h-0 flex-1 overflow-hidden bg-bg-canvas">
            {activeTab.ended && (
              <div className="absolute inset-0 grid place-items-center">
                <div className="flex flex-col items-center gap-1 text-center">
                  <span className="text-[14px] font-semibold text-text-secondary">
                    Session ended
                  </span>
                  <span className="text-[12px] text-text-muted">
                    {activeTab.owner.label} finished browsing · kept read-only for review
                  </span>
                </div>
              </div>
            )}
          </div>
        </main>
      )}
    </div>
  );
}
