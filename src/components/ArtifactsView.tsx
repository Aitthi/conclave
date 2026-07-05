import { useCallback, useEffect, useMemo, useState } from "react";
import { Code, Eye, FileCode2, Package, X } from "lucide-react";
import { ipc, type Artifact, type WorkspaceAgent } from "../ipc";
import { useArtifactChanged } from "../ipc/events";
import { timeHint } from "../lib/timeHint";
import { ArtifactFrame } from "./ArtifactView";

type ArtifactFilter = "all" | "markdown" | "code" | "html" | "svg" | "mermaid" | "react" | "text";
type PreviewMode = "preview" | "code";

const FILTERS: ArtifactFilter[] = [
  "all",
  "markdown",
  "code",
  "html",
  "svg",
  "mermaid",
  "react",
  "text",
];

function HeaderPill({ children }: { children: React.ReactNode }) {
  return (
    <span className="inline-flex items-center gap-1.5 h-6 px-2.5 rounded-full bg-surface-raised text-[11px] text-text-secondary ring-hair">
      {children}
    </span>
  );
}

function KindBadge({ kind }: { kind: string }) {
  return (
    <span className="inline-flex items-center rounded-full px-2 py-0.5 text-[10px] font-medium uppercase tracking-[0.08em] bg-accent/10 text-accent">
      {kind}
    </span>
  );
}

function ViewToggle({
  view,
  onChange,
}: {
  view: PreviewMode;
  onChange: (view: PreviewMode) => void;
}) {
  return (
    <div className="flex items-center rounded-md overflow-hidden ring-1 ring-overlay/[0.08] bg-overlay/[0.03]">
      <button
        onClick={() => onChange("preview")}
        aria-pressed={view === "preview"}
        className={`flex items-center gap-1 px-2 py-1 text-[11px] font-medium transition-colors ${
          view === "preview"
            ? "bg-surface text-accent shadow-sm"
            : "text-text-secondary hover:text-text-body"
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
            ? "bg-surface text-accent shadow-sm"
            : "text-text-secondary hover:text-text-body"
        }`}
      >
        <Code className="w-3 h-3" />
        Code
      </button>
    </div>
  );
}

function normalizeKind(kind?: string): ArtifactFilter {
  switch (kind) {
    case "markdown":
    case "code":
    case "html":
    case "svg":
    case "mermaid":
    case "react":
    case "text":
      return kind;
    default:
      return "text";
  }
}

function agentLabel(artifact: Artifact, roster: WorkspaceAgent[]): string {
  if (!artifact.agentId) return "Unknown agent";
  const match = roster.find((row) => row.id === artifact.agentId);
  return match?.name ?? artifact.agentId;
}

function htmlForPreview(artifact: Artifact): string {
  const content = artifact.content ?? artifact.html ?? "";
  if (normalizeKind(artifact.kind) !== "svg") return content;
  return `<!doctype html><html><body style="margin:0;display:grid;place-items:center;min-height:100vh;background:transparent;">${content}</body></html>`;
}

function ArtifactBody({
  artifact,
  view,
}: {
  artifact: Artifact;
  view: PreviewMode;
}) {
  const kind = normalizeKind(artifact.kind);
  const content = artifact.content ?? artifact.html ?? "";

  if ((kind === "html" || kind === "svg") && view === "preview") {
    return <ArtifactFrame html={htmlForPreview(artifact)} title={artifact.title ?? artifact.id} />;
  }

  // Phase 1 keeps `react` artifacts in code mode only — no in-app transpiler.
  const note =
    kind === "react" ? "React artifacts render as code in Phase 1." : null;

  return (
    <div className="h-full overflow-auto bg-fill-softer">
      {note && (
        <div className="px-4 pt-4 text-[11px] text-text-tertiary">{note}</div>
      )}
      <pre className="min-h-full p-4 m-0 whitespace-pre-wrap break-words text-[12px] font-mono leading-relaxed text-text-body">
        <code>{content}</code>
      </pre>
    </div>
  );
}

