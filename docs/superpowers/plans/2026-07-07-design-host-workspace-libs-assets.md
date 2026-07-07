# design-host: per-workspace libs + public assets — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

owner: bfb737ff-486d-4581-b407-95711d5e07ab (Detoro) · authority: in-loop
spec: `docs/superpowers/specs/2026-07-07-design-host-workspace-libs-assets-design.md` (commit 27f6d97)
escalation: design/spec conflicts → Detoro via `conclave task challenge`; implementation judgment within this plan → implementer, logged as task notes.

**Goal:** A workspace's `design/` dir can declare its own npm deps (`design/package.json`, auto-installed by the host) and serve static assets (relative imports from `design/assets/`, absolute URLs from `design/public/`), without ever duplicating the curated React stack.

**Architecture:** All changes live in the Node-side host (`design-host/`): a `resolveId` gate in the `designApp()` Vite plugin resolves non-curated bare imports from the workspace's own `node_modules` (via Vite's condition-aware resolver) and fails loudly otherwise; a watcher-driven installer runs pnpm/npm in the workspace `design/` dir; a connect middleware serves `design/public/` at `/p/<projectId>/…`. The Rust engine is untouched.

**Tech Stack:** Vite 6 plugin API, node:test + esbuild data-URL pattern (existing convention in `design-host/test/screen-selection.test.mjs`), no new npm dependencies.

## Global Constraints

- **No new npm dependencies** in `design-host/package.json` — hand-roll the MIME map, use `node:test`.
- **CURATED set gets ONE source of truth**: new `design-host/vite/curated.ts`; `vite.config.ts` and all new code import it. The list itself does not change: `react, react-dom, react-router-dom, motion, lucide-react, recharts, clsx, tailwind-merge`.
- **Do not touch `src-tauri/src/`** — installs are host-side by spec D2. The Rust supervisor already handles host boot.
- **All log/error copy in English** (workspace convention).
- **Tests create dirs with `fs.mkdtempSync(path.join(os.tmpdir(), "dh-"))`** — never a fixed global temp path (flakes under concurrent agents).
- **Run all commands from `design-host/`** unless stated. Fresh lane worktrees need `pnpm install` in `design-host/` first (esbuild is a devDep the tests use).
- Test command shape: `node --test test/*.test.mjs` (a bare directory arg does NOT work on this Node).
- Gates before READY: `node --test test/*.test.mjs` and `pnpm typecheck`, both green, recorded via `conclave task gate <ws> design-host-workspace-libs -- sh -c "cd design-host && node --test test/*.test.mjs"` (and the typecheck twin).

## File Structure

- Create `design-host/vite/curated.ts` — curated list + package-name helpers (pure, testable).
- Create `design-host/vite/workspace-deps.ts` — importer→project-dir mapping (pure core, testable).
- Create `design-host/vite/workspace-install.ts` — manager choice, staleness marker, serialized installer.
- Create `design-host/vite/public-assets.ts` — path guard (pure, testable) + middleware.
- Modify `design-host/vite.config.ts` — import CURATED from `vite/curated.ts`.
- Modify `design-host/vite/host-app.ts` — wire resolver gate, installer watch, assets middleware.
- Modify `design-host/src/main.tsx` — expose `window.__DESIGN_PUBLIC_BASE__`.
- Create `design-host/test/{curated,workspace-deps,workspace-install,public-assets,e2e-workspace}.test.mjs`.
- Create `design-host/README.md` — the authoring contract (libs + assets).
- Modify `src-tauri/skills/design-canvas/SKILL.md` (~line 43) — teach the new contract.

---

### Task 1: `curated.ts` — single source of truth

**Files:**
- Create: `design-host/vite/curated.ts`
- Test: `design-host/test/curated.test.mjs`
- Modify: `design-host/vite.config.ts`

**Interfaces:**
- Produces: `CURATED: string[]`, `packageName(specifier: string): string`, `isCurated(specifier: string): boolean` — consumed by Tasks 2 and 5.

- [ ] **Step 1: Write the failing test** — `design-host/test/curated.test.mjs`:

