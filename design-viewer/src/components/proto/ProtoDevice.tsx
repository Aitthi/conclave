import { useEffect, useMemo, useRef } from "react";
import { captureFramedPng } from "../../lib/capture";
import { reportSnapshot } from "../../lib/useArta";

export interface AnnotateTarget { tag: string; text: string; selector: string }

interface Props {
  projectId: string; screenId: string; title: string; annotate: boolean;
  captureNodeRef: React.RefObject<HTMLElement | null>;
  go: (to: string) => void;
  onError: (m: string) => void;
  onAnnotate: (t: AnnotateTarget) => void;
}

export function ProtoDevice({ projectId, screenId, title, annotate, captureNodeRef, go, onError, onAnnotate }: Props) {
  const frameRef = useRef<HTMLIFrameElement>(null);
  const cbs = useRef({ go, onError, onAnnotate });
  cbs.current = { go, onError, onAnnotate };
  // Load the proto app ONCE per project; navigate via postMessage (no iframe
  // reload per screen — the shell is a router).
  const src = useMemo(() => `/proto/index.html?project=${encodeURIComponent(projectId)}#/${encodeURIComponent(screenId)}`,
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [projectId]);
  const post = (msg: Record<string, unknown>) =>
    frameRef.current?.contentWindow?.postMessage({ source: "arta-parent", ...msg }, "*");

  useEffect(() => { post({ type: "nav", to: screenId }); }, [screenId]);
  useEffect(() => { post({ type: "annotate", on: annotate }); }, [annotate]);

  useEffect(() => {
    const onMsg = (e: MessageEvent) => {
      const d = e.data;
      if (!d || d.source !== "arta-frame") return;
      if (d.type === "route" && typeof d.screen === "string" && d.screen !== screenId) cbs.current.go(d.screen);
      else if (d.type === "error" && d.message) cbs.current.onError(String(d.message));
      else if (d.type === "annotate" && d.target) cbs.current.onAnnotate(d.target as AnnotateTarget);
      else if (d.type === "ready") {
        const node = captureNodeRef.current;
        if (node) captureFramedPng(node).then((png) => reportSnapshot(String(d.screen), png)).catch(() => {});
        post({ type: "annotate", on: annotate });
      }
    };
    window.addEventListener("message", onMsg);
    return () => window.removeEventListener("message", onMsg);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [screenId, annotate]);

  return (
    <iframe ref={frameRef} title={title} src={src}
      sandbox="allow-scripts allow-forms allow-popups allow-same-origin"
      className="h-full w-full border-0 bg-white" />
  );
}
