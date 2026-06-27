import { useEffect, useRef, useState } from "react";
import { Code, Eye, Maximize2, X, FileCode2 } from "lucide-react";

// ---------------------------------------------------------------------------
// ArtifactView — renders a sandboxed HTML artifact emitted by an AI assistant.
//
// HONESTY NOTE — artifact DB persistence (the `artifact` table that FKs to
// `message(id)`) is deferred: message persistence itself is not yet
// implemented. Artifacts are parsed live from streamed assistant message text;
// nothing is written to the artifact table here. This will be wired once
// message persistence lands (future milestone).
// ---------------------------------------------------------------------------

// ── Types ───────────────────────────────────────────────────────────────────

export type ArtifactSegment =
  | { kind: "text"; text: string }
  | { kind: "html"; html: string; filename?: string };

// ── Parser ──────────────────────────────────────────────────────────────────

/**
 * Split an assistant message's raw text into ordered segments of plain text
 * and closed HTML fenced blocks. Unclosed trailing ```html blocks are left as
 * part of the text segment (they are still streaming — not yet a complete
 * artifact).
 *
 * The closing ``` is anchored to the START of a line (preceded by `\n`, then
 * `` ``` `` + optional trailing spaces + line-end). This prevents a triple
 * backtick that appears *inside* the HTML body (e.g. in a `<script>` template
 * literal or an embedded code sample) from prematurely closing the fence.
 *
 * Walk-through:
 * (i)  One closed block: text-before becomes a text segment, the matched fence
 *      becomes an html segment, text-after becomes a text segment.
 * (ii) Unclosed trailing ```html: FENCE_RE finds no match (no line-start ```),
 *      so the entire string falls through as a single text segment.
 * (iii) Two blocks with text between: text, html, text, html, text — emitted
 *       in source order via the lastIndex-tracked exec loop.
 */
export function splitMessageArtifacts(text: string): ArtifactSegment[] {
  // Matches a CLOSED ```html or ```htm fence.
  // Group 1: info string (everything after `html`/`htm` on the opening line).
  // Group 2: inner content (verbatim, lazy). The closing fence must sit at the
  //          start of its own line — `\n` then `` ``` `` then optional spaces
  //          then a newline or end-of-string — so an inline ``` won't close it.
  const FENCE_RE = /```html?\b([^\n]*)\n([\s\S]*?)\n```[ \t]*(?:\r?\n|$)/gi;

  const segments: ArtifactSegment[] = [];
  let lastIndex = 0;
  let match: RegExpExecArray | null;

  while ((match = FENCE_RE.exec(text)) !== null) {
    // Text before this fence → trim and emit if non-empty.
    const gap = text.slice(lastIndex, match.index).trim();
    if (gap) segments.push({ kind: "text", text: gap });

    // Parse optional filename from the info string.
    // Accepts: ```html filename.html  or  ```html title="filename.html"
    const info = match[1].trim();
    let filename: string | undefined;
    if (info) {
      const titleMatch = /title="([^"]*)"/.exec(info);
      if (titleMatch) {
        filename = titleMatch[1] || undefined;
      } else {
        const bare = info.split(/\s+/)[0];
        if (bare) filename = bare;
      }
    }

    segments.push({ kind: "html", html: match[2], filename });
    lastIndex = match.index + match[0].length;
  }

  // Remaining text after the last fence (or the entire string when no fences matched).
  const tail = text.slice(lastIndex).trim();
  if (tail) segments.push({ kind: "text", text: tail });

  return segments;
}

// ── Sandboxed iframe ────────────────────────────────────────────────────────

/**
 * Inject a restrictive Content-Security-Policy `<meta>` into the artifact HTML.
 *
 * The iframe sandbox already isolates the artifact's *origin* (see below), but
 * `allow-scripts` still grants full NETWORK access — a malicious script could
 * `fetch()`/`<img src>` to exfiltrate whatever the model placed in the artifact.
 * `default-src 'none'` blocks all network (fetch / XHR / WebSocket / external
 * scripts, styles, fonts, remote images); inline scripts/styles still run so
 * self-contained interactive artifacts work. Trade-off: artifacts that pull
 * from a CDN won't load remote resources — the secure default for rendering
 * semi-trusted model output inside a desktop app.
 */
function withSandboxCsp(html: string): string {
  const meta =
    `<meta http-equiv="Content-Security-Policy" content="` +
    `default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; ` +
    `img-src data: blob:; font-src data:;">`;
  if (/<head[^>]*>/i.test(html)) {
    return html.replace(/(<head[^>]*>)/i, `$1${meta}`);
  }
  if (/<html[^>]*>/i.test(html)) {
    return html.replace(/(<html[^>]*>)/i, `$1<head>${meta}</head>`);
  }
  return `${meta}${html}`;
}

function ArtifactFrame({ html, title }: { html: string; title: string }) {
  return (
    <iframe
      title={title}
      srcDoc={withSandboxCsp(html)}
      // allow-scripts WITHOUT allow-same-origin → the artifact runs in a UNIQUE
      // OPAQUE origin: its own JS works, but it cannot reach the parent DOM,
      // cookies, localStorage, or Tauri IPC (those live on the parent origin,
      // unreachable across the opaque-origin boundary). NEVER add
      // allow-same-origin here — combined with allow-scripts it would let the
      // artifact script the host app. allow-popups / allow-modals /
      // allow-top-navigation / allow-forms are all withheld by omission.
      // NOTE: empirically confirm on first GUI run that Tauri's IPC init script
      // is NOT injected into this sandboxed frame (forMainFrameOnly); the opaque
      // origin + null-origin ACL rejection is the expected guard.
      sandbox="allow-scripts"
      referrerPolicy="no-referrer"
      className="w-full h-full border-0 bg-white"
    />
  );
}

