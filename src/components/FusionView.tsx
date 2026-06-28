import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowUp, Waypoints, CheckCircle2, AlertTriangle } from "lucide-react";
import { ipc, useFusionStage } from "../ipc";
import type { AgentDefinition, FusionRun, FusionPanelResponse } from "../ipc";
import type { RoutingTarget } from "./RoutingPicker";

// ---------------------------------------------------------------------------
// FusionView — the orchestrator center pane. Drives the REAL M4.3 backend:
// `fusion.run` fans a prompt out to a PANEL of workspace agents → asks a JUDGE
// model for a structured analysis → SYNTHESIZES one final answer, persisting
// `fusion_run` + `fusion_panel_response`. After a run resolves we re-read the
// persisted rows via `fusion.get` and render REAL data only.
//
// HONESTY NOTE — this repo forbids fabricated data:
//  - Panel answers come from persisted `fusion_panel_response` rows. A member
//    that couldn't run shows `status = "error"` with the real error string in
//    `answer` (rendered as an error note, NOT a fake answer).
//  - The judge analysis is parsed defensively from the run's `judgeAnalysis`
//    (a JSON STRING over the wire). On parse failure / `parseError: true` we
//    show an honest fallback — we never invent consensus/insights.
//  - The synthesized answer is the run's real `synthesized`, which may be the
//    honest "[synthesis failed: …]" marker in a no-API-key dev environment —
//    shown truthfully.
//  - The panel here is APPROXIMATED from the roster's chat agents (the same
//    derivation the M4.3 backend does from the workspace); the authoritative
//    membership is whatever the backend persisted as panel responses.
// ---------------------------------------------------------------------------

interface FusionViewProps {
  /** The orchestrator workspace_agent id (= fusion.run orchestratorId). */
  instanceId: string;
  /** Orchestrator identity (name/color/model). */
  def: AgentDefinition;
  /** Resolves a panel member instanceId → name/color. */
  roster: RoutingTarget[];
}

// One fusion turn in the transcript.
interface FusionTurn {
  id: string;
  prompt: string;
  status: "running" | "done" | "error";
  run: FusionRun | null;
  responses: FusionPanelResponse[];
  error?: string;
}

type Stage = "panel" | "judge" | "synthesize";

const STAGE_LABEL: Record<Stage, string> = {
  panel: "Panel…",
  judge: "Judging…",
  synthesize: "Synthesizing…",
};

// Fresh UUID per turn — generated BEFORE the setState updater runs so the
// updater stays pure (React 19 StrictMode double-invokes updaters).
const makeId = (): string => crypto.randomUUID();

// ── Judge-analysis parsing (defensive — `judgeAnalysis` is a JSON STRING) ────

interface JudgeAnalysis {
  consensus?: string;
  disagreements?: string[];
  insights?: string[];
  blindSpots?: string[];
}

type ParsedJudge =
  | { ok: true; analysis: JudgeAnalysis }
  | { ok: false; fallback: string };

function toStrArray(v: unknown): string[] | undefined {
  if (!Array.isArray(v)) return undefined;
  const arr = v.filter((x): x is string => typeof x === "string" && x.length > 0);
  return arr.length > 0 ? arr : undefined;
}

// Parse the run's `judgeAnalysis`. It crosses the wire as a JSON string (M4.3
// stores it as text). We JSON.parse it; on failure or `parseError: true` we
// return an HONEST fallback (the raw/error text, or "Judge analysis
// unavailable") — never inventing structured fields.
function parseJudge(raw: unknown): ParsedJudge {
  let obj: unknown;
  if (typeof raw === "string") {
    if (raw.trim().length === 0) return { ok: false, fallback: "Judge analysis unavailable" };
    try {
      obj = JSON.parse(raw);
    } catch {
      return { ok: false, fallback: raw };
    }
  } else if (raw && typeof raw === "object") {
    // Defensive: tolerate a value that arrives already parsed.
    obj = raw;
  } else {
    return { ok: false, fallback: "Judge analysis unavailable" };
  }

  if (!obj || typeof obj !== "object") {
    return { ok: false, fallback: "Judge analysis unavailable" };
  }
  const o = obj as Record<string, unknown>;
  if (o.parseError === true) {
    const text =
      typeof o.raw === "string" && o.raw.length > 0
        ? o.raw
        : typeof o.error === "string" && o.error.length > 0
          ? o.error
          : "Judge analysis unavailable";
    return { ok: false, fallback: text };
  }
  return {
    ok: true,
    analysis: {
      consensus: typeof o.consensus === "string" && o.consensus.length > 0 ? o.consensus : undefined,
      disagreements: toStrArray(o.disagreements),
      insights: toStrArray(o.insights),
      blindSpots: toStrArray(o.blindSpots),
    },
  };
}

