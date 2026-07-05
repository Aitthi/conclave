import {
  Search, X, Columns3, Network, LoaderCircle, Crown, Filter, Star,
} from "lucide-react";
import { agents, avClass } from "../lib/data";
import { positions, reportsOf, rootMembers } from "../lib/positions";
import { LevelTag, TrackIcon } from "../components/Position";

export const meta = { title: "Org chart · Supervisor tree" };

/* Surface 3 of the Position System canon: the workspace hierarchy as a tree.

   PLACEMENT PROPOSAL (recommended): a Board | Org segment in the Lane board
   header. Rationale: the Lane board already owns "agent work system" topology
   and carries the floating-header + telemetry chrome, so the org tree is one
   more view of the same surface. It adds NO item to the Roster footer menu
   (the standing, human-ruled constraint). The Rail was the other candidate and
   is rejected: at ~272px it is too narrow to read a multi-level hierarchy.

   The tree is VERTICAL and indented (a file-tree shape), not a wide horizontal
   org chart: it scales to a 5-10 agent workspace and fits the pane. The human
   is the crown at the top of every chain; each node shows avatar, name, its
   position line (level ladder + track), sub-lead marker, working state and last
   activity. The indentation edges ARE the escalation routes. */

const WORKING = new Set(["detoro", "dew"]);
const LAST_ACTIVE: Record<string, string> = {
  detoro: "now", mellow: "22m", tiesto: "4m", dew: "now", guetta: "just now", arta: "1h",
};

function NodeRow({ id }: { id: string }) {
  const a = agents[id];
  const p = positions[id];
  const working = WORKING.has(id);
  const reports = reportsOf(id);
  return (
    <div className="lane-card rounded-lg px-2.5 py-2 flex items-center gap-2.5 min-w-0">
      <span className={`av av-md ${avClass[a.color]}`}>{a.initials}</span>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span className="text-[0.82rem] font-semibold heading truncate">{a.name}</span>
          {p.subLeadOf && (
            <span className="inline-flex items-center gap-1 text-[0.58rem] font-semibold px-1.5 py-0.5 rounded-full shrink-0"
              style={{ color: "var(--color-a-sky)", background: "color-mix(in srgb, var(--color-a-sky) 12%, transparent)", border: "1px solid color-mix(in srgb, var(--color-a-sky) 30%, transparent)" }}
              title={`Sub-lead of ${p.subLeadOf} (lead stays tiebreaker)`}>
              <Star size={9} /> Sub-lead · {p.subLeadOf}
            </span>
          )}
          {working && <LoaderCircle size={11} className="rwork-ico shrink-0" style={{ color: "var(--color-working)" }} />}
        </div>
        <div className="flex items-center gap-1.5 mt-0.5">
          <LevelTag levelId={p.levelId} compact />
          <span className="faint text-[0.6rem]">·</span>
          <TrackIcon track={p.track} size={11} className="faint shrink-0" />
          <span className="text-[0.66rem] dim">{p.track}</span>
        </div>
      </div>
      <div className="flex flex-col items-end gap-0.5 shrink-0">
        {reports.length > 0 && (
          <span className="text-[0.6rem] faint num" title={`${reports.length} direct report${reports.length > 1 ? "s" : ""}`}>
            {reports.length} report{reports.length > 1 ? "s" : ""}
          </span>
        )}
        <span className="text-[0.6rem] faint num">{LAST_ACTIVE[id] ?? ""}</span>
      </div>
    </div>
  );
}

/* Flatten the supervisor tree (rooted at the human) into rows carrying their
   own connector guides, so indentation and rails are explicit per depth rather
   than relying on nested padding that reads flat. Each row's `guides` describe
   the ancestor columns (a continuing vertical line, or empty space) and `elbow`
   is this node's own connector (tee = has a following sibling, ell = last). */
type Guide = "line" | "space";
interface Row {
  id: string;
  guides: Guide[];
  elbow: "tee" | "ell";
}

const HUMAN = "__human__";
const kidsOf = (id: string): string[] => (id === HUMAN ? rootMembers() : reportsOf(id));

function buildRows(): Row[] {
  const rows: Row[] = [];
  const rec = (id: string, guides: Guide[]) => {
    const kids = kidsOf(id);
    kids.forEach((k, i) => {
      const last = i === kids.length - 1;
      rows.push({ id: k, guides, elbow: last ? "ell" : "tee" });
      rec(k, [...guides, last ? "space" : "line"]);
    });
  };
  rec(HUMAN, []);
  return rows;
}

const RAIL = "color-mix(in srgb, var(--color-faint) 55%, transparent)";

