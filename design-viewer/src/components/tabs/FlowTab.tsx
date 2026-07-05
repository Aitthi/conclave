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
import { stringify as toYaml } from "yaml";
import { Copy, Download, Eye, EyeOff, Filter, Layers, Maximize2, Monitor, Search, X } from "lucide-react";
import type { ApiDoc, ApiOperation, ApiParameter, ApiSchema, HttpMethod, Screen } from "../../lib/types";
import { HTTP_METHODS } from "../../lib/types";
import { MONO, useTheme, type DarkTokens } from "../../lib/theme";
import { alpha } from "../../lib/utils";

// ── colours per HTTP method ─────────────────────────────────────────────────
function methodColor(m: string, c: DarkTokens): string {
  switch (m.toLowerCase()) {
    case "get": return c.green;
    case "post": return c.accent;
    case "put": return c.amber;
    case "patch": return "#a78bfa";
    case "delete": return c.red;
    default: return c.dim;
  }
}

function MethodBadge({ method, c }: { method: string; c: DarkTokens }) {
  const col = methodColor(method, c);
  return (
    <span
      style={{
        fontFamily: MONO,
        fontSize: 10,
        fontWeight: 700,
        letterSpacing: 0.5,
        color: col,
        background: alpha(col, 0.14),
        border: `1px solid ${alpha(col, 0.4)}`,
        borderRadius: 5,
        padding: "1px 6px",
      }}
    >
      {method.toUpperCase()}
    </span>
  );
}

// ── flattened view of the OpenAPI paths ─────────────────────────────────────
interface Op {
  id: string;
  path: string;
  method: HttpMethod;
  op: ApiOperation;
}
function flattenOps(api: ApiDoc): Op[] {
  const out: Op[] = [];
  const paths = api.paths || {};
  for (const path of Object.keys(paths)) {
    const item = paths[path] || {};
    for (const method of HTTP_METHODS) {
      const op = item[method];
      if (op) out.push({ id: `${method.toUpperCase()} ${path}`, path, method, op });
    }
  }
  return out;
}

// ── React Flow nodes ─────────────────────────────────────────────────────────
const ROUTE_W = 252;
const ROUTE_H = 76;
const MW_W = 156;
const MW_H = 46;
const SCREEN_W = 172;
const SCREEN_H = 48;

type RouteData = { method: string; path: string; summary?: string; mw: number; params: number; body: boolean };
type RouteNode = Node<RouteData, "route">;
type MwNode = Node<{ name: string }, "middleware">;
type ScreenNode = Node<{ id: string; title: string }, "screen">;

function portStyle(col: string, c: DarkTokens) {
  return { width: 9, height: 9, background: c.card, border: `2px solid ${col}`, borderRadius: 99 };
}

function metaChip(c: DarkTokens, label: string) {
  return (
    <span
      key={label}
      style={{ fontFamily: MONO, fontSize: 9.5, color: c.faint, background: c.panel2, border: `1px solid ${c.borderSoft}`, borderRadius: 5, padding: "1px 6px" }}
    >
      {label}
    </span>
  );
}

function RouteNodeView({ data, selected }: NodeProps<RouteNode>) {
  const { c } = useTheme();
  const [hover, setHover] = useState(false);
  const col = methodColor(data.method, c);
  const active = selected || hover;
  return (
    <div
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        width: ROUTE_W,
        borderRadius: 12,
        overflow: "hidden",
        fontFamily: MONO,
        background: c.card,
        border: `1px solid ${selected ? col : active ? alpha(col, 0.6) : c.border}`,
        boxShadow: selected
          ? `0 0 0 1px ${col}, 0 10px 28px ${alpha(col, 0.22)}`
          : c.shadow,
        transform: active ? "translateY(-1px)" : "none",
        transition: "box-shadow .14s ease, transform .14s ease, border-color .14s ease",
        cursor: "pointer",
      }}
    >
      <Handle type="target" position={Position.Left} style={portStyle(col, c)} />
      <div style={{ display: "flex", alignItems: "center", gap: 7, padding: "8px 11px", background: alpha(col, 0.1), borderBottom: `1px solid ${c.borderSoft}` }}>
        <MethodBadge method={data.method} c={c} />
        <span style={{ color: c.text, fontSize: 12.5, fontWeight: 600, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{data.path}</span>
      </div>
      <div style={{ padding: "8px 11px" }}>
        {data.summary && (
          <div style={{ color: c.dim, fontSize: 11, marginBottom: 6, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{data.summary}</div>
        )}
        <div style={{ display: "flex", gap: 5 }}>
          {data.mw > 0 && metaChip(c, `${data.mw} mw`)}
          {data.params > 0 && metaChip(c, `${data.params} params`)}
          {data.body && metaChip(c, "body")}
        </div>
      </div>
      <Handle type="source" position={Position.Right} style={portStyle(col, c)} />
    </div>
  );
}

function MiddlewareNodeView({ data, selected }: NodeProps<MwNode>) {
  const { c } = useTheme();
  const [hover, setHover] = useState(false);
  return (
    <div
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        width: MW_W,
        display: "flex",
        alignItems: "center",
        gap: 8,
        background: c.panel2,
        border: `1px dashed ${selected || hover ? c.dim : alpha(c.dim, 0.5)}`,
        borderRadius: 999,
        padding: "10px 14px",
        fontFamily: MONO,
        fontSize: 12,
        color: c.text,
        boxShadow: hover ? c.shadow : "none",
        transition: "box-shadow .14s ease, border-color .14s ease",
        cursor: "pointer",
      }}
    >
      <Filter size={13} color={c.dim} />
      <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{data.name}</span>
      <Handle type="source" position={Position.Right} style={portStyle(c.dim, c)} />
    </div>
  );
}

