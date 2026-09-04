# All first-class CLI effort controls and bypass default

owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop

## Goal

Give every first-class CLI provider (`claude-code`, `codex`, and `antigravity`)
the same Builder effort control that Antigravity already has, persist the value,
and apply it to the provider's real launch contract. Add `gpt-6-astra` to the
Codex model presets. New first-class CLI agent definitions must default to
`bypassPermissions` from every product creation path.

## Settled product contract

- The shared effort choices are `Auto`, `low`, `medium`, and `high`. `Auto` is
  stored as NULL/absent and omits the provider override. The shared intersection
  is intentional: do not expose Claude-only or model-specific higher levels in
  this change.
- Provider launch mapping:
  - Claude Code: `--effort Q(value)`.
  - Codex: `-c Q(model_reasoning_effort="value")` so the override is a TOML
    string and never writes the user's `~/.codex/config.toml`.
  - Antigravity: keep the existing `--effort Q(value)` behavior.
- Typed effort overrides must precede expert `custom_args`, use the existing
  `shell_quote` contract, and remain absent for Auto/blank values.
- The Builder shows the same segmented Effort control for all three first-class
  CLIs. Copy must describe Auto generically (provider default/omitted override),
  not claim that every provider owns an `--effort` flag.
- A newly created first-class CLI definition defaults to
  `bypassPermissions`. This applies consistently to:
  - a blank Builder;
  - an id-less draft opened in Builder;
  - the one-shot Team Draft apply path;
  - direct `agentDef.save` creation when a first-class CLI payload omits
    `permissionMode`.
- Existing definitions are not migrated. Editing an existing definition keeps
  its explicit stored permission mode; a legacy existing row with NULL keeps the
  pre-change provider fallback (`default` for Antigravity, `auto` for
  Claude/Codex) unless the user changes it. Do not turn an unrelated edit into a
  silent permission escalation.
- Provider switching retains the current explicit effort and permission choices
  when valid. The shared effort values are valid for all three providers.
- `custom`, `chat`, and `orchestrator` are outside this contract because the
  custom harness is not a first-class runtime provider and non-CLI agents have no
  provider launch flags.

## GPT-6 Astra Codex preset

- Add `gpt-6-astra` as the first/highest-priority Codex preset in both the
  TypeScript catalogue and its byte-for-byte Rust mirror. This automatically
  exposes it in Builder and Agent Drafter and includes it in the drafter prompt
  and validation allowlist.
- Add `gpt-6-astra` to `codex_model_context_window` with the current
  Codex-effective value `272_000`, verified from `codex-cli 0.153.2` via
  `codex debug models`. Keep the exact-match/unknown-model fallback unchanged.
- Add focused tests that lock the mirrored catalogue membership/order and the
  `272_000` context override. Do not infer the API headline window; this module
  intentionally uses the effective window reported by the installed Codex
  runtime.

## Persistence and validation

- Keep the existing nullable `agent_definition.effort` column and its
  `low|medium|high` CHECK; no migration is required.
- Generalize command and repository normalization so effort survives only for
  `claude-code`, `codex`, and `antigravity`, and is cleared for custom/non-CLI
  definitions.
- Keep rejecting any nonblank effort outside `low|medium|high` before
  persistence. Update comments and TypeScript contract docs that currently call
  the field Antigravity-only.
- On create only, `agentDef.save` supplies `bypassPermissions` when a
  first-class CLI request omitted permission mode. On update, omission keeps the
  existing behavior; do not use the new-create default to rewrite old rows.
- Explicit Antigravity `Default` must be sent as `permissionMode: "default"`,
  including on new definitions. The former Builder omission convention conflicts
  with the new create-time default: omitting a user-selected Default would save
  Bypass instead. The existing Antigravity launch builder already omits flags
  for explicit `default`, so this needs no launch mapping change. Verify a new
  Antigravity definition explicitly set to Default saves and reopens as Default.

## Exact boundary

- `src/components/Builder.tsx`
- `src/components/AgentDrafter.tsx`
- `src/lib/applyTeamDraft.ts`
- `src/lib/modelCatalogue.ts`
- `src/ipc/types.ts`
- `src/ipc/commands.ts`
- `src-tauri/src/engine/codex_models.rs`
- `src-tauri/src/engine/commands/draft.rs`
- `src-tauri/src/engine/repo/agent_definition.rs`
- `src-tauri/src/engine/commands/agent.rs`
- `src-tauri/src/engine/commands/instance.rs`
- `src-tauri/src/engine/runtime/launch_common.rs`