export function ArtifactsView({
  workspaceId,
  workspaceName,
  onClose,
}: {
  workspaceId: string;
  workspaceName?: string;
  onClose: () => void;
}) {
  const [artifacts, setArtifacts] = useState<Artifact[]>([]);
  const [roster, setRoster] = useState<WorkspaceAgent[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<ArtifactFilter>("all");
  const [activeId, setActiveId] = useState<string | null>(null);
  const [view, setView] = useState<PreviewMode>("preview");

  const loadArtifacts = useCallback(async () => {
    try {
      const [rows, agents] = await Promise.all([
        ipc.artifact.list({ workspaceId }),
        ipc.instance.list({ workspaceId }),
      ]);
      const sorted = [...rows].sort((a, b) =>
        a.createdAt < b.createdAt ? 1 : a.createdAt > b.createdAt ? -1 : 0,
      );
      setArtifacts(sorted);
      setRoster(agents);
      setError(null);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(msg);
    } finally {
      setLoading(false);
    }
  }, [workspaceId]);

  useEffect(() => {
    setLoading(true);
    setArtifacts([]);
    setRoster([]);
    setActiveId(null);
    void loadArtifacts();
  }, [loadArtifacts]);

  useArtifactChanged(workspaceId, () => {
    void loadArtifacts();
  });

  const filtered = useMemo(() => {
    return artifacts.filter((artifact) => {
      if (filter === "all") return true;
      return normalizeKind(artifact.kind) === filter;
    });
  }, [artifacts, filter]);

  useEffect(() => {
    if (filtered.length === 0) {
      setActiveId(null);
      return;
    }
    if (!filtered.some((artifact) => artifact.id === activeId)) {
      setActiveId(filtered[0].id);
    }
  }, [filtered, activeId]);

  const activeArtifact = filtered.find((artifact) => artifact.id === activeId) ?? null;

  useEffect(() => {
    setView("preview");
  }, [activeArtifact?.id]);

  const total = artifacts.length;
  const activeKind = normalizeKind(activeArtifact?.kind);
  const canPreviewToggle = activeKind === "html" || activeKind === "svg";

  return (
    <main className="flex-1 min-w-0 relative overflow-hidden">
      <div
        className="absolute top-0 left-0 right-0 h-12 z-20 flex items-center gap-3 px-4 border-b border-overlay/[0.06]"
        style={{
          background: "color-mix(in srgb, var(--color-bg-canvas) 78%, transparent)",
          backdropFilter: "blur(8px)",
        }}
      >
        <span className="w-6 h-6 rounded-[7px] grid place-items-center shrink-0 bg-surface-raised text-text-primary ring-hair">
          <Package size={13} />
        </span>
        <div className="leading-tight">
          <div className="text-[0.84rem] font-semibold tracking-tight text-text-primary">
            Artifacts
          </div>
          <div className="text-[0.64rem] -mt-0.5 text-text-tertiary">
            {workspaceName ? `${workspaceName} · saved outputs` : "saved outputs"}
          </div>
        </div>
        <HeaderPill>
          <Package size={11} style={{ color: "var(--color-accent)" }} />
          {total} {total === 1 ? "artifact" : "artifacts"}
        </HeaderPill>
        <div className="ml-auto flex items-center gap-2">
          <button
            onClick={onClose}
            title="Close artifacts"
            aria-label="Close artifacts"
            className="w-7 h-7 grid place-items-center rounded-md text-text-muted hover:bg-overlay/[0.06] hover:text-text-primary transition-colors"
          >
            <X size={15} />
          </button>
        </div>
      </div>

      <div className="absolute inset-0 pt-12 flex min-h-0">
        <aside className="w-[320px] shrink-0 border-r border-overlay/[0.06] bg-fill-softer/40 flex flex-col min-h-0">
          <div className="px-4 py-3 border-b border-overlay/[0.06]">
            <div className="flex flex-wrap gap-1.5">
              {FILTERS.map((value) => {
                const active = filter === value;
                return (
                  <button
                    key={value}
                    onClick={() => setFilter(value)}
                    className={`px-2.5 h-7 rounded-full text-[11px] font-medium capitalize transition-colors ${
                      active
                        ? "bg-accent text-white"
                        : "bg-surface text-text-secondary ring-1 ring-overlay/[0.08] hover:text-text-primary"
                    }`}
                  >
                    {value}
                  </button>
                );
              })}
            </div>
          </div>

          <div className="flex-1 min-h-0 overflow-y-auto">
            {loading ? (
              <div className="p-4 text-[12px] text-text-tertiary">Loading artifacts…</div>
            ) : error ? (
              <div className="p-4 text-[12px] text-danger">Failed to load artifacts: {error}</div>
            ) : filtered.length === 0 ? (
              <div className="p-4 space-y-2 text-[12px] text-text-tertiary">
                <div>
                  {total === 0
                    ? "No artifacts saved in this workspace yet."
                    : "No artifacts match the current filter."}
                </div>
                {total === 0 && (
                  <pre className="m-0 whitespace-pre-wrap break-words rounded-lg bg-bg-canvas p-3 text-[11px] text-text-body ring-1 ring-overlay/[0.06]">
                    <code>
                      conclave artifact add &lt;ws&gt; --title &lt;t&gt; --kind &lt;k&gt; --content
                      &lt;text&gt;
                    </code>
                  </pre>
                )}
              </div>
            ) : (
              <div className="p-2">
                {filtered.map((artifact) => {
                  const selected = artifact.id === activeId;
                  const kind = normalizeKind(artifact.kind);
                  return (
                    <button
                      key={artifact.id}
                      onClick={() => setActiveId(artifact.id)}
                      className={`w-full text-left rounded-xl px-3 py-3 mb-2 transition-colors ring-1 ${
                        selected
                          ? "bg-accent/8 ring-accent/30"
                          : "bg-surface ring-overlay/[0.06] hover:bg-overlay/[0.03]"
                      }`}
                    >
                      <div className="flex items-start gap-2">
                        <div className="min-w-0 flex-1">
                          <div className="text-[12.5px] font-semibold text-text-primary truncate">
                            {artifact.title ?? artifact.filename ?? "Untitled artifact"}
                          </div>
                          <div className="mt-1 flex items-center gap-2 min-w-0">
                            <KindBadge kind={kind} />
                            <span className="text-[11px] text-text-tertiary truncate">
                              {agentLabel(artifact, roster)}
                            </span>
                          </div>
                        </div>
                        <span className="text-[10.5px] text-text-tertiary shrink-0">
                          {timeHint(artifact.createdAt)}
                        </span>
                      </div>
                    </button>
                  );
                })}
              </div>
            )}
          </div>
        </aside>

        <section className="flex-1 min-w-0 bg-bg-canvas min-h-0">
          {activeArtifact ? (
            <div className="h-full flex flex-col min-h-0">
              <div className="h-12 shrink-0 px-4 border-b border-overlay/[0.06] flex items-center gap-3">
                <div className="min-w-0 flex-1">
                  <div className="text-[13px] font-semibold text-text-primary truncate">
                    {activeArtifact.title ?? activeArtifact.filename ?? "Untitled artifact"}
                  </div>
                  <div className="text-[11px] text-text-tertiary truncate">
                    {agentLabel(activeArtifact, roster)} · {timeHint(activeArtifact.createdAt)}
                  </div>
                </div>
                <KindBadge kind={activeKind} />
                {canPreviewToggle && <ViewToggle view={view} onChange={setView} />}
              </div>
              <div className="flex-1 min-h-0">
                <ArtifactBody artifact={activeArtifact} view={canPreviewToggle ? view : "code"} />
              </div>
            </div>
          ) : (
            <div className="h-full grid place-items-center text-center px-6">
              <div className="max-w-sm">
                <FileCode2 className="w-6 h-6 mx-auto mb-3 text-text-tertiary" />
                <div className="text-[13px] text-text-primary">Select an artifact to inspect it.</div>
                <div className="mt-1 text-[12px] text-text-tertiary">
                  Saved HTML and SVG artifacts preview in a sandboxed frame. Other kinds show code.
                </div>
              </div>
            </div>
          )}
        </section>
      </div>
    </main>
  );
}
