import type {
  UsageCoverageState,
  UsageDay,
  UsageOverview,
  UsageOverviewRequest,
  UsageTotals,
} from "../../ipc/types";
import type { FixtureHandlers } from "../backend";
import {
  USAGE_DATES_90,
  USAGE_GENERATED_AT,
  USAGE_MODEL_SELECTED,
  type FixtureUsageRecord,
  usageAgentOptions,
  usageContexts,
  usageModelOptions,
  usageRecords,
  usageWorkspaceOptions,
} from "./data";

export type UsageFixtureVariant =
  | "partial"
  | "empty"
  | "loading"
  | "error"
  | "none"
  | "unsupported"
  | "verified-empty";

const EMPTY_TOTAL_BASE = {
  activityCount: 0,
  responseCount: 0,
  invocationCount: 0,
  measuredEventCount: 0,
  unknownUsageCount: 0,
} as const;

function totals(records: FixtureUsageRecord[], coverage: UsageCoverageState): UsageTotals {
  const measured = records.filter(
    (record) => record.inputTokens != null && record.outputTokens != null,
  );
  const knownInput = records.filter((record) => record.inputTokens != null);
  const knownOutput = records.filter((record) => record.outputTokens != null);
  const responseCount = records.filter((record) => record.kind === "response").length;
  const invocationCount = records.length - responseCount;
  const completeEmpty = records.length === 0 && coverage === "complete";
  return {
    ...EMPTY_TOTAL_BASE,
    activityCount: records.length,
    responseCount,
    invocationCount,
    measuredTokens: measured.length > 0
      ? measured.reduce((sum, record) => sum + record.inputTokens! + record.outputTokens!, 0)
      : completeEmpty ? 0 : null,
    measuredEventCount: measured.length,
    unknownUsageCount: records.length - measured.length,
    inputTokens: knownInput.length > 0
      ? knownInput.reduce((sum, record) => sum + record.inputTokens!, 0)
      : completeEmpty ? 0 : null,
    outputTokens: knownOutput.length > 0
      ? knownOutput.reduce((sum, record) => sum + record.outputTokens!, 0)
      : completeEmpty ? 0 : null,
    coverage,
  };
}

function matchesRequest(record: FixtureUsageRecord, request: UsageOverviewRequest): boolean {
  if (request.workspaceId) {
    const wanted = request.workspaceId === "__unscoped__" ? null : request.workspaceId;
    if (record.workspaceId !== wanted) return false;
  }
  if (request.workspaceAgentId) {
    const wanted = request.workspaceAgentId === "__unassigned__" ? null : request.workspaceAgentId;
    if (record.workspaceAgentId !== wanted) return false;
  }
  return !request.modelKey || record.modelKey === request.modelKey;
}

function summaryCoverage(
  variant: UsageFixtureVariant,
  request: UsageOverviewRequest,
): UsageCoverageState {
  if (variant === "none" || variant === "unsupported") return "none";
  if (variant === "verified-empty") return "complete";
  // This scoped fixture is the D4 counterexample: the selected identity has
  // complete activity coverage while its only record has no token measurement.
  if (variant === "partial" && request.modelKey === USAGE_MODEL_SELECTED) return "complete";
  return "partial";
}

function dayCoverage(
  variant: UsageFixtureVariant,
  request: UsageOverviewRequest,
  date: string,
): UsageCoverageState {
  if (variant === "none" || variant === "unsupported") return "none";
  if (variant === "verified-empty") return date === "2026-09-05" ? "partial" : "complete";
  if (variant === "partial" && request.modelKey === USAGE_MODEL_SELECTED) {
    return date === "2026-09-05" ? "partial" : "complete";
  }
  if (variant === "empty") {
    return date >= "2026-09-01" ? "partial" : "none";
  }
  if (date === "2026-09-01" || date === "2026-09-02" || date === "2026-09-04") {
    return "complete";
  }
  if (
    date === "2026-08-15"
    || date === "2026-08-30"
    || date === "2026-09-03"
    || date === "2026-09-05"
  ) {
    return "partial";
  }
  return "none";
}

function localPartsAt(instantMs: number, timeZone: string): Record<string, number> {
  const parts = new Intl.DateTimeFormat("en-US", {
    timeZone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
  }).formatToParts(new Date(instantMs));
  return Object.fromEntries(
    parts
      .filter((part) => part.type !== "literal")
      .map((part) => [part.type, Number(part.value)]),
  );
}

/** Resolve a fixed local calendar midnight without using the current clock. */
function midnightUtc(date: string, timeZone: string): string {
  const [year, month, day] = date.split("-").map(Number);
  const wanted = Date.UTC(year, month - 1, day);
  let instant = wanted;
  // Two corrections cover offset changes around a DST boundary.
  for (let attempt = 0; attempt < 2; attempt += 1) {
    const actual = localPartsAt(instant, timeZone);
    const represented = Date.UTC(
      actual.year,
      actual.month - 1,
      actual.day,
      actual.hour,
      actual.minute,
      actual.second,
    );
    instant += wanted - represented;
  }
  return new Date(instant).toISOString();
}

