//! `draft.agents` — the AI agent & team drafter (spec
//! `docs/superpowers/specs/2026-09-04-ai-agent-team-drafter-design.md`).
//!
//! This module owns the WIRE CONTRACT: the request/response types below are
//! mirrored field-for-field in `src/ipc/types.ts` (Lane C). Any field change
//! goes through a task challenge first — the two lanes copy this shape, they
//! never "improve" it independently.
//!
//! The model may only pick ids that exist (spec D4): `build_catalogue` reads
//! the live roles/skills/definitions/roster, `build_prompt` embeds them, and
//! `validate_draft` rejects anything else with the offending field named. There
//! is no silent coercion.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};
use sqlx::SqlitePool;

use super::draft_prompt::build_prompt;
use super::usage::{record_collected_event, CollectedEvent};
use crate::engine::repo::{self, role::RoleRow, skill::SkillRow};
use crate::engine::runtime::cli_oneshot::{CliKind, Oneshot, OneshotOutcome, OneshotSpec};
use crate::engine::runtime::launch_common::{agent_env_overrides, effective_claude_model};
use crate::engine::runtime::usage::{provider_for_cli_kind, SOURCE_DRAFT};
use crate::engine::{AppError, AppState};

// ── Catalogue constants ──────────────────────────────────────────────────────

/// Human request 2026-09-04: add the Claude 5 family (Fable 5.1, Opus 5,
/// Sonnet 5, Haiku 4.5) shown in Claude Code's picker; keep opus-4-8 so
/// existing rows stay valid. Mirrored by `CLAUDE_MODELS` in
/// `src/lib/modelCatalogue.ts` (Lane C).
pub const CLAUDE_MODELS: &[&str] = &[
    "claude-fable-5-1",
    "claude-opus-5",
    "claude-sonnet-5",
    "claude-haiku-4-5",
    "claude-opus-4-8",
];

/// Codex presets, verbatim from `src/components/Builder.tsx`. The context
/// window is NOT drafted — the backend derives it per model
/// (`codex_models::codex_model_context_window`).
pub const CODEX_MODELS: &[&str] = &[
    "gpt-6-astra",
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
    "gpt-5.5",
    "gpt-5.4",
    "gpt-5.4-mini",
    "gpt-5.3-codex-spark",
];

/// Verbatim from `src/components/Builder.tsx:41-47`.
pub const COLOR_SWATCHES: &[&str] = &[
    "#5e5ce6", "#0a84ff", "#d6409f", "#30d158", "#ff9f0a", "#0fa3a3", "#ff3b30",
];

/// Position levels, in seniority order (mirrors `lib/positions.ts`).
pub const LEVELS: &[&str] = &["junior", "mid", "senior", "principal"];

/// Team-mode cap (spec: "Team mode: 1..12 agents").
pub const MAX_TEAM_SIZE: usize = 12;

/// Free-text caps. The JSON schema DECLARES these (`maxLength`) and
/// `validate_draft` RE-CHECKS them: a schema is a request, not a guarantee —
/// codex gets no `--output-schema` at all (spec R2 ruling), and even claude's
/// structured output is the model's word for it. Both read these constants so
/// the two can never disagree.
pub const MAX_NAME_CHARS: usize = 40;
pub const MAX_RATIONALE_CHARS: usize = 200;
pub const MAX_ROLE_DESCRIPTION_CHARS: usize = 600;
pub const MAX_NOTES_CHARS: usize = 600;

/// Brief length cap (spec R3: keeps the whole prompt comfortably small).
pub const BRIEF_MAX_CHARS: usize = 4000;

/// The model ids a given `cliKind` may be drafted with.
fn models_for(cli_kind: &str) -> Option<&'static [&'static str]> {
    match cli_kind {
        "claude-code" => Some(CLAUDE_MODELS),
        "codex" => Some(CODEX_MODELS),
        _ => None,
    }
}

// ── Wire contract ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DraftMode {
    Agent,
    Team,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftRequest {
    pub mode: DraftMode,
    pub brief: String,
    pub drafter_def_id: String,
    #[serde(default)]
    pub workspace_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftNewRole {
    pub name: String,
    pub description: String,
    pub skill_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftAgent {
    /// Draft-local handle, unique within the draft; `positions` reference it.
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_agent_def_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_role: Option<DraftNewRole>,
    #[serde(default)]
    pub skill_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_level: Option<String>,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftPosition {
    pub key: String,
    pub level: String,
    #[serde(default)]
    pub supervisor_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DrafterInfo {
    pub def_id: String,
    pub cli_kind: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftResponse {
    pub agents: Vec<DraftAgent>,
    pub positions: Vec<DraftPosition>,
    #[serde(default)]
    pub notes: String,
    pub drafter: DrafterInfo,
}

// ── Catalogue ────────────────────────────────────────────────────────────────

/// One line of the workspace's current roster, for the team-mode prompt.
pub struct RosterLine {
    pub name: String,
    pub role_name: Option<String>,
    pub level: Option<String>,
    pub supervisor_name: Option<String>,
}

/// An agent definition the draft may reuse via `existingAgentDefId`.
pub struct ExistingDef {
    pub id: String,
    pub name: String,
    pub role_name: Option<String>,
    pub cli_kind: Option<String>,
    pub model: Option<String>,
}

/// Everything the model is allowed to choose from, and everything the
/// validator checks against. Built once per request.
pub struct Catalogue {
    pub roles: Vec<RoleRow>,
    /// Optional skills only — mandatory builtins attach to every agent anyway,
    /// so listing them would only invite the model to "choose" them.
    pub skills: Vec<SkillRow>,
    pub existing: Vec<ExistingDef>,
    pub roster: Vec<RosterLine>,
}

pub async fn build_catalogue(
    db: &SqlitePool,
    workspace_id: Option<&str>,
) -> Result<Catalogue, AppError> {
    let roles: Vec<RoleRow> = repo::role::list_builtin()
        .into_iter()
        .chain(repo::role::list(db).await?)
        .collect();

    let skills: Vec<SkillRow> = repo::skill::list_builtin()
        .into_iter()
        .chain(repo::skill::list(db).await?)
        .filter(|s| !s.mandatory)
        .collect();

    let existing: Vec<ExistingDef> = repo::agent_definition::list(db)
        .await?
        .into_iter()
        .map(|d| ExistingDef {
            role_name: d
                .role_id
                .as_deref()
                .and_then(|rid| roles.iter().find(|r| r.id == rid))
                .map(|r| r.name.clone())
                .or_else(|| d.role.clone()),
            id: d.id,
            name: d.name,
            cli_kind: d.cli_kind,
            model: d.model,
        })
        .collect();

    // The roster needs `level` / `supervisor_name`, which the plain
    // `workspace_agent` row does not carry (they live on the table but not on
    // `WorkspaceAgentRow`) — the roster query already resolves both plus the
    // definition's display name, so it is the source here.
    let roster = match workspace_id {
        Some(ws) => repo::workspace_agent::list_by_workspace_with_launched_skills(db, ws)
            .await?
            .into_iter()
            .map(|r| RosterLine {
                name: r.name,
                role_name: r.role_name,
                level: r.level,
                supervisor_name: r.supervisor_name,
            })
            .collect(),
        None => Vec::new(),
    };

    Ok(Catalogue {
        roles,
        skills,
        existing,
        roster,
    })
}

// ── JSON schema ──────────────────────────────────────────────────────────────

/// The schema handed to `claude --json-schema` / `codex --output-schema` AND
/// embedded in the prompt, so the model reads the field docs as instructions.
pub fn draft_schema(mode: DraftMode) -> Value {
    let agent = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["key", "rationale"],
        "properties": {
            "key": {"type": "string", "description": "Draft-local handle, unique, e.g. \"lead\", \"impl-1\"."},
            "existingAgentDefId": {"type": "string", "description": "Reuse this existing agent definition id. When set, give NO other field besides key and rationale."},
            "name": {"type": "string", "maxLength": MAX_NAME_CHARS},
            "color": {"type": "string", "enum": COLOR_SWATCHES},
            "cliKind": {"type": "string", "enum": ["claude-code", "codex"]},
            "model": {"type": "string", "description": "A model id from the catalogue for the chosen cliKind."},
            "roleId": {"type": "string", "description": "An existing role id from the catalogue. Mutually exclusive with newRole."},
            "newRole": {
                "type": "object",
                "additionalProperties": false,
                "required": ["name", "description", "skillIds"],
                "properties": {
                    "name": {"type": "string", "maxLength": MAX_NAME_CHARS},
                    "description": {"type": "string", "maxLength": MAX_ROLE_DESCRIPTION_CHARS},
                    "skillIds": {"type": "array", "items": {"type": "string"}}
                }
            },
            "skillIds": {"type": "array", "items": {"type": "string"}, "description": "Optional skill ids from the catalogue (mandatory skills are attached automatically)."},
            "defaultLevel": {"type": "string", "enum": LEVELS},
            "rationale": {"type": "string", "maxLength": MAX_RATIONALE_CHARS}
        }
    });
    let max_agents = match mode {
        DraftMode::Agent => 1,
        DraftMode::Team => MAX_TEAM_SIZE,
    };
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["agents", "positions", "notes"],
        "properties": {
            "agents": {"type": "array", "minItems": 1, "maxItems": max_agents, "items": agent},
            "positions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["key", "level"],
                    "properties": {
                        "key": {"type": "string"},
                        "level": {"type": "string", "enum": LEVELS},
                        "supervisorKey": {"type": ["string", "null"]}
                    }
                }
            },
            "notes": {"type": "string", "maxLength": MAX_NOTES_CHARS, "description": "One short paragraph for the user: assumptions and anything the brief left open."}
        }
    })
}

