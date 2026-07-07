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
