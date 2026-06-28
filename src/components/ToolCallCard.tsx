import { Wrench, Check, CircleAlert, LoaderCircle } from "lucide-react";

interface ToolCallCardProps {
  name: string;
  status: "running" | "done" | "error";
  detail?: string;
}

/**
 * Presentational card for a single tool call inside an assistant turn.
 *
 * Pure UI — no IPC. Mirrors the prototype's "tool · <name>" card (chat.html):
 * a hairline-ringed, rounded card with a header (tool icon + monospace name +
 * status indicator) and an optional detail line.
 *
 * NOTE: no live data source feeds this yet — structured tool-event streaming
 * lands in M5 (providers' tool-use API + tool join tables). See ChatView.
 */
export function ToolCallCard({ name, status, detail }: ToolCallCardProps) {
  return (
    <div className="rounded-xl ring-hair bg-surface overflow-hidden">
      <div className="flex items-center gap-2 px-3 h-8 bg-fill-softer border-b border-overlay/[0.05]">
        <Wrench className="w-3.5 h-3.5 text-text-secondary shrink-0" />
        <span className="text-[11.5px] font-semibold text-text-body">tool</span>
        <span className="font-mono text-[10.5px] text-text-muted truncate">{name}</span>
        <span className="ml-auto shrink-0">
          {status === "running" && (
            <span className="text-[10.5px] text-text-secondary flex items-center gap-1">
              <LoaderCircle className="w-3 h-3 animate-spin" />
              running
            </span>
          )}
          {status === "done" && (
            <span className="text-[10.5px] text-success flex items-center gap-1">
              <Check className="w-3 h-3" />
              done
            </span>
          )}
          {status === "error" && (
            <span className="text-[10.5px] text-danger flex items-center gap-1">
              <CircleAlert className="w-3 h-3" />
              error
            </span>
          )}
        </span>
      </div>
      {detail !== undefined && detail.length > 0 && (
        <div className="px-3 py-1.5 font-mono text-[11.5px] leading-[1.6] text-text-body whitespace-pre-wrap break-words">
          {detail}
        </div>
      )}
    </div>
  );
}
