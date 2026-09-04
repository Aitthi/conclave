# AI Agent & Team Drafter — Design Spec

**Date:** 2026-09-04 · **Owner:** Detoro (lead, 30fa04f4) · **Status:** approved by human (design approved in chat 2026-09-04; forks ruled: CLI print mode, roster = team)

## Problem

Creating an agent today means filling `Builder.tsx` by hand (name, role, model,
skills, level), and creating a *team* means repeating that N times and then
wiring level/supervisor in the Roster one chip at a time. The human asked
("เพิ่มระบบ สร้างทีม และ agent ด้วย AI") for the app to draft both from a plain
brief: "I need a team to port this service to Rust" → a reviewed, editable
proposal of agents and their reporting lines, applied with one click.

## Decisions (with rejected alternatives)

| # | Decision | Rejected because |
|---|----------|------------------|
| D1 | The drafting model runs as a **one-shot CLI subprocess in print mode**: `claude -p --output-format json --json-schema <S> …` or `codex exec --json --output-schema <file> …`, launched through the same login shell + env as `instance::spawn`. Human ruling 2026-09-04. | *Provider API (`Provider::complete_chat`)* rejected: requires an API key in Keychain; this product's users authenticate through the CLI. *Hidden PTY agent (skill-assist pattern, `skill_draft.rs`)* rejected: heavy (hidden workspace + session), slow, and free-text output must be re-parsed. |
| D2 | **A team is the workspace roster.** No `team` table, no template entity. "Build team" = N `AgentDefinition`s created + added to the workspace + positions set. Human ruling 2026-09-04. | *Reusable Team entity* rejected for v1: migration + CRUD + a view, roughly doubling the work with no user asking for cross-workspace reuse yet. Re-propose only when a user wants to re-apply a team. |
| D3 | The **drafter** is one of the user's existing CLI `AgentDefinition`s (`type == "cli"`, `cliKind` claude-code or codex), chosen in the panel — same convention as `SkillAssistPanel.tsx:67-77`. Its `cliKind`, `model`, `customEnv`, `secretEnvKeys` are reused so proxy/base-URL setups keep working. | *Hard-coded `claude` with a default model* rejected: loses per-user env (base URL, keys) and forces Claude on Codex-only users. |
| D4 | The model may only choose from the **live catalogue** passed in the prompt: roles (`role.list`), skills (`skill.list`, optional builtins + customs), model ids (see D8), existing agent definitions (for reuse). The Rust validator rejects the draft (`AppError::Invalid` with the offending field) when an id is unknown. No silent coercion. | *Free-text role/skill names fuzzy-matched* rejected: silent mismatches are exactly the "mysterious autonomy" PRODUCT.md forbids. |
| D5 | The draft may **propose a new custom role** (`newRole {name, description, skillIds}`) and may **reuse an existing definition** (`existingAgentDefId`). Both resolve through existing commands (`role.save`, `agentDef.addToWorkspace`). | *Builtin roles only* rejected: five roles cannot express "QA automation engineer"; role creation is the part of "create agents with AI" that actually saves time. |
| D6 | **Apply is frontend orchestration over existing commands**, sequential: `role.save` → `agentDef.save` → `agentDef.addToWorkspace` → `instance.setPosition` (supervisors resolved after all agents exist, in dependency order). On failure: stop, surface which agents were created, keep them (no rollback). No new write command, no migration. | *Transactional `team.apply` in Rust* rejected for v1: needs new repo plumbing across three tables; the existing per-command validation already runs; a half-applied team is visible in the Library and trivially deletable. |
| D7 | The one-shot runner is a **Rust `Oneshot` enum with `Live` / `#[cfg(test)] Mock` variants**, mirroring `fusion.rs::ModelCaller` (`commands/fusion.rs:71-101`). Prompt building, schema, parsing and validation are pure functions unit-tested against fixtures. No test spawns a real `claude`/`codex` (same rule as `commands/instance.rs` "binary-free"). | *async-trait abstraction* rejected: the codebase deliberately avoids it (fusion.rs comment). |
| D8 | Model catalogue is lifted from `Builder.tsx:74-90` into a shared TS constant AND mirrored as a Rust constant; both lists are passed to the drafter and checked by the validator. Claude list (human request 2026-09-04): `claude-fable-5-1`, `claude-opus-5`, `claude-sonnet-5`, `claude-haiku-4-5`, plus `claude-opus-4-8` kept for existing rows. Codex context window is not drafted (derived backend-side, `codex_models.rs`). | *Let the model invent model ids* rejected (D4). |
| D9 | Fixed **120 s timeout**, no cancel button in v1. The panel shows elapsed seconds and the drafter's name while waiting. | *Cancel* deferred: needs child-process tracking in `AppState`; add when a user hits the timeout in practice. |
| D10 | Launch flags are NOT drafted: `permissionMode`, `customArgs`, `customEnv`, `contextWindow`, `rtkEnabled`, `harnessMode` keep Builder defaults. | Launch flags are operator policy, not team design (PRODUCT.md principle 5). |
| D11 | All UI copy English (CONTEXT.md convention). Design canon by Arta before frontend implementation (Leadership rule: UI lane without canon is improvisation). | — |

