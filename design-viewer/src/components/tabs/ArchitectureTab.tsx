import { useEffect, useMemo, useRef, useState } from "react";
import {
  ReactFlow,
  Background,
  BackgroundVariant,
  Controls,
  MiniMap,
  Handle,
  Position,
  MarkerType,
  useNodesState,
  useEdgesState,
  type Node,
  type Edge,
  type NodeProps,
  type ReactFlowInstance,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import dagre from "@dagrejs/dagre";
import {
  Box, Boxes, Cloud, Database, Globe, Inbox, Maximize2, Monitor, Network, Server, ShieldAlert, Gauge, Zap, X,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { Architecture, ArchEdge, ArchKind, ArchNode } from "../../lib/types";
import { MONO, useTheme, type DarkTokens } from "../../lib/theme";
import { alpha } from "../../lib/utils";

function kindMeta(c: DarkTokens): Record<ArchKind, { color: string; Icon: LucideIcon; label: string }> {
  return {
    client: { color: c.accent, Icon: Monitor, label: "Client" },
    service: { color: c.green, Icon: Server, label: "Service" },
    datastore: { color: c.amber, Icon: Database, label: "Datastore" },
    external: { color: c.dim, Icon: Globe, label: "External" },
    gateway: { color: "#a78bfa", Icon: Network, label: "Gateway" },
    queue: { color: "#fb923c", Icon: Inbox, label: "Queue" },
    cache: { color: "#f472b6", Icon: Zap, label: "Cache" },
    infra: { color: c.faint, Icon: Cloud, label: "Infra" },
  };
}

const NODE_W = 216;
const NODE_H = 74;

type ArchNodeData = {
  id: string;
  name: string;
  kind: ArchKind;
  tech?: string;
  description?: string;
  deployment?: string;
  group?: string;
};
type ArchFlowNode = Node<ArchNodeData, "arch">;

function ArchNodeView({ data, selected }: NodeProps<ArchFlowNode>) {
  const { c } = useTheme();
  const [hover, setHover] = useState(false);
  const meta = kindMeta(c)[data.kind] || kindMeta(c).service;
  const col = meta.color;
  const active = selected || hover;
  const Icon = meta.Icon;
  return (
    <div
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        width: NODE_W,
        borderRadius: 12,
        overflow: "hidden",
        fontFamily: MONO,
        background: c.card,
        border: `1px solid ${selected ? col : active ? alpha(col, 0.6) : c.border}`,
        boxShadow: selected ? `0 0 0 1px ${col}, 0 10px 26px ${alpha(col, 0.2)}` : c.shadow,
        transform: active ? "translateY(-1px)" : "none",
        transition: "box-shadow .14s ease, transform .14s ease, border-color .14s ease",
        cursor: "pointer",
      }}
    >
      <Handle type="target" position={Position.Left} style={{ width: 9, height: 9, background: c.card, border: `2px solid ${col}`, borderRadius: 99 }} />
      <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "8px 11px", background: alpha(col, 0.1), borderBottom: `1px solid ${c.borderSoft}` }}>
        <Icon size={14} color={col} />
        <span style={{ color: c.text, fontSize: 12.5, fontWeight: 600, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{data.name}</span>
        <span style={{ marginLeft: "auto", fontSize: 8.5, letterSpacing: 0.6, textTransform: "uppercase", color: col }}>{meta.label}</span>
      </div>
      <div style={{ padding: "7px 11px" }}>
        {data.tech && <div style={{ color: c.dim, fontSize: 11, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{data.tech}</div>}
        {data.deployment && (
          <div style={{ marginTop: 5, display: "inline-flex", alignItems: "center", gap: 4, fontSize: 9.5, color: c.faint, background: c.panel2, border: `1px solid ${c.borderSoft}`, borderRadius: 5, padding: "1px 6px" }}>
            <Cloud size={9} /> {data.deployment}
          </div>
        )}
      </div>
      <Handle type="source" position={Position.Right} style={{ width: 9, height: 9, background: c.card, border: `2px solid ${col}`, borderRadius: 99 }} />
    </div>
  );
}

const nodeTypes = { arch: ArchNodeView };

function buildGraph(arch: Architecture, c: DarkTokens): { nodes: ArchFlowNode[]; edges: Edge[] } {
  const ns = arch.nodes || [];
  const es = (arch.edges || []).filter((e) => ns.some((n) => n.id === e.from) && ns.some((n) => n.id === e.to));
  const g = new dagre.graphlib.Graph();
  g.setGraph({ rankdir: "LR", nodesep: 30, ranksep: 120, marginx: 28, marginy: 28 });
  g.setDefaultEdgeLabel(() => ({}));
  ns.forEach((n) => g.setNode(n.id, { width: NODE_W, height: NODE_H }));
  es.forEach((e) => g.setEdge(e.from, e.to));
  dagre.layout(g);

  const nodes: ArchFlowNode[] = ns.map((n): ArchFlowNode => {
    const p = g.node(n.id);
    return { id: n.id, type: "arch", position: { x: (p?.x ?? 0) - NODE_W / 2, y: (p?.y ?? 0) - NODE_H / 2 }, data: n };
  });
  const edges: Edge[] = es.map((e, i) => {
    const async = e.mode === "async";
    const label = [e.protocol, e.label].filter(Boolean).join(" · ");
    return {
      id: `ae${i}`,
      source: e.from,
      target: e.to,
      type: "smoothstep",
      animated: async,
      pathOptions: { borderRadius: 14 },
      label: label || undefined,
      labelStyle: { fill: c.dim, fontFamily: MONO, fontSize: 9.5 },
      labelBgStyle: { fill: c.card, fillOpacity: 0.95 },
      labelBgPadding: [5, 2] as [number, number],
      labelBgBorderRadius: 4,
      style: { stroke: alpha(c.dim, 0.55), strokeWidth: 1.4, strokeDasharray: async ? "5 4" : undefined },
      markerEnd: { type: MarkerType.ArrowClosed, color: alpha(c.dim, 0.6), width: 14, height: 14 },
    };
  });
  return { nodes, edges };
}

function NodeInspector({ node, arch, c, onClose }: { node: ArchNode; arch: Architecture; c: DarkTokens; onClose: () => void }) {
  const meta = kindMeta(c)[node.kind] || kindMeta(c).service;
  const nameOf = new Map((arch.nodes || []).map((n) => [n.id, n.name]));
  const out = (arch.edges || []).filter((e) => e.from === node.id);
  const inc = (arch.edges || []).filter((e) => e.to === node.id);
  const Icon = meta.Icon;
  const conn = (e: ArchEdge, dir: "out" | "in") => (
    <div key={`${dir}${e.from}${e.to}`} style={{ display: "flex", alignItems: "center", gap: 7, padding: "6px 0", borderTop: `1px solid ${c.borderSoft}`, fontFamily: MONO, fontSize: 12 }}>
      <span style={{ color: c.faint }}>{dir === "out" ? "→" : "←"}</span>
      <span style={{ color: c.text, flex: 1 }}>{nameOf.get(dir === "out" ? e.to : e.from) || (dir === "out" ? e.to : e.from)}</span>
      {(e.protocol || e.mode) && <span style={{ color: c.dim, fontSize: 10.5 }}>{[e.protocol, e.mode].filter(Boolean).join(" · ")}</span>}
    </div>
  );
  return (
    <aside className="flex h-full flex-col" style={{ width: 360, borderLeft: `1px solid ${c.border}`, background: c.panel }}>
      <div style={{ padding: "14px 16px", borderBottom: `1px solid ${c.border}` }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <Icon size={15} color={meta.color} />
          <span style={{ fontFamily: MONO, fontSize: 13, color: c.text, flex: 1 }}>{node.name}</span>
          <button onClick={onClose} style={{ color: c.dim, background: "transparent", border: "none", cursor: "pointer", padding: 2 }}><X size={16} /></button>
        </div>
        <div style={{ marginTop: 8, display: "flex", flexWrap: "wrap", gap: 6 }}>
          <span style={{ fontFamily: MONO, fontSize: 10, color: meta.color, background: alpha(meta.color, 0.12), border: `1px solid ${alpha(meta.color, 0.4)}`, borderRadius: 5, padding: "1px 7px" }}>{meta.label}</span>
          {node.tech && <span style={{ fontFamily: MONO, fontSize: 10, color: c.dim, background: c.panel2, border: `1px solid ${c.borderSoft}`, borderRadius: 5, padding: "1px 7px" }}>{node.tech}</span>}
          {node.deployment && <span style={{ fontFamily: MONO, fontSize: 10, color: c.dim, background: c.panel2, border: `1px solid ${c.borderSoft}`, borderRadius: 5, padding: "1px 7px" }}>{node.deployment}</span>}
          {node.group && <span style={{ fontFamily: MONO, fontSize: 10, color: c.faint }}>zone: {node.group}</span>}
        </div>
      </div>
      <div style={{ flex: 1, overflow: "auto", padding: 16 }}>
        {node.description && <div style={{ color: c.dim, fontSize: 12.5, lineHeight: 1.5, marginBottom: 14 }}>{node.description}</div>}
        {(out.length > 0 || inc.length > 0) && (
          <div>
            <div style={{ fontFamily: MONO, fontSize: 10, letterSpacing: 0.6, color: c.faint, marginBottom: 4 }}>CONNECTIONS</div>
            {out.map((e) => conn(e, "out"))}
            {inc.map((e) => conn(e, "in"))}
          </div>
        )}
      </div>
    </aside>
  );
}

function DecisionsView({ arch, c }: { arch: Architecture; c: DarkTokens }) {
  const decisions = arch.decisions || [];
  const statusColor = (s?: string) => (s === "accepted" ? c.green : s === "rejected" ? c.red : s === "superseded" ? c.faint : c.amber);
  if (!decisions.length) return <Empty c={c} text="No architecture decisions yet — ADRs (context · options · decision · consequences) appear here." />;
  return (
    <div className="min-h-0 flex-1 overflow-auto" style={{ padding: 24 }}>
      <div style={{ maxWidth: 820, margin: "0 auto", display: "flex", flexDirection: "column", gap: 14 }}>
        {decisions.map((d, i) => (
          <div key={i} style={{ background: c.card, border: `1px solid ${c.border}`, borderRadius: 12, padding: "16px 18px" }}>
            <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
              <span style={{ fontSize: 14.5, fontWeight: 600, color: c.text, flex: 1 }}>{d.id ? `${d.id} · ` : ""}{d.title}</span>
              {d.status && <span style={{ fontFamily: MONO, fontSize: 10, textTransform: "uppercase", letterSpacing: 0.5, color: statusColor(d.status), background: alpha(statusColor(d.status), 0.12), border: `1px solid ${alpha(statusColor(d.status), 0.4)}`, borderRadius: 999, padding: "2px 9px" }}>{d.status}</span>}
            </div>
            {d.context && <Field c={c} label="Context" value={d.context} />}
            {d.options && d.options.length > 0 && (
              <div style={{ marginTop: 10 }}>
                <FieldLabel c={c} label="Options considered" />
                <div style={{ display: "flex", flexWrap: "wrap", gap: 6, marginTop: 4 }}>
                  {d.options.map((o, j) => <span key={j} style={{ fontFamily: MONO, fontSize: 11, color: c.dim, background: c.panel2, border: `1px solid ${c.borderSoft}`, borderRadius: 6, padding: "2px 8px" }}>{o}</span>)}
                </div>
              </div>
            )}
            {d.decision && <Field c={c} label="Decision" value={d.decision} strong />}
            {d.consequences && <Field c={c} label="Consequences" value={d.consequences} />}
          </div>
        ))}
      </div>
    </div>
  );
}

function SecurityView({ arch, c }: { arch: Architecture; c: DarkTokens }) {
  const nfrs = arch.nfrs || [];
  const security = arch.security || [];
  if (!nfrs.length && !security.length) return <Empty c={c} text="No NFRs or security notes yet — non-functional targets and threat-model notes appear here." />;
  return (
    <div className="min-h-0 flex-1 overflow-auto" style={{ padding: 24 }}>
      <div style={{ maxWidth: 820, margin: "0 auto", display: "grid", gridTemplateColumns: "1fr 1fr", gap: 18 }}>
        <Section c={c} Icon={Gauge} title="Non-functional requirements">
          {nfrs.length === 0 ? <Muted c={c} text="None" /> : nfrs.map((n, i) => (
            <div key={i} style={{ padding: "10px 0", borderTop: i ? `1px solid ${c.borderSoft}` : "none" }}>
              <div style={{ display: "flex", alignItems: "baseline", gap: 8 }}>
                <span style={{ fontSize: 13, color: c.text, fontWeight: 600 }}>{n.name}</span>
                {n.target && <span style={{ fontFamily: MONO, fontSize: 11.5, color: c.accent }}>{n.target}</span>}
              </div>
              {n.note && <div style={{ color: c.faint, fontSize: 12, marginTop: 2 }}>{n.note}</div>}
            </div>
          ))}
        </Section>
        <Section c={c} Icon={ShieldAlert} title="Security · threat model">
          {security.length === 0 ? <Muted c={c} text="None" /> : security.map((s, i) => (
            <div key={i} style={{ padding: "10px 0", borderTop: i ? `1px solid ${c.borderSoft}` : "none" }}>
              {s.boundary && <div style={{ fontFamily: MONO, fontSize: 10.5, color: c.red, marginBottom: 3 }}>⛬ {s.boundary}</div>}
              <div style={{ color: c.dim, fontSize: 12.5, lineHeight: 1.45 }}>{s.note}</div>
            </div>
          ))}
        </Section>
      </div>
    </div>
  );
}

// small shared bits
function Empty({ c, text }: { c: DarkTokens; text: string }) {
  return (
    <div className="flex flex-1 items-center justify-center">
      <div className="flex flex-col items-center gap-2.5 text-center" style={{ color: c.faint, fontFamily: MONO }}>
        <Boxes size={28} />
        <div className="max-w-[320px] text-[13px] leading-relaxed">{text}</div>
      </div>
    </div>
  );
}
function FieldLabel({ c, label }: { c: DarkTokens; label: string }) {
  return <div style={{ fontFamily: MONO, fontSize: 10, letterSpacing: 0.6, color: c.faint }}>{label.toUpperCase()}</div>;
}
function Field({ c, label, value, strong }: { c: DarkTokens; label: string; value: string; strong?: boolean }) {
  return (
    <div style={{ marginTop: 10 }}>
      <FieldLabel c={c} label={label} />
      <div style={{ color: strong ? c.text : c.dim, fontSize: 12.5, lineHeight: 1.5, marginTop: 3 }}>{value}</div>
    </div>
  );
}
function Section({ c, Icon, title, children }: { c: DarkTokens; Icon: LucideIcon; title: string; children: React.ReactNode }) {
  return (
    <div style={{ background: c.card, border: `1px solid ${c.border}`, borderRadius: 12, padding: "14px 16px" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 8 }}>
        <Icon size={14} color={c.accent} />
        <span style={{ fontSize: 13, fontWeight: 600, color: c.text }}>{title}</span>
      </div>
      {children}
    </div>
  );
}
function Muted({ c, text }: { c: DarkTokens; text: string }) {
  return <div style={{ color: c.faint, fontFamily: MONO, fontSize: 12 }}>{text}</div>;
}

type View = "diagram" | "decisions" | "security";

export function ArchitectureTab({ architecture }: { architecture: Architecture }) {
  const { c, mode } = useTheme();
  const [view, setView] = useState<View>("diagram");
  const { nodes: laidNodes, edges: laidEdges } = useMemo(() => buildGraph(architecture, c), [architecture, c]);
  const [nodes, setNodes, onNodesChange] = useNodesState<ArchFlowNode>(laidNodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState(laidEdges);
  const [selId, setSelId] = useState<string | null>(null);
  const instRef = useRef<ReactFlowInstance<ArchFlowNode, Edge> | null>(null);

  useEffect(() => {
    setNodes(laidNodes);
    setEdges(laidEdges);
  }, [laidNodes, laidEdges, setNodes, setEdges]);

  const archNodes = architecture.nodes || [];
  const sig = useMemo(() => laidNodes.map((n) => n.id).join("|"), [laidNodes]);
  useEffect(() => {
    if (selId && !archNodes.some((n) => n.id === selId)) setSelId(null);
  }, [archNodes, selId]);
  const selected = archNodes.find((n) => n.id === selId);

  const decisionsN = (architecture.decisions || []).length;
  const notesN = (architecture.nfrs || []).length + (architecture.security || []).length;
  const seg: { id: View; label: string; n?: number }[] = [
    { id: "diagram", label: "Diagram" },
    { id: "decisions", label: "Decisions", n: decisionsN },
    { id: "security", label: "Security & NFRs", n: notesN },
  ];

  return (
    <div className="flex h-full w-full flex-col">
      {/* toolbar */}
      <div style={{ display: "flex", alignItems: "center", gap: 12, padding: "0 14px", minHeight: 46, borderBottom: `1px solid ${c.border}`, background: c.panel, flexShrink: 0, flexWrap: "wrap" }}>
        <Boxes size={15} color={c.accent} />
        <span style={{ fontFamily: MONO, fontSize: 13, color: c.text }}>Architecture</span>
        <div style={{ display: "flex", gap: 2, background: c.panel2, border: `1px solid ${c.borderSoft}`, borderRadius: 8, padding: 2 }}>
          {seg.map((s) => (
            <button key={s.id} onClick={() => setView(s.id)} style={{ fontFamily: MONO, fontSize: 11.5, padding: "5px 11px", borderRadius: 6, border: "none", cursor: "pointer", background: view === s.id ? c.card : "transparent", color: view === s.id ? c.text : c.dim }}>
              {s.label}{s.n ? <span style={{ color: c.faint, marginLeft: 5 }}>{s.n}</span> : ""}
            </button>
          ))}
        </div>
        {view === "diagram" && (
          <button onClick={() => instRef.current?.fitView({ padding: 0.2, duration: 400 })} style={{ marginLeft: "auto", display: "inline-flex", alignItems: "center", gap: 6, fontFamily: MONO, fontSize: 11.5, padding: "6px 10px", borderRadius: 8, border: `1px solid ${c.border}`, background: c.card, color: c.dim, cursor: "pointer" }}>
            <Maximize2 size={13} /> Fit
          </button>
        )}
      </div>

      {(architecture.stack || []).length > 0 && (
        <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "8px 14px", borderBottom: `1px solid ${c.border}`, background: c.panel, flexShrink: 0, overflowX: "auto" }}>
          <span style={{ display: "inline-flex", alignItems: "center", gap: 6, flexShrink: 0, fontFamily: MONO, fontSize: 10, fontWeight: 600, letterSpacing: 0.5, textTransform: "uppercase", color: c.faint }}>
            <Box size={12} /> Stack
          </span>
          {(architecture.stack || []).map((s) => (
            <span key={s} style={{ flexShrink: 0, fontFamily: MONO, fontSize: 11, color: c.dim, background: c.panel2, border: `1px solid ${c.borderSoft}`, borderRadius: 6, padding: "3px 8px", whiteSpace: "nowrap" }}>{s}</span>
          ))}
        </div>
      )}

      {view === "diagram" &&
        (archNodes.length === 0 ? (
          <Empty c={c} text="No system diagram yet — services, datastores, externals and infra appear here (C4-style) as the AI designs the architecture." />
        ) : (
          <div className="flex min-h-0 flex-1">
            <div className="relative min-w-0 flex-1">
              <ReactFlow
                key={sig}
                nodes={nodes}
                edges={edges}
                onNodesChange={onNodesChange}
                onEdgesChange={onEdgesChange}
                onInit={(inst) => { instRef.current = inst; }}
                onNodeClick={(_, n) => setSelId(n.id)}
                onPaneClick={() => setSelId(null)}
                nodeTypes={nodeTypes}
                colorMode={mode}
                fitView
                fitViewOptions={{ padding: 0.2 }}
                minZoom={0.2}
                maxZoom={1.8}
                nodesConnectable={false}
                proOptions={{ hideAttribution: true }}
                style={{ background: c.bg }}
              >
                <Background variant={BackgroundVariant.Dots} color={alpha(c.dim, 0.35)} gap={22} size={1.4} />
                <Controls showInteractive={false} style={{ background: c.card, border: `1px solid ${c.border}`, borderRadius: 8, overflow: "hidden" }} />
                <MiniMap pannable zoomable nodeColor={(n) => kindMeta(c)[(n.data as unknown as ArchNode).kind]?.color || c.dim} maskColor={alpha(c.bg, 0.6)} style={{ background: c.panel, border: `1px solid ${c.border}`, borderRadius: 8 }} />
              </ReactFlow>
            </div>
            {selected && <NodeInspector node={selected} arch={architecture} c={c} onClose={() => setSelId(null)} />}
          </div>
        ))}
      {view === "decisions" && <DecisionsView arch={architecture} c={c} />}
      {view === "security" && <SecurityView arch={architecture} c={c} />}
    </div>
  );
}
