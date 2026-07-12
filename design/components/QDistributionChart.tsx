import {
  Bar,
  BarChart,
  Cell,
  ReferenceLine,
  ResponsiveContainer,
  XAxis,
  YAxis,
} from "recharts";
import {
  GO_THRESHOLD,
  qBuckets,
  type CheckpointSample,
} from "../lib/checkpointMetrics";

// Histogram of q across [0,1] with the GO line drawn at its true position
// (0.793). The whole point is the empty right half: every sample piles up on
// the left, none near the line. Bars at/above the line would turn green — a
// standing visual promise of what GO looks like.
export interface QDistributionChartProps {
  samples: CheckpointSample[];
}

export default function QDistributionChart({ samples }: QDistributionChartProps) {
  const data = qBuckets(samples).map((b) => ({
    center: b.lo + 0.05,
    count: b.count,
    lo: b.lo,
    hi: b.hi,
  }));
  const maxCount = Math.max(...data.map((d) => d.count), 1);

  return (
    <div className="rounded-panel border border-border bg-surface-raised p-6">
      <div className="mb-1 flex items-baseline justify-between">
        <h2 className="text-[13px] font-semibold text-text-primary">
          q distribution
        </h2>
        <span className="text-xs tabular-nums text-text-tertiary">
          {samples.length} samples
        </span>
      </div>
      <p className="mb-4 text-xs leading-snug text-text-muted">
        recoverability ratio per sample · all left of the GO line
      </p>
      <div className="h-52">
        <ResponsiveContainer width="100%" height="100%">
          <BarChart data={data} margin={{ top: 16, right: 8, bottom: 4, left: -22 }}>
            <XAxis
              dataKey="center"
              type="number"
              domain={[0, 1]}
              ticks={[0, 0.2, 0.4, 0.6, 0.8, 1]}
              tickFormatter={(v: number) => v.toFixed(1)}
              tick={{ fill: "var(--color-text-tertiary)", fontSize: 11 }}
              stroke="var(--color-border)"
            />
            <YAxis
              allowDecimals={false}
              domain={[0, maxCount + 1]}
              tick={{ fill: "var(--color-text-tertiary)", fontSize: 11 }}
              stroke="var(--color-border)"
              width={40}
            />
            <ReferenceLine
              x={GO_THRESHOLD}
              stroke="var(--color-live)"
              strokeWidth={2}
              strokeDasharray="4 3"
              label={{
                value: `GO ${GO_THRESHOLD}`,
                position: "top",
                fill: "var(--color-live)",
                fontSize: 11,
                fontWeight: 600,
              }}
            />
            <Bar dataKey="count" barSize={30} radius={[3, 3, 0, 0]} isAnimationActive={false}>
              {data.map((d) => (
                <Cell
                  key={d.center}
                  fill={
                    d.center >= GO_THRESHOLD
                      ? "var(--color-live)"
                      : "var(--color-text-tertiary)"
                  }
                  fillOpacity={d.count === 0 ? 0 : 0.55}
                />
              ))}
            </Bar>
          </BarChart>
        </ResponsiveContainer>
      </div>
    </div>
  );
}
