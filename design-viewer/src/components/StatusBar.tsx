import { useState } from "react";
import type { Meta, Phase } from "../lib/types";
import type { ChangeEntry } from "../lib/useArta";
import { MONO, SANS, useTheme } from "../lib/theme";
import { sendFeedback } from "../lib/useArta";

interface Props {
  meta: Meta;
  updatedAt: string;
  tab: Phase;
  screen: string | undefined;
  changes: ChangeEntry[];
  onJump: (ch: ChangeEntry) => void;
}

export function StatusBar({ meta, updatedAt, tab, screen, changes, onJump }: Props) {
  const { c } = useTheme();
  return (
    <div
      className="flex h-[30px] shrink-0 items-center gap-[14px] px-3 text-[11.5px]"
      style={{ borderTop: `1px solid ${c.border}`, background: c.panel }}
    >
      <span className="flex items-center gap-[7px]" style={{ fontFamily: MONO, color: c.dim }}>
        <span className="h-1.5 w-1.5 rounded-full" style={{ background: c.accent }} />
        .arta/state.json
      </span>
      <span style={{ color: c.faint }}>·</span>
      <span style={{ color: c.faint }}>phase: {meta.phase}</span>
      <div className="ml-auto flex items-center gap-3.5">
        <span style={{ color: c.faint, fontFamily: MONO }}>updated {updatedAt}</span>
        <span style={{ color: c.borderStrong }}>|</span>
        <ChangesButton changes={changes} onJump={onJump} />
        <FeedbackButton tab={tab} screen={screen} />
      </div>
    </div>
  );
}

// A legible feed of what the AI just changed — so the dev can follow the edits
// (and jump to a screen the AI touched) instead of guessing from a flash.
function ChangesButton({ changes, onJump }: { changes: ChangeEntry[]; onJump: (ch: ChangeEntry) => void }) {
  const { c } = useTheme();
  const [open, setOpen] = useState(false);
  const KIND_COLOR: Record<string, string> = {
    screen: c.accent,
    component: c.amber,
    designSystem: c.green,
    state: c.dim,
  };
  return (
    <div className="relative">
      <button
        onClick={() => setOpen((o) => !o)}
        className="flex h-[22px] items-center gap-[7px] rounded-md px-[9px] transition-colors"
        style={{
          color: c.text,
          border: `1px solid ${open ? c.borderStrong : c.border2}`,
          background: open ? c.card : c.bg,
        }}
      >
        <span className="h-1.5 w-1.5 rounded-full" style={{ background: c.green }} />
        Changes
        <span style={{ fontFamily: MONO, color: c.faint }}>{changes.length}</span>
      </button>
      {open && (
        <>
          <div className="fixed inset-0 z-10" onClick={() => setOpen(false)} />
          <div
            className="hs-pop absolute bottom-8 right-0 z-20 flex max-h-[420px] w-[340px] flex-col overflow-hidden rounded-xl"
            style={{ background: c.panel, border: `1px solid ${c.border2}`, boxShadow: c.shadow }}
          >
            <div
              className="flex items-center justify-between px-3.5 py-3"
              style={{ borderBottom: `1px solid ${c.border}` }}
            >
              <span className="text-[13px] font-semibold" style={{ color: c.text }}>
                Changes
              </span>
              <span className="text-[11px]" style={{ fontFamily: MONO, color: c.faint }}>
                live feed
              </span>
            </div>
            <div className="overflow-y-auto p-1.5">
              {changes.length === 0 ? (
                <div className="px-2 py-3 text-center text-[11px]" style={{ color: c.faint }}>
                  No edits yet — changes the AI makes will appear here.
                </div>
              ) : (
                changes.map((ch, i) => (
                  <button
                    key={i}
                    onClick={() => {
                      onJump(ch);
                      setOpen(false);
                    }}
                    className="flex w-full items-start gap-2.5 rounded-[9px] p-2.5 text-left transition-colors"
                    style={{ color: c.text }}
                  >
                    <span
                      className="mt-[5px] h-[7px] w-[7px] shrink-0 rounded-full"
                      style={{ background: KIND_COLOR[ch.kind] || c.faint }}
                    />
                    <span className="flex-1 truncate text-[12.5px]" style={{ fontFamily: SANS }}>
                      {ch.label}
                    </span>
                    <span className="shrink-0 text-[11px]" style={{ fontFamily: MONO, color: c.faint }}>
                      {ch.at}
                    </span>
                  </button>
                ))
              )}
            </div>
          </div>
        </>
      )}
    </div>
  );
}

// Lets the dev leave a note from inside the viewer. The MCP server drains it via
// arta_get_feedback, so the dev → AI half of the loop never leaves the screen.
function FeedbackButton({ tab, screen }: { tab: Phase; screen: string | undefined }) {
  const { c } = useTheme();
  const [open, setOpen] = useState(false);
  const [text, setText] = useState("");
  const [sent, setSent] = useState(false);

  const submit = async () => {
    const t = text.trim();
    if (!t) return;
    const ok = await sendFeedback({ text: t, tab, screen });
    if (ok) {
      setText("");
      setSent(true);
      setTimeout(() => {
        setSent(false);
        setOpen(false);
      }, 1100);
    }
  };

  return (
    <div className="relative">
      <button
        onClick={() => setOpen((o) => !o)}
        className="flex h-[22px] items-center rounded-md px-[9px] transition-colors"
        style={{
          color: c.dim,
          border: `1px solid ${open ? c.borderStrong : c.border2}`,
          background: open ? c.card : c.bg,
        }}
      >
        Feedback
      </button>
      {open && (
        <>
          <div className="fixed inset-0 z-10" onClick={() => setOpen(false)} />
          <div
            className="hs-pop absolute bottom-8 right-0 z-20 w-80 overflow-hidden rounded-xl"
            style={{ background: c.panel, border: `1px solid ${c.border2}`, boxShadow: c.shadow }}
          >
            <div
              className="px-3.5 py-3 text-[13px] font-semibold"
              style={{ color: c.text, borderBottom: `1px solid ${c.border}` }}
            >
              Send feedback to agent
            </div>
            <div className="flex flex-col gap-2.5 px-3.5 py-3">
              <textarea
                autoFocus
                value={text}
                onChange={(e) => setText(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) submit();
                }}
                placeholder="e.g. tighten the amount keypad spacing…"
                spellCheck={false}
                className="h-[78px] w-full resize-none rounded-[9px] p-2.5 text-[12.5px] leading-[1.5] outline-none"
                style={{ fontFamily: SANS, background: c.bg, border: `1px solid ${c.border2}`, color: c.text }}
              />
              <div className="flex items-center justify-between">
                <span className="text-[11px]" style={{ color: c.faint }}>
                  {sent ? "✓ sent to agent" : `Attaches ${screen ? screen : tab}`}
                </span>
                <button
                  onClick={submit}
                  className="flex h-[30px] items-center rounded-lg px-3.5 text-[12.5px] font-semibold transition-colors"
                  style={{ background: c.invBg, color: c.invFg }}
                >
                  Send
                </button>
              </div>
            </div>
          </div>
        </>
      )}
    </div>
  );
}