function GuideCols({ guides, elbow }: { guides: Guide[]; elbow: "tee" | "ell" }) {
  return (
    <>
      {guides.map((g, i) => (
        <span key={i} className="w-6 shrink-0 relative self-stretch" aria-hidden>
          {g === "line" && <span className="absolute top-0 bottom-0 w-px" style={{ left: "50%", background: RAIL }} />}
        </span>
      ))}
      {/* the node's own elbow column */}
      <span className="w-6 shrink-0 relative self-stretch" aria-hidden>
        {/* vertical up to the mid-line (connects to the parent above) */}
        <span className="absolute top-0 w-px" style={{ left: "50%", height: "50%", background: RAIL }} />
        {/* continue down to the next sibling when this node is not the last */}
        {elbow === "tee" && <span className="absolute bottom-0 w-px" style={{ left: "50%", height: "50%", background: RAIL }} />}
        {/* horizontal tick into the node */}
        <span className="absolute h-px" style={{ left: "50%", right: 0, top: "50%", background: RAIL }} />
      </span>
    </>
  );
}

function Tree() {
  const rows = buildRows();
  return (
    <div className="flex flex-col gap-1.5">
      {rows.map((r) => (
        <div key={r.id} className="flex items-stretch">
          <GuideCols guides={r.guides} elbow={r.elbow} />
          <div className="flex-1 min-w-0">
            <NodeRow id={r.id} />
          </div>
        </div>
      ))}
    </div>
  );
}

function Seg({ active, icon, label }: { active: boolean; icon: React.ReactNode; label: string }) {
  return (
    <button
      className={`inline-flex items-center gap-1.5 h-7 px-2.5 rounded-md text-[0.72rem] font-medium transition-colors ${
        active ? "heading" : "dim hover:text-[var(--color-text)]"
      }`}
      style={active ? { background: "var(--color-raised)", boxShadow: "inset 0 0 0 1px var(--color-border)" } : undefined}
    >
      <span className={active ? "text-accent" : "faint"}>{icon}</span>
      {label}
    </button>
  );
}

export default function OrgChart() {
  const memberCount = Object.keys(positions).length;
  return (
    <div className="relative h-screen w-full overflow-hidden" style={{ background: "var(--color-app)" }}>
      {/* floating blurred header — same chrome as the Lane board */}
      <div
        className="absolute top-0 left-0 right-0 h-12 z-20 flex items-center gap-3 px-4 border-b"
        style={{ borderColor: "var(--color-border)", background: "color-mix(in srgb, var(--color-app) 78%, transparent)", backdropFilter: "blur(8px)" }}
      >
        <span className="w-6 h-6 rounded-[7px] grid place-items-center shrink-0" style={{ background: "var(--color-raised)", color: "var(--color-heading)", boxShadow: "inset 0 0 0 1px var(--color-border)" }}>
          <Network size={13} />
        </span>
        <div className="leading-tight">
          <div className="text-[0.84rem] font-semibold tracking-tight heading">Lane board</div>
          <div className="text-[0.64rem] -mt-0.5 faint">codeup · agent work system</div>
        </div>
        {/* Board | Org segmented — the placement proposal, in the header */}
        <div className="ml-1 seg" style={{ height: 32 }}>
          <Seg active={false} icon={<Columns3 size={12} />} label="Board" />
          <Seg active icon={<Network size={12} />} label="Org" />
        </div>
        <div className="ml-auto flex items-center gap-2">
          <div className="flex items-center gap-2 rounded-md px-2.5 h-7" style={{ background: "var(--color-app)", border: "1px solid var(--color-border)" }}>
            <Search size={12} className="faint shrink-0" />
            <span className="text-[0.72rem] faint">Filter</span>
          </div>
          <button className="inline-flex items-center gap-1.5 rounded-md px-2.5 h-7 text-[0.72rem] dim" style={{ background: "var(--color-app)", border: "1px solid var(--color-border)" }}>
            <Filter size={12} /> {memberCount} members
          </button>
          <button className="w-7 h-7 grid place-items-center rounded-md faint ctx-ibtn" title="Close board">
            <X size={15} />
          </button>
        </div>
      </div>

      {/* content */}
      <div className="absolute inset-0 pt-12 overflow-y-auto scroll-thin">
        <div className="max-w-[560px] mx-auto px-6 py-6">
          <p className="text-[0.72rem] faint leading-relaxed mb-4">
            The supervisor chain of the workspace. Every member reports up to the human at the top; the indentation edges
            are the routes escalations, challenges and stall alerts travel.
          </p>

          {/* the tree */}
          <div>
            {/* Human crown — top of every chain */}
            <div className="lane-card rounded-lg px-2.5 py-2 flex items-center gap-2.5" style={{ borderColor: "color-mix(in srgb, var(--color-accent) 30%, var(--color-border))" }}>
              <span className="av av-md av-human"><Crown size={14} /></span>
              <div className="min-w-0 flex-1">
                <div className="text-[0.82rem] font-semibold heading">Human</div>
                <div className="text-[0.66rem] dim">Top of the chain · final tiebreaker</div>
              </div>
              <span className="text-[0.6rem] faint num">{memberCount} report{memberCount > 1 ? "s" : ""} below</span>
            </div>

            {/* supervisor tree beneath the crown */}
            <div className="mt-1.5">
              <Tree />
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
