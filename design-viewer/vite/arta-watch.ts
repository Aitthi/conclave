import fs from "node:fs";
import path from "node:path";
import type { Plugin, ViteDevServer } from "vite";
import type { FrameKind, Prototype, Screen } from "../src/lib/types";
import { idFor, readRegistry, REGISTRY_FILE, type Project } from "./projects";
import { listComponents, listScreens, readConfig } from "./proto-manifest";
import { scaffoldProto } from "./scaffold";
import { detectSlop, detectSlopJsx } from "./slop-detect.mjs";

type GateFinding = ReturnType<typeof detectSlop>[number];

// ── Shared canvas layout (.arta/) ───────────────────────────────────────
// state.json                    meta/spec/plan/dataModel/flow/api/architecture — NEVER
//                                the prototype (see below).
// proto/config.json             start screen + default frame/safeArea/chrome
// proto/theme.css               Tailwind v4 @theme tokens, imported into every screen
// proto/screens/<id>.tsx        each screen (file name = screen id, `export const meta`)
// proto/components/<name>.tsx   shared components (design-system gallery)
// legacy-html-backup/           ADR-0001: old prototype/ dir, renamed here on migration
// legacy-prototype.json         ADR-0001: old inline state.json `prototype`, extracted here
// runtime.json / feedback.json  viewer→agent channel (written by this plugin)
//
// state.prototype is ALWAYS assembled fresh from proto/ (assembleState below), never
// read from state.json — so the client stays unaware of the on-disk split, and an
// agent's slim state.json write can never accidentally stomp the prototype.
//
// ── Multiple projects, one viewer ──────────────────────────────────────
// One viewer process (one port) can show several projects. Each project's MCP
// upserts itself into a shared registry at ~/.arta/registry.json ({id,name,dir});
// this plugin serves any of them (state/runtime/feedback/snapshot take ?project=<id>)
// and watches them all, tagging live pushes with the project id. The client picks the
// active project (persisted in localStorage) and ignores pushes for the others. With a
// single project the registry holds just the home project and everything behaves as
// before.
const ARTA_DIR = ".arta";
const STATE_FILE = "state.json";
const RUNTIME_FILE = "runtime.json";
const FEEDBACK_FILE = "feedback.json";
const PROTO_DIR = "prototype"; // OLD legacy freeform-HTML layout (ADR-0001 migrates it away)
const CSS_FILE = "design-system.css";
const COMP_DIR = "components";
const SCREEN_DIR = "screens";
const NEW_PROTO_DIR = "proto"; // NEW React-screens layout (vite/proto-manifest.ts)
const THEME_FILE = "theme.css";
const CONFIG_FILE = "config.json";
const LEGACY_BACKUP_DIR = "legacy-html-backup";
const LEGACY_PROTOTYPE_FILE = "legacy-prototype.json";

function readJson(file: string): any | null {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch {
    return null;
  }
}
function readRaw(file: string): string | null {
  try {
    return fs.readFileSync(file, "utf8");
  } catch {
    return null;
  }
}
const sanitize = (s: string) => String(s).replace(/[^a-z0-9_-]/gi, "");

// `dir` is a project's .arta dir, so the human label comes from its state's
// meta.name, then the registry's name, then the PARENT folder (the project dir).
function displayName(dir: string, fallback?: string): string {
  const meta = readJson(path.join(dir, STATE_FILE))?.meta;
  return (meta && typeof meta.name === "string" && meta.name.trim()) || fallback || path.basename(path.dirname(dir));
}

