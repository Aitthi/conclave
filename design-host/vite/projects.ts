import fs from "node:fs";
import os from "node:os";
import path from "node:path";

// ── Shared project registry ──────────────────────────────────────────────
// One host process (one port) can show several workspaces — each Conclave workspace
// upserts itself into a shared registry.json ({id,name,dir}) via the engine's
// `design.ensure` IPC command; this module is the single source of truth for the id
// hash + registry shape. The id hash (idFor below) is reimplemented byte-for-byte in
// Rust (engine/runtime/design_host.rs) — the two MUST keep computing the same id for
// the same dir; do not "improve" the hash without updating both sides and their
// cross-language test.
const REGISTRY_HOME = process.env.CONCLAVE_DESIGN_HOME || path.join(os.homedir(), ".conclave", "design-host");
export const REGISTRY_FILE = path.join(REGISTRY_HOME, "registry.json");

export interface Project {
  id: string;
  name: string;
  dir: string;
}

// Stable short id from an absolute project dir (FNV-1a → base36). MUST match the Rust
// reimplementation in engine/runtime/design_host.rs. Do not "improve" the hash.
export function idFor(dir: string): string {
  const s = path.resolve(dir);
  let h = 2166136261 >>> 0;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619) >>> 0;
  }
  return h.toString(36);
}

// Parses the shared registry file into validated Project entries, [] on any failure
// (missing file, bad JSON, not an array). Each entry's id is recomputed fresh via
// idFor(e.dir) rather than trusted from disk, so a stored id can never disagree with
// what idFor(dir) would compute elsewhere.
export function readRegistry(): Project[] {
  let raw: unknown;
  try {
    raw = JSON.parse(fs.readFileSync(REGISTRY_FILE, "utf8"));
  } catch {
    return [];
  }
  if (!Array.isArray(raw)) return [];
  const out: Project[] = [];
  for (const e of raw) {
    if (e && typeof e.dir === "string") {
      const dir = path.resolve(e.dir);
      out.push({ id: idFor(dir), name: typeof e.name === "string" ? e.name : "", dir });
    }
  }
  return out;
}

// Resolves a project id to its registered dir (the workspace's `design/` folder —
// see commands/design.rs's registry entry). Unknown ids resolve to null.
export function resolveProjectDir(projectId: string): string | null {
  const entry = readRegistry().find((p) => p.id === projectId);
  return entry ? entry.dir : null;
}
