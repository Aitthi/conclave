import { useEffect, useRef, useState } from "react";
import { AlertTriangle, AppWindow, Check, Copy, Maximize, Minus, Monitor, MessageSquarePlus, Palette, Plus, Smartphone, Tablet, X } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { FrameKind, Prototype, Spec } from "../../lib/types";
import { MONO, useTheme } from "../../lib/theme";
import { cn } from "../../lib/utils";
import { sendFeedback } from "../../lib/useArta";
import { DeviceFrame } from "../proto/DeviceFrame";
import { ProtoDevice } from "../proto/ProtoDevice";
import type { AnnotateTarget } from "../proto/ProtoDevice";
import { SpecRail } from "./SpecRail";
import { DesignSystemView } from "./DesignSystemView";

interface Props {
  projectId: string;
  prototype: Prototype;
  spec: Spec;
  screen: string | undefined;
  go: (to?: string) => void;
  specOpen: boolean;
  onToggleSpec: () => void;
  onError: (message: string) => void;
}

const FRAMES: { key: FrameKind; label: string; Icon: LucideIcon }[] = [
  { key: "web", label: "Web", Icon: Monitor },
  { key: "desktop", label: "Desktop", Icon: AppWindow },
  { key: "ios", label: "iOS", Icon: Smartphone },
  { key: "android", label: "Android", Icon: Smartphone },
  { key: "ipad", label: "iPad", Icon: Tablet },
];

