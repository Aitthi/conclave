import { useCallback, useEffect, useRef, useState } from "react";
import { Globe, X, ArrowRight, RotateCcw, Link2, Type, MousePointerClick } from "lucide-react";
import { ipc } from "../ipc";
import type { BrowserStatus, BrowserSnapshot } from "../ipc";

// ── In-app browser control surface ──────────────────────────────────────────
// The actual page runs in a SEPARATE native WebView window (runtime::browser);
// this center-pane view is the human-visible control + status inspector. It
// drives the same `browser.*` router commands an agent uses, so what a person
// sees here matches what an agent is driving. No implementation detail leaks
// into visible copy (plan §UI Notes).

export interface InAppBrowserViewProps {
  workspaceId: string;
  workspaceName?: string;
  onClose?: () => void;
}

export function InAppBrowserView({ workspaceName, onClose }: InAppBrowserViewProps) {
  const [urlInput, setUrlInput] = useState("");
  const [status, setStatus] = useState<BrowserStatus | null>(null);
  const [snapshot, setSnapshot] = useState<BrowserSnapshot | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const loadSnapshot = useCallback(async () => {
    try {
      const snap = await ipc.browser.snapshot();
      if (mounted.current) setSnapshot(snap);
    } catch (err) {
      if (import.meta.env.DEV) console.error("InAppBrowserView: snapshot failed", err);
      if (mounted.current) setSnapshot(null);
    }
  }, []);

  const loadStatus = useCallback(async () => {
    try {
      const st = await ipc.browser.status();
      if (!mounted.current) return;
      setStatus(st);
      if (st.ok && st.url) {
        if (!urlInput) setUrlInput(st.url);
        await loadSnapshot();
      } else {
        setSnapshot(null);
      }
    } catch (err) {
      if (import.meta.env.DEV) console.error("InAppBrowserView: status failed", err);
      if (mounted.current) setStatus({ ok: false, message: "couldn't read browser status" });
    }
  }, [urlInput, loadSnapshot]);

  // Reflect the current browser state on mount — never auto-opens a window; the
  // human explicitly opens via the toolbar (keeps fixture-mode render read-only).
  useEffect(() => {
    void loadStatus();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const doOpen = useCallback(async () => {
    const target = urlInput.trim();
    if (!target || busy) return;
    setBusy(true);
    setError(null);
    try {
      const st = await ipc.browser.open({ url: target });
      if (!mounted.current) return;
      setStatus(st);
      if (st.ok) await loadSnapshot();
    } catch (err) {
      if (import.meta.env.DEV) console.error("InAppBrowserView: open failed", err);
      if (mounted.current) setError("Couldn't open that URL");
    } finally {
      if (mounted.current) setBusy(false);
    }
  }, [urlInput, busy, loadSnapshot]);

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
          title="Refresh status + snapshot"
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
          <>
            <span className="font-medium text-text-primary truncate">{status?.title || "—"}</span>
            <span className="text-text-tertiary truncate font-mono">{status?.url}</span>
          </>
        ) : (
          <span className="text-text-tertiary">
            {status?.message || "No browser open"}
          </span>
        )}
        {error && <span className="ml-auto text-danger">{error}</span>}
      </div>

      {/* Inspector. */}
      <div className="flex-1 min-h-0 overflow-y-auto scroll-thin px-4 py-3">
        {!isOpen ? (
          <div className="h-full grid place-items-center text-center">
            <div className="max-w-xs space-y-1.5">
              <Globe className="w-8 h-8 mx-auto text-text-tertiary" />
              <div className="text-[12.5px] text-text-secondary">No browser open</div>
              <div className="text-[11px] text-text-tertiary leading-snug">
                Enter a URL above to open a page. Agents drive the same browser with
                <span className="font-mono"> conclave browser</span>.
              </div>
            </div>
          </div>
        ) : !snapshot ? (
          <div className="text-[11.5px] text-text-tertiary">No snapshot yet — Refresh to capture the page.</div>
        ) : (
          <div className="space-y-4">
            <InspectorSection
              icon={<Type className="w-3.5 h-3.5" />}
              label="Page text"
              count={snapshot.text.length}
            >
              <p className="text-[11.5px] leading-relaxed text-text-secondary whitespace-pre-wrap break-words max-h-40 overflow-y-auto scroll-thin rounded-md bg-fill-soft p-2">
                {snapshot.text || "—"}
              </p>
            </InspectorSection>

            {snapshot.headings.length > 0 && (
              <InspectorSection label="Headings" count={snapshot.headings.length}>
                <ul className="space-y-0.5">
                  {snapshot.headings.map((h, i) => (
                    <li key={i} className="text-[11.5px] text-text-secondary truncate">
                      {h}
                    </li>
                  ))}
                </ul>
              </InspectorSection>
            )}

            {snapshot.links.length > 0 && (
              <InspectorSection
                icon={<Link2 className="w-3.5 h-3.5" />}
                label="Links"
                count={snapshot.links.length}
              >
                <ul className="space-y-1">
                  {snapshot.links.map((l, i) => (
                    <li key={i} className="flex items-baseline gap-2 min-w-0">
                      <span className="text-[11.5px] text-text-primary truncate flex-1 min-w-0">
                        {l.text || l.href}
                      </span>
                      <code className="text-[10px] text-text-tertiary shrink-0 font-mono truncate max-w-[45%]">
                        {l.selector}
                      </code>
                    </li>
                  ))}
                </ul>
              </InspectorSection>
            )}

            {snapshot.inputs.length > 0 && (
              <InspectorSection
                icon={<Type className="w-3.5 h-3.5" />}
                label="Inputs"
                count={snapshot.inputs.length}
              >
                <ul className="space-y-1">
                  {snapshot.inputs.map((inp, i) => (
                    <li key={i} className="flex items-baseline gap-2 min-w-0">
                      <span className="text-[11.5px] text-text-secondary shrink-0">
                        {inp.type || "text"}
                        {inp.name ? ` · ${inp.name}` : ""}
                      </span>
                      <code className="text-[10px] text-text-tertiary font-mono truncate flex-1 min-w-0 text-right">
                        {inp.selector}
                      </code>
                    </li>
                  ))}
                </ul>
              </InspectorSection>
            )}

            {snapshot.buttons.length > 0 && (
              <InspectorSection
                icon={<MousePointerClick className="w-3.5 h-3.5" />}
                label="Buttons"
                count={snapshot.buttons.length}
              >
                <ul className="space-y-1">
                  {snapshot.buttons.map((b, i) => (
                    <li key={i} className="flex items-baseline gap-2 min-w-0">
                      <span className="text-[11.5px] text-text-primary truncate flex-1 min-w-0">
                        {b.text || "—"}
                      </span>
                      <code className="text-[10px] text-text-tertiary shrink-0 font-mono truncate max-w-[45%]">
                        {b.selector}
                      </code>
                    </li>
                  ))}
                </ul>
              </InspectorSection>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function InspectorSection({
  icon,
  label,
  count,
  children,
}: {
  icon?: React.ReactNode;
  label: string;
  count?: number;
  children: React.ReactNode;
}) {
  return (
    <section>
      <div className="flex items-center gap-1.5 mb-1.5 text-[10px] font-bold tracking-wider text-text-tertiary uppercase">
        {icon}
        <span>{label}</span>
        {count != null && <span className="normal-case tracking-normal">{count}</span>}
      </div>
      {children}
    </section>
  );
}
