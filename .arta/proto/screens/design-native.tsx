import {
  PenTool, ExternalLink, X, SquareTerminal, CornerDownLeft,
  LayoutGrid, Search, ShoppingCart, TrendingUp, Users, ArrowUpRight,
} from "lucide-react";

export const meta = { title: "Design view · full-window (ready)" };

/* Lane C canon: Conclave-native Design view, READY state (full window).
   Replaces the Arta-embedded DesignView with our OWN canvas host. While open,
   the Rail (workspace column) and Roster (agent list) are HIDDEN, so the window
   becomes canvas-LEFT + agent-terminal-RIGHT (human ruling D3). The close X
   restores the normal 3-pane layout.

   Anatomy the implementer (Lane B) builds:
   1. Titlebar drag strip (h-7), unchanged from AppShell. It is the only thing
      left of the canvas now, so the canvas header sits BELOW it and clears the
      macOS traffic lights vertically (no left inset needed on the header).
   2. Canvas header (h-12), whose visual baseline is today's DesignView.tsx
      header: PenTool, workspace name, "Design" tag, open-in-browser, close X.
   3. Canvas body: the host iframe, rendering the selected screen FULL-BLEED
      (bg is the VS Code editor tone until the screen paints). The rendered
      artboard here is a MOCK of a hosted design screen, not app chrome.
   4. Floating screen switcher, which lives INSIDE the host iframe (D6:
      discovery is the host's job, not IPC). Designed as an in-canvas floating
      bar so it reads as Conclave chrome over whatever the screen paints.
   Zero Arta imports; theme.css tokens only; lifts into src/ unmodified. */

const SCREENS = ["welcome", "orders", "pricing", "settings", "onboarding"];
const ACTIVE = "orders";

