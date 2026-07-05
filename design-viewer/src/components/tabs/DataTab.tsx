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
import { ArrowRight, Database, Maximize2, Table as TableIcon } from "lucide-react";
import type { DataModel, Field } from "../../lib/types";
import { MONO, useTheme, type DarkTokens } from "../../lib/theme";
import { alpha } from "../../lib/utils";
import { Tag } from "../ui/panel";

// Node geometry — kept in sync between what we tell dagre and what the node renders.
const NODE_W = 264;
const HEADER_H = 40;
const ROW_H = 30;
const entityHeight = (fields: number) => HEADER_H + fields * ROW_H + 4;

type EntityNodeData = { name: string; fields: Field[] };
type EntityNode = Node<EntityNodeData, "entity">;

type View = "diagram" | "tables" | "relationships";

// One ER table: a header (table name) over a list of typed fields, with PK/FK
// tags and a required dot. Hidden handles on the sides let relationship edges
// attach. Reads the theme directly (it renders inside ThemeProvider).
function EntityNodeView({ data }: NodeProps<EntityNode>) {
  const { c } = useTheme();
  const handle = { background: c.accent, border: "none", width: 7, height: 7 };
  return (
    <div
      style={{
        width: NODE_W,
        background: c.card,
        border: `1px solid ${c.border}`,
        borderRadius: 10,
        overflow: "hidden",
        fontFamily: MONO,
        boxShadow: c.shadow,
      }}
    >
      <Handle type="target" position={Position.Left} style={handle} />
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          padding: "9px 14px",
          background: c.panel2,
          borderBottom: `1px solid ${c.border}`,
          color: c.text,
          fontSize: 13,
          fontWeight: 600,
        }}
      >
        <TableIcon size={14} color={c.accent} />
        {data.name}
      </div>
      {data.fields.map((f, fi) => (
        <div
          key={fi}
          style={{
            display: "flex",
            alignItems: "center",
            gap: 8,
            padding: "0 14px",
            height: ROW_H,
            fontSize: 12.5,
            borderTop: fi ? `1px solid ${c.borderSoft}` : "none",
          }}
        >
          <span
            title={f.required ? "required" : "optional"}
            style={{
              width: 6,
              height: 6,
              flexShrink: 0,
              borderRadius: 99,
              background: f.required ? c.amber : "transparent",
              border: f.required ? "none" : `1px solid ${c.border}`,
            }}
          />
          <span style={{ flex: 1, color: c.text }}>{f.name}</span>
          <span style={{ display: "flex", alignItems: "center", gap: 6 }}>
            {f.pk && <Tag label="PK" color={c.yellow} />}
            {f.fk && <Tag label="FK" color={c.accent} />}
            <span style={{ color: c.dim }}>{f.type}</span>
          </span>
        </div>
      ))}
      <Handle type="source" position={Position.Right} style={handle} />
    </div>
  );
}

const nodeTypes = { entity: EntityNodeView };

// Lay the entities out left-to-right with dagre, and turn relationships into
// labelled, arrow-headed edges. Positions are centre-based from dagre, so we
// shift to top-left for React Flow.
function buildGraph(dm: DataModel, c: DarkTokens): { nodes: EntityNode[]; edges: Edge[] } {
  const entities = dm.entities || [];
  const rels = dm.relationships || [];
  const names = new Set(entities.map((e) => e.name));

  const g = new dagre.graphlib.Graph();
  g.setGraph({ rankdir: "LR", nodesep: 36, ranksep: 96, marginx: 24, marginy: 24 });
  g.setDefaultEdgeLabel(() => ({}));
  entities.forEach((e) => g.setNode(e.name, { width: NODE_W, height: entityHeight(e.fields?.length || 0) }));
  rels.forEach((r) => {
    if (names.has(r.from) && names.has(r.to)) g.setEdge(r.from, r.to);
  });
  dagre.layout(g);

  const nodes: EntityNode[] = entities.map((e) => {
    const fields = e.fields || [];
    const h = entityHeight(fields.length);
    const p = g.node(e.name);
    return {
      id: e.name,
      type: "entity",
      position: { x: (p?.x ?? 0) - NODE_W / 2, y: (p?.y ?? 0) - h / 2 },
      data: { name: e.name, fields },
    };
  });

  const edges: Edge[] = rels
    .filter((r) => names.has(r.from) && names.has(r.to))
    .map((r, i) => ({
      id: `e${i}`,
      source: r.from,
      target: r.to,
      type: "smoothstep",
      label: r.label ? `${r.type} · ${r.label}` : r.type,
      labelStyle: { fill: c.dim, fontFamily: MONO, fontSize: 10 },
      labelBgStyle: { fill: c.card, fillOpacity: 0.95 },
      labelBgPadding: [6, 3] as [number, number],
      labelBgBorderRadius: 5,
      style: { stroke: alpha(c.accent, 0.7), strokeWidth: 1.5 },
      markerEnd: { type: MarkerType.ArrowClosed, color: alpha(c.accent, 0.7), width: 16, height: 16 },
    }));

  return { nodes, edges };
}