## Architecture

```
Library "Draft with AI"  ─┐                       ┌─ mode=agent ─► Builder (initialDef, no id) ─► agentDef.save
Roster  "Build team with AI" ─┴─► AgentDrafter.tsx ─┤
                                  brief + drafter   └─ mode=team ─► preview table ─► apply orchestration
                                        │                                            (role.save → agentDef.save
                                        ▼                                             → addToWorkspace → setPosition)
                              ipc "draft.agents"
                                        │
                     engine/commands/draft.rs
                       build_catalogue(db)  ─► build_prompt(mode, brief, catalogue) ─► Oneshot::run(drafter, prompt, schema)
                                        │                                                   │
                                        │                                    runtime/cli_oneshot.rs
                                        │                                    $SHELL -l -i -c "<launch>"  (tokio::process, stdin=prompt)
                                        ▼                                                   │
                       parse_draft(stdout) ─► validate_draft(draft, catalogue) ─► DraftResponse
```

### `draft.agents` command

Request (`Commands["draft.agents"]["req"]`):

```ts
{
  mode: "agent" | "team";
  brief: string;               // 1..4000 chars, trimmed, non-empty
  drafterDefId: string;        // AgentDefinition id, must be type "cli", cliKind "claude-code" | "codex"
  workspaceId?: string;        // team mode: current roster is passed to the prompt as context
}
```

Response (`DraftResponse` in `src/ipc/types.ts`):

```ts
interface DraftAgent {
  key: string;                       // draft-local handle, e.g. "lead", "impl-1"; unique within the draft
  existingAgentDefId?: string;       // reuse this definition instead of creating one (mutually exclusive with the fields below)
  name?: string;                     // 1..40 chars
  color?: string;                    // one of Builder's swatch hex values
  cliKind?: "claude-code" | "codex";
  model?: string;                    // from the model catalogue for that cliKind
  roleId?: string;                   // existing builtin/custom role id (mutually exclusive with newRole)
  newRole?: { name: string; description: string; skillIds: string[] };
  skillIds?: string[];               // optional builtin + custom skill ids; mandatory builtins never listed
  defaultLevel?: "junior" | "mid" | "senior" | "principal";
  rationale: string;                 // one sentence shown in the preview
}
interface DraftPosition { key: string; level: DraftAgent["defaultLevel"]; supervisorKey?: string | null }
interface DraftResponse { agents: DraftAgent[]; positions: DraftPosition[]; notes: string; drafter: { defId: string; cliKind: string; model: string } }
```

Rules the validator enforces (each violation → `AppError::Invalid("draft.<field>: <reason>")`):

- `mode == "agent"` → exactly one agent, `positions` empty.
- `key` unique; every `supervisorKey` names another key; no cycles (reuse `lib/positions.ts` semantics: a DAG rooted at keys with no supervisor).
- `existingAgentDefId` must exist; when set, no other agent fields except `key`, `rationale`.
- `roleId` xor `newRole`; `newRole.skillIds` and `skillIds` ⊆ catalogue skill ids; `newRole.name` not already a role name (case-insensitive).
- `model` ∈ catalogue for `cliKind`; `color` ∈ swatches; `defaultLevel` ∈ LEVELS.
- Team mode: 1..12 agents.

### Prompt (built by `draft::build_prompt`, pure)

