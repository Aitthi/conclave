// Mock swarm data for the right-rail chat redesign.
// AGENTS ONLY (lead ruling R8, human directive): the human never appears in this rail —
// human self-sends go via ipc.message.send and never enter InterAgentMessage, so the
// data has no human identity. Agents: Detoro / Mellow / Dew / Arta.

export type AgentColor = "teal" | "red" | "indigo" | "amber" | "sky" | "violet" | "human" | "hash";
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
  tiesto: { id: "tiesto", name: "Tiësto", role: "Frontend Engineer", initials: "T", color: "red", status: "live" },
  guetta: { id: "guetta", name: "Guetta", role: "Engineer", initials: "G", color: "violet", status: "live" },
};

export const avClass: Record<AgentColor, string> = {
  teal: "av-teal", red: "av-red", indigo: "av-indigo", amber: "av-amber", sky: "av-sky",
  violet: "av-violet", human: "av-human", hash: "av-hash",
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
  to?: string;                  // recipient agent id (ruling R10 — per-message recipient chip; absent on system rows)
  text: string;
  at: string;
  day?: string;                 // hub day-divider bucket (e.g. "Today" / "Yesterday")
  queued?: boolean;             // message pending delivery (recipient not live to receive)
  kind?: "text" | "system";
  mono?: boolean;               // render as command / code line
}

// Thread for the #workspace channel — the hero conversation. Agents only (R8).
// Every inter-agent line carries `to` — the aggregate feed shows who each message
// was sent to (ruling R10). Broadcasts still fan out to one bubble per recipient
// (R9 no-dedup); this thread's lines are directed 1:1.
export const workspaceThread: Message[] = [
  { id: "m1", from: "system", kind: "system", text: "Mellow merged 5b73f9c into main · uds server probes before rebinding", at: "09:12" },
  { id: "m2", from: "mellow", to: "detoro", text: "Regression test proven to fail against the old unconditional remove+bind. Gates green: 396 lib tests, clippy -D warnings clean.", at: "09:12" },
  { id: "m3", from: "detoro", to: "mellow", text: "Read the full diff + reran gates on the branch and again on main post-merge. Probe logic correct on all 4 branches.", at: "09:14" },
  { id: "m4", from: "detoro", to: "mellow", text: "RULING: keep the conservative attempt-bind on the ambiguous probe error, bind() can't steal and abort-hard forfeits recovery. Merged ff.", at: "09:14" },
  { id: "m5", from: "mellow", to: "detoro", text: "Confirmed on main post-merge: 0.2.0 is at /Applications but pid 54865 predates the 23:19 install.", at: "09:16" },
  { id: "m6", from: "detoro", to: "dew", text: "So the relaunch hasn't happened yet. Hold the GUI smoke until the running app is quit and reopened.", at: "09:16" },
  { id: "m7", from: "dew", to: "detoro", text: "Saving my handoff now before the relaunch kills my process.", at: "09:16" },
];

export function firstName(id: string): string {
  return agents[id]?.name ?? id;
}

// ── Full-page Chat Hub feed ─────────────────────────────────────────────────
// Directed inter-agent messages across several pairs (R8: agents only, no human).
// Drives the hub's All feed (grouped by sender, day dividers) and the per-pair
// conversation view. Kept SEPARATE from workspaceThread so the gated rail data
// stays frozen. Oldest → newest.
export const hubThread: Message[] = [
  { id: "h1", from: "mellow", to: "detoro", day: "Yesterday", at: "22:14", text: "sock mtime never moved from 22:50:49 — the probe never rebound. Evidence attached." },
  { id: "h2", from: "detoro", to: "mellow", day: "Yesterday", at: "22:19", text: "Confirmed — the unconditional remove+bind is the regression. Writing the guard now." },
  { id: "h3", from: "arta", to: "detoro", day: "Yesterday", at: "23:02", text: "Right-rail chats proto is up: .msg bubbles, group headers, recipient chip below the bubble." },
  { id: "h4", from: "detoro", to: "arta", day: "Yesterday", at: "23:40", text: "Looks right. Take it to the design gate once Dew lands the component move." },
  { id: "h5", from: "mellow", to: "detoro", day: "Today", at: "09:12", text: "Regression test proven to fail against the old remove+bind. Gates green: 396 lib tests, clippy -D warnings clean." },
  { id: "h6", from: "detoro", to: "mellow", day: "Today", at: "09:14", text: "Read the full diff + reran gates on the branch and again on main post-merge. Probe logic correct on all 4 branches." },
  { id: "h7", from: "detoro", to: "mellow", day: "Today", at: "09:14", text: "RULING: keep the conservative attempt-bind on the ambiguous probe error, bind() can't steal and abort-hard forfeits recovery. Merged ff." },
  { id: "h8", from: "detoro", to: "dew", day: "Today", at: "09:16", queued: true, text: "Hold the GUI smoke until the running app is quit and reopened." },
  { id: "h9", from: "dew", to: "detoro", day: "Today", at: "09:16", text: "Saving my handoff now before the relaunch kills my process." },
  { id: "h10", from: "arta", to: "mellow", day: "Today", at: "09:39", text: "Recipient chip moved below the bubble per R11 — the queued label shares that row now." },
];

// Stable pair key for two agent ids, order-independent.
export function pairKey(a: string, b: string): string {
  return [a, b].sort().join("|");
}

export interface Pair { key: string; a: string; b: string; lastAt: string; }

// Conversation pairs present in the hub feed, most-recent last-message first.
export function derivePairs(thread: Message[]): Pair[] {
  const seen = new Map<string, Pair>();
  for (const m of thread) {
    if (!m.to) continue;
    const key = pairKey(m.from, m.to);
    const [a, b] = key.split("|");
    seen.set(key, { key, a, b, lastAt: m.at });  // later messages overwrite → holds last time
  }
  return [...seen.values()];
}
