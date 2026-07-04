import { useState } from "react";
import { MessageSquare, Search, X } from "lucide-react";
import { agents, avClass, hubThread, derivePairs, pairKey, type Message } from "../lib/data";

export const meta = { title: "Chat Hub · Full page" };

/* Full-page Chat Hub — the workspace's inter-agent conversation at center-pane
   scale, aligned to the right-rail canon (ruling R-hub-1): .msg bubbles (no
   colored side-stripe), sender-group headers name·role·HH:MM, day dividers,
   recipient chip below the bubble (R10-amended), queued label convention.
   Keeps the sidebar + search + All/pair-view structure of the shipped hub. */

/* Two overlapping avatars for a conversation pair (mirrors the rail's DM cluster). */
function PairAv({ a, b }: { a: string; b: string }) {
  return (
    <span className="inline-flex items-center -space-x-1.5 shrink-0">
      <span className={`av av-sm ${avClass[agents[a].color]}`} style={{ boxShadow: "0 0 0 1.5px var(--color-sidebar)" }}>{agents[a].initials}</span>
      <span className={`av av-sm ${avClass[agents[b].color]}`} style={{ boxShadow: "0 0 0 1.5px var(--color-sidebar)" }}>{agents[b].initials}</span>
    </span>
  );
}

type Row =
  | { type: "divider"; day: string }
  | { type: "group"; from: string; at: string; items: Message[] };

/* Build a render list: day dividers + consecutive same-sender groups. */
function buildFeed(msgs: Message[]): Row[] {
  const rows: Row[] = [];
  let day: string | null = null;
  for (const m of msgs) {
    if (m.day && m.day !== day) { day = m.day; rows.push({ type: "divider", day }); }
    const last = rows[rows.length - 1];
    if (last && last.type === "group" && last.from === m.from) last.items.push(m);
    else rows.push({ type: "group", from: m.from, at: m.at, items: [m] });
  }
  return rows;
}

/* The recipient chip + queued label row that sits BELOW each bubble (rail canon). */
function MetaRow({ m, showRecipient }: { m: Message; showRecipient: boolean }) {
  const to = m.to ? agents[m.to] : null;
  return (
    <div className="self-stretch flex items-center gap-1">
      {m.queued && <span className="text-[0.56rem]" style={{ color: "var(--color-queued)" }}>queued</span>}
      <span className="faint num text-[0.6rem]">{m.at}</span>
      {showRecipient && to && (
        <span className="ml-auto inline-flex items-center gap-1 text-[0.62rem] faint" title={`→ ${to.name}`}>
          <span className={`av av-xs ${avClass[to.color]}`}>{to.initials}</span>
          {to.name}
        </span>
      )}
    </div>
  );
}

