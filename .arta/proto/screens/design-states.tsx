import { PenTool, X, LoaderCircle, RotateCcw } from "lucide-react";

export const meta = { title: "Design view · canvas states" };

/* Lane C canon: the non-ready CANVAS states, rendered by DesignView itself
   (Lane B React), NOT by the host, because a failed sidecar has no iframe to
   show them in. Copy is pinned VERBATIM from today's DesignView.tsx so the
   language carries over unchanged:
     loading:        "Starting design viewer…"
     node missing:   "Node.js is required for the Design view."
                     "Install Node.js ≥18 and reopen."           + Retry
     sidecar failed: "Couldn't start the design viewer."
                     <raw error message>                          + Retry
   This is a reference board (three mini-canvases), not a full window, so all
   three states and their exact strings read at a glance for the implementer.
   Zero Arta imports; theme.css tokens only. */

/* One mini-canvas: the DesignView header strip over the #1e1e1e canvas body. */
function CanvasFrame({ caption, children }: { caption: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col">
      <span className="label mb-2">{caption}</span>
      <div className="rounded-xl overflow-hidden flex flex-col" style={{ border: "1px solid var(--color-border)", height: 300 }}>
        {/* header (compact DesignView baseline) */}
        <div className="h-11 shrink-0 flex items-center gap-2 px-3 border-b" style={{ background: "var(--color-sidebar)", borderColor: "var(--color-border)" }}>
          <PenTool size={13} className="faint shrink-0" />
          <span className="text-[0.78rem] font-semibold shrink-0" style={{ color: "var(--color-heading)" }}>codeup</span>
          <span className="text-[0.66rem] faint shrink-0">Design</span>
          <span className="ml-auto w-6 h-6 grid place-items-center rounded-md faint"><X size={13} /></span>
        </div>
        {/* body */}
        <div className="flex-1 min-h-0 grid place-items-center px-5" style={{ background: "#1e1e1e" }}>
          {children}
        </div>
      </div>
    </div>
  );
}

/* Retry pill — DesignView's Retry button (bg-overlay/[0.06]). */
function Retry() {
  return (
    <button className="inline-flex items-center gap-1.5 px-3 h-7 rounded-md text-[0.74rem] font-medium"
      style={{ background: "var(--color-raised)", color: "var(--color-heading)", border: "1px solid var(--color-border)" }}>
      <RotateCcw size={12} /> Retry
    </button>
  );
}

export default function DesignStates() {
  return (
    <div className="min-h-screen w-full px-8 py-8" style={{ background: "var(--color-app)" }}>
      <div className="max-w-[1100px] mx-auto">
        <h1 className="text-[1.15rem] font-semibold" style={{ color: "var(--color-heading)" }}>Design view — canvas states</h1>
        <p className="mt-1 text-[0.84rem] dim">
          What the canvas shows before a screen renders. Rendered by{" "}
          <span className="mono text-[0.8rem]" style={{ color: "var(--color-text)" }}>DesignView</span>, not the host.
          Empty state (screens present, none yet) is its own screen —{" "}
          <span className="mono text-[0.8rem]" style={{ color: "var(--color-text)" }}>design-empty</span>.
        </p>

        <div className="grid grid-cols-3 gap-5 mt-7">
          {/* loading */}
          <CanvasFrame caption="Loading">
            <div className="flex flex-col items-center gap-2.5 text-[0.82rem] faint">
              <LoaderCircle size={20} className="animate-spin" />
              <span>Starting design viewer…</span>
            </div>
          </CanvasFrame>

          {/* node missing */}
          <CanvasFrame caption="Error · Node missing">
            <div className="flex flex-col items-center gap-3 text-center max-w-[280px]">
              <span className="text-[0.84rem] font-medium" style={{ color: "var(--color-rose)" }}>
                Node.js is required for the Design view.
              </span>
              <span className="text-[0.78rem] faint">Install Node.js ≥18 and reopen.</span>
              <Retry />
            </div>
          </CanvasFrame>

          {/* sidecar failed */}
          <CanvasFrame caption="Error · Sidecar failed">
            <div className="flex flex-col items-center gap-3 text-center max-w-[280px]">
              <span className="text-[0.84rem] font-medium" style={{ color: "var(--color-rose)" }}>
                Couldn't start the design viewer.
              </span>
              <span className="text-[0.76rem] faint mono leading-relaxed">
                sidecar exited before the health check passed
              </span>
              <Retry />
            </div>
          </CanvasFrame>
        </div>

        <p className="mt-6 text-[0.76rem] faint">
          The sidecar-failed sub-line is the raw backend error string (AppError collapses to text at the
          Tauri boundary); node-missing is matched on the{" "}
          <span className="mono">node was not found on PATH</span> marker per DesignView.tsx.
        </p>
      </div>
    </div>
  );
}
