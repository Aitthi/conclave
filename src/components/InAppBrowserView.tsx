import { useCallback, useEffect, useRef, useState } from "react";
import { Globe, X, ArrowRight, RotateCcw } from "lucide-react";
import { ipc } from "../ipc";
import type { BrowserStatus } from "../ipc";

// ── In-app browser tab ───────────────────────────────────────────────────────
// The live page runs in a native child webview (runtime::browser) overlaid on
// the empty region below the toolbar. This component owns the URL bar + status
// and keeps the overlay positioned (setBounds) and shown while mounted; on
// unmount (tab switch) it hides the overlay WITHOUT closing it, so an agent
// keeps driving the page in the background. In fixture mode the overlay is
// absent (no Tauri) and the region renders empty — accepted, like PTY panes.

export interface InAppBrowserViewProps {
  workspaceId: string;
  workspaceName?: string;
  onClose?: () => void;
}

export function InAppBrowserView({ workspaceName, onClose }: InAppBrowserViewProps) {
  const [urlInput, setUrlInput] = useState("");
  const [status, setStatus] = useState<BrowserStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mounted = useRef(true);
  const regionRef = useRef<HTMLDivElement | null>(null);
  const rafRef = useRef<number | null>(null);

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

  const loadStatus = useCallback(async (): Promise<BrowserStatus | null> => {
    try {
      const st = await ipc.browser.status();
      if (!mounted.current) return null;
      setStatus(st);
      if (st.ok && st.url && !urlInput) setUrlInput(st.url);
      return st;
    } catch (err) {
      if (import.meta.env.DEV) console.error("InAppBrowserView: status failed", err);
      if (mounted.current) setStatus({ ok: false, message: "couldn't read browser status" });
      return null;
    }
  }, [urlInput]);

  // On mount: show the overlay, position it, read status. On unmount: hide the
  // overlay (never close) so the page keeps running for background agents.
  //
  // Visibility is backend-passive (runtime::browser never calls show/hide on
  // open/goto) — the backend always creates/updates the webview hidden so an
  // agent-initiated open never paints over another tab. That means an agent
  // opening/navigating while this tab is already mounted needs this component
  // to notice and re-assert visibility itself; the `reconcile` poll below is
  // that mechanism (re-showing is idempotent, harmless if already visible).
  useEffect(() => {
    mounted.current = true;
    void ipc.browser.setVisible({ visible: true }).catch(() => {});
    syncBounds();
    void loadStatus();

    const reconcile = async () => {
      const st = await loadStatus();
      if (!mounted.current || !st) return;
      if (st.ok) {
        void ipc.browser.setVisible({ visible: true }).catch(() => {});
        syncBounds();
      }
    };
    const poll = window.setInterval(() => void reconcile(), 2000);

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

  const doOpen = useCallback(async () => {
    const target = urlInput.trim();
    if (!target || busy) return;
    setBusy(true);
    setError(null);
    try {
      const el = regionRef.current;
      const r = el?.getBoundingClientRect();
      const bounds = r ? { x: r.left, y: r.top, width: r.width, height: r.height } : undefined;
      const st = await ipc.browser.open({ url: target, bounds });
      if (!mounted.current) return;
      setStatus(st);
      await ipc.browser.setVisible({ visible: true }).catch(() => {});
    } catch (err) {
      if (import.meta.env.DEV) console.error("InAppBrowserView: open failed", err);
      if (mounted.current) setError("Couldn't open that URL");
    } finally {
      if (mounted.current) setBusy(false);
    }
  }, [urlInput, busy]);

  const doRefresh = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    await loadStatus();
    if (mounted.current) setBusy(false);
  }, [busy, loadStatus]);

  const isOpen = !!status?.ok;

  return (
    <div className="flex-1 min-h-0 flex flex-col bg-surface">
      {/* Header — matches the other center-pane views' 48px title bar. */}
      <div className="h-12 shrink-0 flex items-center gap-2 px-4 border-b border-overlay/[0.06]">
        <Globe className="w-[18px] h-[18px] text-accent shrink-0" />
        <span className="text-[13px] font-semibold text-text-primary">Browser</span>
        {workspaceName && (
          <span className="text-[11px] text-text-tertiary truncate">· {workspaceName}</span>
        )}
        {onClose && (
          <button
            onClick={onClose}
            title="Close"
            className="ml-auto w-7 h-7 grid place-items-center rounded-md text-text-secondary hover:bg-overlay/[0.05]"
          >
            <X className="w-4 h-4" />
          </button>
        )}
      </div>

      {/* Address toolbar. */}
      <div className="shrink-0 flex items-center gap-2 px-4 py-2.5 border-b border-overlay/[0.06]">
        <input
          value={urlInput}
          onChange={(e) => setUrlInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void doOpen();
          }}
          placeholder="Enter a URL (example.com)…"
          spellCheck={false}
          className="flex-1 min-w-0 h-8 px-3 rounded-lg bg-fill-soft ring-hair text-[12.5px] text-text-primary placeholder:text-text-tertiary focus:outline-none focus:ring-1 focus:ring-accent font-mono"
        />
        <button
          onClick={() => void doOpen()}
          disabled={busy || !urlInput.trim()}
          className="h-8 px-3 inline-flex items-center gap-1.5 rounded-lg bg-accent text-white text-[12px] font-medium disabled:opacity-40"
          title="Open / navigate"
        >
          <ArrowRight className="w-[14px] h-[14px]" />
          Open
        </button>
        <button
          onClick={() => void doRefresh()}
          disabled={busy}
          className="h-8 w-8 grid place-items-center rounded-lg ring-hair text-text-secondary hover:bg-overlay/[0.04] disabled:opacity-40"
          title="Refresh status"
        >
          <RotateCcw className={`w-[14px] h-[14px]${busy ? " animate-spin" : ""}`} />
        </button>
      </div>

      {/* Status line. */}
      <div className="shrink-0 flex items-center gap-2 px-4 py-2 border-b border-overlay/[0.06] text-[11.5px]">
        <span
          className={`w-1.5 h-1.5 rounded-full shrink-0 ${isOpen ? "bg-success" : "bg-text-tertiary"}`}
        />
        {isOpen ? (
          <span className="text-text-tertiary truncate font-mono">{status?.url}</span>
        ) : (
          <span className="text-text-tertiary">{status?.message || "No browser open"}</span>
        )}
        {error && <span className="ml-auto text-danger">{error}</span>}
      </div>

      {/* Reserved region — the native webview overlay is positioned over this
          rectangle by syncBounds(). It stays empty in the DOM on purpose. */}
      <div ref={regionRef} className="flex-1 min-h-0 relative">
        {!isOpen && (
          <div className="absolute inset-0 grid place-items-center text-center pointer-events-none">
            <div className="max-w-xs space-y-1.5">
              <Globe className="w-8 h-8 mx-auto text-text-tertiary" />
              <div className="text-[12.5px] text-text-secondary">No browser open</div>
              <div className="text-[11px] text-text-tertiary leading-snug">
                Enter a URL above to open a page. Agents drive the same browser with
                <span className="font-mono"> conclave browser</span>.
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
