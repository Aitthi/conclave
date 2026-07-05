#!/usr/bin/env node
// Design viewer sidecar launcher.
//
// Conclave vendoring deviation: replaces upstream's bin/arta.mjs. This process
// serves EVERY workspace from one Vite dev server (registry.json-driven
// multi-project switching, see vite/projects.ts) — it is not launched against
// a single project directory, has no bun-install-and-retry fallback (the
// engine supervisor owns `pnpm install`, see runtime/design_viewer.rs), and
// does no process-matching/GC of other viewer instances (the engine owns
// lifecycle: one supervised child, restarted on crash, killed on app exit).
//
// Usage: node bin/viewer.mjs [--port <p>]
import path from "node:path";
import { fileURLToPath } from "node:url";

const pkgRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const args = process.argv.slice(2);
const opt = (name, def) => {
  const i = args.indexOf(name);
  return i >= 0 && args[i + 1] ? args[i + 1] : def;
};
const port = Number(opt("--port", process.env.CONCLAVE_DESIGN_PORT || "7343"));

const { createServer } = await import("vite");
const server = await createServer({
  root: pkgRoot,
  configFile: path.join(pkgRoot, "vite.config.ts"),
  server: { port },
});
await server.listen();

// strictPort is off, so Vite bumps to the next free port on a collision — the
// engine's health check parses THIS line for the port it actually bound to,
// never the one it asked for.
const actualPort = server.httpServer?.address()?.port ?? port;
console.log(`DESIGN_VIEWER_READY port=${actualPort}`);
