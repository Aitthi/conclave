import { useEffect, useState } from "react";
import { Play, Square, RefreshCw } from "lucide-react";
import { ipc } from "../ipc";
import { useSessionOutput, useSessionStatus } from "../ipc";
import type { AgentDefinition } from "../ipc";

export interface DraftSession {
  workspaceAgentId: string;
  sessionId: string;
}

export interface SkillAssistPanelProps {
  name: string;
  description: string;
  content: string;
  /** Non-null while a session is active — owned by the parent editor so it
   *  can lock its fields for the duration. */
  draft: DraftSession | null;
  onStarted: (draft: DraftSession) => void;
  onSynced: (v: { name: string; description?: string; content: string }) => void;
  onStopped: () => void;
}

/**
 * "Ask agent to help" panel for the skill editor: pick one of the user's own
 * CLI `AgentDefinition`s, start a real agent-assist session against the
 * skill's scratch file (see docs/specs/2026-07-02-skill-editor-agent-assist-design.md),
 * chat with it, and sync its edits back into the editor — either on the
 * session's next idle transition or via the manual "Sync now" button.
 */
export function SkillAssistPanel({
  name,
  description,
  content,
  draft,
  onStarted,
  onSynced,
  onStopped,
}: SkillAssistPanelProps) {
  const [agents, setAgents] = useState<AgentDefinition[]>([]);
  const [agentDefId, setAgentDefId] = useState<string>("");
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lines, setLines] = useState<string[]>([]);
  const [draftText, setDraftText] = useState("");
  const [sending, setSending] = useState(false);

  useEffect(() => {
    ipc.agentDef
      .list()
      .then((defs) => {
        const cliDefs = defs.filter(
          (d) => d.type === "cli" && (d.cliKind === "claude-code" || d.cliKind === "codex"),
        );
        setAgents(cliDefs);
        setAgentDefId((prev) => prev || (cliDefs[0]?.id ?? ""));
      })
      .catch(() => setAgents([]));
  }, []);

  async function handleStart() {
    if (!agentDefId) return;
    setStarting(true);
    setError(null);
    try {
      const res = await ipc.skill.startDraftSession({
        name: name.trim() || "Untitled skill",
        description: description.trim() || undefined,
        content,
        agentDefId,
      });
      setLines([]);
      onStarted(res);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setStarting(false);
    }
  }

  async function handleStop() {
    if (!draft) return;
    await ipc.skill.stopDraftSession({ workspaceAgentId: draft.workspaceAgentId }).catch(() => {});
    onStopped();
  }

  async function handleSync() {
    if (!draft) return;
    try {
      const v = await ipc.skill.syncDraft({ workspaceAgentId: draft.workspaceAgentId });
      onSynced(v);
    } catch {
      // Leave the editor's current fields untouched — a failed sync must not
      // destroy the last successfully synced state (see design spec).
    }
  }

  useSessionOutput(draft?.sessionId ?? "", (e) => {
    if (!draft) return;
    setLines((prev) => [...prev, e.chunk]);
  });

  useSessionStatus(draft?.sessionId ?? "", (e) => {
    if (!draft) return;
    if (e.status === "idle") void handleSync();
  });

  async function handleSend() {
    if (!draft || draftText.trim().length === 0) return;
    setSending(true);
    try {
      await ipc.message.send({ sessionId: draft.sessionId, text: draftText });
      setDraftText("");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSending(false);
    }
  }

  return (
    <div className="w-[360px] shrink-0 border-l border-overlay/[0.07] flex flex-col bg-sidebar">
      <div className="h-11 flex items-center px-3 border-b border-overlay/[0.06] shrink-0">
        <span className="text-[12.5px] font-semibold tracking-tight">Ask agent to help</span>
      </div>

      {!draft ? (
        <div className="p-3 space-y-2">
          <select
            value={agentDefId}
            onChange={(e) => setAgentDefId(e.target.value)}
            disabled={agents.length === 0}
            className="w-full rounded-lg ring-1 ring-overlay/[0.10] bg-fill-softer px-2.5 h-8 text-[12.5px] outline-none"
          >
            {agents.length === 0 ? (
              <option>No CLI agents configured</option>
            ) : (
              agents.map((a) => (
                <option key={a.id} value={a.id}>
                  {a.name}
                </option>
              ))
            )}
          </select>
          <button
            onClick={() => void handleStart()}
            disabled={starting || !agentDefId}
            className="w-full flex items-center justify-center gap-1.5 rounded-lg bg-accent text-white py-2 text-[12.5px] font-semibold disabled:opacity-50"
          >
            <Play className="w-3.5 h-3.5" />
            {starting ? "Starting…" : "Start"}
          </button>
          {error && <p className="text-[11.5px] text-danger">{error}</p>}
        </div>
      ) : (
        <>
          <div className="flex-1 overflow-y-auto scroll-thin px-3 py-2 text-[11.5px] font-mono whitespace-pre-wrap break-words">
            {lines.join("")}
          </div>
          <div className="border-t border-overlay/[0.06] p-2 space-y-1.5 shrink-0">
            <div className="flex items-center gap-1.5">
              <button
                onClick={() => void handleSync()}
                className="flex-1 flex items-center justify-center gap-1 text-[11px] font-medium text-text-secondary bg-surface ring-1 ring-overlay/[0.08] rounded-lg py-1.5 hover:bg-overlay/[0.02]"
              >
                <RefreshCw className="w-3 h-3" />
                Sync now
              </button>
              <button
                onClick={() => void handleStop()}
                className="flex-1 flex items-center justify-center gap-1 text-[11px] font-medium text-danger bg-danger/[0.06] rounded-lg py-1.5 hover:bg-danger/10"
              >
                <Square className="w-3 h-3" />
                Stop agent
              </button>
            </div>
            <textarea
              value={draftText}
              onChange={(e) => setDraftText(e.target.value)}
              disabled={sending}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  void handleSend();
                }
              }}
              rows={2}
              placeholder="Message the agent…"
              className="w-full rounded-lg ring-1 ring-overlay/[0.10] bg-fill-softer px-2.5 py-1.5 text-[12px] outline-none resize-none disabled:opacity-50"
            />
          </div>
        </>
      )}
    </div>
  );
}
