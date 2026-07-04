import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type PointerEvent as RPE,
  type WheelEvent as RWheel,
  type ReactNode,
} from "react";
import {
  Layers,
  Search,
  Network,
  Maximize2,
  SlidersHorizontal,
  ChevronDown,
  X,
  Sparkles,
  Link2,
  CornerDownRight,
} from "lucide-react";
import { ipc } from "../ipc";
import type { MemoryGraphNode, MemoryGraphEdge } from "../ipc";
import { timeHint } from "../lib/timeHint";

/* Memory view — an Obsidian-style Graph View over the workspace's durable
   memories (the `conclave memory` semantic store). Each node is a memory chunk;
   each edge is a link between two — an explicit [[wikilink]] (`wiki`, solid) or
   a store-inferred proximity tie (`related`, dashed), both derived backend-side
   at query time (ADR 0007). Node radius scales with degree; colour = the agent
   who wrote the memory (resolved from the roster), with a dedicated violet for
   cross-cutting "Shared" memories that have no single author.

   Lifted from the Arta proto (.arta/proto/screens/memory-graph.tsx @ 73ac6fa).
   No graph library: react-flow renders boxed nodes-with-handles (wrong look)
   and d3-force isn't installed, so the physics is a small hand-rolled
   velocity-integrated force sim (repulsion + link springs + centering) drawn to
   SVG. Proto `var(--color-*)` tokens are remapped to the real app's flipping
   semantic tokens so the view is theme-aware (the proto was dark-only); the SVG
   edge/label strokes ride `--color-overlay`, which inverts black↔white with the
   theme. */

// Shared-protocol memories (sourceId null / unresolved) get macOS system purple
// — the canon's `--color-a-violet` @ 73ac6fa. A literal hex (not a CSS var)
// because the real app has no violet identity token and agent identity colours
// are themselves theme-stable hex.
const SHARED_COLOR = "#bf5af0";
const SHARED_NAME = "Shared";

// ── view model ──────────────────────────────────────────────────────────────
interface ViewNode {
  id: string; // chunk UUID — the identity every keyed map uses
  label: string; // first sentence of `text`, ≤64 chars
  body: string; // the full chunk text (detail card)
  color: string; // resolved hex — agent identity colour, or SHARED_COLOR
  authorName: string; // resolved agent name, or "Shared"
  age: string; // relative, from createdAt
  sourceId: string | null;
}

// First sentence of a chunk, collapsed + capped at 64 — the store has no titles,
// so the node label is derived client-side.
function firstSentence(text: string): string {
  const t = text.trim().replace(/\s+/g, " ");
  const m = t.match(/^.*?[.!?](?=\s|$)/);
  const s = (m ? m[0] : t).trim();
  return s.length > 64 ? s.slice(0, 63).trimEnd() + "…" : s;
}

interface Sim {
  id: string;
  x: number;
  y: number;
  vx: number;
  vy: number;
}
interface Params {
  center: number;
  repel: number;
  link: number;
  dist: number;
  damping: number;
}

// deterministic PRNG so the initial scatter is stable across renders
function seeded(seed: number) {
  let s = seed >>> 0;
  return () => {
    s = (s * 1664525 + 1013904223) >>> 0;
    return s / 4294967296;
  };
}

