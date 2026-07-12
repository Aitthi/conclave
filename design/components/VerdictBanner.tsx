import { TriangleAlert, CircleCheck, ShieldCheck } from "lucide-react";
import { GO_THRESHOLD, fmtQ, fmtTurns, type Aggregates } from "../lib/checkpointMetrics";

// The hero: the GO / NO-GO decision, first-read. Driven by avg q vs the 0.793
// GO line. A linear q-meter shows exactly how far the current average sits from
// the line — the empty span between "where we are" and the GO mark IS the
// argument. Three inline facts back it: avg q, break-even at that q, and the
// count_failure health (the M1 instrument must stay at 0).
export interface VerdictBannerProps {
  agg: Aggregates;
}

export default function VerdictBanner({ agg }: VerdictBannerProps) {
  const isGo = agg.verdict === "GO";
  const avgPct = Math.min(agg.qAvg, 1) * 100;
  const goPct = GO_THRESHOLD * 100;
  const healthy = agg.countFailures === 0;

  return (
    <section className="rounded-panel border border-border bg-surface-raised px-8 py-7">
      <div className="flex flex-col gap-7 lg:flex-row lg:items-center lg:gap-10">
        {/* verdict */}
        <div className="flex items-start gap-4 lg:w-[19rem] lg:shrink-0">
          <span
            className={
              "mt-1.5 flex size-11 items-center justify-center rounded-full " +
              (isGo ? "bg-live/10 text-live" : "bg-danger/10 text-danger")
            }
          >
            {isGo ? (
              <CircleCheck className="size-6" strokeWidth={2.5} />
            ) : (
              <TriangleAlert className="size-6" strokeWidth={2.5} />
            )}
          </span>
          <div>
            <div
              className={
                "text-5xl font-bold leading-none tracking-tight " +
                (isGo ? "text-live" : "text-danger")
              }
            >
              {agg.verdict}
            </div>
            <p className="mt-2 max-w-xs text-sm leading-snug text-text-secondary">
              Average recoverability q of{" "}
              <span className="font-semibold text-text-primary">
                {fmtQ(agg.qAvg)}
              </span>{" "}
              sits below the GO line of {fmtQ(GO_THRESHOLD)}. Checkpoint would
              not pay off on this traffic.
            </p>
          </div>
        </div>

        {/* q meter — where we are vs the GO line */}
        <div className="min-w-0 flex-1">
          <div className="mb-2 flex items-baseline justify-between text-xs tabular-nums text-text-tertiary">
            <span>q 0.0</span>
            <span>1.0</span>
          </div>
          <div className="relative h-9 w-full rounded-chip bg-fill">
            {/* filled span = current average */}
            <div
              className="absolute inset-y-0 left-0 rounded-l-chip bg-text-tertiary/45"
              style={{ width: `${avgPct}%` }}
            />
            {/* avg marker */}
            <div
              className="absolute inset-y-0 w-px bg-text-primary"
              style={{ left: `${avgPct}%` }}
            />
            {/* GO line */}
            <div
              className="absolute inset-y-[-6px] w-0.5 bg-live"
              style={{ left: `${goPct}%` }}
            />
            <span
              className="absolute -top-6 -translate-x-1/2 whitespace-nowrap rounded-md bg-live/15 px-1.5 py-0.5 text-[11px] font-semibold tabular-nums text-live"
              style={{ left: `${goPct}%` }}
            >
              GO ≥ {fmtQ(GO_THRESHOLD)}
            </span>
          </div>
          <p className="mt-3 text-xs text-text-muted">
            q must rise ~{agg.requiredMultiple.toFixed(1)}× (gap{" "}
            <span className="tabular-nums">{fmtQ(agg.gapToGo)}</span>) to cross
            the line.
          </p>
        </div>

        {/* inline facts */}
        <div className="flex gap-8 lg:w-56 lg:shrink-0 lg:flex-col lg:gap-4 lg:border-l lg:border-border lg:pl-8">
          <Fact label="break-even @ avg q" value={`~${fmtTurns(agg.breakEvenAtAvg)} turns`} />
          <div className="flex items-center gap-2">
            <ShieldCheck
              className={"size-4 " + (healthy ? "text-live" : "text-danger")}
              strokeWidth={2.5}
            />
            <div>
              <div className="text-sm font-semibold tabular-nums text-text-primary">
                {agg.count - agg.countFailures}/{agg.count} clean
              </div>
              <div className="text-[10px] font-semibold uppercase tracking-[0.06em] text-text-tertiary">
                count_tokens health
              </div>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}

function Fact({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-sm font-semibold tabular-nums text-text-primary">
        {value}
      </div>
      <div className="text-[10px] font-semibold uppercase tracking-[0.06em] text-text-tertiary">
        {label}
      </div>
    </div>
  );
}
