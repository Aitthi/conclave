// Knowledge-graph data for the Memory view (Obsidian Graph View style).
// Nodes = durable workspace memories (the `conclave memory` store + the
// file-memories that link with [[name]]). Edges = a link between two memories:
// an explicit [[wikilink]] in the body, or a "related" tie the store infers by
// semantic proximity. Colour = the agent who authored the memory; radius scales
// with degree (how connected a fact is), exactly like Obsidian sizes by links.
//
// The five real file-memories are present verbatim (see MEMORY.md); the rest are
// believable Conclave facts that give the graph its clustered-with-bridges shape.

import type { AgentColor } from "./data";

export type MemoryKind = "reference" | "feedback" | "project" | "user";

export interface MemoryNode {
  id: string;              // kebab slug, the file/name
  label: string;           // short title shown by the node
  body: string;            // the fact itself (detail card)
  author: string;          // agent id (colour key) — "shared" for cross-cutting protocol
  color: AgentColor;       // resolved avatar colour
  kind: MemoryKind;
  age: string;             // relative, for the detail card
}

export interface MemoryLink {
  a: string;
  b: string;
  // "wiki" = an explicit [[link]] in the body; "related" = store-inferred proximity.
  rel: "wiki" | "related";
}

// ── nodes ────────────────────────────────────────────────────────────────────
export const memoryNodes: MemoryNode[] = [
  // — data & core architecture (Mellow · teal / Detoro · indigo) —
  { id: "sqlx-chain-builder", label: "Data layer: sqlx + chain-builder", author: "mellow", color: "teal", kind: "reference", age: "6d",
    body: "The Rust core uses sqlx(sqlite) + chain-builder, not rusqlite — async pool + compile-checked queries were worth the migration." },
  { id: "uds-probe-before-bind", label: "UDS: probe before rebinding", author: "mellow", color: "teal", kind: "reference", age: "3d",
    body: "The conclave.sock server must probe an existing socket before remove+bind — the unconditional remove+bind was the relaunch regression." },
  { id: "spawn-sandbox-hole", label: "Spawn-time sandbox hole for UDS", author: "dew", color: "amber", kind: "reference", age: "2d",
    body: "Agent spawn needs an explicit sandbox hole for the conclave UDS path, or the child can't reach the socket to register." },

  // — workflow discipline (Detoro · indigo / Dew · amber) —
  { id: "act-as-lead-roster-first", label: "Act as lead: check roster first", author: "detoro", color: "indigo", kind: "feedback", age: "1d",
    body: "Run `conclave agent list` + a blackboard check BEFORE any solo work; delegate or record why-solo; always get peer review." },
  { id: "wait-for-repro", label: "Wait for live repro before hardening", author: "dew", color: "amber", kind: "feedback", age: "2d",
    body: "Don't ship speculative defensive fixes for unreproduced bugs, even after an initial 'harden it' go-ahead." },
  { id: "commit-pathspec", label: "Shared-tree commit needs a pathspec", author: "dew", color: "amber", kind: "feedback", age: "12h",
    body: "Plain `git commit` sweeps the whole shared index; commit design work with `git commit -- .arta/...` and `git show --stat` before reporting the SHA." },
  { id: "verify-before-claim", label: "Verify before you claim", author: "mellow", color: "teal", kind: "feedback", age: "4d",
    body: "'Done' means you ran it and watched it work — tests green with output you read, feature exercised end-to-end, build clean." },

  // — design / UI (Arta · sky) —
  { id: "ui-copy-english", label: "App UI copy is English", author: "arta", color: "sky", kind: "reference", age: "6d",
    body: "The app UI must be English, not Thai — replies to the user stay Thai, but product surfaces mirror the real app's English strings." },
  { id: "role-picker-cards", label: "Role picker is a card grid", author: "arta", color: "sky", kind: "project", age: "1d",
    body: "The role picker replaced <select> with a 2-col card grid; token-identical to the Type cards so both read as one system." },
  { id: "design-sync-record", label: "Implementers build from the .arta record", author: "arta", color: "sky", kind: "reference", age: "2d",
    body: "The .arta proto/screens are canon; a plan for visual work that names no design canon is a gap to escalate, never a licence to improvise." },
  { id: "token-fidelity", label: "Token fidelity to shipped cards", author: "arta", color: "sky", kind: "feedback", age: "1d",
    body: "New surfaces reuse the app's exact tokens (ring-accent/40, bg-surface) so a redesign never drifts a shade from what already ships." },

  // — memory & context protocol (shared · accent/hash) —
  { id: "memory-not-blackboard", label: "Memory is not the blackboard", author: "shared", color: "hash", kind: "reference", age: "5d",
    body: "Blackboard = live coordination (claims, progress), overwritten & keyed. Memory = durable knowledge found by meaning. A ruling can live in both." },
  { id: "strategic-compact-handoff", label: "Compact: the 7-section handoff", author: "shared", color: "hash", kind: "reference", age: "5d",
    body: "On a compact prompt, write NOW → mission → decisions → open threads → hard-won → done → pointers. Reference, don't paste; redact secrets." },
  { id: "recall-before-research", label: "Recall before you research", author: "shared", color: "hash", kind: "reference", age: "5d",
    body: "Search memory before deep-diving a question a past session may have solved — re-deriving a paid-for fact is the most expensive way to agree with a dead agent." },
];

