# rtk hook: denylist observation-critical rewrites

owner: 4fb2198c-e0d9-4e4b-af9e-d4e72542bace (Detoro) · authority: in-loop

## Decision record

Guetta's diagnosis (task rtk-output-distortion, FINDINGS note 43c842dd, gate
log 20260710T040946495Z) proved the distortion is UPSTREAM rtk 0.42.4, not our
hook wiring. Reproduced classes: [A] `rtk grep` on a single file with rg absent
from PATH renders "N matches in 0 files" and drops every match line (BSD-grep
fallback output has no file: prefix, rtk's group-by-file parser attributes 0
files); [B] `rg -l/-t` flags collide with rtk grep's own flags → deterministic
false misses; [C] `rtk ls` is a git-aware lister (hides gitignored entries) —
surprising tree shapes; [D] whitespace-stripped, 80-char-ellipsized, `[+K
more]`-capped rendering. Ruling: rtk stays ON for summary-style rewrites (git
status/diff/log, cargo, pnpm, tsc...) but must NEVER rewrite the
observation-critical filesystem/search commands. Upstream bug filing is a
separate item pending human approval (external publishing).

## Fix (Guetta's direction, accepted)

In `src-tauri/src/bin/conclave-cli.rs` only:

1. In `rtk_hook_response` (~:2889; hook body `run_rtk_hook` ~:2841): after
   obtaining the REWRITTEN command from `rtk rewrite`, if it invokes any of
   `rtk grep`, `rtk ls`, `rtk tree`, `rtk find`, `rtk read` at ANY command
   position (start of string, after `&&`, `||`, `;`, `|`, or `$(`), return
   `None` so the ORIGINAL command runs unfiltered.
2. Match on the rewritten side, not the source side — `rtk rewrite` is the
   single source of truth for the mapping, so every source spelling (ls, rg,
   grep, fd, cat...) is caught automatically.
3. Hook stays fail-open on every existing path; no DB or settings change; the
   per-agent `rtk_enabled` toggle keeps working unchanged.

## Tests (beside the existing rtk_hook_response tests ~:4997)

- Denylisted: a rewrite producing `rtk grep ...` → response is None (raw
  command passes through). Cover at least one compound position
  (`cd X && rtk ls`).
- Non-denylisted: `git status` still rewrites to `rtk git status` exactly as
  today.
- `rtk read`-style rewrite inside a pipeline is also caught.

## Boundary

- src-tauri/src/bin/conclave-cli.rs

## Gates

cd src-tauri && cargo fmt --check · cargo test · cargo clippy --all-targets
--all-features -- -D warnings · git diff --check

## Risk ledger

- Do NOT parse the source command yourself beyond position-splitting the
  rewritten string — the whole point is that rtk's own mapping decides.
- The hook's existing permission-aware exit-code contract (memory 55ba20de)
  must not change: denylist means "return None", never a new exit code.
- Temp fixtures uuid-suffixed (flake class deb52bc9).
