# Review Antigravity CLI product integration

owner: 2004f459-52ad-445c-9c70-e605a0ffdfe3 · authority: in-loop

## Goal

Independently review candidate `9aad40fcdddf4d576e9b8fdd4e914f6a16aa0e7e`
against the accepted Antigravity product-integration plan and design canon before
it can merge.

## Reading order

1. `docs/superpowers/plans/2026-09-04-antigravity-ui-integration.md`
2. Task brief `antigravity-ui-integration`, especially READY note `9fc6875c`
3. `design/screens/antigravity-cli.tsx` at `78c98058fb6e`
4. Task `antigravity-ui-canon`, behavior note `de222061`
5. Diff `736d1460c129839a78719911f93b0dba369a88cb..9aad40fcdddf4d576e9b8fdd4e914f6a16aa0e7e`

## Review environment

Claim without a lane. Treat the implementer's worktree as read-only; use a
reviewer-owned detached scratch worktree when running tests or mutations. Do not
commit product changes.

## Required review

- Trace the real user path end to end: Builder selection/status/configuration ->
  typed `agent.save` -> reopen round trip -> add to workspace -> generic spawn ->
  reviewed AGY launcher. Reject any path that still needs manual DB edits.
- Prove `instance.cliStatus` and spawn share the same login-shell probe, renderer
  input cannot select a command/path, missing `agy` is distinct from shell/IPC
  failure, and the official install URL is not used as an execution input.
- Exercise harness switching in both directions. Claude/Codex Auto must never
  become AGY bypass; AGY default/accept-edits/plan must leave a valid
  Claude/Codex selection; explicit bypass preservation must be intentional.
- Verify blank model/Auto effort/Default mode serialize as omission/null and
  named model/effort/mode round-trip without leaking AGY-only state to other
  harnesses.
- Verify unsupported controls are hidden for AGY while Skills/custom args remain;
  missing status disables Save, transport failure uses different copy, and retry/
  install controls remain keyboard/accessibility reachable.
- Verify provider identity is centralized and long model chips cannot change row
  height in Roster or Change supervisor; full values remain available via title
  and accessible name. Check role-less LaneBoard fallback.
- Verify Skill Assist frontend and Rust allowlists move together and its focused
  tests reject unknown/custom/non-CLI cases.
- Verify fixtures use fixed timestamps, implement every new command in both
  scenarios, and never fabricate context tokens/limits for AGY.
- Classify every remaining exact CLI-kind comparison. Claude/Codex-only one-shot
  drafter/catalogue paths are intentional; flag any other omission.
- Inspect all ten PNGs named in READY note `9fc6875c` directly and compare the
  Builder states and dense-row behavior to the accepted canon.

## Verification

- `pnpm exec tsc --noEmit`
- `pnpm build`
- `cargo test --manifest-path src-tauri/Cargo.toml antigravity`
- `cargo test --manifest-path src-tauri/Cargo.toml skill_draft`
- `git diff --check 736d1460c129839a78719911f93b0dba369a88cb..9aad40fcdddf4d576e9b8fdd4e914f6a16aa0e7e`
- Diff-scoped rustfmt.
- Compare strict clippy output to the recorded base warning. A new warning is a
  blocker; the byte-identical pre-base `collapsible_if` alone is not charged.
- Mutation/falsification where practical: command allowlist, Auto-to-bypass,
  missing-handler fixture, and AGY context-null behavior.

## Output

Post one note with verdict `SHIP` or `FIX`, findings ordered by severity, exact
paths/lines, screenshot inspection result, and independent command evidence. Any
finding returns to the implementation task; do not fix it in the review task.

## File boundary

- `docs/superpowers/plans/2026-09-05-antigravity-ui-integration-review.md`