export default function ChatHub() {
  const [selected, setSelected] = useState<string | null>(null); // null = All
  const [query, setQuery] = useState("");

  const pairs = derivePairs(hubThread);
  const q = query.trim().toLowerCase();

  const filtered = hubThread.filter((m) => {
    if (selected && pairKey(m.from, m.to ?? m.from) !== selected) return false;
    if (!q) return true;
    return (
      m.text.toLowerCase().includes(q) ||
      agents[m.from].name.toLowerCase().includes(q) ||
      (m.to ? agents[m.to].name.toLowerCase().includes(q) : false)
    );
  });

  const rows = buildFeed(filtered);
  const leftId = selected ? selected.split("|")[0] : null;

  return (
    <div className="h-screen w-full flex flex-col overflow-hidden" style={{ background: "var(--color-app)" }}>
      {/* header */}
      <div className="h-12 shrink-0 flex items-center justify-between px-5 border-b" style={{ borderColor: "var(--color-border)" }}>
        <div className="flex items-center gap-2.5">
          <span className="w-6 h-6 rounded-[7px] grid place-items-center" style={{ background: "var(--color-raised)", color: "var(--color-heading)", boxShadow: "inset 0 0 0 1px var(--color-border)" }}>
            <MessageSquare size={13} />
          </span>
          <span className="heading text-[0.86rem] font-semibold tracking-tight">Chat</span>
          <span className="text-[0.62rem] font-medium faint px-1.5 py-px rounded-md" style={{ background: "color-mix(in srgb, var(--color-faint) 12%, transparent)" }}>read-only</span>
        </div>
        <div className="flex items-center gap-2">
          <div className="flex items-center gap-2 rounded-lg px-2.5 h-7 w-52" style={{ background: "color-mix(in srgb, var(--color-faint) 10%, transparent)" }}>
            <Search size={13} className="faint shrink-0" />
            <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search messages"
              className="bg-transparent outline-none text-[0.76rem] w-full" style={{ color: "var(--color-text)" }} />
          </div>
          <button title="Close Chat" className="w-7 h-7 grid place-items-center rounded-md dim hover:text-[var(--color-heading)]"><X size={15} /></button>
        </div>
      </div>

      <div className="flex-1 flex min-h-0">
        {/* sidebar — All + conversation pairs */}
        <aside className="w-[240px] shrink-0 border-r overflow-y-auto scroll-thin p-2 flex flex-col gap-0.5" style={{ borderColor: "var(--color-border)", background: "var(--color-sidebar)" }}>
          <button onClick={() => setSelected(null)}
            className="w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg text-left transition-colors"
            style={selected === null ? { background: "color-mix(in srgb, var(--color-accent) 12%, transparent)" } : undefined}>
            <span className="w-[22px] h-[22px] rounded-md grid place-items-center shrink-0" style={{ background: "var(--color-raised)", color: "var(--color-heading)" }}><MessageSquare size={12} /></span>
            <span className="text-[0.78rem] font-semibold" style={{ color: selected === null ? "var(--color-heading)" : "var(--color-dim)" }}>All</span>
            <span className="ml-auto text-[0.62rem] faint num">{hubThread.length}</span>
          </button>
          <div className="label faint px-2 pt-2 pb-1">Conversations</div>
          {pairs.map((p) => {
            const on = p.key === selected;
            return (
              <button key={p.key} onClick={() => setSelected(p.key)}
                className="w-full flex items-center gap-2.5 px-2 py-1.5 rounded-lg text-left transition-colors"
                style={on ? { background: "color-mix(in srgb, var(--color-accent) 12%, transparent)" } : undefined}>
                <PairAv a={p.a} b={p.b} />
                <span className="text-[0.78rem] font-medium clamp-1 flex-1 min-w-0" style={{ color: on ? "var(--color-heading)" : "var(--color-dim)" }}>
                  {agents[p.a].name} · {agents[p.b].name}
                </span>
                <span className="text-[0.6rem] faint num shrink-0">{p.lastAt}</span>
              </button>
            );
          })}
        </aside>

        {/* timeline */}
        <div className="flex-1 min-w-0 overflow-y-auto scroll-thin px-6 py-4 flex flex-col gap-3">
          {rows.length === 0 ? (
            <div className="text-[0.76rem] faint py-6 text-center">{q ? "No messages match the search" : "No messages yet"}</div>
          ) : selected === null ? (
            // All feed — sender groups, left-aligned (a hub has no "self", so bubble
            // sides would be a lie). Recipient chip below each bubble carries direction.
            rows.map((r, i) => {
              if (r.type === "divider") return <DayDivider key={i} day={r.day} />;
              const a = agents[r.from];
              return (
                <div key={i} className="flex gap-2.5">
                  <span className={`av av-md ${avClass[a.color]} mt-0.5`}>{a.initials}</span>
                  <div className="min-w-0 flex flex-col gap-1 items-start" style={{ maxWidth: "72%" }}>
                    <div className="flex items-baseline gap-2">
                      <span className="text-[0.8rem] font-semibold heading">{a.name}</span>
                      <span className="text-[0.64rem] faint">{a.role}</span>
                      <span className="text-[0.64rem] faint num">{r.at}</span>
                    </div>
                    {r.items.map((m) => (
                      <div key={m.id} className="flex flex-col gap-0.5 items-start self-stretch">
                        <div className="msg self-start" title={m.to ? `→ ${agents[m.to].name}` : undefined}>{m.text}</div>
                        <MetaRow m={m} showRecipient />
                      </div>
                    ))}
                  </div>
                </div>
              );
            })
          ) : (
            // Pair view — a two-party conversation: stable side per participant
            // (pair-key order: lexicographically-first id sits left). Recipient is
            // implied here, so the meta row carries time + queued only.
            rows.map((r, i) => {
              if (r.type === "divider") return <DayDivider key={i} day={r.day} />;
              const onLeft = r.from === leftId;
              const a = agents[r.from];
              return (
                <div key={i} className={`flex flex-col gap-1 ${onLeft ? "items-start" : "items-end"}`}>
                  <div className="flex items-center gap-1.5">
                    <span className={`av av-sm ${avClass[a.color]}`}>{a.initials}</span>
                    <span className="text-[0.72rem] font-semibold heading">{a.name}</span>
                    <span className="text-[0.62rem] faint num">{r.at}</span>
                  </div>
                  {r.items.map((m) => (
                    <div key={m.id} className={`flex flex-col gap-0.5 ${onLeft ? "items-start" : "items-end"}`} style={{ maxWidth: "72%" }}>
                      <div className="msg">{m.text}</div>
                      <div className="flex items-center gap-1.5 text-[0.6rem] faint num">
                        {m.queued && <span style={{ color: "var(--color-queued)" }}>queued</span>}
                        {m.at}
                      </div>
                    </div>
                  ))}
                </div>
              );
            })
          )}
        </div>
      </div>
    </div>
  );
}

function DayDivider({ day }: { day: string }) {
  return (
    <div className="flex items-center gap-2 justify-center py-0.5">
      <span className="h-px flex-1" style={{ background: "var(--color-border-soft)" }} />
      <span className="text-[0.66rem] faint num">{day}</span>
      <span className="h-px flex-1" style={{ background: "var(--color-border-soft)" }} />
    </div>
  );
}
