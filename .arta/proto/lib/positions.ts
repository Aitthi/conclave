// Position System model (design canon for the Conclave feature).
// Track + Level + supervisor link per workspace membership, mirroring the
// engine spec (position-spec): a member has a TRACK (= its role), a LEVEL
// (a small ordered set within the track), and a SUPERVISOR link to another
// member — null means it reports to the HUMAN at the top of the chain.
// Escalations/challenges/stall alerts route UP this chain.
//
// Level set is the human's example (Junior/Mid/Senior/Principal) pending the
// position-spec draft; the visuals render any small ordered set, so swapping
// the array below is the only change if the spec lands on different rungs.

import { agents, avClass, type Agent, type AgentColor } from "./data";

export type Track = "Lead" | "Reviewer" | "Implementer" | "Designer" | "Researcher";

export interface Level {
  id: string;
  name: string;
  short: string; // compact tag form (e.g. roster card)
  rung: number; // 1..MAX_RUNG, ascending seniority
}

export const LEVELS: Level[] = [
  { id: "junior", name: "Junior", short: "Jr", rung: 1 },
  { id: "mid", name: "Mid", short: "Mid", rung: 2 },
  { id: "senior", name: "Senior", short: "Sr", rung: 3 },
  { id: "principal", name: "Principal", short: "Prin", rung: 4 },
];
export const MAX_RUNG = LEVELS.length;
export const levelOf = (id: string): Level => LEVELS.find((l) => l.id === id) ?? LEVELS[0];

export interface Position {
  agentId: string;
  track: Track;
  /** a level id, or null = UNRANKED (backward-compat for members with no level). */
  levelId: string | null;
  /** another member's id, or null = reports to the human (top of chain). */
  supervisorId: string | null;
  /** owns a domain as sub-lead (lead stays the tiebreaker). Display hint only. */
  subLeadOf?: string;
}

// Positions over the canonical mock swarm (lib/data.ts). A 3-deep chain
// (Dew → Tiësto → Detoro → Human) exists on purpose so the org tree and the
// escalation trace have real depth to render.
export const positions: Record<string, Position> = {
  detoro: { agentId: "detoro", track: "Lead", levelId: "principal", supervisorId: null },
  mellow: { agentId: "mellow", track: "Reviewer", levelId: "senior", supervisorId: "detoro" },
  tiesto: { agentId: "tiesto", track: "Implementer", levelId: "senior", supervisorId: "detoro", subLeadOf: "Frontend" },
  dew: { agentId: "dew", track: "Implementer", levelId: "mid", supervisorId: "tiesto" },
  guetta: { agentId: "guetta", track: "Researcher", levelId: "mid", supervisorId: "detoro" },
  arta: { agentId: "arta", track: "Designer", levelId: "senior", supervisorId: "detoro" },
};

export const positionOf = (id: string): Position | undefined => positions[id];

// ── chain helpers ───────────────────────────────────────────────────────────

/** Ids from `agentId` up to the top, e.g. ["dew","tiesto","detoro"].
 *  The human is the implicit top when the last member's supervisor is null.
 *  Cycle-safe: a visited set stops a→b→a from looping forever. */
export function chainUp(agentId: string): string[] {
  const chain: string[] = [];
  const seen = new Set<string>();
  let cur: string | null = agentId;
  while (cur && !seen.has(cur)) {
    seen.add(cur);
    chain.push(cur);
    cur = positions[cur]?.supervisorId ?? null;
  }
  return chain;
}

/** Direct reports of `agentId` (members whose supervisor is this one). */
export function reportsOf(agentId: string): string[] {
  return Object.values(positions)
    .filter((p) => p.supervisorId === agentId)
    .map((p) => p.agentId);
}

/** Members who report to the human (top-level roots of the org tree). */
export function rootMembers(): string[] {
  return Object.values(positions)
    .filter((p) => p.supervisorId === null)
    .map((p) => p.agentId);
}

/** Would setting `supervisorId` as `agentId`'s supervisor create a cycle?
 *  True when the candidate is the agent itself or already reports up to it. */
export function wouldCycle(agentId: string, supervisorId: string): boolean {
  if (agentId === supervisorId) return true;
  return chainUp(supervisorId).includes(agentId);
}

export type { Agent, AgentColor };
export { agents, avClass };