```js
import assert from "node:assert/strict";
import fs from "node:fs";
import { test } from "node:test";
import { transformSync } from "esbuild";

const source = fs.readFileSync(new URL("../vite/curated.ts", import.meta.url), "utf8");
const moduleUrl = `data:text/javascript,${encodeURIComponent(transformSync(source, { loader: "ts" }).code)}`;
const { CURATED, packageName, isCurated } = await import(moduleUrl);

test("curated list is the R6 set, unchanged", () => {
  assert.deepEqual(CURATED, [
    "react", "react-dom", "react-router-dom", "motion",
    "lucide-react", "recharts", "clsx", "tailwind-merge",
  ]);
});

test("packageName handles plain, subpath, and scoped specifiers", () => {
  assert.equal(packageName("react"), "react");
  assert.equal(packageName("react-dom/client"), "react-dom");
  assert.equal(packageName("@tanstack/react-table"), "@tanstack/react-table");
  assert.equal(packageName("@tanstack/react-table/build/lib"), "@tanstack/react-table");
});

test("isCurated matches by package name, not prefix", () => {
  assert.ok(isCurated("react-dom/client"));
  assert.ok(!isCurated("react-table"));
  assert.ok(!isCurated("@tanstack/react-table"));
});
```

- [ ] **Step 2: Run to verify it fails** — `node --test test/curated.test.mjs` → FAIL (module not found).

- [ ] **Step 3: Implement** — `design-host/vite/curated.ts`:

```ts
// Single source of truth for the curated authoring set (R6 — Arta authoring
// parity). vite.config.ts aliases + dedupes these to THIS package's single copy
// (two Reacts crash hooks); workspace-deps.ts refuses to resolve them from a
// workspace's own node_modules for the same reason. Keep in lockstep with the
// design-canvas skill's taught list (src-tauri/skills/design-canvas/SKILL.md).
export const CURATED = [
  "react",
  "react-dom",
  "react-router-dom",
  "motion",
  "lucide-react",
  "recharts",
  "clsx",
  "tailwind-merge",
];

// "@scope/name/deep/path" → "@scope/name"; "name/deep" → "name".
export function packageName(specifier: string): string {
  const parts = specifier.split("/");
  return specifier.startsWith("@") ? parts.slice(0, 2).join("/") : parts[0];
}

export function isCurated(specifier: string): boolean {
  return CURATED.includes(packageName(specifier));
}
```

- [ ] **Step 4: Point `vite.config.ts` at it** — replace the inline `const CURATED = [ … ];` block (keep the explanatory comment above it) with:

```ts
import { CURATED } from "./vite/curated";
```

- [ ] **Step 5: Verify** — `node --test test/curated.test.mjs` → PASS; `pnpm typecheck` → clean.

- [ ] **Step 6: Commit** — `git add design-host/vite/curated.ts design-host/test/curated.test.mjs design-host/vite.config.ts && git commit -m "refactor(design-host): extract CURATED set into vite/curated.ts"`

---

### Task 2: workspace dep resolver

**Files:**
- Create: `design-host/vite/workspace-deps.ts`
- Test: `design-host/test/workspace-deps.test.mjs`
- Modify: `design-host/vite/host-app.ts` (the `resolveId` hook)

**Interfaces:**
- Consumes: `isCurated` from Task 1; `Project`/`readRegistry` from `vite/projects.ts`.
- Produces: `isBareImport(id: string): boolean`, `projectDirFor(importer: string | undefined, projects: {dir: string}[]): string | null`, `missingDepMessage(specifier: string, designDir: string): string` — consumed by host-app.ts and Task 5.

- [ ] **Step 1: Write the failing test** — `design-host/test/workspace-deps.test.mjs` (same esbuild data-URL pattern; `workspace-deps.ts` must import ONLY `node:path` so the pattern works — registry entries are passed in as an argument):