Sections, in order: (1) task — draft one agent / a team for the brief; (2) the JSON contract (the same schema handed to `--json-schema`, so the model sees field docs); (3) catalogue — roles with description and default skills, optional skills with description, models per cliKind, colour swatches, levels with one-line meaning (from `roles/*/ROLE.md` and `lib/positions.ts`); (4) existing agent definitions (id, name, role, model) with the instruction to reuse via `existingAgentDefId` when one already fits; (5) team mode only: the current roster of `workspaceId` (name, role, level, supervisor) so the draft extends rather than duplicates it; (6) house rules — exactly one top-level lead in team mode, reviewers do not supervise implementers, prefer the fewest agents that cover the brief, `rationale` ≤ 1 sentence, English only; (7) the brief, fenced. Prompt text lives in `src-tauri/src/engine/commands/draft_prompt.rs` as `const` fragments so the unit test can assert the catalogue is embedded.

### `runtime/cli_oneshot.rs`

```rust
pub struct OneshotSpec { pub cli_kind: CliKind, pub model: Option<String>, pub prompt: String,
                         pub json_schema: serde_json::Value, pub extra_env: Vec<(String,String)>, pub cwd: PathBuf,
                         pub timeout: Duration }
pub enum Oneshot { Live, #[cfg(test)] Mock(Result<serde_json::Value, String>) }
impl Oneshot { pub async fn run(&self, spec: &OneshotSpec) -> Result<serde_json::Value, OneshotError> }
```

`Live` builds the launch string with the SAME helpers `instance::spawn` uses (`shell_quote`, `effective_claude_model`, `launch_shell`, `agentctx::ensure_conclave_shim` for the PATH prefix — hoist the first three from `commands/instance.rs:50-168` into a small `runtime/launch_common.rs` so both callers share them), then:

- claude-code: `claude -p --output-format json --json-schema '<S>' --no-session-persistence --tools "" [--model <m>]`, prompt on **stdin**, cwd = workspace folder (or the app data dir in agent mode). Result = `structured_output` from the JSON envelope (verified on Claude Code 2.1.260: `{"type":"result","subtype":"success","structured_output":{…},"result":"…"}`); `is_error == true` or `subtype != "success"` → `OneshotError::Model(result_text)`.
- codex: write the schema to a temp file, `codex exec --json --ephemeral --skip-git-repo-check --output-schema <schema.json> -o <last.json> [-m <m>] -` with the prompt on stdin; result = parsed contents of `<last.json>` (codex-cli 0.153.2 flags confirmed via `--help`; behaviour verified in Lane A's gate, see Risk R2).
- Env: `extra_env` = drafter `customEnv` + secret env resolved from Keychain exactly as `instance.rs:883-920` does, plus `CONCLAVE_DRAFT=1`. No `CONCLAVE_WORKSPACE_ID`/`INSTANCE_ID` (there is no instance).
- Timeout via `tokio::time::timeout`; on expiry kill the child (`kill_on_drop(true)`) → `OneshotError::Timeout`.
- Non-zero exit → `OneshotError::Exit { code, stderr_tail }` (last 2 KB).

### Frontend

- `src/components/AgentDrafter.tsx` — overlay (same shell as `Builder.tsx`'s modal, opened from `AppShell.tsx` state `showDrafter: { mode, workspaceId? } | null`). Contents: brief textarea, drafter picker (CLI defs only; defaults to the first; empty state "Configure a Claude Code or Codex agent first" with a button to open Builder), Draft button, waiting state (drafter name + elapsed), error state (message + Retry), result state.
  - agent mode: on success call `onDraftAgent(draftToInitialDef(draft))` → AppShell sets `builderInitialDef` to an id-less `AgentDefinition` and opens Builder. `Builder` key must vary per draft (`key={"draft-" + counter}`; `AppShell.tsx:614-632` keys on `id ?? "new"` today). Builder shows "Drafted by <name>" chip until first edit. Because `isEditing = Boolean(initialDef)` (`Builder.tsx:184`), add an explicit `isDraft` prop so labels read "New agent".
  - team mode: editable preview table — columns Name, Role (select: catalogue + "new: <name>"), Model, Level, Reports to (select of other draft keys / existing roster members / none), Reuse (badge when `existingAgentDefId`), Rationale. Apply button runs the orchestration in `src/lib/applyTeamDraft.ts` (pure planner `planTeamApply(draft, roster) → Step[]` + executor), with a progress list (per-agent state: pending / created / added / positioned / failed). On completion `onSaved()` bumps `libraryRefreshKey`/`agentsVersion` like Builder does.
- Entry points: Library footer beside "New agent" (`Library.tsx:235-244`) → agent mode; Roster footer beside "Add agent" (`Roster.tsx:701-710`) → team mode. Menu/command palette: `draft-agent`, `draft-team`.
- Fixtures: `draft.agents` handler in `default.ts` and `empty.ts` returns a fixed literal team (3 agents, one `newRole`, one reuse) so `pnpm uishot` can render the preview via a `?fixture=default#view=drafter` route (add `drafter` to the view map in `AppShell.tsx:139-152`).
- Lifted constants: `src/lib/modelCatalogue.ts` exporting `CLAUDE_MODELS`, `CODEX_MODELS`, `COLOR_SWATCHES`; Builder imports them (no behaviour change).

## Error handling

| Failure | Where | User sees |
|---|---|---|
| No CLI drafter configured | panel | empty state with "Open Builder" |
| CLI not on PATH / non-zero exit | `OneshotError::Exit` | "claude exited with code N" + stderr tail, Retry |
| Timeout 120 s | `OneshotError::Timeout` | "The drafter did not answer in 120 s", Retry |
| Model output fails validation | `AppError::Invalid` | the field and reason; Retry re-runs with the same brief |
| Apply step fails mid-way | `applyTeamDraft` | progress list marks the failed row, others stay "created"; message names how many were created and that they are in the Library |

## Testing

- Rust (`cargo test -p conclave` in `src-tauri`): `build_prompt` embeds every catalogue id; `parse_draft` on the recorded claude envelope fixture and on a codex `last.json` fixture; `validate_draft` rejects each rule in D4/validator list (one test per rule); `Oneshot::Mock` drives `draft::run` end-to-end without a binary; claude/codex launch-string builders asserted byte-for-byte.
- Manual gate (Lane A, recorded with `conclave task gate`): a real `claude -p` run of the shipped prompt against the "port this service to Rust" brief, output attached as a note; same for `codex exec` if codex is logged in (else note the skip).
- Frontend: `pnpm exec tsc --noEmit`, `pnpm build`, `pnpm uishot drafter` (+ `--scenario empty`), `pnpm uishot library`, `pnpm uishot home` — PNGs opened and inspected (UI Pixel Gate). `planTeamApply` gets a small pure test only if a runner exists; there is none, so its cases are asserted through the fixture preview + a documented manual apply.

## Risk ledger

- **R1 `claude -p` under `-i` login shell**: interactive shell rc files may print banners to stdout and corrupt the JSON. Mitigation: parse the LAST top-level JSON object in stdout (scan for the final `{"type":"result"` line); if `-i` proves noisy, drop `-i` for one-shots (keep `-l`).
- **R2 codex structured output**: `--output-schema` behaviour is documented but unverified here. Lane A verifies with a real run; if it cannot honour the schema, parse the assistant's last message with `fusion::strip_code_fences` + serde and rely on the validator.
- **R3 prompt size**: catalogue + roster fits comfortably (< 8 KB); brief capped at 4000 chars.
- **R4 shared helper hoist** (`shell_quote`, `effective_claude_model`, `launch_shell`): touches `commands/instance.rs`, a 4000-line file others edit. Hoist as pure moves with no behaviour change, gated by the existing instance tests, in Lane A's first commit.
- **R5 proxy env**: no proxy env is injected by `instance.rs` (grep confirms); drafts therefore bypass the context proxy — acceptable, one-shot requests carry no history.
- **R6 Builder `isEditing`** semantics for id-less drafts (see Frontend) — the `isDraft` prop guards copy; save path already posts `id: undefined`.

## Out of scope (v1)

Team templates / cross-workspace reuse (D2); cancel (D9); drafting launch flags (D10); streaming the model's thinking; drafting skill *content* (that is skill-assist); auto-spawning the created team.

## Records

Spec: this file. Plan: `docs/superpowers/plans/2026-09-04-ai-agent-team-drafter.md` (written next). Tasks: `drafter-oneshot-rust` (Lane A), `drafter-ui-canon` (Lane B), `drafter-frontend` (Lane C). Blackboard: `plan:ai-agent-team-drafter`.
