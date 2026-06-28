import { Sparkles, Check, LoaderCircle } from "lucide-react";

interface SkillRunningCardProps {
  name: string;
  status: "running" | "done";
}

/**
 * Presentational pill for a running (or completed) skill inside an assistant
 * turn. Pure UI — no IPC.
 *
 * NOTE: no live data source feeds this yet — structured skill-event streaming
 * lands in M5 (skill join tables + event stream). See ChatView.
 */
export function SkillRunningCard({ name, status }: SkillRunningCardProps) {
  return (
    <div className="inline-flex items-center gap-2 rounded-full ring-hair bg-agent-maestro/[0.06] px-3 py-1.5 text-[12px]">
      <Sparkles className="w-3.5 h-3.5 text-agent-maestro shrink-0" />
      <span className="text-text-body">
        Running skill: <span className="font-mono text-[11.5px]">{name}</span>
      </span>
      {status === "running" ? (
        <LoaderCircle className="w-3 h-3 text-agent-maestro animate-spin shrink-0" />
      ) : (
        <Check className="w-3 h-3 text-success shrink-0" />
      )}
    </div>
  );
}