// ── Preview / Code segmented toggle ─────────────────────────────────────────

type ViewMode = "preview" | "code";

function ViewToggle({
  view,
  onChange,
}: {
  view: ViewMode;
  onChange: (v: ViewMode) => void;
}) {
  return (
    <div className="flex items-center rounded-md overflow-hidden ring-1 ring-black/[0.08] bg-black/[0.03]">
      <button
        onClick={() => onChange("preview")}
        aria-pressed={view === "preview"}
        className={`flex items-center gap-1 px-2 py-1 text-[11px] font-medium transition-colors ${
          view === "preview"
            ? "bg-white text-[#0a84ff] shadow-sm"
            : "text-[#6e6e73] hover:text-[#3a3a3c]"
        }`}
      >
        <Eye className="w-3 h-3" />
        Preview
      </button>
      <button
        onClick={() => onChange("code")}
        aria-pressed={view === "code"}
        className={`flex items-center gap-1 px-2 py-1 text-[11px] font-medium transition-colors ${
          view === "code"
            ? "bg-white text-[#0a84ff] shadow-sm"
            : "text-[#6e6e73] hover:text-[#3a3a3c]"
        }`}
      >
        <Code className="w-3 h-3" />
        Code
      </button>
    </div>
  );
}

// ── Main component ───────────────────────────────────────────────────────────

export function ArtifactView({ html, filename }: { html: string; filename?: string }) {
  const [view, setView] = useState<ViewMode>("preview");
  // Canvas keeps its OWN view mode (resets to Preview each open) so a full-screen
  // view is independent of the inline card's toggle state.
  const [canvasView, setCanvasView] = useState<ViewMode>("preview");
  const [canvasOpen, setCanvasOpen] = useState(false);
  const label = filename ?? "artifact.html";
  const titleId = `artifact-canvas-title`;
  const closeBtnRef = useRef<HTMLButtonElement | null>(null);

  function openCanvas() {
    setCanvasView("preview");
    setCanvasOpen(true);
  }

  // Escape closes the canvas overlay + move focus into the dialog on open
  // (StrictMode-safe: listener is added/removed inside an effect gated on
  // `canvasOpen`, with cleanup on every change).
  useEffect(() => {
    if (!canvasOpen) return;
    closeBtnRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setCanvasOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [canvasOpen]);

  return (
    <>
      {/* ── Inline card ───────────────────────────────────────────────────── */}
      <div className="ring-1 ring-black/[0.08] rounded-xl overflow-hidden bg-white">
        {/* Header */}
        <div className="flex items-center gap-2 px-3 py-2 border-b border-black/[0.06] bg-[#fafafa]">
          <FileCode2 className="w-3.5 h-3.5 text-[#6e6e73] shrink-0" />
          <span className="text-[12.5px] font-medium text-[#3a3a3c] truncate flex-1 min-w-0">
            {label}
          </span>
          <ViewToggle view={view} onChange={setView} />
          <button
            onClick={openCanvas}
            className="ml-1 w-6 h-6 grid place-items-center rounded-md hover:bg-black/[0.06] text-[#6e6e73] shrink-0"
            aria-label="Open in canvas"
          >
            <Maximize2 className="w-3.5 h-3.5" />
          </button>
        </div>

        {/* Body. While the canvas overlay is open we drop the inline live frame
            (a placeholder) so only ONE artifact iframe runs at a time. */}
        <div className="h-72">
          {canvasOpen ? (
            <div className="h-full grid place-items-center text-[12px] text-[#a1a1a6] bg-[#fafafa]">
              Open in canvas — close it to preview here.
            </div>
          ) : view === "preview" ? (
            <ArtifactFrame html={html} title={label} />
          ) : (
            <pre className="h-full overflow-auto p-3 m-0 bg-[#f8f8f8] text-[11.5px] font-mono leading-relaxed text-[#3a3a3c]">
              <code>{html}</code>
            </pre>
          )}
        </div>
      </div>

      {/* ── Canvas overlay (mirrors Timeline: fixed inset-0 z-50) ─────────── */}
      {canvasOpen && (
        <div
          className="fixed inset-0 z-50 flex flex-col bg-[#f5f5f7]"
          role="dialog"
          aria-modal="true"
          aria-labelledby={titleId}
        >
          {/* Canvas header */}
          <div className="h-12 flex items-center justify-between px-4 border-b border-black/[0.06] bg-white shrink-0">
            <div className="flex items-center gap-2 min-w-0">
              <FileCode2 className="w-4 h-4 text-[#6e6e73] shrink-0" />
              <span id={titleId} className="text-[13px] font-semibold tracking-tight truncate">
                {label}
              </span>
            </div>
            <div className="flex items-center gap-2 shrink-0">
              <ViewToggle view={canvasView} onChange={setCanvasView} />
              <button
                ref={closeBtnRef}
                onClick={() => setCanvasOpen(false)}
                className="ml-1 w-7 h-7 grid place-items-center rounded-lg hover:bg-black/[0.06] text-[#6e6e73]"
                aria-label="Close canvas"
              >
                <X className="w-4 h-4" />
              </button>
            </div>
          </div>

          {/* Canvas body */}
          <div className="flex-1 overflow-hidden">
            {canvasView === "preview" ? (
              <ArtifactFrame html={html} title={label} />
            ) : (
              <pre className="h-full overflow-auto p-6 m-0 bg-white text-[12px] font-mono leading-relaxed text-[#3a3a3c]">
                <code>{html}</code>
              </pre>
            )}
          </div>
        </div>
      )}
    </>
  );
}
