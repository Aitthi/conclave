import { useState, type ReactNode } from "react";
import { Search, Plus, SquareTerminal, Users, MessageSquare, Network, Sparkles, History, RotateCcw, Camera, Zap, CornerDownLeft, ChevronDown, ChevronRight } from "lucide-react";

const SKILLS = [
  { n: "Agent Loop", tag: "BUILTIN" },
  { n: "Collaboration", tag: "BUILTIN · ALWAYS" },
  { n: "Leadership", tag: "BUILTIN" },
  { n: "Strategic Compact", tag: "BUILTIN · ALWAYS" },
];
const SNAPS = [
  { tok: "1,975", age: "53m" },
  { tok: "923", age: "9h" },
];

/* The Conclave app shell, rendered quiet/dimmed so the redesigned RIGHT RAIL is the focus.
   Left = workspace switcher + CLI agent list. Center = terminal pane. Right = <children>. */

const agentTabs = ["Detoro", "Mellow", "Dew", "Arta"];
const cliAgents = [
  { n: "Detoro", r: "Principal Engineer", c: "av-indigo", live: true },
  { n: "Mellow", r: "Staff Engineer", c: "av-teal", live: true },
  { n: "Dew", r: "Engineer", c: "av-amber", live: false },
  { n: "Arta", r: "Designer", c: "av-sky", live: true },
];

