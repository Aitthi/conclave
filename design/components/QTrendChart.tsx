import {
  Area,
  AreaChart,
  ReferenceLine,
  ResponsiveContainer,
  XAxis,
  YAxis,
} from "recharts";
import {
  GO_THRESHOLD,
  qTrend,
  type CheckpointSample,
} from "../lib/checkpointMetrics";

// q over the sampling window. This is one growing conversation, so q climbs
// monotonically — but the GO line pinned at the top of the axis makes the gap
// unmissable: the trend rises and still has most of the chart to go.
export interface QTrendChartProps {
  samples: CheckpointSample[];
}

export default function QTrendChart({ samples }: QTrendChartProps) {
  const data = qTrend(samples);

  return (
    <div className="rounded-panel border border-border bg-surface-raised p-6">
      <div className="mb-1 flex items-baseline justify-between">
        <h2 className="text-[13px] font-semibold text-text-primary">q over window</h2>
        <span className="text-xs tabular-nums text-text-tertiary">
          {data[0]?.time}–{data[data.length - 1]?.time} UTC
        </span>
      </div>
      <p className="mb-4 text-xs leading-snug text-text-muted">
        rising, but still far under GO — one growing conversation
      </p>
      <div className="h-52">
        <ResponsiveContainer width="100%" height="100%">
          <AreaChart data={data} margin={{ top: 16, right: 8, bottom: 4, left: -22 }}>
            <defs>
              <linearGradient id="qtrend" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor="var(--color-accent)" stopOpacity={0.28} />
                <stop offset="100%" stopColor="var(--color-accent)" stopOpacity={0.02} />
              </linearGradient>
            </defs>
            <XAxis
              dataKey="n"
              tick={{ fill: "var(--color-text-tertiary)", fontSize: 11 }}
              stroke="var(--color-border)"
              tickFormatter={(v: number) => `#${v}`}
            />
            <YAxis
              domain={[0, 1]}
              ticks={[0, 0.25, 0.5, 0.75, 1]}
              tick={{ fill: "var(--color-text-tertiary)", fontSize: 11 }}
              stroke="var(--color-border)"
              width={40}
            />
            <ReferenceLine
              y={GO_THRESHOLD}
              stroke="var(--color-live)"
              strokeWidth={2}
              strokeDasharray="4 3"
              label={{
                value: `GO ${GO_THRESHOLD}`,
                position: "insideTopRight",
                fill: "var(--color-live)",
                fontSize: 11,
                fontWeight: 600,
              }}
            />
            <Area
              type="monotone"
              dataKey="q"
              stroke="var(--color-accent)"
              strokeWidth={2}
              fill="url(#qtrend)"
              dot={{ r: 2.5, fill: "var(--color-accent)", strokeWidth: 0 }}
              isAnimationActive={false}
            />
          </AreaChart>
        </ResponsiveContainer>
      </div>
    </div>
  );
}
