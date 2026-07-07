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
// (stderr tail) and false is returned — the manifest still loads (spec: error
// handling), screens that don't need the new dep keep rendering.
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
