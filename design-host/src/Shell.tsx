import {
  Component,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ComponentType,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
} from "react";
import { MemoryRouter } from "react-router-dom";
import { filterScreens, parseHashScreen, pickInitialScreen } from "./screenSelection";

type ScreenLoader = () => Promise<{ default: ComponentType }>;

interface ShellProps {
  screens: Record<string, ScreenLoader>;
  screenIds: string[];
}

const PROJECT = new URLSearchParams(window.location.search).get("project") ?? "";
const LS_KEY = `conclave-design-active:${PROJECT}`;

// Conclave-native design canvas shell — the Design view's iframe target. No spec panel,
// no tabs, no Arta concepts: just the selected screen full-bleed plus a small switcher
// when there's more than one. Switcher canon:
// docs/superpowers/plans/2026-07-31-design-host-switcher-redesign.md §Decision.
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

const MONO = "12px ui-monospace, SFMono-Regular, Menlo, monospace";
const SURFACE = "rgba(24,24,27,0.92)";
const HAIRLINE = "1px solid rgba(255,255,255,0.08)";
// A prototype's own chrome lives at the top, so the switcher sits bottom-right and
// fades back to a hint once the pointer leaves it.
const IDLE_AFTER_MS = 2000;
// Below this many screens the list is short enough to scan; the search box would
// be one more thing to ignore.
const FILTER_FROM = 8;

function isTypingTarget(el: Element | null): boolean {
  if (!(el instanceof HTMLElement)) return false;
  if (el.isContentEditable) return true;
  const tag = el.tagName.toLowerCase();
  return tag === "input" || tag === "textarea" || tag === "select";
}