// ── links ────────────────────────────────────────────────────────────────────
// Intra-cluster ties plus a few cross-cluster bridges — the shape that gives an
// Obsidian graph its distinct islands joined by lonely edges.
export const memoryLinks: MemoryLink[] = [
  // data / architecture cluster
  { a: "sqlx-chain-builder", b: "uds-probe-before-bind", rel: "related" },
  { a: "uds-probe-before-bind", b: "spawn-sandbox-hole", rel: "wiki" },
  { a: "sqlx-chain-builder", b: "verify-before-claim", rel: "related" },

  // workflow discipline cluster
  { a: "act-as-lead-roster-first", b: "verify-before-claim", rel: "wiki" },
  { a: "act-as-lead-roster-first", b: "wait-for-repro", rel: "related" },
  { a: "wait-for-repro", b: "verify-before-claim", rel: "wiki" },
  { a: "commit-pathspec", b: "act-as-lead-roster-first", rel: "related" },

  // design cluster
  { a: "ui-copy-english", b: "token-fidelity", rel: "related" },
  { a: "role-picker-cards", b: "token-fidelity", rel: "wiki" },
  { a: "role-picker-cards", b: "design-sync-record", rel: "related" },
  { a: "design-sync-record", b: "ui-copy-english", rel: "related" },

  // memory / context cluster
  { a: "memory-not-blackboard", b: "recall-before-research", rel: "wiki" },
  { a: "strategic-compact-handoff", b: "recall-before-research", rel: "related" },
  { a: "memory-not-blackboard", b: "strategic-compact-handoff", rel: "related" },

  // cross-cluster bridges — the lonely edges between islands
  { a: "design-sync-record", b: "act-as-lead-roster-first", rel: "related" }, // design ↔ discipline
  { a: "commit-pathspec", b: "design-sync-record", rel: "related" },          // discipline ↔ design
  { a: "verify-before-claim", b: "wait-for-repro", rel: "related" },
  { a: "strategic-compact-handoff", b: "act-as-lead-roster-first", rel: "related" }, // memory ↔ discipline
  { a: "recall-before-research", b: "sqlx-chain-builder", rel: "related" },   // memory ↔ data
];

// degree map — how many links touch each node (drives node radius, Obsidian-style)
export function degreeOf(): Record<string, number> {
  const d: Record<string, number> = {};
  for (const n of memoryNodes) d[n.id] = 0;
  for (const l of memoryLinks) { d[l.a] = (d[l.a] ?? 0) + 1; d[l.b] = (d[l.b] ?? 0) + 1; }
  return d;
}

// author → display label for the legend / detail card
export const authorName: Record<string, string> = {
  detoro: "Detoro", mellow: "Mellow", dew: "Dew", arta: "Arta", shared: "Shared protocol",
};
