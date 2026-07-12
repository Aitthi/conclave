import { Activity } from "lucide-react";

// Top bar, in the app's own voice: sans, calm, an icon + title on the left and
// the sampler state as quiet inline facts on the right. Reads like the Browser
// screen's chrome, not a terminal status line.
export interface CheckpointHeaderProps {
  checkpoint: "on" | "off";
  ceilingTokens: number;
  mode: string;
  sampleCount: number;
  updatedClock: string; // "03:57"
}

export default function CheckpointHeader({
  checkpoint,
  ceilingTokens,
  mode,
  sampleCount,
  updatedClock,
}: CheckpointHeaderProps) {
  return (
    <header className="flex flex-wrap items-center justify-between gap-3 border-b border-border px-6 py-3.5">
      <div className="flex items-center gap-2.5">
        <Activity className="size-[18px] shrink-0 text-accent" strokeWidth={2.25} />
        <span className="text-[13px] font-semibold text-text-primary">
          ctx-proxy
          <span className="text-text-tertiary"> / </span>
          <span className="font-normal text-text-secondary">
            checkpoint measurement
          </span>
        </span>
      </div>
      <div className="flex items-center gap-2 text-[12px] text-text-secondary">
        <span className="inline-flex items-center gap-1.5 rounded-md bg-fill px-2 py-1">
          checkpoint
          <span
            className={
              checkpoint === "on"
                ? "font-semibold text-live"
                : "font-semibold text-text-muted"
            }
          >
            {checkpoint.toUpperCase()}
          </span>
        </span>
        <span className="hidden items-center gap-1 rounded-md bg-fill px-2 py-1 tabular-nums sm:inline-flex">
          ceiling
          <span className="font-medium text-text-primary">
            {ceilingTokens / 1000}k
          </span>
        </span>
        <span className="hidden items-center gap-1 rounded-md bg-fill px-2 py-1 sm:inline-flex">
          mode
          <span className="font-medium text-text-primary">{mode}</span>
        </span>
        <span className="pl-1 tabular-nums text-text-tertiary">
          {sampleCount} samples · {updatedClock} UTC
        </span>
      </div>
    </header>
  );
}
