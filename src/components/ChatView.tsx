import { useEffect, useRef, useState } from "react";
import { ArrowUp, CornerDownLeft, CornerUpRight } from "lucide-react";
import { ipc } from "../ipc";
import { useSessionOutput, useMessageInjected } from "../ipc";
import { ToolCallCard } from "./ToolCallCard";
import { SkillRunningCard } from "./SkillRunningCard";
import { RoutingPicker, type RoutingTarget } from "./RoutingPicker";

// ---------------------------------------------------------------------------
// IMPORTANT — backend capability note
//
// The chat backend (M2.4) streams PLAIN TEXT only: a user turn goes out via
// `message.send`, and the assistant reply arrives as `session:output` chunks.
// So in practice only `text` parts are ever produced here today.
//
// `tool` and `skill` parts have NO live data source yet — structured tool/skill
// event streaming (providers' tool-use API + the tool/skill join tables) is a
// later milestone (M5). The <ToolCallCard /> and <SkillRunningCard /> components
// are wired into the render switch below so the capability is COMPLETE and ready
// the moment that event stream exists; we intentionally do NOT fabricate fake
// tool/skill parts in this production path.
// ---------------------------------------------------------------------------

interface ChatViewProps {
  sessionId: string;
  /** Own instance id — the SELF routing target + the inbox/outbox key. */
  instanceId: string;
  /** All routable agents in the workspace (includes self). */
  roster: RoutingTarget[];
  /** Agent display name — drives the assistant avatar letter. */
  agentName: string;
  /** Agent accent color — the assistant avatar fill. */
  agentColor: string;
}

// Message view-model (local to the chat UI).
type ChatPart =
  | { kind: "text"; text: string }
  | { kind: "tool"; name: string; status: "running" | "done" | "error"; detail?: string }
  | { kind: "skill"; name: string; status: "running" | "done" }
  // Inbox: an injection THIS agent received (origin-tagged incoming bubble).
  | { kind: "inbox"; fromName: string; tint: string; autoSubmitted: boolean; text: string }
  // Outbox: a routed send confirmation (this agent → another agent).
  | { kind: "outbox"; toName: string; tint: string; status: "queued" | "delivered"; text: string };

interface ChatMsg {
  id: string;
  // "event" rows carry inbox/outbox parts (inter-agent routing chrome); they
  // render full-width with no avatar, distinct from user/assistant turns.
  role: "user" | "assistant" | "event";
  parts: ChatPart[];
}

// View-model message id. A fresh UUID per message — generated BEFORE the
// `setMessages` updater runs so the updater stays pure (React 19 StrictMode
// double-invokes updaters; a counter mutated inside would skip values).
const makeId = (): string => crypto.randomUUID();

