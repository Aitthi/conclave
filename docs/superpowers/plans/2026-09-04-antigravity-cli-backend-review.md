# Antigravity CLI backend independent review

owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop

## Goal

Independently review candidate `6030ece3fd5fbae51c51ffd3a39a6fed3da23a08` (including `835b31d30e27` and supplemental `81b5ca195772`) against the Antigravity backend plan before integration.

## Reading order

1. `docs/superpowers/plans/2026-09-04-antigravity-cli-backend.md`
2. Task brief `antigravity-cli-backend`, especially READY note `626576bf`, ruling `6e9901a5`, and debug note `fbaa1198`
3. `docs/superpowers/plans/2026-09-04-antigravity-draft-input-compat.md`
4. Diff `a95afe5e192ffdc2729abc4ad926a910ada62a6b..6030ece3fd5fbae51c51ffd3a39a6fed3da23a08`

## Review environment

Claim this task with `conclave task claim`; do not use `lane start`. Per `protocol:review-scratch-worktree`, treat Dabin's implementation worktree as read-only. Create a detached reviewer-owned scratch worktree from `6030ece3fd5fbae51c51ffd3a39a6fed3da23a08` under `/tmp`, run all mutation/fault-injection checks there, and remove it when finished.

## Required review

- Migration: prove `foreign_keys=OFF` happens before `BEGIN` on the same acquired connection; v28 rebuild/copy/version bump is atomic; `foreign_key_check` runs; FK enforcement is restored on success and every error; all current columns including `proxy_enabled` and all inbound relations survive a real populated v27 upgrade; constraints reject invalid `cli_kind` and `effort`; rerun is idempotent.
- Persistence: trace `effort` through row/list/input/create/update/IPC save, validate only null/low/medium/high, and prove non-Antigravity definitions normalize effort to null.
- Launch: inspect exact argv ordering and shell quoting for blank/named models, all efforts and permission modes; ensure `auto` emits no bypass; typed flags precede custom args; missing `agy` fails before spawn through the login-shell PATH seam with a useful official install hint; preserve Claude/Codex argv.
- Runtime: prove AGY receives the bootstrap preamble via `--prompt-interactive`, never receives rtk or sandbox setup, reuses lifecycle Stop/Resume/restart guards, and clears context fields to null without parsing `.pb` files or emitting fake readings.
- Supplemental: verify `81b5ca195772` adds only `effort: None` to the drafter test helper and does not broaden one-shot drafter eligibility.
- Inspect the initial red gate and `6030ece`; confirm it only advances stale latest-version expectations and does not mask a migration failure.

## Verification

- Run focused migration, Antigravity launch/save/spawn/context, and drafter tests.
- Run `cargo test --manifest-path src-tauri/Cargo.toml` independently.
- Run diff-scoped rustfmt and `git diff --check` over the full candidate range.
- If mutation testing is practical, falsify at least the pre-BEGIN FK ordering, effort normalization, and `auto` permission behavior in the scratch worktree.

## Output

Post one task note with verdict `SHIP` or `FIX`, findings ordered by severity, exact paths/lines and commands. Any fix returns to Dabin on the implementation task; do not commit product changes from the review task.

## File boundary

This is read-only review coordination. Only this plan file is writable under the task; candidate product files are inspected/tested in the detached scratch worktree.