// Build state.prototype fresh from `.arta/proto/` (Task 1's manifest helpers) every
// time — never from whatever shape state.json happens to hold. `dir` is a project's
// .arta dir. The rest of state.json (meta/spec/plan/dataModel/flow/api/architecture/
// adrs/feedback-related fields) passes through unchanged; only `prototype` is derived.
export function assembleState(dir: string): Record<string, unknown> | null {
  const raw = readRaw(path.join(dir, STATE_FILE));
  if (raw == null) return null;
  let state: Record<string, unknown>;
  try {
    state = JSON.parse(raw);
  } catch {
    return null;
  }
  // Discard whatever old-shaped prototype value might still be sitting in state.json
  // (e.g. before migration has run) — state.prototype is ALWAYS derived from disk.
  delete state.prototype;

  const protoDir = path.join(dir, NEW_PROTO_DIR);
  const screens: Screen[] = listScreens(protoDir).map((s) => ({
    id: s.id,
    title: s.meta.title ?? s.id,
    frame: s.meta.frame,
    safeArea: s.meta.safeArea,
    chrome: s.meta.chrome,
    url: s.meta.url,
  }));
  const config = readConfig(protoDir);
  const themeCss = readRaw(path.join(protoDir, THEME_FILE)) ?? undefined;
  const componentNames = listComponents(protoDir).map((c) => c.name);
  // legacy flips back to false the moment a real screen exists in .arta/proto/screens/,
  // even though the backup dir is never deleted — so we never need to track a separate
  // "migration done" flag, this is computed fresh on every read.
  const legacy = fs.existsSync(path.join(dir, LEGACY_BACKUP_DIR)) && screens.length === 0;

  const prototype: Prototype = {
    start: config.start,
    frame: config.frame as FrameKind | undefined,
    safeArea: config.safeArea,
    chrome: config.chrome,
    themeCss,
    screens,
    componentNames,
    legacy,
  };
  state.prototype = prototype;
  return state;
}

// Should configureServer's first-run bootstrap scaffold a starter `.arta/proto/` for
// this home project? `dir` is the project's .arta dir. False whenever a React canvas
// already exists (NEW_PROTO_DIR), a legacy HTML canvas is still awaiting migration
// (PROTO_DIR), OR migration has already renamed that legacy dir to LEGACY_BACKUP_DIR —
// scaffolding over the last case would create a placeholder home.tsx, flipping
// assembleState's `legacy` flag false and losing the ADR-0001 regenerate notice on any
// viewer restart before the agent writes real screens.
export function shouldScaffoldHome(dir: string): boolean {
  return (
    !fs.existsSync(path.join(dir, NEW_PROTO_DIR)) &&
    !fs.existsSync(path.join(dir, PROTO_DIR)) &&
    !fs.existsSync(path.join(dir, LEGACY_BACKUP_DIR))
  );
}

// ── Legacy migration (ADR-0001) ──────────────────────────────────────────────
// Non-destructive, idempotent: only renames/copies, never deletes. Runs once per
// project — subsequent calls are cheap no-ops once migrated (or never needed).
// `dir` is a project's .arta dir.
export function migrateLegacyProto(dir: string): void {
  const legacyDir = path.join(dir, PROTO_DIR);
  const hasLegacyDir = fs.existsSync(legacyDir);
  const stateFile = path.join(dir, STATE_FILE);
  const state = readJson(stateFile) as Record<string, unknown> | null;
  const proto = state && typeof state === "object" ? (state as Record<string, unknown>).prototype : undefined;
  const protoScreens = proto && typeof proto === "object" ? (proto as { screens?: unknown[] }).screens : undefined;
  const hasLegacyHtml = Array.isArray(protoScreens) && protoScreens.some((s) => s && typeof (s as { html?: unknown }).html === "string");

  // Cheap common-case guard: nothing here means either already migrated or never a
  // legacy project — skip the (already-cheap, but avoidable) rest of the checks below.
  if (!hasLegacyDir && !hasLegacyHtml) return;

  const backupDir = path.join(dir, LEGACY_BACKUP_DIR);
  if (hasLegacyDir && !fs.existsSync(backupDir)) {
    fs.renameSync(legacyDir, backupDir);
  }

  const legacyProtoFile = path.join(dir, LEGACY_PROTOTYPE_FILE);
  if (proto !== undefined && !fs.existsSync(legacyProtoFile)) {
    fs.writeFileSync(legacyProtoFile, JSON.stringify(proto, null, 2));
    const rest = { ...(state as Record<string, unknown>) };
    delete rest.prototype;
    fs.writeFileSync(stateFile, JSON.stringify(rest, null, 2));
  }
}