function Empty({ c, text }: { c: DarkTokens; text: string }) {
  return (
    <div className="flex h-full w-full items-center justify-center">
      <div className="flex flex-col items-center gap-2.5 text-center" style={{ color: c.faint, fontFamily: MONO }}>
        <Database size={28} />
        <div className="max-w-[300px] text-[13px] leading-relaxed">{text}</div>
      </div>
    </div>
  );
}

// A data-dictionary: every entity as a card listing ALL its fields, untruncated
// (so long enum/union types stay readable, unlike the fixed-width graph nodes).
function TablesView({ dm, c }: { dm: DataModel; c: DarkTokens }) {
  const entities = dm.entities || [];
  return (
    <section style={{ flex: 1, minHeight: 0, overflowY: "auto", padding: 24, background: c.bg }}>
      <div style={{ maxWidth: 1180, margin: "0 auto", display: "grid", gridTemplateColumns: "repeat(auto-fill,minmax(360px,1fr))", gap: 16 }}>
        {entities.map((e) => {
          const fields = e.fields || [];
          return (
            <div key={e.name} style={{ border: `1px solid ${c.border}`, borderRadius: 12, background: c.panel, overflow: "hidden" }}>
              <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "11px 14px", background: c.panel2, borderBottom: `1px solid ${c.border}` }}>
                <TableIcon size={14} color={c.accent} />
                <span style={{ fontFamily: MONO, fontSize: 13, fontWeight: 600, color: c.text, flex: 1 }}>{e.name}</span>
                <span style={{ fontFamily: MONO, fontSize: 11, color: c.faint }}>{fields.length} fields</span>
              </div>
              {fields.length === 0 ? (
                <div style={{ padding: "10px 14px", fontFamily: MONO, fontSize: 12, color: c.faint }}>no fields</div>
              ) : (
                fields.map((f, i) => (
                  <div key={i} style={{ display: "flex", alignItems: "flex-start", gap: 8, padding: "8px 14px", borderTop: i ? `1px solid ${c.borderSoft}` : "none" }}>
                    <span
                      title={f.required ? "required" : "optional"}
                      style={{ width: 6, height: 6, flexShrink: 0, marginTop: 5, borderRadius: 99, background: f.required ? c.amber : "transparent", border: f.required ? "none" : `1px solid ${c.border}` }}
                    />
                    <span style={{ fontFamily: MONO, fontSize: 12.5, color: c.text, minWidth: 84 }}>{f.name}</span>
                    <span style={{ display: "flex", alignItems: "center", gap: 6, flexWrap: "wrap", flex: 1, justifyContent: "flex-end" }}>
                      {f.pk && <Tag label="PK" color={c.yellow} />}
                      {f.fk && <Tag label="FK" color={c.accent} />}
                      <span style={{ fontFamily: MONO, fontSize: 12, color: c.dim, wordBreak: "break-word", textAlign: "right" }}>{f.type}</span>
                    </span>
                  </div>
                ))
              )}
            </div>
          );
        })}
      </div>
    </section>
  );
}

// Every foreign-key relationship as a row: from —[cardinality]→ to · label.
function RelationshipsView({ dm, c }: { dm: DataModel; c: DarkTokens }) {
  const rels = dm.relationships || [];
  if (!rels.length)
    return <Empty c={c} text="No relationships yet — foreign-key links between entities appear here as the AI defines them." />;
  return (
    <section style={{ flex: 1, minHeight: 0, overflowY: "auto", padding: 24, background: c.bg }}>
      <div style={{ maxWidth: 860, margin: "0 auto", display: "flex", flexDirection: "column", gap: 10 }}>
        {rels.map((r, i) => (
          <div key={i} style={{ display: "flex", alignItems: "center", gap: 12, border: `1px solid ${c.border}`, borderRadius: 11, background: c.panel, padding: "12px 16px" }}>
            <span style={{ fontFamily: MONO, fontSize: 13, color: c.text, fontWeight: 600 }}>{r.from}</span>
            <span style={{ display: "inline-flex", alignItems: "center", gap: 7, color: c.faint }}>
              <span style={{ fontFamily: MONO, fontSize: 10.5, color: c.accent2, background: c.accentSoft, border: `1px solid ${alpha(c.accent, 0.4)}`, borderRadius: 5, padding: "1px 7px" }}>{r.type}</span>
              <ArrowRight size={14} />
            </span>
            <span style={{ fontFamily: MONO, fontSize: 13, color: c.text, fontWeight: 600 }}>{r.to}</span>
            {r.label && <span style={{ marginLeft: "auto", fontSize: 12, color: c.dim }}>{r.label}</span>}
          </div>
        ))}
      </div>
    </section>
  );
}

