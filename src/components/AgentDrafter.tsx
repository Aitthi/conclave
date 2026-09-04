// AI agent & team drafter overlay.
//
// Spec:  docs/superpowers/specs/2026-09-04-ai-agent-team-drafter-design.md
// Canon: design/screens/agent-drafter.tsx @ 8385a5e (Arta) — copy deck, flow
//        states and layout come from there. The canon is authored against the
//        design-host token set; this file uses the app's own tokens for the
//        same roles (fill → fill-soft, border → overlay/[0.06], canvas →
//        bg-canvas, live → success, waiting → warning), and the panel shell is
//        Builder.tsx's so the two overlays are visually identical.
//
// Agent mode drafts ONE definition and hands it to the Builder (nothing is
// saved until the user presses Create agent). Team mode shows an editable
// preview and applies it through the existing commands (spec D6) — there is no
// transactional team command and no cancel control (D9).

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  AlertCircle,
  Check,
  ChevronDown,
  CircleDashed,
  LoaderCircle,
  RotateCcw,
  Sparkles,
  X,
} from "lucide-react";
import { ipc } from "../ipc";
import type {
  AgentDefinition,
  DraftAgent,
  DraftLevel,
  DraftMode,
  DraftResponse,
  Role,
} from "../ipc";
import { CLAUDE_MODELS, CODEX_MODELS } from "../lib/modelCatalogue";
import { LEVELS } from "../lib/positions";
import { applyTeamDraft, type ApplyStatus } from "../lib/applyTeamDraft";
import { fixtureScenario } from "../fixtures/mode";

// ── Props ────────────────────────────────────────────────────────────────────

export interface AgentDrafterProps {
  mode: DraftMode;
  workspaceId?: string;
  workspaceName?: string;
  onClose: () => void;
  /** Agent mode: hand the drafted, id-less definition to the Builder. */
  onDraftAgent: (def: AgentDefinition, draftedBy: string) => void;
  /** Team mode: the apply run finished and the roster/Library must re-fetch. */
  onTeamApplied: () => void;
  /** No CLI definition exists yet — send the user to the Builder to make one. */
  onOpenBuilder: () => void;
}

type Phase = "idle" | "running" | "error" | "preview" | "applying" | "done";

/** Sample brief used ONLY in fixture mode, so `pnpm uishot drafter` has
 *  something to render. Fixed literal (fixture rule: no Date.now(), no
 *  randomness). */
const SAMPLE_BRIEF =
  "Port the billing service from Node to Rust, module by module, with tests for each module and a reviewer who checks the ported behaviour against the old service.";

// ── Small pieces (canon: Avatar / FieldLabel / Field) ────────────────────────

function Avatar({ name, color, size = 7 }: { name: string; color?: string; size?: 6 | 7 }) {
  const box = size === 6 ? "w-6 h-6 rounded-[7px] text-[11px]" : "w-7 h-7 rounded-[8px] text-[12px]";
  return (
    <span
      className={`grid shrink-0 place-items-center font-bold text-white ${box}`}
      style={{ backgroundColor: color || "#6e6e73" }}
      aria-hidden="true"
    >
      {(name.trim()[0] ?? "?").toUpperCase()}
    </span>
  );
}

function FieldLabel({ children }: { children: React.ReactNode }) {
  return (
    <div className="mb-1.5 text-[10px] font-bold uppercase tracking-[0.08em] text-text-tertiary">
      {children}
    </div>
  );
}

/** Compact native select styled to the app's fill controls (canon `Field`). */
function Field({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string;
  options: { value: string; text: string }[];
  onChange: (v: string) => void;
}) {
  return (
    <div className="relative">
      <select
        aria-label={label}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="h-7 w-full appearance-none truncate rounded-md bg-fill-soft pl-2 pr-6 text-[11.5px] text-text-primary transition-colors hover:bg-overlay/[0.08] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.text}
          </option>
        ))}
      </select>
      <ChevronDown className="pointer-events-none absolute right-1.5 top-1/2 h-3 w-3 -translate-y-1/2 text-text-muted" />
    </div>
  );
}