export default function AppShell({ children }: { children: ReactNode }) {
  const [skillsOpen, setSkillsOpen] = useState(false);
  const [snapsOpen, setSnapsOpen] = useState(false);
  return (
    <div className="h-screen w-full flex flex-col overflow-hidden" style={{ background: "var(--color-app)" }}>
      {/* app toolbar — the viewer already provides the macOS window frame,
          so no traffic-light chrome here */}
      <div className="flex items-center gap-4 h-11 px-4 border-b bd shrink-0" style={{ borderColor: "var(--color-border)" }}>
        <div className="flex items-center gap-2 faint">
          <span className="w-5 h-5 rounded-md grid place-items-center text-[0.6rem] font-bold" style={{ background: "#2f6bff", color: "#fff" }}>c</span>
          <span className="text-[0.82rem] font-semibold" style={{ color: "var(--color-text)" }}>codeup</span>
          <span className="mono text-[0.7rem] faint">~/code/codeup</span>
        </div>
        <div className="flex items-center gap-1 ml-4 text-[0.78rem]">
          {agentTabs.map((a, i) => (
            <span key={a} className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-md"
              style={i === 3 ? { background: "var(--color-raised)", color: "var(--color-text)" } : { color: "var(--color-faint)" }}>
              <span className="w-1.5 h-1.5 rounded-full" style={{ background: "var(--color-live)" }} />{a}
            </span>
          ))}
        </div>
      </div>

      <div className="flex-1 flex min-h-0">
        {/* left rail — workspace switcher */}
        <div className="w-12 shrink-0 border-r flex flex-col items-center py-3 gap-3" style={{ borderColor: "var(--color-border)", background: "var(--color-app)" }}>
          {["#2dd4bf", "#f26d6d", "#8b8cf0", "#f26d6d"].map((c, i) => (
            <span key={i} className="w-8 h-8 rounded-lg opacity-50" style={{ background: c }} />
          ))}
          <span className="w-8 h-8 rounded-lg grid place-items-center faint" style={{ border: "1px dashed var(--color-border)" }}><Plus size={14} /></span>
        </div>

        {/* agents list */}
        <div className="w-52 shrink-0 border-r flex flex-col" style={{ borderColor: "var(--color-border)", background: "var(--color-sidebar)" }}>
          <div className="p-2.5">
            <div className="flex items-center gap-2 px-2.5 py-1.5 rounded-md faint text-[0.78rem]" style={{ background: "var(--color-app)", border: "1px solid var(--color-border)" }}>
              <Search size={13} /> Search agents
            </div>
          </div>
          <div className="px-3 pt-1 pb-1.5 label faint">CLI Agents</div>
          <div className="px-1.5 flex flex-col gap-0.5">
            {cliAgents.map((a, i) => (
              <div key={a.n} className="flex items-center gap-2.5 px-2 py-1.5 rounded-md"
                style={i === 3 ? { background: "var(--color-raised)" } : undefined}>
                <span className={`av av-md ${a.c}`} style={{ opacity: 0.85 }}>{a.n[0]}</span>
                <div className="min-w-0 flex-1">
                  <div className="text-[0.8rem] leading-tight" style={{ color: i === 3 ? "var(--color-text)" : "var(--color-faint)" }}>{a.n}</div>
                  <div className="text-[0.66rem] leading-tight faint">{a.r}</div>
                </div>
                <span className="w-1.5 h-1.5 rounded-full" style={{ background: a.live ? "var(--color-live)" : "#3b424c" }} />
              </div>
            ))}
          </div>
          <div className="mt-auto p-2.5 flex flex-col gap-1.5 faint text-[0.76rem]">
            <div className="flex items-center gap-2"><Users size={13} /> Blackboard</div>
            {/* Memory destination — paired with Blackboard (both are record surfaces),
                Network glyph matches the graph view's own rail icon */}
            <div className="flex items-center gap-2"><Network size={13} /> Memory</div>
            <div className="flex items-center gap-2"><MessageSquare size={13} /> Chat</div>
          </div>
        </div>

        {/* center — terminal + Context now lives here as top/bottom sections */}
        <div className="flex-1 min-w-0 flex flex-col" style={{ background: "var(--color-center)" }}>
          <div className="flex items-center gap-2 px-4 h-9 faint text-[0.74rem] mono border-b" style={{ borderColor: "var(--color-border-soft)" }}>
            <SquareTerminal size={13} /> Arta · Designer · running
          </div>

          {/* TOP context section — Skills popover + Session (slim single line) */}
          <div className="relative px-3 h-9 shrink-0 border-b flex items-center gap-1.5" style={{ borderColor: "var(--color-border-soft)" }}>
            <button className={`ctx-trigger ${skillsOpen ? "is-open" : ""}`} onClick={() => { setSkillsOpen((v) => !v); setSnapsOpen(false); }}>
              <Sparkles size={13} style={{ color: "color-mix(in srgb, var(--color-accent) 75%, var(--color-faint))" }} />
              Skills <span className="cnt">{SKILLS.length}</span>
              <ChevronDown size={13} className="faint" style={{ transform: skillsOpen ? "rotate(180deg)" : "none", transition: "transform .12s" }} />
            </button>
            <span className="w-px h-3.5" style={{ background: "var(--color-border)" }} />
            <button className="ctx-ibtn" title="Resume last handoff"><History size={14} /></button>
            <button className="ctx-ibtn" title="Restart · resume"><RotateCcw size={14} /></button>

            {skillsOpen && (
              <div className="popover" style={{ top: "100%", left: "0.75rem", marginTop: 4 }}>
                <div className="pop-head"><span className="label">Skills</span><span className="label faint">{SKILLS.length}</span></div>
                {SKILLS.map((s) => (
                  <div key={s.n} className="pop-row">
                    <Sparkles size={14} style={{ color: "var(--color-accent)" }} />
                    <span className="heading text-[0.82rem] flex-1">{s.n}</span>
                    <span className="text-[0.62rem] faint mono tracking-wide">{s.tag}</span>
                  </div>
                ))}
              </div>
            )}
          </div>

          {/* terminal body */}
          <div className="flex-1 p-5 mono text-[0.74rem] leading-relaxed faint select-none overflow-hidden">
            <div style={{ color: "var(--color-faint)" }}>Claude Code v2.1.200</div>
            <div className="mt-3">Welcome back Deew!</div>
            <div className="mt-3 opacity-70">Opus 4.8 (1M context) · Claude Max</div>
            <div className="mt-6 flex items-center gap-2"><span style={{ color: "#3a6d55" }}>›</span> /arta:arta &lt;what to design&gt;</div>
            <div className="mt-2 opacity-60">▸ bypass permissions on (shift+tab to cycle)</div>
          </div>

          {/* BOTTOM context section — Snapshots popover (slim single line) */}
          <div className="relative px-3 h-9 shrink-0 border-t flex items-center gap-1.5" style={{ borderColor: "var(--color-border-soft)" }}>
            <button className={`ctx-trigger ${snapsOpen ? "is-open" : ""}`} onClick={() => { setSnapsOpen((v) => !v); setSkillsOpen(false); }}>
              <Camera size={13} className="faint" />
              Snapshots <span className="cnt">{SNAPS.length}</span>
              <ChevronDown size={13} className="faint" style={{ transform: snapsOpen ? "rotate(180deg)" : "none", transition: "transform .12s" }} />
            </button>
            <span className="dim text-[0.72rem] num clamp-1 min-w-0">last handoff ~1,975 tok · 53m</span>
            <span className="w-px h-3.5 shrink-0" style={{ background: "var(--color-border)" }} />
            <button className="ctx-ibtn ctx-ibtn-accent shrink-0" title="Compact now"><Zap size={14} /></button>

            {snapsOpen && (
              <div className="popover" style={{ bottom: "100%", left: "0.75rem", marginBottom: 4 }}>
                <div className="pop-head">
                  <span className="label">Memory · Snapshots</span>
                  <button className="inline-flex items-center gap-1 text-[0.7rem]" style={{ color: "var(--color-accent)" }}><Zap size={12} /> Compact now</button>
                </div>
                {SNAPS.map((s, i) => (
                  <div key={i} className="pop-row">
                    <Camera size={14} className="faint" />
                    <div className="flex-1 min-w-0">
                      <div className="text-[0.82rem] heading leading-tight">handoff</div>
                      <div className="text-[0.68rem] faint num leading-tight">~{s.tok} tok</div>
                    </div>
                    <span className="text-[0.68rem] faint num">{s.age}</span>
                    <ChevronRight size={14} className="faint" />
                  </div>
                ))}
              </div>
            )}
          </div>

          {/* composer — message the agent (center, unchanged) */}
          <div className="px-3 py-2.5 border-t flex items-center gap-2 text-[0.76rem]" style={{ borderColor: "var(--color-border)" }}>
            <span className="chip"><span className="w-1.5 h-1.5 rounded-full" style={{ background: "var(--color-a-indigo)" }} /> Detoro · self</span>
            <span className="faint mono">›</span>
            <span className="faint">Message the agent…  <span className="opacity-60">(Enter to send)</span></span>
            <span className="ml-auto w-7 h-7 grid place-items-center rounded-md faint" style={{ border: "1px solid var(--color-border)" }}><CornerDownLeft size={14} /></span>
          </div>
        </div>

        {/* RIGHT RAIL — the redesign */}
        <aside className="w-[380px] shrink-0 border-l flex flex-col min-h-0"
          style={{ borderColor: "var(--color-border)", background: "var(--color-rail)", boxShadow: "var(--shadow-rail)" }}>
          {children}
        </aside>
      </div>
    </div>
  );
}
