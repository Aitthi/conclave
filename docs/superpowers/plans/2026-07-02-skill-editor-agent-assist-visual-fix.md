# Skill Editor Visual Fix (Arta-approved design) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Port the Arta-approved redesign of the Skill Editor's agent-assist panel into real code: fix the CodeMirror content editor's light-on-dark visual bug, replace the assist panel's plain-text output with a real xterm.js terminal (matching the app's existing Terminal pane's rendering approach, since PTY output contains ANSI codes plain text can't render), and match the approved composer/header layout.

**Architecture:** Two isolated, additive frontend changes — a custom CodeMirror dark theme extension, and an xterm.js instance (reusing the `@xterm/xterm` + `@xterm/addon-fit` dependencies already used by `Terminal.tsx`) inside `SkillAssistPanel.tsx`. No backend changes.

**Design reference:** Approved interactively via Arta (`.arta/prototype/screens/skill-editor.html`, `skill-editor-active.html`) — both screens are visible at the Arta viewer for comparison during implementation.

## Global Constraints

- Use the app's existing theme-aware Tailwind utility classes (`bg-sidebar`, `bg-fill-softer`, `text-danger`, `bg-accent`, `text-status-running`, `bg-status-running`, `text-status-idle`, `bg-status-idle`, `ring-overlay/[...]`, etc. — all backed by CSS custom properties in `src/styles/app.css` that flip with the `.dark` class) rather than hardcoded hex values, so the result respects the app's light/dark theme toggle. The Arta mockup hardcodes hex (a static prototype tool has no choice); the real implementation must not.
- No new npm dependencies — `@xterm/xterm` and `@xterm/addon-fit` are already dependencies (used by `src/components/Terminal.tsx`).
- The assist panel's xterm instance does NOT need to signal PTY resize back to the backend (`ipc.session.resize`) — that coupling exists in `Terminal.tsx` for the main terminal pane's real user interaction; this panel is a secondary, narrower display surface where exact column-accurate wrapping isn't load-bearing. Skip it (YAGNI).
- The composer's paperclip/attach icon from the Arta mockup is dropped in the real implementation — there is no file-attach IPC wired for a skill-draft session, and this codebase doesn't ship decorative non-functional buttons. Keep the textarea + circular send button only.
- "Sync now" as a visible button is removed; the existing auto-sync-on-idle behavior (already implemented via `useSessionStatus`) is the only sync trigger going forward. "Stop agent" moves to a small icon button in the panel header.
- Frontend verification is `pnpm exec tsc --noEmit` and `pnpm build` — no frontend test runner in this codebase.

---

### Task 1: Dark CodeMirror theme for `SkillEditor.tsx`'s content editor

**Files:**
- Create: `src/lib/skillContentEditorTheme.ts`
- Modify: `src/components/SkillEditor.tsx`

- [ ] **Step 1: Create the theme extension**

```typescript
// Create: src/lib/skillContentEditorTheme.ts
import { EditorView } from "@codemirror/view";

/**
 * Dark CodeMirror theme for the skill content editor, matching this app's
 * real dark palette (src/styles/app.css `.dark` block) exactly rather than
 * a generic third-party theme — the editor's colors must match the rest of
 * the (currently dark-only) full-panel Skill Editor, not clash with it.
 */
export const skillContentEditorTheme = EditorView.theme(
  {
    "&": {
      backgroundColor: "#1c1c1e",
      color: "#d8d8da",
      height: "100%",
    },
    ".cm-content": {
      caretColor: "#f5f5f7",
      fontFamily: '"SF Mono", "JetBrains Mono", ui-monospace, Menlo, monospace',
      fontSize: "12.5px",
    },
    ".cm-cursor, .cm-dropCursor": { borderLeftColor: "#f5f5f7" },
    "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection": {
      backgroundColor: "rgba(10, 132, 255, 0.25)",
    },
    ".cm-activeLine": { backgroundColor: "rgba(10, 132, 255, 0.08)" },
    ".cm-gutters": {
      backgroundColor: "#18181a",
      color: "#525256",
      border: "none",
    },
    ".cm-activeLineGutter": {
      backgroundColor: "rgba(10, 132, 255, 0.08)",
      color: "#9a9aa0",
    },
    ".cm-scroller": { fontFamily: "inherit" },
  },
  { dark: true },
);
```

