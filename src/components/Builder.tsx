import { useCallback, useEffect, useRef, useState } from "react";
import { Sparkles, X } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ipc } from "../ipc";
import type { AgentDefinition, Skill, Role, WorkspaceAgent } from "../ipc";
import { EVENT_NAMES, useEvent } from "../ipc";
import type { RosterChangedEvent } from "../ipc/events";
import { LEVELS, chainUp } from "../lib/positions";
import {
  CLAUDE_MODELS,
  CODEX_MODELS,
  COLOR_SWATCHES,
} from "../lib/modelCatalogue";
import { IdentitySection } from "./builder/IdentitySection";
import { BuilderRail } from "./builder/BuilderRail";
import { RoleLevelSection } from "./builder/RoleLevelSection";
import {
  blockerIsDanger,
  firstBlocker,
  readyLabel,
  SECTION_ORDER,
  sectionReadiness,
} from "./builder/readiness";
import type { ReadinessInput, SectionId } from "./builder/readiness";
import { useScrollSpy } from "./builder/useScrollSpy";
import { RuntimeSection, SECRET_PLACEHOLDER } from "./builder/RuntimeSection";
import type {
  CliAvailability,
  CliEffort,
  CliKind,
  CliModelCatalog,
  PermissionMode,
} from "./builder/RuntimeSection";
import { PositionSection } from "./builder/PositionSection";
import { SkillsSection } from "./builder/SkillsSection";

// ── Types ────────────────────────────────────────────────────────────────────

export interface BuilderProps {
  onClose: () => void;
  onSaved?: (def: AgentDefinition) => void;
  /**
   * Pre-fill the form. A definition WITH an id is an edit; an id-less one is an
   * AI draft (spec R6) — the form reads "New agent" and saves as a creation.
   */
  initialDef?: AgentDefinition;
  workspaceId?: string;
  workspaceAgentId?: string;
  /** Name of the drafter that produced an id-less `initialDef`; shows a chip
   *  under Identity until the user touches the form. */
  draftedBy?: string;
}

type AgentType = "cli" | "chat" | "orchestrator";

// ── Claude Code config presets ───────────────────────────────────────────────

function initialContextWindow(def?: AgentDefinition): string {
  return def?.contextWindow === "1m" ? "1m" : "200k";
}

/** Starter custom-env JSON, matching Claude Code's settings.json `env` shape. */
const DEFAULT_ENV_TEMPLATE = `{
  "env": {
    "ANTHROPIC_BASE_URL": "https://openrouter.ai/api",
    "ANTHROPIC_AUTH_TOKEN": "sk-or-...",
    "ANTHROPIC_MODEL": "...",
    "ANTHROPIC_SMALL_FAST_MODEL": "..."
  }
}`;

/** Build the env-editor text from a saved definition (or the starter template). */
function buildEnvText(def?: AgentDefinition): string {
  const env: Record<string, string> = { ...(def?.customEnv ?? {}) };
  // Secret values aren't returned — show their NAMES with a masked placeholder.
  for (const k of def?.secretEnvKeys ?? []) env[k] = SECRET_PLACEHOLDER;
  if (Object.keys(env).length === 0) return DEFAULT_ENV_TEMPLATE;
  return JSON.stringify({ env }, null, 2);
}

/**
 * Parse the env-editor text into a flat string→string map. Accepts either a
 * bare object or Claude's `{ "env": { … } }` wrapper. Throws on invalid JSON
 * (the caller surfaces it). Returns undefined when empty.
 */
function parseEnvText(text: string): Record<string, string> | undefined {
  const t = text.trim();
  if (!t) return undefined;
  const parsed: unknown = JSON.parse(t);
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("Custom environment must be a JSON object");
  }
  const obj = parsed as Record<string, unknown>;
  const inner =
    obj.env && typeof obj.env === "object" && !Array.isArray(obj.env)
      ? (obj.env as Record<string, unknown>)
      : obj;
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(inner)) {
    if (typeof v !== "string") {
      throw new Error(
        `Env value for "${k}" must be a string (got ${typeof v})`,
      );
    }
    out[k] = v;
  }
  return Object.keys(out).length ? out : undefined;
}