function ScreenNodeView({ data, selected }: NodeProps<ScreenNode>) {
  const { c } = useTheme();
  const [hover, setHover] = useState(false);
  const active = selected || hover;
  return (
    <div
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        width: SCREEN_W,
        display: "flex",
        alignItems: "center",
        gap: 8,
        background: alpha(c.accent, 0.1),
        border: `1px solid ${active ? c.accent : alpha(c.accent, 0.45)}`,
        borderRadius: 10,
        padding: "11px 13px",
        fontFamily: MONO,
        fontSize: 12,
        color: c.text,
        boxShadow: active ? `0 8px 20px ${alpha(c.accent, 0.2)}` : "none",
        transform: active ? "translateY(-1px)" : "none",
        transition: "box-shadow .14s ease, transform .14s ease, border-color .14s ease",
        cursor: "pointer",
      }}
    >
      <Monitor size={13} color={c.accent} />
      <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{data.title}</span>
      <Handle type="source" position={Position.Right} style={portStyle(c.accent, c)} />
    </div>
  );
}

const nodeTypes = { route: RouteNodeView, middleware: MiddlewareNodeView, screen: ScreenNodeView };

// ── graph build + dagre layout ───────────────────────────────────────────────
interface GraphOpts {
  showScreens: boolean;
  showMiddleware: boolean;
  hiddenMethods: Set<string>;
}
function buildGraph(api: ApiDoc, screens: Screen[], opts: GraphOpts, c: DarkTokens): { nodes: Node[]; edges: Edge[] } {
  const ops = flattenOps(api).filter((o) => !opts.hiddenMethods.has(o.method));
  const referenced = opts.showMiddleware ? ops.flatMap((o) => o.op["x-middleware"] || []) : [];
  const declared = opts.showMiddleware ? (api["x-middleware"] || []).map((m) => m.name) : [];
  const mwNames = Array.from(new Set([...declared, ...referenced]));
  const titleOf = new Map(screens.map((s) => [s.id, s.title || s.id]));
  const screenRefs = opts.showScreens ? Array.from(new Set(ops.flatMap((o) => o.op["x-screens"] || []))) : [];

  const g = new dagre.graphlib.Graph();
  g.setGraph({ rankdir: "LR", nodesep: 26, ranksep: 130, marginx: 28, marginy: 28 });
  g.setDefaultEdgeLabel(() => ({}));
  screenRefs.forEach((s) => g.setNode(`screen:${s}`, { width: SCREEN_W, height: SCREEN_H }));
  mwNames.forEach((m) => g.setNode(`mw:${m}`, { width: MW_W, height: MW_H }));
  ops.forEach((o) => g.setNode(o.id, { width: ROUTE_W, height: ROUTE_H }));

  const edgeDefs: { from: string; to: string; kind: "mw" | "screen" }[] = [];
  ops.forEach((o) => {
    if (opts.showMiddleware)
      (o.op["x-middleware"] || []).forEach((m) => {
        g.setEdge(`mw:${m}`, o.id);
        edgeDefs.push({ from: `mw:${m}`, to: o.id, kind: "mw" });
      });
    if (opts.showScreens)
      (o.op["x-screens"] || []).forEach((s) => {
        g.setEdge(`screen:${s}`, o.id);
        edgeDefs.push({ from: `screen:${s}`, to: o.id, kind: "screen" });
      });
  });
  dagre.layout(g);

  const at = (id: string, w: number, h: number) => {
    const p = g.node(id);
    return { x: (p?.x ?? 0) - w / 2, y: (p?.y ?? 0) - h / 2 };
  };

  const nodes: Node[] = [];
  screenRefs.forEach((s) =>
    nodes.push({ id: `screen:${s}`, type: "screen", position: at(`screen:${s}`, SCREEN_W, SCREEN_H), data: { id: s, title: titleOf.get(s) || s } })
  );
  mwNames.forEach((m) => nodes.push({ id: `mw:${m}`, type: "middleware", position: at(`mw:${m}`, MW_W, MW_H), data: { name: m } }));
  ops.forEach((o) =>
    nodes.push({
      id: o.id,
      type: "route",
      position: at(o.id, ROUTE_W, ROUTE_H),
      data: { method: o.method, path: o.path, summary: o.op.summary, mw: (o.op["x-middleware"] || []).length, params: (o.op.parameters || []).length, body: !!o.op.requestBody },
    })
  );

  const edges: Edge[] = edgeDefs.map((e, i) =>
    e.kind === "screen"
      ? {
          id: `se${i}`,
          source: e.from,
          target: e.to,
          type: "smoothstep",
          animated: true,
          pathOptions: { borderRadius: 14 },
          style: { stroke: alpha(c.accent, 0.6), strokeWidth: 1.6 },
          markerEnd: { type: MarkerType.ArrowClosed, color: alpha(c.accent, 0.65), width: 15, height: 15 },
        }
      : {
          id: `me${i}`,
          source: e.from,
          target: e.to,
          type: "smoothstep",
          pathOptions: { borderRadius: 14 },
          style: { stroke: alpha(c.dim, 0.5), strokeWidth: 1.3, strokeDasharray: "5 4" },
          markerEnd: { type: MarkerType.ArrowClosed, color: alpha(c.dim, 0.55), width: 13, height: 13 },
        }
  );

  return { nodes, edges };
}

// ── schema rendering (read-only) ─────────────────────────────────────────────
function schemaLabel(s?: ApiSchema): string {
  if (!s) return "any";
  if (s.$ref) return s.$ref.split("/").pop() || "ref";
  if (s.type === "array") return `${schemaLabel(s.items)}[]`;
  return (s.type || "object") + (s.format ? ` (${s.format})` : "");
}

