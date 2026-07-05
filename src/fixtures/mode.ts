/** Fixture mode: DEV-only, opted into per page-load via `?fixture=<scenario>`.
 *  Returns the scenario name, or null when the app should talk to the real
 *  Tauri host. Never true in a production build. */
export function fixtureScenario(): string | null {
  if (!import.meta.env.DEV) return null;
  const v = new URLSearchParams(window.location.search).get("fixture");
  return v && v.length > 0 ? v : null;
}
