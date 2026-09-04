import { useCallback, useEffect, useState } from "react";
import {
  AlertTriangle,
  Check,
  ChevronDown,
  CircleHelp,
  X,
  Sparkles,
  Waypoints,
  MessageSquare,
  Terminal,
  Compass,
  ShieldCheck,
  Hammer,
  PenTool,
  Microscope,
  Plus,
  RefreshCw,
  UserPen,
} from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ipc } from "../ipc";
import type { AgentDefinition, Skill, Role, WorkspaceAgent } from "../ipc";
import { EVENT_NAMES, useEvent } from "../ipc";
import type { RosterChangedEvent } from "../ipc/events";
import { LEVELS, chainUp, levelOf, wouldCycle } from "../lib/positions";
import { CLAUDE_MODELS, CODEX_MODELS, COLOR_SWATCHES } from "../lib/modelCatalogue";
import { HumanChip, PositionLine } from "./Position";

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
type CliKind = "claude-code" | "codex" | "antigravity" | "custom";
type PermissionMode = "auto" | "default" | "acceptEdits" | "plan" | "bypassPermissions";
type CliEffort = "low" | "medium" | "high" | undefined;
type ClaudeContextWindow = "1m" | "200k";

type CliAvailability =
  | { state: "idle" | "checking" }
  | { state: "available" | "missing"; installUrl: string }
  | { state: "error"; message: string };

/** The authenticated Antigravity model catalog (`instance.cliModels`). Queried
 *  only once availability says `agy` is there, so `error` here always means the
 *  QUERY failed (auth/network) — never that the CLI is missing. */
type CliModelCatalog =
  | { state: "idle" | "loading" }
  | { state: "ready"; models: { id: string; label: string }[] }
  | { state: "error" };

const ANTIGRAVITY_MODE_HELP: Record<Exclude<PermissionMode, "auto">, string> = {
  default: "Pauses for diff review before applying changes.",
  acceptEdits: "Applies file edits automatically. Shell and web actions still ask.",
  plan: "Starts in planning mode before making changes.",
  bypassPermissions: "Skips every permission prompt, including shell and web actions.",
};

// ── Builtin role card looks (ADR 0005) ───────────────────────────────────────

/**
 * The backend `Role` (id / name / description / skillIds / kind) carries no
 * icon or tagline — the card design (Arta proto @ a24f482) assigns one per
 * builtin role id. Custom (user-created) roles have no designed look, so they
 * fall back to a neutral icon + generic tagline; their real content shows in
 * the selected-role callout via `description`.
 */
const BUILTIN_ROLE_LOOKS: Record<string, { Icon: typeof Compass; tagline: string }> = {
  lead: { Icon: Compass, tagline: "Settles & delegates work" },
  reviewer: { Icon: ShieldCheck, tagline: "Grills work with evidence" },
  implementer: { Icon: Hammer, tagline: "Builds the recorded plan" },
  designer: { Icon: PenTool, tagline: "Designs on the canvas" },
  researcher: { Icon: Microscope, tagline: "Investigates open questions" },
};

function roleLook(role: Role): { Icon: typeof Compass; tagline: string } {
  return BUILTIN_ROLE_LOOKS[role.id] ?? { Icon: UserPen, tagline: "Custom role" };
}

// ── Claude Code config presets ───────────────────────────────────────────────

function initialContextWindow(def?: AgentDefinition): string {
  return def?.contextWindow === "1m" ? "1m" : "200k";
}

/**
 * Sentinel shown for a secret env var already stored in the Keychain. Sending
 * it back unchanged means "keep the stored secret" (must match
 * `SECRET_PLACEHOLDER` in `src-tauri/src/engine/commands/agent.rs`).
 */
const SECRET_PLACEHOLDER = "••••••••";

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
      throw new Error(`Env value for "${k}" must be a string (got ${typeof v})`);
    }
    out[k] = v;
  }
  return Object.keys(out).length ? out : undefined;
}

// ── Sub-components ───────────────────────────────────────────────────────────

interface ToggleProps {
  on: boolean;
  onChange: (v: boolean) => void;
  label?: string;
}

