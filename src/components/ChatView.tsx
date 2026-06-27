import { useEffect, useRef, useState } from "react";
import { ArrowUp } from "lucide-react";
import { ipc } from "../ipc";
import { useSessionOutput } from "../ipc";
import { ToolCallCard } from "./ToolCallCard";
import { SkillRunningCard } from "./SkillRunningCard";

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
  /** Agent display name — drives the assistant avatar letter. */
  agentName: string;
  /** Agent accent color — the assistant avatar fill. */
  agentColor: string;
}

// Message view-model (local to the chat UI).
type ChatPart =
  | { kind: "text"; text: string }
  | { kind: "tool"; name: string; status: "running" | "done" | "error"; detail?: string }
  | { kind: "skill"; name: string; status: "running" | "done" };

interface ChatMsg {
  id: string;
  role: "user" | "assistant";
  parts: ChatPart[];
}

// View-model message id. A fresh UUID per message — generated BEFORE the
// `setMessages` updater runs so the updater stays pure (React 19 StrictMode
// double-invokes updaters; a counter mutated inside would skip values).
const makeId = (): string => crypto.randomUUID();

export function ChatView({ sessionId, agentName, agentColor }: ChatViewProps) {
  const [messages, setMessages] = useState<ChatMsg[]>([]);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);

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

  async function handleSend() {
    const text = draft.trim();
    if (text.length === 0 || sending) return;

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
