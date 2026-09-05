import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { CalendarDays, Info, RefreshCw } from "lucide-react";

import { ipc } from "../ipc";
import type {
  UsageAgentOption,
  UsageAgentRow,
  UsageContext,
  UsageModelBasis,
  UsageModelOption,
  UsageModelRow,
  UsageOverview as UsageOverviewData,
  UsageOverviewRequest,
  UsageTotals,
  UsageWorkspaceOption,
  UsageWorkspaceRow,
} from "../ipc/types";
import { UsageHeatmap } from "./UsageHeatmap";
import "./usage-overview.css";

type Breakdown = "model" | "agent" | "workspace";

interface UsageOverviewProps {
  onManageWorkspaces: () => void;
}

interface Catalog {
  models: UsageModelOption[];
  agents: UsageAgentOption[];
  workspaces: UsageWorkspaceOption[];
}

const EMPTY_CATALOG: Catalog = { models: [], agents: [], workspaces: [] };

function formatNumber(value: number | null): string {
  return value == null ? "—" : value.toLocaleString();
}

function basisLabel(basis: UsageModelBasis): string {
  if (basis === "reported") return "Reported";
  if (basis === "selected") return "Selected";
  return "Unknown";
}

function modelLabel(model: UsageModelOption): string {
  if (model.basis === "unknown") return "Unknown model";
  return `${model.name} — ${basisLabel(model.basis)}`;
}

function coverageLabel(coverage: UsageTotals["coverage"]): string {
  return coverage.charAt(0).toUpperCase() + coverage.slice(1);
}

function activitySplit(totals: UsageTotals): string {
  const responses = `${totals.responseCount.toLocaleString()} model ${totals.responseCount === 1 ? "response" : "responses"}`;
  const drafts = `${totals.invocationCount.toLocaleString()} draft ${totals.invocationCount === 1 ? "run" : "runs"}`;
  return `${responses} · ${drafts}`;
}

function formatVerificationTime(instant: string, timeZone: string): string {
  return new Intl.DateTimeFormat("en-US", {
    timeZone,
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
  }).format(new Date(instant));
}

