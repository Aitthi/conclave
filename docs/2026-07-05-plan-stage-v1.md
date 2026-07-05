# Plan: conclave stage v1 — shared-checkout collaboration without worktrees (jj-inspired)

Date: 2026-07-05 · Owner: Detoro bfb737ff · authority: in-loop
Task: `stage-v1` · Implementer: Dew 40d90aed · Reviewer: Mellow (LAND, blocking)
Status: APPROVED by human 2026-07-05 (v1 verbs + snapshot/undo op-log layer, both in scope).

## Why

Lane worktrees isolate agents but cost separate builds and teardown. The
shared-checkout fallback (task fileBoundary + `lane guard`) partitions by
file, but the guard only BLOCKS mistakes (env-var-gated pre-commit hook,
never installed here) — the safe path is not the easy path. Git's single
shared index is the root hazard: any raw `git commit` can sweep a peer's
staged work (accident class b9ab709), and shared author identity makes
commits unattributable (case c3d8fcb).

jj (Jujutsu) solves this class by removing the index and treating the
working copy as content-addressed commits with an operation log. We borrow
exactly three of its concepts — private index, change-identity-per-task,
op log — as a small CLI-only feature. NOT a VCS migration (human ruling
2026-07-05: concept only, no jj adoption).

## Decisions (settled, encode exactly these)

1. **CLI-only feature.** Everything lives in `src-tauri/src/bin/conclave-cli.rs`
   plus doc rows in `src-tauri/skills/tool-map/SKILL.md`. No engine routes, no
   migration, no router.rs/commands/cli.rs changes (ledger writes reuse the
   existing `task note` client path).
2. **Private index per operation**: `stage commit` never touches the shared
   `.git/index`. It builds a temporary index (`GIT_INDEX_FILE=$(mktemp)`),
   `git read-tree HEAD` → `git add -- <boundary paths>` → `write-tree` →
   `commit-tree` → CAS `update-ref`. Two agents committing concurrently
   cannot interfere.
3. **Boundary is the pathspec, always.** Paths come from the task's
   `fileBoundary` (via the existing task-get route). A task without a
   boundary → stage refuses with a clear error. No ad-hoc path overrides —
   the boundary IS the contract; if it's wrong, amend the plan (see the
   router.rs lesson, memory df65b613).
4. **Agent attribution is native git authorship**: `GIT_AUTHOR_NAME=<agent
   name>`, `GIT_AUTHOR_EMAIL=<agent-id>@agents.conclave.local`; committer
   stays the repo default. Trailers: `Conclave-Task: <slug>` and
   `Conclave-Agent: <id>`. Raw human commits stay `detoro` — c3d8fcb-class
   ambiguity ends for agent commits.
5. **Snapshots are git commits on a hidden local ref**
   `refs/conclave/stage/<slug>` (parent = previous snapshot, chain = op log).
   Content-addressed dedup for free; never pushed (local ref namespace);
   survives everything git survives. No DB storage of file contents, ever.
6. **Auto-snap before every mutating operation**: `stage commit` and
   `stage restore` snapshot first, so every state is reachable again —
   including undoing a restore. This is the jj auto-snapshot spirit.
7. **Snapshot refs persist until explicit `stage clear`** — `lane finish` /
   task close do NOT auto-delete (an op log you can lose on teardown is not
   an op log).
8. **Ledger stamp**: after a successful `stage commit`, post a task note
   `stage commit <shortsha> — <msg> (<n> files)` through the existing note
   path. Note failure (engine down) warns but does not roll back the commit.

## Verbs (all under `conclave stage`, usage lines beside the lane family)

- `stage status <ws> <slug>` — `git status --porcelain`, partitioned into
  IN-BOUNDARY vs OUT-OF-BOUNDARY sections by the task's fileBoundary
  prefixes (same prefix semantics as pre_commit_guard.sh `in_scope`).
- `stage diff <ws> <slug>` — `git diff HEAD -- <boundary paths>`.
- `stage commit <ws> <slug> -m <msg>` — decision 2 mechanics, on the current
  branch only (detached HEAD → error). CAS retry loop on `update-ref
  <branch> <new> <expected-old>` failure: refresh HEAD, rebuild tmp index,
  max 3 attempts, then error "branch moving too fast, retry". Tree identical
  to HEAD tree → "nothing to commit in boundary", no ref moved, no snap kept…
  see Task 2 note. Deletions of boundary files commit as deletions.
- `stage snap <ws> <slug> [-m <label>]` — explicit snapshot of boundary
  paths onto `refs/conclave/stage/<slug>`.
- `stage log <ws> <slug>` — `git log --format` over the snapshot ref,
  newest first: short sha, timestamp, label/auto reason.
