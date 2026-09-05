# Conclave public app identity

owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop
Designer/implementer: Hardwell (aee0133c-2b94-4ce7-b39a-01ceb26afeb9).
Escalations: Aoki, final design/integration ruling. User requested a Conclave logo and replacement of default Tauri identity ahead of public use. Local changes authorized; publication and release are outside this task.

## Reading order
PRODUCT.md → src/styles/app.css → this plan → task brief.

## Settled direction
A compact original geometric mark expressing a conclave: distinct participants forming one shared space. Seek a strong C silhouette or open assembly, with generous negative space and few bold pieces. Quiet, precise, operational; recognisable at 16/32 px and in monochrome. Explore internally, select ONE polished solution. Existing native UI typography and colors stay intact. Brand color may be a deliberate independent cool ink/blue with one accent; not tied to mutable macOS system accent. Reject robot heads, AI sparkles, intricate node networks, generic knot/flower marks, literal religious symbolism, and Tauri-style intersecting loops. Native editable SVG is canonical; no stock logo, external font dependency, or raster tracing artifacts.

## Deliverables and exact file boundary
- public/brand/: canonical logo-mark.svg, app-icon.svg (1024 square source with native desktop safe area), monochrome variants if necessary, reusable wordmark/lockup with outlined lettering or existing-font use documented, preview.png showing large mark, Dock tile, light/dark, and 16/32/64 px.
- src-tauri/icons/: regenerate ALL tracked default PNG/ICO/ICNS from canonical icon. Preserve bundle paths in existing tauri.conf.json. Do not add mobile platform asset trees.
- index.html: Conclave title and own favicon in public/brand/.
- docs/brand/: short identity and reproducible export guide with geometry/colors/min size/clear space, commands, verification evidence.
- scripts/generate-brand-icons.* only if needed for deterministic regeneration; prefer installed Tauri CLI and local dependencies. No package manifest/lockfile churn.
- docs/plans/2026-09-05-conclave-brand.md is lead-owned plan, read only to implementer.
Do not edit src/ in this lane. Any identified in-app default identity is a separate implementation task after canon is pinned.

## Workflow and acceptance
Claim via conclave lane start. One isolated worktree, no merges by implementer. Inspect installed CLI --help before using icon generation. Render and OPEN all meaningful identity previews including actual generated 16/32 px and macOS icon export; check silhouette, alpha, safe area, consistent family. Build gate: pnpm build. Check ICO/ICNS contain expected images and all configured asset paths are valid. No new tests for static assets. Record commands as task gates, preview paths in READY note; commit via conclave stage commit. Exact completion phrase: BRAND READY. Aoki reviews pixels and owns integration.

## Risk ledger
Native icon caches may retain old installed icon until rebuilt/relaunched; report accurately. Safe area must avoid Dock tile looking oversized. SVG render support differs across exporters; inspect exported pixels. A foreign Vite on 1420 must not validate another checkout. A src/ UI task requires every affected view pnpm uishot, PNG inspection and task gate before READY, plus empty scenario if affected.

## Review ruling 2026-09-05 — first silhouette rejected
Aoki inspected public/brand/preview.png at 6845b38f. The three stacked offset rounded bars (long top/bottom, short left middle), especially with orange, evoke Replit prompt identity too closely for another developer tool. Reference: https://replit.com/blog/new-logo (official description and artwork). This is visual differentiation judgment, not a trademark clearance claim. The plan allowed this construction; the under-specified distinctiveness check was Aoki's plan defect.
Preserve Open Assembly concept but CHANGE silhouette: use a circular/angular open council chamber C with radial segments or converging planes and a clear central void. No three horizontally stacked pill bars, no disconnected prompt/E motif, no knot or generic flower. Keep restrained palette, mono reproducibility, native desktop safe area and small-size legibility. Designer owns geometry and may replace amber if it improves identity. Regenerate all assets and preview from revised canon. Before READY compare monochrome silhouette with Replit and Tauri so color is not doing the differentiation. First commit is not approved for merge.
