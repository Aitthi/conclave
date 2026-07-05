import type { DesignTokens } from "./types";

// The web-font families preloaded for every prototype (and the viewer shell itself, for
// device-frame snapshots — modern-screenshot embeds @font-face from the PARENT
// document's stylesheets). A neutral sans + mono (Geist), an elegant display serif
// (Instrument Serif), a warm variable serif (Fraunces), and a geometric display (Space
// Grotesk). The Latin faces above are Latin-only, so Noto Sans/Serif Thai are loaded
// too — put them in a font-family fallback chain (`'Fraunces', 'Noto Serif Thai',
// serif`, applied automatically by withThaiFallback below) so non-Latin text renders in
// a real designed face instead of a broken system fallback. This ONE constant is the
// actual source of truth for both HTML shells (index.html, proto/index.html) — injected
// by protoApp()'s transformIndexHtml hook (vite/proto-app.ts), not hand-copied.
export const FONT_LINK =
  `<link rel="preconnect" href="https://fonts.googleapis.com">` +
  `<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>` +
  `<link href="https://fonts.googleapis.com/css2?family=Geist:wght@400;500;600;700&family=Geist+Mono:wght@400;500;600&family=Instrument+Serif:ital@0;1&family=Fraunces:opsz,wght@9..144,400;9..144,500;9..144,600;9..144,700&family=Space+Grotesk:wght@400;500;600;700&family=Noto+Sans+Thai:wght@400;500;600;700&family=Noto+Serif+Thai:wght@400;500;600;700&display=swap" rel="stylesheet">`;

// The preloaded display/text faces (Geist, Instrument Serif, Fraunces, Space Grotesk)
// are Latin-only, so a token like `'Geist', system-ui, sans-serif` leaves Thai (and any
// other non-Latin) to the generic system fallback. That fallback resolves to DIFFERENT
// faces in the live screen iframe than in a snapshot — modern-screenshot repaints the
// cloned iframe inside the viewer's PARENT document (see lib/capture.ts's captureFramedPng), where
// `system-ui` / `sans-serif` map to other Thai faces with different metrics. The dev and
// the agent then saw different glyph widths and long strings wrapped/overlapped only in
// the snapshot. Pinning a loaded Noto Thai face IN the chain ties non-Latin text to one
// webfont that paints identically in both contexts. Injected automatically so EVERY
// prototype gets snapshot↔live parity even when the authored token omits it (e.g. a build
// from a harness that never loaded the skill's font guidance).
export function withThaiFallback(value: string): string {
  if (/Noto\s+(?:Sans|Serif)\s+Thai/i.test(value)) return value; // already pinned
  const serif = /\bserif\b/i.test(value) && !/sans-serif/i.test(value);
  const thai = serif ? "'Noto Serif Thai'" : "'Noto Sans Thai'";
  const generic = /^(?:serif|sans-serif|monospace|system-ui|ui-serif|ui-sans-serif|ui-monospace|cursive|fantasy)$/i;
  const parts = value.split(",").map((p) => p.trim()).filter(Boolean);
  const gi = parts.findIndex((p) => generic.test(p));
  if (gi >= 0) parts.splice(gi, 0, thai);
  else parts.push(thai);
  return parts.join(", ");
}

