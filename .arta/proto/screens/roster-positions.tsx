import { Search, Terminal, Layers, LoaderCircle } from "lucide-react";
import { agents, avClass } from "../lib/data";
import { positions } from "../lib/positions";
import { LevelTag, TrackIcon, ReportsTo, HumanChip, LevelRung } from "../components/Position";

export const meta = { title: "Roster · Position display" };

/* Surface 1 of the Position System canon: how TRACK + LEVEL + REPORTING reads
   on the existing 266px Roster card without clutter.

   The card keeps its three existing rows (name row, subtitle, working line) and
   threads position into the subtitle it already had:
     • the role subtitle BECOMES the position line: a level ladder plus track
       name (e.g. "Senior · Implementer"). Track is the same word/icon
       vocabulary as the role picker; level is the levels.fyi ascending ladder.
     • REPORTING sits at the right edge of that same line as a quiet up-arrow
       plus supervisor avatar (or the Human chip at the top of the chain). It
       reads "reports to X" on hover, adds one 16px chip, no new row.
   Nothing else on the card moves: status dot, hover-remove, and the amber
   working line all behave exactly as today. */

type State = "working" | "running" | "idle" | "waiting";

const rows: { id: string; state: State }[] = [
  { id: "detoro", state: "working" }, // Lead · Principal · reports to Human
  { id: "tiesto", state: "running" }, // Implementer · Senior · sub-lead
  { id: "dew", state: "working" }, //    Implementer · Mid · reports to Tiësto
  { id: "arta", state: "idle" }, //      Designer · Senior
];

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
  const p = positions[id];
  const isCli = p.track !== "Designer"; // stand-in: most swarm agents are CLI
  const working = state === "working";
  return (
    <div className="group flex items-center gap-2.5 px-2 py-1.5 rounded-lg cursor-pointer transition-colors hover:bg-[var(--color-hover)]">
      <span className={`av av-md ${avClass[a.color]}`}>{a.initials}</span>
      <div className="flex-1 min-w-0 leading-tight">
        <div className="text-[0.78rem] font-semibold heading flex items-center gap-1.5 truncate">
          {a.name}
          {isCli && <Terminal size={12} className="faint shrink-0" />}
        </div>
        {/* position line: level ladder + track ······ reports-to at the edge */}
        <div className="flex items-center gap-1.5 mt-0.5">
          <LevelTag levelId={p.levelId} compact />
          <span className="faint text-[0.6rem]">·</span>
          <span className="inline-flex items-center gap-1 min-w-0">
            <TrackIcon track={p.track} size={11} className="faint shrink-0" />
            <span className="text-[0.66rem] dim truncate">{p.track}</span>
          </span>
          <span className="ml-auto shrink-0 pl-1">
            <ReportsTo supervisorId={p.supervisorId} />
          </span>
        </div>
        <WorkLine on={working} />
      </div>
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

function VariantRow({
  label,
  desc,
  children,
}: {
  label: string;
  desc: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-start gap-3 py-2.5">
      <div className="w-[150px] shrink-0">{children}</div>
      <div className="min-w-0">
        <div className="text-[0.76rem] font-semibold heading">{label}</div>
        <div className="text-[0.7rem] dim leading-snug">{desc}</div>
      </div>
    </div>
  );
}

export default function RosterPositions() {
  return (
    <div className="h-screen w-full flex overflow-hidden" style={{ background: "var(--color-app)" }}>
      {/* left sidebar — the real app Roster, 266px */}
      <aside
        className="w-[266px] shrink-0 flex flex-col border-r"
        style={{ borderColor: "var(--color-border)", background: "var(--color-sidebar)" }}
      >
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

      {/* main — the direction explained */}
      <main className="flex-1 min-w-0 overflow-y-auto scroll-thin px-8 py-7">
        <div className="max-w-[560px]">
          <h1 className="heading text-[1.06rem] font-semibold tracking-tight">Position on the roster card</h1>
          <p className="dim text-[0.8rem] leading-relaxed mt-1.5">
            Each card now says <span className="heading font-medium">where a member sits</span> (its level, its track, and
            who it reports to), folded into the subtitle it already had. No new row: the ladder and track replace the old
            plain role line, and reporting is one quiet chip at the right edge. Watch the sidebar.
          </p>

          {/* anatomy of the position line */}
          <div className="mt-6 rounded-lg p-4" style={{ background: "var(--color-sidebar)", border: "1px solid var(--color-border)" }}>
            <div className="label faint pb-1">The position line</div>
            <div className="divide-y" style={{ borderColor: "var(--color-border-soft)" }}>
              <VariantRow
                label="Level: a rung ladder"
                desc="Ascending bars filled to the member's rung (levels.fyi ladder, our dimension is authority not salary). Taller filled stack = more senior; no colour code to learn."
              >
                <div className="flex items-center gap-4">
                  {["junior", "mid", "senior", "principal"].map((id) => (
                    <span key={id} className="inline-flex flex-col items-center gap-1">
                      <LevelRung rung={{ junior: 1, mid: 2, senior: 3, principal: 4 }[id]!} />
                      <span className="text-[0.58rem] faint">{id}</span>
                    </span>
                  ))}
                </div>
              </VariantRow>
              <VariantRow
                label="Track: the role, its own icon"
                desc="The same icon + word the role picker uses (Lead / Reviewer / Implementer / Designer / Researcher), so track identity is stable across the app."
              >
                <div className="inline-flex items-center gap-1.5">
                  <LevelTag levelId="senior" compact />
                  <span className="faint text-[0.6rem]">·</span>
                  <TrackIcon track="Implementer" size={11} className="faint" />
                  <span className="text-[0.66rem] dim">Implementer</span>
                </div>
              </VariantRow>
              <VariantRow
                label="Reports to a peer"
                desc="A corner-up arrow + the supervisor's avatar at the right edge. Hover reads “Reports to Tiësto”. Escalations from this member route up to that avatar first."
              >
                <ReportsTo supervisorId="tiesto" />
              </VariantRow>
              <VariantRow
                label="Reports to the human"
                desc="The top of the chain, a null supervisor. The Human chip (transparent, accent hairline) marks it, distinct from any agent avatar."
              >
                <ReportsTo supervisorId={null} />
              </VariantRow>
              <VariantRow
                label="Unranked: no level set"
                desc="Backward-compat: existing members with no level render honestly as an empty ladder + “Unranked”, never a fake Junior. Track and reporting still show."
              >
                <div className="inline-flex items-center gap-1.5">
                  <LevelTag levelId={null} compact />
                  <span className="faint text-[0.6rem]">·</span>
                  <TrackIcon track="Researcher" size={11} className="faint" />
                  <span className="text-[0.66rem] dim">Researcher</span>
                </div>
              </VariantRow>
            </div>
          </div>

          <div className="mt-5 text-[0.72rem] faint leading-relaxed space-y-1.5">
            <p>
              <span className="dim font-medium">One line, not two.</span> The position line reuses the subtitle slot the
              role text used to fill, so the card gains meaning without gaining height.
            </p>
            <p>
              <span className="dim font-medium">Reporting is quiet by design.</span> A single 16px chip at the row edge;
              the org chart (surface 3) is where the whole chain is read at once.
            </p>
            <p>
              <span className="dim font-medium">The top of the chain is the human.</span> <HumanChip />. Every member
              either reports to a peer or, ultimately, to the human.
            </p>
          </div>
        </div>
      </main>
    </div>
  );
}