function SchemaView({ schema, c, depth = 0 }: { schema?: ApiSchema; c: DarkTokens; depth?: number }) {
  if (!schema) return <span style={{ color: c.faint }}>—</span>;
  if (schema.properties) {
    const req = new Set(schema.required || []);
    return (
      <div style={{ display: "flex", flexDirection: "column", gap: 4, paddingLeft: depth ? 12 : 0, borderLeft: depth ? `1px solid ${c.borderSoft}` : "none" }}>
        {Object.keys(schema.properties).map((k) => {
          const f = schema.properties![k];
          return (
            <div key={k} style={{ fontFamily: MONO, fontSize: 12 }}>
              <div style={{ display: "flex", alignItems: "baseline", gap: 8 }}>
                <span style={{ color: c.text }}>{k}</span>
                <span style={{ color: c.accent, fontSize: 11 }}>{schemaLabel(f)}</span>
                {req.has(k) && <span style={{ color: c.amber, fontSize: 10 }}>required</span>}
              </div>
              {f.description && <div style={{ color: c.faint, fontSize: 11 }}>{f.description}</div>}
              {f.properties && <SchemaView schema={f} c={c} depth={depth + 1} />}
              {f.items?.properties && <SchemaView schema={f.items} c={c} depth={depth + 1} />}
            </div>
          );
        })}
      </div>
    );
  }
  return <span style={{ color: c.accent, fontFamily: MONO, fontSize: 12 }}>{schemaLabel(schema)}</span>;
}

function CodeBlock({ value, c }: { value: unknown; c: DarkTokens }) {
  return (
    <pre style={{ margin: 0, background: c.bg, border: `1px solid ${c.borderSoft}`, borderRadius: 8, padding: 12, fontFamily: MONO, fontSize: 11.5, color: c.dim, overflow: "auto", maxHeight: 220 }}>
      {typeof value === "string" ? value : JSON.stringify(value, null, 2)}
    </pre>
  );
}

// ── Postman-style inspector ───────────────────────────────────────────────────
const TABS = ["Params", "Headers", "Body", "Responses"] as const;
type Tab = (typeof TABS)[number];

function ParamTable({ params, c }: { params: ApiParameter[]; c: DarkTokens }) {
  if (!params.length) return <div style={{ color: c.faint, fontSize: 12, fontFamily: MONO }}>None</div>;
  return (
    <div style={{ display: "flex", flexDirection: "column" }}>
      {params.map((p, i) => (
        <div key={i} style={{ padding: "8px 0", borderTop: i ? `1px solid ${c.borderSoft}` : "none", fontFamily: MONO }}>
          <div style={{ display: "flex", alignItems: "baseline", gap: 8 }}>
            <span style={{ color: c.text, fontSize: 12.5 }}>{p.name}</span>
            <span style={{ color: c.accent, fontSize: 11 }}>{schemaLabel(p.schema)}</span>
            {p.required && <span style={{ color: c.amber, fontSize: 10 }}>required</span>}
          </div>
          {p.description && <div style={{ color: c.faint, fontSize: 11, marginTop: 2 }}>{p.description}</div>}
          {p.example !== undefined && <div style={{ color: c.dim, fontSize: 11, marginTop: 2 }}>e.g. {String(p.example)}</div>}
        </div>
      ))}
    </div>
  );
}