// Recover displayable tokens from the `:root` custom properties in a stylesheet. The
// Design-system tab reads structured design tokens, but the AI often sets up the system
// as raw CSS (arta_set_design_system) with a `:root` block and never calls
// arta_set_design_tokens — which left the tab blank even though a real design system
// existed. Parsing the authored CSS's `:root` vars back into tokens makes the tab reflect
// the system whichever way it was authored. Reads ONLY `:root` blocks, so per-component
// custom properties don't leak in.
// A value is a colour if it's a hex, a colour function, or a common named colour. Anchored
// so a multi-part value (a shadow like `0 1px 2px #0003`) is NOT mistaken for a colour.
const isColorVal = (v: string) =>
  /^#[0-9a-f]{3,8}$/i.test(v) ||
  /^(rgb|rgba|hsl|hsla|hwb|lab|lch|oklab|oklch|color|color-mix)\s*\(/i.test(v) ||
  /^(transparent|currentcolor|black|white|red|green|blue|orange|purple|pink|gray|grey|yellow|teal|cyan|magenta|indigo|violet|slate|navy|gold|brown|beige|cream|coral|salmon|lime|olive|maroon|aqua|silver|crimson|tomato|turquoise|azure|ivory|khaki|plum|tan|wheat)$/i.test(v);
const isLenVal = (v: string) => /^-?[\d.]+(px|rem|em|%|vh|vw|vmin|vmax|pt|ch|ex|q|cm|mm|in|pc)$/i.test(v);
const looksShadow = (n: string, v: string) => /shadow|elevation/i.test(n) || /\binset\b/i.test(v) || /(?:[\d.][\w.%-]*\s+){2,}(?:rgb|hsl|#|oklch|color)/i.test(v);
const looksFontStack = (n: string, v: string) => /font|family|typeface/i.test(n) || v.includes(",") || /\b(serif|sans-serif|monospace|system-ui|ui-sans-serif|ui-serif|ui-monospace|cursive)\b/i.test(v);
const looksRadius = (n: string) => /radius|rounded|corner|radii/i.test(n) || /(^|[-_])r($|[-_\d])/i.test(n);

// Both tokensFromCss and darkVars block-match with a naive `([^{}]+)\{([^}]*)\}` regex over
// the RAW css text, which treats EVERYTHING since the previous `}` (including any `/* ... */`
// comment) as the block's "selector" text. A comment sentence that happens to contain an
// ordinary English semicolon — e.g. the shipped Helix demo's own theme.css header ("...CLI /
// command motifs. Body prose in Geist sans; everything system-ish is mono.") — gets swept
// into that selector text and splits the last-statement-before-`{` heuristic below, so the
// real `@theme`/`.dark` selector is never recognized and the whole token block silently
// vanishes. Comments never carry real tokens, so stripping them before any block-matching
// runs is always safe.
const stripComments = (css: string): string => css.replace(/\/\*[\s\S]*?\*\//g, "");

export function tokensFromCss(css?: string): DesignTokens {
  const out: DesignTokens = {};
  if (!css || !css.trim()) return out;
  css = stripComments(css);
  // Gather custom properties from the GLOBAL root rules only — :root is canonical, but
  // agents commonly hang tokens off html/body/:host too (or a `:root,:host` list). Per-
  // component/class rules are skipped so their local vars don't leak in. `.dark{}` is the
  // dark-theme overlay (handled by darkVars), not the light palette, so exclude it here.
  //
  // Tailwind v4 CSS-first config (the current theme.css scaffold, vite/scaffold.ts) puts
  // tokens in an `@theme { ... }` AT-RULE, not a `:root` selector — so the plain selector
  // checks above never matched it and every real project's tokens silently disappeared.
  // The naive brace-matcher below (`([^{}]+)\{...\}`) captures EVERYTHING since the previous
  // `}` as the "selector" for a block, which for theme.css is several semicolon-terminated
  // at-rules (`@import`; `@source`; `@custom-variant dark (&:where(.dark, .dark *))`;) stacked
  // before `@theme`. Two consequences: (1) we can't just check the whole trimmed text starts
  // with "@theme" — only the LAST statement (after the last top-level `;`) is this block's
  // real selector; (2) the OLD blanket `!/\.dark\b/.test(sel)` guard scanned that whole
  // multi-statement blob for the substring ".dark" and matched it inside the unrelated
  // `@custom-variant` directive (`&:where(.dark, .dark *)`), wrongly excluding the `@theme`
  // block too. Extract the last statement FIRST (splitting on `;`, safe here since none of
  // these at-rules' parens contain a `;`), then comma-split and dark-check only THAT.
  const lastStatement = (sel: string) => {
    const parts = sel.split(";");
    return parts[parts.length - 1].trim();
  };
  const isRootSel = (sel: string) =>
    lastStatement(sel)
      .split(",")
      .some((s) => {
        const t = s.trim();
        if (/\.dark\b/.test(t)) return false; // e.g. `:root.dark` — a dark override, not the light palette
        return (
          t === ":root" ||
          t === "html" ||
          t === "body" ||
          t === ":host" ||
          /(^|[\s>~+]):root\b/.test(t) ||
          /^@theme\b/.test(t)
        );
      });
  const body = [...css.matchAll(/([^{}]+)\{([^}]*)\}/g)]
    .filter((m) => isRootSel(m[1]))
    .map((m) => m[2])
    .join(";");
  const seen = new Set<string>();
  for (const m of body.matchAll(/--([\w-]+)\s*:\s*([^;]+)/g)) {
    const name = m[1].trim();
    const value = m[2].trim();
    if (!value || seen.has(name)) continue;
    seen.add(name);
    // Prefix naming (e.g. `--color-*`, `--font-*`) wins; otherwise classify by the value's
    // shape so a freeform `--accent: #0a84ff` / `--mac-bg` still renders as a swatch instead
    // of leaving the tab on the bare "define tokens" placeholder (the #1 recurring report).
    if (name.startsWith("color-")) (out.colors ||= []).push({ name: name.slice(6), value });
    else if (name.startsWith("font-")) (out.fonts ||= []).push({ name: name.slice(5), value });
    else if (name.startsWith("radius-")) (out.radii ||= []).push({ name: name.slice(7), value });
    else if (name.startsWith("shadow-")) (out.shadows ||= []).push({ name: name.slice(7), value });
    else if (name.startsWith("space-") || name.startsWith("spacing-")) (out.spacing ||= []).push({ name: name.replace(/^spac(?:e|ing)-/, ""), value });
    else if (name.startsWith("text-")) (out.typography ||= []).push({ name: name.slice(5), size: value });
    else if (looksShadow(name, value)) (out.shadows ||= []).push({ name, value });
    else if (isColorVal(value)) (out.colors ||= []).push({ name, value });
    else if (looksFontStack(name, value)) (out.fonts ||= []).push({ name, value });
    else if (looksRadius(name) && isLenVal(value)) (out.radii ||= []).push({ name, value });
    else if (isLenVal(value) && /space|spacing|gap|gutter|inset|pad|margin|size/i.test(name)) (out.spacing ||= []).push({ name, value });
  }
  return out;
}

// The prototype's dark-theme token overrides: every custom property declared under a `.dark`
// rule (e.g. `.dark{--color-bg:#0b0b0c}`), keyed by the raw var name (without the `--`). The
// Design-system tab uses this to show a colour's dark value when previewing the dark theme.
// Returns {} when the system has no `.dark` block (light-only prototype).
//
// NOTE: this used to scan for the bare substring ".dark" and grab everything up to the NEXT
// `{`, same bug class as tokensFromCss's old isRootSel — Tailwind v4's
// `@custom-variant dark (&:where(.dark, .dark *));` directive puts ".dark" earlier in the
// file than the real `.dark{}` rule, so on the current theme.css scaffold this used to
// capture the `@theme{}` block's tokens instead of the real dark overrides. Use the same
// block-splitting + last-statement selector check as tokensFromCss so only an actual
// `.dark { ... }` rule (not incidental text before it) is matched.
export function darkVars(css?: string): Record<string, string> {
  const out: Record<string, string> = {};
  if (!css || !css.trim()) return out;
  css = stripComments(css);
  const isDarkSel = (sel: string) => {
    const parts = sel.split(";");
    const stmt = parts[parts.length - 1].trim();
    return stmt.split(",").some((s) => {
      const t = s.trim();
      return t === ".dark" || /(^|[\s>~+])\.dark\b/.test(t);
    });
  };
  const body = [...css.matchAll(/([^{}]+)\{([^}]*)\}/g)]
    .filter((m) => isDarkSel(m[1]))
    .map((m) => m[2])
    .join(";");
  for (const m of body.matchAll(/--([\w-]+)\s*:\s*([^;]+)/g)) {
    const name = m[1].trim();
    if (!(name in out)) out[name] = m[2].trim();
  }
  return out;
}
