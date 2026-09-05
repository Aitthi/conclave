# Provider logo marks for the Runtime picker

Task `provider-logos` · researcher Guetta (10bd4d86) · 2026-09-05 · owner Detoro (30fa04f4)

Delivered files live in `design/assets/providers/`. Every `<cliKind>.svg` is a
`viewBox="0 0 24 24"` mark filled with `currentColor`; `<cliKind>.color.svg`
keeps the vendor colours where the original is multi-colour.

## Summary

| cliKind | Files | Mark source (commit) | Source license | Official or redraw | Ship-blocker? |
|---|---|---|---|---|---|
| `claude-code` | `claude-code.svg`, `claude-code.color.svg` | lobehub/lobe-icons `src/ClaudeCode/components/{Mono,Color}.tsx` @ `4aaf4ee1fb2678a7f989ea570f0f6ce14a9abf75` (byte-identical path to agent-orchestrator `frontend/src/renderer/assets/agents/claude-code.svg` @ `7678f9f`) | MIT (lobe-icons) / Apache-2.0 (agent-orchestrator) | Community redraw of Anthropic's official Claude Code glyph (pixel-face) | No. Trademark: nominative use only |
| `codex` | `codex.svg`, `codex.color.svg` | mono: lobe-icons `src/Codex/components/Mono.tsx` @ `4aaf4ee`; colour: agent-orchestrator `frontend/src/renderer/assets/agents/codex.svg` @ `fde3a12` (250×250 gradient, wrapped to 24×24 with `scale(0.096)`) | MIT / Apache-2.0 | Redraw of OpenAI's official Codex mark (blob + `>_`); colour file carries OpenAI's gradient | No. Trademark: nominative use only |
| `antigravity` | `antigravity.svg`, `antigravity.color.svg` | lobe-icons `src/Antigravity/components/{Mono,Color}.tsx` @ `4aaf4ee` (JSX converted to plain SVG, ids prefixed `ag-`) | MIT | Redraw of Google's official Antigravity "A" mark; colour file is the 12-blob blurred gradient | No. Trademark: nominative use only. Colour file is 6.5 KB with 11 `feGaussianBlur` filters; fine at 16–32 px |
| `opencode` | `opencode.svg` | lobe-icons `src/OpenCode/components/Mono.tsx` @ `4aaf4ee` (byte-identical path to agent-orchestrator `opencode.svg` @ `fde3a12`) | MIT | Redraw of the official sst/opencode square mark (frame + inner rect); same geometry as the vendor's `opencode-logo-*-square.svg` | No. Vendor publishes its own brand kit (MIT repo) |
| `muse-spark` | `muse-spark.png` only (**no SVG found**) | agent-orchestrator `frontend/src/renderer/assets/agents/muse.png` @ `bf466cc` (PR #3667, 2026-08-06), 128×128 RGBA | Apache-2.0 (repo); raster provenance not stated in the PR | Meta's Muse mark (gold concentric labyrinth), raster only | **Flag**: raster, single colour, no vendor SVG located. See "Muse Spark" below |

License note that applies to all five: the MIT / Apache-2.0 licenses of the
source repos cover the *file copies*; Apache-2.0 §6 and MIT both grant no
trademark rights. The marks remain trademarks of Anthropic, OpenAI, Google,
Anomaly Innovations (sst) and Meta Platforms respectively. Using them as small
identifiers next to the runtime's own name (nominative use, no endorsement
implied, no alteration beyond monochrome recolouring) is the same usage the
Apache-2.0 agent-orchestrator README and the MIT lobe-icons package already
ship publicly. This is my assessment, not legal advice.

Verification sheet (rendered via `qlmanage`, light and dark, 240 px and 24 px):
all seven SVGs render, mono files respond to `currentColor`, colour files keep
vendor gradients.

## Primary source: Untrivial-ai/agent-orchestrator

- Repo https://github.com/Untrivial-ai/agent-orchestrator, license **Apache-2.0**
  (`LICENSE` at HEAD). HEAD on 2026-09-05: `d2c88dae7d968df26c990ac5dd42ed2fc17513b1`.
- The README provider table (`README.md` lines 133–174) renders
  `frontend/src/renderer/assets/agents/<id>.{svg,png}`. Of the five runtimes we
  need, **Claude Code, Codex and opencode are SVG; Muse and Agy (Antigravity)
  are PNG** (`muse.png` 20 037 B and `agy.png` 25 278 B, both 128×128).