export function DataTab({ dataModel }: { dataModel: DataModel }) {
  const { c, mode } = useTheme();
  const [view, setView] = useState<View>("diagram");
  const { nodes: laidNodes, edges: laidEdges } = useMemo(() => buildGraph(dataModel, c), [dataModel, c]);
  const [nodes, setNodes, onNodesChange] = useNodesState<EntityNode>(laidNodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState(laidEdges);
  const instRef = useRef<ReactFlowInstance<EntityNode, Edge> | null>(null);

  // Re-seed when the canvas changes (the AI live-edits the model); a structural
  // change remounts via `sig` so fitView re-frames, while same-shape edits just
  // refresh node contents and keep any manual drag.
  useEffect(() => {
    setNodes(laidNodes);
    setEdges(laidEdges);
  }, [laidNodes, laidEdges, setNodes, setEdges]);

  const sig = useMemo(
    () =>
      (dataModel.entities || []).map((e) => `${e.name}:${e.fields?.length || 0}`).join("|") +
      "#" +
      (dataModel.relationships?.length || 0),
    [dataModel]
  );

  const entities = dataModel.entities || [];
  const rels = dataModel.relationships || [];

  if (!entities.length) {
    return <Empty c={c} text="No entities yet — the data model appears here as the AI defines it." />;
  }

  const seg: { id: View; label: string; n?: number }[] = [
    { id: "diagram", label: "Diagram" },
    { id: "tables", label: "Tables", n: entities.length },
    { id: "relationships", label: "Relationships", n: rels.length },
  ];

  return (
    <div className="flex h-full w-full flex-col">
      {/* toolbar */}
      <div style={{ display: "flex", alignItems: "center", gap: 12, padding: "0 14px", minHeight: 46, borderBottom: `1px solid ${c.border}`, background: c.panel, flexShrink: 0, flexWrap: "wrap" }}>
        <Database size={15} color={c.accent} />
        <span style={{ fontFamily: MONO, fontSize: 13, color: c.text }}>Data model</span>
        <div style={{ display: "flex", gap: 2, background: c.panel2, border: `1px solid ${c.borderSoft}`, borderRadius: 8, padding: 2 }}>
          {seg.map((s) => (
            <button key={s.id} onClick={() => setView(s.id)} style={{ fontFamily: MONO, fontSize: 11.5, padding: "5px 11px", borderRadius: 6, border: "none", cursor: "pointer", background: view === s.id ? c.card : "transparent", color: view === s.id ? c.text : c.dim }}>
              {s.label}
              {s.n ? <span style={{ color: c.faint, marginLeft: 5 }}>{s.n}</span> : ""}
            </button>
          ))}
        </div>
        {view === "diagram" && (
          <button onClick={() => instRef.current?.fitView({ padding: 0.2, duration: 400 })} title="Fit to view" style={{ marginLeft: "auto", display: "inline-flex", alignItems: "center", gap: 6, fontFamily: MONO, fontSize: 11.5, padding: "6px 10px", borderRadius: 8, border: `1px solid ${c.border}`, background: c.card, color: c.dim, cursor: "pointer" }}>
            <Maximize2 size={13} /> Fit
          </button>
        )}
      </div>

      {view === "diagram" && (
        <div className="relative min-h-0 flex-1">
          <ReactFlow
            key={sig}
            nodes={nodes}
            edges={edges}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onInit={(inst) => {
              instRef.current = inst;
            }}
            nodeTypes={nodeTypes}
            colorMode={mode}
            fitView
            fitViewOptions={{ padding: 0.2 }}
            minZoom={0.2}
            maxZoom={1.8}
            nodesConnectable={false}
            elementsSelectable
            proOptions={{ hideAttribution: true }}
            style={{ background: c.bg }}
          >
            <Background variant={BackgroundVariant.Dots} color={alpha(c.dim, 0.35)} gap={22} size={1.4} />
            <Controls
              showInteractive={false}
              style={{ background: c.card, border: `1px solid ${c.border}`, borderRadius: 8, overflow: "hidden" }}
            />
            <MiniMap
              pannable
              zoomable
              nodeColor={() => c.panel2}
              nodeStrokeColor={() => alpha(c.accent, 0.5)}
              maskColor={alpha(c.bg, 0.6)}
              style={{ background: c.panel, border: `1px solid ${c.border}`, borderRadius: 8 }}
            />
          </ReactFlow>
        </div>
      )}
      {view === "tables" && <TablesView dm={dataModel} c={c} />}
      {view === "relationships" && <RelationshipsView dm={dataModel} c={c} />}
    </div>
  );
}
