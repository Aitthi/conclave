#!/usr/bin/env node
// Design host sidecar launcher.
//
// Conclave-native replacement for the vendored Arta viewer's bin/viewer.mjs. Serves
// EVERY workspace's design/ screens from one Vite dev server (registry.json-driven
// multi-project switching, see vite/projects.ts) — the engine supervisor
// (runtime/design_host.rs) owns spawn/health-check/restart/kill lifecycle, so this file
// only boots Vite and reports the port it actually bound to.
//
// Usage: node bin/host.mjs [--port <p>]
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

// strictPort is off, so Vite bumps to the next free port on a collision — the engine's
// health check parses THIS line for the port it actually bound to, never the requested one.
const actualPort = server.httpServer?.address()?.port ?? port;
console.log(`DESIGN_HOST_READY port=${actualPort}`);
