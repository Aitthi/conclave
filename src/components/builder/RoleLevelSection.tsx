// src/components/builder/RoleLevelSection.tsx
//
// Role & Level (spec D3 + D4). The role picker is one row of four compact
// cards; the selected role's tagline, attached skills and the mandatory
// always-on note sit under the row (canon rules 12-14). Level is the shared
// segmented control - Unranked plus the four rungs - replacing the old four
// level cards and the "Clear to Unranked" link (canon rule 15).
//
// The inline custom-role editor is moved verbatim from Builder.tsx.

import type { Dispatch, SetStateAction } from "react";
import {
  Compass,
  Hammer,
  Microscope,
  PenTool,
  Plus,
  ShieldCheck,
  UserPen,
} from "lucide-react";
import type { AgentDefinition, Role, Skill } from "../../ipc";
import { LEVELS } from "../../lib/positions";
import { Section } from "./Section";

// ── Builtin role card looks (ADR 0005) ───────────────────────────────────────

/**
 * The backend `Role` (id / name / description / skillIds / kind) carries no
 * icon or tagline - the card design assigns one per builtin role id. Custom
 * (user-created) roles have no designed look, so they fall back to a neutral
 * icon; their own `description` stands in for the tagline.
 */
const BUILTIN_ROLE_LOOKS: Record<
  string,
  { Icon: typeof Compass; tagline: string }
> = {
  lead: { Icon: Compass, tagline: "Settles & delegates work" },
  reviewer: { Icon: ShieldCheck, tagline: "Grills work with evidence" },
  implementer: { Icon: Hammer, tagline: "Builds the recorded plan" },
  designer: { Icon: PenTool, tagline: "Designs on the canvas" },
  researcher: { Icon: Microscope, tagline: "Investigates open questions" },
};

export function roleLook(role: Role): {
  Icon: typeof Compass;
  tagline: string;
} {
  return (
    BUILTIN_ROLE_LOOKS[role.id] ?? { Icon: UserPen, tagline: "Custom role" }
  );
}

/** Canon rule 13 shows ONE descriptive line under the role row. A builtin has a
 *  designed tagline; a custom role has only its description, which is the whole
 *  reason the user wrote it - so that wins when there is no designed look. */
function roleTagline(role: Role): string {
  const designed = BUILTIN_ROLE_LOOKS[role.id];
  return designed ? designed.tagline : role.description || "Custom role";
}

interface RoleLevelSectionProps {
  orderedRoles: Role[];
  selectedRole: Role | undefined;
  roleId: string;
  selectRole: (id: string) => void;
  clearRole: () => void;
  openCustomRole: () => void;
  customRoleOpen: boolean;
  customRoleName: string;
  setCustomRoleName: (v: string) => void;
  customRoleDesc: string;
  setCustomRoleDesc: (v: string) => void;
  customRoleSkillIds: string[];
  setCustomRoleSkillIds: Dispatch<SetStateAction<string[]>>;
  cancelCustomRole: () => void;
  handleCreateCustomRole: () => void;
  savingRole: boolean;
  allSkills: Skill[];
  attachSkillNames: string[];
  mandatorySkillNames: string[];
  defaultLevel: AgentDefinition["defaultLevel"];
  setDefaultLevel: (v: AgentDefinition["defaultLevel"]) => void;
}

