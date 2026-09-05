# Memory Knowledge Graph repair
owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 (Aoki) · authority: in-loop
Implementer: Dabin. Escalations: Aoki. Read this plan then task brief.

## Goal and observed failure
Human asks to fix Memory Knowledge Graph, screenshot /Users/detoro/Desktop/Screenshot 2569-09-05 at 11.06.33.png. At 277 memories / 584 links, zoom 35%, graph canvas is nearly blank with tiny nodes on a horizontal line partly behind floating settings. Groups sidebar shows many indistinguishable Shared entries and extends beyond available viewport.

## Diagnosis phase (authorized now)
Claim task, inspect screenshot with image reader, read src/components/MemoryGraph.tsx and applicable ADR/spec. Build deterministic repro using real app fixture mode, graph data >=277 nodes and 584 links if necessary. Reproduce actual degeneracy, trace force initialization/integration, first measurement, fit/zoom, stale data callbacks, and groups source-id label resolution. Verify missing/removed agents separately from true shared-source data. Record ranked falsifiable hypotheses before testing and actual commands/results in task note. No product edits until Aoki amends implementation section. Temporary harnesses permitted; no production memory mutation. Read-only backend queries allowed. Return proposed exact file boundary and minimal repair backed by evidence, then notify Aoki.

## Acceptance criteria for implementation ruling
Graph opens visibly distributed and fitted within usable canvas at real-size dataset; positions remain finite; resize, fit/reset, search/filter, select/deselect, zoom/pan continue to work. Group labels distinguish actual sources without inventing agent identity; groups/controls accessible in constrained height. Memory count/data remain intact. Default and empty scenarios tested.

## Global constraints
Independent of Skill repair (Dew) and browser work. Preserve established graph visual design; no broad redesign or dependency migration. Historical canon .arta/proto/screens/memory-graph.tsx commit c134d01 (verify in git; .arta currently absent). Current src/components/MemoryGraph.tsx is behavioral baseline. Any necessary canon exception goes to Aoki, not human.
Before READY run pnpm uishot memory and --scenario empty, OPEN AND INSPECT each PNG, attach absolute shot paths and record each via conclave task gate. Also reproduce and capture >=277-node case; a sparse fixture alone cannot prove this fix. Before trusting any shot inspect lsof -nP -iTCP:1420 -sTCP:LISTEN and process cwd; do not use foreign Vite server. Coordinate port ownership through Aoki before stopping any peer server. Add targeted regression at real bug seam before fix, pnpm build, relevant existing checks. Use fixed literal timestamps in fixture data. No swallowed fixture handlers/errors.

## Implementation ruling
Diagnosis accepted; challenge 2cf630ab ruled. Implementation moves to task memory-graph-fix with expanded immutable boundary; see docs/superpowers/plans/2026-09-05-memory-graph-fix.md. No product edits belong to this diagnosis task.

## Diagnosis findings and decision record
Evidence: task notes e461008e (real-app zero visible nodes), da6bdc99 (fit alone fails), df3fb6cb (unit-vector-only interception restores all 279 nodes), 8a25ca21 (20 former-agent identities mislabeled Shared), challenge 2cf630ab. Aoki inspected MemoryGraph.tsx collision branch, duplicate name key, unconstrained panel and normalized-guard screenshot. Backend memory.approve writes distilled, confirming stale frontend union. Causal correction and dense-label exception approved; retain existing fit limits, no graph-library migration. Full implementation contract in follow-up plan.
