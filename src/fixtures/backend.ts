import type { Commands } from "../ipc/commands";
import { fixtureScenario } from "./mode";

export type FixtureHandlers = {
  [K in keyof Commands]?: (
    req: Commands[K]["req"],
  ) => Commands[K]["res"] | Promise<Commands[K]["res"]>;
};

// Scenario registry. Task F3 fills these modules in; keeping the imports lazy
// means the datasets load only on first fixture call.
const SCENARIOS: Record<string, () => Promise<{ handlers: FixtureHandlers }>> = {
  default: () => import("./scenarios/default"),
  empty: () => import("./scenarios/empty"),
};

/** Route an IPC call to the active scenario. `hit:false` means "not in fixture
 *  mode — caller should invoke the real host". An ACTIVE scenario with a
 *  missing handler THROWS (loudly visible in the page + uishot stderr), never
 *  silently falls through to Tauri. */
export async function maybeFixtureCall(
  cmd: keyof Commands,
  payload: unknown,
): Promise<{ hit: boolean; value?: unknown }> {
  const scenario = fixtureScenario();
  if (!scenario) return { hit: false };
  const load = SCENARIOS[scenario];
  if (!load) throw new Error(`[fixture] unknown scenario "${scenario}"`);
  const { handlers } = await load();
  const handler = handlers[cmd];
  if (!handler)
    throw new Error(
      `[fixture] no handler for command "${cmd}" in scenario "${scenario}"`,
    );
  return { hit: true, value: await handler(payload as never) };
}
