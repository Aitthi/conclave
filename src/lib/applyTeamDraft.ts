// Apply a drafted team over the EXISTING commands (spec D6): there is no
// transactional `team.apply` in Rust, so the frontend orchestrates
// `role.save → agentDef.save → agentDef.addToWorkspace → instance.setPosition`
// sequentially and reports progress per draft key. On failure it stops and
// keeps whatever was created — a half-applied team is visible in the Library
// and trivially deletable; there is no rollback.

import { ipc } from "../ipc";
import type { DraftAgent, DraftPosition, DraftResponse, WorkspaceAgent } from "../ipc";

export type ApplyStatus = "pending" | "created" | "added" | "positioned" | "failed" | "skipped";

export interface ApplyProgress {
  key: string;
  status: ApplyStatus;
  message?: string;
}

export interface ApplyResult {
  created: number;
  failedKey?: string;
  error?: string;
}

/**
 * Draft keys ordered so every supervisor precedes its reports (Kahn). A
 * `supervisorKey` naming no drafted agent is treated as a root — the engine
 * validator already rejects unknown keys, and an orphan must still be created.
 * Throws on a cycle.
 */
export function topoOrder(positions: DraftPosition[]): string[] {
  const keys = positions.map((p) => p.key);
  const known = new Set(keys);
  const parentOf = new Map<string, string | null>();
  for (const p of positions) {
    const sup = p.supervisorKey && known.has(p.supervisorKey) ? p.supervisorKey : null;
    parentOf.set(p.key, sup === p.key ? null : sup);
  }

  const children = new Map<string, string[]>();
  const indegree = new Map<string, number>();
  for (const key of keys) {
    indegree.set(key, 0);
    children.set(key, []);
  }
  for (const key of keys) {
    const sup = parentOf.get(key) ?? null;
    if (sup === null) continue;
    children.get(sup)!.push(key);
    indegree.set(key, (indegree.get(key) ?? 0) + 1);
  }

  const queue = keys.filter((k) => (indegree.get(k) ?? 0) === 0);
  const order: string[] = [];
  while (queue.length > 0) {
    const key = queue.shift()!;
    order.push(key);
    for (const child of children.get(key) ?? []) {
      const left = (indegree.get(child) ?? 0) - 1;
      indegree.set(child, left);
      if (left === 0) queue.push(child);
    }
  }
  if (order.length !== keys.length) throw new Error("cycle in reporting lines");
  return order;
}

/** The `agentDef.save` payload for a drafted agent — the same constants the
 *  Builder sends (`Builder.tsx` handleSave), so a drafted agent and a
 *  hand-built one launch identically (spec D10: launch flags keep Builder
 *  defaults). */
function saveRequest(agent: DraftAgent, roleId: string, roleName: string | undefined, roleSkillIds: string[]) {
  const cliKind = agent.cliKind ?? "claude-code";
  // ADR 0005 COPY semantics: `agentDef.save` seeds a new definition from the
  // role's default bundle ONLY when `skillIds` is absent — an explicit array,
  // `[]` included, is taken as the final list (commands/agent.rs:247-252). The
  // Builder never hits this because it pre-copies the bundle into its checkbox
  // state, so do the same here: union the role bundle with the drafted extras.
  const skillIds = Array.from(new Set([...roleSkillIds, ...agent.skillIds]));
  return {
    name: agent.name ?? agent.key,
    type: "cli" as const,
    role: roleName,
    roleId,
    cliKind,
    color: agent.color || undefined,
    model: agent.model?.trim() || undefined,
    harnessMode: "central" as const,
    shareBlackboard: true,
    autoSubmitInjected: true,
    allowedSenders: "all" as const,
    // contextWindow is omitted on purpose: absent behaves exactly like the
    // Builder's "200k" default (only "1m" changes the launch, see
    // `effective_claude_model` in commands/instance.rs:54), and Codex derives
    // its window from the model backend-side.
    skillIds,
    defaultLevel: agent.defaultLevel ?? null,
  };
}

