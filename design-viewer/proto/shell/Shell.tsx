import { Component, useEffect, useRef, useState, type ComponentType, type ReactNode } from "react";
import { HashRouter, Navigate, Route, Routes, useNavigate, useParams } from "react-router-dom";
import { initBridge, reportError, reportReady } from "./bridge";

// Mirrors the shapes the virtual module (vite/proto-manifest.ts) exports. Duplicated
// here (not imported) on purpose: proto/ is compiled under tsconfig.app.json while
// vite/proto-manifest.ts belongs to the Node-side tsconfig.node.json project — the two
// are separate `tsc -b` project references, and app doesn't reference node, so a
// cross-project import would break the build. Keep these in sync with proto-manifest.ts
// by hand.
export interface ScreenMeta {
  title?: string;
  frame?: "web" | "desktop" | "ios" | "android" | "ipad";
  safeArea?: string;
  chrome?: boolean;
  url?: string;
}
export interface ProtoConfig {
  start?: string;
  frame?: string;
  safeArea?: string;
  chrome?: boolean;
}

type ScreenLoader = () => Promise<{ default: ComponentType; meta?: ScreenMeta }>;
type ComponentLoader = () => Promise<{ default: ComponentType }>;

interface ShellProps {
  config: ProtoConfig;
  metas: Record<string, ScreenMeta>;
  screens: Record<string, ScreenLoader>;
  components: Record<string, ComponentLoader>;
}

export function Shell({ config, metas, screens, components }: ShellProps) {
  const start = config.start ?? Object.keys(screens)[0];
  return (
    <HashRouter>
      <NavBridge />
      <Routes>
        <Route path="/" element={start ? <Navigate to={`/${start}`} replace /> : <EmptyState />} />
        <Route path="/:screenId" element={<ScreenHost screens={screens} metas={metas} />} />
        <Route path="/_component/:name" element={<ComponentHost components={components} />} />
      </Routes>
    </HashRouter>
  );
}

// Wires bridge.ts's parent→shell `nav` messages to the router. Kept as its own
// no-render component so it can sit inside the HashRouter (useNavigate needs the
// Router context) without complicating Shell's route table.
function NavBridge() {
  const navigate = useNavigate();
  // Keep navigate in a ref so initBridge's listener (registered once) always calls
  // the latest navigate function without needing to resubscribe — same pattern as
  // the old FreeformDevice's `cbs` ref (FreeformDevice.tsx:305-306).
  const navigateRef = useRef(navigate);
  navigateRef.current = navigate;
  useEffect(() => {
    initBridge({ onNav: (to) => navigateRef.current(`/${to}`) });
  }, []);
  return null;
}

function EmptyState() {
  return <div style={{ padding: 24, fontFamily: "system-ui, sans-serif", color: "#71717a" }}>No screens in this prototype yet.</div>;
}

// Small inline crash card — shown in place of a screen/component that failed to load
// or threw during render. The dev sees it in the frame; the agent sees it via
// arta_get_view (which reads the DOM), and the same message is also forwarded to the
// parent as a `{type:"error"}` postMessage (via bridge.ts's reportError).
function CrashCard({ message }: { message: string }) {
  return (
    <div
      style={{
        margin: 16,
        padding: 16,
        borderRadius: 8,
        border: "1px solid #fecaca",
        background: "#fef2f2",
        color: "#991b1b",
        fontFamily: "ui-monospace, monospace",
        fontSize: 13,
        whiteSpace: "pre-wrap",
      }}
    >
      <strong>Screen crashed</strong>
      <div>{message}</div>
    </div>
  );
}

// Catches a render-time throw from the loaded screen/component module. This is a
// DIFFERENT error path than bridge.ts's own window.onerror/unhandledrejection
// listeners (those only catch uncaught global errors, not errors React itself
// intercepts during render) — so it reports through the same reportError() the
// bridge's global forwarding uses, keeping one implementation of the error shape.
class ErrorBoundary extends Component<{ children: ReactNode }, { message: string | null }> {
  state: { message: string | null } = { message: null };

  static getDerivedStateFromError(err: unknown): { message: string } {
    return { message: err instanceof Error ? err.message : String(err) };
  }

  componentDidCatch(err: unknown): void {
    reportError(err instanceof Error ? err.message : String(err));
  }

  render(): ReactNode {
    if (this.state.message) return <CrashCard message={this.state.message} />;
    return this.props.children;
  }
}

function ScreenHost({ screens, metas }: { screens: ShellProps["screens"]; metas: ShellProps["metas"] }) {
  const { screenId = "" } = useParams();
  const [Comp, setComp] = useState<ComponentType | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setComp(null);
    setLoadError(null);
    const loader = screens[screenId];
    if (!loader) {
      const message = `No screen named "${screenId}"`;
      setLoadError(message);
      reportError(message);
      return;
    }
    loader()
      .then((mod) => {
        if (cancelled) return;
        setComp(() => mod.default);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        const message = err instanceof Error ? err.message : String(err);
        setLoadError(message);
        reportError(message);
      });
    return () => {
      cancelled = true;
    };
  }, [screenId, screens]);

  // Route contract: tell the parent viewer which screen is showing, and reflect it
  // in the tab title. Runs on every screenId change (cheap + idempotent, so re-firing
  // under StrictMode's dev double-effect is harmless).
  useEffect(() => {
    parent.postMessage({ source: "arta-frame", type: "route", screen: screenId }, "*");
    document.title = metas[screenId]?.title ?? screenId;
  }, [screenId, metas]);

  // Headless-ready contract: only once the screen module AND web fonts have resolved.
  useEffect(() => {
    if (!Comp) return;
    let cancelled = false;
    document.fonts.ready.then(() => {
      if (!cancelled) reportReady(screenId);
    });
    return () => {
      cancelled = true;
    };
  }, [Comp, screenId]);

  if (loadError) return <CrashCard message={loadError} />;
  if (!Comp) return null;

  return (
    <ErrorBoundary key={screenId}>
      <Comp />
    </ErrorBoundary>
  );
}

function ComponentHost({ components }: { components: ShellProps["components"] }) {
  const { name = "" } = useParams();
  const [Comp, setComp] = useState<ComponentType | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setComp(null);
    setLoadError(null);
    const loader = components[name];
    if (!loader) {
      const message = `No component named "${name}"`;
      setLoadError(message);
      reportError(message);
      return;
    }
    loader()
      .then((mod) => {
        if (cancelled) return;
        setComp(() => mod.default);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        const message = err instanceof Error ? err.message : String(err);
        setLoadError(message);
        reportError(message);
      });
    return () => {
      cancelled = true;
    };
  }, [name, components]);

  useEffect(() => {
    if (!Comp) return;
    let cancelled = false;
    document.fonts.ready.then(() => {
      if (!cancelled) reportReady(name);
    });
    return () => {
      cancelled = true;
    };
  }, [Comp, name]);

  if (loadError) return <CrashCard message={loadError} />;
  if (!Comp) return null;

  return (
    <div className="min-h-screen grid place-items-center p-8">
      <ErrorBoundary key={name}>
        <Comp />
      </ErrorBoundary>
    </div>
  );
}
