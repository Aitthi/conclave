import { useEffect, useLayoutEffect, useMemo, useRef, useState, type PointerEvent as RPE, type WheelEvent as RWheel, type ReactNode } from "react";
import {
  Layers, Plus, Search, Network, Maximize2, SlidersHorizontal, ChevronDown,
  X, Sparkles, Link2, CornerDownRight,
} from "lucide-react";
import { memoryNodes, memoryLinks, degreeOf, authorName, type GraphColor } from "../lib/memoryGraph";

export const meta = { title: "Memory · Knowledge graph" };

/* Memory view — an Obsidian-style Graph View over the workspace's durable
   memories. Each node is a memory (a `conclave memory` fact); each edge is a
   link between two — an explicit [[wikilink]] in the body or a store-inferred
   "related" tie. Node radius scales with degree; colour = the agent who wrote
   the memory. Hover highlights a fact and its neighbours (the rest dims), and
   the floating panel carries the real Obsidian affordances: search filter,
   colour groups, and live force sliders wired straight into the simulation.

   No graph library: react-flow renders boxed nodes-with-handles (wrong look)
   and d3-force isn't installed, so the physics is a small hand-rolled
   velocity-integrated force sim (repulsion + link springs + centering) drawn
   to SVG. Zero deps, pixel-matched to the app's dark tokens. */

// GraphColor → the theme's CSS variable. Agents use their identity colour;
// shared-protocol memories use the dedicated violet.
const COLOR: Record<GraphColor, string> = {
  teal: "var(--color-a-teal)", red: "var(--color-a-red)", indigo: "var(--color-a-indigo)",
  amber: "var(--color-a-amber)", sky: "var(--color-a-sky)", violet: "var(--color-a-violet)",
  hash: "var(--color-accent)", human: "var(--color-heading)",
};

interface Sim { id: string; x: number; y: number; vx: number; vy: number; }
interface Params { center: number; repel: number; link: number; dist: number; damping: number; }

// deterministic PRNG so the initial scatter (and screenshots) are stable
function seeded(seed: number) {
  let s = seed >>> 0;
  return () => { s = (s * 1664525 + 1013904223) >>> 0; return s / 4294967296; };
}

// One integration step of the hand-rolled force sim (repulsion + link springs +
// centering). Mutates `ns` in place. Module-scope + pure so the same code both
// pre-warms the layout synchronously and drives the live rAF loop.
function tick(ns: Sim[], p: Params, alpha: number, held: string | null) {
  const byId = new Map(ns.map((n) => [n.id, n]));
  for (const n of ns) { n.vx += -n.x * p.center * alpha; n.vy += -n.y * p.center * alpha; }
  for (let i = 0; i < ns.length; i++) {
    for (let j = i + 1; j < ns.length; j++) {
      const a = ns[i], b = ns[j];
      let dx = b.x - a.x, dy = b.y - a.y, d2 = dx * dx + dy * dy;
      if (d2 < 1) { d2 = 1; dx = (i - j) || 0.5; }
      const d = Math.sqrt(d2), f = (p.repel * alpha) / d2, ux = dx / d, uy = dy / d;
      a.vx -= ux * f; a.vy -= uy * f; b.vx += ux * f; b.vy += uy * f;
    }
  }
  for (const l of memoryLinks) {
    const a = byId.get(l.a)!, b = byId.get(l.b)!;
    const dx = b.x - a.x, dy = b.y - a.y, d = Math.sqrt(dx * dx + dy * dy) || 1;
    const f = ((d - p.dist) * p.link * alpha) / d;
    a.vx += dx * f; a.vy += dy * f; b.vx -= dx * f; b.vy -= dy * f;
  }
  for (const n of ns) {
    if (n.id === held) { n.vx = 0; n.vy = 0; continue; }
    n.vx *= p.damping; n.vy *= p.damping; n.x += n.vx; n.y += n.vy;
  }
}