// One integration step of the hand-rolled force sim (repulsion + link springs +
// centering). Mutates `ns` in place. Module-scope + pure so the same code both
// pre-warms the layout synchronously and drives the live rAF loop. Unlike the
// proto (which closed over module-scope `memoryLinks`), links are passed in —
// the real graph's edges arrive as data.
function tick(ns: Sim[], links: MemoryGraphEdge[], p: Params, alpha: number, held: string | null) {
  const byId = new Map(ns.map((n) => [n.id, n]));
  for (const n of ns) {
    n.vx += -n.x * p.center * alpha;
    n.vy += -n.y * p.center * alpha;
  }
  for (let i = 0; i < ns.length; i++) {
    for (let j = i + 1; j < ns.length; j++) {
      const a = ns[i],
        b = ns[j];
      let dx = b.x - a.x,
        dy = b.y - a.y,
        d2 = dx * dx + dy * dy;
      if (d2 < 1) {
        d2 = 1;
        dx = i - j || 0.5;
      }
      const d = Math.sqrt(d2),
        f = (p.repel * alpha) / d2,
        ux = dx / d,
        uy = dy / d;
      a.vx -= ux * f;
      a.vy -= uy * f;
      b.vx += ux * f;
      b.vy += uy * f;
    }
  }
  for (const l of links) {
    const a = byId.get(l.a),
      b = byId.get(l.b);
    if (!a || !b) continue;
    const dx = b.x - a.x,
      dy = b.y - a.y,
      d = Math.sqrt(dx * dx + dy * dy) || 1;
    const f = ((d - p.dist) * p.link * alpha) / d;
    a.vx += dx * f;
    a.vy += dy * f;
    b.vx -= dx * f;
    b.vy -= dy * f;
  }
  for (const n of ns) {
    if (n.id === held) {
      n.vx = 0;
      n.vy = 0;
      continue;
    }
    n.vx *= p.damping;
    n.vy *= p.damping;
    n.x += n.vx;
    n.y += n.vy;
  }
}

export interface MemoryGraphProps {
  workspaceId: string;
  workspaceName?: string;
  onClose?: () => void;
}

