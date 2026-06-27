import { useState } from "react";
import {
  PanelRight,
  Layers,
  Cpu,
  Terminal as TerminalIcon,
  Bot,
  CheckCircle2,
  Link as LinkIcon,
  Inbox,
  Send,
  Wrench,
  Sparkles,
  Camera,
  Waypoints,
} from "lucide-react";
import type { AgentDefinition, Session, WorkspaceAgent } from "../ipc";

// ---------------------------------------------------------------------------
// Right-side Context drawer — shows the ACTIVE agent's configuration. The
// section set adapts to the agent's `type` (cli / chat / orchestrator).
//
// HONESTY NOTE — this drawer renders ONLY data that genuinely exists in
// `AgentDefinition` (+ the live `Session` when available). Sections without a
// real data source yet (Tools/Skills → M5, inter-agent message log → M3,
// snapshots/Session cwd/branch/pid → M4 / not modelled) render an honest
// deferred placeholder, NOT fabricated chips/rows. Each deferred block names
// its milestone, mirroring how `ChatView.tsx` documents its deferred parts.
// ---------------------------------------------------------------------------

interface ContextDrawerProps {
  /** The active agent's full definition (carries name/color/type/config). */
  def: AgentDefinition;
  /** Live status of the active instance. */
  status: WorkspaceAgent["status"];
  /** The active session, when one has been spawned (for context meter). */
  session?: Session | null;
}

// Human-readable loop label for a cli agent, by `cliKind`.
function cliLoopLabel(cliKind: AgentDefinition["cliKind"]): string {
  switch (cliKind) {
    case "claude-code":
      return "Claude Code loop";
    case "codex":
      return "Codex loop";
    case "custom":
      return "Custom loop";
    default:
      return "CLI loop";
  }
}

function allowedSendersLabel(v: AgentDefinition["allowedSenders"]): string {
  switch (v) {
    case "all":
      return "ทุก agent";
    case "selected":
      return "เฉพาะที่เลือก";
    case "none":
      return "ปิดรับ";
    default:
      return "—";
  }
}

// Small section header, matching the prototype's uppercase label rows.
function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <div className="text-[10px] font-bold tracking-wider text-[#a1a1a6] uppercase mb-1.5 px-0.5 flex items-center justify-between">
      {children}
    </div>
  );
}

// Honest empty/deferred state — a muted line naming the milestone.
function DeferredNote({ children }: { children: React.ReactNode }) {
  return (
    <div className="rounded-xl ring-hair bg-white px-2.5 py-2 text-[11.5px] text-[#a1a1a6]">
      {children}
    </div>
  );
}

