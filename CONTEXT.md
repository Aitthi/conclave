# Conclave

Native macOS app (Tauri v2 + Rust core + React) for orchestrating multiple AI CLI/chat agents inside per-project workspaces.

## Language

**Skill**:
A reusable capability/prompt module that can be attached to one or more `AgentDefinition`s and is injected into that agent's bootstrap preamble at launch, giving the underlying CLI (Claude Code, Codex) additional standing instructions. Has a short `description` (shown in UI lists) and a separate, longer `content` (the actual markdown instructions injected verbatim). Distinct from a `Tool` (an on/off capability toggle, e.g. an MCP server or builtin) and from the transient "skill running" status card shown in chat (a UI affordance with no backing data model).
_Avoid_: Capability, playbook, macro (for this concept specifically — those are generic terms, not this project's canonical name).

Two kinds:
- **System skill** (built-in): a `SKILL.md` file (frontmatter `name`/`description`/`mandatory` + markdown body as `content`) in a `skills/` folder bundled into the app at build time — NOT a DB row (see ADR 0002). Never editable or deletable by the user. Its id is the skill's folder name. Splits into two subtypes by its frontmatter `mandatory` field (default `true` when omitted):
  - **Mandatory system skill**: auto-attached to every `AgentDefinition`, cannot be detached (see ADR 0002).
  - **Optional system skill**: shown as a pickable item per `AgentDefinition`, like a custom skill, but still read-only content — the user chooses whether to attach it, not what it says (see ADR 0003).
- **Custom skill**: user-created via full CRUD, stored as a `skill` DB row, freely attached/detached per `AgentDefinition` via `agent_skill`.
_Avoid_: "Default skill" for system skill (default implies optional/overridable; only some system skills are optional, and even those aren't user-authored).

**AgentDefinition**:
The reusable template for an agent: name, role, model, launch config (`cliKind`, `permissionMode`, `customArgs`/`customEnv`). Not yet running anywhere on its own.
_Avoid_: Agent config, agent template.

**WorkspaceAgent**:
One instantiation of an `AgentDefinition` placed into a specific workspace — pairs 1:1 with a `Session` (context token tracking). This is the "instance id" referenced throughout the messaging/blackboard systems.
_Avoid_: Agent instance (used interchangeably in code comments, but `WorkspaceAgent` is the canonical row/type name).

**Bootstrap preamble**:
The single system-prompt string (`agentctx::bootstrap_preamble`) injected into a CLI agent at launch — via `--append-system-prompt` for Claude Code, or smuggled through `-c developer_instructions=...` for Codex. Currently just identity (name/role/workspace) + `conclave` CLI shim usage; the natural injection point for Skill content.
