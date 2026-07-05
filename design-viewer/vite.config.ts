import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { artaWatch } from "./vite/arta-watch";
import { protoApp } from "./vite/proto-app";

// The running viewer version, read at dev-server start from this package's own
// package.json (Conclave vendoring deviation: upstream read the Claude plugin
// manifest, which this vendored copy does not ship) — surfaced in the UI
// (Topbar) via the __ARTA_VERSION__ define below.
const ARTA_VERSION = (() => {
  try {
    const p = fileURLToPath(new URL("./package.json", import.meta.url));
    return JSON.parse(readFileSync(p, "utf8")).version || "dev";
  } catch {
    return "dev";
  }
})();

const dep = (name: string) => fileURLToPath(new URL(`./node_modules/${name}`, import.meta.url));
// The curated set (ADR-0002). Screens live in USER projects; bare imports must
// resolve to the VIEWER's single copy — two Reacts crash hooks at runtime.
const CURATED = ["react", "react-dom", "react-router-dom", "motion", "lucide-react", "recharts", "clsx", "tailwind-merge"];

// Arta dev server.
// The `artaWatch` plugin is the core of the whole concept: it watches the
// shared canvas file (.arta/state.json) that the AI writes into, and pushes
// every change to the running viewer over Vite's WebSocket — no MCP, no manual
// reload. The AI just uses its normal Write tool; the screen updates instantly.
export default defineConfig({
  define: { __ARTA_VERSION__: JSON.stringify(ARTA_VERSION) },
  plugins: [react(), tailwindcss(), artaWatch(), protoApp()],
  resolve: {
    dedupe: CURATED,
    alias: Object.fromEntries(CURATED.map((n) => [n, dep(n)])),
  },
  optimizeDeps: { include: CURATED },
  server: {
    port: Number(process.env.CONCLAVE_DESIGN_PORT) || 7343,
    strictPort: false,
    // Conclave vendoring deviation: loopback only — the engine spawns this
    // sidecar locally and embeds it via iframe; it is never meant to be
    // reachable over LAN or a tunnel like upstream's standalone `arta` CLI.
    host: "127.0.0.1",
    fs: {
      // Registered projects live in arbitrary Conclave workspace folders
      // anywhere on disk, never under this package's own root — Vite's
      // default allow-list (this root only) 403s every /@fs/ screen import
      // for a real project (proven empirically: a scratch project's
      // proto/screens/home.tsx 403'd until this was added).
      //
      // SECURITY POSTURE (corrected per review — the prior comment here was
      // wrong): `fs.strict: false` makes Vite serve ANY absolute path via
      // `GET /@fs/<abspath>`, completely independent of registry.json —
      // the registry only gates the project SWITCHER list, not this route.
      // Safety rests SOLELY on the `host: "127.0.0.1"` binding above, which
      // blocks LAN access but NOT a same-machine attacker (any other local
      // process, or a webpage the user's browser opens — the classic
      // dev-server localhost-file-read / DNS-rebind vector: while this
      // sidecar runs, an untrusted page could fetch
      // `http://127.0.0.1:<port>/@fs/Users/<you>/.ssh/id_rsa`). Accepted for
      // Phase 1 as a dev-mode posture (see VENDORING.md); a Host/Origin guard
      // is tracked as a follow-up, not implemented here.
      strict: false,
    },
  },
});
