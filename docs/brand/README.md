# Conclave identity

Conclave's **Open Assembly** mark is an open `C` made from three distinct seats around one shared negative space. The separated pieces represent attributable agents; the room they form represents the durable task record where their work meets. The opening keeps the symbol invitational rather than closed or defensive.

The identity is quiet and operational. It deliberately avoids robot faces, AI sparkles, node diagrams, religious symbols, knots, flowers, and the intersecting-loop construction associated with Tauri.

## Canonical artwork

- `public/brand/logo-mark.svg` is the primary two-colour vector mark.
- `public/brand/logo-mark-mono.svg` uses `currentColor` and is the preferred one-colour asset.
- `public/brand/app-icon.svg` is the canonical 1024 × 1024 desktop source. Its transparent outer margin and 896 px cool-ink tile provide the native desktop safe area.
- `public/brand/favicon.svg` is optically redrawn for 16–64 px rather than mechanically shrinking the desktop source.
- `public/brand/logo-lockup.svg` pairs the mark with a live system-font wordmark. The font stack is `-apple-system`, `BlinkMacSystemFont`, `SF Pro Display`, `Helvetica Neue`, `Arial`, `sans-serif`; consumers that require identical cross-platform outlines should use the standalone mark or outline the text in their publishing tool.
- `public/brand/brand-preview.svg` is the editable preview board; `public/brand/preview.png` is its shareable raster export.

Do not redraw from a PNG. The SVGs are the source of truth.

## Geometry

The mark uses a 512-unit grid. Its visible bounds are `x=64…448`, `y=64…448`. Three rounded pieces have 32-unit corner radii:

- upper seat: `128,64` at `320 × 112`
- left seat: `64,192` at `112 × 128`
- lower signal seat: `128,336` at `320 × 112`

The 16-unit vertical separations must remain open. Clear space is at least 64 units (one half of the left seat's height) on every side. Do not rotate, reconnect, outline, add shadows, or recolour individual ink pieces.

## Colour

| Role | Hex | Use |
| --- | --- | --- |
| Desktop ink | `#102A43` | App-icon tile and dark brand surfaces |
| Council ink | `#16324F` | Primary mark and wordmark on light surfaces |
| Signal amber | `#EE9C3A` | One participant seat only |
| Chalk | `#F2F7F9` | Mark on desktop ink or other dark surfaces |

On dark surfaces, change both council-ink pieces to Chalk and retain Signal amber. For one-colour production, set `color` on `logo-mark-mono.svg`. Never simulate the amber seat with a gradient.

## Minimum size and placement

- Use `favicon.svg` below 64 px.
- Use the full mark at 32 px or larger when no enclosing tile is present.
- Keep the central opening and both 16-unit seams unobstructed.
- Prefer the mark alone for app chrome and the lockup for public documentation.
- The app icon already includes its native safe area. Do not add another background plate or crop to the coloured tile.

## Reproducible exports

Prerequisites are the repo's installed Tauri CLI and ImageMagick. From the repository root:

```sh
node scripts/generate-brand-icons.mjs
```

The script invokes Tauri's icon generator in a temporary directory, copies only the tracked desktop/Windows bundle assets into `src-tauri/icons/`, and regenerates `public/brand/preview.png`. Tauri's generated mobile trees are intentionally not copied.

To override locally discovered tools:

```sh
TAURI_CLI=/absolute/path/to/tauri MAGICK=/absolute/path/to/magick node scripts/generate-brand-icons.mjs
```

Verify the container families after export:

```sh
magick identify src-tauri/icons/icon.ico
magick identify src-tauri/icons/icon.icns
file src-tauri/icons/icon.ico src-tauri/icons/icon.icns
```

macOS may retain a cached icon for an already installed application. A clean rebuild/relaunch, and sometimes Dock/Finder cache refresh, can be required before the new tile appears outside the bundle.