export default function DesignNative() {
  return (
    <div className="h-screen w-full flex flex-col overflow-hidden" style={{ background: "var(--color-app)" }}>
      {/* ── titlebar drag strip (macOS unified titlebar; traffic lights float
             here in the real window — hinted faintly at the left) ── */}
      <div className="h-7 shrink-0 flex items-center px-4 border-b" style={{ background: "var(--color-sidebar)", borderColor: "var(--color-border)" }}>
        <div className="flex items-center gap-2">
          {["#ff5f57", "#febc2e", "#28c840"].map((c) => (
            <span key={c} className="w-3 h-3 rounded-full" style={{ background: c, opacity: 0.9 }} />
          ))}
        </div>
        <span className="mx-auto text-[0.7rem] faint mono">codeup — Design</span>
      </div>

      {/* ── full-window split: canvas (left, flex) + terminal (right, fixed) ── */}
      <div className="flex-1 flex min-h-0">
        {/* ═══ CANVAS ═══ */}
        <section className="flex-1 min-w-0 flex flex-col border-r" style={{ borderColor: "var(--color-border)" }}>
          {/* canvas header — DesignView.tsx baseline */}
          <header className="h-12 shrink-0 flex items-center gap-2 px-3 border-b" style={{ background: "var(--color-sidebar)", borderColor: "var(--color-border)" }}>
            <PenTool size={14} className="faint shrink-0" />
            <span className="text-[0.82rem] font-semibold shrink-0" style={{ color: "var(--color-heading)" }}>codeup</span>
            <span className="text-[0.7rem] faint shrink-0">Design</span>
            <div className="ml-auto flex items-center gap-1 shrink-0">
              <button className="w-7 h-7 grid place-items-center rounded-md dim" title="Open in browser"
                style={{ transition: "background .12s" }}>
                <ExternalLink size={14} />
              </button>
              <button className="w-7 h-7 grid place-items-center rounded-md dim" title="Close Design"
                style={{ transition: "background .12s" }}>
                <X size={14} />
              </button>
            </div>
          </header>

          {/* canvas body: host iframe region. The rendered screen paints
              FULL-BLEED inside a min-h-full wrapper (so the wrapper has real
              height for the floating switcher to anchor against); the switcher
              floats over its bottom. */}
          <div className="flex-1 min-h-0 relative overflow-y-auto scroll-thin" style={{ background: "#1e1e1e" }}>
            <div className="relative min-h-full">
              <RenderedScreenMock />

              {/* ── floating screen switcher (renders INSIDE the host iframe;
                   D6: discovery is the host's job, not IPC) ── */}
              <div className="absolute inset-x-0 bottom-5 z-10 flex justify-center pointer-events-none">
                <div className="flex items-center gap-1 pl-2.5 pr-1.5 py-1.5 rounded-full pointer-events-auto"
                style={{
                  background: "color-mix(in srgb, var(--color-raised) 82%, transparent)",
                  border: "1px solid var(--color-border)",
                  boxShadow: "var(--shadow-pop)",
                  backdropFilter: "blur(12px)",
                }}>
                <LayoutGrid size={13} className="faint shrink-0" />
                <span className="mono text-[0.62rem] faint mr-1 shrink-0">{SCREENS.length}</span>
                <div className="flex items-center gap-0.5">
                  {SCREENS.map((s) => {
                    const on = s === ACTIVE;
                    return (
                      <span key={s}
                        className="mono text-[0.72rem] px-2 py-1 rounded-full whitespace-nowrap cursor-pointer"
                        style={on
                          ? { color: "var(--color-accent)", background: "color-mix(in srgb, var(--color-accent) 14%, transparent)", boxShadow: "inset 0 0 0 1px color-mix(in srgb, var(--color-accent) 34%, transparent)" }
                          : { color: "var(--color-dim)" }}>
                        {s}
                      </span>
                    );
                  })}
                </div>
              </div>
              </div>
            </div>
          </div>
        </section>

        {/* ═══ AGENT TERMINAL (kept on the right while designing) ═══ */}
        <aside className="w-[420px] shrink-0 flex flex-col min-h-0" style={{ background: "var(--color-center)" }}>
          <div className="flex items-center gap-2 px-4 h-9 shrink-0 faint text-[0.74rem] mono border-b" style={{ borderColor: "var(--color-border-soft)" }}>
            <SquareTerminal size={13} /> Arta · Designer · running
          </div>
          <div className="flex-1 p-5 mono text-[0.73rem] leading-relaxed faint select-none overflow-hidden">
            <div style={{ color: "var(--color-faint)" }}>Claude Code v2.1.200</div>
            <div className="mt-3">Welcome back Deew!</div>
            <div className="mt-3 opacity-70">Opus 4.8 (1M context) · Claude Max</div>
            <div className="mt-6 flex items-start gap-2">
              <span style={{ color: "#3a6d55" }}>›</span>
              <span className="opacity-80">design an orders screen for the dashboard</span>
            </div>
            <div className="mt-3 flex items-start gap-2">
              <PenTool size={12} className="mt-0.5 shrink-0" style={{ color: "var(--color-a-sky)" }} />
              <span className="opacity-70">Wrote <span style={{ color: "var(--color-text)" }}>design/screens/orders.tsx</span> — live in the canvas.</span>
            </div>
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

/* A MOCK of a design screen the host is rendering full-bleed — deliberately a
   LIGHT product screen so the dark app chrome (header, switcher) reads as
   Conclave's frame around a live artboard. Not part of the canon UI itself. */
function RenderedScreenMock() {
  const stats = [
    { label: "Revenue", value: "$48.2k", delta: "+12.4%", icon: TrendingUp },
    { label: "Orders", value: "1,284", delta: "+3.1%", icon: ShoppingCart },
    { label: "Customers", value: "342", delta: "+8.0%", icon: Users },
  ];
  const rows = [
    { id: "#3921", who: "Nadia Fields", total: "$128.00", status: "Paid" },
    { id: "#3920", who: "Ivan Okoro", total: "$54.90", status: "Paid" },
    { id: "#3919", who: "Mara Lindt", total: "$312.40", status: "Refund" },
    { id: "#3918", who: "Theo Vance", total: "$76.00", status: "Paid" },
    { id: "#3917", who: "Priya Raman", total: "$204.10", status: "Paid" },
    { id: "#3916", who: "Colin Wu", total: "$18.50", status: "Paid" },
    { id: "#3915", who: "Dana Kerr", total: "$96.75", status: "Refund" },
    { id: "#3914", who: "Sam Iverson", total: "$149.99", status: "Paid" },
    { id: "#3913", who: "Lena Fischer", total: "$67.20", status: "Paid" },
    { id: "#3912", who: "Omar Haddad", total: "$233.00", status: "Paid" },
  ];
  return (
    <div className="min-h-full" style={{ background: "#f6f7f9", color: "#1c1f24", fontFamily: "var(--font-body)" }}>
      <div className="max-w-[720px] mx-auto px-8 py-8">
        <div className="flex items-center justify-between">
          <div>
            <h1 className="text-[1.5rem] font-semibold tracking-tight" style={{ color: "#111318" }}>Orders</h1>
            <p className="text-[0.82rem]" style={{ color: "#6b7280" }}>Last 30 days across all channels</p>
          </div>
          <div className="flex items-center gap-2 px-3 h-9 rounded-lg text-[0.8rem]" style={{ background: "#fff", border: "1px solid #e5e7eb", color: "#6b7280" }}>
            <Search size={14} /> Search orders
          </div>
        </div>

        <div className="grid grid-cols-3 gap-3 mt-6">
          {stats.map((s) => (
            <div key={s.label} className="p-4 rounded-xl" style={{ background: "#fff", border: "1px solid #eceef1" }}>
              <div className="flex items-center gap-1.5 text-[0.74rem]" style={{ color: "#6b7280" }}>
                <s.icon size={13} /> {s.label}
              </div>
              <div className="mt-2 text-[1.35rem] font-semibold" style={{ color: "#111318" }}>{s.value}</div>
              <div className="mt-0.5 inline-flex items-center gap-0.5 text-[0.72rem] font-medium" style={{ color: "#12924a" }}>
                <ArrowUpRight size={12} /> {s.delta}
              </div>
            </div>
          ))}
        </div>

        <div className="mt-6 rounded-xl overflow-hidden" style={{ background: "#fff", border: "1px solid #eceef1" }}>
          {rows.map((r, i) => (
            <div key={r.id} className="flex items-center gap-4 px-4 py-3 text-[0.84rem]"
              style={{ borderTop: i === 0 ? "none" : "1px solid #f1f2f4" }}>
              <span className="font-medium tabular-nums" style={{ color: "#111318", fontFamily: "var(--font-mono)", fontSize: "0.78rem" }}>{r.id}</span>
              <span className="flex-1" style={{ color: "#374151" }}>{r.who}</span>
              <span className="tabular-nums" style={{ color: "#111318" }}>{r.total}</span>
              <span className="text-[0.7rem] font-semibold px-2 py-0.5 rounded-full"
                style={r.status === "Paid"
                  ? { background: "#e7f6ee", color: "#12924a" }
                  : { background: "#fbeaea", color: "#c0392b" }}>
                {r.status}
              </span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