function GhostButton({
  children,
  onClick,
}: {
  children: React.ReactNode;
  onClick?: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="rounded-lg px-3.5 py-1.5 text-[12.5px] font-medium text-text-secondary transition-colors hover:bg-overlay/[0.05] hover:text-text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
    >
      {children}
    </button>
  );
}

/** `busy` keeps the accent fill (work is running); a plain `disabled` drops to
 *  the fill surface so an unavailable primary never reads as pressable
 *  (Detoro ruling on the canon). */
function PrimaryButton({
  children,
  onClick,
  disabled,
  busy,
  icon,
}: {
  children: React.ReactNode;
  onClick?: () => void;
  disabled?: boolean;
  busy?: boolean;
  icon: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled || busy}
      className={`flex items-center gap-1.5 rounded-lg px-4 py-1.5 text-[12.5px] font-semibold transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
        busy
          ? "cursor-wait bg-accent/70 text-white"
          : "bg-accent text-white hover:bg-accent-hover disabled:cursor-not-allowed disabled:bg-fill-soft disabled:text-text-tertiary"
      }`}
    >
      {icon}
      {children}
    </button>
  );
}

/** Builder's overlay shell (backdrop + centred panel + header + scroll body +
 *  footer), widened per flow: 620px while briefing, 1020px once the preview
 *  table appears. */
function Panel({
  width,
  title,
  chip,
  onClose,
  children,
  footer,
}: {
  width: string;
  title: string;
  chip: string;
  onClose: () => void;
  children: React.ReactNode;
  footer: React.ReactNode;
}) {
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 px-6">
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className={`${width} max-w-full max-h-[92vh] bg-surface rounded-2xl shadow-2xl flex flex-col overflow-hidden ring-1 ring-overlay/[0.08]`}
      >
        <div className="h-11 flex items-center justify-between px-4 border-b border-overlay/[0.06] shrink-0">
          <div className="flex items-center gap-2">
            <Sparkles className="w-4 h-4 text-accent" />
            <span className="text-[13px] font-semibold tracking-tight">{title}</span>
            <span className="text-[10px] font-medium text-text-muted bg-overlay/[0.04] px-1.5 py-px rounded-md">
              {chip}
            </span>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close drafter"
            className="w-7 h-7 grid place-items-center rounded-md hover:bg-overlay/[0.05] text-text-secondary"
          >
            <X className="w-[15px] h-[15px]" />
          </button>
        </div>
        <div className="flex-1 overflow-y-auto px-5 py-4 min-h-0">{children}</div>
        <div className="flex items-center gap-2 px-5 py-2.5 border-t border-overlay/[0.06] shrink-0">
          {footer}
        </div>
      </div>
    </div>
  );
}

// ── Draft → Builder ──────────────────────────────────────────────────────────

/**
 * The id-less `AgentDefinition` the Builder opens pre-filled. `skillIds` is the
 * union of the role's bundle and the drafted extras for the same reason as
 * `applyTeamDraft`: the Builder seeds its checkbox state from
 * `initialDef.skillIds` and does not re-copy a pre-filled role's bundle.
 */
export function draftToInitialDef(a: DraftAgent, roleSkillIds: string[]): AgentDefinition {
  return {
    id: "",
    name: a.name ?? "",
    color: a.color,
    type: "cli",
    cliKind: a.cliKind,
    model: a.model,
    roleId: a.roleId,
    skillIds: Array.from(new Set([...roleSkillIds, ...a.skillIds])),
    defaultLevel: a.defaultLevel ?? null,
    // Builder re-sends its own fixed constants on save; these two need a value
    // so the pre-fill is a faithful CLI definition. permissionMode "auto" is
    // what every hand-built claude-code/codex definition stores
    // (Builder.tsx:201-203) — leaving it unset would open the Builder showing
    // "auto" over a definition that would save NULL.
    harnessMode: "central",
    permissionMode: "auto",
    createdAt: "",
  };
}

// ── Component ────────────────────────────────────────────────────────────────

