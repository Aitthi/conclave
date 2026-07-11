import { marked } from "marked";

// ---------------------------------------------------------------------------
// markdownToDocument — turn a markdown artifact's source into a self-contained
// HTML document for rendering inside the ArtifactFrame sandbox.
//
// SECURITY: the returned document is rendered ONLY through <ArtifactFrame>,
// which runs it in a sandboxed, opaque-origin iframe and injects a strict CSP
// (`default-src 'none'`) via withSandboxCsp(). Markdown may contain raw HTML;
// inside that frame it is contained exactly like an ``html`` artifact — its
// scripts cannot reach the app DOM, and the CSP blocks all network (remote
// images/fonts/scripts won't load). We therefore deliberately do NOT inject
// markdown-derived HTML into the app's own DOM — the codebase keeps zero
// dangerouslySetInnerHTML, and this preserves that invariant.
//
// The frame's document background is transparent so the host iframe's
// `bg-surface` shows through; text/heading/border colors are chosen per theme
// because the sandboxed document cannot read the app's CSS custom properties.
// ---------------------------------------------------------------------------

function styleBlock(dark: boolean): string {
  const c = dark
    ? {
        text: "#d7dae0",
        heading: "#f2f4f8",
        muted: "#9aa1ac",
        link: "#7aa2f7",
        border: "rgba(255,255,255,0.12)",
        codeBg: "rgba(255,255,255,0.07)",
        codeBorder: "rgba(255,255,255,0.10)",
        quoteBar: "rgba(255,255,255,0.18)",
        thBg: "rgba(255,255,255,0.05)",
      }
    : {
        text: "#24292f",
        heading: "#111418",
        muted: "#616a75",
        link: "#2563eb",
        border: "rgba(0,0,0,0.12)",
        codeBg: "rgba(0,0,0,0.05)",
        codeBorder: "rgba(0,0,0,0.08)",
        quoteBar: "rgba(0,0,0,0.16)",
        thBg: "rgba(0,0,0,0.04)",
      };

  return `<style>
  :root { color-scheme: ${dark ? "dark" : "light"}; }
  * { box-sizing: border-box; }
  html, body { margin: 0; background: transparent; }
  body {
    padding: 20px 22px;
    color: ${c.text};
    font: 14px/1.65 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
    -webkit-font-smoothing: antialiased;
    word-wrap: break-word;
    overflow-wrap: anywhere;
  }
  h1, h2, h3, h4, h5, h6 { color: ${c.heading}; font-weight: 600; line-height: 1.3; margin: 1.4em 0 0.6em; }
  h1 { font-size: 1.7em; } h2 { font-size: 1.4em; } h3 { font-size: 1.2em; } h4 { font-size: 1.05em; }
  h1:first-child, h2:first-child, h3:first-child { margin-top: 0; }
  h1, h2 { padding-bottom: 0.3em; border-bottom: 1px solid ${c.border}; }
  p, ul, ol, blockquote, table, pre { margin: 0 0 0.9em; }
  a { color: ${c.link}; text-decoration: none; }
  a:hover { text-decoration: underline; }
  ul, ol { padding-left: 1.5em; }
  li { margin: 0.2em 0; }
  li > p { margin: 0.2em 0; }
  code {
    font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace;
    font-size: 0.88em;
    background: ${c.codeBg};
    border: 1px solid ${c.codeBorder};
    border-radius: 4px;
    padding: 0.12em 0.35em;
  }
  pre {
    background: ${c.codeBg};
    border: 1px solid ${c.codeBorder};
    border-radius: 8px;
    padding: 12px 14px;
    overflow-x: auto;
  }
  pre code { background: none; border: 0; padding: 0; font-size: 0.86em; }
  blockquote {
    margin-left: 0;
    padding: 0.1em 1em;
    color: ${c.muted};
    border-left: 3px solid ${c.quoteBar};
  }
  hr { border: 0; border-top: 1px solid ${c.border}; margin: 1.6em 0; }
  table { border-collapse: collapse; display: block; overflow-x: auto; }
  th, td { border: 1px solid ${c.border}; padding: 6px 11px; text-align: left; }
  th { background: ${c.thBg}; font-weight: 600; }
  img { max-width: 100%; }
</style>`;
}

/**
 * Render markdown source into a full HTML document string for <ArtifactFrame>.
 *
 * @param md   raw markdown source (nullish → empty document)
 * @param opts.dark  render with the dark-theme palette (default false); pass
 *                   `document.documentElement.classList.contains("dark")`.
 */
export function markdownToDocument(md: string, opts?: { dark?: boolean }): string {
  // marked v5+ is synchronous unless `async: true`; the literal `async: false`
  // selects the string-returning overload, so `inner` is a plain string (no cast).
  const inner = marked.parse(md ?? "", { async: false, gfm: true, breaks: false });
  return `<!doctype html><html><head><meta charset="utf-8">${styleBlock(
    opts?.dark ?? false,
  )}</head><body>${inner}</body></html>`;
}
