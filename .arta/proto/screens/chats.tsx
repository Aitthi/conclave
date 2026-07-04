import {
  PanelRightClose, SlidersHorizontal, Hash, Users, Eye, Maximize2,
} from "lucide-react";
import AppShell from "../components/AppShell";
import { convos, agents, avClass, workspaceThread, type Message } from "../lib/data";

export const meta = { title: "Right rail · Chats" };

const ACTIVE = "workspace";

/* small avatar for a convo: channel = hash; agent-pair DM = two overlapping avatars
   (mirrors the Chat Hub sidebar's -space-x cluster); single member = one avatar. */
function MiniAv({ id, size = "av-md" }: { id: string; size?: string }) {
  const c = convos.find((x) => x.id === id)!;
  if (c.kind === "channel") return <span className={`av ${size} av-hash`}><Hash size={13} /></span>;
  if (c.members.length >= 2) {
    const [a, b] = c.members;
    return (
      <span className="inline-flex items-center -space-x-1.5">
        <span className={`av ${size} ${avClass[agents[a].color]}`} style={{ boxShadow: "0 0 0 1.5px var(--color-rail)" }}>{agents[a].initials}</span>
        <span className={`av ${size} ${avClass[agents[b].color]}`} style={{ boxShadow: "0 0 0 1.5px var(--color-rail)" }}>{agents[b].initials}</span>
      </span>
    );
  }
  const m = agents[c.members[0]];
  return <span className={`av ${size} ${avClass[m.color]}`}>{m.initials}</span>;
}

function group(messages: Message[]) {
  const out: { from: string; kind?: string; items: Message[] }[] = [];
  for (const m of messages) {
    const last = out[out.length - 1];
    if (m.kind === "system") { out.push({ from: "system", kind: "system", items: [m] }); continue; }
    if (last && last.from === m.from && last.kind !== "system") last.items.push(m);
    else out.push({ from: m.from, items: [m] });
  }
  return out;
}

