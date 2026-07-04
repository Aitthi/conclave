import { useState } from "react";
import {
  Sparkles,
  X,
  Compass,
  ShieldCheck,
  Hammer,
  PenTool,
  Microscope,
  Plus,
  Terminal,
  MessageSquare,
  Waypoints,
} from "lucide-react";

export const meta = { title: "Edit agent · Role picker" };

/* Role picker redesign for the real app (src/components/Builder.tsx, ADR 0005).
   Before: a native <select> dropdown plus a wall of raw gray description text,
   the one section that broke the form (Type below already uses a card grid).
   After: a 2-col grid of selectable role cards (icon, name, one-line tagline)
   matching the Type cards, with the selected role's full description and the
   skills it attaches shown in a styled callout beneath. "No role" is a quiet
   toggle in the section header; "Custom..." is a dashed action card that opens
   the existing inline role editor. Click a card to see the callout update. */

type Role = {
  id: string;
  name: string;
  Icon: typeof Compass;
  tagline: string;
  skills: string[];
  desc: string;
};

const ROLES: Role[] = [
  {
    id: "lead",
    name: "Lead",
    Icon: Compass,
    tagline: "Settles & delegates work",
    skills: ["Leadership", "Agent Loop"],
    desc: "Settles decisions before anyone builds and records them where a zero-context implementer can find them. Decomposes work into claimable tasks and delegates rather than implements, rules on escalations, and owns integration: the branch, the merges, the final outcome report.",
  },
  {
    id: "reviewer",
    name: "Reviewer",
    Icon: ShieldCheck,
    tagline: "Grills work with evidence",
    skills: ["Implementer"],
    desc: "Reads a peer's diff or plan adversarially and reports findings with evidence (the file, the line, the recorded decision it conflicts with) plus a proposed resolution. Attacks artifacts, never agents, and verifies claims rather than pulling rank; its verdict is a recommendation the lead rules on.",
  },
  {
    id: "implementer",
    name: "Implementer",
    Icon: Hammer,
    tagline: "Builds the recorded plan",
    skills: ["Implementer"],
    desc: "Turns a lead's recorded plan into working, verified software: claiming a task before touching it, following the recorded decisions, and acting as the tripwire that catches what the plan got wrong. Escalates design and spec conflicts with evidence, and never claims done on work it hasn't run and watched pass.",
  },
  {
    id: "designer",
    name: "Designer",
    Icon: PenTool,
    tagline: "Designs on the canvas",
    skills: ["Arta Designer"],
    desc: "Designs apps on the Arta live canvas the human watches in real time: brainstorming direction first, then authoring real React screens, data models, flows, and plans that implementers build from. Owns the design record and iterates it from the human's feedback.",
  },
  {
    id: "researcher",
    name: "Researcher",
    Icon: Microscope,
    tagline: "Investigates open questions",
    skills: [],
    desc: "Investigates open questions the team can't answer from the code alone: comparing options, tracing prior art, and gathering the evidence a decision needs, then reports findings as sourced, verifiable claims. Separates what it confirmed from what it inferred and hands the lead a conclusion they can act on.",
  },
];

function RoleCard({
  role,
  active,
  onClick,
}: {
  role: Role;
  active: boolean;
  onClick: () => void;
}) {
  const { Icon } = role;
  return (
    <button
      onClick={onClick}
      aria-pressed={active}
      className={`relative rounded-xl p-2.5 text-left transition-colors ring-1 ${
        active ? "ring-accent/50 bg-accent/[0.07]" : "ring-border bg-raised hover:bg-hover"
      }`}
    >
      <Icon size={17} className={`mb-1.5 ${active ? "text-accent" : "dim"}`} strokeWidth={2} />
      <div className="text-[12.5px] font-semibold heading leading-tight">{role.name}</div>
      <div className="text-[11px] faint leading-snug mt-0.5">{role.tagline}</div>
    </button>
  );
}