- `stage restore <ws> <slug> <snapSha>` — auto-snap current state first,
  then `git restore --worktree --source <snapSha> -- <boundary paths>`.
  Never touches the shared index.
- `stage clear <ws> <slug>` — delete the snapshot ref (asks nothing; the
  human protocol is: clear only after merged).

## Task 1 — commit/status/diff + attribution + ledger

`src-tauri/src/bin/conclave-cli.rs`: new `run_stage` dispatcher next to the
lane family (~:243+). Helpers: `stage_boundary(ws, slug) -> Vec<String>`
(task-get route, error if empty), `agent_identity() -> (name, id)` (self-id
mechanism as in `expand_self_args` + agent-list route for the name),
`with_tmp_index(paths, f)` (mktemp + env + cleanup), boundary partition fn
reusing the `in_scope` prefix semantics. Then `stage status|diff|commit`
per decisions 2/3/4/8.

## Task 2 — snap/log/restore/clear (op log)

Same file. Snapshot commit-tree message format:
`snap(<slug>): <label|auto-pre-commit|auto-pre-restore> @ <iso-ts>` with the
same author env as decision 4. `stage commit` auto-snap runs BEFORE the CAS
loop (once, not per retry). Note: when commit then reports
"nothing to commit", the auto-snap it took is harmless (dedup: identical
tree to previous snap should be skipped — compare trees, skip ref update).
`stage log` and `restore` and `clear` per the verbs section. Update
tool-map SKILL.md: one row per verb, plus a one-line warning row that raw
`git add`/`git commit` in the shared checkout is now the discouraged path.

## Tests (mod tests in conclave-cli.rs; temp git repos via std::process)

1. commit: only boundary paths land; out-of-boundary dirty file stays
   dirty and uncommitted; shared `.git/index` file is byte-identical
   before/after.
2. attribution: author name/email from agent identity, committer default,
   both trailers present.
3. CAS: update-ref with stale expected-old fails; retry loop succeeds after
   refresh (move the branch ref manually between attempts in the test).
4. snap → modify → restore roundtrip restores content; the pre-restore
   auto-snap makes the modified state recoverable (restore it back and
   compare).
5. log lists snapshots newest-first with labels.
6. nothing-to-commit: identical tree → error message, branch ref unmoved.
7. boundary file deletion commits as a deletion.
8. task without fileBoundary → stage refuses (all verbs that need paths).
9. duplicate snap (identical tree) skips the ref update (op log stays
   noise-free).

## Boundary

`src-tauri/src/bin/conclave-cli.rs`, `src-tauri/skills/tool-map/SKILL.md`.
Nothing else. (No router route added → router.rs/commands/cli.rs exempt,
per memory df65b613.)

## Gates (commit first, then gate; from src-tauri)

- `cargo test` (full) · `cargo clippy --all-targets -- -D warnings`.
- Mellow LAND (blocking): private-index isolation (test 1 evidence), CAS
  retry correctness, restore-never-touches-index, ref namespace safety
  (nothing under refs/heads or refs/remotes), attribution + trailer
  correctness, tool-map rows match behavior.

## Risk ledger

- Reaches live agents only after next rebuild+install (CLI binary).
- `commit-tree` bypasses git hooks — the pre-commit lane guard does NOT run
  on stage commits. Acceptable: stage enforces the same scope itself,
  stronger (mechanism beats hook). Say so in tool-map.
- `stage restore` overwrites uncommitted boundary changes — mitigated by
  decision 6 auto-snap; still list the files it will touch in the output.
- Advancing the branch ref while a peer is mid-build can surprise them —
  inherent to any shared-checkout commit (raw git included), not new risk.
  Gate SHA-pinning already covers evidence staleness.
- refs/conclave/* is local-only by default; NEVER add it to push refspecs.
- Works in lane worktrees too (harmless, own index anyway) — do not add a
  worktree guard; complexity without payoff.
- conclave-cli.rs is already large; keep stage code in one contiguous
  region with a section comment, mirroring the lane manager region style.
- KNOWN LIMITATION (F1, raised by Dew, confirmed by Mellow at LAND,
  lead-ruled land-as-is): `stage commit` moves the branch ref without
  touching the shared index (decision 2's safety property), so plain
  `git status` — and `stage status`, which wraps porcelain — shows the
  just-committed boundary files as modified until the local index
  refreshes. `stage diff` is immune (diffs against HEAD). Follow-up lane
  `stage-status-head` re-derives `stage status` from HEAD
  (`git diff --name-status HEAD` + `ls-files --others`) and drops the
  porcelain/index path; also adds M1's ancestor-of-snap-ref guard on
  `stage restore`. M2 (n_files unwrap_or(0) cosmetic) and M3 (cwd
  threading inconsistency) accepted as-is.
