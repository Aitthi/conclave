import { Search, Terminal, Layers, LoaderCircle } from "lucide-react";
import { agents, avClass } from "../lib/data";

export const meta = { title: "Roster · Activity states" };

/* Agent working-state indicator, round 2 (human-directed, supersedes the F2
   green halo in plan:agent-activity-indicator).
   Direction: the small right-side dot was too subtle. A working agent, one
   whose backend emitted PTY output in the last 5s, now shows an amber sub-line
   under its name/role: a slow-spinning amber icon plus a "working" label with a
   cycling ellipsis, clearly visible in the 266px sidebar. The right status dot
   is unchanged (green running, gray idle, orange waiting) and still hides on
   row-hover with the remove affordance. Amber (#ffb224) is a yellow-gold,
   deliberately yellower than waiting-orange (#ff9f0a); the icon and animated
   text are the real differentiator from the bare, static waiting dot. The row
   expands only while working (grid-rows 0fr->1fr slide-in) so idle rows stay
   compact. A prefers-reduced-motion fallback stops the spin and shows a static
   "working..." so the signal survives without any animation. */

type State = "working" | "running" | "idle" | "waiting";

const rows: { id: string; state: State }[] = [
  { id: "detoro", state: "working" },
  { id: "mellow", state: "working" },
  { id: "dew", state: "running" },
  { id: "arta", state: "idle" },
];

// The right-side dot reflects the persistent SESSION state; a working agent's
// session is live, so its dot stays green (running). Working layers on top.
const dotState = (s: State): State => (s === "working" ? "running" : s);

function Dot({ state, lg }: { state: State; lg?: boolean }) {
  return <span className={`rdot ${lg ? "rdot-lg" : ""} rdot-${state}`} aria-label={state} />;
}

function WorkLine({ on }: { on: boolean }) {
  return (
    <div className={`rwork-slot ${on ? "is-working" : ""}`} aria-hidden={!on}>
      <div className="rwork">
        <div className="rwork-inner">
          <LoaderCircle size={11} className="rwork-ico" />
          <span className="rwork-label">working</span>
        </div>
      </div>
    </div>
  );
}

function Row({ id, state }: { id: string; state: State }) {
  const a = agents[id];
  const isCli = a.role !== "Designer"; // stand-in: most swarm agents are CLI
  const working = state === "working";
  return (
    <div className="group flex items-center gap-2.5 px-2 py-1.5 rounded-lg cursor-pointer transition-colors hover:bg-[var(--color-hover)]">
      <span className={`av av-md ${avClass[a.color]}`}>{a.initials}</span>
      <div className="flex-1 min-w-0 leading-tight">
        <div className="text-[0.78rem] font-semibold heading flex items-center gap-1.5 truncate">
          {a.name}
          {isCli && <Terminal size={12} className="faint shrink-0" />}
        </div>
        <div className="text-[0.66rem] dim truncate">{a.role}</div>
        <WorkLine on={working} />
      </div>
      {/* Status dot — hidden on hover to free the remove affordance (app parity). */}
      <span className="group-hover:hidden self-start mt-0.5">
        <Dot state={dotState(state)} />
      </span>
      <button
        className="hidden group-hover:grid w-5 h-5 place-items-center rounded-md faint hover:bg-[var(--color-hover)] shrink-0 self-start"
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

      {/* main — the direction explained: amber working line + reduced-motion fallback */}
      <main className="flex-1 min-w-0 overflow-y-auto scroll-thin px-8 py-7">
        <div className="max-w-[520px]">
          <h1 className="heading text-[1.06rem] font-semibold tracking-tight">Working-state indicator</h1>
          <p className="dim text-[0.8rem] leading-relaxed mt-1.5">
            The sidebar now says who is <span className="heading font-medium">actively working</span> in words, not just a
            dot: an agent whose backend emitted output in the last 5&nbsp;seconds shows an amber
            <span className="heading font-medium"> working…</span> line under its name. Watch Detoro &amp; Mellow at left.
          </p>

          <div className="mt-6 rounded-lg p-4" style={{ background: "var(--color-sidebar)", border: "1px solid var(--color-border)" }}>
            <div className="label faint pb-1">States</div>
            <div className="divide-y" style={{ borderColor: "var(--color-border-soft)" }}>
              <Legend
                swatch={<LoaderCircle size={12} className="rwork-ico" style={{ color: "var(--color-working)" }} />}
                name="Working"
                desc="Session live and emitting output (≤ 5s). Green dot stays; an amber spinner + “working…” line appears under the name."
              />
              <Legend
                swatch={<Dot state="running" lg />}
                name="Running · quiet"
                desc="Session alive but idle at its prompt. The app’s static green dot, no sub-line."
              />
              <Legend
                swatch={<Dot state="idle" lg />}
                name="Idle"
                desc="No live session. The app’s static gray dot, unchanged."
              />
              <Legend
                swatch={<Dot state="waiting" lg />}
                name="Waiting"
                desc="Awaiting input. Static orange dot, no motion, no sub-line, never confused with amber working."
              />
              <Legend
                swatch={<LoaderCircle size={12} style={{ color: "var(--color-working)" }} />}
                name="Working · reduced motion"
                desc="prefers-reduced-motion: the icon stops spinning and the label reads a static “working…”, still amber and legible."
              />
            </div>
          </div>

          <div className="mt-5 text-[0.72rem] faint leading-relaxed space-y-1.5">
            <p>
              <span className="dim font-medium">Amber ≠ orange.</span> Working amber (#ffb224) is a yellow-gold; the spinner
              and animated text set it apart from the bare, static waiting-orange dot at a glance.
            </p>
            <p>
              <span className="dim font-medium">Expands, not reserved.</span> The line slides the row open only while working
              (grid-rows 0fr→1fr); idle rows stay compact instead of carrying a permanent blank slot.
            </p>
            <p>
              <span className="dim font-medium">Hover parity.</span> The dot still hides on row-hover to reveal the remove
              button; the amber line reads on the left, clear of that affordance.
            </p>
          </div>
        </div>
      </main>
    </div>
  );
}