```js
import assert from "node:assert/strict";
import fs from "node:fs";
import { test } from "node:test";
import { transformSync } from "esbuild";

const source = fs.readFileSync(new URL("../vite/workspace-deps.ts", import.meta.url), "utf8");
const moduleUrl = `data:text/javascript,${encodeURIComponent(transformSync(source, { loader: "ts" }).code)}`;
const { isBareImport, projectDirFor, missingDepMessage } = await import(moduleUrl);

test("isBareImport", () => {
  assert.ok(isBareImport("@tanstack/react-table"));
  assert.ok(isBareImport("date-fns/format"));
  assert.ok(!isBareImport("./sidebar"));
  assert.ok(!isBareImport("../lib/cn"));
  assert.ok(!isBareImport("/@fs/Users/x/design/screens/a.tsx"));
  assert.ok(!isBareImport("virtual:design-host-manifest/abc"));
  assert.ok(!isBareImport("\0virtual:design-host-manifest/abc"));
});

test("projectDirFor maps /@fs/ and plain absolute importers to their registered dir", () => {
  const projects = [{ dir: "/work/ket-doc/design" }, { dir: "/work/other/design" }];
  assert.equal(projectDirFor("/@fs//work/ket-doc/design/screens/app.tsx", projects), "/work/ket-doc/design");
  assert.equal(projectDirFor("/work/other/design/components/x.tsx?t=123", projects), "/work/other/design");
  assert.equal(projectDirFor("/somewhere/else/file.tsx", projects), null);
  assert.equal(projectDirFor(undefined, projects), null);
});

test("missingDepMessage names the package and the exact file to edit", () => {
  const msg = missingDepMessage("@tanstack/react-table", "/work/ket-doc/design");
  assert.match(msg, /@tanstack\/react-table/);
  assert.match(msg, /\/work\/ket-doc\/design\/package\.json/);
  assert.match(msg, /installed automatically/);
});
```

- [ ] **Step 2: Run to verify it fails** — `node --test test/workspace-deps.test.mjs` → FAIL.

- [ ] **Step 3: Implement** — `design-host/vite/workspace-deps.ts`:

```ts
import path from "node:path";

// Spec D1 (2026-07-07 libs+assets spec): non-curated bare imports from a screen
// resolve against THAT workspace's design/node_modules — explicitly, so failure
// is an actionable message instead of a bare Vite resolve error. Pure helpers
// only (node:path); the registry is passed in by the caller so tests need no fs.

// Bare = npm-style specifier: not relative, not absolute, not a URL/virtual id.
export function isBareImport(id: string): boolean {
  if (!id || id.includes("\0")) return false;
  if (id.startsWith(".") || id.startsWith("/") || id.startsWith("virtual:")) return false;
  return /^[a-zA-Z@]/.test(id);
}

// Vite importer ids arrive either as plain absolute paths or as "/@fs/<abs>"
// URLs (files outside the Vite root), possibly with a query suffix. The /@fs
// form is often "/@fs//Users/…" (manifestCode concatenates "/@fs/" + abspath),
// and POSIX path.resolve PRESERVES a leading double slash — collapse it here.
export function projectDirFor(
  importer: string | undefined,
  projects: { dir: string }[],
): string | null {
  if (!importer) return null;
  let file = importer.split("?")[0];
  if (file.startsWith("/@fs/")) file = file.slice("/@fs".length).replace(/^\/+/, "/");
  const resolved = path.resolve(file);
  for (const p of projects) {
    if (resolved.startsWith(p.dir + path.sep)) return p.dir;
  }
  return null;
}

export function missingDepMessage(specifier: string, designDir: string): string {
  return (
    `[design-host] "${specifier}" is not installed in this workspace — ` +
    `add it to ${path.join(designDir, "package.json")} and it will be ` +
    `installed automatically (ESM packages are the supported target).`
  );
}
```

- [ ] **Step 4: Verify unit tests pass** — `node --test test/workspace-deps.test.mjs` → PASS.

- [ ] **Step 5: Wire into `host-app.ts`** — replace the existing `resolveId` method with (imports at top: `import { readRegistry } from "./projects";`, `import { isCurated } from "./curated";`, `import { isBareImport, projectDirFor, missingDepMessage } from "./workspace-deps";`):

```ts
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
```

- [ ] **Step 6: Verify** — `pnpm typecheck` → clean; `node --test test/*.test.mjs` → all PASS.

- [ ] **Step 7: Commit** — `git add design-host/vite/workspace-deps.ts design-host/test/workspace-deps.test.mjs design-host/vite/host-app.ts && git commit -m "feat(design-host): resolve non-curated bare imports from workspace design/node_modules"`

---

### Task 3: auto-install workspace deps

**Files:**
- Create: `design-host/vite/workspace-install.ts`
- Test: `design-host/test/workspace-install.test.mjs`
- Modify: `design-host/vite/host-app.ts` (project registration + watcher)

**Interfaces:**
- Produces: `parseWorkableVersion(stdout: string): boolean`, `needsInstall(designDir: string): boolean`, `writeMarker(designDir: string): void`, `ensureInstalled(designDir: string, opts?): Promise<boolean>` — host-app.ts consumes `needsInstall`/`ensureInstalled`.

- [ ] **Step 1: Write the failing test** — `design-host/test/workspace-install.test.mjs`:

