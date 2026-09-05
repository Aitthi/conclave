import type { Commands } from "../ipc/commands";
import { fixtureScenario } from "./mode";

export type FixtureHandlers = {
  [K in keyof Commands]?: (
    req: Commands[K]["req"],
  ) => Commands[K]["res"] | Promise<Commands[K]["res"]>;
};

type ScenarioLoader = () => Promise<{ handlers: FixtureHandlers }>;

async function loadUsageScenario(
  variant: "loading" | "error" | "none" | "unsupported" | "partial" | "verified-empty",
): Promise<{ handlers: FixtureHandlers }> {
  const [{ handlers: base }, { createUsageHandlers }] = await Promise.all([
    import("./scenarios/default"),
    import("./scenarios/usage"),
  ]);
  return { handlers: { ...base, ...createUsageHandlers(variant) } };
}

async function loadWorkspaceScenario(
  variant: "empty" | "all-archived" | "loading" | "error" | "archive-error" | "archive-pending" | "restore-error" | "restore-pending" | "started" | "busy",
): Promise<{ handlers: FixtureHandlers }> {
  const [
    { handlers: base },
    { archivedWorkspaces, agents, workspaces },
    { createWorkspaceFixture },
  ] = await Promise.all([
    import("./scenarios/default"),
    import("./scenarios/data"),
    import("./scenarios/workspaces"),
  ]);

  const allArchived = [
    ...archivedWorkspaces,
    ...workspaces.map((workspace, index) => ({
      ...workspace,
      runState: "stopped" as const,
      archivedAt: index === 0
        ? "2026-09-05T07:30:00.000Z"
        : "2026-09-05T06:30:00.000Z",
    })),
  ];
  const active = variant === "all-archived" || variant === "empty"
    ? []
    : variant === "started"
      ? [{ ...workspaces[0], runState: "started" as const, archivedAt: null }]
      : workspaces;
  const archived = variant === "all-archived" ? allArchived : variant === "empty" ? [] : archivedWorkspaces;
  const fixture = createWorkspaceFixture({ active, archived, agents, variant });
  return { handlers: { ...base, ...fixture.handlers } };
}

// Every named state composes a complete base handler set, so incidental shell
// calls stay deterministic and missing commands remain loud.
const SCENARIOS: Record<string, ScenarioLoader> = {
  default: () => import("./scenarios/default"),
  empty: () => import("./scenarios/empty"),
  "usage-loading": () => loadUsageScenario("loading"),
  "usage-error": () => loadUsageScenario("error"),
  "usage-none": () => loadUsageScenario("none"),
  "usage-unsupported": () => loadUsageScenario("unsupported"),
  "usage-partial": () => loadUsageScenario("partial"),
  "usage-verified-empty": () => loadUsageScenario("verified-empty"),
  "workspace-empty": () => loadWorkspaceScenario("empty"),
  "workspace-all-archived": () => loadWorkspaceScenario("all-archived"),
  "workspace-loading": () => loadWorkspaceScenario("loading"),
  "workspace-error": () => loadWorkspaceScenario("error"),
  "workspace-archive-error": () => loadWorkspaceScenario("archive-error"),
  "workspace-archive-pending": () => loadWorkspaceScenario("archive-pending"),
  "workspace-restore-error": () => loadWorkspaceScenario("restore-error"),
  "workspace-restore-pending": () => loadWorkspaceScenario("restore-pending"),
  "workspace-started": () => loadWorkspaceScenario("started"),
  "workspace-busy": () => loadWorkspaceScenario("busy"),
};
const loadedScenarios = new Map<string, Promise<{ handlers: FixtureHandlers }>>();

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
  // Factory-backed scenarios carry mutable collections. Cache the composed
  // handler object for this page so a successful mutation is visible to the
  // next list call instead of being reseeded on every IPC invocation.
  const loaded = loadedScenarios.get(scenario) ?? load();
  loadedScenarios.set(scenario, loaded);
  const { handlers } = await loaded;
  const handler = handlers[cmd];
  if (!handler)
    throw new Error(
      `[fixture] no handler for command "${cmd}" in scenario "${scenario}"`,
    );
  return { hit: true, value: await handler(payload as never) };
}
