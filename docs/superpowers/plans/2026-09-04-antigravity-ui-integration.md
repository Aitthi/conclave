# Complete Antigravity CLI product integration

owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop

## Goal

Make Antigravity a user-reachable first-class CLI in Conclave: a user can select
it in Builder, see whether `agy` is available through the same login-shell lookup
used at launch, configure its model/effort/execution mode, save and reopen the
definition, recognize it across roster/supervisor/task surfaces, add it to a
workspace, and launch it through the reviewed backend.

## Dependencies and accepted canon

- Start only after a fixed descendant of backend candidate `6030ece3` passes
  independent review and is merged to main. Raw `6030ece3` is blocked by review
  note `7cd29bcb`.
- Visual canon: `design/screens/antigravity-cli.tsx` at
  `78c98058fb6e`; behavior checklist: task `antigravity-ui-canon`, note
  `de222061`; designer/escalation target: Hardwell
  (`aee0133c-2b94-4ce7-b39a-01ceb26afeb9`).
- Discovery and exact gap evidence: task `antigravity-ui-gap-audit`, note
  `07c48f67`.

## Product rulings

- `antigravity` is a real `cliKind`, not `custom` and not an inferred provider.
- Model remains free text. Blank means the authenticated AGY default; do not
  hardcode or query a model catalogue in this task.
- Effort is `Auto | low | medium | high`; Auto serializes as omitted/null.
- AGY's control is labeled **Execution mode**:
  `Default | Accept edits | Plan | Bypass permissions`. Default emits no flag.
  Never translate Claude/Codex `auto` into AGY bypass.
- Hide unsupported AGY controls: context window, rtk, sandbox, and custom
  environment. Keep Skills and expert custom arguments.
- Builder availability and runtime spawn must use one shared login-shell PATH
  probe for bare `agy`. Never persist an absolute executable path and never
  execute renderer-supplied command text.
- AGY context usage remains unavailable/null; never fabricate `0 / 200k`.
- One-shot AI team drafting, dynamic model catalogues, AGY sandbox/rtk hooks,
  native conversation resume, plugin import, and authenticated live smoke are
  explicitly out of scope.

## File boundary

- `src/ipc/types.ts`
- `src/ipc/commands.ts`
- `src/components/Builder.tsx`
- `src/lib/providerLabel.ts`
- `src/components/Roster.tsx`
- `src/components/SupervisorPicker.tsx`
- `src/components/LaneBoard.tsx`
- `src/components/SkillAssistPanel.tsx`
- `src/fixtures/scenarios/data.ts`
- `src/fixtures/scenarios/default.ts`
- `src/fixtures/scenarios/empty.ts`
- `src-tauri/src/engine/router.rs`
- `src-tauri/src/engine/commands/instance.rs`
- `src-tauri/src/engine/commands/skill_draft.rs`

Do not edit `AgentDrafter.tsx`, `applyTeamDraft.ts`, `Library.tsx`, migrations,
agent-definition persistence, launch argv construction, transcript readers, or
any other path without filing an evidence-backed task challenge first.

## Implementation

### 1. Wire types and save payload

- In `src/ipc/types.ts`, widen only persisted/live `AgentDefinition.cliKind` and
  `WorkspaceAgent.cliKind` with `"antigravity"`; add optional
  `effort?: "low" | "medium" | "high"`; widen the relevant persisted permission
  mode with `"default" | "acceptEdits" | "plan"`.
- Keep `DraftAgent.cliKind` Claude/Codex-only: one-shot drafting is deferred.
- Mirror the same unions and optional `effort` in the `agent.save` request in
  `src/ipc/commands.ts`.

### 2. Shared availability IPC

- Refactor the backend candidate's login-shell binary check so both spawn and a
  new read-only `instance.cliStatus` command share exactly the same process/PATH
  lookup and error handling.
- The renderer request is `{ cliKind: "antigravity" }`; the Rust handler maps a
  fixed allowlist to bare `agy`. It must not accept an executable name or path.
- Response is `{ available: boolean, installUrl: string }`. A normal login-shell
  `command -v agy` miss is `available:false`; failure to start/query the shell is
  a transport/command error, not a false missing result.
- Register the command in `src-tauri/src/engine/router.rs`, type it in
  `src/ipc/commands.ts`, and keep spawn's existing clear missing-binary error.
- Add Rust tests for available, missing, and shell-error behavior plus rejection
  of any unsupported `cliKind`.

### 3. Builder state, normalization, and save behavior

