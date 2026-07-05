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