function Inspector({ op, path, method, c, onClose, screens }: { op: ApiOperation; path: string; method: string; c: DarkTokens; onClose: () => void; screens: Screen[] }) {
  const [tab, setTab] = useState<Tab>("Params");
  const params = op.parameters || [];
  const pathQuery = params.filter((p) => p.in === "path" || p.in === "query");
  const headers = params.filter((p) => p.in === "header");
  const body = op.requestBody;
  const bodyMedia = body?.content ? Object.entries(body.content) : [];
  const responses = op.responses ? Object.entries(op.responses) : [];
  const mw = op["x-middleware"] || [];
  const titleOf = new Map(screens.map((s) => [s.id, s.title || s.id]));
  const usedBy = op["x-screens"] || [];

  return (
    <aside className="flex h-full flex-col" style={{ width: 400, borderLeft: `1px solid ${c.border}`, background: c.panel }}>
      <div style={{ padding: "14px 16px", borderBottom: `1px solid ${c.border}` }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <MethodBadge method={method} c={c} />
          <span style={{ fontFamily: MONO, fontSize: 13, color: c.text, flex: 1, wordBreak: "break-all" }}>{path}</span>
          <button onClick={onClose} style={{ color: c.dim, background: "transparent", border: "none", cursor: "pointer", padding: 2 }}>
            <X size={16} />
          </button>
        </div>
        {op.summary && <div style={{ color: c.dim, fontSize: 12, marginTop: 6 }}>{op.summary}</div>}
        {mw.length > 0 && (
          <div style={{ display: "flex", flexWrap: "wrap", gap: 6, marginTop: 10 }}>
            {mw.map((m) => (
              <span key={m} style={{ display: "inline-flex", alignItems: "center", gap: 4, fontFamily: MONO, fontSize: 10.5, color: c.dim, background: c.panel2, border: `1px solid ${c.borderSoft}`, borderRadius: 999, padding: "2px 8px" }}>
                <Filter size={10} /> {m}
              </span>
            ))}
          </div>
        )}
        {usedBy.length > 0 && (
          <div style={{ marginTop: 10 }}>
            <div style={{ fontFamily: MONO, fontSize: 10, letterSpacing: 0.6, color: c.faint, marginBottom: 5 }}>USED BY</div>
            <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
              {usedBy.map((s) => (
                <span key={s} style={{ display: "inline-flex", alignItems: "center", gap: 4, fontFamily: MONO, fontSize: 10.5, color: c.accent, background: alpha(c.accent, 0.1), border: `1px solid ${alpha(c.accent, 0.4)}`, borderRadius: 999, padding: "2px 8px" }}>
                  <Monitor size={10} /> {titleOf.get(s) || s}
                </span>
              ))}
            </div>
          </div>
        )}
      </div>

      <div style={{ display: "flex", gap: 2, padding: "0 8px", borderBottom: `1px solid ${c.border}` }}>
        {TABS.map((t) => {
          const count = t === "Params" ? pathQuery.length : t === "Headers" ? headers.length : t === "Responses" ? responses.length : bodyMedia.length;
          const activeTab = tab === t;
          return (
            <button key={t} onClick={() => setTab(t)} style={{ fontFamily: MONO, fontSize: 12, padding: "10px 10px", background: "transparent", border: "none", borderBottom: `2px solid ${activeTab ? c.accent : "transparent"}`, color: activeTab ? c.text : c.faint, cursor: "pointer" }}>
              {t}
              {count > 0 && <span style={{ color: c.faint, marginLeft: 5 }}>{count}</span>}
            </button>
          );
        })}
      </div>

      <div style={{ flex: 1, overflow: "auto", padding: 16 }}>
        {tab === "Params" && <ParamTable params={pathQuery} c={c} />}
        {tab === "Headers" && <ParamTable params={headers} c={c} />}
        {tab === "Body" &&
          (bodyMedia.length === 0 ? (
            <div style={{ color: c.faint, fontSize: 12, fontFamily: MONO }}>No request body</div>
          ) : (
            bodyMedia.map(([ct, media]) => (
              <div key={ct} style={{ display: "flex", flexDirection: "column", gap: 8, marginBottom: 14 }}>
                <div style={{ fontFamily: MONO, fontSize: 11, color: c.faint }}>
                  {ct}
                  {body?.required && <span style={{ color: c.amber, marginLeft: 8 }}>required</span>}
                </div>
                <SchemaView schema={media.schema} c={c} />
                {media.example !== undefined && <CodeBlock value={media.example} c={c} />}
              </div>
            ))
          ))}
        {tab === "Responses" &&
          (responses.length === 0 ? (
            <div style={{ color: c.faint, fontSize: 12, fontFamily: MONO }}>None</div>
          ) : (
            responses.map(([code, res]) => {
              const ok = code.startsWith("2");
              const col = ok ? c.green : code.startsWith("4") || code.startsWith("5") ? c.red : c.amber;
              const media = res.content ? Object.entries(res.content) : [];
              return (
                <div key={code} style={{ marginBottom: 14 }}>
                  <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                    <span style={{ fontFamily: MONO, fontSize: 12, fontWeight: 700, color: col }}>{code}</span>
                    <span style={{ color: c.dim, fontSize: 12 }}>{res.description}</span>
                  </div>
                  {media.map(([ct, m]) => (
                    <div key={ct} style={{ marginTop: 8 }}>
                      <div style={{ fontFamily: MONO, fontSize: 11, color: c.faint, marginBottom: 6 }}>{ct}</div>
                      <SchemaView schema={m.schema} c={c} />
                      {m.example !== undefined && <CodeBlock value={m.example} c={c} />}
                    </div>
                  ))}
                </div>
              );
            })
          ))}
      </div>
    </aside>
  );
}

// Screen inspector — which APIs this screen calls, and the middleware each goes through.
function ScreenInspector({ title, ops, c, onClose, onPick }: { title: string; ops: Op[]; c: DarkTokens; onClose: () => void; onPick: (path: string, method: HttpMethod) => void }) {
  return (
    <aside className="flex h-full flex-col" style={{ width: 400, borderLeft: `1px solid ${c.border}`, background: c.panel }}>
      <div style={{ padding: "14px 16px", borderBottom: `1px solid ${c.border}`, display: "flex", alignItems: "center", gap: 8 }}>
        <Monitor size={14} color={c.accent} />
        <span style={{ fontFamily: MONO, fontSize: 13, color: c.text, flex: 1 }}>{title}</span>
        <button onClick={onClose} style={{ color: c.dim, background: "transparent", border: "none", cursor: "pointer", padding: 2 }}>
          <X size={16} />
        </button>
      </div>
      <div style={{ flex: 1, overflow: "auto", padding: 16 }}>
        <div style={{ fontFamily: MONO, fontSize: 10, letterSpacing: 0.6, color: c.faint, marginBottom: 8 }}>CALLS {ops.length} API{ops.length === 1 ? "" : "S"}</div>
        {ops.length === 0 ? (
          <div style={{ color: c.faint, fontSize: 12, fontFamily: MONO }}>No API calls declared for this screen.</div>
        ) : (
          ops.map((o) => (
            <button key={o.id} onClick={() => onPick(o.path, o.method)} style={{ display: "block", width: "100%", textAlign: "left", marginBottom: 8, padding: "9px 11px", borderRadius: 8, border: `1px solid ${c.border}`, background: c.card, cursor: "pointer" }}>
              <div style={{ display: "flex", alignItems: "center", gap: 7 }}>
                <MethodBadge method={o.method} c={c} />
                <span style={{ fontFamily: MONO, fontSize: 12.5, color: c.text }}>{o.path}</span>
              </div>
              {(o.op["x-middleware"] || []).length > 0 && (
                <div style={{ display: "flex", flexWrap: "wrap", gap: 5, marginTop: 7 }}>
                  {(o.op["x-middleware"] || []).map((m) => (
                    <span key={m} style={{ display: "inline-flex", alignItems: "center", gap: 3, fontFamily: MONO, fontSize: 10, color: c.dim, background: c.panel2, border: `1px solid ${c.borderSoft}`, borderRadius: 999, padding: "1px 7px" }}>
                      <Filter size={9} /> {m}
                    </span>
                  ))}
                </div>
              )}
            </button>
          ))
        )}
      </div>
    </aside>
  );
}

// ── OpenAPI export ────────────────────────────────────────────────────────────
function toOpenApi(api: ApiDoc): Record<string, unknown> {
  return {
    openapi: "3.0.3",
    info: api.info || { title: "API", version: "1.0.0" },
    ...(api.servers ? { servers: api.servers } : {}),
    ...(api["x-middleware"] ? { "x-middleware": api["x-middleware"] } : {}),
    paths: api.paths || {},
  };
}

function ExportModal({ api, c, onClose }: { api: ApiDoc; c: DarkTokens; onClose: () => void }) {
  const [fmt, setFmt] = useState<"json" | "yaml">("yaml");
  const [copied, setCopied] = useState(false);
  const doc = useMemo(() => toOpenApi(api), [api]);
  const text = useMemo(() => (fmt === "json" ? JSON.stringify(doc, null, 2) : toYaml(doc)), [doc, fmt]);
  const copy = () => {
    navigator.clipboard?.writeText(text).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1400);
    });
  };
  return (
    <div onClick={onClose} className="absolute inset-0 z-50 flex items-center justify-center p-8" style={{ background: alpha("#000000", 0.6) }}>
      <div onClick={(e) => e.stopPropagation()} className="flex max-h-full w-full max-w-[760px] flex-col overflow-hidden rounded-xl" style={{ background: c.panel, border: `1px solid ${c.border}` }}>
        <div style={{ display: "flex", alignItems: "center", gap: 10, padding: "12px 14px", borderBottom: `1px solid ${c.border}` }}>
          <span style={{ fontFamily: MONO, fontSize: 13, color: c.text, flex: 1 }}>Export · OpenAPI 3</span>
          <div style={{ display: "flex", border: `1px solid ${c.border}`, borderRadius: 7, overflow: "hidden" }}>
            {(["yaml", "json"] as const).map((f) => (
              <button key={f} onClick={() => setFmt(f)} style={{ fontFamily: MONO, fontSize: 11.5, padding: "5px 11px", border: "none", cursor: "pointer", background: fmt === f ? c.accent : "transparent", color: fmt === f ? "#06121b" : c.dim }}>
                {f.toUpperCase()}
              </button>
            ))}
          </div>
          <button onClick={copy} style={{ display: "inline-flex", alignItems: "center", gap: 6, fontFamily: MONO, fontSize: 11.5, padding: "5px 11px", borderRadius: 7, border: `1px solid ${c.border}`, background: c.card, color: c.text, cursor: "pointer" }}>
            <Copy size={13} /> {copied ? "Copied" : "Copy"}
          </button>
          <button onClick={onClose} style={{ color: c.dim, background: "transparent", border: "none", cursor: "pointer", padding: 4 }}>
            <X size={16} />
          </button>
        </div>
        <pre style={{ margin: 0, flex: 1, overflow: "auto", padding: 16, fontFamily: MONO, fontSize: 12, color: c.dim, background: c.bg }}>{text}</pre>
      </div>
    </div>
  );
}