export default function MemoryGraph() {
  const degree = useMemo(degreeOf, []);
  const nodeById = useMemo(() => Object.fromEntries(memoryNodes.map((n) => [n.id, n])), []);
  const adjacency = useMemo(() => {
    const m: Record<string, Set<string>> = {};
    for (const n of memoryNodes) m[n.id] = new Set();
    for (const l of memoryLinks) { m[l.a].add(l.b); m[l.b].add(l.a); }
    return m;
  }, []);
  const radius = (id: string) => 5 + (degree[id] ?? 0) * 1.7;

  // ── simulation state (mutable, driven by rAF) ──
  const nodesRef = useRef<Sim[]>([]);
  if (nodesRef.current.length === 0) {
    const rnd = seeded(42);
    nodesRef.current = memoryNodes.map((n) => {
      const a = rnd() * Math.PI * 2;
      const r = 90 + rnd() * 150;
      return { id: n.id, x: Math.cos(a) * r, y: Math.sin(a) * r, vx: 0, vy: 0 };
    });
  }
  const alphaRef = useRef(0.9);
  const [, setFrame] = useState(0);

  const [forces, setForces] = useState({ center: 0.025, repel: 2500, link: 0.045, dist: 118 });
  const paramsRef = useRef({ ...forces, damping: 0.82 });
  useEffect(() => {
    paramsRef.current = { ...forces, damping: 0.82 };
    alphaRef.current = Math.max(alphaRef.current, 0.5); // reheat on slider change
  }, [forces]);

  // ── view transform (pan / zoom) ──
  const wrapRef = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ k: 1, tx: 0, ty: 0 });
  const viewInit = useRef(false);
  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const set = () => { if (!viewInit.current) { const r = el.getBoundingClientRect(); setView((v) => ({ ...v, tx: r.width / 2, ty: r.height / 2 })); viewInit.current = true; } };
    set();
    const ro = new ResizeObserver(set);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // ── interaction state ──
  const [hover, setHover] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const dragRef = useRef<{ id: string | null; panX: number; panY: number; sx: number; sy: number; moved: boolean } | null>(null);

  // ── physics loop ──
  useEffect(() => {
    let raf = 0;
    const step = () => {
      const alpha = alphaRef.current;
      tick(nodesRef.current, paramsRef.current, alpha, dragRef.current?.id ?? null);
      alphaRef.current = Math.max(alpha * 0.986, 0.015);
      setFrame((f) => (f + 1) % 1_000_000);
      raf = requestAnimationFrame(step);
    };
    raf = requestAnimationFrame(step);
    return () => cancelAnimationFrame(raf);
  }, []);

  // ── pointer: node drag + background pan ──
  const worldAt = (clientX: number, clientY: number) => {
    const r = wrapRef.current!.getBoundingClientRect();
    return { x: (clientX - r.left - view.tx) / view.k, y: (clientY - r.top - view.ty) / view.k };
  };
  const onNodeDown = (id: string) => (e: RPE) => {
    e.stopPropagation();
    (e.target as Element).setPointerCapture?.(e.pointerId);
    dragRef.current = { id, panX: 0, panY: 0, sx: e.clientX, sy: e.clientY, moved: false };
  };
  const onBgDown = (e: RPE) => {
    dragRef.current = { id: null, panX: view.tx, panY: view.ty, sx: e.clientX, sy: e.clientY, moved: false };
  };
  const onMove = (e: RPE) => {
    const d = dragRef.current;
    if (!d) return;
    if (!d.moved && Math.abs(e.clientX - d.sx) + Math.abs(e.clientY - d.sy) > 3) d.moved = true;
    if (d.id) {
      const w = worldAt(e.clientX, e.clientY);
      const n = nodesRef.current.find((s) => s.id === d.id);
      if (n) { n.x = w.x; n.y = w.y; n.vx = 0; n.vy = 0; }
      alphaRef.current = Math.max(alphaRef.current, 0.4);
    } else {
      setView((v) => ({ ...v, tx: d.panX + (e.clientX - d.sx), ty: d.panY + (e.clientY - d.sy) }));
    }
  };
  const onUp = () => {
    const d = dragRef.current;
    if (d && d.id && !d.moved) setSelected((s) => (s === d.id ? null : d.id));
    if (d && !d.id && !d.moved) setSelected(null);
    dragRef.current = null;
  };
  const onWheel = (e: RWheel) => {
    const r = wrapRef.current!.getBoundingClientRect();
    const mx = e.clientX - r.left, my = e.clientY - r.top;
    setView((v) => {
      const k = Math.min(3, Math.max(0.35, v.k * (e.deltaY < 0 ? 1.12 : 1 / 1.12)));
      const wx = (mx - v.tx) / v.k, wy = (my - v.ty) / v.k;
      return { k, tx: mx - wx * k, ty: my - wy * k };
    });
  };
  const fit = () => {
    if (!wrapRef.current) return;
    const ns = nodesRef.current;
    const minX = Math.min(...ns.map((n) => n.x)), maxX = Math.max(...ns.map((n) => n.x));
    const minY = Math.min(...ns.map((n) => n.y)), maxY = Math.max(...ns.map((n) => n.y));
    const cx = (minX + maxX) / 2, cy = (minY + maxY) / 2;
    const bw = Math.max(maxX - minX, 1), bh = Math.max(maxY - minY, 1);
    const r = wrapRef.current.getBoundingClientRect();
    // frame into the canvas RIGHT of the floating panel, with breathing room
    const padL = 288, padR = 56, padY = 96;
    const availW = Math.max(r.width - padL - padR, 200), availH = Math.max(r.height - padY * 2, 200);
    const k = Math.min(1.5, Math.max(0.4, Math.min(availW / bw, availH / bh)));
    viewInit.current = true; // claim the view so the center-init effect won't clobber it
    setView({ k, tx: padL + availW / 2 - cx * k, ty: padY + availH / 2 - cy * k });
  };
  // Pre-warm the sim synchronously before first paint, then frame it. Makes the
  // very first frame already spread + centered — no dependence on rAF timing.
  const fitRef = useRef(fit);
  fitRef.current = fit;
  useLayoutEffect(() => {
    const p = paramsRef.current;
    let a = 0.9;
    for (let i = 0; i < 240; i++) { tick(nodesRef.current, p, a, null); a *= 0.99; }
    alphaRef.current = 0.05; // settled but gently alive for interaction
    fitRef.current();
  }, []);

  // ── derived highlight sets ──
  const focus = hover ?? selected;
  const q = query.trim().toLowerCase();
  const matches = (id: string) => {
    if (!q) return true;
    const n = nodeById[id];
    return (n.label + " " + n.body).toLowerCase().includes(q);
  };
  const isActive = (id: string) => {
    if (focus) return id === focus || adjacency[focus].has(id);
    return true;
  };
  const nodeOpacity = (id: string) => {
    const dimByFocus = focus && !isActive(id);
    const dimByQuery = q && !matches(id);
    return dimByFocus || dimByQuery ? 0.16 : 1;
  };
  const edgeActive = (a: string, b: string) => !focus || a === focus || b === focus;

  const pos = (id: string) => nodesRef.current.find((n) => n.id === id)!;
  const sel = selected ? nodeById[selected] : null;

  // author counts for the Groups legend
  const groups = useMemo(() => {
    const counts: Record<string, number> = {};
    for (const n of memoryNodes) counts[n.author] = (counts[n.author] ?? 0) + 1;
    return Object.entries(counts).map(([author, count]) => ({ author, count, color: memoryNodes.find((n) => n.author === author)!.color }));
  }, []);

  return (
    <div className="h-screen w-full flex overflow-hidden" style={{ background: "var(--color-app)", color: "var(--color-text)" }}>
      {/* workspace icon rail — anchors the graph inside the real app chrome */}
      <div className="w-13 shrink-0 border-r flex flex-col items-center py-3 gap-3" style={{ width: 52, borderColor: "var(--color-border)", background: "var(--color-app)" }}>
        <span className="w-8 h-8 rounded-lg grid place-items-center text-[0.7rem] font-bold" style={{ background: "#2f6bff", color: "#fff" }}>c</span>
        {["#0fa3a3", "#ff7a45", "#5e5ce6"].map((c, i) => (
          <span key={i} className="w-8 h-8 rounded-lg opacity-45" style={{ background: c }} />
        ))}
        <span className="w-8 h-8 rounded-lg grid place-items-center faint" style={{ border: "1px dashed var(--color-border)" }}><Plus size={14} /></span>
        <span className="mt-auto w-8 h-8 rounded-lg grid place-items-center" style={{ background: "color-mix(in srgb, var(--color-accent) 16%, transparent)", color: "var(--color-accent)" }}><Network size={16} /></span>
      </div>

      {/* graph pane */}
      <div ref={wrapRef} className="flex-1 min-w-0 relative overflow-hidden" style={{ background: "radial-gradient(120% 100% at 50% 0%, #191a1d 0%, var(--color-app) 62%)" }}>
        {/* header strip */}
        <div className="absolute top-0 left-0 right-0 h-12 z-20 flex items-center gap-3 px-4 border-b"
          style={{ borderColor: "var(--color-border-soft)", background: "color-mix(in srgb, var(--color-app) 78%, transparent)", backdropFilter: "blur(8px)" }}>
          <span className="w-6 h-6 rounded-[7px] grid place-items-center" style={{ background: "var(--color-raised)", color: "var(--color-heading)" }}><Layers size={13} /></span>
          <div className="leading-tight">
            <div className="heading text-[0.84rem] font-semibold tracking-tight">Memory</div>
            <div className="faint text-[0.64rem] -mt-0.5">codeup · knowledge graph</div>
          </div>
          <span className="pill ml-1"><Sparkles size={11} style={{ color: "var(--color-accent)" }} />{memoryNodes.length} memories</span>
          <span className="pill"><Link2 size={11} />{memoryLinks.length} links</span>
          <div className="ml-auto flex items-center gap-1.5">
            <span className="num faint text-[0.64rem]">{Math.round(view.k * 100)}%</span>
            <button onClick={fit} className="ctx-ibtn" title="Fit graph to view"><Maximize2 size={14} /></button>
          </div>
        </div>

        {/* the canvas */}
        <svg
          className="absolute inset-0 w-full h-full touch-none"
          style={{ cursor: dragRef.current?.id ? "grabbing" : "grab" }}
          onPointerDown={onBgDown}
          onPointerMove={onMove}
          onPointerUp={onUp}
          onWheel={onWheel}
        >
          <g transform={`translate(${view.tx} ${view.ty}) scale(${view.k})`}>
            {/* edges */}
            {memoryLinks.map((l, i) => {
              const a = pos(l.a), b = pos(l.b);
              const on = edgeActive(l.a, l.b) && (!q || matches(l.a) || matches(l.b));
              const touchesFocus = focus && (l.a === focus || l.b === focus);
              return (
                <line key={i} x1={a.x} y1={a.y} x2={b.x} y2={b.y}
                  stroke={touchesFocus ? "var(--color-accent)" : "#ffffff"}
                  strokeOpacity={touchesFocus ? 0.55 : on ? 0.12 : 0.03}
                  strokeWidth={(touchesFocus ? 1.6 : 1) / view.k}
                  strokeDasharray={l.rel === "related" ? `${4 / view.k} ${4 / view.k}` : undefined}
                />
              );
            })}
            {/* nodes */}
            {memoryNodes.map((n) => {
              const p = pos(n.id);
              const r = radius(n.id) * (focus === n.id ? 1.28 : 1);
              const op = nodeOpacity(n.id);
              const isFocus = focus === n.id;
              const isMatch = q ? matches(n.id) : false;
              const showLabel = op > 0.5 && (view.k > 0.95 || isFocus || (focus != null && adjacency[focus!].has(n.id)));
              return (
                <g key={n.id} style={{ opacity: op, transition: "opacity .18s ease" }}
                  onPointerDown={onNodeDown(n.id)}
                  onPointerEnter={() => setHover(n.id)}
                  onPointerLeave={() => setHover((h) => (h === n.id ? null : h))}>
                  {(isFocus || isMatch) && (
                    <circle cx={p.x} cy={p.y} r={r + 5 / view.k} fill="none"
                      stroke={isMatch && !isFocus ? "var(--color-accent)" : COLOR[n.color]}
                      strokeOpacity={0.5} strokeWidth={1.5 / view.k} />
                  )}
                  <circle cx={p.x} cy={p.y} r={r} fill={COLOR[n.color]}
                    stroke="var(--color-app)" strokeWidth={1.5 / view.k}
                    style={{ cursor: "pointer", filter: isFocus ? "brightness(1.15)" : undefined }} />
                  {showLabel && (
                    <text x={p.x} y={p.y + r + 11 / view.k} textAnchor="middle"
                      fontSize={9 / view.k}
                      fill={isFocus ? "var(--color-heading)" : "var(--color-dim)"}
                      style={{ pointerEvents: "none", fontWeight: isFocus ? 600 : 500,
                        paintOrder: "stroke", stroke: "var(--color-app)", strokeWidth: 3 / view.k, strokeLinejoin: "round" }}>
                      {n.label.length > 26 ? n.label.slice(0, 25) + "…" : n.label}
                    </text>
                  )}
                </g>
              );
            })}
          </g>
        </svg>

        {/* ── Obsidian-style control panel ── */}
        <ControlPanel
          query={query} setQuery={setQuery}
          forces={forces} setForces={setForces}
          groups={groups}
        />

        {/* ── detail card ── */}
        <div className="absolute top-16 right-4 z-20"
          style={{ width: 288, opacity: sel ? 1 : 0, transform: sel ? "translateX(0)" : "translateX(12px)", pointerEvents: sel ? "auto" : "none", transition: "opacity .2s ease, transform .2s ease" }}>
          {sel && (
            <div className="rounded-xl overflow-hidden" style={{ background: "var(--color-raised)", border: "1px solid var(--color-border)", boxShadow: "var(--shadow-pop)" }}>
              <div className="flex items-start gap-2.5 p-3.5 pb-3" style={{ borderBottom: "1px solid var(--color-border-soft)" }}>
                <span className="w-2.5 h-2.5 rounded-full mt-1.5 shrink-0" style={{ background: COLOR[sel.color], boxShadow: `0 0 0 3px color-mix(in srgb, ${COLOR[sel.color]} 22%, transparent)` }} />
                <div className="min-w-0 flex-1">
                  <div className="heading text-[0.86rem] font-semibold leading-snug">{sel.label}</div>
                  <div className="flex items-center gap-1.5 mt-1">
                    <span className="pill">{sel.kind}</span>
                    <span className="faint text-[0.66rem]">{authorName[sel.author]} · {sel.age}</span>
                  </div>
                </div>
                <button onClick={() => setSelected(null)} className="ctx-ibtn shrink-0"><X size={14} /></button>
              </div>
              <div className="px-3.5 py-3 text-[0.78rem] leading-relaxed" style={{ color: "var(--color-text)" }}>{sel.body}</div>
              <div className="px-3.5 pb-3.5">
                <div className="label faint mb-1.5">Linked · {adjacency[sel.id].size}</div>
                <div className="flex flex-col gap-0.5">
                  {[...adjacency[sel.id]].map((id) => {
                    const ln = nodeById[id];
                    return (
                      <button key={id} onClick={() => setSelected(id)}
                        onPointerEnter={() => setHover(id)} onPointerLeave={() => setHover(null)}
                        className="flex items-center gap-2 px-2 py-1.5 rounded-md text-left transition-colors hover:bg-[var(--color-hover)]">
                        <CornerDownRight size={12} className="faint shrink-0" />
                        <span className="w-2 h-2 rounded-full shrink-0" style={{ background: COLOR[ln.color] }} />
                        <span className="text-[0.74rem] truncate" style={{ color: "var(--color-text)" }}>{ln.label}</span>
                      </button>
                    );
                  })}
                </div>
              </div>
            </div>
          )}
        </div>

        {/* hint */}
        <div className="absolute bottom-3 right-4 z-10 faint text-[0.64rem] num select-none">scroll to zoom · drag to pan · drag a node to pull</div>
      </div>
    </div>
  );
}