// ── Main component ───────────────────────────────────────────────────────────

export function Builder({
  onClose,
  onSaved,
  initialDef,
  workspaceId,
  workspaceAgentId,
  draftedBy,
}: BuilderProps) {
  // An id-less `initialDef` is a DRAFT, not an edit (spec R6): the save path
  // already posts `id: undefined`, so only the copy must follow.
  const isEditing = Boolean(initialDef?.id);
  // Cleared once the user edits anything — the "Drafted by" chip stops
  // claiming authorship the moment the draft becomes their own.
  const [touched, setTouched] = useState(false);

  // ── Form state (lazy-initialised from initialDef when editing) ─────────────
  const [name, setName] = useState(initialDef?.name ?? "");
  // Position System seed (D1/D3) — the level a NEW instance is created with.
  // Definition-level, all agent types, distinct from the per-workspace
  // Position section's `levelDraft` below (which edits an existing instance).
  const [defaultLevelDraft, setDefaultLevelDraft] = useState<
    AgentDefinition["defaultLevel"]
  >(initialDef?.defaultLevel ?? null);
  // D5 removed the Type picker: every agent the Builder can create is a CLI
  // agent. The value is still read by handleSave, and a legacy chat/
  // orchestrator definition keeps its own type through an edit.
  const [agentType] = useState<AgentType>(initialDef?.type ?? "cli");
  const [cliKind, setCliKind] = useState<CliKind>(
    initialDef?.cliKind ?? "claude-code",
  );
  const [color, setColor] = useState(initialDef?.color ?? COLOR_SWATCHES[0]);
  // Color picker popover (anchored to the avatar).
  const [showColors, setShowColors] = useState(false);
  const [model, setModel] = useState(initialDef?.model ?? "");
  // ── First-class CLI launch config ─────────────────────────────────────────
  const [permissionMode, setPermissionMode] = useState<PermissionMode>(
    initialDef?.id
      ? initialDef.cliKind === "antigravity"
        ? (initialDef.permissionMode ?? "default")
        : initialDef.permissionMode === "bypassPermissions"
          ? "bypassPermissions"
          : "auto"
      : (initialDef?.permissionMode ?? "bypassPermissions"),
  );
  // Distinguishes a user-picked value from the display fallback for a legacy
  // existing row whose stored permission_mode is NULL. An unrelated edit must
  // not silently turn that NULL into a more permissive explicit mode.
  const [permissionModeDirty, setPermissionModeDirty] = useState(false);
  const [effort, setEffort] = useState<CliEffort>(initialDef?.effort);
  const [cliAvailability, setCliAvailability] = useState<CliAvailability>({
    state: initialDef?.cliKind === "antigravity" ? "checking" : "idle",
  });
  const [modelCatalog, setModelCatalog] = useState<CliModelCatalog>({
    state: "idle",
  });
  const [contextWindow, setContextWindow] = useState<string>(() =>
    initialContextWindow(initialDef),
  );
  // Token filter (rtk): absent/null on the definition means enabled (default ON).
  const [rtkEnabled, setRtkEnabled] = useState<boolean>(
    initialDef?.rtkEnabled ?? true,
  );
  const [customArgs, setCustomArgs] = useState(initialDef?.customArgs ?? "");
  // Custom env is opt-in so the starter template isn't saved by accident.
  const [useCustomEnv, setUseCustomEnv] = useState(
    Object.keys(initialDef?.customEnv ?? {}).length > 0 ||
      (initialDef?.secretEnvKeys?.length ?? 0) > 0,
  );
  const [envText, setEnvText] = useState(() => buildEnvText(initialDef));
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // ── Skills ─────────────────────────────────────────────────────────────────
  const [allSkills, setAllSkills] = useState<Skill[]>([]);
  // initialDef?.skillIds may now also contain builtin ids (agentDef.list
  // annotates the FULL builtin+custom set) — harmless here since the custom
  // checklist below only ever tests membership against `kind === "custom"`
  // skills, so a builtin id in this state simply never matches any checkbox.
  const [skillIds, setSkillIds] = useState<string[]>(
    initialDef?.skillIds ?? [],
  );
  // ── Roles (ADR 0005) ─────────────────────────────────────────────────────────
  const [allRoles, setAllRoles] = useState<Role[]>([]);
  const [roleId, setRoleId] = useState<string>(initialDef?.roleId ?? "");
  // Inline "Custom…" role editor state.
  const [customRoleOpen, setCustomRoleOpen] = useState(false);
  const [customRoleName, setCustomRoleName] = useState("");
  const [customRoleDesc, setCustomRoleDesc] = useState("");
  const [customRoleSkillIds, setCustomRoleSkillIds] = useState<string[]>([]);
  const [savingRole, setSavingRole] = useState(false);
  const [positionRoster, setPositionRoster] = useState<WorkspaceAgent[]>([]);
  const [scopedAgent, setScopedAgent] = useState<WorkspaceAgent | null>(null);
  const [levelDraft, setLevelDraft] = useState<string | null>(null);
  const [supervisorDraft, setSupervisorDraft] = useState<string | null>(null);

  useEffect(() => {
    ipc.skill
      .list()
      .then(setAllSkills)
      .catch((err: unknown) => {
        if (import.meta.env.DEV)
          console.error("Builder: skill.list failed", err);
      });
    ipc.role
      .list()
      .then(setAllRoles)
      .catch((err: unknown) => {
        if (import.meta.env.DEV)
          console.error("Builder: role.list failed", err);
      });
  }, []);

  const positionScopeRequested = Boolean(
    initialDef?.id && workspaceId && workspaceAgentId,
  );

  useEffect(() => {
    if (
      !positionScopeRequested ||
      !workspaceId ||
      !workspaceAgentId ||
      !initialDef?.id
    ) {
      setPositionRoster([]);
      setScopedAgent(null);
      setLevelDraft(null);
      setSupervisorDraft(null);
      return;
    }

    let active = true;
    ipc.instance
      .list({ workspaceId })
      .then((instances) => {
        if (!active) return;
        setPositionRoster(instances);
        const scoped =
          instances.find(
            (agent) =>
              agent.id === workspaceAgentId &&
              agent.agentDefId === initialDef.id,
          ) ?? null;
        setScopedAgent(scoped);
        setLevelDraft(scoped?.level ?? null);
        setSupervisorDraft(scoped?.supervisorAgentId ?? null);
      })
      .catch((err: unknown) => {
        if (import.meta.env.DEV)
          console.error("Builder: instance.list failed", err);
      });

    return () => {
      active = false;
    };
  }, [initialDef?.id, positionScopeRequested, workspaceAgentId, workspaceId]);

  useEvent<RosterChangedEvent>(EVENT_NAMES.rosterChanged, (payload) => {
    if (
      !positionScopeRequested ||
      !workspaceId ||
      payload.workspaceId !== workspaceId
    )
      return;
    ipc.instance
      .list({ workspaceId })
      .then((instances) => {
        setPositionRoster(instances);
        const scoped =
          instances.find(
            (agent) =>
              agent.id === workspaceAgentId &&
              agent.agentDefId === initialDef?.id,
          ) ?? null;
        setScopedAgent(scoped);
      })
      .catch(() => {
        // The open editor keeps its current draft if the refresh fails.
      });
  });

  // Keep only skill ids that still resolve to an existing skill (mirror the
  // engine's copy filter — ADR 0005 review obligation): a role naming a since-
  // deleted skill must pre-select nothing for that id, not a dangling checkbox.
  function liveSkillIds(ids: string[]): string[] {
    return ids.filter((id) => allSkills.some((s) => s.id === id));
  }

  // Apply role COPY semantics in the UI: remove the outgoing role's live
  // defaults, then add the incoming role's live defaults. Manual picks outside
  // those defaults survive the transition; the engine remains uninvolved.
  function applyRoleTransition(
    fromId: string,
    toId?: string,
    roles = allRoles,
  ) {
    const outgoingRole = roles.find((r) => r.id === fromId);
    const incomingRole = roles.find((r) => r.id === toId);
    const outgoingSkillIds = new Set(
      liveSkillIds(outgoingRole?.skillIds ?? []),
    );
    const incomingSkillIds = liveSkillIds(incomingRole?.skillIds ?? []);
    setSkillIds((prev) =>
      Array.from(
        new Set([
          ...prev.filter((id) => !outgoingSkillIds.has(id)),
          ...incomingSkillIds,
        ]),
      ),
    );
  }

  function selectRole(id: string) {
    applyRoleTransition(roleId, id);
    setRoleId(id);
    setCustomRoleOpen(false);
    setTouched(true);
  }

  async function handleCreateCustomRole() {
    if (!customRoleName.trim() || !customRoleDesc.trim()) {
      setError("Custom role needs a name and a description");
      return;
    }
    setSavingRole(true);
    setError(null);
    try {
      const created = await ipc.role.save({
        name: customRoleName.trim(),
        description: customRoleDesc.trim(),
        skillIds: customRoleSkillIds,
      });
      const refreshed = await ipc.role.list();
      setAllRoles(refreshed);
      applyRoleTransition(roleId, created.id, refreshed);
      setRoleId(created.id);
      setCustomRoleOpen(false);
      setCustomRoleName("");
      setCustomRoleDesc("");
      setCustomRoleSkillIds([]);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSavingRole(false);
    }
  }

  // ── Derived ────────────────────────────────────────────────────────────────
  const letter = name.trim().charAt(0).toUpperCase() || "?";
  const selectedRole = allRoles.find((r) => r.id === roleId);
  // Fall back to any legacy free-text label when no first-class role resolves.
  const selectedRoleName =
    selectedRole?.name ?? (initialDef?.role || undefined);
  // Cards are rendered builtin-first, then custom, then the dashed "Custom…"
  // create card (stable sort keeps each group's list order from role.list).
  const orderedRoles = [...allRoles].sort(
    (a, b) => (a.kind === "builtin" ? 0 : 1) - (b.kind === "builtin" ? 0 : 1),
  );
  // Mandatory skills attach to every agent regardless of role — shown as the
  // "always on" note in the callout (derived, not hardcoded).
  const mandatorySkillNames = allSkills
    .filter((s) => s.kind === "builtin" && s.mandatory)
    .map((s) => s.name);
  // The selected role's default skills, resolved to display names (ids that no
  // longer resolve to a skill are dropped — they wouldn't pre-select either).
  const attachSkillNames = selectedRole
    ? selectedRole.skillIds
        .map((id) => allSkills.find((s) => s.id === id)?.name)
        .filter((n): n is string => Boolean(n))
    : [];
  // CLI launch config applies to every first-class CLI (custom isn't wired yet).
  const isClaudeCode = agentType === "cli" && cliKind === "claude-code";
  const isCodex = agentType === "cli" && cliKind === "codex";
  const isAntigravity = agentType === "cli" && cliKind === "antigravity";
  const showCliConfig = isClaudeCode || isCodex || isAntigravity;
  const antigravitySaveBlocked =
    isAntigravity &&
    cliAvailability.state !== "available" &&
    cliAvailability.state !== "error";
  const modelPresets = isCodex ? CODEX_MODELS : CLAUDE_MODELS;
  const catalogModels =
    modelCatalog.state === "ready" ? modelCatalog.models : [];
  // Editing must be lossless: a saved model the catalog no longer lists keeps
  // its own selected option instead of being silently reset to Auto.
  const savedModelUnlisted =
    isAntigravity &&
    model !== "" &&
    !catalogModels.some((entry) => entry.id === model);
  const positionEnabled = Boolean(
    scopedAgent && workspaceId && workspaceAgentId && initialDef?.id,
  );
  const trackLabel = selectedRoleName ?? scopedAgent?.roleName ?? "No role";
  // Canon rule 11: the line under the name reads "Role · Level".
  const defaultLevelName = defaultLevelDraft
    ? (LEVELS.find((l) => l.id === defaultLevelDraft)?.name ?? "Unranked")
    : "Unranked";
  const identityLine = `${selectedRoleName ?? "No role"} · ${defaultLevelName}`;
  const supervisorOptions = positionEnabled
    ? positionRoster
        .filter((agent) => agent.id !== scopedAgent!.id)
        .sort((left, right) =>
          (left.name ?? left.id).localeCompare(right.name ?? right.id),
        )
    : [];
  const previewRoster = positionEnabled
    ? positionRoster.map((agent) =>
        agent.id === scopedAgent!.id
          ? {
              ...agent,
              level: levelDraft ?? undefined,
              supervisorAgentId: supervisorDraft ?? undefined,
            }
          : agent,
      )
    : [];
  const previewChainIds = positionEnabled
    ? chainUp(scopedAgent!.id, previewRoster)
    : [];
  const levelChanged =
    positionEnabled && (scopedAgent?.level ?? null) !== levelDraft;
  const supervisorChanged =
    positionEnabled &&
    (scopedAgent?.supervisorAgentId ?? null) !== supervisorDraft;

  const checkAntigravityAvailability = useCallback(async () => {
    setCliAvailability({ state: "checking" });
    try {
      const status = await ipc.instance.cliStatus({ cliKind: "antigravity" });
      setCliAvailability({
        state: status.available ? "available" : "missing",
        installUrl: status.installUrl,
      });
    } catch (e) {
      setCliAvailability({
        state: "error",
        message: e instanceof Error ? e.message : String(e),
      });
    }
  }, []);

  useEffect(() => {
    if (!isAntigravity) {
      setCliAvailability({ state: "idle" });
      return;
    }
    void checkAntigravityAvailability();
  }, [checkAntigravityAvailability, isAntigravity]);

  const loadAntigravityModels = useCallback(async () => {
    setModelCatalog({ state: "loading" });
    try {
      const { models } = await ipc.instance.cliModels({
        cliKind: "antigravity",
      });
      setModelCatalog({ state: "ready", models });
    } catch {
      // The backend's message is raw shell/auth text — surface a fixed retryable
      // line instead and keep the detail out of the UI copy.
      setModelCatalog({ state: "error" });
    }
  }, []);

  // Discovery is gated on availability, so "Check again" (which flips
  // available -> checking -> available) re-runs the catalog query too.
  useEffect(() => {
    if (!isAntigravity || cliAvailability.state !== "available") {
      setModelCatalog({ state: "idle" });
      return;
    }
    void loadAntigravityModels();
  }, [cliAvailability.state, isAntigravity, loadAntigravityModels]);

  async function openAntigravityInstallGuide() {
    if (cliAvailability.state !== "missing") return;
    try {
      await openUrl(cliAvailability.installUrl);
    } catch {
      window.open(cliAvailability.installUrl, "_blank", "noopener,noreferrer");
    }
  }

  function selectCliKind(next: CliKind) {
    if (next !== cliKind) setPermissionModeDirty(true);
    if (next === "antigravity" && permissionMode === "auto") {
      setPermissionMode("default");
    } else if (
      cliKind === "antigravity" &&
      next !== "antigravity" &&
      permissionMode !== "bypassPermissions"
    ) {
      setPermissionMode("auto");
    }
    if (next === "antigravity") setCliAvailability({ state: "checking" });
    setCliKind(next);
    if (
      next === "claude-code" &&
      contextWindow !== "1m" &&
      contextWindow !== "200k"
    ) {
      setContextWindow("200k");
    }
  }

  function selectModelPreset(next: string) {
    setModel(next);
  }

  // ── Save ───────────────────────────────────────────────────────────────────
  async function handleSave() {
    if (antigravitySaveBlocked) return;
    if (!name.trim()) {
      setError("Name is required");
      return;
    }
    // Claude Code keeps its "1m"/"200k" segmented value; Codex sends undefined
    // (R2/R4 — Auto, backend derives the window from the model, any stored
    // value is ignored at launch).
    const contextWindowForSave: string | undefined = isClaudeCode
      ? contextWindow === "1m"
        ? "1m"
        : "200k"
      : undefined;
    const permissionModeForSave: AgentDefinition["permissionMode"] =
      isEditing && !permissionModeDirty && initialDef?.cliKind === cliKind
        ? initialDef.permissionMode
        : isAntigravity
          ? permissionMode === "auto"
            ? "default"
            : permissionMode
          : isClaudeCode || isCodex
            ? permissionMode === "bypassPermissions"
              ? "bypassPermissions"
              : "auto"
            : undefined;
    // Parse the custom env up front so a JSON error is reported before saving.
    // Claude Code only — Codex doesn't use ANTHROPIC_* env config.
    let customEnv: Record<string, string> | undefined;
    if (isClaudeCode && useCustomEnv) {
      try {
        customEnv = parseEnvText(envText);
      } catch (e) {
        setError(
          e instanceof Error ? e.message : "Invalid custom environment JSON",
        );
        return;
      }
    }

    setSaving(true);
    setError(null);
    try {
      // First-class role (ADR 0005): send `roleId` + the role's display name as
      // the legacy `role` fallback. With no role picked, preserve any existing
      // legacy free-text label rather than clearing it (Phase B persists roleId;
      // the engine ignores it until then). `selectedRole` is derived below.
      const def = await ipc.agentDef.save({
        // Pass `id` when editing so the backend upserts rather than inserts.
        id: initialDef?.id,
        name: name.trim(),
        type: agentType,
        role: selectedRole?.name ?? (initialDef?.role || undefined),
        roleId: roleId || undefined,
        cliKind: agentType === "cli" ? cliKind : undefined,
        color: color || undefined,
        model: model.trim() || undefined,
        // Harness is no longer a per-agent setting: every agent uses the shared
        // central Conclave harness + shared blackboard. Kept in the payload as
        // fixed constants so the backend contract is unchanged (no migration).
        harnessMode: "central",
        shareBlackboard: true,
        // Messaging is no longer a per-agent setting: every agent can message
        // every other agent, and injected messages always auto-submit (the
        // backend already behaves this way). Sent as fixed constants.
        autoSubmitInjected: true,
        allowedSenders: "all",
        // Harness-specific launch config. Auto effort is omitted so the
        // provider default wins. Explicit Antigravity Default is preserved;
        // only an untouched legacy NULL edit remains omitted.
        permissionMode: permissionModeForSave,
        effort: showCliConfig ? effort : undefined,
        contextWindow: contextWindowForSave,
        // Token filter (rtk) — claude-code + codex; the engine treats absent as ON.
        rtkEnabled: isClaudeCode || isCodex ? rtkEnabled : undefined,
        customArgs:
          showCliConfig && customArgs.trim() ? customArgs.trim() : undefined,
        customEnv,
        // Skills are cli-only in v1 — omit for other types so a chat/orchestrator
        // save never sends a stale list.
        skillIds: agentType === "cli" ? skillIds : undefined,
        // Position System seed (D1/D3) — all agent types, create and edit.
        defaultLevel: defaultLevelDraft,
      });
      if (positionEnabled && (levelChanged || supervisorChanged)) {
        const req: {
          workspaceId: string;
          workspaceAgentId: string;
          level?: string | null;
          supervisorAgentId?: string | null;
        } = {
          workspaceId: workspaceId!,
          workspaceAgentId: workspaceAgentId!,
        };
        if (levelChanged) req.level = levelDraft;
        if (supervisorChanged) req.supervisorAgentId = supervisorDraft;
        await ipc.instance.setPosition(req);
      }
      onSaved?.(def);
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  // ── Rail, readiness and scroll-spy (D1/D2/D7) ────────────────────────────
  const railItems: SectionId[] = positionEnabled
    ? SECTION_ORDER
    : SECTION_ORDER.filter((id) => id !== "position");
  const readinessInput: ReadinessInput = {
    name,
    isAntigravity,
    cliAvailabilityState: cliAvailability.state,
    isEditing,
  };
  const readiness = sectionReadiness(readinessInput);
  const blocker = firstBlocker(readinessInput);
  const scrollRef = useRef<HTMLDivElement>(null);
  const { activeId, jumpTo } = useScrollSpy(scrollRef, railItems);

  // ── Render ─────────────────────────────────────────────────────────────────
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30">
      <div className="w-[880px] max-h-[90vh] bg-surface rounded-2xl shadow-2xl flex flex-col overflow-hidden ring-1 ring-overlay/[0.08]">
        {/* ── Header ── */}
        <div className="h-11 flex items-center justify-between px-4 border-b border-overlay/[0.06] shrink-0">
          <div className="flex items-center gap-2">
            <Sparkles className="w-4 h-4 text-accent" />
            <span className="text-[13px] font-semibold tracking-tight">
              {isEditing ? "Edit agent" : "New agent"}
            </span>
            <span className="text-[10px] font-medium text-text-muted bg-overlay/[0.04] px-1.5 py-px rounded-md">
              {isEditing ? "update definition" : "saved to Library"}
            </span>
          </div>
          <button
            onClick={onClose}
            className="w-7 h-7 grid place-items-center rounded-md hover:bg-overlay/[0.05] text-text-secondary"
            aria-label="Close builder"
          >
            <X className="w-[15px] h-[15px]" />
          </button>
        </div>

        {/* ── Body: fixed rail + scrolling content column (D1/D2) ── */}
        <div className="flex flex-1 min-h-0">
          <BuilderRail
            items={railItems}
            readiness={readiness}
            activeId={activeId}
            onJump={jumpTo}
          />
          {/* `relative`: useScrollSpy.jumpTo measures offsetTop against this box. */}
          <div
            ref={scrollRef}
            className="relative flex-1 min-h-0 overflow-y-auto px-6 py-5"
          >
            <IdentitySection
              name={name}
              setName={setName}
              color={color}
              setColor={setColor}
              showColors={showColors}
              setShowColors={setShowColors}
              letter={letter}
              identityLine={identityLine}
              draftedBy={draftedBy}
              touched={touched}
              setTouched={setTouched}
            />

            <RoleLevelSection
              orderedRoles={orderedRoles}
              selectedRole={selectedRole}
              roleId={roleId}
              selectRole={selectRole}
              clearRole={() => {
                applyRoleTransition(roleId);
                setRoleId("");
                setCustomRoleOpen(false);
              }}
              openCustomRole={() => {
                applyRoleTransition(roleId);
                setCustomRoleOpen(true);
                setRoleId("");
              }}
              customRoleOpen={customRoleOpen}
              customRoleName={customRoleName}
              setCustomRoleName={setCustomRoleName}
              customRoleDesc={customRoleDesc}
              setCustomRoleDesc={setCustomRoleDesc}
              customRoleSkillIds={customRoleSkillIds}
              setCustomRoleSkillIds={setCustomRoleSkillIds}
              cancelCustomRole={() => {
                setCustomRoleOpen(false);
                setCustomRoleName("");
                setCustomRoleDesc("");
                setCustomRoleSkillIds([]);
              }}
              handleCreateCustomRole={() => void handleCreateCustomRole()}
              savingRole={savingRole}
              allSkills={allSkills}
              attachSkillNames={attachSkillNames}
              mandatorySkillNames={mandatorySkillNames}
              defaultLevel={defaultLevelDraft}
              setDefaultLevel={setDefaultLevelDraft}
            />

            <RuntimeSection
              cliKind={cliKind}
              selectCliKind={selectCliKind}
              isClaudeCode={isClaudeCode}
              isCodex={isCodex}
              isAntigravity={isAntigravity}
              showCliConfig={showCliConfig}
              cliAvailability={cliAvailability}
              checkAntigravityAvailability={() =>
                void checkAntigravityAvailability()
              }
              openAntigravityInstallGuide={() =>
                void openAntigravityInstallGuide()
              }
              modelCatalog={modelCatalog}
              loadAntigravityModels={() => void loadAntigravityModels()}
              catalogModels={catalogModels}
              savedModelUnlisted={savedModelUnlisted}
              model={model}
              setModel={setModel}
              modelPresets={modelPresets}
              selectModelPreset={selectModelPreset}
              effort={effort}
              setEffort={setEffort}
              permissionMode={permissionMode}
              setPermissionMode={setPermissionMode}
              setPermissionModeDirty={setPermissionModeDirty}
              contextWindow={contextWindow}
              setContextWindow={setContextWindow}
              rtkEnabled={rtkEnabled}
              setRtkEnabled={setRtkEnabled}
              customArgs={customArgs}
              setCustomArgs={setCustomArgs}
              useCustomEnv={useCustomEnv}
              setUseCustomEnv={setUseCustomEnv}
              envText={envText}
              setEnvText={setEnvText}
              advancedInitiallyOpen={
                Boolean(initialDef?.customArgs) || useCustomEnv
              }
            />

            <SkillsSection
              allSkills={allSkills}
              skillIds={skillIds}
              setSkillIds={setSkillIds}
            />

            {positionEnabled && (
              <PositionSection
                scopedAgent={scopedAgent!}
                positionRoster={positionRoster}
                supervisorOptions={supervisorOptions}
                previewRoster={previewRoster}
                previewChainIds={previewChainIds}
                trackLabel={trackLabel}
                levelDraft={levelDraft}
                setLevelDraft={setLevelDraft}
                supervisorDraft={supervisorDraft}
                setSupervisorDraft={setSupervisorDraft}
              />
            )}

            {/* Error */}
            {error && (
              <p className="mt-4 px-1 text-[12px] text-danger">{error}</p>
            )}
          </div>
        </div>

        {/* ── Footer actions ── */}
        <div className="border-t border-overlay/[0.07] px-5 py-2.5 bg-surface shrink-0 flex items-center justify-between gap-3">
          <span
            aria-live="polite"
            className={`text-[11.5px] ${blockerIsDanger(blocker) ? "text-danger" : "text-text-tertiary"}`}
          >
            {blocker ?? readyLabel(readinessInput)}
          </span>
          <div className="flex items-center gap-2">
            <button
              onClick={onClose}
              className="text-[12.5px] font-medium text-text-secondary px-3.5 py-1.5 rounded-lg hover:bg-overlay/[0.05]"
            >
              Cancel
            </button>
            <button
              onClick={handleSave}
              disabled={saving || blocker !== null || antigravitySaveBlocked}
              className="text-[12.5px] font-semibold text-white bg-accent px-4 py-1.5 rounded-lg hover:brightness-105 disabled:cursor-not-allowed disabled:opacity-45 flex items-center gap-1.5"
            >
              <Sparkles className="w-3.5 h-3.5" />
              {saving
                ? "Saving…"
                : cliAvailability.state === "checking" && isAntigravity
                  ? "Checking agy…"
                  : cliAvailability.state === "missing" && isAntigravity
                    ? "Install agy to continue"
                    : isEditing
                      ? "Save changes"
                      : "Create agent"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