export function ContextDrawer({ def, status, session }: ContextDrawerProps) {
  // Simplest collapse affordance: internal open/closed state. The header
  // `panel-right` button toggles it; collapsed → a thin strip to reopen.
  // Scope: this state is workspace-scoped — it persists across tab switches
  // (drawer visibility is a user preference, not per-agent) and resets only
  // when the pane remounts (WorkspacePane is keyed by workspaceId).
  const [open, setOpen] = useState(true);

  if (!open) {
    return (
      <aside className="w-9 vibrancy border-l border-black/[0.06] flex flex-col items-center shrink-0">
        <button
          title="แสดง Context"
          onClick={() => setOpen(true)}
          className="w-7 h-7 mt-2.5 grid place-items-center rounded-md hover:bg-black/[0.05] text-[#6e6e73]"
        >
          <PanelRight className="w-[15px] h-[15px]" />
        </button>
      </aside>
    );
  }

  const isOwn = def.harnessMode === "own";

  return (
    <aside className="w-[306px] vibrancy border-l border-black/[0.06] flex flex-col shrink-0">
      {/* Header */}
      <div className="h-12 flex items-center justify-between px-4 border-b border-black/[0.06] shrink-0">
        <span className="text-[12px] font-semibold text-[#6e6e73] tracking-tight">Context</span>
        <button
          title="ซ่อน Context"
          onClick={() => setOpen(false)}
          className="w-7 h-7 grid place-items-center rounded-md hover:bg-black/[0.05] text-[#6e6e73]"
        >
          <PanelRight className="w-[15px] h-[15px]" />
        </button>
      </div>

      <div className="flex-1 overflow-y-auto scroll-thin p-3 space-y-4">
        {def.type === "orchestrator" ? (
          // Orchestrator (Fusion) config is M4 — no honest source yet.
          <div>
            <SectionLabel>Fusion</SectionLabel>
            <DeferredNote>การตั้งค่า Fusion จะมาใน M4</DeferredNote>
          </div>
        ) : (
          <>
            {/* Harness — REAL (harnessMode + shareBlackboard) */}
            <div>
              <SectionLabel>Harness</SectionLabel>
              <div className="rounded-xl ring-hair bg-white p-2.5 space-y-2">
                <div className="flex items-center justify-between">
                  <span className="text-[12px] font-medium flex items-center gap-1.5">
                    {isOwn ? (
                      <>
                        {def.type === "cli" ? (
                          <TerminalIcon className="w-3.5 h-3.5 text-[#ff7a45]" />
                        ) : (
                          <Cpu className="w-3.5 h-3.5 text-[#0a84ff]" />
                        )}
                        {def.type === "cli" ? cliLoopLabel(def.cliKind) : "Own loop"}
                      </>
                    ) : (
                      <>
                        <Layers className="w-3.5 h-3.5 text-[#0fa3a3]" />
                        Central harness
                      </>
                    )}
                  </span>
                  {isOwn ? (
                    <span className="text-[10px] text-white bg-[#ff7a45] px-1.5 py-0.5 rounded-full font-semibold">
                      own
                    </span>
                  ) : (
                    <span className="text-[10px] text-white bg-[#0fa3a3] px-1.5 py-0.5 rounded-full font-semibold">
                      shared
                    </span>
                  )}
                </div>
                <div className="flex items-center justify-between text-[11.5px] text-[#6e6e73]">
                  <span className="flex items-center gap-1.5">
                    <Layers className="w-3.5 h-3.5" />
                    Central blackboard
                  </span>
                  {def.shareBlackboard ? (
                    <span className="flex items-center gap-1 text-[#30a14e]">
                      <LinkIcon className="w-3 h-3" />
                      shared
                    </span>
                  ) : (
                    <span>off</span>
                  )}
                </div>
              </div>
            </div>

            {/* Model · API — REAL (model + providerId), chat-focused */}
            {def.type === "chat" && (
              <div>
                <SectionLabel>Model · API</SectionLabel>
                {def.model ? (
                  <div className="rounded-xl ring-hair bg-white px-2.5 py-2 flex items-center gap-2">
                    <div className="w-6 h-6 rounded-md bg-[#10a37f] grid place-items-center text-white shrink-0">
                      <Bot className="w-3.5 h-3.5" />
                    </div>
                    <div className="leading-tight flex-1 min-w-0">
                      <div className="text-[12px] font-medium truncate">
                        {def.model}
                        {def.role ? ` · ${def.role}` : ""}
                      </div>
                      <div className="text-[10.5px] text-[#86868b] truncate">
                        {def.providerId ? `provider · ${def.providerId}` : "—"}
                      </div>
                    </div>
                    {/* Static "configured" affordance — only because `model` is set. */}
                    <CheckCircle2 className="w-4 h-4 text-[#30a14e] shrink-0" />
                  </div>
                ) : (
                  <DeferredNote>ยังไม่ได้ตั้งค่าโมเดล</DeferredNote>
                )}
              </div>
            )}

            {/* Tools — DEFERRED to M5 (no tool join tables yet) */}
            <div>
              <SectionLabel>
                {def.type === "cli" ? "Tools · permissions" : "Tools · plugins"}
              </SectionLabel>
              <DeferredNote>
                <span className="flex items-center gap-1.5">
                  <Wrench className="w-3.5 h-3.5" />
                  ยังไม่ได้เชื่อมต่อ — จะมาใน M5
                </span>
              </DeferredNote>
            </div>

            {/* Skills — DEFERRED to M5 (no skill join tables yet) */}
            <div>
              <SectionLabel>Skills</SectionLabel>
              <DeferredNote>
                <span className="flex items-center gap-1.5">
                  <Sparkles className="w-3.5 h-3.5" />
                  ยังไม่ได้ตั้งค่า — จะมาใน M5
                </span>
              </DeferredNote>
            </div>

            {/* Memory · snapshots (cli) — DEFERRED to M4 (snapshot manager).
                Kept as an honest deferred note so the cli drawer's section set
                matches the prototype; the live context meter below lights up
                only when real Session token counts exist. */}
            {def.type === "cli" && (
              <div>
                <SectionLabel>Memory · snapshots</SectionLabel>
                <DeferredNote>
                  <span className="flex items-center gap-1.5">
                    <Camera className="w-3.5 h-3.5" />
                    snapshot ของ session จะมาใน M4
                  </span>
                </DeferredNote>
              </div>
            )}

            {/* Messages — message LOG is M3 (deferred). The routing POLICY below
                is REAL config from AgentDefinition (allowedSenders / autoSubmitInjected). */}
            <div>
              <SectionLabel>Messages</SectionLabel>
              <div className="rounded-xl ring-hair bg-white p-2.5 space-y-1.5 text-[11.5px]">
                <div className="flex items-center justify-between">
                  <span className="text-[#6e6e73] flex items-center gap-1.5">
                    <Inbox className="w-3.5 h-3.5" />
                    รับข้อความจาก
                  </span>
                  <span className="font-medium">{allowedSendersLabel(def.allowedSenders)}</span>
                </div>
                <div className="flex items-center justify-between">
                  <span className="text-[#6e6e73] flex items-center gap-1.5">
                    <Send className="w-3.5 h-3.5" />
                    ส่งอัตโนมัติเมื่อ inject
                  </span>
                  <span className="font-medium">{def.autoSubmitInjected ? "เปิด" : "ปิด"}</span>
                </div>
                <div className="text-[10.5px] text-[#a1a1a6] pt-0.5">
                  บันทึกข้อความระหว่าง agent จะมาใน M3
                </div>
              </div>
            </div>
          </>
        )}

        {/* Context meter — REAL only if the session reports token counts.
            (Snapshot manager / Memory section is M4; we do NOT fabricate a meter.) */}
        {session && session.contextTokens != null && session.contextLimit != null && session.contextLimit > 0 && (
          <div>
            <SectionLabel>Context</SectionLabel>
            <div className="rounded-xl ring-hair bg-white p-2.5 space-y-1.5 text-[11.5px]">
              <div className="flex items-center justify-between text-[#6e6e73]">
                <span>tokens</span>
                <span className="font-mono">
                  {session.contextTokens.toLocaleString()} / {session.contextLimit.toLocaleString()}
                </span>
              </div>
              <div className="h-1.5 rounded-full bg-black/[0.06] overflow-hidden">
                <div
                  className="h-full bg-[#0a84ff]"
                  style={{
                    width: `${Math.min(100, Math.round((session.contextTokens / session.contextLimit) * 100))}%`,
                  }}
                />
              </div>
            </div>
          </div>
        )}
      </div>

      {/* Footer — agent identity (real: name/color/status). */}
      <div className="border-t border-black/[0.06] px-3 py-2 shrink-0 flex items-center gap-2">
        <div
          className="w-6 h-6 rounded-[7px] text-white grid place-items-center text-[11px] font-bold ring-hair shrink-0"
          style={{ backgroundColor: def.color ?? "#6e6e73" }}
        >
          {def.type === "orchestrator" ? (
            <Waypoints className="w-[14px] h-[14px]" />
          ) : (
            (def.name[0]?.toUpperCase() ?? "A")
          )}
        </div>
        <div className="leading-tight flex-1 min-w-0">
          <div className="text-[12px] font-semibold truncate">{def.name}</div>
          <div className="text-[10.5px] text-[#86868b] truncate">{status}</div>
        </div>
      </div>
    </aside>
  );
}