export function FusionView({ instanceId, def, roster }: FusionViewProps) {
  const [turns, setTurns] = useState<FusionTurn[]>([]);
  const [draft, setDraft] = useState("");
  const [running, setRunning] = useState(false);
  // Latest fusion stage of the in-flight run. Only one run is in flight at a
  // time, so we track the most-recent stage WITHOUT runId correlation — an
  // honest approximation good enough to drive the small progress indicator.
  const [stage, setStage] = useState<Stage | null>(null);

  // Derived panel = roster chat agents excluding self, capped at 8 (mirrors the
  // M4.3 backend derivation; the authoritative count is the persisted responses).
  // Memoised so stage/turn re-renders don't re-filter the roster each time.
  const panelCount = useMemo(
    () =>
      Math.min(8, roster.filter((t) => t.type === "chat" && t.instanceId !== instanceId).length),
    [roster, instanceId],
  );
  // Cost multiplier = panel calls + judge + synth (honest derived estimate).
  const costMult = panelCount + 2;

  // Guard setState against a run resolving after unmount.
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  // Auto-scroll to the newest turn whenever the list grows/changes. Deliberately
  // NOT keyed on `stage` — a stage label change is an in-place text update and
  // shouldn't yank a user who scrolled up to read an earlier turn.
  const scrollRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [turns]);

  // Subscribe to fusion stage events to drive the in-flight progress indicator.
  useFusionStage((e) => {
    if (mounted.current) setStage(e.stage);
  });

  async function handleSend() {
    const prompt = draft.trim();
    if (prompt.length === 0 || running) return;

    const turnId = makeId();
    setTurns((prev) => [
      ...prev,
      { id: turnId, prompt, status: "running", run: null, responses: [] },
    ]);
    setDraft("");
    setRunning(true);
    setStage("panel"); // optimistic initial stage; real stages arrive via events

    try {
      const run = await ipc.fusion.run({ orchestratorId: instanceId, prompt });
      // Re-read the persisted run + its panel responses to render REAL data.
      const { run: full, responses } = await ipc.fusion.get({ runId: run.id });
      if (mounted.current) {
        setTurns((prev) =>
          prev.map((t) =>
            t.id === turnId ? { ...t, status: "done", run: full, responses } : t,
          ),
        );
      }
    } catch (err) {
      if (import.meta.env.DEV) {
        console.error("FusionView: fusion run/get failed", err);
      }
      const msg = err instanceof Error ? err.message : String(err);
      if (mounted.current) {
        setTurns((prev) =>
          prev.map((t) => (t.id === turnId ? { ...t, status: "error", error: msg } : t)),
        );
      }
    } finally {
      if (mounted.current) {
        setRunning(false);
        setStage(null);
      }
    }
  }

  return (
    <main className="flex-1 flex flex-col min-w-0 bg-surface">
      {/* Transcript */}
      <div ref={scrollRef} className="flex-1 overflow-y-auto scroll-thin px-8 py-6">
        {/* Slightly wider than ChatView's 720px — pipeline cards need the room. */}
        <div className="max-w-[760px] mx-auto space-y-6">
          {turns.length === 0 ? (
            <div className="text-center text-[13px] text-text-tertiary pt-16">
              Ask the orchestrator to fuse your workspace panel into one answer.
            </div>
          ) : (
            turns.map((turn) => (
              <TurnView
                key={turn.id}
                turn={turn}
                // The live stage only belongs to the in-flight turn.
                stage={turn.status === "running" ? stage : null}
                def={def}
                roster={roster}
              />
            ))
          )}
        </div>
      </div>

      {/* Composer */}
      <div className="border-t border-overlay/[0.07] px-8 py-3 bg-surface shrink-0">
        <div className="max-w-[760px] mx-auto">
          {/* Honest derived cost hint. */}
          <div className="mb-2 text-[11px] text-text-tertiary">
            <span className="font-medium text-text-secondary">Fusion: on</span> · {panelCount}{" "}
            {panelCount === 1 ? "agent" : "agents"} → judge → synthesize · ~{costMult}× cost
          </div>
          <div className="rounded-2xl ring-1 ring-overlay/[0.1] bg-fill-softer focus-within:ring-accent/50 px-3 pt-2.5 pb-2">
            <div className="flex items-end gap-2">
              <textarea
                rows={1}
                value={draft}
                disabled={running}
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !e.shiftKey) {
                    e.preventDefault();
                    void handleSend();
                  }
                }}
                placeholder={`Ask ${def.name}…  (fuses a panel of workspace agents into one answer)`}
                className="flex-1 bg-transparent outline-none resize-none text-[13.5px] leading-relaxed placeholder:text-text-tertiary py-1 max-h-40 disabled:opacity-50"
              />
              <button
                onClick={() => void handleSend()}
                disabled={running || draft.trim().length === 0}
                className="w-8 h-8 rounded-full bg-accent text-white grid place-items-center shrink-0 hover:brightness-105 disabled:opacity-40 disabled:hover:brightness-100"
              >
                <ArrowUp className="w-4 h-4" />
              </button>
            </div>
          </div>
        </div>
      </div>
    </main>
  );
}

