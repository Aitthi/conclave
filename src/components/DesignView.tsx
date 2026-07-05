import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ExternalLink, LoaderCircle, PenTool, X } from "lucide-react";
import { ipc } from "../ipc";

interface DesignViewProps {
  workspaceId: string;
  workspaceName?: string;
  /** Clears the parent's `designOpen` flag — WorkspacePane keeps rendering
   *  the (now-empty) design slot; only this component unmounts. */
  onClose: () => void;
}

type State =
  | { kind: "loading" }
  | { kind: "error"; nodeMissing: boolean; message: string }
  | { kind: "ready"; url: string; projectId: string };

const NODE_MISSING_MARKER = "node was not found on PATH";

export function DesignView({ workspaceId, workspaceName, onClose }: DesignViewProps) {
  const [state, setState] = useState<State>({ kind: "loading" });
  // Bumped by the Retry button to re-run the effect without duplicating its body.
  const [attempt, setAttempt] = useState(0);

  // Re-runs on `workspaceId` change too — belt-and-suspenders: in practice
  // AppShell remounts the WHOLE WorkspacePane (and this component with it)
  // on a workspace switch already (key={`${workspaceId}:${agentsVersion}`}),
  // but this keeps DesignView correct standalone if that ever changes.
  useEffect(() => {
    let active = true;
    setState({ kind: "loading" });
    ipc.design
      .ensure({ workspaceId })
      .then((info) => {
        if (!active) return;
        if (info.running && info.url) {
          setState({ kind: "ready", url: info.url, projectId: info.projectId });
        } else {
          // Backend contract: `ensure` either resolves with running:true+url,
          // or rejects — this is defensive, not an expected path.
          setState({ kind: "error", nodeMissing: false, message: "design viewer did not report a running URL" });
        }
      })
      .catch((err: unknown) => {
        if (!active) return;
        const message = err instanceof Error ? err.message : String(err);
        setState({ kind: "error", nodeMissing: message.includes(NODE_MISSING_MARKER), message });
      });
    return () => {
      active = false;
    };
  }, [workspaceId, attempt]);

  return (
    <div className="flex flex-col h-full min-h-0">
      {/* Header strip — matches the h-12 height of the other view headers
          (LaneBoard/MemoryGraph) but sits in normal flow (not absolute/
          floating over content) since it shares this column with an iframe:
          an absolutely-positioned header above an iframe is a pointer-event
          minefield, plain flex stacking has none of that risk. */}
      <div className="h-12 shrink-0 flex items-center gap-2 px-3 border-b border-overlay/[0.06] bg-sidebar">
        <PenTool size={14} className="text-text-tertiary shrink-0" />
        <span className="text-[13px] font-semibold text-text-primary truncate">
          {workspaceName ?? "Design"}
        </span>
        <span className="text-[11px] text-text-tertiary shrink-0">Design</span>
        <div className="ml-auto flex items-center gap-1 shrink-0">
          {state.kind === "ready" && (
            <button
              onClick={() => {
                openUrl(state.url).catch(() => {
                  // Best-effort — no in-app fallback if the OS opener fails.
                });
              }}
              title="Open in browser"
              className="w-7 h-7 grid place-items-center rounded-md hover:bg-overlay/[0.05] text-text-secondary"
            >
              <ExternalLink size={14} />
            </button>
          )}
          <button
            onClick={onClose}
            title="Close Design"
            className="w-7 h-7 grid place-items-center rounded-md hover:bg-overlay/[0.05] text-text-secondary"
          >
            <X size={14} />
          </button>
        </div>
      </div>

      <div className="flex-1 min-h-0 relative bg-[#1e1e1e]">
        {state.kind === "loading" && (
          <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 text-[13px] text-text-tertiary">
            <LoaderCircle size={20} className="animate-spin" />
            <span>Starting design viewer…</span>
          </div>
        )}
        {state.kind === "error" && (
          <div className="absolute inset-0 grid place-items-center px-6 text-center">
            <div className="flex flex-col items-center gap-3 max-w-[320px]">
              <span className="text-[13px] text-danger">
                {state.nodeMissing
                  ? "Node.js is required for the Design view."
                  : "Couldn't start the design viewer."}
              </span>
              <span className="text-[12px] text-text-tertiary">
                {state.nodeMissing ? "Install Node.js ≥18 and reopen." : state.message}
              </span>
              <button
                onClick={() => setAttempt((a) => a + 1)}
                className="px-3 h-7 rounded-md text-[12px] font-medium bg-overlay/[0.06] hover:bg-overlay/[0.1] text-text-primary"
              >
                Retry
              </button>
            </div>
          </div>
        )}
        {state.kind === "ready" && (
          // Unsandboxed, deliberately: this is a same-machine, engine-spawned
          // sidecar bound to 127.0.0.1 (never remote content) whose own Vite
          // dev server needs full script access for its HMR WebSocket and
          // same-origin XHR — a sandboxed iframe would break both. Hardened
          // at the sidecar's own config instead (review ruling 4d81be26:
          // cors:false, explicit allowedHosts, fs.deny for secret globs).
          <iframe
            key={state.projectId}
            src={state.url}
            title="Design canvas"
            className="absolute inset-0 w-full h-full border-0"
          />
        )}
      </div>
    </div>
  );
}
