// A `WorkspaceAgent[]` structurally satisfies this — every existing caller
// (Builder's positionRoster) keeps compiling unchanged. Narrowed so callers
// that only have a flattened view model (Roster's RosterEntry) don't need to
// fabricate the rest of WorkspaceAgent's fields just to walk the chain.
export interface PositionLink {
  id: string;
  supervisorAgentId?: string | null;
}

export interface Level {
  id: string;
  name: string;
  short: string;
  rung: number;
}

export const LEVELS: Level[] = [
  { id: "junior", name: "Junior", short: "Jr", rung: 1 },
  { id: "mid", name: "Mid", short: "Mid", rung: 2 },
  { id: "senior", name: "Senior", short: "Sr", rung: 3 },
  { id: "principal", name: "Principal", short: "Prin", rung: 4 },
];

export const MAX_RUNG = LEVELS.length;

export function levelOf(id: string): Level {
  return LEVELS.find((level) => level.id === id) ?? LEVELS[0];
}

function rosterById(roster: PositionLink[]): Map<string, PositionLink> {
  return new Map(roster.map((agent) => [agent.id, agent]));
}

export function chainUp(agentId: string, roster: PositionLink[]): string[] {
  const chain: string[] = [];
  const seen = new Set<string>();
  const byId = rosterById(roster);
  let current: string | null = agentId;
  while (current && !seen.has(current)) {
    seen.add(current);
    chain.push(current);
    current = byId.get(current)?.supervisorAgentId ?? null;
  }
  return chain;
}

export function reportsOf(agentId: string, roster: PositionLink[]): string[] {
  return roster
    .filter((agent) => agent.supervisorAgentId === agentId)
    .map((agent) => agent.id);
}

export function rootMembers(roster: PositionLink[]): string[] {
  return roster
    .filter((agent) => !agent.supervisorAgentId)
    .map((agent) => agent.id);
}

export function wouldCycle(
  agentId: string,
  supervisorId: string,
  roster: PositionLink[],
): boolean {
  if (agentId === supervisorId) return true;
  return chainUp(supervisorId, roster).includes(agentId);
}

/** Every agent transitively reporting up to `agentId` (direct + indirect
 *  reports), NOT including `agentId` itself. Picking one of these as
 *  `agentId`'s supervisor would create a cycle — same rule `wouldCycle`
 *  checks from the candidate-supervisor's side, exposed here as a set for
 *  callers (SupervisorPicker) that need to disable multiple rows at once. */
export function descendantsOf(agentId: string, roster: PositionLink[]): string[] {
  const result: string[] = [];
  const seen = new Set<string>([agentId]);
  const queue = [agentId];
  while (queue.length > 0) {
    const current = queue.shift()!;
    for (const childId of reportsOf(current, roster)) {
      if (seen.has(childId)) continue;
      seen.add(childId);
      result.push(childId);
      queue.push(childId);
    }
  }
  return result;
}

export function lowestCommonSupervisor(
  agentAId: string,
  agentBId: string,
  roster: PositionLink[],
): string | null {
  const chainA = new Set(chainUp(agentAId, roster));
  for (const candidate of chainUp(agentBId, roster)) {
    if (chainA.has(candidate)) return candidate;
  }
  return null;
}
