import path from "node:path";
import type { Plugin, ResolvedConfig, ViteDevServer } from "vite";
import { manifestCode } from "./proto-manifest";
import { resolveProjectDir } from "./projects";
import { FONT_LINK, withThaiFallback } from "../src/lib/prototype";

const VIRT = "virtual:arta-proto-manifest/";
const RESOLVED = "\0" + VIRT;

// Pre-rewrite main's compileTokens() ran withThaiFallback() over EVERY font token before
// it ever reached CSS — a write-time transform that no longer exists (ADR-0001 deleted
// the whole pipeline it lived in). This is the ADR-0004-shaped replacement: a transform
// on theme.css itself, in the ONE place every consumer (live iframe, headless snapshot,
// PDF export, static export) resolves it through — so `--font-*` tokens keep the same
// snapshot↔live glyph-metrics guarantee `withThaiFallback` was written for, without the
// agent's on-disk theme.css needing to spell out the Thai fallback by hand. Only
// `--font-*` custom properties are touched; every other token (colors, radii, shadows,
// prose) passes through byte-for-byte.
export function injectThaiFallback(css: string): string {
  return css.replace(/(--font-[\w-]*\s*:\s*)([^;]+)(;)/g, (_m, pre, value, post) => `${pre}${withThaiFallback(value.trim())}${post}`);
}

// Compiles each project's .arta/proto/ through the viewer's own Vite server.
// The proto entry (proto/main.tsx) dynamic-imports this virtual module with the
// project id from its URL; we generate /@fs/ imports because project dirs live
// OUTSIDE the Vite root (the viewer runs from the installed package, projects
// are wherever the user's repos are — see ~/.arta/registry.json).
export function protoApp(): Plugin {
  let server: ViteDevServer | undefined;
  let homeDir = "";

  const invalidate = (projectId: string) => {
    const mod = server?.moduleGraph.getModuleById(RESOLVED + projectId);
    if (mod) server!.moduleGraph.invalidateModule(mod);
    // Screen list / config / theme changed shape → the proto page must reload.
    server?.ws.send({ type: "full-reload", path: "/proto/index.html" });
  };

  return {
    name: "arta-proto-app",
    // `pre` so our transform below sees theme.css's raw `--font-*: "Geist", ...;` text
    // BEFORE @tailwindcss/vite's own transform touches the same file — order doesn't
    // change what Tailwind ultimately emits (we only rewrite custom-property VALUES,
    // never add/remove a token), but running after would mean guessing at whatever
    // intermediate shape Tailwind's own CSS pipeline produces instead of the plain
    // source the agent actually wrote.
    enforce: "pre",
    transform(code, id) {
      // Match the plain path AND Vite's `?direct`/`?url`-suffixed CSS module ids alike —
      // both resolve back to the same on-disk theme.css.
      if (!/\/theme\.css(?:$|\?)/.test(id)) return undefined;
      const transformed = injectThaiFallback(code);
      return transformed === code ? undefined : { code: transformed, map: null };
    },
    // Both HTML shells (the viewer's own index.html AND proto/index.html) need the same
    // font `<link>`s — FONT_LINK (src/lib/prototype.ts) is the one place that list is
    // written down. Runs for every HTML page Vite processes (`vite dev`, and the static
    // export's `vite build` — same configFile, see export-static.ts), so both shells stay
    // byte-identical without hand-copying the Google Fonts URL into each.
    transformIndexHtml(html) {
      return html.replace("</head>", `${FONT_LINK}</head>`);
    },
    // Fires in BOTH `vite dev` and `vite build` (unlike configureServer, which is
    // dev-only and never runs during a programmatic build() — Task 12's static
    // export). homeDir must be resolved here so load() below works during a build
    // too, instead of silently falling through to the shared registry.
    configResolved(resolvedConfig: ResolvedConfig) {
      // Canonical project dir = the .arta dir ITSELF (registry `dir`, idFor
      // input, resolveProjectDir return) — matches arta-watch.ts. Conclave
      // vendoring deviation: no ARTA_DIR env (see arta-watch.ts) — this always
      // resolves to a phantom, never-real home dir; every real project comes
      // through resolveProjectDir's registry lookup instead.
      homeDir = path.join(resolvedConfig.root, ".arta");
    },
    configureServer(srv) {
      // Server-only wiring: capturing the live server for invalidate() and exposing
      // the invalidation hook arta-watch.ts calls on file changes. Meaningless during
      // a one-shot build (no server, no live clients to push updates to) — this hook
      // simply never runs in build mode, so nothing below needs an extra guard.
      server = srv;
      // arta-watch already watches every project's .arta dir; it calls
      // __artaProtoInvalidate (set below) on proto-shaped changes (Task 6 wiring).
      (globalThis as Record<string, unknown>).__artaProtoInvalidate = invalidate;
    },
    resolveId(id) {
      if (id.startsWith(VIRT)) return "\0" + id;
      if (id.startsWith("/@id/" + VIRT)) return "\0" + id.slice("/@id/".length);
      return undefined;
    },
    load(id) {
      if (!id.startsWith(RESOLVED)) return undefined;
      const pid = id.slice(RESOLVED.length);
      const artaDir = resolveProjectDir(pid, homeDir); // the .arta dir, NOT the project root
      if (!artaDir) return `export const config={};export const metas={};export const screens={};export const components={};`;
      return manifestCode(path.join(artaDir, "proto"));
    },
  };
}
