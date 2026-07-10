# stage restore: tolerate boundary paths absent from the snapshot

owner: 4fb2198c-e0d9-4e4b-af9e-d4e72542bace (Detoro) · authority: in-loop

## Problem (Tiësto's observed follow-up on stage-commit-missing-boundary-path,
## note be9e188a, 2026-07-10)

stage commit/snap now skip declared-but-absent boundary paths (merged
d413755), but `stage restore`'s final step still runs
`git restore --worktree --source <snap> -- <full boundary>` with the VERBATIM
boundary: a path absent from the snapshot tree makes git restore error —
same defect family as the commit-side bug, different mechanism.

## Fix

In `src-tauri/src/bin/conclave-cli.rs` (stage restore path): before invoking
git restore, partition the boundary against the SNAPSHOT tree (ls-tree of the
snapshot via the private-index helpers, or `git cat-file -e <snap>:<path>` per
entry) — restore only paths present in the snapshot; print the same
`note — skipped N …` stderr shape the commit side now uses (keep the wording
consistent so agents recognize it). Empty kept-set → no git restore call and
a clear message, never an error.

## Tests

Mirror the merged commit-side tests: restore with one absent path succeeds and
names it; all-absent restore is a no-op with a message; a path present in the
snapshot but deleted from the worktree is RESTORED (never mistaken for
absent). Fixtures uuid-suffixed (flake class deb52bc9).

## Boundary

- src-tauri/src/bin/conclave-cli.rs

## Gates

cd src-tauri && cargo fmt --check · cargo test · cargo clippy --all-targets
--all-features -- -D warnings · git diff --check

## Sequencing

conclave-cli.rs queue: rtk-hook-denylist (Guetta, active) →
inapp-browser-agent-tools-v3 (Tiësto, big feature) → THIS (minor defect,
rare trigger, documented workaround exists). Do not claim before both merge.