function ageLabel(observedAt: string | null, generatedAt: string): string {
  if (!observedAt) return "Observation time unavailable";
  const observed = Date.parse(observedAt);
  const generated = Date.parse(generatedAt);
  if (!Number.isFinite(observed) || !Number.isFinite(generated)) return observedAt;
  const minutes = Math.max(0, Math.floor((generated - observed) / 60_000));
  if (minutes < 1) return "Observed just now";
  if (minutes < 60) return `Observed ${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `Observed ${hours}h ago`;
  return `Observed ${Math.floor(hours / 24)}d ago · stale`;
}

function sourceLabel(source: string | null): string {
  if (!source) return "Source unavailable";
  const normalized = source.toLowerCase();
  if (normalized.includes("claude")) return "Claude transcript";
  if (normalized.includes("codex")) return "Codex transcript";
  if (normalized.includes("draft")) return "Draft result";
  if (normalized.includes("direct") || normalized.includes("provider")) return "Provider response";
  return "Recorded source";
}

function selectedModel(catalog: Catalog, key: string): UsageModelOption | undefined {
  return catalog.models.find((model) => model.key === key);
}

function requestKey(request: UsageOverviewRequest): string {
  return JSON.stringify([
    request.days,
    request.timeZone,
    request.workspaceId ?? null,
    request.workspaceAgentId ?? null,
    request.modelKey ?? null,
  ]);
}

function SummaryCard({ label, value, helper }: { label: string; value: string; helper: string }) {
  return (
    <div className="usage-summary-card">
      <h2>{label}</h2>
      <strong>{value}</strong>
      <p>{helper}</p>
    </div>
  );
}

function BreakdownIdentity({ row }: { row: UsageModelRow | UsageAgentRow | UsageWorkspaceRow }) {
  if ("key" in row) {
    return (
      <div className="usage-identity">
        <strong title={row.name}>{row.basis === "unknown" ? "Unknown model" : row.name}</strong>
        <span className={`usage-basis is-${row.basis}`}>{basisLabel(row.basis)}</span>
        {row.provider && <small>{row.provider}</small>}
      </div>
    );
  }
  return (
    <div className="usage-identity">
      <strong title={row.name}>{row.name}</strong>
      {"archived" in row && row.archived && <span className="usage-muted-badge">Archived</span>}
    </div>
  );
}

function ContextRow({ context, catalog, generatedAt }: { context: UsageContext; catalog: Catalog; generatedAt: string }) {
  const model = selectedModel(catalog, context.modelKey);
  const usableCapacity = context.capacity != null && context.capacity > 0;
  const percent = context.tokens != null && usableCapacity
    ? Math.max(0, Math.min(100, Math.round((context.tokens / context.capacity!) * 100)))
    : null;
  return (
    <div className="usage-context-row">
      <div className="usage-context-agent">
        <strong title={context.agentName}>{context.agentName}</strong>
        <span title={context.workspaceName}>
          {context.workspaceName}{context.archived ? " · Archived" : ""}
        </span>
      </div>
      <div className="usage-context-meter">
        {context.tokens == null ? (
          <span className="usage-unavailable">Unavailable</span>
        ) : (
          <>
            <div>
              {context.tokens.toLocaleString()} / {usableCapacity ? context.capacity!.toLocaleString() : "—"} tokens
              {percent == null ? "" : ` · ${percent}%`}
            </div>
            {usableCapacity && (
              <meter
                min={0}
                max={context.capacity!}
                value={Math.min(context.tokens, context.capacity!)}
                aria-label={`${context.agentName} current context`}
              />
            )}
          </>
        )}
      </div>
      <div className="usage-context-meta">
        <span title={model ? modelLabel(model) : "Model basis unavailable"}>
          {model ? modelLabel(model) : "Model unavailable"}
        </span>
        <span>{sourceLabel(context.source)}</span>
        <span>{ageLabel(context.observedAt, generatedAt)}</span>
      </div>
    </div>
  );
}

export function UsageOverview({ onManageWorkspaces }: UsageOverviewProps) {
  const timeZone = useMemo(
    () => Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC",
    [],
  );
  const [days, setDays] = useState<30 | 90>(90);
  const [modelKey, setModelKey] = useState("");
  const [workspaceId, setWorkspaceId] = useState("");
  const [workspaceAgentId, setWorkspaceAgentId] = useState("");
  const [breakdown, setBreakdown] = useState<Breakdown>("model");
  const [data, setData] = useState<UsageOverviewData | null>(null);
  const [dataKey, setDataKey] = useState("");
  const [catalog, setCatalog] = useState<Catalog>(EMPTY_CATALOG);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const cache = useRef(new Map<string, UsageOverviewData>());
  const requestSequence = useRef(0);
  const inFlight = useRef<{ key: string; id: number } | null>(null);

  const request = useMemo<UsageOverviewRequest>(
    () => ({
      days,
      timeZone,
      ...(modelKey ? { modelKey } : {}),
      ...(workspaceId ? { workspaceId } : {}),
      ...(workspaceAgentId ? { workspaceAgentId } : {}),
    }),
    [days, modelKey, timeZone, workspaceAgentId, workspaceId],
  );
  const key = useMemo(() => requestKey(request), [request]);

  const load = useCallback(async (background = false) => {
    if (inFlight.current?.key === key) return;
    const cached = cache.current.get(key) ?? null;
    const id = ++requestSequence.current;
    inFlight.current = { key, id };
    setError(null);
    if (cached) {
      setData(cached);
      setDataKey(key);
      setLoading(false);
      setRefreshing(true);
    } else {
      setData(null);
      setDataKey("");
      setLoading(!background || !cached);
      setRefreshing(false);
    }
    try {
      const result = await ipc.usage.overview(request);
      if (requestSequence.current !== id) return;
      cache.current.set(key, result);
      setData(result);
      setDataKey(key);
      setCatalog({ models: result.models, agents: result.agents, workspaces: result.workspaces });
      setError(null);
    } catch (reason) {
      if (requestSequence.current !== id) return;
      const message = reason instanceof Error ? reason.message : String(reason);
      setError(message || "Usage measurements are unavailable.");
      if (!cached) {
        setData(null);
        setDataKey("");
      }
    } finally {
      if (requestSequence.current === id) {
        inFlight.current = null;
        setLoading(false);
        setRefreshing(false);
      }
    }
  }, [key, request]);

  useEffect(() => {
    void load();
    const timer = window.setInterval(() => void load(true), 10_000);
    return () => {
      window.clearInterval(timer);
      requestSequence.current += 1;
      if (inFlight.current?.key === key) inFlight.current = null;
    };
  }, [key, load]);

  const visible = dataKey === key ? data : null;
  const agentOptions = catalog.agents.filter((agent) => {
    if (!workspaceId) return true;
    if (workspaceId === "__unscoped__") return agent.workspaceId == null;
    return agent.workspaceId === workspaceId || agent.id === "__unassigned__";
  });

  function changeWorkspace(nextWorkspaceId: string) {
    setWorkspaceId(nextWorkspaceId);
    if (!workspaceAgentId || workspaceAgentId === "__unassigned__") return;
    const selected = catalog.agents.find((agent) => agent.id === workspaceAgentId);
    if (!selected) {
      setWorkspaceAgentId("");
      return;
    }
    const matches = !nextWorkspaceId
      || (nextWorkspaceId === "__unscoped__" ? selected.workspaceId == null : selected.workspaceId === nextWorkspaceId);
    if (!matches) setWorkspaceAgentId("");
  }

  const rows = visible
    ? breakdown === "model"
      ? visible.byModel
      : breakdown === "agent"
        ? visible.byAgent
        : visible.byWorkspace
    : [];
  const summary = visible?.summary;

  useEffect(() => {
    document.body.dataset.conclaveState = loading && !visible ? "loading" : error && !visible ? "error" : "ready";
    return () => {
      delete document.body.dataset.conclaveState;
    };
  }, [error, loading, visible]);

  return (
    <main className="usage-overview" aria-labelledby="usage-overview-title">
      <header className="usage-page-header">
        <div>
          <h1 id="usage-overview-title">Overview</h1>
          <p>Recorded AI activity and measured tokens</p>
        </div>
        <button type="button" className="usage-compact-button" onClick={onManageWorkspaces}>
          Manage workspaces
        </button>
      </header>

      <div className="usage-content">
        <div className="usage-filter-row" aria-label="Usage filters">
          <label>
            <span className="sr-only">Model</span>
            <select data-usage-filter="model" value={modelKey} onChange={(event) => setModelKey(event.target.value)}>
              <option value="">All models</option>
              {catalog.models.map((model) => (
                <option key={model.key} value={model.key}>{modelLabel(model)}</option>
              ))}
            </select>
          </label>
          <label>
            <span className="sr-only">Agent</span>
            <select data-usage-filter="agent" value={workspaceAgentId} onChange={(event) => setWorkspaceAgentId(event.target.value)}>
              <option value="">All agents</option>
              {agentOptions.map((agent) => {
                const workspace = catalog.workspaces.find((candidate) => candidate.id === agent.workspaceId);
                return (
                  <option key={agent.id} value={agent.id}>
                    {agent.name}{workspace ? ` — ${workspace.name}${workspace.archived ? " (Archived)" : ""}` : ""}
                  </option>
                );
              })}
            </select>
          </label>
          <label>
            <span className="sr-only">Workspace</span>
            <select data-usage-filter="workspace" value={workspaceId} onChange={(event) => changeWorkspace(event.target.value)}>
              <option value="">All workspaces</option>
              {catalog.workspaces.map((workspace) => (
                <option key={workspace.id} value={workspace.id}>
                  {workspace.name}{workspace.archived ? " — Archived" : ""}
                </option>
              ))}
            </select>
          </label>
          <label className="usage-range-select">
            <CalendarDays aria-hidden="true" />
            <span className="sr-only">Date range</span>
            <select data-usage-filter="days" value={days} onChange={(event) => setDays(Number(event.target.value) as 30 | 90)}>
              <option value={30}>Last 30 days</option>
              <option value={90}>Last 90 days</option>
            </select>
          </label>
          <button
            type="button"
            className="usage-refresh-button"
            disabled={refreshing || loading}
            onClick={() => void load(true)}
            aria-label="Refresh usage"
            title="Refresh usage"
          >
            <RefreshCw aria-hidden="true" className={refreshing ? "is-spinning" : ""} />
          </button>
        </div>
        <p className="usage-filter-note">Archived workspace history included · hidden internal workspaces excluded</p>

        {error && (
          <div className="usage-request-error" role="alert">
            <span>
              {visible
                ? "Couldn’t refresh usage. Showing the last measurement for these filters."
                : "Couldn’t load usage. Measurements are unavailable."}
            </span>
            <button type="button" onClick={() => void load()}>Retry</button>
          </div>
        )}

        {loading && !visible ? (
          <div className="usage-loading" aria-busy="true" aria-label="Loading usage overview">
            <div className="usage-loading-summary"><i /><i /><i /></div>
            <div className="usage-loading-chart" />
          </div>
        ) : visible && summary ? (
          <>
            <section className="usage-summary" aria-label="Period measurements">
              <SummaryCard
                label="Measured tokens"
                value={formatNumber(summary.measuredTokens)}
                helper={`Input ${formatNumber(summary.inputTokens)} · Output ${formatNumber(summary.outputTokens)} · ${summary.unknownUsageCount.toLocaleString()} ${summary.unknownUsageCount === 1 ? "record has" : "records have"} unknown usage`}
              />
              <SummaryCard
                label="Recorded activity"
                value={summary.coverage === "none" && summary.activityCount === 0 ? "Unavailable" : summary.activityCount === 0 && summary.coverage === "partial" ? "—" : summary.activityCount.toLocaleString()}
                helper={`Model responses and draft runs · ${activitySplit(summary)}`}
              />
              <SummaryCard
                label="Coverage"
                value={coverageLabel(visible.coverage.state)}
                helper={`${visible.coverage.collectingSince
                  ? `Collection since ${visible.coverage.collectingSince.slice(0, 10)}`
                  : "Collection start unknown"} · ${visible.coverage.lastVerifiedAt
                  ? `Last verified ${formatVerificationTime(visible.coverage.lastVerifiedAt, visible.range.timeZone)}`
                  : "Last verification unknown"}`}
              />
            </section>

            <p className="usage-truth-note" role="status">
              <Info aria-hidden="true" />
              <span>
                Measured tokens include known input + output only. They are not total account consumption; missing usage and observation gaps remain visible.
                {visible.coverage.pendingImport ? " Recent activity is still being imported." : ""}
                {visible.coverage.unsupportedSources.length > 0 ? " Some activity sources are not yet supported." : ""}
              </span>
            </p>

            <UsageHeatmap days={visible.daily} range={visible.range} />

            <section className="usage-breakdown" aria-labelledby="usage-breakdown-title">
              <div className="usage-section-heading">
                <div>
                  <h2 id="usage-breakdown-title">Usage breakdown</h2>
                  <p>Coverage follows the selected scope; model rows do not claim per-model observation.</p>
                </div>
                <div className="usage-segmented" aria-label="Breakdown dimension">
                  {(["model", "agent", "workspace"] as Breakdown[]).map((value) => (
                    <button
                      type="button"
                      key={value}
                      aria-pressed={breakdown === value}
                      onClick={() => setBreakdown(value)}
                    >
                      {value.charAt(0).toUpperCase() + value.slice(1)}
                    </button>
                  ))}
                </div>
              </div>
              <div className="usage-breakdown-table-wrap">
                <table className="usage-table">
                  <thead>
                    <tr>
                      <th>{breakdown.charAt(0).toUpperCase() + breakdown.slice(1)}</th>
                      <th>Recorded activity</th>
                      <th>Measured tokens</th>
                      <th>Unknown usage</th>
                    </tr>
                  </thead>
                  <tbody>
                    {rows.map((row) => (
                      <tr key={"key" in row ? row.key : row.id}>
                        <td><BreakdownIdentity row={row} /></td>
                        <td>
                          <strong>{row.activityCount.toLocaleString()}</strong>
                          <span>{activitySplit(row)}</span>
                        </td>
                        <td>
                          <strong>{formatNumber(row.measuredTokens)}</strong>
                          <span>Input {formatNumber(row.inputTokens)} · Output {formatNumber(row.outputTokens)}</span>
                        </td>
                        <td>{row.unknownUsageCount.toLocaleString()} records</td>
                      </tr>
                    ))}
                    {rows.length === 0 && (
                      <tr>
                        <td colSpan={4} className="usage-empty-row">
                          {summary.coverage === "none"
                            ? "Usage is unavailable for these filters. Unknown does not mean zero."
                            : "No recorded activity matches these filters."}
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </div>
            </section>

            <section className="usage-context" aria-labelledby="usage-context-title">
              <div>
                <h2 id="usage-context-title">Current context <span>Latest snapshots · independent of date range</span></h2>
                <p>Context is not cumulative usage. Capacities and snapshots are never summed.</p>
              </div>
              {visible.contexts.length > 0 ? visible.contexts.map((context) => (
                <ContextRow
                  key={context.workspaceAgentId}
                  context={context}
                  catalog={{ models: visible.models, agents: visible.agents, workspaces: visible.workspaces }}
                  generatedAt={visible.generatedAt}
                />
              )) : (
                <p className="usage-context-empty">No current context observation matches these filters.</p>
              )}
            </section>
          </>
        ) : null}
      </div>
    </main>
  );
}