- [ ] **Step 2: Wire it into `SkillEditor.tsx`**

In `src/components/SkillEditor.tsx`, add the import:

```tsx
import { skillContentEditorTheme } from "../lib/skillContentEditorTheme";
```

Add `theme={skillContentEditorTheme}` to the existing `<CodeMirror>` element (which already has `value`, `onChange`, `editable`, `extensions={[markdown()]}`, `height="100%"`, `className="h-full text-[12.5px]"` — do not change those props, only add `theme`):

```tsx
              <CodeMirror
                value={content}
                onChange={(value) => setContent(value)}
                editable={!locked}
                theme={skillContentEditorTheme}
                extensions={[markdown()]}
                height="100%"
                className="h-full text-[12.5px]"
              />
```

- [ ] **Step 3: Verify**

Run: `pnpm exec tsc --noEmit && pnpm build`
Expected: clean. Manually compare against the Arta prototype screen `skill-editor` (dark editor surface, `#18181a` gutter, no white background) — if a running `.app` isn't available in this environment, say so explicitly rather than claim a visual check was performed.

- [ ] **Step 4: Commit**

```bash
git add src/lib/skillContentEditorTheme.ts src/components/SkillEditor.tsx
git commit -m "fix(skill-editor): dark CodeMirror theme matching the app's real dark palette"
```

---

### Task 2: `SkillAssistPanel.tsx` — xterm.js output, header status/stop, composer redesign

**Files:**
- Modify: `src/components/SkillAssistPanel.tsx`

**Interfaces:** Component's public props (`SkillAssistPanelProps`, `DraftSession`) and its consumer in `SkillEditor.tsx` (`onStarted`/`onSynced`/`onStopped` callbacks, `draft` prop) are UNCHANGED — this task only changes the component's internals and JSX, not its contract.

- [ ] **Step 1: Replace the file**

```tsx
// Replace: src/components/SkillAssistPanel.tsx
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
```

- [ ] **Step 2: Verify**

Run: `pnpm exec tsc --noEmit && pnpm build`
Expected: clean.

Trace these specific correctness points while reviewing (don't just eyeball the diff):
- The xterm-creation `useEffect` is keyed on `draft?.workspaceAgentId`, not `draft` — confirm a `DraftSession` object identity change with the SAME `workspaceAgentId` (shouldn't happen in practice, but the dependency choice matters) doesn't spuriously recreate the terminal, while a genuinely new session (new `workspaceAgentId`) does.
- `termRef.current?.write(...)` inside `useSessionOutput`'s callback must be a no-op (not throw) if called after the terminal was disposed (e.g. a chunk arrives in the brief window between `stop` and the effect's cleanup running) — confirm this is safe by construction (the ref is nulled in the cleanup function BEFORE React can process a subsequent event callback in the same tick... trace this carefully, it's the one real lifecycle risk in this task).
- `resizeObserver.disconnect()` and `term.dispose()` both run in the cleanup — confirm no leak if `Start` → `Stop` → `Start` happens in quick succession (each mount gets its own fresh observer + terminal, previous ones fully torn down).

- [ ] **Step 3: Commit**

```bash
git add src/components/SkillAssistPanel.tsx
git commit -m "feat(skill-editor): render agent-assist output via xterm.js, redesign composer (Arta-approved)"
```

---

## Final verification (after both tasks, done by the orchestrator)

Run:
1. `pnpm exec tsc --noEmit`
2. `pnpm build`

Compare the result against the two approved Arta screens (`skill-editor`, `skill-editor-active`) for visual fidelity. Disclose explicitly that a live `.app` smoke test (actually starting a real agent-assist session and watching xterm receive real PTY output) was not performed if no display is available in this environment — this is a real, not-yet-exercised code path (xterm.js DOM/canvas rendering can't be verified by `tsc`/`build` alone).
