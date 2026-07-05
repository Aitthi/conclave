import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { designApp } from "./vite/host-app";

const dep = (name: string) => fileURLToPath(new URL(`./node_modules/${name}`, import.meta.url));
// Screens live in workspace `design/` folders, never in this package — bare imports
// must resolve to THIS package's single copy or two Reacts crash hooks at runtime (the
// same rationale the vendored Arta viewer this replaces used for its curated set).
const CURATED = ["react", "react-dom"];

// Conclave design-canvas host — spawned and supervised by the engine
// (`runtime/design_host.rs`), one process serving every workspace via the shared
// project registry (`vite/projects.ts`).
export default defineConfig({
  plugins: [react(), designApp()],
  resolve: {
    dedupe: CURATED,
    alias: Object.fromEntries(CURATED.map((n) => [n, dep(n)])),
  },
  optimizeDeps: { include: CURATED },
  server: {
    port: Number(process.env.CONCLAVE_DESIGN_PORT) || 7343,
    strictPort: false,
    // Loopback only — the engine spawns this sidecar locally and embeds it via iframe;
    // it is never meant to be reachable over LAN or a tunnel.
    host: "127.0.0.1",
    // SECURITY POSTURE — carried over verbatim from the Arta-embed ruling (4d81be26,
    // 2026-07-05) this D1-D6 swap does not reopen. `fs.strict: false` is required so
    // `/@fs/<abspath>` can serve screens from arbitrary workspace `design/` directories
    // outside this package's root; what closes the attack surface is this trio:
    // - `cors: false` — a page on a different localhost port cannot read a response
    //   cross-origin (Vite's default CORS is a permissive "any localhost origin" regex).
    // - explicit `allowedHosts` — pins the Host-header check.
    // - `fs.deny` — defense-in-depth against the common secret-file globs.
    // Residual, accepted risk: another same-user local process can still read
    // `/@fs/<abspath>` directly — a full Host/Origin guard middleware remains the
    // deferred follow-up (see the superseded `design-viewer-hardening` task, D7).
    cors: false,
    allowedHosts: ["127.0.0.1", "localhost"],
    fs: {
      strict: false,
      deny: ["**/.ssh/**", "**/.env", "**/.env.*", "**/id_rsa*", "**/*.pem"],
    },
  },
});