/* ── the floating settings panel (Filters / Groups / Forces) ──
   Section + Slider live at MODULE scope on purpose: defining them inside
   ControlPanel would give them a fresh identity every render, remounting the
   search <input> and dropping focus on each keystroke. */
type OpenState = { filters: boolean; groups: boolean; forces: boolean };

function Section({ id, label, open, setOpen, children }: {
  id: keyof OpenState; label: string; open: OpenState;
  setOpen: (fn: (o: OpenState) => OpenState) => void; children: ReactNode;
}) {
  return (
    <div className="border-t first:border-t-0" style={{ borderColor: "var(--color-border-soft)" }}>
      <button onClick={() => setOpen((o) => ({ ...o, [id]: !o[id] }))} className="w-full flex items-center gap-1.5 px-3 py-2 label faint hover:text-[var(--color-text)] transition-colors">
        <ChevronDown size={12} style={{ transform: open[id] ? "none" : "rotate(-90deg)", transition: "transform .15s" }} />
        {label}
      </button>
      {open[id] && <div className="px-3 pb-3">{children}</div>}
    </div>
  );
}

function Slider({ label, value, min, max, step, onChange }: {
  label: string; value: number; min: number; max: number; step: number; onChange: (v: number) => void;
}) {
  return (
    <label className="block mb-2.5 last:mb-0">
      <span className="flex items-center justify-between text-[0.72rem] mb-1">
        <span style={{ color: "var(--color-text)" }}>{label}</span>
        <span className="num faint text-[0.64rem]">{value}</span>
      </span>
      <input type="range" min={min} max={max} step={step} value={value} onChange={(e) => onChange(Number(e.target.value))} className="gr-range" />
    </label>
  );
}