```js
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { transformSync } from "esbuild";

const source = fs.readFileSync(new URL("../vite/workspace-install.ts", import.meta.url), "utf8");
const moduleUrl = `data:text/javascript,${encodeURIComponent(transformSync(source, { loader: "ts" }).code)}`;
const { parseWorkableVersion, needsInstall, writeMarker } = await import(moduleUrl);

test("parseWorkableVersion tolerates rc-file noise, rejects junk", () => {
  assert.ok(parseWorkableVersion("11.5.2\n"));
  assert.ok(parseWorkableVersion("nvm is loading...\n11.5.2\n"));
  assert.ok(!parseWorkableVersion(""));
  assert.ok(!parseWorkableVersion("TypeError: Invalid host defined options"));
});

test("needsInstall: no package.json → false; fresh → true; marker current → false; pkg edited → true", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "dh-"));
  assert.equal(needsInstall(dir), false);
  fs.writeFileSync(path.join(dir, "package.json"), '{"dependencies":{}}');
  assert.equal(needsInstall(dir), true);
  fs.mkdirSync(path.join(dir, "node_modules"), { recursive: true });
  writeMarker(dir);
  assert.equal(needsInstall(dir), false);
  const future = new Date(Date.now() + 5000);
  fs.utimesSync(path.join(dir, "package.json"), future, future);
  assert.equal(needsInstall(dir), true);
});
```

- [ ] **Step 2: Run to verify it fails** — `node --test test/workspace-install.test.mjs` → FAIL.

- [ ] **Step 3: Implement** — `design-host/vite/workspace-install.ts`:

```ts
import { spawn, spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

// Spec D2: the long-lived host process owns workspace-dep installs. Manager
// choice mirrors the Rust-side design-host-node-guard ruling (9f11f98):
// pnpm only when WORKABLE — a corepack shim can exist on PATH yet crash on an
// old node, so presence is not workability. Anything else → npm.

export type Manager = "pnpm" | "npm";

const MARKER = ".design-host-install.json";

// Some line of stdout must be a bare semver (nvm hooks love to chat first).
export function parseWorkableVersion(stdout: string): boolean {
  return stdout.split("\n").some((l) => /^\d+\.\d+\.\d+$/.test(l.trim()));
}

export function chooseManager(): Manager {
  try {
    const r = spawnSync("pnpm", ["--version"], { encoding: "utf8", timeout: 15_000 });
    if (r.status === 0 && parseWorkableVersion(r.stdout ?? "")) return "pnpm";
  } catch {
    // fall through to npm
  }
  return "npm";
}

function markerPath(designDir: string): string {
  return path.join(designDir, "node_modules", MARKER);
}

// Stale = package.json exists and its mtime differs from the recorded one.
// No package.json at all means "nothing to install" — existing workspaces
// without one take exactly today's path (spec acceptance #4).
export function needsInstall(designDir: string): boolean {
  let st: fs.Stats;
  try {
    st = fs.statSync(path.join(designDir, "package.json"));
  } catch {
    return false;
  }
  try {
    const m = JSON.parse(fs.readFileSync(markerPath(designDir), "utf8"));
    return m.pkgMtimeMs !== st.mtimeMs;
  } catch {
    return true;
  }
}

export function writeMarker(designDir: string): void {
  const st = fs.statSync(path.join(designDir, "package.json"));
  fs.writeFileSync(markerPath(designDir), JSON.stringify({ pkgMtimeMs: st.mtimeMs }));
}

const inflight = new Map<string, Promise<boolean>>();

// Serialized per dir; resolves true iff deps are ready. Failures are logged
// (stderr tail) and false is returned — the manifest still loads, screens that
// don't need the new dep keep rendering (spec: error handling).
export function ensureInstalled(
  designDir: string,
  opts: { log?: (msg: string) => void } = {},
): Promise<boolean> {
  const log = opts.log ?? ((m) => console.error(m));
  const existing = inflight.get(designDir);
  if (existing) return existing;
  if (!needsInstall(designDir)) return Promise.resolve(true);

  const manager = chooseManager();
  log(`[design-host] installing workspace deps in ${designDir} via ${manager}`);
  const p = new Promise<boolean>((resolve) => {
    const child = spawn(manager, ["install"], { cwd: designDir, stdio: ["ignore", "pipe", "pipe"] });
    let tail = "";
    child.stderr.on("data", (d) => {
      tail = (tail + String(d)).slice(-2000);
    });
    child.on("error", (err) => {
      log(`[design-host] ${manager} install failed to start in ${designDir}: ${err.message}`);
      resolve(false);
    });
    child.on("exit", (code) => {
      if (code === 0) {
        try {
          writeMarker(designDir);
          log(`[design-host] workspace deps ready in ${designDir}`);
          resolve(true);
          return;
        } catch (err) {
          log(`[design-host] install marker write failed in ${designDir}: ${String(err)}`);
        }
      } else {
        log(`[design-host] ${manager} install exited ${code} in ${designDir}\n${tail}`);
      }
      resolve(false);
    });
  }).finally(() => inflight.delete(designDir));
  inflight.set(designDir, p);
  return p;
}
```

