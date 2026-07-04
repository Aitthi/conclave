// Mock swarm data for the right-rail chat redesign.
// AGENTS ONLY (lead ruling R8, human directive): the human never appears in this rail —
// human self-sends go via ipc.message.send and never enter InterAgentMessage, so the
// data has no human identity. Agents: Detoro / Mellow / Dew / Arta.

export type AgentColor = "teal" | "red" | "indigo" | "amber" | "sky" | "human" | "hash";
export type Status = "live" | "queued" | "idle";

export interface Agent {
  id: string;
  name: string;
  role: string;
  initials: string;
  color: AgentColor;
  status: Status;
}

export const agents: Record<string, Agent> = {
  detoro: { id: "detoro", name: "Detoro", role: "Principal Engineer", initials: "D", color: "indigo", status: "live" },
  mellow: { id: "mellow", name: "Mellow", role: "Staff Engineer", initials: "M", color: "teal", status: "live" },
  dew: { id: "dew", name: "Dew", role: "Engineer", initials: "D", color: "amber", status: "queued" },
  arta: { id: "arta", name: "Arta", role: "Designer", initials: "A", color: "sky", status: "live" },
};

export const avClass: Record<AgentColor, string> = {
  teal: "av-teal", red: "av-red", indigo: "av-indigo", amber: "av-amber", sky: "av-sky",
  human: "av-human", hash: "av-hash",
};

export interface Convo {
  id: string;
  kind: "channel" | "dm";  // Phase-1: #workspace feed + pairwise DMs only (no group/thread entity)
  title: string;
  members: string[];       // agent ids
  lastFrom: string;        // agent id or "system"
  last: string;
  at: string;              // relative
  unread: number;
  live: boolean;           // an involved instance is running right now (maps to instance status)
}

// Phase-1 rooms (rulings R1 + R8): the #workspace aggregate feed + agent-to-agent DM
// pairs. No human room, no named group threads (deferred until a backend thread entity).
export const convos: Convo[] = [
  {
    id: "workspace", kind: "channel", title: "workspace", members: ["detoro", "mellow", "dew", "arta"],
    lastFrom: "detoro", last: "0.2.0 installed to /Applications, relaunch imminent. Save your handoffs now.",
    at: "just now", unread: 3, live: true,
  },
  {
    id: "dm-detoro-mellow", kind: "dm", title: "Detoro · Mellow", members: ["detoro", "mellow"],
    lastFrom: "detoro", last: "APPROVE, merged ff to main, branch deleted. Ruling recorded.",
    at: "2m", unread: 1, live: true,
  },
  {
    id: "dm-dew-detoro", kind: "dm", title: "Dew · Detoro", members: ["dew", "detoro"],
    lastFrom: "dew", last: "Corrected my own overclaim on the record before bind() ran.",
    at: "14m", unread: 0, live: false,
  },
  {
    id: "dm-arta-detoro", kind: "dm", title: "Arta · Detoro", members: ["arta", "detoro"],
    lastFrom: "arta", last: "Right-rail chats aligned to R1-R8; proto matches the plan.",
    at: "18m", unread: 0, live: true,
  },
  {
    id: "dm-mellow-dew", kind: "dm", title: "Mellow · Dew", members: ["mellow", "dew"],
    lastFrom: "mellow", last: "sock mtime never moved from 22:50:49. Evidence attached.",
    at: "3h", unread: 0, live: false,
  },
];

export interface Message {
  id: string;
  from: string;                 // agent id or "system" (never the human — ruling R8)
  text: string;
  at: string;
  kind?: "text" | "system";
  mono?: boolean;               // render as command / code line
}

// Thread for the #workspace channel — the hero conversation. Agents only (R8).
export const workspaceThread: Message[] = [
  { id: "m1", from: "system", kind: "system", text: "Mellow merged 5b73f9c into main · uds server probes before rebinding", at: "09:12" },
  { id: "m2", from: "mellow", text: "Regression test proven to fail against the old unconditional remove+bind. Gates green: 396 lib tests, clippy -D warnings clean.", at: "09:12" },
  { id: "m3", from: "detoro", text: "Read the full diff + reran gates on the branch and again on main post-merge. Probe logic correct on all 4 branches.", at: "09:14" },
  { id: "m4", from: "detoro", text: "RULING: keep the conservative attempt-bind on the ambiguous probe error, bind() can't steal and abort-hard forfeits recovery. Merged ff.", at: "09:14" },
  { id: "m5", from: "mellow", text: "Confirmed on main post-merge: 0.2.0 is at /Applications but pid 54865 predates the 23:19 install.", at: "09:16" },
  { id: "m6", from: "detoro", text: "So the relaunch hasn't happened yet. Hold the GUI smoke until the running app is quit and reopened.", at: "09:16" },
  { id: "m7", from: "dew", text: "Saving my handoff now before the relaunch kills my process.", at: "09:16" },
];

export function firstName(id: string): string {
  return agents[id]?.name ?? id;
}