// ── Validator ────────────────────────────────────────────────────────────────

/// Reject anything the model invented. Every `Err` names the offending field
/// (`draft.<field>: <reason>`) so the panel can show it verbatim; the caller
/// wraps it in `AppError::Invalid`.
pub fn validate_draft(
    draft: &DraftResponse,
    mode: DraftMode,
    cat: &Catalogue,
) -> Result<(), String> {
    if draft.agents.is_empty() {
        return Err("draft.agents: no agents drafted".into());
    }
    match mode {
        DraftMode::Agent => {
            if draft.agents.len() != 1 {
                return Err(format!(
                    "draft.agents: agent mode drafts exactly one agent, got {}",
                    draft.agents.len()
                ));
            }
            if !draft.positions.is_empty() {
                return Err("draft.positions: agent mode has no positions".into());
            }
        }
        DraftMode::Team => {
            if draft.agents.len() > MAX_TEAM_SIZE {
                return Err(format!(
                    "draft.agents: a team is at most {MAX_TEAM_SIZE} agents, got {}",
                    draft.agents.len()
                ));
            }
        }
    }

    // Keys are the draft's only identity — duplicates would make `positions`
    // ambiguous.
    for (i, a) in draft.agents.iter().enumerate() {
        if a.key.trim().is_empty() {
            return Err(format!("draft.agents[{i}].key: empty"));
        }
        if draft.agents[..i].iter().any(|b| b.key == a.key) {
            return Err(format!("draft.agents[{i}].key: duplicate key '{}'", a.key));
        }
    }

    for (i, a) in draft.agents.iter().enumerate() {
        validate_agent(i, a, &draft.agents[..i], cat)?;
    }

    if draft.notes.chars().count() > MAX_NOTES_CHARS {
        return Err(format!(
            "draft.notes: longer than {MAX_NOTES_CHARS} characters"
        ));
    }

    if mode == DraftMode::Team {
        validate_positions(draft, cat)?;
    }
    Ok(())
}

fn validate_agent(
    i: usize,
    a: &DraftAgent,
    earlier: &[DraftAgent],
    cat: &Catalogue,
) -> Result<(), String> {
    // Every leg carries a rationale, including reuse — so this is checked
    // before the reuse branch returns.
    check_len(&a.rationale, MAX_RATIONALE_CHARS)
        .map_err(|n| format!("draft.agents[{i}].rationale: longer than {n} characters"))?;

    if let Some(def_id) = a.existing_agent_def_id.as_deref() {
        if !cat.existing.iter().any(|d| d.id == def_id) {
            return Err(format!(
                "draft.agents[{i}].existingAgentDefId: no agent definition '{def_id}'"
            ));
        }
        // One definition, one roster row: two keys reusing the same definition
        // collapse onto a single `workspace_agent`, and the second
        // `instance.setPosition` of the apply orchestration silently overwrites
        // the first (plan amendment c5f6ec3, from Mellow's frontend review).
        if let Some(dup) = earlier
            .iter()
            .find(|b| b.existing_agent_def_id.as_deref() == Some(def_id))
        {
            return Err(format!(
                "draft.agents[{i}].existingAgentDefId: already used by {}",
                dup.key
            ));
        }
        // Reuse means reuse: any other field would silently be dropped by the
        // apply orchestration, which only calls `addToWorkspace` for this case.
        let carries_other = a.name.is_some()
            || a.color.is_some()
            || a.cli_kind.is_some()
            || a.model.is_some()
            || a.role_id.is_some()
            || a.new_role.is_some()
            || !a.skill_ids.is_empty()
            || a.default_level.is_some();
        if carries_other {
            return Err(format!(
                "draft.agents[{i}].existingAgentDefId: reuse takes only key and rationale, no other fields"
            ));
        }
        return Ok(());
    }

    let name = a.name.as_deref().unwrap_or("").trim();
    if name.is_empty() {
        return Err(format!("draft.agents[{i}].name: missing"));
    }
    check_len(name, MAX_NAME_CHARS)
        .map_err(|n| format!("draft.agents[{i}].name: longer than {n} characters"))?;
    let cli_kind = a.cli_kind.as_deref().unwrap_or("");
    let models = models_for(cli_kind).ok_or_else(|| {
        format!("draft.agents[{i}].cliKind: must be 'claude-code' or 'codex', got '{cli_kind}'")
    })?;
    let model = a
        .model
        .as_deref()
        .ok_or_else(|| format!("draft.agents[{i}].model: missing"))?;
    if !models.contains(&model) {
        return Err(format!(
            "draft.agents[{i}].model: '{model}' is not a {cli_kind} model in the catalogue"
        ));
    }

    match (a.role_id.as_deref(), a.new_role.as_ref()) {
        (Some(_), Some(_)) => {
            return Err(format!(
                "draft.agents[{i}].roleId: give roleId or newRole, not both"
            ))
        }
        (None, None) => {
            return Err(format!(
                "draft.agents[{i}].role: give a roleId or a newRole"
            ))
        }
        (Some(rid), None) => {
            if !cat.roles.iter().any(|r| r.id == rid) {
                return Err(format!("draft.agents[{i}].roleId: no role '{rid}'"));
            }
        }
        (None, Some(nr)) => {
            let fresh = nr.name.trim();
            if fresh.is_empty() {
                return Err(format!("draft.agents[{i}].newRole.name: empty"));
            }
            check_len(fresh, MAX_NAME_CHARS).map_err(|n| {
                format!("draft.agents[{i}].newRole.name: longer than {n} characters")
            })?;
            check_len(&nr.description, MAX_ROLE_DESCRIPTION_CHARS).map_err(|n| {
                format!("draft.agents[{i}].newRole.description: longer than {n} characters")
            })?;
            if cat
                .roles
                .iter()
                .any(|r| r.name.eq_ignore_ascii_case(fresh) || r.id.eq_ignore_ascii_case(fresh))
            {
                return Err(format!(
                    "draft.agents[{i}].newRole.name: '{fresh}' already exists — use roleId instead"
                ));
            }
            check_skill_ids(&nr.skill_ids, cat)
                .map_err(|id| format!("draft.agents[{i}].newRole.skillIds: no skill '{id}'"))?;
        }
    }

    check_skill_ids(&a.skill_ids, cat)
        .map_err(|id| format!("draft.agents[{i}].skillIds: no skill '{id}'"))?;

    if let Some(color) = a.color.as_deref() {
        if !COLOR_SWATCHES.contains(&color) {
            return Err(format!(
                "draft.agents[{i}].color: '{color}' is not a Builder swatch"
            ));
        }
    }
    if let Some(level) = a.default_level.as_deref() {
        if !LEVELS.contains(&level) {
            return Err(format!(
                "draft.agents[{i}].defaultLevel: unknown level '{level}'"
            ));
        }
    }
    Ok(())
}

