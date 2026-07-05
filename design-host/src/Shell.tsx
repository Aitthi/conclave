import { Component, useEffect, useState, type ComponentType, type ReactNode } from "react";
import { parseHashScreen, pickInitialScreen } from "./screenSelection";

type ScreenLoader = () => Promise<{ default: ComponentType }>;

interface ShellProps {
  screens: Record<string, ScreenLoader>;
  screenIds: string[];
}

const PROJECT = new URLSearchParams(window.location.search).get("project") ?? "";
const LS_KEY = `conclave-design-active:${PROJECT}`;

// Conclave-native design canvas shell — the Design view's iframe target. No spec panel,
// no tabs, no Arta concepts: just the selected screen full-bleed plus a small switcher
// when there's more than one. The switcher's placement/look is Lane C's canon to design;
// this is the functional baseline it pins.
export function Shell({ screens, screenIds }: ShellProps) {
  const [active, setActiveState] = useState<string | null>(() =>
    pickInitialScreen(parseHashScreen(window.location.hash), localStorage.getItem(LS_KEY), screenIds),
  );

  const setActive = (id: string | null) => {
    setActiveState(id);
    if (id) {
      history.replaceState(null, "", `#/${encodeURIComponent(id)}`);
      localStorage.setItem(LS_KEY, id);
    }
  };

  useEffect(() => {
    const hashScreen = parseHashScreen(window.location.hash);
    const stored = localStorage.getItem(LS_KEY);

    if (active && !screenIds.includes(active)) {
      setActive(pickInitialScreen(hashScreen, stored, screenIds));
    } else if (!active && screenIds.length) {
      setActive(pickInitialScreen(hashScreen, stored, screenIds));
    } else if (active && (hashScreen !== active || stored !== active)) {
      setActive(active);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [screenIds, active]);

  if (!active) return <EmptyState />;

  return (
    <div style={{ minHeight: "100vh" }}>
      {screenIds.length > 1 && <Switcher ids={screenIds} active={active} onSelect={setActive} />}
      <ScreenHost key={active} loader={screens[active]} />
    </div>
  );
}

function Switcher({ ids, active, onSelect }: { ids: string[]; active: string; onSelect: (id: string) => void }) {
  return (
    <div
      style={{
        position: "fixed",
        top: 12,
        right: 12,
        zIndex: 9999,
        background: "rgba(24,24,27,0.85)",
        borderRadius: 8,
        padding: 4,
        display: "flex",
        gap: 4,
      }}
    >
      {ids.map((id) => (
        <button
          key={id}
          type="button"
          onClick={() => onSelect(id)}
          style={{
            font: "12px ui-monospace, monospace",
            padding: "4px 8px",
            borderRadius: 6,
            border: "none",
            cursor: "pointer",
            background: id === active ? "#fafafa" : "transparent",
            color: id === active ? "#18181b" : "#fafafa",
          }}
        >
          {id}
        </button>
      ))}
    </div>
  );
}

function EmptyState() {
  return (
    <div style={{ padding: 24, fontFamily: "system-ui, sans-serif", color: "#71717a" }}>
      No screens yet — ask your agent to write one under design/screens/.
    </div>
  );
}

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

// Catches a render-time throw from the loaded screen module, distinct from the
// load-time rejection ScreenHost's own .catch() already handles.
class ErrorBoundary extends Component<{ children: ReactNode }, { message: string | null }> {
  state: { message: string | null } = { message: null };

  static getDerivedStateFromError(err: unknown): { message: string } {
    return { message: err instanceof Error ? err.message : String(err) };
  }

  render(): ReactNode {
    if (this.state.message) return <CrashCard message={this.state.message} />;
    return this.props.children;
  }
}

function ScreenHost({ loader }: { loader: ScreenLoader | undefined }) {
  const [Comp, setComp] = useState<ComponentType | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setComp(null);
    setLoadError(null);
    if (!loader) {
      setLoadError("screen not found");
      return;
    }
    loader()
      .then((mod) => {
        if (!cancelled) setComp(() => mod.default);
      })
      .catch((err: unknown) => {
        if (!cancelled) setLoadError(err instanceof Error ? err.message : String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [loader]);

  if (loadError) return <CrashCard message={loadError} />;
  if (!Comp) return null;
  return (
    <ErrorBoundary>
      <Comp />
    </ErrorBoundary>
  );
}