// ── runtime.json read-modify-write ──────────────────────────────────────────
// runtime.json has TWO independent writers that must not clobber each other:
//   1. the client's `/__arta/runtime` POST — fires on nearly every tab/screen/error
//      change in the live viewer, and knows nothing about `gates`.
//   2. this plugin's own watcher-driven gate scan (below) — knows nothing about the
//      client's live tab/screen/error state.
// Both read the file first and only overwrite the field(s) they own, so a heartbeat
// from one writer can never wipe out the other's last write.
function readRuntime(dir: string): Record<string, unknown> {
  return (readJson(path.join(dir, RUNTIME_FILE)) as Record<string, unknown>) || {};
}
function writeRuntimePatch(dir: string, patch: Record<string, unknown>): void {
  const merged = { ...readRuntime(dir), ...patch };
  fs.writeFileSync(path.join(dir, RUNTIME_FILE), JSON.stringify(merged, null, 2));
}

// Run the slop-detector over every screen + component source in a project's proto/,
// plus theme.css. Pure regex, no compile step — fast enough to run synchronously on
// every qualifying watcher event (see srv.watcher.on("all", ...) below).
//
// theme.css judgment call: scanned with the PLAIN `detectSlop` entry point (not
// `detectSlopJsx` — it's not JSX) rather than skipped. Several gates target exactly
// the shape a Tailwind v4 `@theme` block has (`--color-bg:#hex`, hex literals) —
// cream-palette and ai-color-palette in particular are precisely aimed at a token's
// default background/accent, so this is real signal, not noise. Gates that need
// element/class co-occurrence (nested-cards, icon-tile-stack, etc.) simply never
// match a token file's syntax, so they contribute nothing either way.
//
// NOTE — compile/JS errors are NOT part of `gates`. A Vite/esbuild transform error in
// a screen can't be caught here (this only ever sees text that parses enough to read
// off disk); those surface through the EXISTING `errors` path instead — see the long
// comment on `scanProjectGates`'s caller in the watcher handler below.
function scanProjectGates(proj: Project): GateFinding[] {
  const protoDir = path.join(proj.dir, NEW_PROTO_DIR);
  const findings: GateFinding[] = [];
  for (const s of listScreens(protoDir)) {
    const src = readRaw(s.file);
    if (src != null) findings.push(...detectSlopJsx(src, { file: path.relative(protoDir, s.file) }));
  }
  for (const c of listComponents(protoDir)) {
    const src = readRaw(c.file);
    if (src != null) findings.push(...detectSlopJsx(src, { file: path.relative(protoDir, c.file) }));
  }
  const themeCss = readRaw(path.join(protoDir, THEME_FILE));
  if (themeCss != null) findings.push(...detectSlop(themeCss, { file: THEME_FILE }));
  return findings;
}

function send(res: import("node:http").ServerResponse, code: number, body: unknown) {
  res.statusCode = code;
  res.setHeader("Content-Type", "application/json");
  res.setHeader("Cache-Control", "no-store");
  res.end(JSON.stringify(body));
}

function readBody(req: import("node:http").IncomingMessage): Promise<string> {
  return new Promise((resolve, reject) => {
    let data = "";
    req.on("data", (c) => {
      data += c;
      if (data.length > 4_000_000) reject(new Error("payload too large"));
    });
    req.on("end", () => resolve(data));
    req.on("error", reject);
  });
}

