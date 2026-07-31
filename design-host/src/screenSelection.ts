// Selection restore precedence for the design canvas: URL hash (survives
// reload in-place) beats localStorage (survives tab close), beats default.
export function pickInitialScreen(
  hashScreen: string | null,
  stored: string | null,
  ids: string[],
): string | null {
  if (hashScreen && ids.includes(hashScreen)) return hashScreen;
  if (stored && ids.includes(stored)) return stored;
  return ids.find((id) => id === "welcome") ?? ids[0] ?? null;
}

export function parseHashScreen(hash: string): string | null {
  const match = /^#\/(.+)$/.exec(hash);
  return match ? decodeURIComponent(match[1]) : null;
}

// Screen-id filter behind the switcher popover's search box. Case-insensitive
// substring; a blank (or whitespace-only) query means "everything", so the list
// never collapses just because the user pressed space.
export function filterScreens(ids: string[], query: string): string[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return ids;
  return ids.filter((id) => id.toLowerCase().includes(needle));
}
