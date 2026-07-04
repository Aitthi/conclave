import { Search, Terminal, Layers } from "lucide-react";
import { agents, avClass } from "../lib/data";

export const meta = { title: "Roster · Activity states" };

/* Agent working-state indicator (plan F1, agent-activity-indicator @ 64f7a10).
   Direction: the left-sidebar status dot gains a WORKING treatment, a steady
   green core with a slow expanding halo, shown for an agent that emitted PTY
   output within the last 5s (lead ruling R-act-1). Three states read at a
   glance in the 266px sidebar: working (halo) / running-quiet (plain green,
   the app's current dot) / idle (plain gray). No layout shift (the halo is a
   scaled ::after), and the dot hides on row-hover with the remove affordance
   just as today. A prefers-reduced-motion fallback swaps the halo for a
   persistent green ring, so working stays distinct without any animation. */

type State = "working" | "running" | "idle" | "waiting";

const rows: { id: string; state: State }[] = [
  { id: "detoro", state: "working" },
  { id: "mellow", state: "working" },
  { id: "dew", state: "running" },
  { id: "arta", state: "idle" },
];

function Dot({ state, lg }: { state: State; lg?: boolean }) {
  return <span className={`rdot ${lg ? "rdot-lg" : ""} rdot-${state}`} aria-label={state} />;
}

function Row({ id, state }: { id: string; state: State }) {
  const a = agents[id];
  const isCli = a.role !== "Designer"; // stand-in: most swarm agents are CLI
  return (
    <div className="group flex items-center gap-2.5 px-2 py-1.5 rounded-lg cursor-pointer transition-colors hover:bg-[var(--color-hover)]">
      <span className={`av av-md ${avClass[a.color]}`}>{a.initials}</span>
      <div className="flex-1 min-w-0 leading-tight">
        <div className="text-[0.78rem] font-semibold heading flex items-center gap-1.5 truncate">
          {a.name}
          {isCli && <Terminal size={12} className="faint shrink-0" />}
        </div>
        <div className="text-[0.66rem] dim truncate">{a.role}</div>
      </div>
      {/* Status dot — hidden on hover to free the remove affordance (app parity). */}
      <span className="group-hover:hidden"><Dot state={state} /></span>
      <button
        className="hidden group-hover:grid w-5 h-5 place-items-center rounded-md faint hover:bg-[var(--color-hover)] shrink-0"
        title="Remove from workspace"
      >
        <span className="text-[0.9rem] leading-none">×</span>
      </button>
    </div>
  );
}

function Legend({
  swatch,
  name,
  desc,
}: {
  swatch: React.ReactNode;
  name: string;
  desc: string;
}) {
  return (
    <div className="flex items-start gap-3 py-2.5">
      <span className="w-4 grid place-items-center pt-0.5 shrink-0">{swatch}</span>
      <div className="min-w-0">
        <div className="text-[0.78rem] font-semibold heading">{name}</div>
        <div className="text-[0.7rem] dim leading-snug">{desc}</div>
      </div>
    </div>
  );
}

export default function RosterActivity() {
  return (
    <div className="h-screen w-full flex overflow-hidden" style={{ background: "var(--color-app)" }}>
      {/* left sidebar — the real app Roster, 266px */}
      <aside className="w-[266px] shrink-0 flex flex-col border-r" style={{ borderColor: "var(--color-border)", background: "var(--color-sidebar)" }}>
        <div className="h-12 shrink-0 flex items-center gap-2.5 px-3 border-b" style={{ borderColor: "var(--color-border)" }}>
          <span className="w-6 h-6 rounded-[7px] grid place-items-center" style={{ background: "var(--color-raised)", color: "var(--color-heading)" }}>
            <Layers size={13} />
          </span>
          <span className="heading text-[0.82rem] font-semibold tracking-tight flex-1">codeup</span>
          <span className="faint num text-[0.62rem]">{rows.length}</span>
        </div>
        <div className="px-3 py-2 shrink-0">
          <div className="flex items-center gap-2 rounded-lg px-2.5 h-7" style={{ background: "color-mix(in srgb, var(--color-faint) 10%, transparent)" }}>
            <Search size={12} className="faint shrink-0" />
            <span className="text-[0.72rem] faint">Search agents</span>
          </div>
        </div>
        <div className="label faint px-3 pt-1 pb-1">Agents</div>
        <div className="flex-1 overflow-y-auto scroll-thin px-2 pb-2 flex flex-col gap-0.5">
          {rows.map((r) => (
            <Row key={r.id} id={r.id} state={r.state} />
          ))}
        </div>
      </aside>

      {/* main — the direction explained: enlarged states + reduced-motion fallback */}
      <main className="flex-1 min-w-0 overflow-y-auto scroll-thin px-8 py-7">
        <div className="max-w-[520px]">
          <h1 className="heading text-[1.06rem] font-semibold tracking-tight">Working-state indicator</h1>
          <p className="dim text-[0.8rem] leading-relaxed mt-1.5">
            The sidebar status dot signals who is <span className="heading font-medium">actively working</span> right now: an
            agent whose backend emitted output in the last 5&nbsp;seconds. Motion carries the signal; colour still separates
            live from idle. Watch Detoro &amp; Mellow pulse in the roster at left.
          </p>

          <div className="mt-6 rounded-lg p-4" style={{ background: "var(--color-sidebar)", border: "1px solid var(--color-border)" }}>
            <div className="label faint pb-1">States</div>
            <div className="divide-y" style={{ borderColor: "var(--color-border-soft)" }}>
              <Legend
                swatch={<Dot state="working" lg />}
                name="Working"
                desc="Session live and emitting output (≤ 5s). Steady green core + slow expanding halo."
              />
              <Legend
                swatch={<Dot state="running" lg />}
                name="Running · quiet"
                desc="Session alive but idle at its prompt. The app's current static green dot, unchanged."
              />
              <Legend
                swatch={<Dot state="idle" lg />}
                name="Idle"
                desc="No live session. The app's current static gray dot, unchanged."
              />
              <Legend
                swatch={<span className="rdot rdot-lg rdot-running rdot-ring" aria-label="working (reduced motion)" />}
                name="Working · reduced motion"
                desc="prefers-reduced-motion: the halo becomes a persistent green ring, still distinct from quiet, with no animation."
              />
            </div>
          </div>

          <div className="mt-5 text-[0.72rem] faint leading-relaxed space-y-1.5">
            <p>
              <span className="dim font-medium">No layout shift.</span> The halo is a scaled pseudo-element, so rows never
              reflow as agents start and stop working.
            </p>
            <p>
              <span className="dim font-medium">Hover parity.</span> The dot still hides on row-hover to reveal the remove
              button; the working signal never fights that affordance.
            </p>
            <p>
              <span className="dim font-medium">Waiting</span> (orange) stays exactly as it is. Working is an orthogonal
              signal layered only on live sessions.
            </p>
          </div>
        </div>
      </main>
    </div>
  );
}