export function artaWatch(): Plugin {
  let homeDir = "";
  let homeId = "";
  let projects: Project[] = [];
  let projectDirs: string[] = [];

  // A file is a design source if it lives under SOME known project dir and isn't one
  // of the viewer→agent output files (runtime/feedback/snapshots).
  const isArtaSource = (file: string) => {
    const r = path.resolve(file);
    if (!projectDirs.some((d) => r === d || r.startsWith(d + path.sep))) return false;
    const base = path.basename(r);
    if (base === RUNTIME_FILE || base === FEEDBACK_FILE) return false;
    if (r.split(path.sep).includes("snapshots")) return false;
    return true;
  };
  const projectForFile = (file: string): Project | undefined => {
    const r = path.resolve(file);
    return projects.find((p) => r === p.dir || r.startsWith(p.dir + path.sep));
  };
  const projectById = (id: string | null): Project | undefined =>
    (id && projects.find((p) => p.id === id)) || undefined;

  // The home project (this process was launched for) is always present and first;
  // registry entries follow, but only if their canvas still exists on disk.
  const loadProjects = (): Project[] => {
    const seen = new Map<string, Project>();
    const add = (dir: string, name: string | undefined, requireState: boolean) => {
      const rd = path.resolve(dir);
      const id = idFor(rd);
      if (seen.has(id)) return;
      // Runs for every project discovered here (home + every registry entry, on every
      // refreshProjects() call) — cheap no-op once a project is migrated or never needed
      // it, so this covers newly-registered projects too without extra plumbing.
      migrateLegacyProto(rd);
      if (requireState && !fs.existsSync(path.join(rd, STATE_FILE))) return;
      seen.set(id, { id, name: displayName(rd, name), dir: rd });
    };
    // Conclave vendoring deviation: no per-launch home project (one sidecar
    // serves every workspace, all registered via registry.json), so `homeDir`
    // never corresponds to a real project — requiring state.json here (like
    // any registry entry) keeps it out of the switcher instead of showing a
    // permanent phantom entry with no canvas.
    add(homeDir, path.basename(path.dirname(homeDir)), true);
    for (const e of readRegistry()) add(e.dir, e.name, true);
    return [...seen.values()];
  };

  return {
    name: "arta-watch",

    // Suppress Vite's default full-page reload for any .arta source change —
    // we push an in-place update over the WebSocket instead (keeps the dev's
    // current tab/screen). The actual push happens in the watcher handler below.
    handleHotUpdate(ctx) {
      if (isArtaSource(ctx.file)) return [];
      return;
    },

    configureServer(srv: ViteDevServer) {
      // Conclave vendoring deviation: no ARTA_DIR env (the engine never
      // launches this sidecar for one specific project — see the requireState
      // change on the `add(homeDir, ...)` call below), always <root>/.arta —
      // a directory that structurally never exists for the vendored package.
      homeDir = path.resolve(srv.config.root, ARTA_DIR);
      homeId = idFor(homeDir);

      // Conclave vendoring deviation: no home-project auto-scaffold. Upstream
      // bootstrapped a starter canvas for the ONE project `arta` was launched
      // against; this sidecar has no such project (see above) — every real
      // project's `.arta/` is scaffolded by the engine's `design.ensure`
      // command instead, before this sidecar is even asked to start.

      // Pre-create the home snapshots dir so chokidar watches files created later. The
      // `.arta/proto/{screens,components}` equivalent is no longer pre-created here —
      // scaffoldProto() above already creates it for a brand-new home project, and
      // pre-creating it unconditionally would make `.arta/prototype/` (the OLD legacy
      // dir name, PROTO_DIR) exist for every project, wrongly tripping the legacy
      // migration below. Non-home, non-legacy projects simply have no `.arta/proto/`
      // until scaffolded or written by an agent — assembleState degrades gracefully.
      fs.mkdirSync(path.join(homeDir, "snapshots"), { recursive: true });

      const watchProject = (p: Project) => {
        srv.watcher.add(path.join(p.dir, STATE_FILE));
        srv.watcher.add(path.join(p.dir, PROTO_DIR));
        srv.watcher.add(path.join(p.dir, NEW_PROTO_DIR));
      };
      const refreshProjects = () => {
        projects = loadProjects();
        const next = projects.map((p) => p.dir);
        for (const p of projects) if (!projectDirs.includes(p.dir)) watchProject(p);
        projectDirs = next;
      };
      refreshProjects();
      srv.watcher.add(REGISTRY_FILE);

      // Describe an edit so the viewer can show a legible "what the AI changed" feed.
      // Classifies both the NEW `.arta/proto/` layout and the OLD `.arta/prototype/`
      // layout (a legacy project's backup dir may still get incidental fs events —
      // this degrades to the "other" fallback rather than crashing on either).
      const describeChange = (file: string, event: string, dir: string) => {
        const r = path.resolve(file);
        const verb = event === "unlink" ? "removed" : event === "add" ? "added" : "updated";
        const protoDir = path.join(dir, NEW_PROTO_DIR);
        if (r.startsWith(path.join(protoDir, SCREEN_DIR) + path.sep)) {
          const id = path.basename(r).replace(/\.[^./]+$/, "");
          return { kind: "screen", id, label: `${verb} screen: ${id}` };
        }
        if (r.startsWith(path.join(protoDir, COMP_DIR) + path.sep)) {
          const name = path.basename(r).replace(/\.[^./]+$/, "");
          return { kind: "component", id: name, label: `${verb} component: ${name}` };
        }
        if (r === path.join(protoDir, THEME_FILE)) return { kind: "theme", label: `${verb} theme` };
        if (r === path.join(protoDir, CONFIG_FILE)) return { kind: "config", label: `${verb} config` };
        if (r.startsWith(path.join(dir, PROTO_DIR, SCREEN_DIR))) {
          const id = path.basename(r, ".html");
          return { kind: "screen", id, label: `${verb} screen: ${id}` };
        }
        if (r.startsWith(path.join(dir, PROTO_DIR, COMP_DIR))) {
          const name = path.basename(r, ".html");
          return { kind: "component", id: name, label: `${verb} component: ${name}` };
        }
        if (r === path.join(dir, PROTO_DIR, CSS_FILE)) return { kind: "designSystem", label: `${verb} design system` };
        if (r === path.join(dir, STATE_FILE)) return { kind: "state", label: `${verb} spec / plan / data / flow` };
        return { kind: "other", label: `${verb} ${path.basename(r)}` };
      };

      const pushProjects = () =>
        srv.ws.send({ type: "custom", event: "arta:projects", data: projects.map((p) => ({ id: p.id, name: p.name })) });

      srv.watcher.on("all", (event, file) => {
        const r = path.resolve(file);
        if (r === path.resolve(REGISTRY_FILE)) {
          refreshProjects();
          pushProjects();
          return;
        }
        if ((event === "add" || event === "change" || event === "unlink") && isArtaSource(file)) {
          const proj = projectForFile(file);
          if (!proj) return;
          const change = describeChange(file, event, proj.dir);
          // The proto manifest (virtual module the /proto page imports) is a CACHE keyed
          // by project id — it must be invalidated whenever a proto/ change reshapes it:
          // a screen/component file appearing or disappearing, or theme/config changing.
          // A content-only edit to an EXISTING screen/component is left alone here —
          // Vite's own module HMR already handles that, no manifest-shape change.
          const underNewProto = r === path.join(proj.dir, NEW_PROTO_DIR) || r.startsWith(path.join(proj.dir, NEW_PROTO_DIR) + path.sep);
          const shapeChanged =
            underNewProto &&
            (((change.kind === "screen" || change.kind === "component") && event !== "change") ||
              change.kind === "theme" ||
              change.kind === "config");

          // Gate scan: broader than `shapeChanged` above — a content-only edit to an
          // EXISTING screen/component doesn't reshape the manifest, but it absolutely
          // needs a gate re-scan, since the edited content is exactly what the gates
          // check. `config.json` is skipped here (it holds no scannable JSX/CSS source),
          // even though it still trips `shapeChanged` above.
          let label = change.label;
          const needsGateScan = underNewProto && (change.kind === "screen" || change.kind === "component" || change.kind === "theme");
          if (needsGateScan) {
            const allGates = scanProjectGates(proj);
            writeRuntimePatch(proj.dir, { gates: allGates });
            // Append the finding count for the SPECIFIC file that changed (not the
            // project-wide total) to the change label, e.g.
            // "updated screen: cart · 2 gate findings".
            const relFile = path.relative(path.join(proj.dir, NEW_PROTO_DIR), r);
            const fileFindings = allGates.filter((g) => g.file === relFile);
            if (fileFindings.length) {
              label = `${label} · ${fileFindings.length} gate finding${fileFindings.length === 1 ? "" : "s"}`;
            }
          }
          srv.ws.send({ type: "custom", event: "arta:change", data: { project: proj.id, ...change, label } });

          if (shapeChanged) {
            (globalThis as { __artaProtoInvalidate?: (id: string) => void }).__artaProtoInvalidate?.(proj.id);
          }
          const assembled = assembleState(proj.dir);
          if (assembled != null) srv.ws.send({ type: "custom", event: "arta:update", data: JSON.stringify({ project: proj.id, state: assembled }) });
          // A state.json write may have set/changed meta.name → keep the switcher labels fresh.
          if (path.basename(r) === STATE_FILE) {
            const fresh = displayName(proj.dir, proj.name);
            if (fresh !== proj.name) { proj.name = fresh; pushProjects(); }
          }
        }
      });

      // Resolve ?project=<id> to a project dir, defaulting to home.
      const dirFor = (req: import("node:http").IncomingMessage): string => {
        const q = (req.url || "").split("?")[1] || "";
        const id = new URLSearchParams(q).get("project");
        return projectById(id)?.dir ?? homeDir;
      };

      srv.middlewares.use(async (req, res, next) => {
        const url = (req.url || "").split("?")[0];

        // The list of projects this viewer can show (home first).
        if (url === "/__arta/projects" && req.method === "GET") {
          return send(res, 200, { ok: true, home: homeId, projects: projects.map((p) => ({ id: p.id, name: p.name })) });
        }

        // Drop a project from the switcher: removes its entry from the shared registry.
        // This unregisters only — it never touches the project's .arta/ on disk, so the
        // canvas stays intact and the project reappears here if its MCP writes again.
        // The home project (the one this viewer was launched for) is always re-added by
        // loadProjects(), so deleting it is refused.
        if (url === "/__arta/projects" && req.method === "DELETE") {
          const id = new URLSearchParams((req.url || "").split("?")[1] || "").get("project");
          if (!id) return send(res, 400, { ok: false, error: "missing ?project" });
          if (id === homeId) return send(res, 409, { ok: false, error: "the launched project stays pinned" });
          const reg = readJson(REGISTRY_FILE);
          if (Array.isArray(reg)) {
            const next = reg.filter((e) => e && idFor(String(e.dir)) !== id);
            try {
              fs.writeFileSync(REGISTRY_FILE, JSON.stringify(next, null, 2));
            } catch (e) {
              return send(res, 500, { ok: false, error: String(e) });
            }
          }
          refreshProjects();
          pushProjects();
          return send(res, 200, { ok: true, projects: projects.map((p) => ({ id: p.id, name: p.name })) });
        }

        // Initial state load for the viewer — fully assembled from the split files.
        if (url === "/__arta/state" && req.method === "GET") {
          const dir = dirFor(req);
          const raw = readRaw(path.join(dir, STATE_FILE));
          if (raw == null) return send(res, 200, { ok: true, state: null });
          try {
            JSON.parse(raw);
          } catch (e) {
            return send(res, 200, { ok: false, error: String(e) });
          }
          return send(res, 200, { ok: true, state: assembleState(dir) });
        }

        // Viewer reports which tab/screen the dev is looking at — into that project's dir.
        // This fires on nearly every tab/screen/error change, so it must NOT blindly
        // overwrite the whole file: `gates` is written by this plugin's own watcher-driven
        // slop scan (scanProjectGates, above) and the client never sends it — a blind
        // overwrite here would wipe it out on the very next heartbeat. Read the existing
        // file first and keep whatever `gates` is already on disk untouched.
        //
        // Compile/JS errors: a Vite/esbuild transform error in a screen surfaces through
        // THIS `errors` array, not through `gates` (gates are purely the regex
        // slop-detector's findings) — see proto/shell/bridge.ts's window.onerror/
        // unhandledrejection forwarding and Shell.tsx's ScreenHost, whose dynamic
        // `import()` `.catch()` reports a failed module load through the same
        // `reportError()` path. Both postMessage into src/components/proto/ProtoDevice.tsx,
        // which App.tsx relays into this exact POST body's `errors` field.
        if (url === "/__arta/runtime" && req.method === "POST") {
          try {
            const body = JSON.parse((await readBody(req)) || "{}");
            const dir = dirFor(req);
            const runtime = {
              ...readRuntime(dir),
              tab: body.tab ?? null,
              screen: body.screen ?? null,
              screens: Array.isArray(body.screens) ? body.screens : [],
              phase: body.phase ?? null,
              errors: Array.isArray(body.errors) ? body.errors : [],
              updatedAt: body.updatedAt ?? null,
              reportedAt: new Date().toISOString(),
            };
            fs.writeFileSync(path.join(dir, RUNTIME_FILE), JSON.stringify(runtime, null, 2));
            return send(res, 200, { ok: true });
          } catch (e) {
            return send(res, 400, { ok: false, error: String(e) });
          }
        }

        // Dev leaves feedback from inside the viewer → appended to that project's queue.
        if (url === "/__arta/feedback" && req.method === "POST") {
          try {
            const body = JSON.parse((await readBody(req)) || "{}");
            const text = String(body.text || "").trim();
            if (!text) return send(res, 400, { ok: false, error: "empty feedback" });
            const feedbackFile = path.join(dirFor(req), FEEDBACK_FILE);
            const list = (readJson(feedbackFile) as unknown[]) || [];
            list.push({
              text,
              tab: body.tab ?? null,
              screen: body.screen ?? null,
              element: body.element ?? null,
              at: new Date().toISOString(),
              read: false,
            });
            fs.writeFileSync(feedbackFile, JSON.stringify(list, null, 2));
            return send(res, 200, { ok: true, count: list.length });
          } catch (e) {
            return send(res, 400, { ok: false, error: String(e) });
          }
        }

        if (url === "/__arta/feedback" && req.method === "GET") {
          const list = (readJson(path.join(dirFor(req), FEEDBACK_FILE)) as { read?: boolean }[]) || [];
          return send(res, 200, { ok: true, pending: list.filter((f) => !f.read).length });
        }

        // Viewer captures the rendered screen and hands the agent a real picture.
        if (url === "/__arta/snapshot" && req.method === "POST") {
          try {
            const body = JSON.parse((await readBody(req)) || "{}");
            const id = sanitize(String(body.screen || ""));
            const data = String(body.dataUrl || "");
            const m = data.match(/^data:image\/png;base64,(.+)$/);
            if (!id || !m) return send(res, 400, { ok: false, error: "expected screen + png dataUrl" });
            const snapDir = path.join(dirFor(req), "snapshots");
            fs.mkdirSync(snapDir, { recursive: true });
            // `.full.png` = the whole screen at content length; `.png` = the framed viewport.
            fs.writeFileSync(path.join(snapDir, id + (body.full ? ".full.png" : ".png")), Buffer.from(m[1], "base64"));
            return send(res, 200, { ok: true });
          } catch (e) {
            return send(res, 400, { ok: false, error: String(e) });
          }
        }

        // Scaffold this project's `.arta/proto/` starter (config.json, theme.css,
        // screens/home.tsx, lib/, components/) if it doesn't have one yet. Shares
        // scaffoldProto with this plugin's own first-run bootstrap (configureServer,
        // above) so both paths use the EXACT same templates — the MCP `arta_doctor`
        // tool POSTs here EVERY SESSION (SKILL.md calls it first, unconditionally), when
        // the viewer is up instead of duplicating them (it only inlines a minimal
        // fallback when the viewer is unreachable; see server.mjs). Because doctor calls
        // this on every session start — not just a rare cold-boot — this is the SAME
        // shouldScaffoldHome guard configureServer's bootstrap uses above: without it, a
        // migrated-but-not-yet-regenerated legacy project gets a placeholder home.tsx on
        // its very first post-migration doctor call, flipping assembleState's `legacy`
        // flag false and losing the ADR-0001 regenerate notice before the agent ever
        // sees it. `legacy` in the response tells the caller (arta_doctor) this project
        // still needs regenerating even when scaffolding was skipped.
        if (url === "/__arta/scaffold" && req.method === "POST") {
          try {
            const dir = dirFor(req);
            if (!shouldScaffoldHome(dir)) {
              const state = assembleState(dir) as { prototype?: { legacy?: boolean } } | null;
              return send(res, 200, { ok: true, created: [], scaffolded: false, legacy: !!state?.prototype?.legacy });
            }
            const result = scaffoldProto(path.dirname(dir));
            return send(res, 200, { ok: true, created: result.created, scaffolded: true, legacy: false });
          } catch (e) {
            return send(res, 500, { ok: false, error: String((e as Error)?.message || e) });
          }
        }

        next();
      });
    },
  };
}