export function AgentDrafter({
  mode,
  workspaceId,
  workspaceName,
  onClose,
  onDraftAgent,
  onTeamApplied,
  onOpenBuilder,
}: AgentDrafterProps) {
  const [phase, setPhase] = useState<Phase>("idle");
  const [brief, setBrief] = useState("");
  const [drafters, setDrafters] = useState<AgentDefinition[]>([]);
  /** Every definition, not just the CLI ones: a reuse row names an existing
   *  definition and must show ITS name, colour, role and model — the draft
   *  carries only the id (spec: existingAgentDefId sets no other field). */
  const [defsById, setDefsById] = useState<Map<string, AgentDefinition>>(new Map());
  const [drafterDefId, setDrafterDefId] = useState("");
  const [roles, setRoles] = useState<Role[]>([]);
  const [draft, setDraft] = useState<DraftResponse | null>(null);
  const [error, setError] = useState<{ message: string; detail?: string } | null>(null);
  const [elapsed, setElapsed] = useState(0);
  const [progress, setProgress] = useState<Record<string, { status: ApplyStatus; message?: string }>>({});
  const [applyError, setApplyError] = useState<{ name: string; created: number } | null>(null);
  const [createdCount, setCreatedCount] = useState(0);

  const target = workspaceName ?? "this workspace";
  const drafter = drafters.find((d) => d.id === drafterDefId);
  const drafterName = drafter?.name ?? "the drafter";
  const noDrafter = drafters.length === 0;

  // Catalogue for the preview selects. Roles double as the role-bundle source
  // for `draftToInitialDef`.
  useEffect(() => {
    let live = true;
    ipc.agentDef
      .list()
      .then((defs) => {
        if (!live) return;
        setDefsById(new Map(defs.map((d) => [d.id, d])));
        const cliDefs = defs.filter(
          (d) => d.type === "cli" && (d.cliKind === "claude-code" || d.cliKind === "codex"),
        );
        setDrafters(cliDefs);
        setDrafterDefId((prev) => prev || (cliDefs[0]?.id ?? ""));
      })
      .catch(() => setDrafters([]));
    ipc.role
      .list()
      .then((r) => live && setRoles(r))
      .catch(() => setRoles([]));
    return () => {
      live = false;
    };
  }, []);

  // Elapsed seconds while the model runs (D9: no cancel, so the only feedback
  // is the drafter's name and the clock).
  useEffect(() => {
    if (phase !== "running") return;
    setElapsed(0);
    const id = window.setInterval(() => setElapsed((s) => s + 1), 1000);
    return () => window.clearInterval(id);
  }, [phase]);

  const roleSkillIdsById = useMemo(
    () => new Map(roles.map((r) => [r.id, r.skillIds ?? []])),
    [roles],
  );

  const run = useCallback(
    async (theBrief: string, defId: string) => {
      if (!theBrief.trim() || !defId) return;
      setPhase("running");
      setError(null);
      try {
        const res = await ipc.draft.agents({
          mode,
          brief: theBrief.trim(),
          drafterDefId: defId,
          workspaceId,
        });
        if (mode === "agent") {
          const a = res.agents[0];
          if (!a) {
            setError({ message: "The drafter returned no agent" });
            setPhase("error");
            return;
          }
          // A proposed role must exist before the Builder can select it; the
          // Builder has no UI for an unsaved role (spec D5 resolves newRole
          // through role.save).
          let roleId = a.roleId;
          let bundle = roleId ? (roleSkillIdsById.get(roleId) ?? []) : [];
          if (a.newRole) {
            const role = await ipc.role.save({
              name: a.newRole.name,
              description: a.newRole.description,
              skillIds: a.newRole.skillIds,
            });
            roleId = role.id;
            bundle = role.skillIds ?? [];
          }
          if (a.existingAgentDefId) {
            const defs = await ipc.agentDef.list();
            const existing = defs.find((d) => d.id === a.existingAgentDefId);
            if (existing) {
              onDraftAgent(existing, res.drafter.defId === defId ? drafterName : drafterName);
              return;
            }
          }
          onDraftAgent(draftToInitialDef({ ...a, roleId }, bundle), drafterName);
          return;
        }
        setDraft(res);
        setPhase("preview");
      } catch (e) {
        const raw = e instanceof Error ? e.message : String(e);
        // The engine returns "draft.<field>: <reason>" for a validation
        // failure; show the human sentence first and the identifier below.
        const validation = /^draft\.[^\s:]+:\s*(.*)$/.exec(raw);
        setError(
          validation
            ? { message: "The draft did not match the catalogue", detail: raw }
            : { message: raw },
        );
        setPhase("error");
      }
    },
    [mode, workspaceId, roleSkillIdsById, onDraftAgent, drafterName],
  );

  // Fixture mode only (DEV): auto-run once so `pnpm uishot drafter` renders a
  // real preview instead of an empty brief form.
  const autoRan = useRef(false);
  useEffect(() => {
    if (autoRan.current || !fixtureScenario()) return;
    if (!drafterDefId) return;
    autoRan.current = true;
    setBrief(SAMPLE_BRIEF);
    void run(SAMPLE_BRIEF, drafterDefId);
  }, [drafterDefId, run]);

  // ── Preview editing ──────────────────────────────────────────────────────
  function patchAgent(key: string, patch: Partial<DraftAgent>) {
    setDraft((d) =>
      d ? { ...d, agents: d.agents.map((a) => (a.key === key ? { ...a, ...patch } : a)) } : d,
    );
  }
  function patchPosition(key: string, patch: { level?: DraftLevel; supervisorKey?: string | null }) {
    setDraft((d) =>
      d ? { ...d, positions: d.positions.map((p) => (p.key === key ? { ...p, ...patch } : p)) } : d,
    );
  }

  async function handleApply() {
    if (!draft || !workspaceId) return;
    setPhase("applying");
    setProgress({});
    setApplyError(null);
    const result = await applyTeamDraft(draft, workspaceId, (p) =>
      setProgress((prev) => ({ ...prev, [p.key]: { status: p.status, message: p.message } })),
    );
    setCreatedCount(result.created);
    if (result.failedKey) {
      const failed = draft.agents.find((a) => a.key === result.failedKey);
      setApplyError({
        name: failed ? displayName(failed) : result.failedKey,
        created: result.created,
      });
    }
    setPhase("done");
  }

  const levelName = (id: string) => LEVELS.find((l) => l.id === id)?.name ?? id;
  /** A reuse row carries only `existingAgentDefId`, so its display identity
   *  comes from the definition it points at. */
  const reusedDef = (a: DraftAgent) =>
    a.existingAgentDefId ? defsById.get(a.existingAgentDefId) : undefined;
  const displayName = (a: DraftAgent) => a.name ?? reusedDef(a)?.name ?? a.key;
  const displayColor = (a: DraftAgent) => a.color ?? reusedDef(a)?.color;
  const displayModel = (a: DraftAgent) => a.model ?? reusedDef(a)?.model;
  const modelsFor = (cliKind?: string) => (cliKind === "codex" ? CODEX_MODELS : CLAUDE_MODELS);
  const roleName = (a: DraftAgent) => {
    if (a.newRole) return `New: ${a.newRole.name}`;
    const id = a.roleId ?? reusedDef(a)?.roleId;
    return roles.find((r) => r.id === id)?.name ?? reusedDef(a)?.role ?? id ?? "—";
  };

  const title = mode === "agent" ? "Draft an agent" : "Build a team";
  const briefChip = mode === "agent" ? "opens in Builder" : `applies to ${target}`;

  // ── Brief / waiting / error ──────────────────────────────────────────────
  if (phase === "idle" || phase === "running" || phase === "error") {
    const waiting = phase === "running";
    return (
      <Panel
        width="w-[620px]"
        title={title}
        chip={briefChip}
        onClose={onClose}
        footer={
          <>
            <span className="mr-auto text-[10.5px] text-text-tertiary">
              {waiting
                ? "Closing this panel does not stop the run."
                : "The drafter answers within 120 s or times out."}
            </span>
            <GhostButton onClick={onClose}>Cancel</GhostButton>
            {phase === "error" ? (
              <PrimaryButton
                icon={<RotateCcw className="w-3.5 h-3.5" />}
                onClick={() => void run(brief, drafterDefId)}
              >
                Retry
              </PrimaryButton>
            ) : (
              <PrimaryButton
                icon={
                  waiting ? (
                    <LoaderCircle className="w-3.5 h-3.5 animate-spin motion-reduce:animate-none" />
                  ) : (
                    <Sparkles className="w-3.5 h-3.5" />
                  )
                }
                busy={waiting}
                disabled={noDrafter || brief.trim().length === 0}
                onClick={() => void run(brief, drafterDefId)}
              >
                {waiting ? "Drafting…" : "Draft"}
              </PrimaryButton>
            )}
          </>
        }
      >
        <div className="space-y-4">
          <section>
            <FieldLabel>Brief</FieldLabel>
            <textarea
              aria-label="Brief"
              value={brief}
              readOnly={waiting}
              onChange={(e) => setBrief(e.target.value)}
              rows={4}
              placeholder="Describe the job. Example: a team to port our billing service from Node to Rust with tests and a reviewer."
              className={`w-full resize-none rounded-lg bg-fill-soft px-3 py-2.5 text-[12.5px] leading-relaxed text-text-primary placeholder:text-text-tertiary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                waiting ? "text-text-secondary" : ""
              }`}
            />
            <p className="mt-1.5 px-0.5 text-[10.5px] text-text-tertiary">
              {mode === "agent"
                ? "One agent is drafted and opened in the Builder. Nothing is saved until you press Create agent."
                : `Agents, levels and reporting lines are proposed for ${target}. You review and edit every row before anything is created.`}
            </p>
          </section>

          <section>
            <FieldLabel>Drafter</FieldLabel>
            {noDrafter ? (
              <>
                <div
                  className="flex items-center gap-2.5 rounded-lg bg-warning/[0.12] px-3 py-2.5 text-[12px] font-semibold text-warning"
                  role="alert"
                >
                  <AlertCircle className="w-4 h-4 shrink-0" />
                  <span className="min-w-0 flex-1">
                    Configure a Claude Code or Codex agent first
                  </span>
                  <button
                    type="button"
                    onClick={onOpenBuilder}
                    className="shrink-0 rounded-md bg-warning px-2.5 py-1 text-[11.5px] font-semibold text-bg-canvas transition-[filter] hover:brightness-110 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
                  >
                    Open Builder
                  </button>
                </div>
                <p className="mt-1.5 px-0.5 text-[10.5px] leading-relaxed text-text-tertiary">
                  Drafting runs through one of your CLI agent definitions and reuses its model,
                  environment and credentials.
                </p>
              </>
            ) : (
              <div className="space-y-1">
                {drafters.map((candidate) => {
                  const selected = candidate.id === drafterDefId;
                  return (
                    <button
                      key={candidate.id}
                      type="button"
                      disabled={waiting}
                      aria-pressed={selected}
                      onClick={() => setDrafterDefId(candidate.id)}
                      className={`flex w-full items-center gap-2.5 rounded-lg px-2 py-1.5 text-left transition-colors disabled:cursor-not-allowed focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                        selected ? "bg-accent/[0.12]" : "hover:bg-overlay/[0.05]"
                      } ${waiting && !selected ? "opacity-45" : ""}`}
                    >
                      <Avatar name={candidate.name} color={candidate.color} size={6} />
                      <span className="min-w-0 flex-1 truncate text-[12.5px] font-semibold">
                        {candidate.name}
                      </span>
                      <span className="shrink-0 font-mono text-[10.5px] text-text-muted">
                        {candidate.cliKind}
                        {candidate.model ? ` · ${candidate.model}` : ""}
                      </span>
                      <span
                        className={`grid w-4 h-4 shrink-0 place-items-center rounded-full ${
                          selected ? "bg-accent text-white" : "bg-overlay/[0.10] text-transparent"
                        }`}
                        aria-hidden="true"
                      >
                        <Check className="w-2.5 h-2.5" />
                      </span>
                    </button>
                  );
                })}
              </div>
            )}
          </section>

          {waiting && (
            <section className="flex items-center gap-3 rounded-lg bg-fill-soft px-3 py-2.5">
              <LoaderCircle className="w-4 h-4 shrink-0 animate-spin text-accent motion-reduce:animate-none" />
              <div className="min-w-0 flex-1 leading-tight">
                <p className="text-[12.5px] font-semibold">Drafting with {drafterName}</p>
                <p className="mt-0.5 font-mono text-[10.5px] text-text-muted">
                  {drafter?.cliKind}
                  {drafter?.model ? ` · ${drafter.model}` : ""}
                </p>
              </div>
              <span className="shrink-0 font-mono text-[13px] tabular-nums text-text-secondary">
                {Math.floor(elapsed / 60)}:{String(elapsed % 60).padStart(2, "0")}
              </span>
            </section>
          )}

          {phase === "error" && error && (
            <section>
              <div
                className="flex items-start gap-2.5 rounded-lg bg-danger/[0.12] px-3 py-2.5 text-[12px] leading-relaxed text-danger"
                role="alert"
              >
                <AlertCircle className="mt-0.5 w-3.5 h-3.5 shrink-0" />
                <div className="min-w-0">
                  <p className="font-semibold">{error.message}</p>
                  {error.detail && <p className="mt-1 font-mono text-[10.5px]">{error.detail}</p>}
                </div>
              </div>
              <p className="mt-1.5 px-0.5 text-[10.5px] text-text-tertiary">
                Retry sends the same brief again. Nothing was created.
              </p>
            </section>
          )}
        </div>
      </Panel>
    );
  }

  // ── Preview (team mode only) ─────────────────────────────────────────────
  const COLUMNS = "grid-cols-[176px_148px_150px_92px_124px_1fr]";

  if (phase === "preview" && draft) {
    const reuseCount = draft.agents.filter((a) => a.existingAgentDefId).length;
    const newRoleNames = draft.agents.filter((a) => a.newRole).map((a) => a.newRole!.name);
    return (
      <Panel
        width="w-[1020px]"
        title="Build a team"
        chip={`applies to ${target}`}
        onClose={onClose}
        footer={
          <>
            <span className="mr-auto text-[11px] text-text-tertiary">
              {draft.agents.length} agents
              {reuseCount > 0 ? ` · ${reuseCount} reuses an existing definition` : ""} · drafted by{" "}
              {drafterName}
            </span>
            <GhostButton onClick={() => setPhase("idle")}>Back</GhostButton>
            <PrimaryButton
              icon={<Check className="w-3.5 h-3.5" />}
              disabled={!workspaceId || draft.agents.length === 0}
              onClick={() => void handleApply()}
            >
              Apply {draft.agents.length} agents
            </PrimaryButton>
          </>
        }
      >
        <div className="space-y-3.5">
          {draft.notes && (
            <section className="rounded-lg bg-fill-soft px-3 py-2.5">
              <FieldLabel>Notes from the drafter</FieldLabel>
              <p className="text-[12px] leading-relaxed text-text-secondary">{draft.notes}</p>
              <p className="mt-2 flex items-center gap-1.5 font-mono text-[10px] text-text-muted">
                <Avatar name={drafterName} color={drafter?.color} size={6} />
                {drafterName} · {draft.drafter.model}
              </p>
            </section>
          )}

          <section>
            <div
              className={`grid ${COLUMNS} gap-2 border-b border-overlay/[0.06] pb-1.5 text-[9.5px] font-bold uppercase tracking-[0.08em] text-text-tertiary`}
            >
              <span>Name</span>
              <span>Role</span>
              <span>Model</span>
              <span>Level</span>
              <span>Reports to</span>
              <span>Rationale</span>
            </div>
            <div className="divide-y divide-overlay/[0.06]">
              {draft.agents.map((a) => {
                const position = draft.positions.find((p) => p.key === a.key);
                const reuse = Boolean(a.existingAgentDefId);
                return (
                  <div key={a.key} className={`grid ${COLUMNS} items-center gap-2 py-2`}>
                    <div className="flex min-w-0 items-center gap-2">
                      <Avatar name={displayName(a)} color={displayColor(a)} size={6} />
                      {reuse ? (
                        <span className="min-w-0 flex-1 truncate text-[12px] font-semibold text-text-secondary">
                          {displayName(a)}
                        </span>
                      ) : (
                        <input
                          aria-label={`Name for ${a.key}`}
                          value={a.name ?? ""}
                          onChange={(e) => patchAgent(a.key, { name: e.target.value })}
                          className="min-w-0 flex-1 rounded-md bg-transparent px-1 py-0.5 text-[12px] font-semibold text-text-primary transition-colors hover:bg-overlay/[0.06] focus-visible:bg-overlay/[0.06] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
                        />
                      )}
                      {reuse && (
                        <span
                          className="inline-flex shrink-0 items-center gap-1 rounded-md bg-accent/[0.14] px-1.5 py-0.5 text-[10px] font-semibold text-accent"
                          title="Reuses an existing definition"
                        >
                          <Check className="w-2.5 h-2.5" />
                          Reuse
                        </span>
                      )}
                    </div>

                    {reuse || a.newRole ? (
                      <span className="truncate text-[11.5px] text-text-secondary">
                        {roleName(a)}
                      </span>
                    ) : (
                      <Field
                        label={`Role for ${a.key}`}
                        value={a.roleId ?? ""}
                        options={roles.map((r) => ({ value: r.id, text: r.name }))}
                        onChange={(roleId) => patchAgent(a.key, { roleId })}
                      />
                    )}

                    {reuse ? (
                      <span className="truncate font-mono text-[10.5px] text-text-muted">
                        {displayModel(a) ?? "—"}
                      </span>
                    ) : (
                      <Field
                        label={`Model for ${a.key}`}
                        value={a.model ?? ""}
                        options={modelsFor(a.cliKind).map((m) => ({ value: m, text: m }))}
                        onChange={(model) => patchAgent(a.key, { model })}
                      />
                    )}

                    <Field
                      label={`Level for ${a.key}`}
                      value={position?.level ?? "senior"}
                      options={LEVELS.map((l) => ({ value: l.id, text: l.name }))}
                      onChange={(level) => patchPosition(a.key, { level: level as DraftLevel })}
                    />

                    <Field
                      label={`Supervisor for ${a.key}`}
                      value={position?.supervisorKey ?? ""}
                      options={[
                        { value: "", text: "—" },
                        ...draft.agents
                          .filter((o) => o.key !== a.key)
                          .map((o) => ({ value: o.key, text: displayName(o) })),
                      ]}
                      onChange={(key) => patchPosition(a.key, { supervisorKey: key || null })}
                    />

                    <p
                      className="line-clamp-2 text-[11px] leading-snug text-text-tertiary"
                      title={a.rationale}
                    >
                      {a.rationale}
                    </p>
                  </div>
                );
              })}
            </div>
            <p className="pt-2 text-[10.5px] text-text-tertiary">
              {newRoleNames.length > 0 &&
                `${newRoleNames.join(", ")} ${newRoleNames.length === 1 ? "is" : "are"} created as a custom role. `}
              Every value here is checked against the live catalogue before it is applied.
            </p>
          </section>
        </div>
      </Panel>
    );
  }

  // ── Apply ledger ─────────────────────────────────────────────────────────
  if ((phase === "applying" || phase === "done") && draft) {
    const running = phase === "applying";
    const STEP_ORDER: ApplyStatus[] = ["created", "added", "positioned"];
    const STEP_LABEL: Record<string, string> = {
      created: "Created",
      added: "Added",
      positioned: "Positioned",
    };
    const reachedFor = (status?: ApplyStatus) => {
      switch (status) {
        case "positioned":
          return 3;
        case "added":
        case "skipped":
          return 2;
        case "created":
          return 1;
        default:
          return 0;
      }
    };
    const applied = draft.agents.filter(
      (a) => progress[a.key]?.status === "positioned",
    ).length;

    return (
      <Panel
        width="w-[1020px]"
        title="Build a team"
        chip={running ? `applying to ${target}` : `applies to ${target}`}
        onClose={onClose}
        footer={
          <>
            <span className="mr-auto text-[11px] text-text-tertiary">
              {applied} of {draft.agents.length} applied
            </span>
            {/* No "Back to preview" and no Apply-again: applyTeamDraft is
                SINGLE-SHOT — a second pass would create a second role and a
                second definition for every key already applied (ruling on
                Mellow's M1). Done/Close is the only exit. */}
            <PrimaryButton
              icon={
                running ? (
                  <LoaderCircle className="w-3.5 h-3.5 animate-spin motion-reduce:animate-none" />
                ) : (
                  <Check className="w-3.5 h-3.5" />
                )
              }
              busy={running}
              onClick={applyError ? onClose : onTeamApplied}
            >
              {running ? "Applying…" : applyError ? "Close" : "Done"}
            </PrimaryButton>
          </>
        }
      >
        <div className="space-y-3">
          <div className="divide-y divide-overlay/[0.06]">
            {draft.agents.map((a) => {
              const entry = progress[a.key];
              const failed = entry?.status === "failed";
              const reached = reachedFor(entry?.status);
              const inFlight = running && entry?.status === "pending";
              const position = draft.positions.find((p) => p.key === a.key);
              return (
                <div key={a.key} className="flex items-center gap-3 py-2.5">
                  <Avatar name={displayName(a)} color={displayColor(a)} />
                  <div className="min-w-0 w-[150px] leading-tight">
                    <div className="truncate text-[12.5px] font-semibold">{displayName(a)}</div>
                    <div className="mt-0.5 truncate text-[10.5px] text-text-muted">
                      {roleName(a)} · {position ? levelName(position.level) : "—"}
                    </div>
                  </div>
                  <div className="flex min-w-0 flex-1 items-center gap-1.5">
                    {STEP_ORDER.map((step, stepIndex) => {
                      const done = stepIndex < reached;
                      const active = inFlight && stepIndex === reached;
                      const broken = failed && stepIndex === reached;
                      return (
                        <span
                          key={step}
                          className={`inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[10.5px] font-semibold ${
                            broken
                              ? "bg-danger/[0.14] text-danger"
                              : done
                                ? "bg-success/[0.14] text-success"
                                : active
                                  ? "bg-warning/[0.14] text-warning"
                                  : "bg-overlay/[0.05] text-text-tertiary"
                          }`}
                        >
                          {broken ? (
                            <X className="w-3 h-3" />
                          ) : done ? (
                            <Check className="w-3 h-3" />
                          ) : active ? (
                            <LoaderCircle className="w-3 h-3 animate-spin motion-reduce:animate-none" />
                          ) : (
                            <CircleDashed className="w-3 h-3" />
                          )}
                          {STEP_LABEL[step]}
                        </span>
                      );
                    })}
                  </div>
                  <span
                    className={`shrink-0 text-[11px] font-semibold ${
                      failed
                        ? "text-danger"
                        : entry && entry.status !== "pending"
                          ? "text-text-secondary"
                          : "text-text-tertiary"
                    }`}
                  >
                    {failed
                      ? "Failed"
                      : entry && entry.status !== "pending"
                        ? (STEP_LABEL[entry.status] ?? "Working")
                        : "Pending"}
                  </span>
                </div>
              );
            })}
          </div>

          {running && (
            <p className="px-0.5 text-[11px] text-text-tertiary">
              Steps run in reporting order so every supervisor exists before its reports.
            </p>
          )}

          {!running && !applyError && (
            <p className="flex items-center gap-2 rounded-lg bg-success/[0.10] px-3 py-2.5 text-[12.5px] font-semibold text-success">
              <Check className="w-4 h-4 shrink-0" />
              {createdCount} agents created and added to {target}.
            </p>
          )}

          {!running && applyError && (
            <div>
              <p
                className="flex items-center gap-2 rounded-lg bg-danger/[0.12] px-3 py-2.5 text-[12.5px] font-semibold text-danger"
                role="alert"
              >
                <AlertCircle className="w-4 h-4 shrink-0" />
                Couldn&rsquo;t add {applyError.name} to {target}.
              </p>
              <p className="mt-1.5 px-0.5 text-[11px] leading-relaxed text-text-tertiary">
                {applyError.created} {applyError.created === 1 ? "agent was" : "agents were"} created
                before the failure. Created agents stay in the Library, so you can fix the cause and
                apply the rest.
              </p>
            </div>
          )}
        </div>
      </Panel>
    );
  }

  return null;
}