- [ ] **Step 4: Verify unit tests pass** — `node --test test/workspace-install.test.mjs` → PASS.

- [ ] **Step 5: Wire into `host-app.ts`** — two touch points (import `needsInstall, ensureInstalled` at top):

In `load()`, extend the first-registration block:

```ts
      if (!watchedDirs.has(dir)) {
        watchedDirs.add(dir);
        server?.watcher.add(path.join(dir, "screens"));
        server?.watcher.add(path.join(dir, "package.json"));
        if (needsInstall(dir)) {
          void ensureInstalled(dir).then((ok) => {
            if (ok) server?.ws.send({ type: "full-reload", path: "/index.html" });
          });
        }
      }
```

In `configureServer`'s watcher handler, add a branch BEFORE the `.tsx` filter:

```ts
      srv.watcher.on("all", (event, file) => {
        const r = path.resolve(file);
        for (const dir of watchedDirs) {
          if (r === path.join(dir, "package.json")) {
            if (event === "add" || event === "change") {
              void ensureInstalled(dir).then((ok) => {
                if (ok) srv.ws.send({ type: "full-reload", path: "/index.html" });
              });
            }
            return;
          }
        }
        if (event !== "add" && event !== "unlink") return;
        if (!r.endsWith(".tsx")) return;
        // …existing screens/ logic unchanged…
      });
```

- [ ] **Step 6: Verify** — `pnpm typecheck` → clean; `node --test test/*.test.mjs` → all PASS.

- [ ] **Step 7: Commit** — `git add design-host/vite/workspace-install.ts design-host/test/workspace-install.test.mjs design-host/vite/host-app.ts && git commit -m "feat(design-host): auto-install workspace design deps (pnpm-workable-else-npm)"`

---

### Task 4: public assets middleware + `__DESIGN_PUBLIC_BASE__`

**Files:**
- Create: `design-host/vite/public-assets.ts`
- Test: `design-host/test/public-assets.test.mjs`
- Modify: `design-host/vite/host-app.ts` (mount middleware), `design-host/src/main.tsx`

**Interfaces:**
- Produces: `resolvePublicPath(designDir: string, rest: string): string | null` (null = traversal), `publicAssets(): (req, res, next) => void` connect middleware; browser global `window.__DESIGN_PUBLIC_BASE__ = "/p/<projectId>"`.

