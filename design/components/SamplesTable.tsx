import { Check } from "lucide-react";
import {
  GO_THRESHOLD,
  breakEvenTurns,
  fmtClock,
  fmtQ,
  fmtTokens,
  fmtTurns,
  type CheckpointSample,
} from "../lib/checkpointMetrics";

// The raw window, newest first — the forensic layer under the charts. Sans with
// tabular figures so columns align without a terminal look; a hairline q-bar in
// each row echoes the hero meter at row scale so the eye reads the spread.
export interface SamplesTableProps {
  samples: CheckpointSample[];
}

export default function SamplesTable({ samples }: SamplesTableProps) {
  const rows = [...samples].reverse();
  const latestId = rows[0]?.id;

  return (
    <div className="overflow-hidden rounded-panel border border-border bg-surface-raised">
      <div className="flex items-baseline justify-between px-6 pb-3 pt-5">
        <h2 className="text-[13px] font-semibold text-text-primary">samples</h2>
        <span className="text-xs text-text-tertiary">newest first</span>
      </div>
      <div className="overflow-x-auto">
        <table className="w-full min-w-[46rem] border-collapse text-sm">
          <thead>
            <tr className="border-y border-border text-left text-[10px] font-semibold uppercase tracking-[0.06em] text-text-tertiary">
              <Th className="pl-6">time</Th>
              <Th>model</Th>
              <Th className="text-right">q</Th>
              <Th className="w-32">vs GO</Th>
              <Th className="text-right">break-even</Th>
              <Th className="text-right">candidate</Th>
              <Th>outcome</Th>
              <Th className="pr-6 text-center">count</Th>
            </tr>
          </thead>
          <tbody className="text-[13px] tabular-nums">
            {rows.map((s) => (
              <tr
                key={s.id}
                className={
                  "border-b border-border/60 last:border-0 " +
                  (s.id === latestId ? "bg-accent/[.05]" : "")
                }
              >
                <Td className="pl-6 text-text-secondary">{fmtClock(s.createdAt)}</Td>
                <Td className="text-text-muted">{s.model}</Td>
                <Td className="text-right font-semibold text-text-primary">
                  {fmtQ(s.q)}
                </Td>
                <Td>
                  <div className="relative h-1.5 w-24 rounded-full bg-fill">
                    <div
                      className="absolute inset-y-0 left-0 rounded-full bg-text-tertiary/60"
                      style={{ width: `${Math.min(s.q / GO_THRESHOLD, 1) * 100}%` }}
                    />
                    <div className="absolute inset-y-[-2px] right-0 w-px bg-live" />
                  </div>
                </Td>
                <Td className="text-right text-text-secondary">
                  ~{fmtTurns(breakEvenTurns(s.q))} turns
                </Td>
                <Td className="text-right text-text-secondary">
                  {fmtTokens(s.grossCandidateTokens)}
                </Td>
                <Td>
                  <span className="rounded-md bg-waiting/12 px-2 py-0.5 text-xs font-medium text-waiting">
                    {s.outcome}
                  </span>
                </Td>
                <Td className="pr-6 text-center">
                  {s.countFailure === 0 ? (
                    <Check
                      className="mx-auto size-4 text-live"
                      strokeWidth={2.5}
                      aria-label="ok"
                    />
                  ) : (
                    <span className="font-medium text-danger">fail</span>
                  )}
                </Td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function Th({ children, className = "" }: { children: React.ReactNode; className?: string }) {
  return <th className={"px-3 py-2 " + className}>{children}</th>;
}

function Td({ children, className = "" }: { children: React.ReactNode; className?: string }) {
  return <td className={"px-3 py-2.5 " + className}>{children}</td>;
}