export default function Chats() {
  const active = convos.find((c) => c.id === ACTIVE)!;
  const groups = group(workspaceThread);
  const liveMembers = active.members.filter((m) => agents[m].status === "live").length;

  return (
    <AppShell>
      {/* compact top bar — title + controls (no Context tab) */}
      <div className="flex items-center gap-2 px-3.5 h-11 shrink-0 border-b" style={{ borderColor: "var(--color-border-soft)" }}>
        <span className="heading text-[0.86rem] font-semibold">Chats</span>
        <span className="text-[0.7rem] faint">{convos.reduce((n, c) => n + c.unread, 0)} unread</span>
        <div className="ml-auto flex items-center gap-0.5 faint">
          <button className="w-7 h-7 grid place-items-center rounded-md hover:text-[var(--color-text)]" title="Filter"><SlidersHorizontal size={15} /></button>
          <button className="w-7 h-7 grid place-items-center rounded-md hover:text-[var(--color-text)]" title="Open in Chat Hub"><Maximize2 size={14} /></button>
          <button className="w-7 h-7 grid place-items-center rounded-md hover:text-[var(--color-text)]" title="Collapse"><PanelRightClose size={16} /></button>
        </div>
      </div>

      {/* room switcher — horizontal, tap to switch which chat you observe */}
      <div className="flex items-center gap-1.5 overflow-x-auto scroll-thin px-3 py-2 shrink-0">
        {convos.map((c) => {
          const on = c.id === ACTIVE;
          return (
            <button key={c.id}
              className="relative inline-flex items-center gap-1.5 shrink-0 pl-1 pr-2.5 py-1 rounded-full border transition-colors"
              style={on
                ? { borderColor: "color-mix(in srgb, var(--color-accent) 40%, transparent)", background: "color-mix(in srgb, var(--color-accent) 12%, transparent)" }
                : { borderColor: "var(--color-border)", background: "var(--color-app)" }}>
              <MiniAv id={c.id} size="av-sm" />
              <span className="text-[0.76rem] font-medium clamp-1 max-w-[92px]"
                style={{ color: on ? "var(--color-heading)" : "var(--color-dim)" }}>{c.title}</span>
              {c.unread > 0 && <span className="badge" style={{ minWidth: 15, height: 15, fontSize: "0.6rem" }}>{c.unread}</span>}
              {c.live && c.unread === 0 && <span className="dot dot-live w-1.5 h-1.5" />}
            </button>
          );
        })}
      </div>

      {/* thin active-room meta line (no big header band) */}
      <div className="flex items-center gap-1.5 px-4 pb-1.5 pt-0.5 shrink-0 text-[0.72rem]">
        <Users size={11} className="faint" />
        <span className="dim"><span className="heading font-semibold">#{active.title}</span> · {active.members.length} members · <span style={{ color: "var(--color-live)" }}>{liveMembers} live now</span></span>
      </div>

      {/* message stream */}
      <div className="flex-1 min-h-0 overflow-y-auto scroll-thin px-3.5 pt-1 pb-3 flex flex-col gap-2.5 border-t" style={{ borderColor: "var(--color-border-soft)" }}>
        <div className="flex items-center gap-2 justify-center">
          <span className="h-px flex-1" style={{ background: "var(--color-border-soft)" }} />
          <span className="text-[0.66rem] faint num">Today</span>
          <span className="h-px flex-1" style={{ background: "var(--color-border-soft)" }} />
        </div>

        {groups.map((g, gi) => {
          if (g.kind === "system") {
            return (
              <div key={gi} className="flex justify-center">
                <span className="msg-system px-3 py-1 rounded-full text-center max-w-[92%]">{g.items[0].text}</span>
              </div>
            );
          }
          const a = agents[g.from];
          return (
            <div key={gi} className="flex gap-2.5">
              <span className={`av av-md ${avClass[a.color]} mt-0.5`}>{a.initials}</span>
              <div className="min-w-0 flex flex-col gap-1 items-start" style={{ maxWidth: "82%" }}>
                <div className="flex items-baseline gap-2">
                  <span className="text-[0.78rem] font-semibold" style={{ color: "var(--color-heading)" }}>{a.name}</span>
                  <span className="text-[0.64rem] faint">{a.role}</span>
                  <span className="text-[0.64rem] faint num">{g.items[0].at}</span>
                </div>
                {g.items.map((m) => {
                  const to = m.to ? agents[m.to] : null;
                  const queued = to?.status === "queued";
                  return (
                    <div key={m.id} className="flex flex-col gap-0.5 items-start self-stretch">
                      <div className="msg self-start" title={to ? `→ ${to.name}` : undefined}>{m.text}</div>
                      {/* recipient chip (ruling R10, amended R11) — BELOW the bubble;
                          the queued label (recipient hasn't picked up yet) shares this
                          row on the left, chip stays right-aligned. Visible without hover. */}
                      {to && (
                        <div className="self-stretch flex items-center gap-1">
                          {/* queued label styling mirrors the app convention (lowercase,
                              warning color, no dot) — it predates R10/R11, which only
                              relocated it into this shared row. */}
                          {queued && (
                            <span className="text-[0.56rem]" style={{ color: "var(--color-queued)" }}>queued</span>
                          )}
                          <span className="ml-auto inline-flex items-center gap-1 text-[0.62rem] faint" title={`→ ${to.name}`}>
                            <span className={`av av-xs ${avClass[to.color]}`}>{to.initials}</span>
                            {to.name}
                          </span>
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          );
        })}
      </div>

      {/* read-only footer — this rail observes, it doesn't send */}
      <div className="shrink-0 px-4 py-2.5 border-t flex items-center gap-2 text-[0.72rem]"
        style={{ borderColor: "var(--color-border)", background: "color-mix(in srgb, var(--color-app) 60%, transparent)" }}>
        <Eye size={13} className="faint" />
        <span className="dim">Read-only live view — agents reply from their own terminal.</span>
      </div>
    </AppShell>
  );
}