// ---------------------------------------------------------------------------
// One fusion turn: user prompt + (running indicator | error | pipeline result).
// ---------------------------------------------------------------------------

interface TurnViewProps {
  turn: FusionTurn;
  stage: Stage | null;
  def: AgentDefinition;
  roster: RoutingTarget[];
}

function TurnView({ turn, stage, def, roster }: TurnViewProps) {
  return (
    <div className="space-y-3">
      {/* User prompt bubble — right-aligned, accent fill (mirrors ChatView). */}
      <div className="flex justify-end">
        <div className="max-w-[82%] bg-accent text-white rounded-2xl rounded-tr-md px-3.5 py-2.5 text-[13.5px] leading-relaxed whitespace-pre-wrap break-words">
          {turn.prompt}
        </div>
      </div>

      {turn.status === "running" && (
        <div className="rounded-2xl ring-hair bg-fill-softer px-3.5 py-3 flex items-center gap-2.5">
          <span className="w-2 h-2 rounded-full bg-accent animate-pulse shrink-0" />
          <span className="text-[12.5px] text-text-secondary">
            Running fusion…
            {stage ? <span className="text-text-tertiary"> · {STAGE_LABEL[stage]}</span> : null}
          </span>
        </div>
      )}

      {turn.status === "error" && (
        <div className="rounded-2xl ring-1 ring-danger/30 bg-danger-soft px-3.5 py-2.5 text-[12.5px] text-danger flex items-start gap-2">
          <AlertTriangle className="w-4 h-4 shrink-0 mt-0.5" />
          <span className="break-words">Fusion failed: {turn.error}</span>
        </div>
      )}

      {turn.status === "done" && turn.run && (
        <CompletedTurn run={turn.run} responses={turn.responses} def={def} roster={roster} />
      )}
    </div>
  );
}

// ── A completed pipeline turn: panel + judge card, then the synth bubble. ────

interface CompletedTurnProps {
  run: FusionRun;
  responses: FusionPanelResponse[];
  def: AgentDefinition;
  roster: RoutingTarget[];
}