- [ ] **Step 1: Write the failing test** — `design-host/test/public-assets.test.mjs` (test the pure guard; the middleware is covered by Task 5's e2e):

```js
import assert from "node:assert/strict";
import fs from "node:fs";
import { test } from "node:test";
import { transformSync } from "esbuild";

const source = fs.readFileSync(new URL("../vite/public-assets.ts", import.meta.url), "utf8");
const moduleUrl = `data:text/javascript,${encodeURIComponent(transformSync(source, { loader: "ts" }).code)}`;
const { resolvePublicPath, contentTypeFor } = await import(moduleUrl);

test("resolvePublicPath stays inside design/public", () => {
  assert.equal(resolvePublicPath("/w/design", "logo.png"), "/w/design/public/logo.png");
  assert.equal(resolvePublicPath("/w/design", "img/a.svg"), "/w/design/public/img/a.svg");
  assert.equal(resolvePublicPath("/w/design", "../theme.css"), null);
  assert.equal(resolvePublicPath("/w/design", "..%2F..%2Fetc/passwd"), null);
  assert.equal(resolvePublicPath("/w/design", "a/../../secret"), null);
  assert.equal(resolvePublicPath("/w/design", ""), null);
});

test("contentTypeFor known and unknown extensions", () => {
  assert.equal(contentTypeFor("/x/logo.png"), "image/png");
  assert.equal(contentTypeFor("/x/font.woff2"), "font/woff2");
  assert.equal(contentTypeFor("/x/blob.xyz"), "application/octet-stream");
});
```

- [ ] **Step 2: Run to verify it fails** — `node --test test/public-assets.test.mjs` → FAIL.

- [ ] **Step 3: Implement** — `design-host/vite/public-assets.ts`:

```ts
import fs from "node:fs";
import path from "node:path";
import type { IncomingMessage, ServerResponse } from "node:http";
import { resolveProjectDir } from "./projects";

// Spec D3: `design/public/` served at /p/<projectId>/<path> — the absolute-URL
// asset model that mirrors a real project's public/ dir. Traversal-guarded,
// no-store (design iteration, not production), 404 on unknown id or file.

const MIME: Record<string, string> = {
  ".png": "image/png", ".jpg": "image/jpeg", ".jpeg": "image/jpeg",
  ".gif": "image/gif", ".svg": "image/svg+xml", ".webp": "image/webp",
  ".avif": "image/avif", ".ico": "image/x-icon",
  ".woff2": "font/woff2", ".woff": "font/woff", ".ttf": "font/ttf", ".otf": "font/otf",
  ".css": "text/css", ".js": "text/javascript", ".json": "application/json",
  ".txt": "text/plain", ".pdf": "application/pdf",
  ".mp4": "video/mp4", ".webm": "video/webm",
};

export function contentTypeFor(file: string): string {
  return MIME[path.extname(file).toLowerCase()] ?? "application/octet-stream";
}

// null = empty or escapes design/public (decoded before resolving, so an
// encoded ../ cannot sneak past the prefix check).
export function resolvePublicPath(designDir: string, rest: string): string | null {
  if (!rest) return null;
  let decoded: string;
  try {
    decoded = decodeURIComponent(rest);
  } catch {
    return null;
  }
  const root = path.resolve(designDir, "public");
  const target = path.resolve(root, decoded);
  if (!target.startsWith(root + path.sep)) return null;
  return target;
}

export function publicAssets() {
  return (req: IncomingMessage, res: ServerResponse, next: () => void) => {
    const url = (req.url || "").split("?")[0];
    const m = /^\/p\/([a-z0-9]+)\/(.*)$/.exec(url);
    if (!m) return next();
    const dir = resolveProjectDir(m[1]);
    const target = dir ? resolvePublicPath(dir, m[2]) : null;
    if (dir && target === null && m[2]) {
      res.statusCode = 403;
      res.end("forbidden");
      return;
    }
    if (!dir || !target || !fs.existsSync(target) || !fs.statSync(target).isFile()) {
      res.statusCode = 404;
      res.end("not found");
      return;
    }
    res.statusCode = 200;
    res.setHeader("Content-Type", contentTypeFor(target));
    res.setHeader("Cache-Control", "no-store");
    fs.createReadStream(target).pipe(res);
  };
}
```

- [ ] **Step 4: Mount in `host-app.ts`** — in `configureServer`, right after the health-check middleware: `srv.middlewares.use(publicAssets());` (import at top).

- [ ] **Step 5: Expose the base in `src/main.tsx`** — replace the project-id parsing so it runs once, before the dynamic import:

```ts
const project = new URLSearchParams(location.search).get("project") ?? "";
// Spec D3: screens/lib build absolute asset URLs without hard-coding a project
// id — e.g. a workspace helper `asset(p) => `${window.__DESIGN_PUBLIC_BASE__}/${p}``.
(window as unknown as { __DESIGN_PUBLIC_BASE__: string }).__DESIGN_PUBLIC_BASE__ = `/p/${project}`;

const mod = await import(
  /* @vite-ignore */ `/@id/virtual:design-host-manifest/${project}`
);
```

- [ ] **Step 6: Verify** — `node --test test/public-assets.test.mjs` → PASS; `pnpm typecheck` → clean.

- [ ] **Step 7: Commit** — `git add design-host/vite/public-assets.ts design-host/test/public-assets.test.mjs design-host/vite/host-app.ts design-host/src/main.tsx && git commit -m "feat(design-host): serve design/public at /p/<projectId>, expose __DESIGN_PUBLIC_BASE__"`

---

### Task 5: end-to-end fixture test (no network)

**Files:**
- Test: `design-host/test/e2e-workspace.test.mjs`

**Interfaces:**
- Consumes: everything above via the real `vite.config.ts` (`createServer` from Vite's JS API).

- [ ] **Step 1: Write the test** — builds a throwaway workspace (fake ESM dep hand-written into `node_modules` — no network), points the registry at it via `CONCLAVE_DESIGN_HOME`, boots the real config on an ephemeral port, and asserts over HTTP:

```js
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { createServer } from "vite";
import { transformSync } from "esbuild";

const pkgRoot = path.dirname(path.dirname(new URL(import.meta.url).pathname));
const projectsSrc = fs.readFileSync(path.join(pkgRoot, "vite", "projects.ts"), "utf8");
const { idFor } = await import(
  `data:text/javascript,${encodeURIComponent(transformSync(projectsSrc, { loader: "ts" }).code)}`
);

function makeFixture() {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "dh-home-"));
  const design = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "dh-ws-")), "design");
  fs.mkdirSync(path.join(design, "screens"), { recursive: true });
  fs.mkdirSync(path.join(design, "public", "img"), { recursive: true });
  // Fake pre-installed ESM dep — auto-install is unit-tested; e2e only proves resolution.
  const dep = path.join(design, "node_modules", "demo-lib");
  fs.mkdirSync(dep, { recursive: true });
  fs.writeFileSync(path.join(dep, "package.json"), JSON.stringify({ name: "demo-lib", version: "1.0.0", type: "module", main: "index.js" }));
  fs.writeFileSync(path.join(dep, "index.js"), "export const label = 'from-demo-lib';");
  fs.writeFileSync(
    path.join(design, "screens", "demo.tsx"),
    "import { label } from 'demo-lib';\nexport default function Demo(){ return <div>{label}</div>; }\n",
  );
  fs.writeFileSync(path.join(design, "public", "img", "dot.svg"), "<svg xmlns='http://www.w3.org/2000/svg'/>");
  fs.writeFileSync(path.join(home, "registry.json"), JSON.stringify([{ id: "x", name: "fixture", dir: design }]));
  return { home, design, id: idFor(design) };
}

test("e2e: workspace dep resolves, missing dep errors, public assets guarded", async (t) => {
  const { home, design, id } = makeFixture();
  process.env.CONCLAVE_DESIGN_HOME = home;
  const server = await createServer({
    configFile: path.join(pkgRoot, "vite.config.ts"),
    root: pkgRoot,
    server: { port: 0 },
  });
  await server.listen();
  t.after(() => server.close());
  const base = `http://127.0.0.1:${server.config.server.port ?? server.httpServer.address().port}`;

  const manifest = await (await fetch(`${base}/@id/virtual:design-host-manifest/${id}`)).text();
  assert.match(manifest, /demo/);

  const screen = await server.transformRequest("/@fs" + path.join(design, "screens", "demo.tsx"));
  assert.ok(screen && screen.code.includes("node_modules/demo-lib"), "bare import must resolve into the workspace's node_modules");

  fs.writeFileSync(
    path.join(design, "screens", "broken.tsx"),
    "import x from 'not-installed-lib';\nexport default () => null;\n",
  );
  await assert.rejects(
    () => server.transformRequest("/@fs" + path.join(design, "screens", "broken.tsx")),
    /not installed in this workspace/,
  );

  assert.equal((await fetch(`${base}/p/${id}/img/dot.svg`)).status, 200);
  assert.equal((await fetch(`${base}/p/${id}/img/dot.svg`)).headers.get("content-type"), "image/svg+xml");
  assert.equal((await fetch(`${base}/p/${id}/missing.png`)).status, 404);
  assert.equal((await fetch(`${base}/p/${id}/..%2Ftheme.css`)).status, 403);
  assert.equal((await fetch(`${base}/p/zzzz/anything.png`)).status, 404);
});
```

- [ ] **Step 2: Run it** — `node --test test/e2e-workspace.test.mjs` → PASS. If the port/address line needs adjusting for Vite 6's `server.httpServer`, fix the test, not the product code. If `transformRequest` of `broken.tsx` resolves against the HOST's node_modules instead of erroring, that is the known root-fallback leak — tighten `resolveId` to also throw when Vite's resolution lands OUTSIDE both the workspace dir and the host package dir is NOT required; instead assert the error only for a package name that exists nowhere (as written: `not-installed-lib`).

- [ ] **Step 3: Run the whole suite + typecheck** — `node --test test/*.test.mjs` and `pnpm typecheck` → all green.

- [ ] **Step 4: Commit** — `git add design-host/test/e2e-workspace.test.mjs && git commit -m "test(design-host): e2e fixture — workspace deps + public assets over real vite server"`

---

### Task 6: docs + skill — teach the contract

**Files:**
- Create: `design-host/README.md`
- Modify: `src-tauri/skills/design-canvas/SKILL.md` (the import-contract passage around line 43)

- [ ] **Step 1: Write `design-host/README.md`** — sections (concise, English): what the host is (one paragraph, link the two specs); **Adding libraries** — `design/package.json`, auto-install (pnpm-workable-else-npm), curated 8 always come from the host (never install your own react), ESM packages are the supported target, the exact overlay error text you get on a missing dep; **Assets** — `design/assets/` + relative import (primary), `design/public/` + `window.__DESIGN_PUBLIC_BASE__` helper pattern with the `asset()` one-liner example; **Caveats** — CJS-only packages may fail (pick an ESM build), `Cache-Control: no-store`.

- [ ] **Step 2: Update `src-tauri/skills/design-canvas/SKILL.md`** — the passage that today says only the curated 8 resolve ("Anything else will not …") must now say: curated 8 are aliased to the host and must never be installed per-workspace; anything else is importable AFTER adding it to `design/package.json` (auto-installed, ESM target); assets via relative `design/assets/` import or `design/public/` + `__DESIGN_PUBLIC_BASE__`.

- [ ] **Step 3: Commit** — `git add design-host/README.md src-tauri/skills/design-canvas/SKILL.md && git commit -m "docs(design-host): authoring contract — workspace libs + assets"`

---

### Task 7: live verification on the real deployment (ket-doc)

No repo code changes — evidence gathering for the READY note. The ket-doc workspace is at `/Users/detoro/code/ket-doc/design` (already registered if the human has opened its Design view; otherwise register by opening it in the app).

- [ ] **Step 1:** Add a real ESM dep to ket-doc: in `/Users/detoro/code/ket-doc/design/package.json` (create it) put `{ "dependencies": { "@tanstack/react-table": "^8.20.0" } }`. Start/restart the design host (open the workspace's Design view in the packaged app, or run the host manually with `CONCLAVE_DESIGN_HOME` defaulted). Confirm the host log shows the install line and completes.
- [ ] **Step 2:** Add a scratch screen `screens/zz-e2e-check.tsx` that imports the dep, renders a tiny table, shows one image via relative `../assets/` import and one via `__DESIGN_PUBLIC_BASE__` from `public/` (drop any small png into both dirs). Load the Design view.
- [ ] **Step 3:** SCREENSHOT the rendered screen and LOOK at it (Read the image file) — both images visible, table rendered, no overlay error. Attach the shot path in the READY note (pixel-gate spirit; `pnpm uishot` does not cover the design host, so a manual shot is the evidence).
- [ ] **Step 4:** Delete the scratch screen + scratch assets from ket-doc (leave `package.json` if the human wants the dep; note what was left).
- [ ] **Step 5:** Regression: open a workspace WITHOUT `design/package.json` (this repo's own registered fixture or any other) — renders exactly as before, no install attempt in the log (spec acceptance #4).

---

## Risk ledger

- **Vite root-fallback leak:** `this.resolve` may find a dep in the HOST's own `node_modules` when the workspace lacks it (Vite also tries the config root). Accepted for v1 — deterministic and harmless (host deps are the curated stack's own tree); do NOT add a containment check unless it bites in Task 5.
- **CJS-only packages:** not prebundled for workspace deps → may break in-browser with `require is not defined`-style errors. Accepted limitation per spec; the README/skill must say "pick an ESM build". Do not attempt per-workspace prebundling in this lane.
- **`server.watcher` breadth:** we add per-file watches (`package.json`) — chokidar handles this fine, but the watcher callback now runs on EVERY fs event; keep the package.json branch cheap (path compare only).
- **Install storms:** `ensureInstalled` serializes per dir and re-arms only on mtime change; a failing install will not retry-loop (retries require another package.json edit). Intended.
- **`node --test test/`** (bare dir) does NOT work on this Node — always glob `test/*.test.mjs`.
- **Fresh lane worktree:** run `pnpm install` inside `design-host/` before anything (no node_modules in a new worktree).
- **e2e test env var:** `CONCLAVE_DESIGN_HOME` is read at module import of `projects.ts` — set it BEFORE `createServer` (the test does; don't reorder).