- Add Antigravity as an enabled fourth CLI segment and include it in the existing
  CLI configuration path.
- Seed effort from `initialDef.effort`; represent Auto as `undefined`.
- Query `instance.cliStatus` whenever an Antigravity definition is selected or
  opened. Provide **Check again** and the official install link. Disable Save only
  while checking or when `agy` is confirmed missing; a probe transport error has
  distinct retry copy and must not masquerade as missing installation.
- Preserve model and custom-argument text across harness switches. Serialize
  model only when non-blank and effort only for Antigravity/non-Auto.
- Normalize modes both directions: Claude/Codex `auto` becomes AGY `default`,
  never bypass; leaving AGY maps `default|acceptEdits|plan` to Claude/Codex
  `auto`, while explicit bypass may stay bypass.
- Serialize AGY Default as omitted/null; named modes as
  `acceptEdits|plan|bypassPermissions`.
- Show the canon's free-text model field (`Auto (authenticated default)`), effort
  choices, Execution mode choices/help, and warning treatment only for Bypass.
  Hide context window, rtk, sandbox, and custom environment for AGY; retain Skills
  and custom args.

### 4. Provider identity and dense-row safety

- Extend `providerLabel` with `antigravity -> Antigravity`. Blank model produces
  provider-only text; arbitrary named models remain text and must not be mapped to
  a guessed vendor.
- Roster and Change supervisor already consume the shared helper. Bound their
  provider chips so long AGY models truncate without increasing row height:
  Roster max width 104px, picker max width 145px, `min-w-0` + truncate, full value
  in `title`, and a complete `aria-label`.
- Add the no-role `antigravity -> Antigravity` fallback in LaneBoard.

### 5. Skill Assist and fixtures

- Widen the existing-definition chooser in `SkillAssistPanel.tsx` and the
  matching Rust validation guard/error in `skill_draft.rs`. Add a focused Rust
  test proving Antigravity passes the pre-resource guard while unknown/custom/
  non-CLI values still fail. Do not widen the one-shot Agent Drafter.
- Add fixed-timestamp fixture definitions/instances for a blank-model AGY and a
  long named-model AGY. Their fixture sessions must omit context tokens/limit.
- Make one role-less AGY visible in LaneBoard data. Add `instance.cliStatus`
  handlers: available in `default`, missing in `empty`. Missing handlers must
  remain loud.

## Risk ledger

- Cross-kind permission leakage is the highest risk. Exercise switches in both
  directions and assert Auto never becomes bypass.
- A frontend-only Skill Assist change still fails at runtime; its Rust allowlist
  must land in the same task.
- Builder and spawn PATH checks must not diverge. Both call the same helper.
- Long provider/model text can silently break dense row height even with a green
  typecheck; inspect real pixels and accessible/title text.
- Fixture AGY sessions that inherit generic 42k/200k synthesis would lie about
  metering; keep both fields absent.
- Re-scan exact `cliKind` comparisons after implementation. Remaining intentional
  Claude/Codex-only paths are the one-shot DraftAgent/AgentDrafter flow; every
  other exclusion needs a recorded reason.

## Verification

- `pnpm exec tsc --noEmit`
- `pnpm build`
- `cargo test --manifest-path src-tauri/Cargo.toml antigravity`
- `cargo test --manifest-path src-tauri/Cargo.toml skill_draft`
- `cargo test --manifest-path src-tauri/Cargo.toml`
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
- `git diff --check`
- Classify every remaining exact `cliKind`/`cli_kind` comparison; attach the
  intentional-exclusion list in the READY note.

Standing UI pixel protocol, after verifying :1420 belongs to this worktree:

- Record `pnpm uishot builder` and `pnpm uishot builder --scenario empty`.
- Record `pnpm uishot home` and `pnpm uishot laneboard`.
- Open and visually inspect `.shots/builder-default.png`,
  `.shots/builder-empty.png`, `.shots/home-default.png`, and
  `.shots/laneboard-default.png`; attach all paths in READY.
- Manually inspect and capture Builder's AGY ready, missing, bypass, and compact
  920x720 states, plus Change supervisor with a long model chip. Attach paths and
  verify Cancel/Retry/link/control accessibility.

## Done

A user can create and reopen Antigravity definitions with correct round trips,
availability is truthful and shared with launch, provider identity is visible and
bounded, Skill Assist accepts AGY consistently, all automated gates are green,
and every required real-app PNG has been opened and recorded. Authenticated AGY
provider execution remains a human-controlled manual smoke because it can spend
quota.
