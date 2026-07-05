import {
  PenTool, ExternalLink, X, SquareTerminal, CornerDownLeft, LayoutGrid, ArrowRight,
} from "lucide-react";

export const meta = { title: "Design view · empty state" };

/* Lane C canon: Conclave-native Design view, EMPTY state (full window).
   Serves BOTH first-run cases with the SAME UI:
     - no design/ folder yet (design.ensure scaffolds it, then this shows until
       a screen exists), and
     - a design/ folder with zero screens/*.tsx.
   The host sidecar IS running (the iframe is up), so this empty UI is rendered
   INSIDE the host, in Conclave's own DARK chrome (not a light artboard) — same
   surface the floating switcher lives on. The app header therefore still shows
   open-in-browser + close X (state is "ready", just screen-less), and there is
   NO floating switcher (nothing to switch between).
   Zero Arta imports; theme.css tokens only; lifts into the host unmodified. */

export default function DesignEmpty() {
  return (
    <div className="h-screen w-full flex flex-col overflow-hidden" style={{ background: "var(--color-app)" }}>
      {/* titlebar drag strip */}
      <div className="h-7 shrink-0 flex items-center px-4 border-b" style={{ background: "var(--color-sidebar)", borderColor: "var(--color-border)" }}>
        <div className="flex items-center gap-2">
          {["#ff5f57", "#febc2e", "#28c840"].map((c) => (
            <span key={c} className="w-3 h-3 rounded-full" style={{ background: c, opacity: 0.9 }} />
          ))}
        </div>
        <span className="mx-auto text-[0.7rem] faint mono">codeup — Design</span>
      </div>

      <div className="flex-1 flex min-h-0">
        {/* ═══ CANVAS ═══ */}
        <section className="flex-1 min-w-0 flex flex-col border-r" style={{ borderColor: "var(--color-border)" }}>
          <header className="h-12 shrink-0 flex items-center gap-2 px-3 border-b" style={{ background: "var(--color-sidebar)", borderColor: "var(--color-border)" }}>
            <PenTool size={14} className="faint shrink-0" />
            <span className="text-[0.82rem] font-semibold shrink-0" style={{ color: "var(--color-heading)" }}>codeup</span>
            <span className="text-[0.7rem] faint shrink-0">Design</span>
            <div className="ml-auto flex items-center gap-1 shrink-0">
              <button className="w-7 h-7 grid place-items-center rounded-md dim" title="Open in browser"><ExternalLink size={14} /></button>
              <button className="w-7 h-7 grid place-items-center rounded-md dim" title="Close Design"><X size={14} /></button>
            </div>
          </header>

          {/* host-rendered empty state, centered on the dark canvas */}
          <div className="flex-1 min-h-0 grid place-items-center px-6" style={{ background: "#1e1e1e" }}>
            <div className="flex flex-col items-center text-center max-w-[380px]">
              <div className="w-14 h-14 rounded-2xl grid place-items-center mb-5"
                style={{ background: "color-mix(in srgb, var(--color-accent) 12%, transparent)", border: "1px solid color-mix(in srgb, var(--color-accent) 26%, transparent)" }}>
                <LayoutGrid size={22} style={{ color: "var(--color-accent)" }} />
              </div>
              <h1 className="text-[1.05rem] font-semibold" style={{ color: "var(--color-heading)" }}>No screens yet</h1>
              <p className="mt-2 text-[0.84rem] leading-relaxed dim">
                Screens are React files in{" "}
                <span className="mono text-[0.8rem]" style={{ color: "var(--color-text)" }}>design/screens/</span>.
                Ask your agent to create one and it shows up here live.
              </p>
              <div className="mt-5 inline-flex items-center gap-2 pl-3 pr-2.5 py-1.5 rounded-full text-[0.76rem] dim"
                style={{ background: "var(--color-raised)", border: "1px solid var(--color-border)" }}>
                Try <span className="mono" style={{ color: "var(--color-text)" }}>"design a settings screen"</span>
                <ArrowRight size={13} style={{ color: "var(--color-accent)" }} />
              </div>
            </div>
          </div>
        </section>

        {/* ═══ AGENT TERMINAL ═══ */}
        <aside className="w-[420px] shrink-0 flex flex-col min-h-0" style={{ background: "var(--color-center)" }}>
          <div className="flex items-center gap-2 px-4 h-9 shrink-0 faint text-[0.74rem] mono border-b" style={{ borderColor: "var(--color-border-soft)" }}>
            <SquareTerminal size={13} /> Arta · Designer · running
          </div>
          <div className="flex-1 p-5 mono text-[0.73rem] leading-relaxed faint select-none overflow-hidden">
            <div style={{ color: "var(--color-faint)" }}>Claude Code v2.1.200</div>
            <div className="mt-3">Welcome back Deew!</div>
            <div className="mt-3 opacity-70">Opus 4.8 (1M context) · Claude Max</div>
            <div className="mt-6 flex items-center gap-2"><span style={{ color: "#3a6d55" }}>›</span> /arta:arta &lt;what to design&gt;</div>
            <div className="mt-2 opacity-60">▸ bypass permissions on (shift+tab to cycle)</div>
          </div>
          <div className="px-3 py-2.5 border-t flex items-center gap-2 text-[0.76rem] shrink-0" style={{ borderColor: "var(--color-border)" }}>
            <span className="chip"><span className="w-1.5 h-1.5 rounded-full" style={{ background: "var(--color-a-sky)" }} /> Arta · self</span>
            <span className="faint mono">›</span>
            <span className="faint clamp-1 min-w-0">Message the agent…</span>
            <span className="ml-auto w-7 h-7 grid place-items-center rounded-md faint shrink-0" style={{ border: "1px solid var(--color-border)" }}><CornerDownLeft size={14} /></span>
          </div>
        </aside>
      </div>
    </div>
  );
}