export function RoleLevelSection({
  orderedRoles,
  selectedRole,
  roleId,
  selectRole,
  clearRole,
  openCustomRole,
  customRoleOpen,
  customRoleName,
  setCustomRoleName,
  customRoleDesc,
  setCustomRoleDesc,
  customRoleSkillIds,
  setCustomRoleSkillIds,
  cancelCustomRole,
  handleCreateCustomRole,
  savingRole,
  allSkills,
  attachSkillNames,
  mandatorySkillNames,
  defaultLevel,
  setDefaultLevel,
}: RoleLevelSectionProps) {
  // Unranked plus the live rungs, in rung order (never hardcode the names).
  const levelOptions: { id: AgentDefinition["defaultLevel"]; name: string }[] =
    [
      { id: null, name: "Unranked" },
      ...LEVELS.map((l) => ({
        id: l.id as AgentDefinition["defaultLevel"],
        name: l.name,
      })),
    ];

  return (
    <Section
      id="role"
      title="Role & Level"
      actions={
        <span className="flex items-center gap-2 text-[11px] font-medium">
          <button
            type="button"
            onClick={clearRole}
            className={`transition-colors ${
              roleId === "" && !customRoleOpen
                ? "text-accent"
                : "text-text-tertiary hover:text-text-secondary"
            }`}
          >
            No role
          </button>
          <span className="text-text-tertiary" aria-hidden="true">
            ·
          </span>
          <button
            type="button"
            onClick={openCustomRole}
            aria-pressed={customRoleOpen}
            className={`inline-flex items-center gap-1 transition-colors ${
              customRoleOpen
                ? "text-accent"
                : "text-text-tertiary hover:text-text-secondary"
            }`}
          >
            <Plus className="w-3 h-3" />
            Custom…
          </button>
        </span>
      }
    >
      {/* Role row — one line of compact cards (D3). Custom roles beyond the
          builtins wrap to a second row of the same grid. */}
      <div
        role="radiogroup"
        aria-label="Role"
        className="grid grid-cols-4 gap-2"
      >
        {orderedRoles.map((r) => {
          const { Icon } = roleLook(r);
          const active = roleId === r.id && !customRoleOpen;
          return (
            <button
              key={r.id}
              type="button"
              role="radio"
              aria-checked={active}
              onClick={() => selectRole(r.id)}
              className={`rounded-xl p-2.5 text-left transition-colors ring-1 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                active
                  ? "ring-accent/40 bg-accent/[0.06]"
                  : "ring-overlay/[0.08] bg-surface hover:bg-overlay/[0.02]"
              }`}
            >
              <Icon
                className={`w-[17px] h-[17px] mb-1.5 ${
                  active ? "text-accent" : "text-text-secondary"
                }`}
              />
              <div className="text-[12.5px] font-semibold leading-tight">
                {r.name}
              </div>
            </button>
          );
        })}
      </div>

      {/* Selected role: tagline, then the skills it attaches (canon rule 13). */}
      {!customRoleOpen && selectedRole && (
        <div className="mt-2.5">
          <p className="text-[11.5px] text-text-secondary leading-relaxed">
            {roleTagline(selectedRole)}
          </p>
          <div className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1.5">
            <span className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase">
              Attaches
            </span>
            {attachSkillNames.length > 0 ? (
              attachSkillNames.map((s) => (
                <span
                  key={s}
                  className="text-[11px] font-medium px-2 py-0.5 rounded-md ring-1 ring-accent/40 bg-accent/[0.08] text-accent"
                >
                  {s}
                </span>
              ))
            ) : (
              <span className="text-[11px] text-text-tertiary italic">
                mandatory skills only
              </span>
            )}
            {mandatorySkillNames.length > 0 && (
              <span className="text-[10.5px] text-text-tertiary">
                + {mandatorySkillNames.join(", ")}, always on
              </span>
            )}
          </div>
        </div>
      )}

      {/* No-role note (canon rule 13, copy deck). */}
      {!customRoleOpen && !selectedRole && (
        <p className="mt-2.5 text-[11.5px] leading-relaxed text-text-tertiary">
          No role: the agent runs with only the mandatory
          {mandatorySkillNames.length > 0
            ? ` ${mandatorySkillNames.join(" and ")} `
            : " "}
          skills, and no job description in its preamble.
        </p>
      )}

      {/* Inline custom-role editor — moved verbatim. */}
      {customRoleOpen && (
        <div className="mt-2 rounded-xl ring-1 ring-overlay/[0.08] bg-surface p-3 space-y-2">
          <input
            value={customRoleName}
            onChange={(e) => setCustomRoleName(e.target.value)}
            placeholder="Role name"
            className="w-full text-[12.5px] font-semibold bg-transparent outline-none border-b border-overlay/10 focus:border-accent pb-0.5"
          />
          <textarea
            value={customRoleDesc}
            onChange={(e) => setCustomRoleDesc(e.target.value)}
            placeholder="One-paragraph job description (baked into the agent's preamble)"
            rows={3}
            className="w-full text-[12px] text-text-secondary bg-transparent outline-none ring-1 ring-overlay/[0.08] rounded-lg px-2 py-1.5 resize-none focus:ring-accent"
          />
          {allSkills.filter(
            (s) =>
              (s.kind === "builtin" && !s.mandatory) || s.kind === "custom",
          ).length > 0 && (
            <div>
              <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1">
                Default skills
              </div>
              <div className="space-y-1 max-h-32 overflow-y-auto">
                {allSkills
                  .filter(
                    (s) =>
                      (s.kind === "builtin" && !s.mandatory) ||
                      s.kind === "custom",
                  )
                  .map((s) => {
                    const checked = customRoleSkillIds.includes(s.id);
                    return (
                      <label
                        key={s.id}
                        className="flex items-center gap-2 text-[12px] text-text-secondary cursor-pointer"
                      >
                        <input
                          type="checkbox"
                          checked={checked}
                          onChange={(e) =>
                            setCustomRoleSkillIds((prev) =>
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
          <div className="flex items-center justify-end gap-2 pt-0.5">
            <button
              onClick={cancelCustomRole}
              className="text-[12px] font-medium text-text-secondary px-3 py-1 rounded-lg hover:bg-overlay/[0.05]"
            >
              Cancel
            </button>
            <button
              onClick={handleCreateCustomRole}
              disabled={savingRole}
              className="text-[12px] font-semibold text-white bg-accent px-3 py-1 rounded-lg hover:brightness-105 disabled:opacity-60"
            >
              {savingRole ? "Creating…" : "Create role"}
            </button>
          </div>
        </div>
      )}

      {/* Level — the Position System SEED (D1/D3): remembered on the definition
          so it is restored whenever a new instance is created from it. Distinct
          from the Position section, which edits an EXISTING instance's live
          level and never touches this value (canon rule 30). */}
      <div className="mt-3.5 flex items-center justify-between gap-3">
        <span className="text-[12.5px] text-text-secondary">Level</span>
        <div
          role="radiogroup"
          aria-label="Level"
          className="flex rounded-lg bg-overlay/[0.04] p-0.5"
        >
          {levelOptions.map((opt) => {
            const active = (defaultLevel ?? null) === opt.id;
            return (
              <button
                key={opt.name}
                type="button"
                role="radio"
                aria-checked={active}
                onClick={() => setDefaultLevel(opt.id)}
                className={`rounded-[7px] px-2.5 py-1 text-[11.5px] transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                  active
                    ? "bg-surface font-semibold text-text-primary shadow-sm"
                    : "text-text-secondary hover:text-text-primary"
                }`}
              >
                {opt.name}
              </button>
            );
          })}
        </div>
      </div>
    </Section>
  );
}