Do not edit migrations, provider discovery, permission-mode launch mappings, or
the Antigravity design canon. If a required compile fix
falls outside this boundary, file a task challenge with exact evidence before
editing it.

## Design canon and UI behavior

- Canon: `design/screens/antigravity-cli.tsx` at commit
  `3af7b2995eae1bdc9c6ace81bc184aae172e6564`; use its existing Effort segmented
  control without inventing a second visual treatment. Designer escalation:
  Hardwell (`aee0133c-2b94-4ce7-b39a-01ceb26afeb9`).
- Keep Effort in the existing CLI configuration card, before permission mode.
- New blank Builder should visibly select Bypass for Claude Code (the default
  provider), and provider changes should preserve Bypass.
- Existing danger/warning treatment remains visible whenever Bypass is selected.

## Required tests and gates

- Repository tests cover create/get/list/update effort round trips for all three
  first-class CLIs and normalization to NULL for custom/non-CLI definitions.
- `agentDef.save` tests cover all three providers, invalid effort rejection,
  direct-create omitted permission -> `bypassPermissions`, and update semantics
  that do not silently rewrite an existing NULL permission.
- Catalogue/drafter tests prove `gpt-6-astra` is offered and accepted as a Codex
  model, and the Codex launch receives `model_context_window=272000`.
- Pure/launch tests prove:
  - Claude has `--effort 'low|medium|high'` and omits it for Auto;
  - Codex has exactly one shell-quoted
    `model_reasoning_effort="low|medium|high"` override and omits it for Auto;
  - Antigravity behavior remains unchanged;
  - typed overrides precede `custom_args` and embedded shell syntax cannot
    escape quoting;
  - all existing permission mappings remain byte-for-byte behaviorally intact.
- Build/typecheck: `pnpm build`.
- Rust formatting and checks: `cargo fmt --manifest-path src-tauri/Cargo.toml --check`,
  focused effort/save/launch tests, then
  `cargo test --manifest-path src-tauri/Cargo.toml`.
- `git diff --check` on the lane diff.
- Formatting ruling for Dabin's challenge `a9ec1768`: main `058cf1a` independently
  reproduces the global fmt failure in six out-of-boundary files (`agentctx.rs`,
  `commands/browser.rs`, `repo/workspace_agent.rs`, `runtime/browser.rs`,
  `runtime/task_timer.rs`, and `lib.rs`). Record the failing global gate; accept
  this lane with standalone rustfmt checks passing on all six in-boundary Rust
  files and a final global check showing no additional offending files. Do not
  expand this feature into the separately planned formatting sweep.
- UI pixel protocol before READY:
  1. Check `lsof -nP -iTCP:1420 -sTCP:LISTEN` and stop any Vite server owned by
     another checkout/worktree.
  2. Run and record `conclave task gate ... -- pnpm uishot builder`.
  3. Run and record
     `conclave task gate ... -- pnpm uishot builder --scenario empty`.
  4. Open and visually inspect both `.shots/builder-default.png` and
     `.shots/builder-empty.png`; READY note must name both paths and what was
     checked (Effort visible, Bypass selected for new Claude, no overlap/cutoff).
- Do not run authenticated provider prompts or consume provider quota.

## Risk ledger

- Codex's setting is a TOML config override, not `--effort`; dropping the inner
  string quotes can change parsing semantics even if the CLI accepts the argv.
- Effort is currently cleared twice (command and repository). Updating only one
  layer produces a UI that appears to save but reopens as Auto.
- A blanket permission fallback in Builder would escalate legacy existing rows
  during unrelated edits. The new-create and existing-edit cases must be tested
  separately.
- Team Draft and id-less Builder drafts have their own defaults; changing only
  the blank Builder leaves inconsistent creation paths.
- `src/lib/modelCatalogue.ts` and `commands/draft.rs::CODEX_MODELS` are a mirrored
  contract; changing one side makes the drafter reject a model the UI offers.
- GPT-6 Astra's API headline window and Codex-effective window differ. This
  codebase uses the latter for auto-compaction safety.
- The UI lane touches `src/`, so a green build without opened PNGs is not READY.

Implementation judgment within this contract belongs to the implementer.
Contract or boundary conflicts escalate to Aoki, final, as a task challenge.