function Switcher({ ids, active, onSelect }: { ids: string[]; active: string; onSelect: (id: string) => void }) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [highlight, setHighlight] = useState(-1);
  const [engaged, setEngaged] = useState(false);
  const [idle, setIdle] = useState(false);
  const [hover, setHover] = useState<string | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const labelRef = useRef<HTMLButtonElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const index = ids.indexOf(active);
  const showFilter = ids.length > FILTER_FROM;
  const matches = useMemo(() => filterScreens(ids, showFilter ? query : ""), [ids, query, showFilter]);

  const close = (refocus: boolean) => {
    setOpen(false);
    setQuery("");
    setHighlight(-1);
    if (refocus) labelRef.current?.focus();
  };

  const step = (delta: number) => {
    if (!ids.length) return;
    const from = index < 0 ? 0 : index;
    onSelect(ids[(from + delta + ids.length) % ids.length]);
  };

  // Fade to a hint when nobody is interacting. Re-arming on `active` also means a
  // screen switch flashes the pill back to full opacity — the confirmation you'd
  // otherwise have to hunt for.
  useEffect(() => {
    setIdle(false);
    if (engaged || open) return;
    const timer = setTimeout(() => setIdle(true), IDLE_AFTER_MS);
    return () => clearTimeout(timer);
  }, [engaged, open, active]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) close(false);
    };
    document.addEventListener("pointerdown", onDown);
    return () => document.removeEventListener("pointerdown", onDown);
  }, [open]);

  // Global left/right cycling. Prototype screens own their own keys: never fire while
  // the popover has focus, while a modifier is held, or while anything text-like is focused.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (open) return;
      if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      if (isTypingTarget(document.activeElement)) return;
      e.preventDefault();
      step(e.key === "ArrowLeft" ? -1 : 1);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, ids, active]);

  useEffect(() => {
    if (!open) return;
    if (showFilter) inputRef.current?.focus();
    else listRef.current?.focus();
  }, [open, showFilter]);

  useEffect(() => {
    if (highlight < 0) return;
    listRef.current?.querySelector(`[data-row="${highlight}"]`)?.scrollIntoView({ block: "nearest" });
  }, [highlight]);

  const onPopoverKey = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    if (e.key === "Escape") {
      e.preventDefault();
      close(true);
      return;
    }
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!matches.length) return;
      const delta = e.key === "ArrowDown" ? 1 : -1;
      setHighlight((h) =>
        h < 0 ? (delta === 1 ? 0 : matches.length - 1) : (h + delta + matches.length) % matches.length,
      );
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      const pick = matches[highlight >= 0 ? highlight : 0];
      if (pick) {
        onSelect(pick);
        close(true);
      }
    }
  };

  const arrowStyle = (key: string): CSSProperties => ({
    font: MONO,
    lineHeight: 1,
    width: 22,
    height: 24,
    display: "grid",
    placeItems: "center",
    borderRadius: 999,
    border: "none",
    cursor: "pointer",
    background: hover === key ? "rgba(255,255,255,0.10)" : "transparent",
    color: hover === key ? "#fafafa" : "rgba(250,250,250,0.65)",
    transition: "background 120ms ease, color 120ms ease",
  });

  return (
    <div
      ref={rootRef}
      onPointerEnter={() => setEngaged(true)}
      onPointerLeave={() => {
        setEngaged(false);
        setHover(null);
      }}
      onFocus={() => setEngaged(true)}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node)) setEngaged(false);
      }}
      style={{
        position: "fixed",
        bottom: 16,
        right: 16,
        zIndex: 9999,
        opacity: idle ? 0.4 : 1,
        transition: "opacity 150ms ease",
      }}
    >
      {open && (
        <div
          onKeyDown={onPopoverKey}
          style={{
            position: "absolute",
            bottom: 40,
            right: 0,
            width: 260,
            maxHeight: "60vh",
            display: "flex",
            flexDirection: "column",
            overflow: "hidden",
            background: SURFACE,
            backdropFilter: "blur(8px)",
            border: HAIRLINE,
            borderRadius: 10,
            boxShadow: "0 16px 40px rgba(0,0,0,0.45)",
          }}
        >
          {showFilter && (
            <input
              ref={inputRef}
              value={query}
              placeholder="filter…"
              spellCheck={false}
              onChange={(e) => {
                setQuery(e.target.value);
                setHighlight(-1);
              }}
              style={{
                font: MONO,
                flex: "0 0 auto",
                width: "100%",
                padding: "9px 12px",
                border: "none",
                borderBottom: HAIRLINE,
                outline: "none",
                background: "rgba(255,255,255,0.04)",
                color: "#fafafa",
              }}
            />
          )}
          <div
            ref={listRef}
            role="listbox"
            aria-label="Screens"
            tabIndex={-1}
            style={{ overflowY: "auto", padding: 4, outline: "none" }}
          >
            {matches.length === 0 ? (
              <div style={{ font: MONO, padding: "8px 10px", color: "rgba(250,250,250,0.38)" }}>no match</div>
            ) : (
              matches.map((id, i) => {
                const isActive = id === active;
                const isCursor = i === highlight;
                return (
                  <button
                    key={id}
                    data-row={i}
                    type="button"
                    role="option"
                    aria-selected={isActive}
                    onPointerEnter={() => setHighlight(i)}
                    onClick={() => {
                      onSelect(id);
                      close(true);
                    }}
                    style={{
                      font: MONO,
                      display: "flex",
                      alignItems: "center",
                      gap: 8,
                      width: "100%",
                      padding: "7px 8px",
                      borderRadius: 6,
                      border: "none",
                      cursor: "pointer",
                      textAlign: "left",
                      color: isActive ? "#fafafa" : "rgba(250,250,250,0.72)",
                      background: isActive
                        ? isCursor
                          ? "rgba(250,250,250,0.16)"
                          : "rgba(250,250,250,0.10)"
                        : isCursor
                          ? "rgba(255,255,255,0.06)"
                          : "transparent",
                      transition: "background 100ms ease",
                    }}
                  >
                    <span style={{ width: 8, flex: "0 0 auto", color: "#fafafa" }}>{isActive ? "●" : ""}</span>
                    <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{id}</span>
                  </button>
                );
              })
            )}
          </div>
        </div>
      )}

      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 2,
          padding: 3,
          background: SURFACE,
          backdropFilter: "blur(8px)",
          border: HAIRLINE,
          borderRadius: 999,
          boxShadow: "0 8px 24px rgba(0,0,0,0.35)",
        }}
      >
        <button
          type="button"
          aria-label="Previous screen"
          onClick={() => step(-1)}
          onPointerEnter={() => setHover("prev")}
          onPointerLeave={() => setHover(null)}
          style={arrowStyle("prev")}
        >
          ‹
        </button>
        <button
          ref={labelRef}
          type="button"
          aria-haspopup="listbox"
          aria-expanded={open}
          onClick={() => (open ? close(true) : setOpen(true))}
          onPointerEnter={() => setHover("label")}
          onPointerLeave={() => setHover(null)}
          style={{
            font: MONO,
            display: "flex",
            alignItems: "center",
            gap: 7,
            height: 24,
            padding: "0 10px",
            borderRadius: 999,
            border: "none",
            cursor: "pointer",
            color: "#fafafa",
            background: open || hover === "label" ? "rgba(255,255,255,0.10)" : "transparent",
            transition: "background 120ms ease",
          }}
        >
          <span style={{ maxWidth: 200, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {active}
          </span>
          <span style={{ color: "rgba(250,250,250,0.5)" }}>
            {index < 0 ? "–" : index + 1}/{ids.length}
          </span>
          <svg
            width="9"
            height="9"
            viewBox="0 0 10 10"
            aria-hidden="true"
            style={{
              flex: "0 0 auto",
              opacity: 0.65,
              transform: open ? "rotate(180deg)" : "none",
              transition: "transform 150ms ease",
            }}
          >
            <path
              d="M1.6 3.6 L5 7 L8.4 3.6"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.4"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        </button>
        <button
          type="button"
          aria-label="Next screen"
          onClick={() => step(1)}
          onPointerEnter={() => setHover("next")}
          onPointerLeave={() => setHover(null)}
          style={arrowStyle("next")}
        >
          ›
        </button>
      </div>
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
  // Screens are promised ordinary react-router-dom (README workspace deps), so the
  // host must supply a router context; MemoryRouter keeps the canvas URL and the
  // #/<screen> hash contract untouched.
  return (
    <MemoryRouter>
      <ErrorBoundary>
        <Comp />
      </ErrorBoundary>
    </MemoryRouter>
  );
}