function groupTotals(
  records: FixtureUsageRecord[],
  coverage: UsageCoverageState,
  key: (record: FixtureUsageRecord) => string,
): Map<string, UsageTotals> {
  const grouped = new Map<string, FixtureUsageRecord[]>();
  for (const record of records) {
    const id = key(record);
    grouped.set(id, [...(grouped.get(id) ?? []), record]);
  }
  return new Map([...grouped].map(([id, rows]) => [id, totals(rows, coverage)]));
}

function buildOverview(
  variant: UsageFixtureVariant,
  request: UsageOverviewRequest,
): UsageOverview {
  const dates = USAGE_DATES_90.slice(-request.days);
  const dateSet = new Set<string>(dates);
  const source = variant === "none" || variant === "unsupported" || variant === "verified-empty" || variant === "empty"
    ? []
    : usageRecords;
  const filtered = source.filter(
    (record) => dateSet.has(record.date) && matchesRequest(record, request),
  );
  const coverage = summaryCoverage(variant, request);
  const byModelTotals = groupTotals(filtered, coverage, (record) => record.modelKey);
  const byAgentTotals = groupTotals(
    filtered,
    coverage,
    (record) => record.workspaceAgentId ?? "__unassigned__",
  );
  const byWorkspaceTotals = groupTotals(
    filtered,
    coverage,
    (record) => record.workspaceId ?? "__unscoped__",
  );

  const daily: UsageDay[] = dates.map((date, index) => {
    const rowCoverage = dayCoverage(variant, request, date);
    const rows = filtered.filter((record) => record.date === date);
    const nextDate = dates[index + 1];
    return {
      date,
      startUtc: midnightUtc(date, request.timeZone),
      endUtc: nextDate ? midnightUtc(nextDate, request.timeZone) : USAGE_GENERATED_AT,
      inProgress: date === "2026-09-05",
      ...totals(rows, rowCoverage),
    };
  });

  const contexts = usageContexts.filter((context) => {
    if (request.workspaceId) {
      if (request.workspaceId === "__unscoped__" || context.workspaceId !== request.workspaceId) return false;
    }
    if (request.workspaceAgentId) {
      if (request.workspaceAgentId === "__unassigned__" || context.workspaceAgentId !== request.workspaceAgentId) {
        return false;
      }
    }
    return !request.modelKey || context.modelKey === request.modelKey;
  });

  return {
    generatedAt: USAGE_GENERATED_AT,
    range: {
      days: request.days,
      timeZone: request.timeZone,
      startDate: dates[0],
      endDate: dates[dates.length - 1],
      startUtc: midnightUtc(dates[0], request.timeZone),
      endUtc: USAGE_GENERATED_AT,
    },
    summary: totals(filtered, coverage),
    daily,
    models: usageModelOptions.map((model) => ({ ...model })),
    agents: usageAgentOptions.map((agent) => ({ ...agent })),
    workspaces: usageWorkspaceOptions.map((workspace) => ({ ...workspace })),
    byModel: usageModelOptions.flatMap((model) => {
      const row = byModelTotals.get(model.key);
      return row ? [{ ...model, ...row }] : [];
    }),
    byAgent: usageAgentOptions.flatMap((agent) => {
      const row = byAgentTotals.get(agent.id);
      return row ? [{ ...agent, ...row }] : [];
    }),
    byWorkspace: usageWorkspaceOptions.flatMap((workspace) => {
      const row = byWorkspaceTotals.get(workspace.id);
      return row ? [{ ...workspace, ...row }] : [];
    }),
    contexts: contexts.map((context) => ({ ...context })),
    coverage: {
      state: coverage,
      collectingSince: variant === "none" || variant === "unsupported"
        ? null
        : variant === "verified-empty"
          ? "2026-06-08T00:00:00.000Z"
          : "2026-08-15T00:00:00.000Z",
      lastVerifiedAt: variant === "none" ? null : "2026-09-05T07:59:00.000Z",
      pendingImport: coverage !== "complete" && (variant === "partial" || variant === "empty"),
      unsupportedSources: variant === "unsupported"
        ? ["older Codex transcript"]
        : coverage === "partial" && variant === "partial"
          ? ["older Codex transcript"]
          : [],
    },
  };
}

export function createUsageHandlers(variant: UsageFixtureVariant): FixtureHandlers {
  return {
    "usage.overview": (request) => {
      if (variant === "loading") return new Promise<UsageOverview>(() => {});
      if (variant === "error") throw new Error("Usage measurements are temporarily unavailable.");
      return buildOverview(variant, request);
    },
  };
}