/// `Err(max)` when `s` is longer than `max` CHARS — a byte length would reject
/// a legal name early on any non-ASCII text.
fn check_len(s: &str, max: usize) -> Result<(), usize> {
    if s.chars().count() > max {
        return Err(max);
    }
    Ok(())
}

/// `Err(id)` names the first id that is not in the catalogue.
fn check_skill_ids<'a>(ids: &'a [String], cat: &Catalogue) -> Result<(), &'a str> {
    for id in ids {
        if !cat.skills.iter().any(|s| &s.id == id) {
            return Err(id);
        }
    }
    Ok(())
}

fn validate_positions(draft: &DraftResponse, _cat: &Catalogue) -> Result<(), String> {
    // One position per agent, both directions — the apply orchestration walks
    // `positions`, so a missing one silently leaves an agent unplaced.
    if draft.positions.len() != draft.agents.len() {
        return Err("draft.positions: every agent needs one position".into());
    }
    for a in &draft.agents {
        if draft.positions.iter().filter(|p| p.key == a.key).count() != 1 {
            return Err(format!(
                "draft.positions: every agent needs one position (agent '{}')",
                a.key
            ));
        }
    }
    for (i, p) in draft.positions.iter().enumerate() {
        if !draft.agents.iter().any(|a| a.key == p.key) {
            return Err(format!(
                "draft.positions[{i}].key: no drafted agent '{}'",
                p.key
            ));
        }
        if !LEVELS.contains(&p.level.as_str()) {
            return Err(format!(
                "draft.positions[{i}].level: unknown level '{}'",
                p.level
            ));
        }
        if let Some(sup) = p.supervisor_key.as_deref() {
            if sup == p.key {
                return Err(format!(
                    "draft.positions[{i}].supervisorKey: '{sup}' supervises itself (cycle)"
                ));
            }
            if !draft.agents.iter().any(|a| a.key == sup) {
                return Err(format!(
                    "draft.positions[{i}].supervisorKey: no drafted agent '{sup}'"
                ));
            }
        }
    }

    // Reporting lines form a forest: walking supervisors from any key must
    // terminate at a key with no supervisor (`lib/positions.ts` semantics).
    for start in draft.positions.iter() {
        let mut seen = vec![start.key.as_str()];
        let mut cur = start.supervisor_key.as_deref();
        while let Some(k) = cur {
            if seen.contains(&k) {
                return Err(format!(
                    "draft.positions: reporting lines form a cycle at '{k}'"
                ));
            }
            seen.push(k);
            cur = draft
                .positions
                .iter()
                .find(|p| p.key == k)
                .and_then(|p| p.supervisor_key.as_deref());
        }
    }
    Ok(())
}

// ── Handler ──────────────────────────────────────────────────────────────────

/// Fixed one-shot budget (spec D9). No cancel in v1 — the panel shows elapsed
/// seconds and this timeout kills the child.
const DRAFT_TIMEOUT: Duration = Duration::from_secs(120);

pub async fn run(state: &AppState, payload: Value) -> Result<Value, AppError> {
    run_with_state(state, payload, &Oneshot::Live).await
}

async fn run_with_state(
    state: &AppState,
    payload: Value,
    oneshot: &Oneshot,
) -> Result<Value, AppError> {
    let req: DraftRequest =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;
    let lock = req
        .workspace_id
        .as_ref()
        .map(|id| state.workspace_lifecycle_lock(id));
    let _guard = match &lock {
        Some(lock) => Some(lock.read().await),
        None => None,
    };
    if let Some(id) = &req.workspace_id {
        super::workspace::require_active(state, id).await?;
    }
    let out = run_with(&state.db, oneshot, req).await?;
    serde_json::to_value(out).map_err(|e| AppError::Internal(e.to_string()))
}

