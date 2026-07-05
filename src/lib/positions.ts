import type { WorkspaceAgent } from "../ipc";

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

function rosterById(roster: WorkspaceAgent[]): Map<string, WorkspaceAgent> {
  return new Map(roster.map((agent) => [agent.id, agent]));
}

export function chainUp(agentId: string, roster: WorkspaceAgent[]): string[] {
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

export function reportsOf(agentId: string, roster: WorkspaceAgent[]): string[] {
  return roster
    .filter((agent) => agent.supervisorAgentId === agentId)
    .map((agent) => agent.id);
}

export function rootMembers(roster: WorkspaceAgent[]): string[] {
  return roster
    .filter((agent) => !agent.supervisorAgentId)
    .map((agent) => agent.id);
}

export function wouldCycle(
  agentId: string,
  supervisorId: string,
  roster: WorkspaceAgent[],
): boolean {
  if (agentId === supervisorId) return true;
  return chainUp(supervisorId, roster).includes(agentId);
}

export function lowestCommonSupervisor(
  agentAId: string,
  agentBId: string,
  roster: WorkspaceAgent[],
): string | null {
  const chainA = new Set(chainUp(agentAId, roster));
  for (const candidate of chainUp(agentBId, roster)) {
    if (chainA.has(candidate)) return candidate;
  }
  return null;
}
