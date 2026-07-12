import CheckpointHeader from "../components/CheckpointHeader";
import VerdictBanner from "../components/VerdictBanner";
import QDistributionChart from "../components/QDistributionChart";
import QTrendChart from "../components/QTrendChart";
import SamplesTable from "../components/SamplesTable";
import {
  CEILING_TOKENS,
  SAMPLES,
  aggregate,
  fmtClock,
  fmtQ,
  fmtTokens,
  fmtTurns,
} from "../lib/checkpointMetrics";

export const meta = { title: "Proxy checkpoint — GO / NO-GO" };

// Live-watch oversight for the infinity-turn-checkpoint measurement. One dense
// screen answers one question — is q above the GO line yet? — and stays honest
// when the answer is no. Reads top-to-bottom: verdict, then the evidence that
// earns it (distribution + trend), then the raw window.
export default function ProxyCheckpoint() {
  const agg = aggregate(SAMPLES);

  return (
    <div className="flex h-screen flex-col bg-canvas font-sans text-text-primary antialiased">
      <CheckpointHeader
        checkpoint="on"
        ceilingTokens={CEILING_TOKENS}
        mode="log"
        sampleCount={agg.count}
        updatedClock={fmtClock(agg.latest.createdAt)}
      />

      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex max-w-[80rem] flex-col gap-5 px-8 py-6">
          <VerdictBanner agg={agg} />

          <KpiStrip
            qRange={`${fmtQ(agg.qMin)} – ${fmtQ(agg.qMax)}`}
            turnsRange={`${fmtTurns(agg.breakEvenMin)} – ${fmtTurns(agg.breakEvenMax)} turns`}
            saturated={agg.outcomeCounts.saturated}
            eligible={agg.outcomeCounts.eligible}
            belowCeiling={agg.outcomeCounts.below_ceiling}
            candidate={`${fmtTokens(agg.latest.grossCandidateTokens)} / ${fmtTokens(CEILING_TOKENS)}`}
          />

          <div className="grid grid-cols-1 gap-5 lg:grid-cols-2">
            <QDistributionChart samples={SAMPLES} />
            <QTrendChart samples={SAMPLES} />
          </div>

          <SamplesTable samples={SAMPLES} />
        </div>
      </div>
    </div>
  );
}

// Secondary facts the hero doesn't carry: the spread of q and break-even, why
// every sample reads "saturated" (candidate set is a sliver of the ceiling),
// and the outcome split. One panel, divided cells — no card-in-card.
interface KpiStripProps {
  qRange: string;
  turnsRange: string;
  saturated: number;
  eligible: number;
  belowCeiling: number;
  candidate: string;
}

function KpiStrip({
  qRange,
  turnsRange,
  saturated,
  eligible,
  belowCeiling,
  candidate,
}: KpiStripProps) {
  const total = saturated + eligible + belowCeiling;
  return (
    <div className="grid grid-cols-2 divide-x divide-y divide-border rounded-panel border border-border bg-surface-raised sm:grid-cols-4 sm:divide-y-0">
      <Cell label="q range (min – max)" value={qRange} />
      <Cell label="break-even spread" value={turnsRange} />
      <div className="px-6 py-5">
        <div className="text-lg font-semibold tabular-nums tracking-tight text-text-primary">
          {saturated}
          <span className="text-sm font-normal text-text-tertiary"> / {total} saturated</span>
        </div>
        <div className="mt-2 flex h-1.5 overflow-hidden rounded-full bg-fill">
          <span
            className="bg-waiting"
            style={{ width: `${(saturated / total) * 100}%` }}
          />
          <span
            className="bg-live"
            style={{ width: `${(eligible / total) * 100}%` }}
          />
          <span
            className="bg-accent"
            style={{ width: `${(belowCeiling / total) * 100}%` }}
          />
        </div>
        <div className="mt-1.5 text-[10px] font-semibold uppercase tracking-[0.06em] text-text-tertiary">
          {eligible} eligible · {belowCeiling} below-ceiling
        </div>
      </div>
      <Cell
        label="candidate vs ceiling"
        value={candidate}
        hint="tiny removal set → saturated"
      />
    </div>
  );
}

function Cell({
  label,
  value,
  hint,
}: {
  label: string;
  value: string;
  hint?: string;
}) {
  return (
    <div className="px-6 py-5">
      <div className="text-lg font-semibold tabular-nums tracking-tight text-text-primary">
        {value}
      </div>
      <div className="mt-2 text-[10px] font-semibold uppercase tracking-[0.06em] text-text-tertiary">
        {label}
      </div>
      {hint ? (
        <div className="mt-1 text-[11px] text-text-muted">{hint}</div>
      ) : null}
    </div>
  );
}
