# Conclave identity

Conclave's **Open Assembly** mark is an architectural `C` made from three radial council tiers around one shared chamber. The separated planes represent attributable agents; the room they form represents the durable task record where their work meets. The east-facing opening keeps the symbol invitational rather than closed or defensive.

The identity is quiet and operational. It deliberately avoids robot faces, AI sparkles, node diagrams, religious symbols, knots, flowers, and the intersecting-loop construction associated with Tauri.

## Canonical artwork

- `public/brand/logo-mark.svg` is the primary two-colour vector mark.
- `public/brand/logo-mark-mono.svg` uses an explicit Council ink fill for portable SVG export and is the preferred one-colour asset. Override the group's `fill` in CSS or an authoring tool when another single colour is required.
- `public/brand/app-icon.svg` is the canonical 1024 × 1024 desktop source. Its transparent outer margin and 896 px cool-ink tile provide the native desktop safe area.
- `public/brand/favicon.svg` is optically redrawn for 16–64 px rather than mechanically shrinking the desktop source.
- `public/brand/logo-lockup.svg` pairs the mark with a live system-font wordmark. The font stack is `-apple-system`, `BlinkMacSystemFont`, `SF Pro Display`, `Helvetica Neue`, `Arial`, `sans-serif`; consumers that require identical cross-platform outlines should use the standalone mark or outline the text in their publishing tool.
- `public/brand/brand-preview.svg` is the editable preview board; `public/brand/preview.png` is its shareable raster export.

Do not redraw from a PNG. The SVGs are the source of truth.

## Geometry

The mark uses a 512-unit grid centred on `256,256`. Its visible bounds are `x=68…445`, `y=67…445`. Three faceted annular planes share an approximate 192-unit outer radius and 104-unit inner radius:

- upper tier: outer angles `236°…320°`
- left tier: outer angles `132°…228°`
- lower signal tier: outer angles `40°…124°`

The east opening spans 80 degrees; the two radial seams span 8 degrees each. These voids are structural and must remain open. Clear space is at least 64 units on every side. Do not rotate, reconnect, outline, round into loops, add shadows, or recolour individual ink pieces.

## Colour

| Role | Hex | Use |
| --- | --- | --- |
| Desktop ink | `#102A43` | App-icon tile and dark brand surfaces |
| Council ink | `#16324F` | Primary mark and wordmark on light surfaces |
| Signal coral | `#E05F67` | One participant tier only |
| Chalk | `#F2F7F9` | Mark on desktop ink or other dark surfaces |

On dark surfaces, change both council-ink pieces to Chalk and retain Signal coral. For one-colour production, override the `fill` on the group in `logo-mark-mono.svg`. Never simulate the signal tier with a gradient.

## Minimum size and placement

- Use `favicon.svg` below 64 px.
- Use the full mark at 32 px or larger when no enclosing tile is present.
- Keep the central opening and both 16-unit seams unobstructed.
- Prefer the mark alone for app chrome and the lockup for public documentation.
- The app icon already includes its native safe area. Do not add another background plate or crop to the coloured tile.

## Reproducible exports

Prerequisites are the repo's installed Tauri CLI, ImageMagick, and macOS `iconutil`. From the repository root:

```sh
node scripts/generate-brand-icons.mjs
```

The script invokes Tauri's icon generator in a temporary directory, copies only the tracked desktop/Windows bundle assets into `src-tauri/icons/`, normalizes the ICNS container through `iconutil`, and regenerates `public/brand/preview.png`. The normalization makes repeat runs byte-stable; Tauri's generated mobile trees are intentionally not copied.

To override locally discovered tools:

```sh
TAURI_CLI=/absolute/path/to/tauri MAGICK=/absolute/path/to/magick ICONUTIL=/usr/bin/iconutil node scripts/generate-brand-icons.mjs
```

Verify the container families after export:

```sh
magick identify src-tauri/icons/icon.ico
file src-tauri/icons/icon.ico src-tauri/icons/icon.icns
sips -g pixelWidth -g pixelHeight src-tauri/icons/icon.icns
brand_check_dir="$(mktemp -d /tmp/conclave-icon-check.XXXXXX)"
iconutil --convert iconset --output "$brand_check_dir/icon.iconset" src-tauri/icons/icon.icns
```

macOS may retain a cached icon for an already installed application. A clean rebuild/relaunch, and sometimes Dock/Finder cache refresh, can be required before the new tile appears outside the bundle.

## Verification evidence

The final monochrome silhouette was compared on 2026-09-05 against [Replit's official prompt mark](https://replit.com/blog/new-logo) and [Tauri's official SVG](https://github.com/tauri-apps/tauri-docs/blob/v2/src/assets/logo.svg). This is a visual differentiation check, not trademark clearance.

- Replit uses three orthogonal rounded blocks arranged as a vertical prompt. Open Assembly is a radial, faceted `C` with a polygonal central chamber and east aperture; it shares neither the stack nor the prompt topology.
- Tauri uses two interlocking circular tracks with internal dots. Open Assembly uses three non-interlocking annular planes, no dots, and one explicit opening.
- The generated 16 px ICO member, 32 px PNG, 1024 px ICNS extraction, primary mark, monochrome mark, light use, and dark use were opened and visually inspected. The chamber and seams remain visible throughout the family.
- The ICO contains 16, 24, 32, 48, 64, and 256 px members. The normalized ICNS expands to ten standard 16–1024 px iconset members.

Tauri's first ICNS container changed byte-for-byte across regeneration even though all ten extracted PNG members were byte-identical. The generator now unpacks Tauri's output and repacks it with `iconutil`; consecutive full regeneration runs then produced identical SHA-256 hashes for every tracked icon and the preview. This preserves semantic pixels and byte-level reproducibility rather than hiding the container delta.
