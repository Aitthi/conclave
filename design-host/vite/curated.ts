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