export function PrototypeTab({
  projectId,
  prototype,
  spec,
  screen,
  go,
  specOpen,
  onToggleSpec,
  onError,
}: Props) {
  const { c, grid } = useTheme();
  const [view, setView] = useState<"preview" | "design">("preview");
  const screens = prototype.screens || [];
  const cur = screens.find((s) => s.id === screen) || screens[0] || { id: "", title: "—" };
  const legacy = !!prototype.legacy;

  const stateFrame: FrameKind = cur.frame || prototype.frame || "web";
  const [frameOverride, setFrameOverride] = useState<FrameKind | null>(null);
  useEffect(() => setFrameOverride(null), [stateFrame]);
  const frame = frameOverride ?? stateFrame;
  // Phone / tablet frames — they get the centered canvas layout. iPad included.
  const isMobile = frame === "ios" || frame === "android" || frame === "ipad";
  // A web/desktop-DECLARED prototype previewed at phone/tablet size is a website viewed in
  // a mobile browser — render a browser bar (not a native iOS status bar that would overlap
  // the un-padded web header). Only when the dev overrides the frame; a natively-declared
  // ios/android prototype keeps its real status bar.
  const browser = (stateFrame === "web" || stateFrame === "desktop") && isMobile;

  // Full-screen with the chrome on top: content is edge-to-edge (no safe-area bands)
  // and the iOS-style status bar (real time) + notch + home indicator float OVER it —
  // no on/off toggle. A screen can hide the chrome with `chrome: false` in state.
  const chrome = cur.chrome ?? prototype.chrome ?? true;

  // The device-frame outer node — the snapshot captures THIS (bezel + chrome + content),
  // so the agent sees the same framed device the dev sees, not a bare content card.
  const frameNodeRef = useRef<HTMLDivElement>(null);

  // Comment-on-element mode: click an element in the prototype to attach feedback.
  const [annotate, setAnnotate] = useState(false);
  const [target, setTarget] = useState<AnnotateTarget | null>(null);
  useEffect(() => {
    setAnnotate(false);
    setTarget(null);
  }, [cur.id]);

  // Canvas zoom: the device scales in place (100% = fit). No panning — it stays put on the canvas.
  // The user zoom is an OUTER transform composed on top of DeviceFrame's own shrink-to-fit, which
  // measures clientWidth (layout, not transformed) — so the two never feed back into each other.
  const [zoom, setZoom] = useState(1);
  const resetView = () => setZoom(1);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => resetView(), [cur.id, frame, view]);
  const stageRef = useRef<HTMLDivElement>(null);
  // Native wheel listener (passive:false) so cmd/ctrl+wheel can zoom without scrolling the page.
  useEffect(() => {
    const el = stageRef.current;
    if (!el || view !== "preview") return;
    const onWheel = (e: WheelEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      e.preventDefault();
      setZoom((z) => Math.min(3, Math.max(0.25, z - e.deltaY * 0.0015)));
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [view]);

  return (
    <div className="flex h-full min-w-0 flex-1 flex-col">
      <div className="flex min-h-0 flex-1">
          <div
            className="flex w-[216px] shrink-0 flex-col border-r"
            style={{ borderColor: c.border, background: c.panel }}
          >
        <div
          className="px-[15px] pb-[9px] pt-3.5 text-[10.5px] font-medium uppercase tracking-[0.6px]"
          style={{ fontFamily: MONO, color: c.faint }}
        >
          Screens
        </div>
        <div className="flex flex-1 flex-col gap-0.5 overflow-auto px-2">
          {screens.map((sc) => {
            const active = sc.id === cur.id;
            return (
              <button
                key={sc.id}
                onClick={() => go(sc.id)}
                className={cn(
                  "flex items-center gap-[9px] rounded-[7px] px-2.5 py-2 text-[13px] transition-colors",
                  !active && "hover:opacity-80"
                )}
                style={{
                  color: active ? c.text : c.dim,
                  background: active ? c.card : "transparent",
                  boxShadow: active ? `inset 0 0 0 1px ${c.border2}` : "none",
                }}
              >
                <AppWindow size={15} color={active ? c.accent : c.faint} />
                <span className="overflow-hidden text-ellipsis whitespace-nowrap">{sc.title}</span>
              </button>
            );
          })}
        </div>

      </div>

      {/* Canvas — a zoom workspace; the device scales in place (no panning) */}
      <div
        ref={stageRef}
        className="relative flex min-w-0 flex-1 overflow-hidden"
        style={{
          background: c.bg,
          backgroundImage: grid && view === "preview" ? `radial-gradient(${c.gridDot} 1px, transparent 1px)` : "none",
          backgroundSize: "22px 22px",
        }}
      >
        {/* floating: Preview | Design system */}
        <div
          data-toolbar
          className="absolute left-3 top-3 z-20 flex items-center gap-0.5 rounded-xl p-1 backdrop-blur"
          style={{ background: `${c.panel}e6`, border: `1px solid ${c.border}`, boxShadow: c.shadow }}
        >
          {([["preview", "Preview", Monitor], ["design", "Design system", Palette]] as const).map(([id, label, Icon]) => {
            const on = view === id;
            return (
              <button
                key={id}
                onClick={() => setView(id)}
                className="flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-[12.5px] font-medium transition-colors"
                style={{ color: on ? c.accent2 : c.dim, background: on ? c.accentSoft : "transparent" }}
              >
                <Icon size={14} /> {label}
              </button>
            );
          })}
        </div>

        {view === "preview" && !legacy && (
          <>
            {/* floating: device selector · comment · zoom */}
            <div
              data-toolbar
              className="absolute left-1/2 top-3 z-20 flex -translate-x-1/2 items-center gap-0.5 rounded-xl px-1.5 py-1 backdrop-blur"
              style={{ background: `${c.panel}e6`, border: `1px solid ${c.border}`, boxShadow: c.shadow }}
            >
              {FRAMES.map(({ key, label, Icon }) => {
                const active = frame === key;
                return (
                  <button
                    key={key}
                    onClick={() => setFrameOverride(key)}
                    className="grid h-8 w-8 place-items-center rounded-lg transition-colors"
                    style={{ color: active ? c.accent2 : c.dim, background: active ? c.accentSoft : "transparent" }}
                    title={label}
                  >
                    <Icon size={16} />
                  </button>
                );
              })}
              <span className="mx-0.5 h-5 w-px" style={{ background: c.border }} />
              <button
                onClick={() => {
                  setAnnotate((a) => !a);
                  setTarget(null);
                }}
                className="grid h-8 w-8 place-items-center rounded-lg transition-colors"
                style={{ color: annotate ? c.accent2 : c.dim, background: annotate ? c.accentSoft : "transparent" }}
                title={annotate ? "Click an element…" : "Comment on an element"}
              >
                <MessageSquarePlus size={16} />
              </button>
              <span className="mx-0.5 h-5 w-px" style={{ background: c.border }} />
              <button onClick={() => setZoom((z) => Math.max(0.25, +(z - 0.1).toFixed(2)))} className="grid h-8 w-8 place-items-center rounded-lg transition-colors" style={{ color: c.dim }} title="Zoom out">
                <Minus size={16} />
              </button>
              <button onClick={resetView} className="min-w-[52px] rounded-md px-1 py-1 text-center text-[12px] font-medium tabular-nums transition-colors" style={{ color: c.text }} title="Reset view">
                {Math.round(zoom * 100)}%
              </button>
              <button onClick={() => setZoom((z) => Math.min(3, +(z + 0.1).toFixed(2)))} className="grid h-8 w-8 place-items-center rounded-lg transition-colors" style={{ color: c.dim }} title="Zoom in">
                <Plus size={16} />
              </button>
              <span className="mx-0.5 h-5 w-px" style={{ background: c.border }} />
              <button onClick={resetView} className="grid h-8 w-8 place-items-center rounded-lg transition-colors" style={{ color: c.dim }} title="Reset view">
                <Maximize size={16} />
              </button>
            </div>
          </>
        )}

        {legacy ? (
          <div className="absolute inset-0 flex items-center justify-center px-6">
            <div
              className="flex max-w-[440px] flex-col items-center gap-3 rounded-2xl p-7 text-center"
              style={{ background: c.panel, border: `1px solid ${c.border2}`, boxShadow: c.shadow }}
            >
              <div className="grid h-11 w-11 place-items-center rounded-xl" style={{ background: c.accentSoft }}>
                <AlertTriangle size={22} color={c.accent2} />
              </div>
              <p className="text-[13.5px] leading-[1.6]" style={{ color: c.text }}>
                This prototype used the old HTML format — a backup is at{" "}
                <code className="rounded px-1.5 py-0.5" style={{ fontFamily: MONO, background: c.panel2, color: c.accent }}>
                  .arta/legacy-html-backup/
                </code>
                . Ask your agent to{" "}
                <code className="rounded px-1.5 py-0.5" style={{ fontFamily: MONO, background: c.panel2, color: c.accent }}>
                  /arta:arta update
                </code>{" "}
                to regenerate it as React screens.
              </p>
            </div>
          </div>
        ) : view === "design" ? (
          <div className="absolute inset-0 overflow-auto pt-[52px]">
            <DesignSystemView prototype={prototype} projectId={projectId} />
          </div>
        ) : (
          <div
            className={cn("absolute inset-0 flex px-6 pb-6 pt-[68px]", isMobile ? "items-center justify-center" : "items-stretch justify-center")}
            style={{
              transform: `scale(${zoom})`,
              transformOrigin: "center",
              transition: "transform .12s ease-out",
            }}
          >
            <div className="contents">
              <DeviceFrame
                frame={frame}
                url={cur.url || `shop.demo/${cur.id || ""}`}
                title={cur.title}
                safeArea={cur.safeArea ?? prototype.safeArea}
                chrome={chrome}
                browser={browser}
                rootRef={frameNodeRef}
              >
                <ProtoDevice
                  projectId={projectId}
                  screenId={cur.id}
                  title={cur.title}
                  annotate={annotate}
                  captureNodeRef={frameNodeRef}
                  go={go}
                  onError={onError}
                  onAnnotate={(t) => {
                    setTarget(t);
                    setAnnotate(false);
                  }}
                />
              </DeviceFrame>
            </div>
          </div>
        )}

        {target && (
          <AnnotationComposer
            target={target}
            screen={cur.id}
            onClose={() => setTarget(null)}
            onCommentMore={() => {
              setTarget(null);
              setAnnotate(true); // back to click-an-element mode for the next note
            }}
          />
        )}
      </div>

        <SpecRail spec={spec} open={specOpen} onToggle={onToggleSpec} />
      </div>

    </div>
  );
}

// Compose feedback anchored to a clicked element, so the note reaches the agent
// with the exact target ("this button on cart"), not as a vague description. The
// viewer is read-only and CAN'T message the AI itself — saving queues the note in
// .arta/feedback.json; the dev then runs `/arta:arta feedback` in chat to have the AI
// drain it. So the button SAVES (not "sends"), and the saved state surfaces that command.
// Conclave vendoring deviation: upstream told the dev to run a Claude-plugin
// slash command (`/arta:arta feedback`) that does not exist here — any agent
// reads feedback straight off disk (design-canvas skill), so this points at
// the file instead of a vendor-specific command.
const FEEDBACK_HINT = ".arta/feedback.json";
function AnnotationComposer({
  target,
  screen,
  onClose,
  onCommentMore,
}: {
  target: AnnotateTarget;
  screen: string;
  onClose: () => void;
  onCommentMore: () => void;
}) {
  const { c } = useTheme();
  const [text, setText] = useState("");
  const [saved, setSaved] = useState(false);
  const [copied, setCopied] = useState(false);

  const submit = async () => {
    const t = text.trim();
    if (!t) return;
    const ok = await sendFeedback({ text: t, tab: "prototype", screen, element: target });
    if (ok) setSaved(true); // don't auto-close — show the "what next" step
  };

  const copyCmd = () => {
    navigator.clipboard
      ?.writeText(FEEDBACK_HINT)
      .then(() => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), 1500);
      })
      .catch(() => {});
  };

  return (
    <div
      className="absolute left-1/2 top-4 z-20 w-[340px] -translate-x-1/2 rounded-xl p-3 shadow-2xl"
      style={{ background: c.panel, border: `1px solid ${c.accent}` }}
    >
      <div className="mb-2 flex items-center gap-2">
        <span className="text-[10px] uppercase tracking-[0.6px]" style={{ color: c.faint }}>
          {saved ? "Comment saved" : "Comment on"}
        </span>
        {!saved && (
          <code
            className="rounded px-1.5 py-0.5 text-[11px]"
            style={{ fontFamily: MONO, background: c.panel2, color: c.accent }}
          >
            {target.selector}
          </code>
        )}
        <button onClick={onClose} className="ml-auto" style={{ color: c.faint }}>
          <X size={14} />
        </button>
      </div>

      {saved ? (
        <div className="flex flex-col gap-2.5">
          <div className="flex items-center gap-1.5 text-[12px] font-medium" style={{ color: c.green }}>
            <Check size={13} /> Saved to this project
          </div>
          <p className="text-[11.5px] leading-[1.5]" style={{ color: c.dim }}>
            The viewer can’t message the agent itself — point it at this file so it picks
            up your comments:
          </p>
          <div
            className="flex items-center gap-2 rounded-lg p-1 pl-2.5"
            style={{ background: c.bg, border: `1px solid ${c.border}` }}
          >
            <code className="flex-1 truncate text-[12px]" style={{ fontFamily: MONO, color: c.text }}>
              {FEEDBACK_HINT}
            </code>
            <button
              onClick={copyCmd}
              className="flex items-center gap-1.5 rounded-md px-2.5 py-1.5 text-[11.5px] font-semibold"
              style={{ background: c.invBg, color: c.invFg }}
            >
              {copied ? <Check size={12} /> : <Copy size={12} />}
              {copied ? "Copied" : "Copy"}
            </button>
          </div>
          <div className="flex items-center gap-2 pt-0.5">
            <button
              onClick={() => onCommentMore()}
              className="flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-[12px] font-medium"
              style={{ border: `1px solid ${c.border2}`, color: c.text, background: c.panel2 }}
            >
              <MessageSquarePlus size={12} /> Comment more
            </button>
            <button
              onClick={onClose}
              className="ml-auto rounded-lg px-3 py-1.5 text-[12px] font-medium"
              style={{ color: c.dim }}
            >
              Done
            </button>
          </div>
        </div>
      ) : (
        <>
          {target.text && (
            <div className="mb-2 truncate text-[11px]" style={{ color: c.dim }}>
              “{target.text}”
            </div>
          )}
          <textarea
            autoFocus
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) submit();
            }}
            placeholder="What should change here?"
            spellCheck={false}
            className="h-16 w-full resize-none rounded-lg p-2.5 text-[12px] leading-[1.5] outline-none"
            style={{ fontFamily: "var(--font-sans)", background: c.bg, border: `1px solid ${c.border}`, color: c.text }}
          />
          <div className="mt-2 flex items-center justify-between">
            <span className="text-[10px]" style={{ color: c.faint }}>
              ⌘↵ to save
            </span>
            <button
              onClick={submit}
              className="flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-[12px] font-semibold disabled:opacity-50"
              style={{ background: c.invBg, color: c.invFg }}
              disabled={!text.trim()}
            >
              <MessageSquarePlus size={12} />
              Save comment
            </button>
          </div>
        </>
      )}
    </div>
  );
}