function CompletedTurn({ run, responses, def, roster }: CompletedTurnProps) {
  const judge = parseJudge(run.judgeAnalysis);
  const synthesized = run.synthesized && run.synthesized.length > 0 ? run.synthesized : null;
  const n = responses.length;

  return (
    <div className="space-y-3">
      {/* Pipeline card — panel + judge. */}
      <div className="ring-hair bg-fill-softer rounded-2xl overflow-hidden">
        {/* Panel section */}
        <div className="p-3.5">
          <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-2">
            Panel · {n} {n === 1 ? "agent" : "agents"}
          </div>
          {responses.length === 0 ? (
            <div className="text-[11.5px] text-text-tertiary">
              No panel members were available to answer.
            </div>
          ) : (
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
              {responses.map((r) => (
                <PanelMemberCard key={r.id} resp={r} roster={roster} />
              ))}
            </div>
          )}
        </div>

        {/* Judge section */}
        <div className="border-t border-overlay/[0.06] p-3.5">
          <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-2">
            Judge analysis
          </div>
          {judge.ok ? (
            <JudgeRows analysis={judge.analysis} />
          ) : (
            <div className="text-[11.5px] text-text-muted break-words whitespace-pre-wrap">
              {judge.fallback}
            </div>
          )}
        </div>
      </div>

      {/* Synthesized answer bubble — orchestrator avatar + fused tag + answer. */}
      <div className="flex gap-2.5">
        <div
          className="w-6 h-6 rounded-[7px] text-white grid place-items-center ring-hair shrink-0 mt-0.5"
          style={{ backgroundColor: def.color ?? "#6e6e73" }}
        >
          <Waypoints className="w-[14px] h-[14px]" />
        </div>
        <div className="max-w-[82%] space-y-1.5">
          <div className="text-[10.5px] font-medium text-text-tertiary">
            fused · {n} {n === 1 ? "agent" : "agents"} + judge
          </div>
          {synthesized ? (
            <div className="bg-fill-soft rounded-2xl rounded-tl-md px-3.5 py-2.5 text-[13.5px] leading-relaxed whitespace-pre-wrap break-words">
              {synthesized}
            </div>
          ) : (
            <div className="bg-fill-soft rounded-2xl rounded-tl-md px-3.5 py-2.5 text-[12.5px] text-text-tertiary">
              No synthesized answer.
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

// ── One panel member card: avatar/name + status glyph + answer/error. ────────

function PanelMemberCard({
  resp,
  roster,
}: {
  resp: FusionPanelResponse;
  roster: RoutingTarget[];
}) {
  const member = resp.instanceId
    ? roster.find((t) => t.instanceId === resp.instanceId)
    : undefined;
  const name = member?.name ?? resp.instanceId ?? "unknown";
  const color = member?.color ?? "#6e6e73";
  const isError = resp.status === "error";
  const isDone = resp.status === "done";

  return (
    <div className="rounded-xl ring-hair bg-surface p-2.5">
      <div className="flex items-center gap-1.5 mb-1.5">
        <span
          className="w-4 h-4 rounded-[5px] text-white grid place-items-center text-[9px] font-bold shrink-0"
          style={{ backgroundColor: color }}
        >
          {name[0]?.toUpperCase() ?? "A"}
        </span>
        <span className="text-[11.5px] font-medium truncate flex-1 min-w-0">{name}</span>
        {isDone ? (
          <CheckCircle2 className="w-3.5 h-3.5 text-success shrink-0" />
        ) : isError ? (
          <AlertTriangle className="w-3.5 h-3.5 text-warning shrink-0" />
        ) : (
          <span className="w-2 h-2 rounded-full bg-accent animate-pulse shrink-0" />
        )}
      </div>
      {isError ? (
        // `answer` holds the honest error string on an error row — clearly not
        // an answer.
        <div className="text-[11px] text-danger break-words whitespace-pre-wrap">
          Couldn't run: {resp.answer ?? "unknown error"}
        </div>
      ) : resp.answer ? (
        <div className="text-[11.5px] text-text-secondary break-words whitespace-pre-wrap line-clamp-6">
          {resp.answer}
        </div>
      ) : (
        <div className="text-[11px] text-text-tertiary">No answer recorded.</div>
      )}
    </div>
  );
}

// ── Judge rows: only render fields that are present/non-empty. ───────────────

function JudgeRows({ analysis }: { analysis: JudgeAnalysis }) {
  const hasAny =
    analysis.consensus != null ||
    (analysis.disagreements?.length ?? 0) > 0 ||
    (analysis.insights?.length ?? 0) > 0 ||
    (analysis.blindSpots?.length ?? 0) > 0;

  if (!hasAny) {
    return (
      <div className="text-[11.5px] text-text-muted">Judge returned no analysis fields.</div>
    );
  }

  return (
    <div className="space-y-2">
      {analysis.consensus != null && (
        <JudgeRow label="consensus" labelClass="bg-success/12 text-success">
          <span className="whitespace-pre-wrap break-words">{analysis.consensus}</span>
        </JudgeRow>
      )}
      {analysis.disagreements != null && (
        <JudgeRow label="disagree" labelClass="bg-warning/15 text-warning">
          <JudgeList items={analysis.disagreements} />
        </JudgeRow>
      )}
      {analysis.insights != null && (
        <JudgeRow label="insight" labelClass="bg-accent/12 text-accent-hover">
          <JudgeList items={analysis.insights} />
        </JudgeRow>
      )}
      {analysis.blindSpots != null && (
        <JudgeRow label="blind spot" labelClass="bg-danger/12 text-danger">
          <JudgeList items={analysis.blindSpots} />
        </JudgeRow>
      )}
    </div>
  );
}

function JudgeRow({
  label,
  labelClass,
  children,
}: {
  label: string;
  labelClass: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex gap-2 text-[11.5px]">
      <span
        className={`shrink-0 h-fit rounded px-1.5 py-0.5 text-[9.5px] font-bold uppercase tracking-wide ${labelClass}`}
      >
        {label}
      </span>
      <div className="flex-1 min-w-0 text-text-body">{children}</div>
    </div>
  );
}

function JudgeList({ items }: { items: string[] }) {
  return (
    <ul className="space-y-0.5">
      {items.map((item, i) => (
        <li key={i} className="whitespace-pre-wrap break-words">
          • {item}
        </li>
      ))}
    </ul>
  );
}
