import fs from "node:fs";
import path from "node:path";

export type FrameKind = "web" | "desktop" | "ios" | "android" | "ipad";
const FRAME_KINDS: FrameKind[] = ["web", "desktop", "ios", "android", "ipad"];

export interface ScreenMeta {
  title?: string;
  frame?: FrameKind;
  safeArea?: string;
  chrome?: boolean;
  url?: string;
}
export interface ProtoScreen { id: string; file: string; meta: ScreenMeta }
export interface ProtoConfig { start?: string; frame?: string; safeArea?: string; chrome?: boolean }

// Agents sometimes write "device" instead of the documented "frame" key (SKILL.md
// used to describe the concept without ever naming the JSON key) — alias it in so
// existing/mistaken authoring doesn't silently fall back to the "web" default.
function normalizeFrame<T extends { frame?: string; device?: string }>(obj: T): T {
  const { device, ...rest } = obj as T & { device?: string };
  const raw = typeof rest.frame === "string" ? rest.frame : typeof device === "string" ? device : undefined;
  const lower = raw?.toLowerCase();
  const frame = FRAME_KINDS.includes(lower as FrameKind) ? (lower as FrameKind) : undefined;
  return { ...rest, frame } as T;
}

const META_RE = /export\s+const\s+meta\s*=\s*({[\s\S]*?})\s*(?:;|\n\s*export|\n\s*function|\n\s*const|$)/;

// meta MUST be a pure object literal (the authoring contract, ADR-0002) — evaluated
// in isolation, never as part of the module, so a screen that fails to compile still
// reports its title/frame to the viewer rail.
export function extractMeta(src: string): ScreenMeta {
  const m = src.match(META_RE);
  if (!m) return {};
  try {
    const v = new Function(`return (${m[1]})`)();
    return v && typeof v === "object" ? normalizeFrame(v as ScreenMeta) : {};
  } catch {
    return {};
  }
}

const sane = (s: string) => /^[a-z0-9_-]+$/i.test(s);

export function listScreens(protoDir: string): ProtoScreen[] {
  const dir = path.join(protoDir, "screens");
  let files: string[] = [];
  try { files = fs.readdirSync(dir); } catch { return []; }
  return files
    .filter((f) => f.endsWith(".tsx") && sane(f.slice(0, -4)))
    .sort()
    .map((f) => {
      const file = path.join(dir, f);
      let meta: ScreenMeta = {};
      try { meta = extractMeta(fs.readFileSync(file, "utf8")); } catch { /* unreadable → bare id */ }
      return { id: f.slice(0, -4), file, meta };
    });
}

export function listComponents(protoDir: string): { name: string; file: string }[] {
  const dir = path.join(protoDir, "components");
  try {
    return fs.readdirSync(dir)
      .filter((f) => f.endsWith(".tsx") && sane(f.slice(0, -4)))
      .sort()
      .map((f) => ({ name: f.slice(0, -4), file: path.join(dir, f) }));
  } catch { return []; }
}

export function readConfig(protoDir: string): ProtoConfig {
  try {
    const cfg = JSON.parse(fs.readFileSync(path.join(protoDir, "config.json"), "utf8"));
    return normalizeFrame(cfg);
  } catch { return {}; }
}

// The virtual module the proto entry imports: theme side-effect import, config,
// lazy screen loaders, meta map (for the shell's document.title etc.), components.
export function manifestCode(protoDir: string): string {
  const screens = listScreens(protoDir);
  const components = listComponents(protoDir);
  const theme = path.join(protoDir, "theme.css");
  const out: string[] = [];
  if (fs.existsSync(theme)) out.push(`import ${JSON.stringify("/@fs/" + theme)};`);
  let cfgRaw = "{}";
  try { cfgRaw = fs.readFileSync(path.join(protoDir, "config.json"), "utf8"); JSON.parse(cfgRaw); }
  catch { cfgRaw = "{}"; }
  out.push(`export const config = ${cfgRaw.trim()};`);
  out.push(`export const metas = ${JSON.stringify(Object.fromEntries(screens.map((s) => [s.id, s.meta])))};`);
  out.push(`export const screens = {`);
  for (const s of screens) out.push(`  ${JSON.stringify(s.id)}: () => import(${JSON.stringify("/@fs/" + s.file)}),`);
  out.push(`};`);
  out.push(`export const components = {`);
  for (const c of components) out.push(`  ${JSON.stringify(c.name)}: () => import(${JSON.stringify("/@fs/" + c.file)}),`);
  out.push(`};`);
  return out.join("\n");
}