function ControlPanel({
  query, setQuery, forces, setForces, groups,
}: {
  query: string; setQuery: (v: string) => void;
  forces: { center: number; repel: number; link: number; dist: number };
  setForces: (f: { center: number; repel: number; link: number; dist: number }) => void;
  groups: { author: string; count: number; color: GraphColor }[];
}) {
  const [open, setOpen] = useState<OpenState>({ filters: true, groups: true, forces: true });
  return (
    <div className="absolute top-16 left-4 z-20" style={{ width: 236 }}>
      <div className="rounded-xl overflow-hidden" style={{ background: "color-mix(in srgb, var(--color-raised) 92%, transparent)", border: "1px solid var(--color-border)", boxShadow: "var(--shadow-pop)", backdropFilter: "blur(10px)" }}>
        <div className="flex items-center gap-2 px-3 h-10" style={{ borderBottom: "1px solid var(--color-border-soft)" }}>
          <SlidersHorizontal size={13} style={{ color: "var(--color-accent)" }} />
          <span className="heading text-[0.78rem] font-semibold">Graph settings</span>
        </div>

        <Section id="filters" label="Filters" open={open} setOpen={setOpen}>
          <div className="flex items-center gap-2 rounded-lg px-2.5 h-8" style={{ background: "var(--color-app)", border: "1px solid var(--color-border)" }}>
            <Search size={12} className="faint shrink-0" />
            <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search memories…"
              className="bg-transparent outline-none text-[0.75rem] w-full" style={{ color: "var(--color-text)" }} />
            {query && <button onClick={() => setQuery("")} className="faint shrink-0"><X size={12} /></button>}
          </div>
        </Section>

        <Section id="groups" label="Groups" open={open} setOpen={setOpen}>
          <div className="flex flex-col gap-0.5">
            {groups.map((g) => (
              <div key={g.author} className="flex items-center gap-2.5 px-1 py-1">
                <span className="w-2.5 h-2.5 rounded-full shrink-0" style={{ background: COLOR[g.color] }} />
                <span className="text-[0.74rem] flex-1" style={{ color: "var(--color-text)" }}>{authorName[g.author]}</span>
                <span className="num faint text-[0.64rem]">{g.count}</span>
              </div>
            ))}
          </div>
        </Section>

        <Section id="forces" label="Forces" open={open} setOpen={setOpen}>
          <Slider label="Center force" value={forces.center} min={0} max={0.1} step={0.005} onChange={(v) => setForces({ ...forces, center: v })} />
          <Slider label="Repel force" value={forces.repel} min={400} max={3200} step={100} onChange={(v) => setForces({ ...forces, repel: v })} />
          <Slider label="Link force" value={forces.link} min={0} max={0.2} step={0.005} onChange={(v) => setForces({ ...forces, link: v })} />
          <Slider label="Link distance" value={forces.dist} min={40} max={180} step={2} onChange={(v) => setForces({ ...forces, dist: v })} />
        </Section>
      </div>
    </div>
  );
}
