import path from "node:path";
import type { Plugin, ViteDevServer } from "vite";
import { idFor, resolveProjectDir, readRegistry } from "./projects";
import { manifestCode } from "./screens";
import { isCurated } from "./curated";
import { isBareImport, projectDirFor, missingDepMessage } from "./workspace-deps";

const VIRT = "virtual:design-host-manifest/";
const RESOLVED = "\0" + VIRT;

// Compiles a project's `design/screens/` into the manifest the app entry
// (src/main.tsx) dynamic-imports with the project id parsed from its own URL.
// Generates `/@fs/` imports because project dirs live OUTSIDE this package's Vite
// root (workspaces are wherever the user's repos are — see the shared registry.json,
// vite/projects.ts).
export function designApp(): Plugin {
  let server: ViteDevServer | undefined;
  const watchedDirs = new Set<string>();

  const invalidate = (projectId: string) => {
    const mod = server?.moduleGraph.getModuleById(RESOLVED + projectId);
    if (mod) server!.moduleGraph.invalidateModule(mod);
    // The screen list changed shape (a file appeared/disappeared) — full reload is the
    // simplest correct response; a content-only edit to an existing screen is left to
    // Vite's ordinary module HMR instead (see the watcher handler below).
    server?.ws.send({ type: "full-reload", path: "/index.html" });
  };

  return {
    name: "conclave-design-host",

    configureServer(srv) {
      server = srv;

      // Health check the engine supervisor polls (runtime/design_host.rs's
      // wait_state_ok) — replaces the vendored Arta viewer's `/__arta/state` route now
      // that this host has no Arta state to report (D1: fully separate from Arta).
      srv.middlewares.use((req, res, next) => {
        const url = (req.url || "").split("?")[0];
        if (url === "/__design/health") {
          res.statusCode = 200;
          res.setHeader("Content-Type", "application/json");
          res.setHeader("Cache-Control", "no-store");
          res.end(JSON.stringify({ ok: true }));
          return;
        }
        next();
      });

      // A screen file appearing/disappearing reshapes the manifest for whichever
      // project's screens/ dir it lives under — content-only edits to an EXISTING
      // screen are left to Vite's ordinary module HMR (no manifest shape change).
      srv.watcher.on("all", (event, file) => {
        if (event !== "add" && event !== "unlink") return;
        const r = path.resolve(file);
        if (!r.endsWith(".tsx")) return;
        for (const dir of watchedDirs) {
          const screensDir = path.join(dir, "screens");
          if (r === screensDir || r.startsWith(screensDir + path.sep)) {
            invalidate(idFor(dir));
            return;
          }
        }
      });
    },

    async resolveId(id, importer) {
      if (id.startsWith(VIRT)) return "\0" + id;
      if (id.startsWith("/@id/" + VIRT)) return "\0" + id.slice("/@id/".length);
      // Spec D1: a non-curated bare import from inside a registered workspace
      // resolves via Vite's own condition-aware pipeline (which walks up from
      // the importer and finds design/node_modules) — we gate it so a miss
      // becomes an actionable error, not a cryptic overlay. Curated specifiers
      // never reach here meaningfully: resolve.alias rewrites them first.
      if (isBareImport(id) && !isCurated(id)) {
        const dir = projectDirFor(importer, readRegistry());
        if (dir) {
          const r = await this.resolve(id, importer, { skipSelf: true });
          if (r) return r;
          throw new Error(missingDepMessage(id, dir));
        }
      }
      return undefined;
    },

    load(id) {
      if (!id.startsWith(RESOLVED)) return undefined;
      const projectId = id.slice(RESOLVED.length);
      const dir = resolveProjectDir(projectId);
      if (!dir) return `export const screens={};export const screenIds=[];`;
      if (!watchedDirs.has(dir)) {
        watchedDirs.add(dir);
        server?.watcher.add(path.join(dir, "screens"));
      }
      return manifestCode(dir);
    },
  };
}