/**
 * Create/reuse every drafted agent in supervisor-first order, add each to the
 * workspace, then set its level and supervisor. Progress is reported per key
 * as each step lands. Returns the number of definitions created, plus the
 * failing key and its error when a step throws.
 */
export async function applyTeamDraft(
  draft: DraftResponse,
  workspaceId: string,
  onProgress: (p: ApplyProgress) => void,
): Promise<ApplyResult> {
  const order = topoOrder(draft.positions);
  const agentByKey = new Map(draft.agents.map((a) => [a.key, a]));
  const positionByKey = new Map(draft.positions.map((p) => [p.key, p]));
  // Draft key -> the workspace_agent id it ended up as; supervisors resolve
  // through this map, which is why the order is supervisor-first.
  const keyToWsAgent = new Map<string, string>();

  let roster: WorkspaceAgent[] = await ipc.instance.list({ workspaceId });
  const roles = await ipc.role.list();
  const roleNameById = new Map(roles.map((r) => [r.id, r.name]));
  const roleSkillIdsById = new Map(roles.map((r) => [r.id, r.skillIds ?? []]));

  let created = 0;
  for (const key of order) {
    const agent = agentByKey.get(key);
    if (!agent) continue; // validator guarantees a pairing; skip defensively.
    const position = positionByKey.get(key);
    onProgress({ key, status: "pending" });

    try {
      // ── 1. The definition: reuse, or create (with its new role first) ──
      let defId: string;
      if (agent.existingAgentDefId) {
        defId = agent.existingAgentDefId;
        onProgress({ key, status: "skipped", message: "reused existing definition" });
      } else {
        let roleId = agent.roleId ?? "";
        let roleName = roleId ? roleNameById.get(roleId) : undefined;
        let roleSkillIds = roleId ? (roleSkillIdsById.get(roleId) ?? []) : [];
        if (agent.newRole) {
          const role = await ipc.role.save({
            name: agent.newRole.name,
            description: agent.newRole.description,
            skillIds: agent.newRole.skillIds,
          });
          roleId = role.id;
          roleName = role.name;
          roleSkillIds = role.skillIds ?? [];
          roleNameById.set(role.id, role.name);
          roleSkillIdsById.set(role.id, roleSkillIds);
        }
        const def = await ipc.agentDef.save(saveRequest(agent, roleId, roleName, roleSkillIds));
        defId = def.id;
        created += 1;
        onProgress({ key, status: "created" });
      }

      // ── 2. Membership: reuse the roster row when one already exists ──
      const existingRow = roster.find((r) => r.agentDefId === defId);
      let workspaceAgentId: string;
      if (existingRow) {
        workspaceAgentId = existingRow.id;
        onProgress({ key, status: "skipped", message: "already in this workspace" });
      } else {
        await ipc.agentDef.addToWorkspace({ agentDefId: defId, workspaceIds: [workspaceId] });
        const before = new Set(roster.map((r) => r.id));
        roster = await ipc.instance.list({ workspaceId });
        const added = roster.find((r) => r.agentDefId === defId && !before.has(r.id));
        if (!added) throw new Error(`could not find the new roster row for ${agent.name ?? key}`);
        workspaceAgentId = added.id;
        onProgress({ key, status: "added" });
      }

      // ── 3. Position: level + supervisor (already applied, hence in order) ──
      const supervisorAgentId = position?.supervisorKey
        ? (keyToWsAgent.get(position.supervisorKey) ?? null)
        : null;
      await ipc.instance.setPosition({
        workspaceId,
        workspaceAgentId,
        level: position?.level ?? agent.defaultLevel ?? null,
        supervisorAgentId,
      });
      keyToWsAgent.set(key, workspaceAgentId);
      onProgress({ key, status: "positioned" });
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      onProgress({ key, status: "failed", message });
      return { created, failedKey: key, error: message };
    }
  }

  return { created };
}
