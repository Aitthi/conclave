# First-class Antigravity CLI backend

owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop

## Goal

Make `antigravity` a first-class CLI harness that Conclave can persist, launch in its PTY, stop/resume through the shared lifecycle, bootstrap with Conclave identity/skills, and report honestly when context usage is unavailable.

## Dependencies and non-goals

- Base includes lifecycle migration v27 and merge `376d6bf64efa719dfcffe4c6aca74f8b2a9ba8b0` or later.
- User-installed/authenticated `agy` is discovered through the login shell PATH; Conclave neither bundles it nor persists an absolute executable path.
- v1 does not add AGY to the one-shot AI drafter (`runtime/cli_oneshot.rs` / `draft.rs`), does not enable rtk hooks, does not enable AGY sandbox, does not import AGY plugins/agents, and does not implement native conversation resume. Those require separate proven contracts.

## Persisted contract and migration v28

- Add `antigravity` to the `agent_definition.cli_kind` CHECK.
- Add nullable `effort TEXT CHECK(effort IN ('low','medium','high'))`; null means Auto/omit.
- Because SQLite cannot alter the existing CHECK, rebuild `agent_definition` while preserving every current column, including dormant schema-only `proxy_enabled`, and every row/relation.
- Inbound foreign keys from `workspace_agent`, `agent_tool`, `agent_skill`, and `fusion_panel_member` make the normal in-transaction migration unsafe. Migration v28 must acquire one connection, set `PRAGMA foreign_keys=OFF` before `BEGIN`, rebuild/copy/rename and bump `user_version` atomically, commit, run `PRAGMA foreign_key_check`, and restore `foreign_keys=ON` on both success and error paths. Do not paste a rebuild into the existing already-open transaction.
- Add a populated v27->v28 test covering all inbound relations, every retained column (especially `proxy_enabled`), invalid cli_kind/effort checks, empty foreign-key check, idempotency and latest/contiguous version assertions.
- Thread `effort` through AgentDef row/list/input/create/update/serialization and `agent.save`. Validate it only as null/low/medium/high; non-Antigravity definitions normalize it to null.

## Launch contract

- Extract/extend a pure harness launch builder rather than adding another opaque arm to the long spawn body.
- Conclave-owned order is:
  `agy [--model Q(model)] [--effort Q(effort)] [--mode Q(accept-edits|plan) | --dangerously-skip-permissions] --prompt-interactive Q(bootstrap) [custom_args]`
- `Q` is the existing shell-quote contract. Blank model/effort omit flags. Typed Conclave flags precede expert `custom_args`.
- Harness-aware permission mapping:
  - null/`auto` => no AGY flag (Default)
  - `acceptEdits` => `--mode accept-edits`
  - `plan` => `--mode plan`
  - `bypassPermissions` => `--dangerously-skip-permissions`
- Never map `auto` to bypass. Preserve existing Claude/Codex behavior.
- Use the existing outer `$SHELL -l -i -c`, PATH shim, cwd and `CONCLAVE_WORKSPACE_ID`/`CONCLAVE_INSTANCE_ID` environment.
- Before returning spawn success, run a login-shell `command -v agy` preflight. Missing binary returns a clear launch error with the official install URL/direction; do not let an inner command-not-found look like a successful PTY spawn.
- Use `--prompt-interactive` for `agentctx::bootstrap_preamble` plus the skill-sidecar pointer and a short acknowledge-and-wait instruction. Do not mutate `~/.gemini`, repository `.agents`, AGY plugins or account settings.
- Gate all rtk computation, hook setup and rtk-awareness copy to Claude/Codex so AGY is never told a hook exists. Do not pass `--sandbox` in v1.
- Generic PTY transport, bracketed input, Conclave lifecycle eligibility, Stop/Resume and fresh-process handoff restart are reused unchanged. Do not use `agy --continue`; several agents share the same cwd. Resume remains fresh process + Conclave handoff.

## Context truthfulness

- `TranscriptContextReader` must treat Antigravity as unsupported and never scan opaque `.pb` conversation files or estimate from TUI bytes.
- On every AGY generation clear `session.context_tokens` and `context_limit` to NULL via a repository method; do not write `0/200000` and do not emit a fake context event.
- Existing UI hides the meter when fields are null. Snapshot/handoff summary must remain valid with unknown usage.

## Exact boundary

- `src-tauri/src/engine/migrations/0028_antigravity_cli.sql`
- `src-tauri/src/engine/db.rs`
- `src-tauri/src/engine/repo/agent_definition.rs`
- `src-tauri/src/engine/repo/session.rs`
- `src-tauri/src/engine/commands/agent.rs`
- `src-tauri/src/engine/commands/instance.rs`
- `src-tauri/src/engine/runtime/transcript_context.rs`
- `src-tauri/src/engine/runtime/launch_common.rs`

Do not edit one-shot drafter modules or `src/` UI in this lane. If the safe v28 transaction needs one additional DB abstraction file, file a task challenge with exact evidence before editing it.

Ruling `6e9901a5`: adding the required `AgentDefinitionInput.effort` field makes the exhaustive test helper in `src-tauri/src/engine/commands/draft.rs` fail to compile. Keep that path outside this task boundary; supplemental task `antigravity-draft-input-compat` owns the sole mechanical `effort: None` initializer. This is constructor compatibility only and does not make Antigravity eligible for one-shot drafting.

## Required verification

- Migration tests described above on a real populated v27 database.
- Repository/save/list/update effort round trip and invalid values.
- Pure launch matrix for blank/named model, blank/all efforts, every execution mode, quote injection resistance, custom arg ordering, no rtk/sandbox claims and preserved Claude/Codex argv.
- Missing-binary preflight test through an isolated PATH/login-shell seam.
- AGY spawn inherits lifecycle stopped guards; Stop/Resume and restart remain shared.
- Context fields clear to null for AGY and existing Claude/Codex meter tests remain green.
- Run focused tests, `git diff --check`, diff-scoped rustfmt per the recorded baseline ruling, and `cargo test --manifest-path src-tauri/Cargo.toml`.
- Live authenticated smoke is manual/non-blocking: do not consume provider quota or modify settings as part of automated gates.
- Commit boundary only and file READY with SHA and exact gate evidence. Implementation judgment within this contract belongs to the implementer; contract conflicts go to Aoki, final.