// ── left rail: legend, layer toggles, method filter, route list ───────────────
function ToggleRow({ on, label, count, onClick, c }: { on: boolean; label: string; count: number; onClick: () => void; c: DarkTokens }) {
  return (
    <button onClick={onClick} style={{ display: "flex", alignItems: "center", gap: 8, width: "100%", padding: "6px 8px", borderRadius: 7, border: "none", background: "transparent", color: on ? c.text : c.faint, cursor: "pointer", fontFamily: MONO, fontSize: 12 }}>
      {on ? <Eye size={13} /> : <EyeOff size={13} />}
      <span style={{ flex: 1, textAlign: "left" }}>{label}</span>
      <span style={{ color: c.faint, fontSize: 11 }}>{count}</span>
    </button>
  );
}

function LeftRail({
  ops, api, screenCount, hiddenMethods, toggleMethod, showMiddleware, setShowMiddleware, showScreens, setShowScreens, search, setSearch, sel, onPick, c,
}: {
  ops: Op[];
  api: ApiDoc;
  screenCount: number;
  hiddenMethods: Set<string>;
  toggleMethod: (m: string) => void;
  showMiddleware: boolean;
  setShowMiddleware: (v: boolean) => void;
  showScreens: boolean;
  setShowScreens: (v: boolean) => void;
  search: string;
  setSearch: (s: string) => void;
  sel: Sel;
  onPick: (path: string, method: HttpMethod) => void;
  c: DarkTokens;
}) {
  const presentMethods = useMemo(() => Array.from(new Set(ops.map((o) => o.method))), [ops]);
  const q = search.trim().toLowerCase();
  const listed = ops.filter((o) => !q || o.path.toLowerCase().includes(q) || (o.op.summary || "").toLowerCase().includes(q));
  const groups = useMemo(() => {
    const m = new Map<string, Op[]>();
    listed.forEach((o) => {
      const tag = o.op.tags?.[0] || "Other";
      (m.get(tag) || m.set(tag, []).get(tag)!).push(o);
    });
    return Array.from(m.entries());
  }, [listed]);
  const head = (t: string) => <div style={{ fontFamily: MONO, fontSize: 10, letterSpacing: 0.7, color: c.faint, margin: "14px 0 6px" }}>{t}</div>;

  return (
    <aside className="flex h-full flex-col" style={{ width: 236, borderRight: `1px solid ${c.border}`, background: c.panel }}>
      <div style={{ flex: 1, overflow: "auto", padding: "10px 12px" }}>
        {head("LAYERS")}
        <ToggleRow on={showMiddleware} label="Middleware" count={(api["x-middleware"] || []).length} onClick={() => setShowMiddleware(!showMiddleware)} c={c} />
        <ToggleRow on={showScreens} label="Screens" count={screenCount} onClick={() => setShowScreens(!showScreens)} c={c} />

        {head("METHODS")}
        <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
          {presentMethods.map((m) => {
            const col = methodColor(m, c);
            const off = hiddenMethods.has(m);
            return (
              <button key={m} onClick={() => toggleMethod(m)} style={{ display: "inline-flex", alignItems: "center", gap: 5, fontFamily: MONO, fontSize: 10, fontWeight: 700, padding: "2px 7px", borderRadius: 5, cursor: "pointer", color: off ? c.faint : col, background: off ? "transparent" : alpha(col, 0.14), border: `1px solid ${off ? c.borderSoft : alpha(col, 0.4)}`, opacity: off ? 0.55 : 1 }}>
                {m.toUpperCase()}
              </button>
            );
          })}
        </div>

        {head("ROUTES")}
        <div style={{ display: "flex", alignItems: "center", gap: 6, background: c.bg, border: `1px solid ${c.border}`, borderRadius: 7, padding: "5px 8px", marginBottom: 8 }}>
          <Search size={12} color={c.faint} />
          <input value={search} onChange={(e) => setSearch(e.target.value)} placeholder="Filter routes…" style={{ flex: 1, minWidth: 0, background: "transparent", border: "none", outline: "none", color: c.text, fontFamily: MONO, fontSize: 11.5 }} />
        </div>
        {groups.map(([tag, list]) => (
          <div key={tag} style={{ marginBottom: 8 }}>
            <div style={{ fontFamily: MONO, fontSize: 10, color: c.faint, margin: "6px 2px 4px" }}>{tag}</div>
            {list.map((o) => {
              const on = sel?.kind === "route" && sel.path === o.path && sel.method === o.method;
              return (
                <button key={o.id} onClick={() => onPick(o.path, o.method)} style={{ display: "flex", alignItems: "center", gap: 7, width: "100%", textAlign: "left", padding: "6px 8px", borderRadius: 7, border: "none", background: on ? alpha(c.accent, 0.12) : "transparent", cursor: "pointer", marginBottom: 1 }}>
                  <MethodBadge method={o.method} c={c} />
                  <span style={{ fontFamily: MONO, fontSize: 11.5, color: on ? c.text : c.dim, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{o.path}</span>
                </button>
              );
            })}
          </div>
        ))}
        {listed.length === 0 && <div style={{ color: c.faint, fontFamily: MONO, fontSize: 11, padding: "6px 2px" }}>No matches.</div>}
      </div>
    </aside>
  );
}

// ── the tab ───────────────────────────────────────────────────────────────────
type Sel = { kind: "route"; path: string; method: HttpMethod } | { kind: "screen"; id: string } | null;

type FlowView = "graph" | "endpoints" | "middleware";

function MetaChip({ children, c }: { children: React.ReactNode; c: DarkTokens }) {
  return (
    <span style={{ fontFamily: MONO, fontSize: 10.5, color: c.faint, background: c.panel2, border: `1px solid ${c.borderSoft}`, borderRadius: 5, padding: "1px 7px" }}>
      {children}
    </span>
  );
}

// A readable API reference: routes grouped by tag, each with method, path,
// summary, middleware and a params/body/responses summary. Clicking a row jumps
// to the graph and focuses that route.
function EndpointsView({ ops, c, onPick, setView }: { ops: Op[]; c: DarkTokens; onPick: (p: string, m: HttpMethod) => void; setView: (v: FlowView) => void }) {
  const groups: Record<string, Op[]> = {};
  ops.forEach((o) => {
    const k = o.op.tags?.[0] || "Other";
    (groups[k] ||= []).push(o);
  });
  return (
    <section style={{ flex: 1, minHeight: 0, overflowY: "auto", padding: 24, background: c.bg }}>
      <div style={{ maxWidth: 960, margin: "0 auto", display: "flex", flexDirection: "column", gap: 22 }}>
        {Object.entries(groups).map(([tag, list]) => (
          <div key={tag}>
            <div style={{ fontFamily: MONO, fontSize: 11, fontWeight: 600, letterSpacing: 0.5, textTransform: "uppercase", color: c.faint, marginBottom: 10 }}>{tag}</div>
            <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
              {list.map((o) => {
                const mw = o.op["x-middleware"] || [];
                const params = (o.op.parameters || []).length;
                const resps = Object.keys(o.op.responses || {}).length;
                return (
                  <button
                    key={o.id}
                    onClick={() => {
                      setView("graph");
                      onPick(o.path, o.method);
                    }}
                    style={{ textAlign: "left", display: "flex", flexDirection: "column", gap: 9, border: `1px solid ${c.border}`, borderRadius: 11, background: c.panel, padding: "12px 14px", cursor: "pointer" }}
                  >
                    <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                      <MethodBadge method={o.method} c={c} />
                      <span style={{ fontFamily: MONO, fontSize: 13, color: c.text }}>{o.path}</span>
                      {o.op.summary && <span style={{ fontSize: 12.5, color: c.dim, marginLeft: "auto", textAlign: "right" }}>{o.op.summary}</span>}
                    </div>
                    <div style={{ display: "flex", alignItems: "center", gap: 6, flexWrap: "wrap" }}>
                      {mw.map((m) => (
                        <span key={m} style={{ fontFamily: MONO, fontSize: 10.5, color: c.amber, background: alpha(c.amber, 0.12), border: `1px solid ${alpha(c.amber, 0.35)}`, borderRadius: 5, padding: "1px 7px" }}>{m}</span>
                      ))}
                      <MetaChip c={c}>{params} params</MetaChip>
                      {o.op.requestBody && <MetaChip c={c}>body</MetaChip>}
                      {resps > 0 && <MetaChip c={c}>{resps} responses</MetaChip>}
                    </div>
                  </button>
                );
              })}
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}

// Each middleware as a card: name, description, and the routes that run through it.
function MiddlewareView({ api, ops, c }: { api: ApiDoc; ops: Op[]; c: DarkTokens }) {
  const declared = api["x-middleware"] || [];
  if (!declared.length)
    return (
      <div className="flex h-full w-full items-center justify-center">
        <div className="flex flex-col items-center gap-2.5 text-center" style={{ color: c.faint, fontFamily: MONO }}>
          <Filter size={26} />
          <div className="max-w-[300px] text-[13px] leading-relaxed">No middleware declared — auth, rate-limit, CORS and friends appear here as the AI adds them.</div>
        </div>
      </div>
    );
  return (
    <section style={{ flex: 1, minHeight: 0, overflowY: "auto", padding: 24, background: c.bg }}>
      <div style={{ maxWidth: 960, margin: "0 auto", display: "flex", flexDirection: "column", gap: 16 }}>
        {declared.map((m) => {
          const users = ops.filter((o) => (o.op["x-middleware"] || []).includes(m.name));
          return (
            <div key={m.name} style={{ border: `1px solid ${c.border}`, borderRadius: 12, background: c.panel, overflow: "hidden" }}>
              <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "12px 16px", borderBottom: `1px solid ${c.border}` }}>
                <span style={{ width: 6, height: 6, borderRadius: 99, background: c.amber }} />
                <span style={{ fontFamily: MONO, fontSize: 13, fontWeight: 600, color: c.text, flex: 1 }}>{m.name}</span>
                <span style={{ fontFamily: MONO, fontSize: 11, color: c.faint }}>{users.length} routes</span>
              </div>
              {m.description && <p style={{ margin: 0, padding: "12px 16px 0", fontSize: 12.5, color: c.dim, lineHeight: 1.55 }}>{m.description}</p>}
              <div style={{ display: "flex", flexWrap: "wrap", gap: 6, padding: "12px 16px" }}>
                {users.length === 0 ? (
                  <span style={{ fontFamily: MONO, fontSize: 12, color: c.faint }}>not attached to any route</span>
                ) : (
                  users.map((o) => (
                    <span key={o.id} style={{ display: "inline-flex", alignItems: "center", gap: 7, border: `1px solid ${c.border}`, borderRadius: 8, background: c.bg, padding: "4px 8px" }}>
                      <MethodBadge method={o.method} c={c} />
                      <span style={{ fontFamily: MONO, fontSize: 12, color: c.text }}>{o.path}</span>
                    </span>
                  ))
                )}
              </div>
            </div>
          );
        })}
      </div>
    </section>
  );
}

export function FlowTab({ api, screens }: { api: ApiDoc; screens: Screen[] }) {
  const { c, mode } = useTheme();
  const [showScreens, setShowScreens] = useState(true);
  const [showMiddleware, setShowMiddleware] = useState(true);
  const [hiddenMethods, setHiddenMethods] = useState<Set<string>>(new Set());
  const [search, setSearch] = useState("");
  const [sel, setSel] = useState<Sel>(null);
  const [exporting, setExporting] = useState(false);
  const [view, setView] = useState<FlowView>("graph");
  const instRef = useRef<ReactFlowInstance | null>(null);

  const opts = useMemo<GraphOpts>(() => ({ showScreens, showMiddleware, hiddenMethods }), [showScreens, showMiddleware, hiddenMethods]);
  const { nodes: laidNodes, edges: laidEdges } = useMemo(() => buildGraph(api, screens, opts, c), [api, screens, opts, c]);
  const [nodes, setNodes, onNodesChange] = useNodesState(laidNodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState(laidEdges);

  useEffect(() => {
    setNodes(laidNodes);
    setEdges(laidEdges);
  }, [laidNodes, laidEdges, setNodes, setEdges]);

  const allOps = useMemo(() => flattenOps(api), [api]);
  const titleOf = useMemo(() => new Map(screens.map((s) => [s.id, s.title || s.id])), [screens]);
  const screenCount = useMemo(() => new Set(allOps.flatMap((o) => o.op["x-screens"] || [])).size, [allOps]);
  const sig = useMemo(
    () => laidNodes.map((n) => n.id).join("|") + `#${showScreens ? "S" : ""}${showMiddleware ? "M" : ""}`,
    [laidNodes, showScreens, showMiddleware]
  );

  useEffect(() => {
    if (sel?.kind === "route" && !allOps.some((o) => o.path === sel.path && o.method === sel.method)) setSel(null);
  }, [allOps, sel]);

  const selectedOp = sel?.kind === "route" ? allOps.find((o) => o.path === sel.path && o.method === sel.method) : undefined;
  const screenOps = sel?.kind === "screen" ? allOps.filter((o) => (o.op["x-screens"] || []).includes(sel.id)) : [];

  const focusRoute = (path: string, method: HttpMethod) => {
    setSel({ kind: "route", path, method });
    const id = `${method.toUpperCase()} ${path}`;
    const n = nodes.find((x) => x.id === id);
    if (n && instRef.current) instRef.current.setCenter(n.position.x + ROUTE_W / 2, n.position.y + ROUTE_H / 2, { zoom: 1.15, duration: 450 });
  };
  const toggleMethod = (m: string) =>
    setHiddenMethods((prev) => {
      const next = new Set(prev);
      next.has(m) ? next.delete(m) : next.add(m);
      return next;
    });

  const seg: { id: FlowView; label: string; n?: number }[] = [
    { id: "graph", label: "Graph" },
    { id: "endpoints", label: "Endpoints", n: allOps.length },
    { id: "middleware", label: "Middleware", n: (api["x-middleware"] || []).length },
  ];

  if (!allOps.length) {
    return (
      <div className="flex h-full w-full items-center justify-center">
        <div className="flex flex-col items-center gap-2.5 text-center" style={{ color: c.faint, fontFamily: MONO }}>
          <Layers size={28} />
          <div className="max-w-[300px] text-[13px] leading-relaxed">No API routes yet — they appear here (OpenAPI 3) as the AI designs the endpoints.</div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full w-full flex-col">
      {/* top toolbar */}
      <div style={{ display: "flex", alignItems: "center", gap: 10, padding: "0 14px", height: 46, borderBottom: `1px solid ${c.border}`, background: c.panel, flexShrink: 0 }}>
        <Layers size={15} color={c.accent} />
        <span style={{ fontFamily: MONO, fontSize: 13, color: c.text }}>{api.info?.title || "API"}</span>
        {api.info?.version && <span style={{ fontFamily: MONO, fontSize: 10.5, color: c.faint, background: c.panel2, border: `1px solid ${c.borderSoft}`, borderRadius: 5, padding: "1px 6px" }}>v{api.info.version}</span>}
        <span style={{ fontFamily: MONO, fontSize: 11, color: c.faint, marginLeft: 4 }}>
          {allOps.length} routes · {(api["x-middleware"] || []).length} middleware{screenCount > 0 ? ` · ${screenCount} screens` : ""}
        </span>
        <div style={{ display: "flex", gap: 2, background: c.panel2, border: `1px solid ${c.borderSoft}`, borderRadius: 8, padding: 2, marginLeft: 6 }}>
          {seg.map((s) => (
            <button key={s.id} onClick={() => setView(s.id)} style={{ fontFamily: MONO, fontSize: 11.5, padding: "5px 11px", borderRadius: 6, border: "none", cursor: "pointer", background: view === s.id ? c.card : "transparent", color: view === s.id ? c.text : c.dim }}>
              {s.label}
              {s.n ? <span style={{ color: c.faint, marginLeft: 5 }}>{s.n}</span> : ""}
            </button>
          ))}
        </div>
        <div style={{ flex: 1 }} />
        {view === "graph" && (
          <button onClick={() => instRef.current?.fitView({ padding: 0.2, duration: 400 })} title="Fit to view" style={{ display: "inline-flex", alignItems: "center", gap: 6, fontFamily: MONO, fontSize: 11.5, padding: "6px 10px", borderRadius: 8, border: `1px solid ${c.border}`, background: c.card, color: c.dim, cursor: "pointer" }}>
            <Maximize2 size={13} /> Fit
          </button>
        )}
        <button onClick={() => setExporting(true)} style={{ display: "inline-flex", alignItems: "center", gap: 6, fontFamily: MONO, fontSize: 11.5, padding: "6px 11px", borderRadius: 8, border: `1px solid ${c.border}`, background: c.card, color: c.text, cursor: "pointer" }}>
          <Download size={13} /> Export OpenAPI 3
        </button>
      </div>

      {view === "graph" && (
      <div className="flex min-h-0 flex-1">
        <LeftRail
          ops={allOps}
          api={api}
          screenCount={screenCount}
          hiddenMethods={hiddenMethods}
          toggleMethod={toggleMethod}
          showMiddleware={showMiddleware}
          setShowMiddleware={setShowMiddleware}
          showScreens={showScreens}
          setShowScreens={(v) => {
            if (!v && sel?.kind === "screen") setSel(null);
            setShowScreens(v);
          }}
          search={search}
          setSearch={setSearch}
          sel={sel}
          onPick={focusRoute}
          c={c}
        />

        <div className="relative min-w-0 flex-1">
          <ReactFlow
            key={sig}
            nodes={nodes}
            edges={edges}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onInit={(inst) => {
              instRef.current = inst;
            }}
            onNodeClick={(_, n) => {
              if (n.type === "route") {
                const d = n.data as RouteData;
                setSel({ kind: "route", path: d.path, method: d.method as HttpMethod });
              } else if (n.type === "screen") {
                setSel({ kind: "screen", id: (n.data as { id: string }).id });
              }
            }}
            onPaneClick={() => setSel(null)}
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
            <MiniMap
              pannable
              zoomable
              nodeColor={(n) => (n.type === "route" ? methodColor((n.data as RouteData).method, c) : n.type === "screen" ? c.accent : c.dim)}
              maskColor={alpha(c.bg, 0.6)}
              style={{ background: c.panel, border: `1px solid ${c.border}`, borderRadius: 8 }}
            />
          </ReactFlow>
        </div>

        {selectedOp && sel?.kind === "route" && <Inspector op={selectedOp.op} path={sel.path} method={sel.method} c={c} screens={screens} onClose={() => setSel(null)} />}
        {sel?.kind === "screen" && <ScreenInspector title={titleOf.get(sel.id) || sel.id} ops={screenOps} c={c} onClose={() => setSel(null)} onPick={focusRoute} />}
      </div>
      )}
      {view === "endpoints" && <EndpointsView ops={allOps} c={c} onPick={focusRoute} setView={setView} />}
      {view === "middleware" && <MiddlewareView api={api} ops={allOps} c={c} />}
      {exporting && <ExportModal api={api} c={c} onClose={() => setExporting(false)} />}
    </div>
  );
}