export function ChatView({
  sessionId,
  instanceId,
  roster,
  agentName,
  agentColor,
}: ChatViewProps) {
  const [messages, setMessages] = useState<ChatMsg[]>([]);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  // Routing target for the NEXT send. Default = self (own session).
  const [targetId, setTargetId] = useState(instanceId);

  const avatarLetter = agentName[0]?.toUpperCase() ?? "A";

  // Guard setState against a send resolving after unmount.
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  // Auto-scroll to the newest content whenever the message list grows/changes.
  const scrollRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages]);

  // Append a streamed chunk to the LAST assistant message's trailing text part.
  // If the last message is not an open assistant message (e.g. unsolicited
  // output before any send), start a fresh assistant message.
  function appendChunk(chunk: string) {
    // Pre-generate the id for the rare "no open assistant" branch so the
    // updater body has no side effects (purity under StrictMode).
    const freshId = makeId();
    setMessages((prev) => {
      const last = prev[prev.length - 1];
      if (last && last.role === "assistant") {
        const parts = last.parts.slice();
        const tail = parts[parts.length - 1];
        if (tail && tail.kind === "text") {
          parts[parts.length - 1] = { kind: "text", text: tail.text + chunk };
        } else {
          parts.push({ kind: "text", text: chunk });
        }
        const updated: ChatMsg = { ...last, parts };
        return [...prev.slice(0, -1), updated];
      }
      // No open assistant message — create one.
      const fresh: ChatMsg = {
        id: freshId,
        role: "assistant",
        parts: [{ kind: "text", text: chunk }],
      };
      return [...prev, fresh];
    });
  }

  useSessionOutput(sessionId, (e) => appendChunk(e.chunk));

  // Inbox: render an origin-tagged INCOMING bubble whenever this agent RECEIVES
  // an injection. The agent's actual reply still streams via useSessionOutput;
  // this bubble is the inbound stimulus that precedes it. (Outbound events are
  // ignored here — the outbox confirmation is rendered synchronously on send.)
  useMessageInjected(instanceId, (e) => {
    if (e.toInstanceId !== instanceId) return;
    const sender = roster.find((t) => t.instanceId === e.fromInstanceId);
    const fromName = sender?.name ?? e.fromInstanceId;
    const tint = sender?.color ?? "#6e6e73";
    const id = makeId();
    setMessages((prev) => [
      ...prev,
      {
        id,
        role: "event",
        parts: [{ kind: "inbox", fromName, tint, autoSubmitted: e.autoSubmitted, text: e.text }],
      },
    ]);
  });

  async function handleSend() {
    const text = draft.trim();
    if (text.length === 0 || sending) return;

    // Routed send (→ another agent) takes a different path from a self send.
    const target = roster.find((t) => t.instanceId === targetId);
    if (target && target.instanceId !== instanceId) {
      await handleRoutedSend(target, text);
      return;
    }

    // ── Self send (own session) — unchanged behavior ──
    // Append the user turn AND a fresh empty assistant bubble (the streaming
    // target the chunks will flow into). Ids pre-generated so the updater is pure.
    const userId = makeId();
    const assistantId = makeId();
    setMessages((prev) => [
      ...prev,
      { id: userId, role: "user", parts: [{ kind: "text", text }] },
      { id: assistantId, role: "assistant", parts: [{ kind: "text", text: "" }] },
    ]);
    setDraft("");
    setSending(true);

    try {
      // Chat, not a terminal — no trailing newline.
      await ipc.message.send({ sessionId, text });
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      // Surface the failure on the open assistant bubble.
      if (mounted.current) {
        setMessages((prev) => {
          const last = prev[prev.length - 1];
          if (last && last.role === "assistant") {
            const updated: ChatMsg = {
              ...last,
              parts: [{ kind: "text", text: `⚠️ ส่งไม่สำเร็จ: ${msg}` }],
            };
            return [...prev.slice(0, -1), updated];
          }
          return prev;
        });
      }
    } finally {
      if (mounted.current) setSending(false);
    }
  }

  // Routed send: inject into a TARGET agent's live input. `inject` appends the
  // newline server-side, so we send the raw text. On success an OUTBOX bubble
  // confirms delivery (delivered vs queued); target resets to self afterward.
  async function handleRoutedSend(target: RoutingTarget, text: string) {
    setDraft("");
    setSending(true);
    try {
      const msg = await ipc.message.inject({
        fromInstanceId: instanceId,
        toInstanceId: target.instanceId,
        text,
      });
      if (mounted.current) {
        const id = makeId();
        setMessages((prev) => [
          ...prev,
          {
            id,
            role: "event",
            parts: [
              {
                kind: "outbox",
                toName: target.name,
                tint: target.color,
                status: msg.status,
                text,
              },
            ],
          },
        ]);
        setTargetId(instanceId);
      }
    } catch (err) {
      const detail = err instanceof Error ? err.message : String(err);
      if (mounted.current) {
        const id = makeId();
        setMessages((prev) => [
          ...prev,
          {
            id,
            role: "event",
            parts: [{ kind: "text", text: `⚠️ ส่งให้ ${target.name} ไม่สำเร็จ: ${detail}` }],
          },
        ]);
      }
    } finally {
      if (mounted.current) setSending(false);
    }
  }

  return (
    <main className="flex-1 flex flex-col min-w-0 bg-white">
      {/* Message list */}
      <div ref={scrollRef} className="flex-1 overflow-y-auto scroll-thin px-8 py-6">
        <div className="max-w-[720px] mx-auto space-y-5">
          {messages.length === 0 ? (
            <div className="text-center text-[13px] text-[#a1a1a6] pt-10">
              เริ่มแชทกับ agent ได้เลย
            </div>
          ) : (
            messages.map((msg, i) => {
              const isLast = i === messages.length - 1;
              return (
                <MessageRow
                  key={msg.id}
                  msg={msg}
                  isLast={isLast}
                  avatarLetter={avatarLetter}
                  avatarColor={agentColor}
                />
              );
            })
          )}
        </div>
      </div>

      {/* Composer */}
      <div className="border-t border-black/[0.07] px-8 py-3 bg-white shrink-0">
        <div className="max-w-[720px] mx-auto">
          {/* Route reply into… — default self, choose another agent to inject. */}
          <div className="mb-2 flex items-center gap-2">
            <RoutingPicker
              selfId={instanceId}
              roster={roster}
              value={targetId}
              onChange={setTargetId}
              disabled={sending}
            />
            {targetId !== instanceId && (
              <span className="text-[11px] text-[#a1a1a6]">ฉีดเข้า input ของ agent ปลายทาง · ส่งอัตโนมัติ</span>
            )}
          </div>
          <div className="rounded-2xl ring-1 ring-black/[0.1] bg-[#f7f7f8] focus-within:ring-[#0a84ff]/50 px-3 pt-2.5 pb-2">
            <div className="flex items-end gap-2">
              <textarea
                rows={1}
                value={draft}
                disabled={sending}
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !e.shiftKey) {
                    e.preventDefault();
                    void handleSend();
                  }
                }}
                placeholder="พิมพ์ข้อความ…  (Enter ส่ง · Shift+Enter ขึ้นบรรทัดใหม่)"
                className="flex-1 bg-transparent outline-none resize-none text-[13.5px] leading-relaxed placeholder:text-[#a1a1a6] py-1 max-h-40 disabled:opacity-50"
              />
              <button
                onClick={() => void handleSend()}
                disabled={sending || draft.trim().length === 0}
                className="w-8 h-8 rounded-full bg-[#0a84ff] text-white grid place-items-center shrink-0 hover:brightness-105 disabled:opacity-40 disabled:hover:brightness-100"
              >
                <ArrowUp className="w-4 h-4" />
              </button>
            </div>
          </div>
        </div>
      </div>
    </main>
  );
}

