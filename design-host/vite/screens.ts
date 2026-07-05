import fs from "node:fs";
import path from "node:path";

export interface Screen {
  id: string;
  file: string;
}

const sane = (s: string) => /^[a-z0-9_-]+$/i.test(s);

// Lists `<designDir>/screens/*.tsx`, sorted by id. `designDir` is a workspace's
// `design/` folder (the registered project dir — see vite/projects.ts).
export function listScreens(designDir: string): Screen[] {
  const dir = path.join(designDir, "screens");
  let files: string[] = [];
  try {
    files = fs.readdirSync(dir);
  } catch {
    return [];
  }
  return files
    .filter((f) => f.endsWith(".tsx") && sane(f.slice(0, -4)))
    .sort()
    .map((f) => ({ id: f.slice(0, -4), file: path.join(dir, f) }));
}

// The virtual module the app entry (src/main.tsx) dynamic-imports with the project id
// parsed from its own URL: lazy screen loaders keyed by id, plus the id list the
// switcher renders. Conceptually the host's screen-discovery contract (spec D6:
// "the host app lists screens/*.tsx via import.meta.glob") — implemented as generated
// static imports rather than a literal `import.meta.glob` call, since a project's
// `design/` dir lives outside this package's Vite root and is only known at request
// time (the project id in the URL query string), while `import.meta.glob`'s pattern
// argument must be a compile-time-static string. This is the same technique the Arta
// studio this replaces used for its own proto manifest (`vite/proto-manifest.ts`
// upstream) — an exhaustive enumeration of the same files a static glob would match.
export function manifestCode(designDir: string): string {
  const screens = listScreens(designDir);
  const out: string[] = [];
  // R6 (Arta authoring parity): pull in the workspace's Tailwind v4 sheet so
  // `@tailwindcss/vite` compiles its `@theme` tokens + the utilities the screens use.
  // A static top-of-module import (not a dynamic one) so Vite treats it as a real CSS
  // entry and injects the compiled stylesheet. Emitted only when the file exists — a
  // workspace without a theme.css simply renders unstyled screens rather than 500ing on
  // a missing import. The sheet's own `@source` globs (relative to this dir) decide
  // what gets scanned for utility classes.
  const themeFile = path.join(designDir, "theme.css");
  if (fs.existsSync(themeFile)) {
    out.push(`import ${JSON.stringify("/@fs/" + themeFile)};`);
  }
  out.push(`export const screens = {`);
  for (const s of screens) out.push(`  ${JSON.stringify(s.id)}: () => import(${JSON.stringify("/@fs/" + s.file)}),`);
  out.push(`};`);
  out.push(`export const screenIds = ${JSON.stringify(screens.map((s) => s.id))};`);
  return out.join("\n");
}