export default function RolePicker() {
  // Defaults to Reviewer to mirror the screenshot the redesign started from.
  const [selId, setSelId] = useState<string | null>("reviewer");
  const [customOpen, setCustomOpen] = useState(false);

  const sel = ROLES.find((r) => r.id === selId) ?? null;

  return (
    <div
      className="min-h-screen w-full py-10 px-6 overflow-y-auto scroll-thin"
      style={{ background: "var(--color-app)" }}
    >
      <div style={{ maxWidth: 420, width: "100%", margin: "0 auto" }}>
        {/* caption: what changed */}
        <div className="mb-5">
          <h1 className="heading text-[1.02rem] font-semibold tracking-tight">
            Role picker, cards not a dropdown
          </h1>
          <p className="dim text-[0.8rem] leading-relaxed mt-1">
            The <span className="heading font-medium">Role</span> section now matches the{" "}
            <span className="heading font-medium">Type</span> cards below it: pick a role by card,
            read its job and attached skills in one callout. Click a card and watch it update.
          </p>
        </div>

        {/* the Edit-agent modal */}
        <div
          className="rounded-2xl overflow-hidden"
          style={{
            background: "var(--color-center)",
            border: "1px solid var(--color-border)",
            boxShadow: "var(--shadow-pop)",
          }}
        >
          {/* header */}
          <div
            className="flex items-center gap-2.5 px-4 h-12 border-b"
            style={{ borderColor: "var(--color-border)" }}
          >
            <Sparkles size={15} className="text-accent" />
            <span className="heading text-[0.9rem] font-semibold tracking-tight">Edit agent</span>
            <span
              className="text-[0.62rem] font-medium faint px-1.5 py-0.5 rounded-md"
              style={{ background: "var(--color-app)", border: "1px solid var(--color-border)" }}
            >
              update definition
            </span>
            <button className="ml-auto ctx-ibtn" aria-label="Close">
              <X size={15} />
            </button>
          </div>

          <div className="p-4 space-y-5">
            {/* Identity */}
            <section>
              <div className="label faint mb-2">Identity</div>
              <div className="flex items-center gap-3">
                <span className="av av-lg av-indigo">M</span>
                <div className="min-w-0">
                  <div className="heading text-[0.9rem] font-semibold tracking-tight">Mellow</div>
                  <div className="text-[0.72rem] dim truncate">{sel?.name ?? "No role"}</div>
                </div>
              </div>
            </section>

            {/* Role: the redesigned section */}
            <section>
              <div className="flex items-center justify-between mb-2">
                <span className="label faint">Role</span>
                <button
                  onClick={() => {
                    setSelId(null);
                    setCustomOpen(false);
                  }}
                  className={`text-[11px] font-medium transition-colors ${
                    selId === null && !customOpen ? "text-accent" : "faint hover:text-[var(--color-text)]"
                  }`}
                >
                  No role
                </button>
              </div>

              <div className="grid grid-cols-2 gap-2">
                {ROLES.map((r) => (
                  <RoleCard
                    key={r.id}
                    role={r}
                    active={selId === r.id && !customOpen}
                    onClick={() => {
                      setSelId(r.id);
                      setCustomOpen(false);
                    }}
                  />
                ))}

                {/* Custom: action card (opens the inline editor in the real app) */}
                <button
                  onClick={() => {
                    setCustomOpen(true);
                    setSelId(null);
                  }}
                  aria-pressed={customOpen}
                  className={`relative rounded-xl p-2.5 text-left transition-colors border border-dashed ${
                    customOpen
                      ? "border-accent/60 bg-accent/[0.07]"
                      : "border-[var(--color-border)] hover:bg-hover"
                  }`}
                >
                  <Plus size={17} className={`mb-1.5 ${customOpen ? "text-accent" : "dim"}`} />
                  <div className="text-[12.5px] font-semibold heading leading-tight">Custom...</div>
                  <div className="text-[11px] faint leading-snug mt-0.5">Define your own role</div>
                </button>
              </div>

              {/* Callout: selected role's full description and attached skills */}
              {sel && (
                <div
                  className="mt-2.5 rounded-xl p-3"
                  style={{ background: "var(--color-app)", border: "1px solid var(--color-border)" }}
                >
                  <div className="flex items-center gap-2 mb-1.5">
                    <sel.Icon size={14} className="text-accent shrink-0" />
                    <span className="heading text-[0.8rem] font-semibold">{sel.name}</span>
                  </div>
                  <p className="text-[11.5px] dim leading-relaxed">{sel.desc}</p>
                  <div className="mt-2.5 flex flex-wrap items-center gap-1.5">
                    <span className="label faint">Attaches</span>
                    {sel.skills.length > 0 ? (
                      sel.skills.map((s) => (
                        <span key={s} className="pill pill-accent">
                          {s}
                        </span>
                      ))
                    ) : (
                      <span className="text-[0.7rem] faint italic">mandatory skills only</span>
                    )}
                  </div>
                  <div className="mt-1.5 text-[0.66rem] faint leading-snug">
                    + Collaboration, Strategic Compact
                    <span className="opacity-70"> · always on</span>
                  </div>
                </div>
              )}

              {/* No-role empty note */}
              {!sel && !customOpen && (
                <div
                  className="mt-2.5 rounded-xl p-3 text-[11.5px] faint leading-relaxed"
                  style={{ background: "var(--color-app)", border: "1px dashed var(--color-border)" }}
                >
                  No role: the agent runs with only the mandatory{" "}
                  <span className="dim">Collaboration</span> and{" "}
                  <span className="dim">Strategic Compact</span> skills, and no job description in its
                  preamble.
                </div>
              )}

              {/* Custom editor stand-in */}
              {customOpen && (
                <div
                  className="mt-2.5 rounded-xl p-3 space-y-2"
                  style={{ background: "var(--color-app)", border: "1px solid var(--color-border)" }}
                >
                  <input
                    placeholder="Role name"
                    className="w-full text-[12.5px] font-semibold bg-transparent outline-none border-b pb-1 heading placeholder:text-[var(--color-faint)]"
                    style={{ borderColor: "var(--color-border)" }}
                  />
                  <textarea
                    rows={3}
                    placeholder="One-paragraph job description (baked into the agent's preamble)"
                    className="w-full text-[12px] dim bg-transparent outline-none rounded-lg px-2 py-1.5 resize-none placeholder:text-[var(--color-faint)]"
                    style={{ border: "1px solid var(--color-border)" }}
                  />
                  <div className="flex items-center justify-end gap-2 pt-0.5">
                    <button
                      onClick={() => setCustomOpen(false)}
                      className="text-[12px] font-medium dim px-3 py-1 rounded-lg hover:bg-[var(--color-hover)]"
                    >
                      Cancel
                    </button>
                    <button
                      className="text-[12px] font-semibold px-3 py-1 rounded-lg"
                      style={{ background: "var(--color-accent)", color: "var(--color-accent-ink)" }}
                    >
                      Create role
                    </button>
                  </div>
                </div>
              )}
            </section>

            {/* Type: unchanged, shown for design-consistency proof */}
            <section>
              <div className="label faint mb-2">Type</div>
              <div className="grid grid-cols-3 gap-2">
                {(
                  [
                    { label: "CLI agent", Icon: Terminal, soon: false },
                    { label: "Chat agent", Icon: MessageSquare, soon: true },
                    { label: "Orchestrator", Icon: Waypoints, soon: true },
                  ] as { label: string; Icon: typeof Terminal; soon: boolean }[]
                ).map(({ label, Icon, soon }) => {
                  const active = label === "CLI agent";
                  return (
                    <div
                      key={label}
                      className={`relative rounded-xl p-2 ring-1 ${
                        soon
                          ? "ring-border opacity-50"
                          : active
                            ? "ring-accent/50 bg-accent/[0.07]"
                            : "ring-border bg-raised"
                      }`}
                      style={soon ? { background: "var(--color-app)" } : undefined}
                    >
                      {soon && (
                        <span
                          className="absolute top-1.5 right-1.5 text-[8px] font-bold tracking-wide faint px-1 py-px rounded-full uppercase"
                          style={{ background: "var(--color-hover)" }}
                        >
                          Soon
                        </span>
                      )}
                      <Icon size={15} className={`mb-1 ${active ? "text-accent" : "faint"}`} />
                      <div className={`text-[11.5px] font-semibold ${active ? "heading" : "dim"}`}>
                        {label}
                      </div>
                    </div>
                  );
                })}
              </div>

              {/* CLI kind segmented (static) */}
              <div className="mt-2 flex gap-1 rounded-xl p-1" style={{ background: "var(--color-app)" }}>
                {[
                  { label: "Claude Code", on: true, soon: false },
                  { label: "Codex", on: false, soon: false },
                  { label: "Custom", on: false, soon: true },
                ].map(({ label, on, soon }) => (
                  <div
                    key={label}
                    className={`flex-1 flex items-center justify-center gap-1 text-[12px] py-1.5 rounded-lg ${
                      on ? "heading font-semibold" : soon ? "faint" : "dim"
                    }`}
                    style={
                      on
                        ? { background: "var(--color-center)", boxShadow: "inset 0 0 0 1px var(--color-border)" }
                        : undefined
                    }
                  >
                    {label}
                    {soon && (
                      <span
                        className="text-[8px] font-bold tracking-wide faint px-1 py-px rounded uppercase"
                        style={{ background: "var(--color-hover)" }}
                      >
                        Soon
                      </span>
                    )}
                  </div>
                ))}
              </div>
            </section>
          </div>

          {/* footer */}
          <div
            className="flex items-center justify-end gap-2 px-4 h-14 border-t"
            style={{ borderColor: "var(--color-border)", background: "var(--color-sidebar)" }}
          >
            <button className="text-[12.5px] font-medium dim px-3 py-1.5 rounded-lg hover:bg-[var(--color-hover)]">
              Cancel
            </button>
            <button
              className="flex items-center gap-1.5 text-[12.5px] font-semibold px-3.5 py-1.5 rounded-lg"
              style={{ background: "var(--color-accent)", color: "var(--color-accent-ink)" }}
            >
              <Sparkles size={13} />
              Save changes
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