- `claude-code.svg` and `opencode.svg` there carry the lobe-icons wrapper
  (`height="1em" style="flex:none;line-height:1"` + `<title>`), i.e. they were
  exported from `@lobehub/icons`. The Codex SVG is a different, 250×250
  gradient export (source not stated; visually identical to OpenAI's Codex mark).
- Muse vendor identification: `backend/internal/adapters/agent/muse/muse.go`
  header — "Muse Code is Meta's terminal coding agent, installed from
  https://dev.meta.ai/install.sh as the executable `muse`". Display name in
  `packages/product-ui/src/agents.ts` is `muse: "Muse"`; `agy: "AGY"`.
- Source sha256 (files as fetched at `d2c88da`):
  - `claude-code.svg` `670b3d8d749d0815ad8f7e62a59d51cb5e2053cb19403f3541a8aed7036a877f`
  - `codex.svg` `e6709627fbfa25d7df2461ef38116eb717b68196dff814234a37cf8064e68cc8`
  - `opencode.svg` `87bbb7e1e30dbfba8e0ae7fd67e9ff1723bfc6d8eea10b57f107c569c33a0cc9`
  - `muse.png` `9601c9fd9fdf3ad69b43a0da60a3ceb633041e4302d7bc19321e6530d522fed0`

## Secondary source: lobehub/lobe-icons

- https://github.com/lobehub/lobe-icons, license **MIT**, master
  `4aaf4ee1fb2678a7f989ea570f0f6ce14a9abf75` on 2026-09-05.
- Provides `Mono` (currentColor) and `Color` variants for `ClaudeCode`, `Codex`,
  `Antigravity` (`index.mdx`: "Antigravity (Google) — https://antigravity.google"),
  `OpenCode`. No `Muse` / `MuseCode` / `MuseSpark` entry exists (tree grep, and
  only PR #403 "use Meta AI wordmark for the meta provider icon" mentions Meta).
- `src/Antigravity/components/Color.tsx` sha256
  `bb85138dc952b4ed3dd7bed735ed346ff1b9005c6a25e8967adb548b87add9dc`; converted
  by replacing `useFillIds` ids with fixed `ag-a…ag-l`, JSX attrs to kebab-case,
  dropping `size`/`style`/`{...rest}`.

## Vendor references (official assets / trademark terms)

| Vendor | Official brand page | Notes |
|---|---|---|
| Anthropic (Claude Code) | https://www.anthropic.com/press-kit · trademark guidelines https://www.anthropic.com/legal/trademark-guidelines (both HTTP 200 on 2026-09-05) | No SVG of the Claude Code glyph in the press kit; the pixel-face glyph is the one Claude Code's own CLI/extension uses (open-vsx `Anthropic/claude-code` ships `claude-logo.png`) |
| OpenAI (Codex) | https://openai.com/brand/ (403 to curl, browser-accessible) | openai/codex repo has no logo SVG (only skill sample icons) |
| Google (Antigravity) | https://about.google/brand-resource-center/ → https://partnermarketinghub.withgoogle.com/brands/google/overview/ · product https://antigravity.google | antigravity.google HTML has no inline SVG or logo URL (JS-rendered) |
| Anomaly Innovations / sst (opencode) | https://opencode.ai/brand · repo https://github.com/sst/opencode `packages/console/app/src/asset/brand/` @ `e2894562f8ba943d72172d10b727c24d5f650c16` (MIT): `opencode-logo-{dark,light}[-square].svg`, wordmarks, `opencode-brand-assets.zip` | Official square mark: 300×300, frame `#F1ECEC` + inner rect `#4B4646` on dark. If a two-tone official version is wanted, take `opencode-logo-dark-square.svg` from there |
| Meta (Muse Spark / Muse Code) | https://www.meta.com/brand/resources/ (Meta corporate marks; no Muse assets) · product docs https://dev.meta.ai/docs/muse-code · SDK https://github.com/meta-models/muse-code-sdk (MIT, `fbce769`, contains no image assets) | See below |

## Muse Spark: what was searched, and the candidates

Vendor: **Meta Platforms**. The CLI product is "Muse Code" (binary `muse`,
installer `https://dev.meta.ai/install.sh` → `https://api.meta.ai/muse-launcher.sh`,
release channel `muse-stable`, version `1.0.3-R2198.1` on 2026-09-05, artifacts
served from lookaside.facebook.com). The model family is "Muse Spark"
(`meta/muse-spark-1.1`, `-1.2`, `-1.3` on models.dev). The agent-orchestrator
screenshot label "Muse" = this product; `cliKind: muse-spark` is the right key.

The canonical mark is the **gold concentric labyrinth** (see `muse-spark.png`).
No vector version was found:

- dev.meta.ai landing (465 KB), `/docs/muse-code` (333 KB) and their 12
  `static.xx.fbcdn.net` JS bundles: zero `<svg>`, zero `.svg` URLs, no
  `Muse*Logo|Icon` identifiers (only `MuseImage`, `museCodeUpsell`, …).
- ai.meta.com, meta.ai, meta.com/muse-spark: 401/403/404 (JS or login-walled).
- lobe-icons, simple-icons (issue search), Wikimedia Commons: nothing.
- GitHub filename search `muse.svg` / `muse-code.svg` / `muse-spark.svg`: only
  unrelated hits (Papirus theme, a blog's post illustrations).
- `meta-models/muse-code-sdk` (official, MIT): no image assets at all.
- Social preview of the docs page is a 1200×630 JPEG.

Decision per plan ("leave the file absent rather than drawing one"):
`muse-spark.svg` is **absent**. Shipped instead, clearly labelled:

1. `muse-spark.png` — the 128×128 raster from agent-orchestrator `bf466cc`
   (Apache-2.0 repo; the PR does not say where the raster came from). Usable as
   an `<img>` tile at ≤32 px; cannot be recoloured with `currentColor`.

Alternatives if the raster is rejected:

- lobe-icons `src/Meta/components/{Mono,Color,BrandMono}.tsx` @ `4aaf4ee` (MIT):
  the Meta corporate infinity mark. Vendor mark, not the product mark; would
  read as "Meta" rather than "Muse".
- https://models.dev/logos/meta.svg (HTTP 200): same corporate mark.
- Ask Meta for the Muse press asset via https://www.meta.com/brand/resources/;
  or trace the vector once Meta publishes a Muse Code VS Code extension or
  docs site with an inline SVG (none exist on 2026-09-05).

## Not done / caveats

- No `src/` edits; `src/components/builder/providerLogos.tsx` still owns the
  in-app map and is untouched.
- The `.color.svg` for Antigravity was machine-converted from JSX; ids are
  fixed (`ag-*`) so inlining two copies on one page would collide — use it as
  an `<img>`/`url()` or rewrite ids when inlining.
- Trademark review is my reading of the licenses; if the app ships publicly
  with the OpenAI/Google colour marks, a human should skim those two brand
  pages once.
