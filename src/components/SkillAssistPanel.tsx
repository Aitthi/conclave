import { useEffect, useRef, useState } from "react";
import { ArrowUp, Play, Square } from "lucide-react";
import { Terminal as XTerm } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
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

// Matches Terminal.tsx's dark palette so the assist panel's CLI output looks
// identical to the app's main terminal pane, not a different shade of dark.
const XTERM_THEME = {
  background: "#1e1e22",
  foreground: "#d4d4d8",
  cursor: "#d4d4d8",
  selectionBackground: "rgba(10, 132, 255, 0.3)",
};

/**
 * "Ask agent to help" panel for the skill editor: pick one of the user's own
 * CLI `AgentDefinition`s, start a real agent-assist session against the
 * skill's scratch file (see docs/specs/2026-07-02-skill-editor-agent-assist-design.md),
 * watch its raw terminal output (a real xterm.js instance — the streamed
 * chunks are PTY bytes, including ANSI codes, so a plain-text div can't
 * render them), chat with it, and sync its edits back into the editor —
 * either on the session's next idle transition or via the manual "Sync now"
 * button. Design approved interactively via Arta; see
 * .arta/prototype/screens/skill-editor-active.html for the reference mock.
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
  const [draftText, setDraftText] = useState("");
  const [sending, setSending] = useState(false);
  const [running, setRunning] = useState(true);

  const termContainerRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<XTerm | null>(null);

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

  // A fresh xterm instance per session (keyed on workspaceAgentId, not just
  // "draft is non-null") — a new "Start" always begins with an empty
  // scrollback, matching a fresh terminal.
  useEffect(() => {
    if (!draft || !termContainerRef.current) return;
    const term = new XTerm({
      convertEol: true,
      disableStdin: true,
      fontSize: 11.5,
      lineHeight: 1.5,
      fontFamily: '"SF Mono", "JetBrains Mono", ui-monospace, Menlo, monospace',
      theme: XTERM_THEME,
      scrollback: 5000,
    });
    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.open(termContainerRef.current);
    try {
      fitAddon.fit();
    } catch {
      // Container not laid out yet on the very first paint — the
      // ResizeObserver below re-fits once it is.
    }
    termRef.current = term;
    setRunning(true);

    const resizeObserver = new ResizeObserver(() => {
      try {
        fitAddon.fit();
      } catch {
        // Ignore transient fits during unmount/resize races.
      }
    });
    resizeObserver.observe(termContainerRef.current);

    return () => {
      resizeObserver.disconnect();
      term.dispose();
      termRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [draft?.workspaceAgentId]);

  useSessionOutput(draft?.sessionId ?? "", (e) => {
    if (!draft) return;
    termRef.current?.write(e.chunk);
  });

  useSessionStatus(draft?.sessionId ?? "", (e) => {
    if (!draft) return;
    setRunning(e.status !== "idle");
    if (e.status === "idle") void handleSync();
  });

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

  async function handleSend() {
    if (!draft || draftText.trim().length === 0) return;
    setSending(true);
    try {
      // `paste: true`: a draft longer than the PTY's ~1 KB read chunk must
      // travel as ONE bracketed paste (see StdinBar) — this session is always
      // a PTY CLI (skill_draft.rs accepts only claude-code/codex).
      await ipc.message.send({
        sessionId: draft.sessionId,
        text: draftText,
        paste: true,
      });
      setDraftText("");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSending(false);
    }
  }

  return (
    <div className="w-[360px] shrink-0 border-l border-overlay/[0.07] flex flex-col bg-sidebar">
      <div className="h-11 flex items-center justify-between px-3 border-b border-overlay/[0.06] shrink-0">
        <div className="flex items-center gap-2">
          <span className="text-[12.5px] font-semibold tracking-tight">Ask agent to help</span>
          {draft && (
            <span
              className={`flex items-center gap-1 text-[10.5px] font-medium ${
                running ? "text-status-running" : "text-status-idle"
              }`}
            >
              <span
                className={`w-1.5 h-1.5 rounded-full ${running ? "bg-status-running" : "bg-status-idle"}`}
              />
              {running ? "running" : "idle"}
            </span>
          )}
        </div>
        {draft && (
          <button
            onClick={() => void handleStop()}
            title="Stop agent"
            aria-label="Stop agent"
            className="w-7 h-7 grid place-items-center rounded-md text-danger hover:bg-danger/[0.12]"
          >
            <Square className="w-3.5 h-3.5" />
          </button>
        )}
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
          <div ref={termContainerRef} className="flex-1 min-h-0 overflow-hidden" />
          <div className="border-t border-overlay/[0.06] p-2 shrink-0">
            <div className="rounded-2xl ring-1 ring-overlay/[0.10] bg-fill-softer px-3 pt-2 pb-1.5 focus-within:ring-accent/50">
              <div className="flex items-end gap-1.5">
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
                  rows={1}
                  placeholder="Message the agent…"
                  className="flex-1 bg-transparent outline-none resize-none text-[12.5px] leading-relaxed placeholder:text-text-tertiary py-1 max-h-32 disabled:opacity-50"
                />
                <button
                  onClick={() => void handleSend()}
                  disabled={sending || draftText.trim().length === 0}
                  className="w-7 h-7 rounded-full bg-accent text-white grid place-items-center shrink-0 hover:brightness-105 disabled:opacity-40"
                >
                  <ArrowUp className="w-3.5 h-3.5" />
                </button>
              </div>
            </div>
            {error && <p className="text-[11.5px] text-danger mt-1.5">{error}</p>}
          </div>
        </>
      )}
    </div>
  );
}
