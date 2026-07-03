import type { AgentDefinition } from "../ipc";

// A `cli` instance's skill set is "stale" when its definition's current FULL
// skill id set (builtin + attached custom, same basis/order as the launch
// snapshot — see AgentDefinition.skillIds' doc comment) differs from what its
// session actually launched with (WorkspaceAgent.launchedSkillIds). Order
// matters (mirrors repo::skill::content_for_agent's deterministic ordering),
// so this is a straight array comparison, not a set comparison. `undefined`
// launchedSkillIds (never launched yet) is never stale — nothing to compare
// against. Shared by the Roster (stale badge) and the Context drawer's Skills
// section (the "restart to apply" hint).
export function computeSkillsStale(
  def: AgentDefinition,
  launchedSkillIds: string[] | undefined,
): boolean {
  if (def.type !== "cli") return false;
  if (launchedSkillIds === undefined) return false;
  const current = def.skillIds ?? [];
  if (current.length !== launchedSkillIds.length) return true;
  return current.some((id, i) => id !== launchedSkillIds[i]);
}
