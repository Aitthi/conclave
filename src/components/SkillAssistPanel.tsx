import { useCallback, useEffect, useRef, useState } from "react";
import { ArrowUp, Play, RefreshCw, Square } from "lucide-react";
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

/** Gap between the pasted body and the standalone submit CR. Mirrors
 *  StdinBar.tsx: a CR arriving in the same burst as the text is read as part of
 *  the paste and inserted literally instead of submitting. */
const SUBMIT_CR_DELAY_MS = 40;

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
  const [syncing, setSyncing] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [syncError, setSyncError] = useState<string | null>(null);
  const [stopError, setStopError] = useState<string | null>(null);
  // Revealed only after a sync failure blocks Stop, so the user can end the
  // session knowingly rather than being wedged (challenge on this task).
  const [offerForceStop, setOfferForceStop] = useState(false);

  const termContainerRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<XTerm | null>(null);

  // Live view of the current draft for callbacks that outlive a render, and a
  // mounted flag so a start that resolves after unmount can still be cleaned up.
  const draftRef = useRef(draft);
  useEffect(() => {
    draftRef.current = draft;
  }, [draft]);
  // The fallback belongs to ONE session's failure; a new session starts clean
  // (R4 amendment: "Clear the fallback when sync succeeds or the active
  // session changes"). Clearing on success lives in runSync.
  useEffect(() => {
    setOfferForceStop(false);
    setSyncError(null);
    setStopError(null);
  }, [draft?.workspaceAgentId]);
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  /**
   * Write to the draft session's PTY stdin preserving EMISSION ORDER, the way
   * Terminal.tsx does. `ipc.message.send` resolves a session from the DB before
   * it reaches the PTY, so two sends fired back-to-back can land in either
   * order — and xterm answers a terminal-query burst as several `onData` calls
   * in ONE parse pass. Reordering those replies makes Claude Code conclude a
   * reply never came. One promise chain keeps them ordered; the chain never
   * rejects, so a single dead invoke cannot wedge the rest.
   */
  const stdinChainRef = useRef<Promise<void>>(Promise.resolve());
  const sendStdin = useCallback((sessionId: string, text: string) => {
    stdinChainRef.current = stdinChainRef.current
      .then(() => ipc.message.send({ sessionId, text }))
      .then(
        () => {},
        (e) => {
          // Surfaced, not swallowed: if keystrokes are not reaching the agent
          // the user is answering a prompt into the void and must be told.
          if (mountedRef.current) {
            setError(`Keystroke not delivered: ${e instanceof Error ? e.message : String(e)}`);
          }
        },
      );
  }, []);

  useEffect(() => {
    ipc.agentDef
      .list()
      .then((defs) => {
        const cliDefs = defs.filter(
          (d) =>
            d.type === "cli" &&
            (d.cliKind === "claude-code" ||
              d.cliKind === "codex" ||
              d.cliKind === "antigravity"),
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
    const sessionId = draft.sessionId;
    const term = new XTerm({
      convertEol: true,
      // R1: the CLI's own prompts (Claude Code's trust check, later permission
      // questions) are answered with arrows + Enter. With stdin disabled the
      // app made that normal step impossible and the session sat blocked
      // forever — the failure in the human's screenshot. Nothing is trusted on
      // the user's behalf; they answer the real prompt themselves.
      disableStdin: false,
      fontSize: 11.5,
      lineHeight: 1.5,
      fontFamily: '"SF Mono", "JetBrains Mono", ui-monospace, Menlo, monospace',
      theme: XTERM_THEME,
      scrollback: 5000,
    });
    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    const el = termContainerRef.current;
    term.open(el);

    // The pane's pixel size as xterm measures its own canvas — the container is
    // the wrong source, its width includes the sub-cell remainder the grid does
    // not cover. Matches Terminal.tsx.
    const pixelDims = (): { pixelWidth?: number; pixelHeight?: number } => {
      const canvas = term.dimensions?.css.canvas;
      if (!canvas) return {};
      return { pixelWidth: Math.round(canvas.width), pixelHeight: Math.round(canvas.height) };
    };

    // R2: fitting xterm alone left the PTY at its spawn 80x24 while this pane
    // is ~48 columns. Claude Code positions each word at an absolute column for
    // the width it believes it has, so everything past the pane's width piled
    // up at the right edge — the scrambled text in the screenshot. Both sides
    // must move together.
    let lastCols = 0;
    let lastRows = 0;
    const applyResize = () => {
      // A hidden or detached pane has no used layout; fitting it would push a
      // bogus tiny grid to a live PTY and permanently rewrap the transcript.
      if (el.getBoundingClientRect().width === 0) return;
      try {
        fitAddon.fit();
      } catch {
        return; // detached mid-teardown
      }
      const { cols, rows } = term;
      if (cols <= 0 || rows <= 0) return; // never send degenerate dimensions
      if (cols === lastCols && rows === lastRows) return;
      lastCols = cols;
      lastRows = rows;
      void ipc.session.resize({ sessionId, cols, rows, ...pixelDims() }).catch((e) => {
        if (mountedRef.current) {
          setError(`Terminal resize failed: ${e instanceof Error ? e.message : String(e)}`);
        }
      });
    };
    applyResize();
    termRef.current = term;
    setRunning(true);

    const resizeObserver = new ResizeObserver(applyResize);
    resizeObserver.observe(el);

    // Forward every keystroke and every terminal-query reply xterm generates,
    // raw and in order, through the single ordered channel.
    const dataSub = term.onData((data) => sendStdin(sessionId, data));

    return () => {
      resizeObserver.disconnect();
      dataSub.dispose();
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
    if (e.status === "idle") void runSync(false);
  });

  async function handleStart() {
    if (!agentDefId || starting) return;
    setStarting(true);
    setError(null);
    try {
      const res = await ipc.skill.startDraftSession({
        name: name.trim() || "Untitled skill",
        description: description.trim() || undefined,
        content,
        agentDefId,
      });
      // The editor can be closed while this await is in flight. Nobody would
      // ever learn this session's id, so it would leak as a hidden workspace
      // plus a live agent process — stop what we just started instead.
      if (!mountedRef.current) {
        void ipc.skill.stopDraftSession({ workspaceAgentId: res.workspaceAgentId }).catch(() => {});
        return;
      }
      onStarted(res);
    } catch (e) {
      if (mountedRef.current) setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (mountedRef.current) setStarting(false);
    }
  }

  /**
   * Pull the agent's current SKILL.md into the editor.
   *
   * `throwOnFailure` separates the two callers: the idle-transition and the
   * manual button treat a failure as "nothing new to show" (the file is
   * probably mid-write), while Stop must know, because stopping deletes the
   * scratch dir and an unread edit would be gone for good.
   */
  const runSync = useCallback(
    async (throwOnFailure: boolean) => {
      const active = draftRef.current;
      if (!active) return;
      // Pin the id ACROSS the await: a stale result from a previous session
      // must never overwrite a newer draft's fields.
      const workspaceAgentId = active.workspaceAgentId;
      try {
        const v = await ipc.skill.syncDraft({ workspaceAgentId });
        if (draftRef.current?.workspaceAgentId !== workspaceAgentId) return;
        if (mountedRef.current) {
          setSyncError(null);
          setOfferForceStop(false);
        }
        onSynced(v);
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        // The editor's fields are deliberately left untouched — a failed sync
        // must not destroy the last successfully synced state (design spec).
        if (draftRef.current?.workspaceAgentId === workspaceAgentId && mountedRef.current) {
          setSyncError(`Couldn't read the agent's file — ${msg}. Your last synced values are unchanged.`);
        }
        if (throwOnFailure) throw e;
      }
    },
    [onSynced],
  );

  async function handleSyncNow() {
    if (!draftRef.current || syncing) return;
    setSyncing(true);
    try {
      await runSync(false);
    } finally {
      if (mountedRef.current) setSyncing(false);
    }
  }

  /**
   * Stop the session. Syncs FIRST: `stopDraftSession` deletes the scratch dir,
   * so an unsynced edit is unrecoverable after it. A failed sync therefore
   * blocks the stop and offers `force` as an explicit second choice rather than
   * discarding the newest file silently — or wedging the session forever when
   * SKILL.md is missing or unparsable and sync can never succeed.
   */
  async function handleStop(force = false) {
    const active = draftRef.current;
    if (!active || stopping) return;
    const workspaceAgentId = active.workspaceAgentId;
    setStopping(true);
    setStopError(null);
    try {
      if (!force) {
        try {
          await runSync(true);
        } catch {
          if (mountedRef.current) setOfferForceStop(true);
          return; // draft kept, editor stays locked, error is on screen
        }
      }
      await ipc.skill.stopDraftSession({ workspaceAgentId });
      if (draftRef.current?.workspaceAgentId !== workspaceAgentId) return;
      if (mountedRef.current) {
        setOfferForceStop(false);
        setSyncError(null);
      }
      // Unlock ONLY after the stop actually succeeded.
      onStopped();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (mountedRef.current) setStopError(`Couldn't stop the agent — ${msg}. The draft is kept; try again.`);
    } finally {
      if (mountedRef.current) setStopping(false);
    }
  }

  async function handleSend() {
    const active = draftRef.current;
    if (!active || sending || draftText.trim().length === 0) return;
    const text = draftText;
    const sessionId = active.sessionId;
    setSending(true);
    setError(null);
    try {
      // Two steps, exactly as StdinBar does it. `paste: true` keeps text longer
      // than the PTY's ~1 KB read chunk in ONE bracketed paste; the CR then
      // arrives on its own a beat later, because a CR inside the same burst is
      // read as part of the paste and inserted literally instead of submitting.
      await ipc.message.send({ sessionId, text, paste: true });
      await new Promise((r) => setTimeout(r, SUBMIT_CR_DELAY_MS));
      await ipc.message.send({ sessionId, text: "\r" });
      // Cleared only once BOTH landed — otherwise a failed submit would lose
      // the user's text with nothing sent.
      if (mountedRef.current) setDraftText("");
    } catch (e) {
      if (mountedRef.current) setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (mountedRef.current) setSending(false);
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
          <div className="flex items-center gap-0.5">
            <button
              onClick={() => void handleSyncNow()}
              disabled={syncing}
              title="Sync now — pull the agent's edits into the editor"
              aria-label="Sync now"
              className="w-7 h-7 grid place-items-center rounded-md text-text-secondary hover:bg-overlay/[0.06] disabled:opacity-50"
            >
              <RefreshCw className={`w-3.5 h-3.5${syncing ? " animate-spin motion-reduce:animate-none" : ""}`} />
            </button>
            <button
              onClick={() => void handleStop()}
              disabled={stopping}
              title="Stop agent"
              aria-label="Stop agent"
              className="w-7 h-7 grid place-items-center rounded-md text-danger hover:bg-danger/[0.12] disabled:opacity-50"
            >
              <Square className="w-3.5 h-3.5" />
            </button>
          </div>
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
          {/* R1: the terminal is a real one. Say so — the CLI's trust and
              permission prompts are answered right here, with the keyboard. */}
          <div className="px-2.5 py-1 text-[10.5px] leading-snug text-text-tertiary border-t border-overlay/[0.06] shrink-0">
            Click the terminal and type to answer the agent's prompts — arrows and Enter work.
          </div>
          {(syncError || stopError) && (
            <div className="px-2.5 pb-1.5 space-y-1 shrink-0">
              {syncError && <p className="text-[11px] text-danger leading-snug">{syncError}</p>}
              {stopError && <p className="text-[11px] text-danger leading-snug">{stopError}</p>}
              {offerForceStop && (
                <div className="rounded-lg bg-overlay/[0.05] p-2 space-y-1.5">
                  <p className="text-[10.5px] leading-snug text-text-secondary">
                    The latest draft could not be read. Stopping without syncing keeps the version
                    shown in the editor and discards unsynced draft changes. Retry Sync remains
                    available.
                  </p>
                  <button
                    onClick={() => void handleStop(true)}
                    disabled={stopping}
                    className="w-full rounded-md bg-danger/[0.12] text-danger text-[11px] font-semibold py-1.5 hover:bg-danger/[0.18] disabled:opacity-50"
                  >
                    Stop without syncing
                  </button>
                </div>
              )}
            </div>
          )}
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
