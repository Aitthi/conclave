// src/components/builder/SkillsSection.tsx
//
// Skills picker: always-on chips, optional system skills, custom skills.
// Card moved verbatim from Builder.tsx (spec D9, canon rule 28).

import type { Dispatch, SetStateAction } from "react";
import type { Skill } from "../../ipc";
import { Section } from "./Section";

interface SkillsSectionProps {
  allSkills: Skill[];
  skillIds: string[];
  setSkillIds: Dispatch<SetStateAction<string[]>>;
}

export function SkillsSection({
  allSkills,
  skillIds,
  setSkillIds,
}: SkillsSectionProps) {
  return (
    <Section id="skills" title="Skills">
      <div className="rounded-xl ring-1 ring-overlay/[0.08] bg-surface divide-y divide-overlay/[0.06]">
        {allSkills.filter((s) => s.kind === "builtin" && s.mandatory).length >
          0 && (
          <div className="px-3 py-2">
            <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
              System skills — always on
            </div>
            <div className="flex flex-wrap gap-1.5">
              {allSkills
                .filter((s) => s.kind === "builtin" && s.mandatory)
                .map((s) => (
                  <span
                    key={s.id}
                    className="text-[11px] font-medium px-2 py-0.5 rounded-md ring-1 ring-overlay/[0.08] text-text-secondary"
                  >
                    {s.name}
                  </span>
                ))}
            </div>
          </div>
        )}
        {allSkills.filter((s) => s.kind === "builtin" && !s.mandatory).length >
          0 && (
          <div className="px-3 py-2">
            <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
              System skills — optional
            </div>
            <div className="space-y-1">
              {allSkills
                .filter((s) => s.kind === "builtin" && !s.mandatory)
                .map((s) => {
                  const checked = skillIds.includes(s.id);
                  return (
                    <label
                      key={s.id}
                      className="flex items-center gap-2 text-[12.5px] text-text-secondary cursor-pointer"
                    >
                      <input
                        type="checkbox"
                        checked={checked}
                        onChange={(e) =>
                          setSkillIds((prev) =>
                            e.target.checked
                              ? [...prev, s.id]
                              : prev.filter((id) => id !== s.id),
                          )
                        }
                      />
                      {s.name}
                    </label>
                  );
                })}
            </div>
          </div>
        )}
        <div className="px-3 py-2">
          <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
            Custom skills
          </div>
          {allSkills.filter((s) => s.kind === "custom").length === 0 ? (
            <p className="text-[11.5px] text-text-tertiary">
              No custom skills yet — create one in the Skill Library.
            </p>
          ) : (
            <div className="space-y-1">
              {allSkills
                .filter((s) => s.kind === "custom")
                .map((s) => {
                  const checked = skillIds.includes(s.id);
                  return (
                    <label
                      key={s.id}
                      className="flex items-center gap-2 text-[12.5px] text-text-secondary cursor-pointer"
                    >
                      <input
                        type="checkbox"
                        checked={checked}
                        onChange={(e) =>
                          setSkillIds((prev) =>
                            e.target.checked
                              ? [...prev, s.id]
                              : prev.filter((id) => id !== s.id),
                          )
                        }
                      />
                      {s.name}
                    </label>
                  );
                })}
            </div>
          )}
        </div>
      </div>
    </Section>
  );
}
