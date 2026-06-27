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
    <div className="rounded-xl ring-hair bg-white overflow-hidden">
      <div className="flex items-center gap-2 px-3 h-8 bg-[#fafafa] border-b border-black/[0.05]">
        <Wrench className="w-3.5 h-3.5 text-[#6e6e73] shrink-0" />
        <span className="text-[11.5px] font-semibold text-[#3a3a3c]">tool</span>
        <span className="font-mono text-[10.5px] text-[#86868b] truncate">{name}</span>
        <span className="ml-auto shrink-0">
          {status === "running" && (
            <span className="text-[10.5px] text-[#6e6e73] flex items-center gap-1">
              <LoaderCircle className="w-3 h-3 animate-spin" />
              running
            </span>
          )}
          {status === "done" && (
            <span className="text-[10.5px] text-[#30a14e] flex items-center gap-1">
              <Check className="w-3 h-3" />
              done
            </span>
          )}
          {status === "error" && (
            <span className="text-[10.5px] text-[#b62324] flex items-center gap-1">
              <CircleAlert className="w-3 h-3" />
              error
            </span>
          )}
        </span>
      </div>
      {detail !== undefined && detail.length > 0 && (
        <div className="px-3 py-1.5 font-mono text-[11.5px] leading-[1.6] text-[#3a3a3c] whitespace-pre-wrap break-words">
          {detail}
        </div>
      )}
    </div>
  );
}