function Toggle({ on, onChange, label }: ToggleProps) {
  return (
    <button
      role="switch"
      aria-checked={on}
      aria-label={label}
      onClick={() => onChange(!on)}
      className={`w-9 h-5 rounded-full relative transition-colors ${on ? "bg-status-running" : "bg-black/20"}`}
    >
      <span
        className={`absolute top-0.5 w-4 h-4 rounded-full bg-surface shadow-sm transition-transform ${on ? "right-0.5" : "left-0.5"}`}
      />
    </button>
  );
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
  const [defaultLevelDraft, setDefaultLevelDraft] = useState<AgentDefinition["defaultLevel"]>(
    initialDef?.defaultLevel ?? null,
  );
  const [agentType, setAgentType] = useState<AgentType>(initialDef?.type ?? "cli");
  const [cliKind, setCliKind] = useState<CliKind>(initialDef?.cliKind ?? "claude-code");
  const [color, setColor] = useState(initialDef?.color ?? COLOR_SWATCHES[0]);
  // Color picker popover (anchored to the avatar).
  const [showColors, setShowColors] = useState(false);
  const [model, setModel] = useState(initialDef?.model ?? "");
  // ── First-class CLI launch config ─────────────────────────────────────────
  const [permissionMode, setPermissionMode] = useState<PermissionMode>(
    initialDef?.id
      ? initialDef.cliKind === "antigravity"
        ? initialDef.permissionMode ?? "default"
        : initialDef.permissionMode === "bypassPermissions"
          ? "bypassPermissions"
          : "auto"
      : initialDef?.permissionMode ?? "bypassPermissions",
  );
  // Distinguishes a user-picked value from the display fallback for a legacy
  // existing row whose stored permission_mode is NULL. An unrelated edit must
  // not silently turn that NULL into a more permissive explicit mode.
  const [permissionModeDirty, setPermissionModeDirty] = useState(false);
  const [effort, setEffort] = useState<CliEffort>(initialDef?.effort);
  const [cliAvailability, setCliAvailability] = useState<CliAvailability>({
    state: initialDef?.cliKind === "antigravity" ? "checking" : "idle",
  });
  const [modelCatalog, setModelCatalog] = useState<CliModelCatalog>({ state: "idle" });
  const [contextWindow, setContextWindow] = useState<string>(() => initialContextWindow(initialDef));
  // Token filter (rtk): absent/null on the definition means enabled (default ON).
  const [rtkEnabled, setRtkEnabled] = useState<boolean>(initialDef?.rtkEnabled ?? true);
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
  const [skillIds, setSkillIds] = useState<string[]>(initialDef?.skillIds ?? []);
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
        if (import.meta.env.DEV) console.error("Builder: skill.list failed", err);
      });
    ipc.role
      .list()
      .then(setAllRoles)
      .catch((err: unknown) => {
        if (import.meta.env.DEV) console.error("Builder: role.list failed", err);
      });
  }, []);

  const positionScopeRequested = Boolean(initialDef?.id && workspaceId && workspaceAgentId);

  useEffect(() => {
    if (!positionScopeRequested || !workspaceId || !workspaceAgentId || !initialDef?.id) {
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
            (agent) => agent.id === workspaceAgentId && agent.agentDefId === initialDef.id,
          ) ?? null;
        setScopedAgent(scoped);
        setLevelDraft(scoped?.level ?? null);
        setSupervisorDraft(scoped?.supervisorAgentId ?? null);
      })
      .catch((err: unknown) => {
        if (import.meta.env.DEV) console.error("Builder: instance.list failed", err);
      });

    return () => {
      active = false;
    };
  }, [initialDef?.id, positionScopeRequested, workspaceAgentId, workspaceId]);

  useEvent<RosterChangedEvent>(EVENT_NAMES.rosterChanged, (payload) => {
    if (!positionScopeRequested || !workspaceId || payload.workspaceId !== workspaceId) return;
    ipc.instance
      .list({ workspaceId })
      .then((instances) => {
        setPositionRoster(instances);
        const scoped =
          instances.find(
            (agent) => agent.id === workspaceAgentId && agent.agentDefId === initialDef?.id,
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
  function applyRoleTransition(fromId: string, toId?: string, roles = allRoles) {
    const outgoingRole = roles.find((r) => r.id === fromId);
    const incomingRole = roles.find((r) => r.id === toId);
    const outgoingSkillIds = new Set(liveSkillIds(outgoingRole?.skillIds ?? []));
    const incomingSkillIds = liveSkillIds(incomingRole?.skillIds ?? []);
    setSkillIds((prev) =>
      Array.from(
        new Set([...prev.filter((id) => !outgoingSkillIds.has(id)), ...incomingSkillIds]),
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
  const selectedRoleName = selectedRole?.name ?? (initialDef?.role || undefined);
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
  const catalogModels = modelCatalog.state === "ready" ? modelCatalog.models : [];
  // Editing must be lossless: a saved model the catalog no longer lists keeps
  // its own selected option instead of being silently reset to Auto.
  const savedModelUnlisted =
    isAntigravity && model !== "" && !catalogModels.some((entry) => entry.id === model);
  const positionEnabled = Boolean(
    scopedAgent && workspaceId && workspaceAgentId && initialDef?.id,
  );
  const trackLabel = selectedRoleName ?? scopedAgent?.roleName ?? "No role";
  const supervisorOptions = positionEnabled
    ? positionRoster
        .filter((agent) => agent.id !== scopedAgent!.id)
        .sort((left, right) => (left.name ?? left.id).localeCompare(right.name ?? right.id))
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
  const previewChainIds = positionEnabled ? chainUp(scopedAgent!.id, previewRoster) : [];
  const levelChanged = positionEnabled && (scopedAgent?.level ?? null) !== levelDraft;
  const supervisorChanged =
    positionEnabled && (scopedAgent?.supervisorAgentId ?? null) !== supervisorDraft;

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
      const { models } = await ipc.instance.cliModels({ cliKind: "antigravity" });
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
    if (next === "claude-code" && contextWindow !== "1m" && contextWindow !== "200k") {
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
      ? contextWindow === "1m" ? "1m" : "200k"
      : undefined;
    const permissionModeForSave: AgentDefinition["permissionMode"] =
      isEditing && !permissionModeDirty && initialDef?.cliKind === cliKind
        ? initialDef.permissionMode
        : isAntigravity
          ? permissionMode === "auto" ? "default" : permissionMode
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
        setError(e instanceof Error ? e.message : "Invalid custom environment JSON");
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
        customArgs: showCliConfig && customArgs.trim() ? customArgs.trim() : undefined,
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

  // ── Render ─────────────────────────────────────────────────────────────────
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30">
      <div className="w-[560px] max-h-[90vh] bg-surface rounded-2xl shadow-2xl flex flex-col overflow-hidden ring-1 ring-overlay/[0.08]">

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

        {/* ── Scrollable body ── */}
        <div className="flex-1 overflow-y-auto px-5 py-4 space-y-3.5 min-h-0">

          {/* Identity */}
          <section>
            <div className="flex items-center justify-between gap-2 mb-2">
              <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase">
                Identity
              </div>
              {draftedBy && !touched && (
                <span className="text-[11px] text-text-tertiary inline-flex items-center gap-1">
                  <Sparkles className="w-3 h-3" />
                  Drafted by {draftedBy}
                </span>
              )}
            </div>
            <div className="flex items-center gap-2.5">
              {/* Avatar doubles as the color picker — click to choose. */}
              <div className="relative shrink-0">
                <button
                  type="button"
                  onClick={() => setShowColors((v) => !v)}
                  className="w-10 h-10 rounded-[10px] text-white grid place-items-center text-[15px] font-bold ring-1 ring-overlay/[0.06] hover:brightness-105"
                  style={{ backgroundColor: color }}
                  title="Change color"
                  aria-label="Change color"
                >
                  {letter}
                </button>
                {showColors && (
                  <>
                    {/* Click-away backdrop. */}
                    <div className="fixed inset-0 z-10" onClick={() => setShowColors(false)} />
                    <div className="absolute z-20 top-full left-0 mt-1.5 flex items-center gap-1.5 bg-surface rounded-xl ring-1 ring-overlay/[0.1] shadow-lg p-2">
                      {COLOR_SWATCHES.map((swatch) => (
                        <button
                          key={swatch}
                          onClick={() => {
                            setColor(swatch);
                            setShowColors(false);
                          }}
                          className={`w-[18px] h-[18px] rounded-full transition-all ${
                            color === swatch ? "ring-2 ring-offset-1" : "hover:scale-110"
                          }`}
                          style={
                            {
                              backgroundColor: swatch,
                              "--tw-ring-color": swatch,
                            } as React.CSSProperties
                          }
                          aria-label={`Color ${swatch}`}
                        />
                      ))}
                      {/* Custom color — opens the OS color picker. The popover
                          stays open so the avatar preview updates live. */}
                      <label
                        className="w-[18px] h-[18px] rounded-full cursor-pointer ring-1 ring-overlay/15 relative overflow-hidden shrink-0"
                        title="Custom color"
                        style={{
                          background:
                            "conic-gradient(red, yellow, lime, aqua, blue, magenta, red)",
                        }}
                      >
                        <input
                          type="color"
                          value={color}
                          onChange={(e) => setColor(e.target.value)}
                          className="absolute inset-0 opacity-0 cursor-pointer"
                          aria-label="Custom color"
                        />
                      </label>
                    </div>
                  </>
                )}
              </div>
              <div className="flex-1 space-y-1">
                <input
                  value={name}
                  onChange={(e) => {
                    setName(e.target.value);
                    setTouched(true);
                  }}
                  placeholder="Agent name"
                  className="w-full text-[14px] font-semibold tracking-tight bg-transparent outline-none border-b border-overlay/10 focus:border-accent pb-0.5"
                />
                <div className="text-[11.5px] text-text-muted truncate">
                  {selectedRoleName ?? "No role"}
                </div>
              </div>
            </div>

          </section>

          {/* Level — the Position System SEED (D1/D3): remembered on the
              definition so it's restored whenever a new instance is created
              from it (removing + re-adding an agent to a workspace). Distinct
              from the per-workspace Position section below, which edits an
              EXISTING instance's live level and never touches this value. */}
          <section>
            <div className="flex items-center justify-between mb-2">
              <span className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase">
                Level
              </span>
              <button
                type="button"
                onClick={() => setDefaultLevelDraft(null)}
                className={`text-[11px] font-medium ${
                  defaultLevelDraft == null
                    ? "text-accent"
                    : "text-text-tertiary hover:text-text-secondary"
                }`}
              >
                Clear to Unranked
              </button>
            </div>
            <div className="grid grid-cols-4 gap-2">
              {LEVELS.map((level) => {
                const active = defaultLevelDraft === level.id;
                return (
                  <button
                    key={level.id}
                    type="button"
                    onClick={() =>
                      setDefaultLevelDraft(level.id as AgentDefinition["defaultLevel"])
                    }
                    className={`rounded-xl px-2.5 py-2 text-left transition-all ring-1 ${
                      active
                        ? "ring-accent/40 bg-accent/[0.06]"
                        : "ring-overlay/[0.08] bg-surface hover:bg-overlay/[0.02]"
                    }`}
                  >
                    <div className="text-[11.5px] font-semibold leading-tight">{level.name}</div>
                    <div className="mt-1 text-[11px] text-text-tertiary">rung {level.rung}</div>
                  </button>
                );
              })}
            </div>
          </section>

          {/* Role (ADR 0005) — card grid (matches the Type cards below), with a
              quiet "No role" toggle in the header and a "Custom…" create card. */}
          <section>
            <div className="flex items-center justify-between mb-2">
              <span className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase">
                Role
              </span>
              <button
                type="button"
                onClick={() => {
                  applyRoleTransition(roleId);
                  setRoleId("");
                  setCustomRoleOpen(false);
                }}
                className={`text-[11px] font-medium transition-colors ${
                  roleId === "" && !customRoleOpen
                    ? "text-accent"
                    : "text-text-tertiary hover:text-text-secondary"
                }`}
              >
                No role
              </button>
            </div>

            <div className="grid grid-cols-2 gap-2">
              {orderedRoles.map((r) => {
                const { Icon, tagline } = roleLook(r);
                const active = roleId === r.id && !customRoleOpen;
                return (
                  <button
                    key={r.id}
                    type="button"
                    onClick={() => selectRole(r.id)}
                    aria-pressed={active}
                    className={`relative rounded-xl p-2.5 text-left transition-all ring-1 ${
                      active
                        ? "ring-accent/40 bg-accent/[0.06]"
                        : "ring-overlay/[0.08] bg-surface hover:bg-overlay/[0.02]"
                    }`}
                  >
                    <Icon
                      className={`w-[17px] h-[17px] mb-1.5 ${
                        active ? "text-accent" : "text-text-secondary"
                      }`}
                    />
                    <div className="text-[12.5px] font-semibold leading-tight">{r.name}</div>
                    <div className="text-[11px] text-text-tertiary leading-snug mt-0.5">
                      {tagline}
                    </div>
                  </button>
                );
              })}

              {/* Custom… — dashed action card that opens the inline role editor. */}
              <button
                type="button"
                onClick={() => {
                  applyRoleTransition(roleId);
                  setCustomRoleOpen(true);
                  setRoleId("");
                }}
                aria-pressed={customRoleOpen}
                className={`relative rounded-xl p-2.5 text-left transition-all border border-dashed ${
                  customRoleOpen
                    ? "border-accent/60 bg-accent/[0.06]"
                    : "border-overlay/[0.12] hover:bg-overlay/[0.02]"
                }`}
              >
                <Plus
                  className={`w-[17px] h-[17px] mb-1.5 ${
                    customRoleOpen ? "text-accent" : "text-text-secondary"
                  }`}
                />
                <div className="text-[12.5px] font-semibold leading-tight">Custom…</div>
                <div className="text-[11px] text-text-tertiary leading-snug mt-0.5">
                  Define your own role
                </div>
              </button>
            </div>

            {/* Selected-role callout: description + attached skills + always-on note. */}
            {selectedRole && (
              <div className="mt-2.5 rounded-xl ring-1 ring-overlay/[0.08] bg-surface p-3">
                <div className="flex items-center gap-2 mb-1.5">
                  {(() => {
                    const { Icon } = roleLook(selectedRole);
                    return <Icon className="w-3.5 h-3.5 text-accent shrink-0" />;
                  })()}
                  <span className="text-[12.5px] font-semibold">{selectedRole.name}</span>
                </div>
                <p className="text-[11.5px] text-text-tertiary leading-relaxed">
                  {selectedRole.description}
                </p>
                <div className="mt-2.5 flex flex-wrap items-center gap-1.5">
                  <span className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase">
                    Attaches
                  </span>
                  {attachSkillNames.length > 0 ? (
                    attachSkillNames.map((s) => (
                      <span
                        key={s}
                        className="text-[11px] font-medium px-2 py-0.5 rounded-md ring-1 ring-accent/40 bg-accent/[0.08] text-accent"
                      >
                        {s}
                      </span>
                    ))
                  ) : (
                    <span className="text-[11px] text-text-tertiary italic">
                      mandatory skills only
                    </span>
                  )}
                </div>
                {mandatorySkillNames.length > 0 && (
                  <div className="mt-1.5 text-[10.5px] text-text-tertiary leading-snug">
                    + {mandatorySkillNames.join(", ")}
                    <span className="opacity-70"> · always on</span>
                  </div>
                )}
              </div>
            )}

            {/* No-role empty note. */}
            {!selectedRole && !customRoleOpen && (
              <div className="mt-2.5 rounded-xl border border-dashed border-overlay/[0.12] bg-surface p-3 text-[11.5px] text-text-tertiary leading-relaxed">
                No role: the agent runs with only the mandatory
                {mandatorySkillNames.length > 0 ? ` ${mandatorySkillNames.join(" and ")} ` : " "}
                skills, and no job description in its preamble.
              </div>
            )}

            {/* Inline custom-role editor */}
            {customRoleOpen && (
              <div className="mt-2 rounded-xl ring-1 ring-overlay/[0.08] bg-surface p-3 space-y-2">
                <input
                  value={customRoleName}
                  onChange={(e) => setCustomRoleName(e.target.value)}
                  placeholder="Role name"
                  className="w-full text-[12.5px] font-semibold bg-transparent outline-none border-b border-overlay/10 focus:border-accent pb-0.5"
                />
                <textarea
                  value={customRoleDesc}
                  onChange={(e) => setCustomRoleDesc(e.target.value)}
                  placeholder="One-paragraph job description (baked into the agent's preamble)"
                  rows={3}
                  className="w-full text-[12px] text-text-secondary bg-transparent outline-none ring-1 ring-overlay/[0.08] rounded-lg px-2 py-1.5 resize-none focus:ring-accent"
                />
                {allSkills.filter((s) => (s.kind === "builtin" && !s.mandatory) || s.kind === "custom").length > 0 && (
                  <div>
                    <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1">
                      Default skills
                    </div>
                    <div className="space-y-1 max-h-32 overflow-y-auto">
                      {allSkills
                        .filter((s) => (s.kind === "builtin" && !s.mandatory) || s.kind === "custom")
                        .map((s) => {
                          const checked = customRoleSkillIds.includes(s.id);
                          return (
                            <label
                              key={s.id}
                              className="flex items-center gap-2 text-[12px] text-text-secondary cursor-pointer"
                            >
                              <input
                                type="checkbox"
                                checked={checked}
                                onChange={(e) =>
                                  setCustomRoleSkillIds((prev) =>
                                    e.target.checked
                                      ? [...prev, s.id]
                                      : prev.filter((id) => id !== s.id),
                                  )
                                }
                              />
                              {s.name}
                            </label>
                          );
                        })}
                    </div>
                  </div>
                )}
                <div className="flex items-center justify-end gap-2 pt-0.5">
                  <button
                    onClick={() => {
                      setCustomRoleOpen(false);
                      setCustomRoleName("");
                      setCustomRoleDesc("");
                      setCustomRoleSkillIds([]);
                    }}
                    className="text-[12px] font-medium text-text-secondary px-3 py-1 rounded-lg hover:bg-overlay/[0.05]"
                  >
                    Cancel
                  </button>
                  <button
                    onClick={handleCreateCustomRole}
                    disabled={savingRole}
                    className="text-[12px] font-semibold text-white bg-accent px-3 py-1 rounded-lg hover:brightness-105 disabled:opacity-60"
                  >
                    {savingRole ? "Creating…" : "Create role"}
                  </button>
                </div>
              </div>
            )}
          </section>

          {positionEnabled && (
            <section>
              <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-2">
                Position
              </div>

              <div className="rounded-xl ring-1 ring-overlay/[0.08] bg-surface p-3 space-y-3">
                <div>
                  <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
                    Track
                  </div>
                  <div className="rounded-lg bg-overlay/[0.04] px-3 py-2">
                    <PositionLine
                      levelId={levelDraft}
                      track={trackLabel}
                      compact={false}
                      supervisor={
                        supervisorDraft
                          ? (() => {
                              const next = positionRoster.find(
                                (agent) => agent.id === supervisorDraft,
                              );
                              return next
                                ? { name: next.name ?? next.id }
                                : null;
                            })()
                          : null
                      }
                      showReportsTo
                    />
                  </div>
                </div>

                <div>
                  <div className="flex items-center justify-between mb-1.5">
                    <span className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase">
                      Level
                    </span>
                    <button
                      type="button"
                      onClick={() => setLevelDraft(null)}
                      className={`text-[11px] font-medium ${
                        levelDraft == null
                          ? "text-accent"
                          : "text-text-tertiary hover:text-text-secondary"
                      }`}
                    >
                      Clear to Unranked
                    </button>
                  </div>
                  <div className="grid grid-cols-4 gap-2">
                    {LEVELS.map((level) => {
                      const active = levelDraft === level.id;
                      return (
                        <button
                          key={level.id}
                          type="button"
                          onClick={() => setLevelDraft(level.id)}
                          className={`rounded-xl px-2.5 py-2 text-left transition-all ring-1 ${
                            active
                              ? "ring-accent/40 bg-accent/[0.06]"
                              : "ring-overlay/[0.08] bg-surface hover:bg-overlay/[0.02]"
                          }`}
                        >
                          <div className="text-[11.5px] font-semibold leading-tight">{level.name}</div>
                          <div className="mt-1 text-[11px] text-text-tertiary">
                            rung {level.rung}
                          </div>
                        </button>
                      );
                    })}
                  </div>
                </div>

                <div>
                  <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
                    Supervisor
                  </div>
                  <div className="space-y-1.5 max-h-40 overflow-y-auto pr-1">
                    <button
                      type="button"
                      onClick={() => setSupervisorDraft(null)}
                      className={`w-full rounded-lg px-2.5 py-2 text-left transition-all ring-1 ${
                        supervisorDraft == null
                          ? "ring-accent/40 bg-accent/[0.06]"
                          : "ring-overlay/[0.08] bg-surface hover:bg-overlay/[0.02]"
                      }`}
                    >
                      <div className="flex items-center gap-2">
                        <HumanChip />
                        <span className="text-[11px] text-text-tertiary">Top of the chain</span>
                      </div>
                    </button>
                    {supervisorOptions.map((agent) => {
                      const disabled = wouldCycle(scopedAgent!.id, agent.id, positionRoster);
                      const active = supervisorDraft === agent.id;
                      return (
                        <button
                          key={agent.id}
                          type="button"
                          onClick={() => !disabled && setSupervisorDraft(agent.id)}
                          disabled={disabled}
                          className={`w-full rounded-lg px-2.5 py-2 text-left transition-all ring-1 ${
                            active
                              ? "ring-accent/40 bg-accent/[0.06]"
                              : "ring-overlay/[0.08] bg-surface hover:bg-overlay/[0.02]"
                          } ${disabled ? "opacity-50 cursor-not-allowed" : ""}`}
                        >
                          <div className="text-[12px] font-semibold leading-tight">
                            {agent.name ?? agent.id}
                          </div>
                          <PositionLine
                            levelId={agent.level}
                            track={agent.roleName ?? "Agent"}
                            compact
                            className="mt-1"
                          />
                          {disabled && (
                            <div className="mt-1 text-[10.5px] text-text-tertiary">
                              Self and descendants cannot supervise this member
                            </div>
                          )}
                        </button>
                      );
                    })}
                  </div>
                </div>

                <div>
                  <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
                    Escalation chain
                  </div>
                  <div className="rounded-lg bg-overlay/[0.04] px-3 py-2">
                    <div className="flex items-center gap-1.5 flex-wrap text-[11.5px] text-text-secondary">
                      {previewChainIds.map((id, index) => {
                        const agent = previewRoster.find((item) => item.id === id);
                        return (
                          <span key={id} className="inline-flex items-center gap-1.5">
                            {index > 0 && <span className="text-text-tertiary">→</span>}
                            <span className="font-medium text-text-primary">
                              {agent?.name ?? id}
                            </span>
                            <span className="text-text-tertiary">
                              ({agent?.level ? levelOf(agent.level).short : "Unranked"})
                            </span>
                          </span>
                        );
                      })}
                      {previewChainIds.length > 0 && (
                        <>
                          <span className="text-text-tertiary">→</span>
                          <HumanChip label />
                        </>
                      )}
                    </div>
                  </div>
                </div>
              </div>
            </section>
          )}

          {/* Type */}
          <section>
            <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-2">
              Type
            </div>
            <div className="grid grid-cols-3 gap-2">
              {(
                [
                  { value: "cli", label: "CLI agent", Icon: Terminal, soon: false },
                  { value: "chat", label: "Chat agent", Icon: MessageSquare, soon: true },
                  { value: "orchestrator", label: "Orchestrator", Icon: Waypoints, soon: true },
                ] as { value: AgentType; label: string; Icon: typeof Terminal; soon: boolean }[]
              ).map(({ value, label, Icon, soon }) => {
                const active = agentType === value;
                // Chat / Orchestrator aren't ready yet — shown disabled with a
                // "Soon" badge so only CLI agents can be created for now.
                return (
                  <button
                    key={value}
                    onClick={() => !soon && setAgentType(value)}
                    disabled={soon}
                    aria-disabled={soon}
                    className={`relative rounded-xl p-2 text-left transition-all ${
                      soon
                        ? "ring-1 ring-overlay/[0.06] bg-overlay/[0.02] opacity-60 cursor-not-allowed"
                        : active
                          ? "ring-1 ring-accent/40 bg-accent/[0.06]"
                          : "ring-1 ring-overlay/[0.08] bg-surface hover:bg-overlay/[0.02]"
                    }`}
                  >
                    {soon && (
                      <span className="absolute top-1.5 right-1.5 text-[9px] font-bold tracking-wide text-text-muted bg-overlay/[0.06] px-1.5 py-px rounded-full uppercase">
                        Soon
                      </span>
                    )}
                    <Icon
                      className={`w-4 h-4 mb-1 ${
                        soon ? "text-text-tertiary" : active ? "text-accent" : "text-text-secondary"
                      }`}
                    />
                    <div className="text-[12px] font-semibold">{label}</div>
                  </button>
                );
              })}
            </div>

            {/* CLI Kind (only when type = cli) */}
            {agentType === "cli" && (
              // Segmented control (matches the Permission/Context pickers). Custom
              // CLI isn't wired in the backend yet, so it's shown disabled.
              <div
                role="radiogroup"
                aria-label="CLI kind"
                className="mt-2 grid grid-cols-4 gap-1 rounded-xl bg-overlay/[0.04] p-1"
              >
                {(
                  [
                    { value: "claude-code", label: "Claude Code", soon: false },
                    { value: "codex", label: "Codex", soon: false },
                    { value: "antigravity", label: "Antigravity", soon: false },
                    { value: "custom", label: "Custom", soon: true },
                  ] as { value: CliKind; label: string; soon: boolean }[]
                ).map(({ value, label, soon }) => (
                  <button
                    key={value}
                    role="radio"
                    aria-checked={cliKind === value}
                    disabled={soon}
                    onClick={() => !soon && selectCliKind(value)}
                    className={`min-w-0 rounded-lg px-1 py-1.5 text-[11.5px] transition-colors ${
                      soon
                        ? "text-text-tertiary cursor-not-allowed"
                        : cliKind === value
                          ? "bg-surface shadow-sm font-semibold"
                          : "text-text-secondary hover:bg-overlay/[0.03]"
                    }`}
                  >
                    <span className="block truncate">{label}</span>
                    {soon && (
                      <span className="block text-[8px] font-bold tracking-wide text-text-muted uppercase">
                        Soon
                      </span>
                    )}
                  </button>
                ))}
              </div>
            )}
          </section>

          {/* Model / API — for first-class CLI harnesses the model lives in the CLI
              config section below (with its presets), so it's hidden here to
              avoid two disconnected model fields. */}
          {!showCliConfig && (
            <section>
              <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-2">
                Model
              </div>
              <div className="rounded-xl ring-1 ring-overlay/[0.08] bg-surface divide-y divide-overlay/[0.06]">
                <div className="flex items-center justify-between px-3 py-2">
                  <span className="text-[12.5px] text-text-secondary">Provider</span>
                  {/* TODO(M5): real provider picker wired to provider.upsert */}
                  <span className="text-[12.5px] text-text-tertiary">Configure in Settings</span>
                </div>
                <div className="flex items-center justify-between px-3 py-2">
                  <span className="text-[12.5px] text-text-secondary">Model</span>
                  <input
                    value={model}
                    onChange={(e) => setModel(e.target.value)}
                    placeholder="e.g. claude-opus-4-8"
                    className="text-[12.5px] text-right bg-transparent outline-none w-44 placeholder:text-text-quaternary"
                  />
                </div>
              </div>
            </section>
          )}

          {/* CLI launch config — for first-class CLI harnesses. */}
          {showCliConfig && (
            <section>
              <div className="mb-2 flex items-center justify-between gap-3">
                <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase">
                  {isAntigravity ? "Antigravity" : isCodex ? "Codex" : "Claude Code"}
                </div>
                {isAntigravity && (
                  <span
                    className={`inline-flex items-center gap-1 text-[10px] font-medium ${
                      cliAvailability.state === "missing" || cliAvailability.state === "error"
                        ? "text-danger"
                        : "text-text-muted"
                    }`}
                  >
                    {cliAvailability.state === "checking" ? (
                      <RefreshCw className="h-3 w-3 animate-spin motion-reduce:animate-none" />
                    ) : cliAvailability.state === "available" ? (
                      <Check className="h-3 w-3 text-success" />
                    ) : cliAvailability.state === "missing" || cliAvailability.state === "error" ? (
                      <AlertTriangle className="h-3 w-3" />
                    ) : null}
                    {cliAvailability.state === "checking"
                      ? "Checking agy…"
                      : cliAvailability.state === "available"
                        ? "agy available"
                        : cliAvailability.state === "missing"
                          ? "agy not found"
                          : cliAvailability.state === "error"
                            ? "Check failed"
                            : ""}
                  </span>
                )}
              </div>

              {isAntigravity && cliAvailability.state === "missing" && (
                <div role="alert" className="mb-2 rounded-xl bg-danger/[0.09] px-3 py-2.5 text-danger">
                  <div className="flex items-start gap-2">
                    <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
                    <div className="min-w-0 flex-1">
                      <div className="text-[11.5px] font-semibold">
                        Antigravity CLI is not available
                      </div>
                      <p className="mt-0.5 text-[10.5px] leading-relaxed">
                        Install the CLI from the Antigravity documentation, then make sure{" "}
                        <span className="font-mono">agy</span> is on your login-shell PATH.
                      </p>
                      <div className="mt-2 flex flex-wrap items-center gap-2">
                        <button
                          type="button"
                          onClick={() => void openAntigravityInstallGuide()}
                          title={cliAvailability.installUrl}
                          aria-label="Open Antigravity installation guide"
                          className="inline-flex h-7 items-center gap-1.5 rounded-md bg-danger px-2.5 text-[10.5px] font-semibold text-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-danger"
                        >
                          <CircleHelp className="h-3 w-3" /> Installation guide
                        </button>
                        <button
                          type="button"
                          onClick={() => void checkAntigravityAvailability()}
                          className="inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-[10.5px] font-semibold text-danger hover:bg-danger/[0.08] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-danger"
                        >
                          <RefreshCw className="h-3 w-3" /> Check again
                        </button>
                      </div>
                    </div>
                  </div>
                </div>
              )}

              {isAntigravity && cliAvailability.state === "error" && (
                <div role="alert" className="mb-2 rounded-xl bg-warning/[0.09] px-3 py-2.5 text-warning">
                  <div className="flex items-start gap-2">
                    <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
                    <div className="min-w-0 flex-1">
                      <div className="text-[11.5px] font-semibold">Couldn’t check Antigravity CLI</div>
                      <p className="mt-0.5 text-[10.5px] leading-relaxed">
                        Conclave couldn’t query your login shell. Check its configuration, then try again.
                      </p>
                      <button
                        type="button"
                        onClick={() => void checkAntigravityAvailability()}
                        title={cliAvailability.message}
                        className="mt-2 inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-[10.5px] font-semibold hover:bg-warning/[0.08] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-warning"
                      >
                        <RefreshCw className="h-3 w-3" /> Check again
                      </button>
                    </div>
                  </div>
                </div>
              )}

              <div className="rounded-xl ring-1 ring-overlay/[0.08] bg-surface divide-y divide-overlay/[0.06]">
                {/* Model — field + quick-presets together so picking a preset
                    visibly fills the same field. */}
                <div className="px-3 py-2">
                  <div className="flex items-center justify-between gap-3">
                    <label htmlFor="cli-model" className="text-[12.5px] text-text-secondary shrink-0">
                      Model
                    </label>
                    {isAntigravity ? (
                      // Antigravity's models are discovered from the user's own
                      // authenticated CLI, so there is nothing to type and no
                      // list to hardcode — same native select as Execution mode.
                      // Fixed width, not min-width: a native select sizes
                      // itself to its WIDEST option, so one long model id would
                      // otherwise stretch this control across the row. 240px is
                      // the smallest width that shows "Auto (authenticated
                      // default)" (158px of text) whole; longer labels clip, and
                      // the exact id stays reachable through the option title
                      // and the hint line below.
                      <div className="relative w-[240px] shrink-0">
                        <select
                          id="cli-model"
                          value={model}
                          disabled={modelCatalog.state === "loading"}
                          onChange={(event) => setModel(event.target.value)}
                          title={model || "Auto (authenticated default)"}
                          className="h-7 w-full appearance-none rounded-lg bg-overlay/[0.04] pl-2.5 pr-7 text-[11.5px] font-medium text-text-primary outline-none focus-visible:ring-2 focus-visible:ring-accent disabled:text-text-muted"
                        >
                          <option value="">Auto (authenticated default)</option>
                          {savedModelUnlisted && (
                            <option value={model} title={model}>
                              {modelCatalog.state === "ready" ? `${model} (unavailable)` : model}
                            </option>
                          )}
                          {catalogModels.map((entry) => (
                            <option key={entry.id} value={entry.id} title={entry.id}>
                              {entry.label}
                            </option>
                          ))}
                        </select>
                        <ChevronDown className="pointer-events-none absolute right-2 top-2 h-3 w-3 text-text-muted" />
                      </div>
                    ) : (
                      <input
                        id="cli-model"
                        value={model}
                        onChange={(e) => setModel(e.target.value)}
                        placeholder={isCodex ? "gpt-5.5" : "claude-opus-4-8"}
                        className="min-w-0 flex-1 bg-transparent text-right font-mono text-[12.5px] outline-none placeholder:text-text-tertiary focus-visible:rounded-sm focus-visible:ring-2 focus-visible:ring-accent"
                      />
                    )}
                  </div>
                  {isAntigravity ? (
                    <div className="mt-1.5 text-[10px] leading-relaxed text-text-tertiary">
                      {modelCatalog.state === "loading" ? (
                        <span className="inline-flex items-center gap-1.5">
                          <RefreshCw className="h-3 w-3 animate-spin motion-reduce:animate-none" />
                          Loading your authenticated models…
                        </span>
                      ) : modelCatalog.state === "error" ? (
                        <span className="inline-flex flex-wrap items-center gap-1.5 text-warning">
                          Couldn’t load your Antigravity models.
                          <button
                            type="button"
                            onClick={() => void loadAntigravityModels()}
                            className="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 font-semibold hover:bg-warning/[0.08] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-warning"
                          >
                            <RefreshCw className="h-3 w-3" /> Retry
                          </button>
                        </span>
                      ) : savedModelUnlisted ? (
                        <span className="text-warning">
                          <span className="font-mono">{model}</span> isn’t in your authenticated
                          models. It is kept until you pick another.
                        </span>
                      ) : model ? (
                        <>
                          Launches as <span className="font-mono">{model}</span>.
                        </>
                      ) : (
                        "Auto lets Antigravity choose from your authenticated models."
                      )}
                    </div>
                  ) : (
                    <div className="flex flex-wrap gap-1.5 mt-2">
                      {modelPresets.map((m) => (
                        <button
                          key={m}
                          onClick={() => selectModelPreset(m)}
                          className={`text-[11px] font-mono px-2 py-0.5 rounded-md ring-1 transition-colors ${
                            model === m
                              ? "ring-accent/40 bg-accent/[0.08] text-accent"
                              : "ring-overlay/[0.08] text-text-secondary hover:bg-overlay/[0.03]"
                          }`}
                        >
                          {m.replace("claude-", "")}
                        </button>
                      ))}
                    </div>
                  )}
                </div>

                {showCliConfig && (
                  <div className="px-3 py-2">
                    <div className="flex items-center justify-between gap-3">
                      <span className="text-[12.5px] text-text-secondary">Effort</span>
                      <div
                        role="radiogroup"
                        aria-label="Effort"
                        className="flex rounded-lg bg-overlay/[0.04] p-0.5"
                      >
                        {(
                          [
                            { value: undefined, label: "Auto" },
                            { value: "low", label: "low" },
                            { value: "medium", label: "medium" },
                            { value: "high", label: "high" },
                          ] as { value: CliEffort; label: string }[]
                        ).map(({ value, label }) => (
                          <button
                            key={label}
                            type="button"
                            role="radio"
                            aria-checked={effort === value}
                            onClick={() => setEffort(value)}
                            className={`rounded-[7px] px-2.5 py-1 text-[11.5px] transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                              effort === value
                                ? "bg-surface font-semibold text-text-primary shadow-sm"
                                : "text-text-secondary hover:text-text-primary"
                            }`}
                          >
                            {label}
                          </button>
                        ))}
                      </div>
                    </div>
                    <p className="mt-1.5 text-[10px] leading-relaxed text-text-tertiary">
                      Auto uses the provider default and omits the effort override.
                    </p>
                  </div>
                )}

                {/* Permission mode */}
                {isAntigravity ? (
                  <div className="px-3 py-2">
                    <div className="flex items-center justify-between gap-3">
                      <label
                        htmlFor="antigravity-execution-mode"
                        className="shrink-0 text-[12.5px] text-text-secondary"
                      >
                        Execution mode
                      </label>
                      <div className="relative min-w-[190px]">
                        <select
                          id="antigravity-execution-mode"
                          value={permissionMode === "auto" ? "default" : permissionMode}
                          onChange={(event) => {
                            setPermissionMode(event.target.value as PermissionMode);
                            setPermissionModeDirty(true);
                          }}
                          className={`h-7 w-full appearance-none rounded-lg bg-overlay/[0.04] pl-2.5 pr-7 text-[11.5px] font-medium outline-none focus-visible:ring-2 focus-visible:ring-accent ${
                            permissionMode === "bypassPermissions"
                              ? "text-danger"
                              : "text-text-primary"
                          }`}
                        >
                          <option value="default">Default</option>
                          <option value="acceptEdits">Accept edits</option>
                          <option value="plan">Plan</option>
                          <option value="bypassPermissions">Bypass permissions</option>
                        </select>
                        <ChevronDown className="pointer-events-none absolute right-2 top-2 h-3 w-3 text-text-muted" />
                      </div>
                    </div>
                    <p
                      className={`mt-1.5 text-[10px] leading-relaxed ${
                        permissionMode === "bypassPermissions"
                          ? "text-danger"
                          : "text-text-tertiary"
                      }`}
                    >
                      {
                        ANTIGRAVITY_MODE_HELP[
                          permissionMode === "auto" ? "default" : permissionMode
                        ]
                      }
                    </p>
                    {permissionMode === "bypassPermissions" && (
                      <div className="mt-2 flex items-start gap-2 rounded-lg bg-danger/[0.09] px-2.5 py-2 text-[10.5px] leading-relaxed text-danger">
                        <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                        <span>
                          <strong>Use only in workspaces you trust.</strong> This disables
                          Antigravity permission checks.
                        </span>
                      </div>
                    )}
                  </div>
                ) : (
                  <div className="px-3 py-2">
                    <div className="flex items-center justify-between">
                      <span className="text-[12.5px] text-text-secondary">Permission mode</span>
                      <div
                        role="radiogroup"
                        aria-label="Permission mode"
                        className="flex rounded-lg bg-overlay/[0.04] p-0.5"
                      >
                        {(
                          [
                            { value: "auto", label: "Auto" },
                            { value: "bypassPermissions", label: "Bypass" },
                          ] as { value: PermissionMode; label: string }[]
                        ).map(({ value, label }) => (
                          <button
                            key={value}
                            role="radio"
                            aria-checked={permissionMode === value}
                            onClick={() => {
                              setPermissionMode(value);
                              setPermissionModeDirty(true);
                            }}
                            className={`text-[12px] px-2.5 py-1 rounded-[7px] transition-colors ${
                              permissionMode === value
                                ? "bg-surface shadow-sm font-semibold"
                                : "text-text-secondary"
                            }`}
                            title={
                              isCodex
                                ? value === "auto"
                                  ? "codex --ask-for-approval never"
                                  : "codex --yolo"
                                : `claude --permission-mode ${value}`
                            }
                          >
                            {label}
                          </button>
                        ))}
                      </div>
                    </div>
                    {permissionMode === "bypassPermissions" && (
                      <p className="text-[10.5px] text-warning mt-1.5">
                        Skips every permission prompt — use only in workspaces you trust.
                      </p>
                    )}
                  </div>
                )}

                {/* Context window — Claude's [1m] suffix remains a segmented
                    choice; Codex uses a numeric model_context_window override. */}
                {isClaudeCode && (
                <div className="px-3 py-2">
                  <div className="flex items-center justify-between">
                    <span className="text-[12.5px] text-text-secondary">Context window</span>
                    <div
                      role="radiogroup"
                      aria-label="Context window"
                      className="flex rounded-lg bg-overlay/[0.04] p-0.5"
                    >
                      {(
                        [
                          { value: "200k", label: "200K" },
                          { value: "1m", label: "1M" },
                        ] as { value: ClaudeContextWindow; label: string }[]
                      ).map(({ value, label }) => (
                        <button
                          key={value}
                          role="radio"
                          aria-checked={contextWindow === value}
                          onClick={() => setContextWindow(value)}
                          className={`text-[12px] px-2.5 py-1 rounded-[7px] transition-colors ${
                            contextWindow === value
                              ? "bg-surface shadow-sm font-semibold"
                              : "text-text-secondary"
                          }`}
                        >
                          {label}
                        </button>
                      ))}
                    </div>
                  </div>
                  {contextWindow === "1m" && (
                    <p className="text-[10.5px] text-text-tertiary mt-1.5">
                      Launches as <span className="font-mono">{model || "model"}[1m]</span>.
                    </p>
                  )}
                </div>
                )}

                {isCodex && (
                  <div className="px-3 py-2">
                    <div className="flex items-center justify-between">
                      <span className="text-[12.5px] text-text-secondary">Context window</span>
                      <span className="text-[12px] text-text-tertiary">Auto</span>
                    </div>
                    <p className="text-[10.5px] text-text-tertiary mt-1.5">
                      Derived from the model — no manual override.
                    </p>
                  </div>
                )}

                {/* Token filter (rtk) — Claude Code + Codex; absent/null = ON. */}
                {(isClaudeCode || isCodex) && (
                <div className="px-3 py-2">
                  <div className="flex items-center justify-between">
                    <span className="text-[12.5px] text-text-secondary">Token filter (rtk)</span>
                    <Toggle on={rtkEnabled} onChange={setRtkEnabled} label="Token filter (rtk)" />
                  </div>
                  <p className="text-[10.5px] text-text-tertiary mt-1.5">
                    Rewrites shell commands through rtk to compress output and save tokens.
                  </p>
                </div>
                )}

                {/* Custom args */}
                <div className="flex items-center justify-between px-3 py-2.5 gap-3">
                  <span className="text-[12.5px] text-text-secondary shrink-0">Custom args</span>
                  <input
                    value={customArgs}
                    onChange={(e) => setCustomArgs(e.target.value)}
                    placeholder="e.g. --verbose --mcp-config ./mcp.json"
                    className="text-[12px] font-mono text-right bg-transparent outline-none flex-1 placeholder:text-text-quaternary"
                  />
                </div>

                {/* Custom env (opt-in) — Claude Code only. Codex is configured
                    via its own config.toml / -c flags, not ANTHROPIC_* env. */}
                {isClaudeCode && (
                <div className="px-3 py-2">
                  <div className="flex items-center justify-between">
                    <span className="text-[12.5px] text-text-secondary">Custom environment</span>
                    <Toggle
                      on={useCustomEnv}
                      onChange={setUseCustomEnv}
                      label="Use custom environment"
                    />
                  </div>
                  {useCustomEnv && (
                    <>
                      <textarea
                        value={envText}
                        onChange={(e) => setEnvText(e.target.value)}
                        spellCheck={false}
                        rows={8}
                        className="mt-2 w-full rounded-lg ring-1 ring-overlay/[0.1] bg-fill-softer focus:ring-accent/50 outline-none px-2.5 py-2 text-[11.5px] font-mono leading-relaxed resize-y"
                      />
                      <p className="text-[10.5px] text-text-tertiary mt-1.5">
                        Secrets (AUTH_TOKEN / API_KEY / …) are stored in the macOS Keychain, never
                        in the database. Leave a value as{" "}
                        <span className="font-mono">{SECRET_PLACEHOLDER}</span> to keep the stored
                        secret.
                      </p>
                    </>
                  )}
                </div>
                )}
              </div>
              {isAntigravity && (
                <p className="mt-2 text-[10px] leading-relaxed text-text-tertiary">
                  Token filtering and sandbox controls are not available for Antigravity in this
                  version.
                </p>
              )}
            </section>
          )}

          {/* Skills — for every first-class CLI harness. */}
          {showCliConfig && (
            <section>
              <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-2">
                Skills
              </div>
              <div className="rounded-xl ring-1 ring-overlay/[0.08] bg-surface divide-y divide-overlay/[0.06]">
                {allSkills.filter((s) => s.kind === "builtin" && s.mandatory).length > 0 && (
                  <div className="px-3 py-2">
                    <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
                      System skills — always on
                    </div>
                    <div className="flex flex-wrap gap-1.5">
                      {allSkills
                        .filter((s) => s.kind === "builtin" && s.mandatory)
                        .map((s) => (
                          <span
                            key={s.id}
                            className="text-[11px] font-medium px-2 py-0.5 rounded-md ring-1 ring-overlay/[0.08] text-text-secondary"
                          >
                            {s.name}
                          </span>
                        ))}
                    </div>
                  </div>
                )}
                {allSkills.filter((s) => s.kind === "builtin" && !s.mandatory).length > 0 && (
                  <div className="px-3 py-2">
                    <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
                      System skills — optional
                    </div>
                    <div className="space-y-1">
                      {allSkills
                        .filter((s) => s.kind === "builtin" && !s.mandatory)
                        .map((s) => {
                          const checked = skillIds.includes(s.id);
                          return (
                            <label
                              key={s.id}
                              className="flex items-center gap-2 text-[12.5px] text-text-secondary cursor-pointer"
                            >
                              <input
                                type="checkbox"
                                checked={checked}
                                onChange={(e) =>
                                  setSkillIds((prev) =>
                                    e.target.checked ? [...prev, s.id] : prev.filter((id) => id !== s.id),
                                  )
                                }
                              />
                              {s.name}
                            </label>
                          );
                        })}
                    </div>
                  </div>
                )}
                <div className="px-3 py-2">
                  <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
                    Custom skills
                  </div>
                  {allSkills.filter((s) => s.kind === "custom").length === 0 ? (
                    <p className="text-[11.5px] text-text-tertiary">
                      No custom skills yet — create one in the Skill Library.
                    </p>
                  ) : (
                    <div className="space-y-1">
                      {allSkills
                        .filter((s) => s.kind === "custom")
                        .map((s) => {
                          const checked = skillIds.includes(s.id);
                          return (
                            <label
                              key={s.id}
                              className="flex items-center gap-2 text-[12.5px] text-text-secondary cursor-pointer"
                            >
                              <input
                                type="checkbox"
                                checked={checked}
                                onChange={(e) =>
                                  setSkillIds((prev) =>
                                    e.target.checked ? [...prev, s.id] : prev.filter((id) => id !== s.id),
                                  )
                                }
                              />
                              {s.name}
                            </label>
                          );
                        })}
                    </div>
                  )}
                </div>
              </div>
            </section>
          )}

          {/* Error */}
          {error && (
            <p className="text-[12px] text-danger px-1">{error}</p>
          )}
        </div>

        {/* ── Footer actions ── */}
        <div className="border-t border-overlay/[0.07] px-5 py-2.5 bg-surface shrink-0 flex items-center justify-end gap-2">
          <button
            onClick={onClose}
            className="text-[12.5px] font-medium text-text-secondary px-3.5 py-1.5 rounded-lg hover:bg-overlay/[0.05]"
          >
            Cancel
          </button>
          <button
            onClick={handleSave}
            disabled={saving || antigravitySaveBlocked}
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
  );
}