/// The whole command with the runner injected, so tests drive it end to end
/// without spawning a binary (spec D7).
///
/// Request validation runs BEFORE the runner: a bad brief or a non-CLI drafter
/// must never cost the user a 120 s model call.
pub async fn run_with(
    db: &SqlitePool,
    oneshot: &Oneshot,
    req: DraftRequest,
) -> Result<DraftResponse, AppError> {
    let brief = req.brief.trim();
    if brief.is_empty() {
        return Err(AppError::Invalid("draft.brief: brief is empty".into()));
    }
    if brief.chars().count() > BRIEF_MAX_CHARS {
        return Err(AppError::Invalid(format!(
            "draft.brief: longer than {BRIEF_MAX_CHARS} characters"
        )));
    }

    let def = repo::agent_definition::get(db, &req.drafter_def_id)
        .await?
        .ok_or_else(|| AppError::Invalid("draft.drafterDefId: no such agent definition".into()))?;
    let cli_kind = def
        .cli_kind
        .as_deref()
        .filter(|_| def.r#type == "cli")
        .and_then(CliKind::parse)
        .ok_or_else(|| {
            AppError::Invalid(
                "draft.drafterDefId: the drafter must be a Claude Code or Codex CLI agent".into(),
            )
        })?;
    let model = def.model.clone().filter(|m| !m.is_empty());
    let model_for_launch = match cli_kind {
        CliKind::ClaudeCode => model
            .as_deref()
            .map(|m| effective_claude_model(m, def.context_window.as_deref())),
        CliKind::Codex => model.clone(),
    };

    let cat = build_catalogue(db, req.workspace_id.as_deref()).await?;
    let schema = draft_schema(req.mode);
    let prompt = build_prompt(req.mode, brief, &cat, &schema);

    // Run inside the workspace folder so the drafter's own rc/env resolve the
    // way a spawned agent's would; no workspace (agent mode from the Library)
    // falls back to the temp dir.
    let cwd = match req.workspace_id.as_deref() {
        Some(ws) => repo::workspace::get(db, ws)
            .await?
            .map(|w| PathBuf::from(w.folder_path))
            .unwrap_or_else(std::env::temp_dir),
        None => std::env::temp_dir(),
    };

    let spec = OneshotSpec {
        cli_kind,
        model: model_for_launch,
        prompt,
        json_schema: schema,
        extra_env: agent_env_overrides(&def),
        cwd,
        timeout: DRAFT_TIMEOUT,
    };
    // Timeout / non-zero exit / model errors surface verbatim so the panel can
    // show the CLI's own words (spec error table). A failed run is not
    // activity: nothing below records it.
    let outcome = oneshot
        .run_measured(&spec)
        .await
        .map_err(|e| AppError::Invalid(format!("draft.run: {e}")))?;
    // One successful invocation = one `invocation` activity, whatever the
    // schema validation below says about the answer: the model was called and
    // completed. Best-effort — a collection failure must never turn a good
    // draft into a user-facing error (plan, direct/one-shot collectors).
    if let Err(e) = record_draft_invocation(
        db,
        &outcome,
        req.workspace_id.as_deref(),
        cli_kind,
        model.as_deref(),
    )
    .await
    {
        eprintln!(
            "[usage] draft invocation {} not recorded: {e}",
            outcome.invocation_id
        );
    }
    let raw = outcome.value;

    /// The model answers the schema, which has no `drafter` field — that is
    /// filled in here from the definition the user picked.
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ModelOut {
        agents: Vec<DraftAgent>,
        #[serde(default)]
        positions: Vec<DraftPosition>,
        #[serde(default)]
        notes: String,
    }
    let parsed: ModelOut =
        serde_json::from_value(raw).map_err(|e| AppError::Invalid(format!("draft.parse: {e}")))?;

    let resp = DraftResponse {
        agents: parsed.agents,
        positions: parsed.positions,
        notes: parsed.notes,
        drafter: DrafterInfo {
            def_id: def.id.clone(),
            cli_kind: def.cli_kind.clone().unwrap_or_default(),
            model: model.unwrap_or_default(),
        },
    };
    validate_draft(&resp, req.mode, &cat).map_err(AppError::Invalid)?;
    Ok(resp)
}

/// Persist one successful draft invocation as measured usage
/// ([`record_collected_event`]).
///
/// * `event_key = draft:v1:<invocation id>` — the id was generated before the
///   child spawned, so a replay of the same invocation is a no-op and two
///   invocations can never share an identity.
/// * `workspace_id` is the request's, which is legitimately `None` for a
///   Library draft: it is stored unscoped and surfaced as `__unscoped__`, never
///   attributed to the current workspace (preflight correction D1).
/// * `requested_model` is the definition's CONFIGURED model — the user's
///   selection — not the launch-time effective id, so the event unifies with
///   the agent's context gauge under one Selected key. `served_model` is set
///   only when the CLI itself named exactly one serving model.
async fn record_draft_invocation(
    db: &SqlitePool,
    outcome: &OneshotOutcome,
    workspace_id: Option<&str>,
    cli_kind: CliKind,
    configured_model: Option<&str>,
) -> Result<(), sqlx::Error> {
    let cli_kind_name = match cli_kind {
        CliKind::ClaudeCode => "claude-code",
        CliKind::Codex => "codex",
    };
    record_collected_event(
        db,
        &CollectedEvent {
            source_kind: SOURCE_DRAFT,
            operation_id: &outcome.invocation_id,
            event_kind: "invocation",
            workspace_id,
            workspace_agent_id: None,
            session_id: None,
            generation: None,
            source_session_id: outcome.source_session_id.as_deref(),
            source_request_id: None,
            source_response_id: None,
            occurred_at: outcome.completed_at,
            provider: provider_for_cli_kind(cli_kind_name),
            requested_model: configured_model,
            served_model: outcome.served_model.as_deref(),
            usage: &outcome.usage,
        },
    )
    .await
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Shared by `draft_prompt`'s tests — one catalogue both modules assert
    /// against, so a catalogue field the prompt drops is caught here too.
    pub(crate) fn cat() -> Catalogue {
        Catalogue {
            roles: vec![
                RoleRow {
                    id: "lead".into(),
                    name: "Lead".into(),
                    description: "d".into(),
                    skill_ids: vec!["leadership".into()],
                    kind: "builtin".into(),
                },
                RoleRow {
                    id: "implementer".into(),
                    name: "Implementer".into(),
                    description: "d".into(),
                    skill_ids: vec!["implementer".into()],
                    kind: "builtin".into(),
                },
            ],
            skills: vec![
                skill("leadership"),
                skill("implementer"),
                skill("agent-loop"),
            ],
            existing: vec![ExistingDef {
                id: "def-existing".into(),
                name: "Dew".into(),
                role_name: Some("Implementer".into()),
                cli_kind: Some("claude-code".into()),
                model: Some("claude-opus-4-8".into()),
            }],
            roster: vec![],
        }
    }

    fn skill(id: &str) -> SkillRow {
        SkillRow {
            id: id.into(),
            name: id.into(),
            description: Some(format!("{id} description")),
            content: String::new(),
            kind: "builtin".into(),
            mandatory: false,
            icon: None,
        }
    }

    fn agent(key: &str) -> DraftAgent {
        DraftAgent {
            key: key.into(),
            existing_agent_def_id: None,
            name: Some(format!("A-{key}")),
            color: Some(COLOR_SWATCHES[0].into()),
            cli_kind: Some("claude-code".into()),
            model: Some("claude-sonnet-5".into()),
            role_id: Some("implementer".into()),
            new_role: None,
            skill_ids: vec!["agent-loop".into()],
            default_level: Some("senior".into()),
            rationale: "r".into(),
        }
    }

    fn resp(agents: Vec<DraftAgent>, positions: Vec<DraftPosition>) -> DraftResponse {
        DraftResponse {
            agents,
            positions,
            notes: String::new(),
            drafter: DrafterInfo {
                def_id: "d".into(),
                cli_kind: "claude-code".into(),
                model: "m".into(),
            },
        }
    }

    fn pos(key: &str, sup: Option<&str>) -> DraftPosition {
        DraftPosition {
            key: key.into(),
            level: "senior".into(),
            supervisor_key: sup.map(String::from),
        }
    }

    #[test]
    fn agent_mode_requires_exactly_one_agent_and_no_positions() {
        assert!(validate_draft(&resp(vec![agent("a")], vec![]), DraftMode::Agent, &cat()).is_ok());
        assert!(validate_draft(
            &resp(vec![agent("a"), agent("b")], vec![]),
            DraftMode::Agent,
            &cat()
        )
        .unwrap_err()
        .starts_with("draft.agents"));
        assert!(validate_draft(
            &resp(vec![agent("a")], vec![pos("a", None)]),
            DraftMode::Agent,
            &cat()
        )
        .unwrap_err()
        .starts_with("draft.positions"));
    }

    #[test]
    fn team_mode_rejects_duplicate_keys_unknown_supervisor_and_cycles() {
        let c = cat();
        assert!(validate_draft(
            &resp(vec![agent("a"), agent("a")], vec![pos("a", None)]),
            DraftMode::Team,
            &c
        )
        .unwrap_err()
        .contains("key"));
        assert!(validate_draft(
            &resp(vec![agent("a")], vec![pos("a", Some("zz"))]),
            DraftMode::Team,
            &c
        )
        .unwrap_err()
        .contains("supervisorKey"));
        let cyc = resp(
            vec![agent("a"), agent("b")],
            vec![pos("a", Some("b")), pos("b", Some("a"))],
        );
        assert!(validate_draft(&cyc, DraftMode::Team, &c)
            .unwrap_err()
            .contains("cycle"));
    }

    #[test]
    fn team_mode_requires_a_position_per_agent_and_size_cap() {
        let c = cat();
        assert!(
            validate_draft(&resp(vec![agent("a")], vec![]), DraftMode::Team, &c)
                .unwrap_err()
                .contains("positions")
        );
        let many: Vec<_> = (0..13).map(|i| agent(&format!("k{i}"))).collect();
        let ps: Vec<_> = (0..13).map(|i| pos(&format!("k{i}"), None)).collect();
        assert!(validate_draft(&resp(many, ps), DraftMode::Team, &c)
            .unwrap_err()
            .contains("12"));
    }

    #[test]
    fn rejects_unknown_role_skill_model_color_level() {
        let c = cat();
        let mut a = agent("a");
        a.role_id = Some("nope".into());
        assert!(validate_draft(&resp(vec![a], vec![]), DraftMode::Agent, &c)
            .unwrap_err()
            .contains("roleId"));
        let mut a = agent("a");
        a.skill_ids = vec!["nope".into()];
        assert!(validate_draft(&resp(vec![a], vec![]), DraftMode::Agent, &c)
            .unwrap_err()
            .contains("skillIds"));
        let mut a = agent("a");
        a.model = Some("gpt-5.5".into()); // codex model on claude-code
        assert!(validate_draft(&resp(vec![a], vec![]), DraftMode::Agent, &c)
            .unwrap_err()
            .contains("model"));
        let mut a = agent("a");
        a.color = Some("#123456".into());
        assert!(validate_draft(&resp(vec![a], vec![]), DraftMode::Agent, &c)
            .unwrap_err()
            .contains("color"));
        let mut a = agent("a");
        a.default_level = Some("boss".into());
        assert!(validate_draft(&resp(vec![a], vec![]), DraftMode::Agent, &c)
            .unwrap_err()
            .contains("defaultLevel"));
    }

    #[test]
    fn codex_catalogue_matches_typescript_order_and_accepts_astra() {
        let typescript = include_str!("../../../../src/lib/modelCatalogue.ts");
        let block = typescript
            .split("export const CODEX_MODELS = [")
            .nth(1)
            .and_then(|tail| tail.split("];").next())
            .expect("TypeScript CODEX_MODELS block");
        let typescript_models: Vec<&str> = block
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with('"'))
            .map(|line| line.trim_end_matches(',').trim_matches('"'))
            .collect();
        assert_eq!(typescript_models, CODEX_MODELS);
        assert_eq!(CODEX_MODELS.first().copied(), Some("gpt-6-astra"));

        let mut astra = agent("astra");
        astra.cli_kind = Some("codex".into());
        astra.model = Some("gpt-6-astra".into());
        validate_draft(&resp(vec![astra], vec![]), DraftMode::Agent, &cat())
            .expect("the highest-priority Codex preset must pass draft validation");
    }

    #[test]
    fn role_id_xor_new_role_and_new_role_name_must_be_fresh() {
        let c = cat();
        let mut a = agent("a");
        a.new_role = Some(DraftNewRole {
            name: "QA".into(),
            description: "d".into(),
            skill_ids: vec!["agent-loop".into()],
        });
        assert!(
            validate_draft(&resp(vec![a.clone()], vec![]), DraftMode::Agent, &c)
                .unwrap_err()
                .contains("roleId")
        );
        a.role_id = None;
        assert!(validate_draft(&resp(vec![a.clone()], vec![]), DraftMode::Agent, &c).is_ok());
        a.new_role.as_mut().unwrap().name = "lead".into();
        assert!(validate_draft(&resp(vec![a], vec![]), DraftMode::Agent, &c)
            .unwrap_err()
            .contains("newRole.name"));
    }

    #[test]
    fn existing_def_reuse_must_exist_and_carry_no_other_fields() {
        let c = cat();
        let a = DraftAgent {
            key: "x".into(),
            existing_agent_def_id: Some("def-existing".into()),
            name: None,
            color: None,
            cli_kind: None,
            model: None,
            role_id: None,
            new_role: None,
            skill_ids: vec![],
            default_level: None,
            rationale: "r".into(),
        };
        assert!(validate_draft(&resp(vec![a.clone()], vec![]), DraftMode::Agent, &c).is_ok());
        let mut bad = a.clone();
        bad.existing_agent_def_id = Some("ghost".into());
        assert!(
            validate_draft(&resp(vec![bad], vec![]), DraftMode::Agent, &c)
                .unwrap_err()
                .contains("existingAgentDefId")
        );
        let mut bad = a;
        bad.name = Some("N".into());
        assert!(
            validate_draft(&resp(vec![bad], vec![]), DraftMode::Agent, &c)
                .unwrap_err()
                .contains("existingAgentDefId")
        );
    }

    #[test]
    fn existing_def_reuse_must_be_unique_across_the_draft() {
        let c = cat();
        let reuse = |key: &str| DraftAgent {
            key: key.into(),
            existing_agent_def_id: Some("def-existing".into()),
            name: None,
            color: None,
            cli_kind: None,
            model: None,
            role_id: None,
            new_role: None,
            skill_ids: vec![],
            default_level: None,
            rationale: "r".into(),
        };
        let twice = resp(
            vec![reuse("a"), reuse("b")],
            vec![pos("a", None), pos("b", Some("a"))],
        );
        let err = validate_draft(&twice, DraftMode::Team, &c).unwrap_err();
        assert!(err.contains("existingAgentDefId"), "got {err}");
        assert!(err.contains("already used by a"), "got {err}");

        // One reuse plus a fresh agent is still fine.
        let once = resp(
            vec![reuse("a"), agent("b")],
            vec![pos("a", None), pos("b", Some("a"))],
        );
        assert!(validate_draft(&once, DraftMode::Team, &c).is_ok());
    }

    #[test]
    fn rejects_free_text_longer_than_the_schema_declares() {
        let c = cat();
        // The caps the schema declares are re-checked here, because codex gets
        // no --output-schema at all and a model can overrun either way.
        let mut a = agent("a");
        a.name = Some("N".repeat(MAX_NAME_CHARS + 1));
        assert!(validate_draft(&resp(vec![a], vec![]), DraftMode::Agent, &c)
            .unwrap_err()
            .contains("name: longer than 40"));

        let mut a = agent("a");
        a.rationale = "r".repeat(MAX_RATIONALE_CHARS + 1);
        assert!(validate_draft(&resp(vec![a], vec![]), DraftMode::Agent, &c)
            .unwrap_err()
            .contains("rationale: longer than 200"));

        // rationale is checked on the REUSE leg too, which returns early.
        let reuse = DraftAgent {
            key: "x".into(),
            existing_agent_def_id: Some("def-existing".into()),
            name: None,
            color: None,
            cli_kind: None,
            model: None,
            role_id: None,
            new_role: None,
            skill_ids: vec![],
            default_level: None,
            rationale: "r".repeat(MAX_RATIONALE_CHARS + 1),
        };
        assert!(
            validate_draft(&resp(vec![reuse], vec![]), DraftMode::Agent, &c)
                .unwrap_err()
                .contains("rationale: longer than 200")
        );

        let mut a = agent("a");
        a.role_id = None;
        a.new_role = Some(DraftNewRole {
            name: "Q".repeat(MAX_NAME_CHARS + 1),
            description: "d".into(),
            skill_ids: vec![],
        });
        assert!(validate_draft(&resp(vec![a], vec![]), DraftMode::Agent, &c)
            .unwrap_err()
            .contains("newRole.name: longer than 40"));

        let mut a = agent("a");
        a.role_id = None;
        a.new_role = Some(DraftNewRole {
            name: "QA".into(),
            description: "d".repeat(MAX_ROLE_DESCRIPTION_CHARS + 1),
            skill_ids: vec![],
        });
        assert!(validate_draft(&resp(vec![a], vec![]), DraftMode::Agent, &c)
            .unwrap_err()
            .contains("newRole.description: longer than 600"));

        let mut r = resp(vec![agent("a")], vec![]);
        r.notes = "n".repeat(MAX_NOTES_CHARS + 1);
        assert!(validate_draft(&r, DraftMode::Agent, &c)
            .unwrap_err()
            .contains("draft.notes: longer than 600"));

        // Exactly at the cap is legal — the check is `>`, not `>=`.
        let mut a = agent("a");
        a.name = Some("N".repeat(MAX_NAME_CHARS));
        a.rationale = "r".repeat(MAX_RATIONALE_CHARS);
        assert!(validate_draft(&resp(vec![a], vec![]), DraftMode::Agent, &c).is_ok());

        // Multi-byte text is measured in CHARS, not bytes: 40 e-acutes is 80
        // bytes and must still pass.
        let mut a = agent("a");
        a.name = Some("é".repeat(MAX_NAME_CHARS));
        assert!(validate_draft(&resp(vec![a], vec![]), DraftMode::Agent, &c).is_ok());
    }

    #[test]
    fn schema_max_lengths_come_from_the_same_constants_the_validator_uses() {
        let s = draft_schema(DraftMode::Team);
        let items = &s["properties"]["agents"]["items"]["properties"];
        assert_eq!(items["name"]["maxLength"], MAX_NAME_CHARS);
        assert_eq!(items["rationale"]["maxLength"], MAX_RATIONALE_CHARS);
        assert_eq!(
            items["newRole"]["properties"]["name"]["maxLength"],
            MAX_NAME_CHARS
        );
        assert_eq!(
            items["newRole"]["properties"]["description"]["maxLength"],
            MAX_ROLE_DESCRIPTION_CHARS
        );
        assert_eq!(s["properties"]["notes"]["maxLength"], MAX_NOTES_CHARS);
    }

    #[test]
    fn new_agent_needs_name_cli_kind_model_and_a_role() {
        let c = cat();
        let mut a = agent("a");
        a.name = None;
        assert!(validate_draft(&resp(vec![a], vec![]), DraftMode::Agent, &c)
            .unwrap_err()
            .contains("name"));
        let mut a = agent("a");
        a.role_id = None;
        assert!(validate_draft(&resp(vec![a], vec![]), DraftMode::Agent, &c)
            .unwrap_err()
            .contains("role"));
    }

    #[test]
    fn schema_lists_every_field_and_requires_key_and_rationale() {
        let s = draft_schema(DraftMode::Team);
        let props = &s["properties"]["agents"]["items"]["properties"];
        for f in [
            "key",
            "existingAgentDefId",
            "name",
            "color",
            "cliKind",
            "model",
            "roleId",
            "newRole",
            "skillIds",
            "defaultLevel",
            "rationale",
        ] {
            assert!(props.get(f).is_some(), "missing {f}");
        }
        assert_eq!(
            s["properties"]["agents"]["items"]["required"],
            serde_json::json!(["key", "rationale"])
        );
        assert!(s["properties"].get("positions").is_some());
        assert_eq!(
            s["required"],
            serde_json::json!(["agents", "positions", "notes"])
        );
        assert_eq!(s["properties"]["agents"]["maxItems"], 12);
        assert_eq!(
            draft_schema(DraftMode::Agent)["properties"]["agents"]["maxItems"],
            1
        );
    }
    // ── run_with: the command end to end, runner mocked ──────────────────

    use crate::engine::db::connect_in_memory;
    use crate::engine::repo::agent_definition::AgentDefinitionInput;

    /// A minimal CLI drafter definition. `agent_type` is what makes it eligible
    /// (`"cli"` + a known `cli_kind`); tests flip either to prove the rejection.
    fn drafter_input(agent_type: &str, cli_kind: Option<&str>) -> AgentDefinitionInput {
        AgentDefinitionInput {
            name: "Drafter".to_owned(),
            role: None,
            role_id: None,
            agent_type: agent_type.to_owned(),
            cli_kind: cli_kind.map(str::to_owned),
            color: None,
            default_level: None,
            provider_id: None,
            model: Some("claude-sonnet-5".to_owned()),
            effort: None,
            harness_mode: "own".to_owned(),
            share_blackboard: None,
            auto_submit_injected: None,
            allowed_senders: None,
            permission_mode: None,
            custom_args: None,
            custom_env: None,
            secret_env_keys: None,
            context_window: None,
            selected_builtin_skill_ids: None,
            rtk_enabled: None,
        }
    }

    fn canned_team() -> Value {
        json!({
            "agents": [{
                "key": "lead",
                "name": "Nova",
                "color": COLOR_SWATCHES[0],
                "cliKind": "claude-code",
                "model": "claude-sonnet-5",
                "roleId": "lead",
                "skillIds": [],
                "defaultLevel": "principal",
                "rationale": "r"
            }],
            "positions": [{"key": "lead", "level": "principal", "supervisorKey": null}],
            "notes": "n"
        })
    }

    fn req(def_id: &str, brief: &str) -> DraftRequest {
        DraftRequest {
            mode: DraftMode::Team,
            brief: brief.to_owned(),
            drafter_def_id: def_id.to_owned(),
            workspace_id: None,
        }
    }

    #[tokio::test]
    async fn run_with_mock_returns_validated_response() {
        let db = connect_in_memory().await;
        let def = repo::agent_definition::create(&db, &drafter_input("cli", Some("claude-code")))
            .await
            .unwrap();
        let out = run_with(&db, &Oneshot::Mock(Ok(canned_team())), req(&def.id, "b"))
            .await
            .unwrap();
        assert_eq!(out.agents[0].name.as_deref(), Some("Nova"));
        assert_eq!(out.drafter.def_id, def.id);
        assert_eq!(out.drafter.cli_kind, "claude-code");
        assert_eq!(out.drafter.model, "claude-sonnet-5");
        assert_eq!(out.notes, "n");
    }

    #[tokio::test]
    async fn archive_serializes_with_production_draft_guard_and_rechecks_after_lock() {
        let state = AppState::for_tests().await;
        let ws = repo::workspace::create(&state.db, "Draft", "/tmp/draft", None)
            .await
            .unwrap();
        let def =
            repo::agent_definition::create(&state.db, &drafter_input("cli", Some("claude-code")))
                .await
                .unwrap();
        let payload = json!({"workspaceId":ws.id,"drafterDefId":def.id,"mode":"team","brief":"Create a team"});
        let mock = Oneshot::Mock(Ok(canned_team()));
        // Exhaust the single test connection: the real handler acquires its
        // lifecycle read guard, then yields during its fresh archive check.
        let connection = state.db.acquire().await.unwrap();
        let mut draft = Box::pin(run_with_state(&state, payload.clone(), &mock));
        assert!(futures_util::poll!(&mut draft).is_pending());
        let error = super::super::workspace::archive(&state, json!({"workspaceId":ws.id}))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("busy"));
        drop(connection);
        assert!(
            draft.await.is_ok(),
            "stopped active workspaces can still draft"
        );
        let lock = state.workspace_lifecycle_lock(&ws.id);
        let guard = lock.write().await;
        let mut draft = Box::pin(run_with_state(&state, payload, &mock));
        assert!(futures_util::poll!(&mut draft).is_pending());
        repo::workspace::set_archived(&state.db, &ws.id, Some("2026-09-05T00:00:00Z"))
            .await
            .unwrap();
        drop(guard);
        assert!(draft.await.unwrap_err().to_string().contains("archived"));
    }

    #[tokio::test]
    async fn run_with_rejects_invalid_model_output_as_invalid() {
        let db = connect_in_memory().await;
        let def = repo::agent_definition::create(&db, &drafter_input("cli", Some("claude-code")))
            .await
            .unwrap();
        let mut canned = canned_team();
        canned["agents"][0]["roleId"] = json!("ghost");
        let err = run_with(&db, &Oneshot::Mock(Ok(canned)), req(&def.id, "b"))
            .await
            .unwrap_err();
        match err {
            AppError::Invalid(m) => assert!(m.contains("roleId"), "got {m}"),
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_with_rejects_empty_brief_and_non_cli_drafter() {
        let db = connect_in_memory().await;
        let cli_def =
            repo::agent_definition::create(&db, &drafter_input("cli", Some("claude-code")))
                .await
                .unwrap();
        let chat_def = repo::agent_definition::create(&db, &drafter_input("chat", None))
            .await
            .unwrap();

        // `Mock(Err(...))` would surface as `draft.run: …` — asserting on the
        // Invalid instead proves the runner was never reached.
        let must_not_run = Oneshot::Mock(Err("must not run".into()));

        let err = run_with(&db, &must_not_run, req(&cli_def.id, "   "))
            .await
            .unwrap_err();
        match err {
            AppError::Invalid(m) => assert!(m.starts_with("draft.brief"), "got {m}"),
            other => panic!("expected Invalid, got {other:?}"),
        }

        let err = run_with(&db, &must_not_run, req(&chat_def.id, "b"))
            .await
            .unwrap_err();
        match err {
            AppError::Invalid(m) => assert!(m.starts_with("draft.drafterDefId"), "got {m}"),
            other => panic!("expected Invalid, got {other:?}"),
        }

        let err = run_with(&db, &must_not_run, req("ghost-def", "b"))
            .await
            .unwrap_err();
        match err {
            AppError::Invalid(m) => assert!(m.starts_with("draft.drafterDefId"), "got {m}"),
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn build_catalogue_lists_builtin_roles_and_omits_mandatory_skills() {
        let db = connect_in_memory().await;
        let cat = build_catalogue(&db, None).await.unwrap();
        assert!(cat.roles.iter().any(|r| r.id == "lead"), "builtin roles");
        assert!(
            cat.skills.iter().all(|s| !s.mandatory),
            "mandatory skills must not be offered"
        );
        assert!(cat.roster.is_empty(), "no workspace -> no roster");
    }
    /// The REAL answer Claude Code 2.1.260 gave the shipped prompt in the Task
    /// A5 probe (`claude -p --output-format json --json-schema … --model
    /// claude-sonnet-5`, envelope at /tmp/draft-probe/claude.out, recorded as a
    /// task gate). Prose fields are shortened; every id-bearing field is
    /// verbatim. This is the regression guard that the prompt and the validator
    /// agree — if a future prompt edit makes a real model answer fail
    /// validation, the fix belongs in the prompt, not the validator.
    #[test]
    fn recorded_claude_envelope_parses_and_validates() {
        let envelope = json!({
            "type": "result",
            "subtype": "success",
            "is_error": false,
            "structured_output": {
                "agents": [
                    {"key": "lead", "name": "Elena", "cliKind": "claude-code",
                     "model": "claude-opus-5", "color": "#5e5ce6", "roleId": "lead",
                     "defaultLevel": "principal", "rationale": "r"},
                    {"key": "dew", "existingAgentDefId": "def-existing", "rationale": "r"},
                    {"key": "reviewer", "name": "Priya", "cliKind": "claude-code",
                     "model": "claude-sonnet-5", "color": "#0a84ff", "defaultLevel": "senior",
                     "newRole": {"name": "Reviewer", "description": "d", "skillIds": []},
                     "rationale": "r"}
                ],
                "positions": [
                    {"key": "lead", "level": "principal"},
                    {"key": "dew", "level": "senior", "supervisorKey": "lead"},
                    {"key": "reviewer", "level": "senior", "supervisorKey": "lead"}
                ],
                "notes": "n"
            },
            "result": "…"
        });

        let structured =
            crate::engine::runtime::cli_oneshot::claude_structured_result(&envelope).unwrap();
        let mut draft: DraftResponse = serde_json::from_value(json!({
            "agents": structured["agents"],
            "positions": structured["positions"],
            "notes": structured["notes"],
            "drafter": {"defId": "d", "cliKind": "claude-code", "model": "claude-sonnet-5"}
        }))
        .expect("the recorded structured_output deserializes into the wire types");

        validate_draft(&draft, DraftMode::Team, &cat()).expect("the real draft validates");

        // The shape the house rules ask for: exactly one top-level lead.
        assert_eq!(
            draft
                .positions
                .iter()
                .filter(|p| p.supervisor_key.is_none())
                .count(),
            1
        );
        // …and the validator is not vacuous on it.
        draft.agents[0].model = Some("gpt-5.5".into());
        assert!(validate_draft(&draft, DraftMode::Team, &cat())
            .unwrap_err()
            .contains("model"));
    }
    /// The REAL answer codex-cli 0.153.2 (gpt-5.5) gave the SAME shipped prompt
    /// in the Task A5 probe, read back from its `-o last.json` sink
    /// (/tmp/draft-probe/last-noschema.json). Prose fields shortened; every
    /// id-bearing field verbatim. This is the evidence behind the R2 ruling:
    /// with no `--output-schema`, the schema embedded in the prompt is enough —
    /// codex answered catalogue ids only, and the validator accepts it.
    #[test]
    fn recorded_codex_last_message_parses_and_validates() {
        let last_json = r##"{
            "agents": [
                {"key":"lead","cliKind":"codex","color":"#5e5ce6","defaultLevel":"principal",
                 "model":"gpt-5.6-sol","name":"Mara","roleId":"lead","rationale":"r"},
                {"key":"porter","existingAgentDefId":"def-existing","rationale":"r"},
                {"key":"reviewer","cliKind":"codex","color":"#0a84ff","defaultLevel":"senior",
                 "model":"gpt-5.6-terra","name":"Iris",
                 "newRole":{"name":"Reviewer","description":"d","skillIds":["implementer"]},
                 "rationale":"r"}
            ],
            "positions": [
                {"key":"lead","level":"principal","supervisorKey":null},
                {"key":"porter","level":"senior","supervisorKey":"lead"},
                {"key":"reviewer","level":"senior","supervisorKey":"lead"}
            ],
            "notes": "n"
        }"##;

        let parsed =
            crate::engine::runtime::cli_oneshot::parse_codex_last_message(last_json).unwrap();
        let draft: DraftResponse = serde_json::from_value(json!({
            "agents": parsed["agents"],
            "positions": parsed["positions"],
            "notes": parsed["notes"],
            "drafter": {"defId": "d", "cliKind": "codex", "model": "gpt-5.5"}
        }))
        .expect("the recorded codex message deserializes into the wire types");

        validate_draft(&draft, DraftMode::Team, &cat()).expect("the real codex draft validates");
        // An explicit `"supervisorKey": null` must read as "no supervisor",
        // not as a supervisor named null — codex emits the key, claude omits it.
        assert!(draft.positions[0].supervisor_key.is_none());
    }

    // ── Usage collection ─────────────────────────────────────────────────

    use crate::engine::runtime::cli_oneshot::OneshotUsage;

    fn measured_outcome(value: Value) -> OneshotOutcome {
        OneshotOutcome {
            value,
            invocation_id: "inv-1".into(),
            requested_model: Some("claude-sonnet-5[1m]".into()),
            served_model: Some("claude-sonnet-5".into()),
            source_session_id: Some("sess-1".into()),
            usage: OneshotUsage {
                input_tokens: Some(31_063),
                output_tokens: Some(3_297),
                cache_read_input_tokens: Some(0),
                cache_write_input_tokens: Some(31_061),
                reasoning_output_tokens: Some(2_489),
            },
            completed_at: chrono::Utc::now(),
        }
    }

    #[derive(sqlx::FromRow)]
    struct StoredEvent {
        event_key: String,
        workspace_id: Option<String>,
        workspace_agent_id: Option<String>,
        source_kind: String,
        event_kind: String,
        source_session_id: Option<String>,
        provider: Option<String>,
        requested_model: Option<String>,
        served_model: Option<String>,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        reasoning_output_tokens: Option<i64>,
        token_completeness: String,
    }

    async fn stored_events(db: &SqlitePool) -> Vec<StoredEvent> {
        sqlx::query_as(
            "SELECT event_key, workspace_id, workspace_agent_id, source_kind, event_kind,
                    source_session_id, provider, requested_model, served_model,
                    input_tokens, output_tokens, reasoning_output_tokens, token_completeness
               FROM model_usage_event ORDER BY event_key",
        )
        .fetch_all(db)
        .await
        .unwrap()
    }

    /// A successful Library draft is ONE unscoped invocation carrying the
    /// CLI's own usage, keyed by the pre-generated invocation id.
    #[tokio::test]
    async fn a_successful_draft_records_one_unscoped_invocation() {
        let db = connect_in_memory().await;
        let def = repo::agent_definition::create(&db, &drafter_input("cli", Some("claude-code")))
            .await
            .unwrap();
        let mock = Oneshot::MockMeasured(Ok(measured_outcome(canned_team())));
        run_with(&db, &mock, req(&def.id, "b")).await.unwrap();

        let events = stored_events(&db).await;
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.event_key, "draft:v1:inv-1");
        assert_eq!(e.workspace_id, None, "a Library draft has no workspace");
        assert_eq!(e.workspace_agent_id, None);
        assert_eq!(e.source_kind, SOURCE_DRAFT);
        assert_eq!(e.event_kind, "invocation");
        assert_eq!(e.source_session_id.as_deref(), Some("sess-1"));
        assert_eq!(e.provider.as_deref(), Some("anthropic"));
        assert_eq!(
            e.requested_model.as_deref(),
            Some("claude-sonnet-5"),
            "the configured selection, not the launch-time effective id"
        );
        assert_eq!(e.served_model.as_deref(), Some("claude-sonnet-5"));
        assert_eq!(e.input_tokens, Some(31_063));
        assert_eq!(e.output_tokens, Some(3_297));
        assert_eq!(e.reasoning_output_tokens, Some(2_489));
        assert_eq!(e.token_completeness, "known");
    }

    /// A workspace draft is scoped to that workspace, still with no agent.
    #[tokio::test]
    async fn a_workspace_draft_is_scoped_to_its_workspace_without_an_agent() {
        let state = AppState::for_tests().await;
        let ws = repo::workspace::create(&state.db, "Draft", "/tmp/draft", None)
            .await
            .unwrap();
        repo::workspace::set_run_state(&state.db, &ws.id, "started")
            .await
            .unwrap();
        let def = repo::agent_definition::create(&state.db, &drafter_input("cli", Some("codex")))
            .await
            .unwrap();
        let mut outcome = measured_outcome(canned_team());
        outcome.served_model = None; // codex never proves the serving model
        outcome.usage = OneshotUsage::default(); // …and may report no usage
        run_with_state(
            &state,
            json!({
                "mode": "team",
                "brief": "b",
                "drafterDefId": def.id,
                "workspaceId": ws.id,
            }),
            &Oneshot::MockMeasured(Ok(outcome)),
        )
        .await
        .unwrap();

        let events = stored_events(&state.db).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].workspace_id.as_deref(), Some(ws.id.as_str()));
        assert_eq!(events[0].workspace_agent_id, None);
        assert_eq!(events[0].provider.as_deref(), Some("openai"));
        assert_eq!(events[0].served_model, None);
        assert_eq!(
            events[0].input_tokens, None,
            "unavailable usage stays unknown"
        );
        assert_eq!(events[0].token_completeness, "unknown");
    }

    /// A failed run is not activity — no row, not even an unknown one.
    #[tokio::test]
    async fn a_failed_draft_records_nothing() {
        let db = connect_in_memory().await;
        let def = repo::agent_definition::create(&db, &drafter_input("cli", Some("claude-code")))
            .await
            .unwrap();
        let err = run_with(
            &db,
            &Oneshot::MockMeasured(Err("The drafter did not answer in 120 s".into())),
            req(&def.id, "b"),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
        assert!(stored_events(&db).await.is_empty());
    }

    /// The invocation id is the identity: replaying the same outcome is a
    /// no-op, two different invocations are two activities.
    #[tokio::test]
    async fn invocation_identity_dedups_replay_but_not_a_second_run() {
        let db = connect_in_memory().await;
        let def = repo::agent_definition::create(&db, &drafter_input("cli", Some("claude-code")))
            .await
            .unwrap();
        let same = Oneshot::MockMeasured(Ok(measured_outcome(canned_team())));
        run_with(&db, &same, req(&def.id, "b")).await.unwrap();
        run_with(&db, &same, req(&def.id, "b")).await.unwrap();
        assert_eq!(
            stored_events(&db).await.len(),
            1,
            "same invocation id, one row"
        );

        let mut second = measured_outcome(canned_team());
        second.invocation_id = "inv-2".into();
        run_with(&db, &Oneshot::MockMeasured(Ok(second)), req(&def.id, "b"))
            .await
            .unwrap();
        assert_eq!(stored_events(&db).await.len(), 2);
    }

    /// An invalid answer still records the invocation: the model was called
    /// and completed even though the draft is rejected.
    #[tokio::test]
    async fn an_invalid_answer_still_records_the_completed_invocation() {
        let db = connect_in_memory().await;
        let def = repo::agent_definition::create(&db, &drafter_input("cli", Some("claude-code")))
            .await
            .unwrap();
        let mut canned = canned_team();
        canned["agents"][0]["model"] = json!("not-a-real-model");
        let err = run_with(
            &db,
            &Oneshot::MockMeasured(Ok(measured_outcome(canned))),
            req(&def.id, "b"),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::Invalid(_)));
        assert_eq!(stored_events(&db).await.len(), 1);
    }

    /// With the collectors online, a draft also extends the `draft` source's
    /// complete, unrestricted coverage from the online instant to completion.
    #[tokio::test]
    async fn a_draft_extends_the_draft_source_coverage_when_collectors_are_online() {
        let db = connect_in_memory().await;
        let def = repo::agent_definition::create(&db, &drafter_input("cli", Some("claude-code")))
            .await
            .unwrap();
        crate::engine::runtime::usage::mark_collectors_online();
        let online = crate::engine::runtime::usage::collectors_online_since().expect("marked");
        let mut outcome = measured_outcome(canned_team());
        outcome.completed_at = online + chrono::Duration::seconds(30);
        run_with(&db, &Oneshot::MockMeasured(Ok(outcome)), req(&def.id, "b"))
            .await
            .unwrap();

        #[derive(sqlx::FromRow)]
        struct Interval {
            workspace_id: Option<String>,
            workspace_agent_id: Option<String>,
            source_kind: String,
            state: String,
            interval_start: String,
            interval_end: String,
        }
        let rows: Vec<Interval> = sqlx::query_as(
            "SELECT workspace_id, workspace_agent_id, source_kind, state,
                    interval_start, interval_end
               FROM model_usage_coverage",
        )
        .fetch_all(&db)
        .await
        .unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(
            (&row.workspace_id, &row.workspace_agent_id),
            (&None, &None),
            "unrestricted: every draft passes here"
        );
        assert_eq!(row.source_kind, SOURCE_DRAFT);
        assert_eq!(row.state, "complete");
        assert_eq!(
            row.interval_start,
            crate::engine::repo::model_usage::canonical_ts(online)
        );
        assert_eq!(
            row.interval_end,
            crate::engine::repo::model_usage::canonical_ts(online + chrono::Duration::seconds(30))
        );
    }
}
