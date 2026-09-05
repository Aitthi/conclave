// src/components/builder/PositionSection.tsx
//
// Position (edit mode only): Track, Level cards, Supervisor list, Escalation
// chain. Content moved verbatim from Builder.tsx (spec D8). Canon rule 29
// drops the outer rounded panel so the four groups sit on the modal surface;
// canon rule 30 keeps the four LEVEL CARDS here on purpose - they edit this
// workspace instance\u2019s live level, while Role & Level\u2019s segmented control
// edits the definition\u2019s remembered default.

import type { WorkspaceAgent } from "../../ipc";
import { levelOf, wouldCycle, LEVELS } from "../../lib/positions";
import { HumanChip, PositionLine } from "../Position";
import { Section } from "./Section";

interface PositionSectionProps {
  scopedAgent: WorkspaceAgent;
  positionRoster: WorkspaceAgent[];
  supervisorOptions: WorkspaceAgent[];
  previewRoster: WorkspaceAgent[];
  previewChainIds: string[];
  trackLabel: string;
  levelDraft: string | null;
  setLevelDraft: (v: string | null) => void;
  supervisorDraft: string | null;
  setSupervisorDraft: (v: string | null) => void;
}

export function PositionSection({
  scopedAgent,
  positionRoster,
  supervisorOptions,
  previewRoster,
  previewChainIds,
  trackLabel,
  levelDraft,
  setLevelDraft,
  supervisorDraft,
  setSupervisorDraft,
}: PositionSectionProps) {
  return (
    <Section id="position" title="Position">
      <div className="space-y-3.5">
        <div>
          <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
            Track
          </div>
          <div className="rounded-lg bg-overlay/[0.04] px-3 py-2">
            <PositionLine
              levelId={levelDraft}
              track={trackLabel}
              compact={false}
              supervisor={
                supervisorDraft
                  ? (() => {
                      const next = positionRoster.find(
                        (agent) => agent.id === supervisorDraft,
                      );
                      return next ? { name: next.name ?? next.id } : null;
                    })()
                  : null
              }
              showReportsTo
            />
          </div>
        </div>

        <div>
          <div className="flex items-center justify-between mb-1.5">
            <span className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase">
              Level
            </span>
            <button
              type="button"
              onClick={() => setLevelDraft(null)}
              className={`text-[11px] font-medium ${
                levelDraft == null
                  ? "text-accent"
                  : "text-text-tertiary hover:text-text-secondary"
              }`}
            >
              Clear to Unranked
            </button>
          </div>
          <div className="grid grid-cols-4 gap-2">
            {LEVELS.map((level) => {
              const active = levelDraft === level.id;
              return (
                <button
                  key={level.id}
                  type="button"
                  onClick={() => setLevelDraft(level.id)}
                  className={`rounded-xl px-2.5 py-2 text-left transition-all ring-1 ${
                    active
                      ? "ring-accent/40 bg-accent/[0.06]"
                      : "ring-overlay/[0.08] bg-surface hover:bg-overlay/[0.02]"
                  }`}
                >
                  <div className="text-[11.5px] font-semibold leading-tight">
                    {level.name}
                  </div>
                  <div className="mt-1 text-[11px] text-text-tertiary">
                    rung {level.rung}
                  </div>
                </button>
              );
            })}
          </div>
        </div>

        <div>
          <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
            Supervisor
          </div>
          <div className="space-y-1.5 max-h-40 overflow-y-auto pr-1">
            <button
              type="button"
              onClick={() => setSupervisorDraft(null)}
              className={`w-full rounded-lg px-2.5 py-2 text-left transition-all ring-1 ${
                supervisorDraft == null
                  ? "ring-accent/40 bg-accent/[0.06]"
                  : "ring-overlay/[0.08] bg-surface hover:bg-overlay/[0.02]"
              }`}
            >
              <div className="flex items-center gap-2">
                <HumanChip />
                <span className="text-[11px] text-text-tertiary">
                  Top of the chain
                </span>
              </div>
            </button>
            {supervisorOptions.map((agent) => {
              const disabled = wouldCycle(
                scopedAgent.id,
                agent.id,
                positionRoster,
              );
              const active = supervisorDraft === agent.id;
              return (
                <button
                  key={agent.id}
                  type="button"
                  onClick={() => !disabled && setSupervisorDraft(agent.id)}
                  disabled={disabled}
                  className={`w-full rounded-lg px-2.5 py-2 text-left transition-all ring-1 ${
                    active
                      ? "ring-accent/40 bg-accent/[0.06]"
                      : "ring-overlay/[0.08] bg-surface hover:bg-overlay/[0.02]"
                  } ${disabled ? "opacity-50 cursor-not-allowed" : ""}`}
                >
                  <div className="text-[12px] font-semibold leading-tight">
                    {agent.name ?? agent.id}
                  </div>
                  <PositionLine
                    levelId={agent.level}
                    track={agent.roleName ?? "Agent"}
                    compact
                    className="mt-1"
                  />
                  {disabled && (
                    <div className="mt-1 text-[10.5px] text-text-tertiary">
                      Self and descendants cannot supervise this member
                    </div>
                  )}
                </button>
              );
            })}
          </div>
        </div>

        <div>
          <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
            Escalation chain
          </div>
          <div className="rounded-lg bg-overlay/[0.04] px-3 py-2">
            <div className="flex items-center gap-1.5 flex-wrap text-[11.5px] text-text-secondary">
              {previewChainIds.map((id, index) => {
                const agent = previewRoster.find((item) => item.id === id);
                return (
                  <span key={id} className="inline-flex items-center gap-1.5">
                    {index > 0 && <span className="text-text-tertiary">→</span>}
                    <span className="font-medium text-text-primary">
                      {agent?.name ?? id}
                    </span>
                    <span className="text-text-tertiary">
                      ({agent?.level ? levelOf(agent.level).short : "Unranked"})
                    </span>
                  </span>
                );
              })}
              {previewChainIds.length > 0 && (
                <>
                  <span className="text-text-tertiary">→</span>
                  <HumanChip label />
                </>
              )}
            </div>
          </div>
        </div>
      </div>
    </Section>
  );
}