// ---------------------------------------------------------------------------
// One message row: avatar + the message's parts rendered in order.
// ---------------------------------------------------------------------------

interface MessageRowProps {
  msg: ChatMsg;
  isLast: boolean;
  avatarLetter: string;
  avatarColor: string;
}

function MessageRow({ msg, isLast, avatarLetter, avatarColor }: MessageRowProps) {
  // Event rows — inter-agent routing chrome (inbox / outbox). Full-width, no avatar.
  if (msg.role === "event") {
    return (
      <div className="space-y-2.5">
        {msg.parts.map((part, i) => {
          const key = `${msg.id}-${i}`;
          if (part.kind === "inbox") {
            return (
              <div
                key={key}
                className="rounded-2xl rounded-tl-md px-3.5 py-2.5 bg-[#fafafa]"
                style={{ boxShadow: `inset 0 0 0 1px ${part.tint}59` }}
              >
                <div className="flex items-center gap-1.5 text-[10.5px] font-medium mb-1" style={{ color: part.tint }}>
                  <CornerDownLeft className="w-3 h-3" />
                  ฉีดเข้ามาจาก {part.fromName}
                  {part.autoSubmitted ? " · ส่งอัตโนมัติ" : ""}
                </div>
                <div className="text-[13.5px] leading-relaxed whitespace-pre-wrap break-words text-[#1d1d1f]">
                  {part.text}
                </div>
              </div>
            );
          }
          if (part.kind === "outbox") {
            return (
              <div key={key} className="flex justify-center">
                <div className="max-w-[90%] rounded-full bg-[#f2f2f4] px-3 py-1.5 text-[11.5px] text-[#6e6e73] flex items-center gap-1.5">
                  <CornerUpRight className="w-3 h-3 shrink-0" style={{ color: part.tint }} />
                  <span className="font-medium text-[#1d1d1f]">→ ส่งให้ {part.toName}</span>
                  {part.status === "delivered" ? (
                    <span>· ส่งอัตโนมัติ</span>
                  ) : (
                    <span className="text-[#ff9f0a]">· agent ปลายทางยังไม่ทำงาน — คิวไว้แล้ว</span>
                  )}
                </div>
              </div>
            );
          }
          // A routed-send failure surfaces as a plain text notice.
          if (part.kind === "text") {
            return (
              <div key={key} className="flex justify-center">
                <div className="max-w-[90%] rounded-full bg-[#fff1f0] px-3 py-1.5 text-[11.5px] text-[#ff3b30]">
                  {part.text}
                </div>
              </div>
            );
          }
          return null;
        })}
      </div>
    );
  }

  const isUser = msg.role === "user";

  if (isUser) {
    // User bubbles: right-aligned, accent fill.
    return (
      <div className="flex justify-end">
        <div className="max-w-[82%]">
          {msg.parts.map((part, i) =>
            part.kind === "text" ? (
              <div
                key={`${msg.id}-${i}`}
                className="bg-[#0a84ff] text-white rounded-2xl rounded-tr-md px-3.5 py-2.5 text-[13.5px] leading-relaxed whitespace-pre-wrap break-words"
              >
                {part.text}
              </div>
            ) : null,
          )}
        </div>
      </div>
    );
  }

  // Assistant bubbles: left-aligned, light fill.
  // A trailing empty text part on the last message → typing indicator.
  return (
    <div className="flex gap-2.5">
      <div
        className="w-6 h-6 rounded-[7px] text-white grid place-items-center text-[11px] font-bold ring-hair shrink-0 mt-0.5"
        style={{ backgroundColor: avatarColor }}
      >
        {avatarLetter}
      </div>
      <div className="max-w-[82%] space-y-2.5">
        {msg.parts.map((part, i) => {
          const key = `${msg.id}-${i}`;
          switch (part.kind) {
            case "text": {
              const isTrailing = i === msg.parts.length - 1;
              if (part.text.length === 0) {
                // Only show the typing indicator for the live (last) message.
                return isLast && isTrailing ? <TypingDots key={key} /> : null;
              }
              return (
                <div
                  key={key}
                  className="bg-[#f2f2f4] rounded-2xl rounded-tl-md px-3.5 py-2.5 text-[13.5px] leading-relaxed whitespace-pre-wrap break-words"
                >
                  {part.text}
                </div>
              );
            }
            case "tool":
              return (
                <ToolCallCard
                  key={key}
                  name={part.name}
                  status={part.status}
                  detail={part.detail}
                />
              );
            case "skill":
              return <SkillRunningCard key={key} name={part.name} status={part.status} />;
            // inbox / outbox parts only ever appear on "event" rows (handled above).
            default:
              return null;
          }
        })}
      </div>
    </div>
  );
}

function TypingDots() {
  return (
    <div className="bg-[#f2f2f4] rounded-2xl rounded-tl-md px-3.5 py-3 inline-flex items-center gap-1">
      <span className="w-1.5 h-1.5 rounded-full bg-[#a1a1a6] animate-bounce [animation-delay:-0.3s]" />
      <span className="w-1.5 h-1.5 rounded-full bg-[#a1a1a6] animate-bounce [animation-delay:-0.15s]" />
      <span className="w-1.5 h-1.5 rounded-full bg-[#a1a1a6] animate-bounce" />
    </div>
  );
}
