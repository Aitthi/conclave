# Plan: stage-status-head — HEAD-based `stage status` + restore guard + tool-map catch-up

Date: 2026-07-05 · Owner: Detoro bfb737ff · authority: in-loop
Task: `stage-status-head` · Implementer: Dabin eecebcbe · Reviewer: Mellow (LAND, blocking)
Status: Lead-filed follow-up under in-loop authority (display-only CLI change;
Mellow's LAND note on stage-v1 pre-endorsed the shape; human informed).

## Why

stage-v1's known limitation F1 (plan `2026-07-05-plan-stage-v1.md`, risk
ledger): `stage commit` deliberately never touches the shared index, so
`stage status` — spec'd as porcelain — shows just-committed files as
modified until the index refreshes; an agent reads that as "commit failed".
`stage diff` is already immune (HEAD-based). Fix status the same way.
Two riders while the files are open: M1 (restore accepts any commit-ish)
and the deferred tool-map doc rows.

## Task 1 — HEAD-based `stage status` (conclave-cli.rs)

Replace the porcelain/index path in `stage status` with (Mellow's shape,
endorsed on the stage-v1 ledger):
- tracked changes: `git diff --name-status HEAD -- <boundary paths>`
- untracked: `git ls-files --others --exclude-standard -- <boundary paths>`
- **BOTH commands run under a private `GIT_INDEX_FILE` seeded with
  `git read-tree HEAD`** (reuse `git_with_index`) — amended at LAND review:
  both consult the index for tracked-set membership, so against the stale
  shared index a newly stage-committed file reads as `D` + `??`
  simultaneously (Mellow's B1, empirically verified along with the fix).
  Without this line the commands are only index-independent for files that
  were already tracked.
Keep the IN-BOUNDARY / OUT-OF-BOUNDARY partition output format unchanged
(out-of-boundary section = same two commands with no pathspec, minus the
in-boundary set — implementer's judgment on the cleanest set arithmetic,
output shape is the contract). The shared index must remain unread on this
path (F1's whole point): no `git status` call remains in `run_stage`.

## Task 2 — M1 restore guard (conclave-cli.rs)

`stage restore <ws> <slug> <snapSha>` refuses a sha that is not reachable
from `refs/conclave/stage/<slug>` (`git merge-base --is-ancestor <sha>
<ref>`; equal counts as reachable). Error names the ref and suggests
`stage log`. Pre-restore auto-snap behavior unchanged.

## Task 3 — tool-map catch-up rows (tool-map/SKILL.md)

The two deferred Lows now due (this is "the NEXT tool-map touch" both
records point at):
1. Gate row: append "commit first, then gate — the gate pins `git rev-parse
   HEAD` at run time" (mirrors the implementer-skill sentence landed in
   skill-prose-pass).
2. Watch/note rows: one line — notes prefixed `READY`/`BLOCKED`/`ESCALATION`
   wake watchers; unmarked notes and passing gates are ledger-only (behavior
   ships in lane watch-filter, same rebuild).

## Tests (extend the stage tests in conclave-cli.rs mod tests)

1. post-commit status clean: stage commit, then stage status reports the
   committed boundary file as CLEAN (the F1 repro, inverted — this test is
   the acceptance criterion).
2. untracked boundary file appears in status; tracked modification appears
   with its letter status.
3. shared-index independence: corrupt/backdate the shared index (or leave a
   staged stranger entry in it) — stage status output unaffected.
4. restore rejects a sha outside the snap ref's history (use a normal branch
   commit); accepts one from `stage log`.
5. Existing stage tests stay green unchanged (the output-shape contract).

## Boundary

`src-tauri/src/bin/conclave-cli.rs`, `src-tauri/skills/tool-map/SKILL.md`.
Nothing else.

## Gates (commit first, then gate; from src-tauri)

- `cargo test` (full) · `cargo clippy --all-targets -- -D warnings`.
- Mellow LAND (blocking): F1 acceptance test present and honest, no
  index-reading call left in the stage region, ancestor guard edge cases
  (ref missing → clear error), tool-map rows match shipped behavior.

## Risk ledger

- Reaches live agents after next rebuild+install, same batch as stage-v1.
- `ls-files --others` in a big untracked-heavy tree is fine under a boundary
  pathspec; do not run it bare except for the out-of-boundary section where
  `--exclude-standard` keeps it sane.
- The watch-filter tool-map row documents behavior landing in a PARALLEL
  lane (watch-filter, Tiësto) — if that lane's wake list changes at review,
  this row must follow; integrator (lead) checks consistency at merge.
- M3 cwd-threading inconsistency from stage-v1 is explicitly NOT in scope —
  cosmetic, unbounded refactor risk in a 2000-line bin file.