export function MemoryGraph({ workspaceId, workspaceName, onClose }: MemoryGraphProps) {
  // ── data: the graph + the roster identity map (instanceId → name/colour) ──
  const [nodes, setNodes] = useState<ViewNode[]>([]);
  const [edges, setEdges] = useState<MemoryGraphEdge[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState(false);

  const seq = useRef(0);
  useEffect(() => {
    const mine = ++seq.current;
    setLoading(true);
    setLoadError(false);
    Promise.all([
      ipc.memory.graph({ workspaceId }),
      ipc.instance.list({ workspaceId }),
      ipc.agentDef.list(),
    ])
      .then(([graph, instances, defs]) => {
        if (mine !== seq.current) return; // a newer load supersedes this one
        // instanceId → { name, colour } — the same join the Roster/Blackboard use.
        const defById = new Map(defs.map((d) => [d.id, d]));
        const identity = new Map<string, { name: string; color: string }>();
        for (const inst of instances) {
          const def = defById.get(inst.agentDefId);
          if (!def) continue;
          identity.set(inst.id, { name: def.name, color: def.color ?? "#6e6e73" });
        }
        const view: ViewNode[] = graph.nodes.map((n: MemoryGraphNode) => {
          const who = n.sourceId ? identity.get(n.sourceId) : undefined;
          return {
            id: n.id,
            label: firstSentence(n.text),
            body: n.text,
            color: who?.color ?? SHARED_COLOR,
            authorName: who?.name ?? SHARED_NAME,
            age: timeHint(n.createdAt),
            sourceId: n.sourceId,
          };
        });
        // Guard against an edge referencing a chunk that isn't in the node set
        // (shouldn't happen from the backend, but a dangling id would crash the
        // renderer's `pos(...)!`).
        const ids = new Set(view.map((n) => n.id));
        const safeEdges = graph.edges.filter((e) => ids.has(e.a) && ids.has(e.b));
        setNodes(view);
        setEdges(safeEdges);
        setLoading(false);
      })
      .catch((err: unknown) => {
        if (mine !== seq.current) return;
        if (import.meta.env.DEV) console.error("MemoryGraph: load failed", err);
        setNodes([]);
        setEdges([]);
        setLoading(false);
        setLoadError(true);
      });
  }, [workspaceId]);

  // ── derived structure ──
  const nodeById = useMemo(() => Object.fromEntries(nodes.map((n) => [n.id, n])), [nodes]);
  const degree = useMemo(() => {
    const d: Record<string, number> = {};
    for (const n of nodes) d[n.id] = 0;
    for (const l of edges) {
      d[l.a] = (d[l.a] ?? 0) + 1;
      d[l.b] = (d[l.b] ?? 0) + 1;
    }
    return d;
  }, [nodes, edges]);
  const adjacency = useMemo(() => {
    const m: Record<string, Set<string>> = {};
    for (const n of nodes) m[n.id] = new Set();
    for (const l of edges) {
      m[l.a]?.add(l.b);
      m[l.b]?.add(l.a);
    }
    return m;
  }, [nodes, edges]);
  const radius = (id: string) => 5 + (degree[id] ?? 0) * 1.7;

  // author groups for the legend — one bucket per resolved identity (Shared
  // folds every unauthored chunk together). Sorted by count desc for stability.
  const groups = useMemo(() => {
    const m = new Map<string, { name: string; color: string; count: number }>();
    for (const n of nodes) {
      const key = n.sourceId ?? "__shared__";
      const g = m.get(key) ?? { name: n.authorName, color: n.color, count: 0 };
      g.count += 1;
      m.set(key, g);
    }
    return [...m.values()].sort((a, b) => b.count - a.count || a.name.localeCompare(b.name));
  }, [nodes]);

  // ── simulation state (mutable, driven by rAF) ──
  const nodesRef = useRef<Sim[]>([]);
  const edgesRef = useRef<MemoryGraphEdge[]>([]);
  edgesRef.current = edges;
  const alphaRef = useRef(0.9);
  const [, setFrame] = useState(0);

  const [forces, setForces] = useState({ center: 0.025, repel: 2500, link: 0.045, dist: 118 });
  const paramsRef = useRef<Params>({ ...forces, damping: 0.82 });
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
    const set = () => {
      if (!viewInit.current) {
        const r = el.getBoundingClientRect();
        setView((v) => ({ ...v, tx: r.width / 2, ty: r.height / 2 }));
        viewInit.current = true;
      }
    };
    set();
    const ro = new ResizeObserver(set);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // ── interaction state ──
  const [hover, setHover] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const dragRef = useRef<{
    id: string | null;
    panX: number;
    panY: number;
    sx: number;
    sy: number;
    moved: boolean;
  } | null>(null);

  // ── physics loop ──
  useEffect(() => {
    let raf = 0;
    const step = () => {
      const alpha = alphaRef.current;
      tick(nodesRef.current, edgesRef.current, paramsRef.current, alpha, dragRef.current?.id ?? null);
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
      if (n) {
        n.x = w.x;
        n.y = w.y;
        n.vx = 0;
        n.vy = 0;
      }
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
    const mx = e.clientX - r.left,
      my = e.clientY - r.top;
    setView((v) => {
      const k = Math.min(3, Math.max(0.35, v.k * (e.deltaY < 0 ? 1.12 : 1 / 1.12)));
      const wx = (mx - v.tx) / v.k,
        wy = (my - v.ty) / v.k;
      return { k, tx: mx - wx * k, ty: my - wy * k };
    });
  };
  const fit = () => {
    if (!wrapRef.current) return;
    const ns = nodesRef.current;
    if (ns.length === 0) return;
    const minX = Math.min(...ns.map((n) => n.x)),
      maxX = Math.max(...ns.map((n) => n.x));
    const minY = Math.min(...ns.map((n) => n.y)),
      maxY = Math.max(...ns.map((n) => n.y));
    const cx = (minX + maxX) / 2,
      cy = (minY + maxY) / 2;
    const bw = Math.max(maxX - minX, 1),
      bh = Math.max(maxY - minY, 1);
    const r = wrapRef.current.getBoundingClientRect();
    // frame into the canvas RIGHT of the floating panel, with breathing room
    const padL = 288,
      padR = 56,
      padY = 96;
    const availW = Math.max(r.width - padL - padR, 200),
      availH = Math.max(r.height - padY * 2, 200);
    const k = Math.min(1.5, Math.max(0.4, Math.min(availW / bw, availH / bh)));
    viewInit.current = true; // claim the view so the center-init effect won't clobber it
    setView({ k, tx: padL + availW / 2 - cx * k, ty: padY + availH / 2 - cy * k });
  };
  const fitRef = useRef(fit);
  fitRef.current = fit;

  // (Re)build the sim whenever the node set changes: seeded scatter, pre-warm
  // synchronously before first paint, then frame it. `nodes` identity only
  // changes on a fresh load, so this doesn't churn every render.
  useLayoutEffect(() => {
    if (nodes.length === 0) {
      nodesRef.current = [];
      return;
    }
    const rnd = seeded(42);
    nodesRef.current = nodes.map((n) => {
      const a = rnd() * Math.PI * 2;
      const r = 90 + rnd() * 150;
      return { id: n.id, x: Math.cos(a) * r, y: Math.sin(a) * r, vx: 0, vy: 0 };
    });
    const p = paramsRef.current;
    let a = 0.9;
    for (let i = 0; i < 240; i++) {
      tick(nodesRef.current, edgesRef.current, p, a, null);
      a *= 0.99;
    }
    alphaRef.current = 0.05; // settled but gently alive for interaction
    fitRef.current();
  }, [nodes]);

  // ── derived highlight sets ──
  const focus = hover ?? selected;
  const q = query.trim().toLowerCase();
  const matches = (id: string) => {
    if (!q) return true;
    const n = nodeById[id];
    return (n.label + " " + n.body).toLowerCase().includes(q);
  };
  const isActive = (id: string) => {
    if (focus) return id === focus || adjacency[focus]?.has(id);
    return true;
  };
  const nodeOpacity = (id: string) => {
    const dimByFocus = focus && !isActive(id);
    const dimByQuery = q && !matches(id);
    return dimByFocus || dimByQuery ? 0.16 : 1;
  };
  const edgeActive = (a: string, b: string) => !focus || a === focus || b === focus;

  const posOf = (id: string) => nodesRef.current.find((n) => n.id === id);
  const sel = selected ? nodeById[selected] : null;

  const isEmpty = !loading && !loadError && nodes.length === 0;

  return (
    <main
      ref={wrapRef}
      className="flex-1 min-w-0 relative overflow-hidden"
      style={{
        background:
          "radial-gradient(120% 100% at 50% 0%, var(--color-surface) 0%, var(--color-bg-canvas) 62%)",
        color: "var(--color-text-body)",
      }}
    >
      {/* header strip — the proto's floating blurred bar, plus the center-pane
          close affordance (returns to the agent pane, like Blackboard/Chat) */}
      <div
        className="absolute top-0 left-0 right-0 h-12 z-20 flex items-center gap-3 px-4 border-b border-overlay/[0.06]"
        style={{
          background: "color-mix(in srgb, var(--color-bg-canvas) 78%, transparent)",
          backdropFilter: "blur(8px)",
        }}
      >
        <span className="w-6 h-6 rounded-[7px] grid place-items-center bg-ink text-on-ink shrink-0 ring-hair">
          <Layers size={13} />
        </span>
        <div className="leading-tight">
          <div className="text-[0.84rem] font-semibold tracking-tight text-text-primary">Memory</div>
          <div className="text-[0.64rem] -mt-0.5 text-text-tertiary">
            {workspaceName ? `${workspaceName} · knowledge graph` : "knowledge graph"}
          </div>
        </div>
        <Pill>
          <Sparkles size={11} style={{ color: "var(--color-accent)" }} />
          {nodes.length} {nodes.length === 1 ? "memory" : "memories"}
        </Pill>
        <Pill>
          <Link2 size={11} />
          {edges.length} {edges.length === 1 ? "link" : "links"}
        </Pill>
        <div className="ml-auto flex items-center gap-1.5">
          <span className="font-mono tabular-nums text-text-tertiary text-[0.64rem]">
            {Math.round(view.k * 100)}%
          </span>
          <IconBtn onClick={() => fitRef.current()} title="Fit graph to view">
            <Maximize2 size={14} />
          </IconBtn>
          {onClose && (
            <IconBtn onClick={onClose} title="Close Memory" aria-label="Close Memory">
              <X size={14} />
            </IconBtn>
          )}
        </div>
      </div>

      {isEmpty ? (
        <EmptyState />
      ) : loadError ? (
        <CenterMessage>Couldn’t load the memory graph.</CenterMessage>
      ) : loading ? (
        <CenterMessage>Loading memory graph…</CenterMessage>
      ) : (
        <>
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
              {edges.map((l, i) => {
                const a = posOf(l.a),
                  b = posOf(l.b);
                if (!a || !b) return null;
                const on = edgeActive(l.a, l.b) && (!q || matches(l.a) || matches(l.b));
                const touchesFocus = focus && (l.a === focus || l.b === focus);
                return (
                  <line
                    key={i}
                    x1={a.x}
                    y1={a.y}
                    x2={b.x}
                    y2={b.y}
                    stroke={touchesFocus ? "var(--color-accent)" : "var(--color-overlay)"}
                    strokeOpacity={touchesFocus ? 0.55 : on ? 0.12 : 0.03}
                    strokeWidth={(touchesFocus ? 1.6 : 1) / view.k}
                    strokeDasharray={l.rel === "related" ? `${4 / view.k} ${4 / view.k}` : undefined}
                  />
                );
              })}
              {/* nodes */}
              {nodes.map((n) => {
                const p = posOf(n.id);
                if (!p) return null;
                const r = radius(n.id) * (focus === n.id ? 1.28 : 1);
                const op = nodeOpacity(n.id);
                const isFocus = focus === n.id;
                const isMatch = q ? matches(n.id) : false;
                const showLabel =
                  op > 0.5 &&
                  (view.k > 0.95 || isFocus || (focus != null && adjacency[focus]?.has(n.id)));
                return (
                  <g
                    key={n.id}
                    style={{ opacity: op, transition: "opacity .18s ease" }}
                    onPointerDown={onNodeDown(n.id)}
                    onPointerEnter={() => setHover(n.id)}
                    onPointerLeave={() => setHover((h) => (h === n.id ? null : h))}
                  >
                    {(isFocus || isMatch) && (
                      <circle
                        cx={p.x}
                        cy={p.y}
                        r={r + 5 / view.k}
                        fill="none"
                        stroke={isMatch && !isFocus ? "var(--color-accent)" : n.color}
                        strokeOpacity={0.5}
                        strokeWidth={1.5 / view.k}
                      />
                    )}
                    <circle
                      cx={p.x}
                      cy={p.y}
                      r={r}
                      fill={n.color}
                      stroke="var(--color-bg-canvas)"
                      strokeWidth={1.5 / view.k}
                      style={{ cursor: "pointer", filter: isFocus ? "brightness(1.15)" : undefined }}
                    />
                    {showLabel && (
                      <text
                        x={p.x}
                        y={p.y + r + 11 / view.k}
                        textAnchor="middle"
                        fontSize={9 / view.k}
                        fill={isFocus ? "var(--color-text-primary)" : "var(--color-text-secondary)"}
                        style={{
                          pointerEvents: "none",
                          fontWeight: isFocus ? 600 : 500,
                          paintOrder: "stroke",
                          stroke: "var(--color-bg-canvas)",
                          strokeWidth: 3 / view.k,
                          strokeLinejoin: "round",
                        }}
                      >
                        {n.label.length > 26 ? n.label.slice(0, 25) + "…" : n.label}
                      </text>
                    )}
                  </g>
                );
              })}
            </g>
          </svg>

          {/* ── Obsidian-style control panel ── */}
          <ControlPanel query={query} setQuery={setQuery} forces={forces} setForces={setForces} groups={groups} />

          {/* ── detail card ── */}
          <div
            className="absolute top-16 right-4 z-20"
            style={{
              width: 288,
              opacity: sel ? 1 : 0,
              transform: sel ? "translateX(0)" : "translateX(12px)",
              pointerEvents: sel ? "auto" : "none",
              transition: "opacity .2s ease, transform .2s ease",
            }}
          >
            {sel && (
              <div
                className="rounded-xl overflow-hidden bg-surface-raised"
                style={{
                  border: "1px solid color-mix(in srgb, var(--color-overlay) 8%, transparent)",
                  boxShadow: "var(--shadow-popover)",
                }}
              >
                <div
                  className="flex items-start gap-2.5 p-3.5 pb-3"
                  style={{ borderBottom: "1px solid color-mix(in srgb, var(--color-overlay) 6%, transparent)" }}
                >
                  <span
                    className="w-2.5 h-2.5 rounded-full mt-1.5 shrink-0"
                    style={{
                      background: sel.color,
                      boxShadow: `0 0 0 3px color-mix(in srgb, ${sel.color} 22%, transparent)`,
                    }}
                  />
                  <div className="min-w-0 flex-1">
                    <div className="text-[0.86rem] font-semibold leading-snug text-text-primary">{sel.label}</div>
                    <div className="flex items-center gap-1.5 mt-1">
                      <span className="text-[0.66rem] text-text-tertiary">
                        {sel.authorName} · {sel.age}
                      </span>
                    </div>
                  </div>
                  <IconBtn onClick={() => setSelected(null)} aria-label="Close detail">
                    <X size={14} />
                  </IconBtn>
                </div>
                <div className="px-3.5 py-3 text-[0.78rem] leading-relaxed text-text-body">{sel.body}</div>
                <div className="px-3.5 pb-3.5">
                  <Label className="mb-1.5">Linked · {adjacency[sel.id]?.size ?? 0}</Label>
                  <div className="flex flex-col gap-0.5">
                    {[...(adjacency[sel.id] ?? [])].map((id) => {
                      const ln = nodeById[id];
                      if (!ln) return null;
                      return (
                        <button
                          key={id}
                          onClick={() => setSelected(id)}
                          onPointerEnter={() => setHover(id)}
                          onPointerLeave={() => setHover(null)}
                          className="flex items-center gap-2 px-2 py-1.5 rounded-md text-left transition-colors hover:bg-overlay/[0.04]"
                        >
                          <CornerDownRight size={12} className="text-text-tertiary shrink-0" />
                          <span className="w-2 h-2 rounded-full shrink-0" style={{ background: ln.color }} />
                          <span className="text-[0.74rem] truncate text-text-body">{ln.label}</span>
                        </button>
                      );
                    })}
                  </div>
                </div>
              </div>
            )}
          </div>

          {/* hint */}
          <div className="absolute bottom-3 right-4 z-10 text-[0.64rem] font-mono text-text-tertiary select-none">
            scroll to zoom · drag to pan · drag a node to pull
          </div>
        </>
      )}
    </main>
  );
}

// ── small presentational helpers (real-app token equivalents of the proto's
//    .pill / .ctx-ibtn / .label helper classes) ──────────────────────────────

function Pill({ children }: { children: ReactNode }) {
  return (
    <span
      className="inline-flex items-center gap-1 text-[0.68rem] font-semibold px-1.5 py-0.5 rounded-full text-text-secondary bg-bg-canvas border border-overlay/[0.08]"
    >
      {children}
    </span>
  );
}

function IconBtn({
  children,
  onClick,
  title,
  ...rest
}: {
  children: ReactNode;
  onClick?: () => void;
  title?: string;
  "aria-label"?: string;
}) {
  return (
    <button
      onClick={onClick}
      title={title}
      className="w-6 h-6 grid place-items-center rounded-md text-text-muted hover:bg-overlay/[0.06] hover:text-text-primary shrink-0 transition-colors"
      {...rest}
    >
      {children}
    </button>
  );
}

function Label({ children, className = "" }: { children: ReactNode; className?: string }) {
  return (
    <div
      className={`text-[0.68rem] tracking-[0.09em] uppercase font-semibold text-text-tertiary ${className}`}
    >
      {children}
    </div>
  );
}

function CenterMessage({ children }: { children: ReactNode }) {
  return (
    <div className="absolute inset-0 grid place-items-center text-[13px] text-text-tertiary px-6 text-center">
      {children}
    </div>
  );
}

// Honest empty state — a store with no durable memories yet (the expected first
// render: the workspace has written nothing to `conclave memory`).
function EmptyState() {
  return (
    <div className="absolute inset-0 grid place-items-center px-6">
      <div className="flex flex-col items-center text-center max-w-[360px]">
        <span className="w-11 h-11 rounded-xl grid place-items-center bg-surface-raised ring-hair text-text-tertiary mb-3">
          <Network size={20} />
        </span>
        <div className="text-[13px] font-semibold text-text-primary">No memories yet</div>
        <div className="text-[12px] text-text-tertiary mt-1 leading-relaxed">
          Durable facts saved with <span className="font-mono">conclave memory remember</span> appear
          here as a knowledge graph — nodes are memories, edges are the links between them.
        </div>
      </div>
    </div>
  );
}

/* ── the floating settings panel (Filters / Groups / Forces) ──
   Section + Slider live at MODULE scope on purpose: defining them inside
   ControlPanel would give them a fresh identity every render, remounting the
   search <input> and dropping focus on each keystroke. */
type OpenState = { filters: boolean; groups: boolean; forces: boolean };

function Section({
  id,
  label,
  open,
  setOpen,
  children,
}: {
  id: keyof OpenState;
  label: string;
  open: OpenState;
  setOpen: (fn: (o: OpenState) => OpenState) => void;
  children: ReactNode;
}) {
  return (
    <div
      className="border-t first:border-t-0"
      style={{ borderColor: "color-mix(in srgb, var(--color-overlay) 6%, transparent)" }}
    >
      <button
        onClick={() => setOpen((o) => ({ ...o, [id]: !o[id] }))}
        className="w-full flex items-center gap-1.5 px-3 py-2 text-[0.68rem] tracking-[0.09em] uppercase font-semibold text-text-tertiary hover:text-text-body transition-colors"
      >
        <ChevronDown
          size={12}
          style={{ transform: open[id] ? "none" : "rotate(-90deg)", transition: "transform .15s" }}
        />
        {label}
      </button>
      {open[id] && <div className="px-3 pb-3">{children}</div>}
    </div>
  );
}

function Slider({
  label,
  value,
  min,
  max,
  step,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  onChange: (v: number) => void;
}) {
  return (
    <label className="block mb-2.5 last:mb-0">
      <span className="flex items-center justify-between text-[0.72rem] mb-1">
        <span className="text-text-body">{label}</span>
        <span className="font-mono tabular-nums text-text-tertiary text-[0.64rem]">{value}</span>
      </span>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
        className="gr-range"
      />
    </label>
  );
}

function ControlPanel({
  query,
  setQuery,
  forces,
  setForces,
  groups,
}: {
  query: string;
  setQuery: (v: string) => void;
  forces: { center: number; repel: number; link: number; dist: number };
  setForces: (f: { center: number; repel: number; link: number; dist: number }) => void;
  groups: { name: string; count: number; color: string }[];
}) {
  const [open, setOpen] = useState<OpenState>({ filters: true, groups: true, forces: true });
  return (
    <div className="absolute top-16 left-4 z-20" style={{ width: 236 }}>
      <div
        className="rounded-xl overflow-hidden"
        style={{
          background: "color-mix(in srgb, var(--color-surface-raised) 92%, transparent)",
          border: "1px solid color-mix(in srgb, var(--color-overlay) 8%, transparent)",
          boxShadow: "var(--shadow-popover)",
          backdropFilter: "blur(10px)",
        }}
      >
        <div
          className="flex items-center gap-2 px-3 h-10"
          style={{ borderBottom: "1px solid color-mix(in srgb, var(--color-overlay) 6%, transparent)" }}
        >
          <SlidersHorizontal size={13} style={{ color: "var(--color-accent)" }} />
          <span className="text-[0.78rem] font-semibold text-text-primary">Graph settings</span>
        </div>

        <Section id="filters" label="Filters" open={open} setOpen={setOpen}>
          <div
            className="flex items-center gap-2 rounded-lg px-2.5 h-8 bg-bg-canvas"
            style={{ border: "1px solid color-mix(in srgb, var(--color-overlay) 8%, transparent)" }}
          >
            <Search size={12} className="text-text-tertiary shrink-0" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search memories…"
              className="bg-transparent outline-none text-[0.75rem] w-full text-text-body placeholder:text-text-tertiary"
            />
            {query && (
              <button onClick={() => setQuery("")} className="text-text-tertiary shrink-0" aria-label="Clear search">
                <X size={12} />
              </button>
            )}
          </div>
        </Section>

        <Section id="groups" label="Groups" open={open} setOpen={setOpen}>
          {groups.length === 0 ? (
            <div className="text-[0.72rem] text-text-tertiary px-1 py-1">No authors yet</div>
          ) : (
            <div className="flex flex-col gap-0.5">
              {groups.map((g) => (
                <div key={g.name} className="flex items-center gap-2.5 px-1 py-1">
                  <span className="w-2.5 h-2.5 rounded-full shrink-0" style={{ background: g.color }} />
                  <span className="text-[0.74rem] flex-1 text-text-body">{g.name}</span>
                  <span className="font-mono tabular-nums text-text-tertiary text-[0.64rem]">{g.count}</span>
                </div>
              ))}
            </div>
          )}
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
