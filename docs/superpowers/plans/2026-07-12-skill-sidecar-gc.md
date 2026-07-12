# Skill sidecar GC — stop `Conclave/skills/` growing forever

owner: 4fb2198c-e0d9-4e4b-af9e-d4e72542bace (Detoro) · authority: in-loop

## Problem

`agentctx::write_skill_sidecar` writes `<data_dir>/Conclave/skills/<workspace_agent_id>.md`
on every instance launch, but nothing ever deletes a file. `workspace_agent`
rows are removed on agent delete / instance remove / workspace delete (rows
cascade), yet the sidecar file survives. Measured on the human's machine
2026-07-12: 1,242 files, only 46 matching a live `workspace_agent` row —
1,196 orphans (9.0 MB). Every Conclave user accumulates the same garbage.

## Decisions (settled, do not re-open)

- **D1 — startup sweep is the load-bearing fix.** One-shot async task spawned
  from `lib.rs` `setup()` (same idiom as `task_timer::run`). It deletes every
  file in the skills dir whose stem is a UUID with **no** matching
  `workspace_agent.id`. This retroactively cleans every existing user's machine
  on first launch after update. Rejected alternative: periodic timer — files
  only accumulate across launches, so once per boot is enough.
- **D2 — also delete on removal.** Best-effort `remove_skill_sidecar` call at
  the three command-layer funnels that call `repo::workspace_agent::remove`
  (see file list). Keeps the dir honest between launches. FS deletion stays in
  the command layer / agentctx, NOT in `repo/` (repo does no filesystem IO).
- **D3 — safety guards in the sweep.** (a) only files directly in the skills
  dir, never recursive; (b) only `<uuid>.md` names — anything else is skipped;
  (c) skip files with mtime younger than 60 s (a row inserted mid-sweep may
  not have been in the id set read at sweep start); (d) read the id set AFTER
  listing files; (e) every delete is best-effort (`let _ =` with a log line),
  a locked/missing file must never abort the sweep or the app.
- **D4 — `cli-output/` is OUT of scope.** It also accumulates (1,326 files /
  21 MB) but `task gate` events reference `logPath` inside it; age-based GC
  there deletes gate evidence and needs a separate human ruling.

## Changes (boundary = exactly these 6 files)

1. `src-tauri/src/engine/repo/workspace_agent.rs` — add
   `pub async fn list_all_ids(pool: &SqlitePool) -> sqlx::Result<Vec<String>>`
   (`SELECT id FROM workspace_agent`, no workspace filter). Unit test beside
   the existing repo tests.
2. `src-tauri/src/engine/agentctx.rs` — three additions next to
   `write_skill_sidecar` (line ~387):
   - `pub fn skills_dir() -> Option<PathBuf>` — extract the dir computation
     `write_skill_sidecar` already does, reuse it there (single source of
     truth for the path).
   - `pub fn remove_skill_sidecar(instance_id: &str)` — best-effort
     `fs::remove_file`, ignores NotFound, logs other errors, returns nothing.
     Unix + non-unix (no-op) variants mirroring `write_skill_sidecar`.
   - `pub fn sweep_orphan_skill_sidecars(live_ids: &HashSet<String>) -> usize`
     — pure FS part (list, filter per D3, delete), returns count deleted;
     takes the id set as a param so tests need no DB. Unit tests in the
     existing `#[cfg(test)]` block using a temp dir — note the fn must take
     the dir as a param (`sweep_in_dir(dir, live_ids)`) with a thin
     `data_dir`-resolving wrapper, or tests would sweep the real dir.
3. `src-tauri/src/lib.rs` — in `setup()`, spawn the one-shot sweep after the
   other background tasks: fetch `list_all_ids`, build the set, call the
   sweep, `log::info!` the deleted count.
4. `src-tauri/src/engine/commands/agent.rs` (~line 396),
   `src-tauri/src/engine/commands/instance.rs` (~line 1032),
   `src-tauri/src/engine/commands/workspace.rs` (~line 169) — call
   `agentctx::remove_skill_sidecar(&inst.id)` right after each successful
   `repo::workspace_agent::remove`.

## Global constraints

- Prose/doc references: before renaming or extracting anything
  `write_skill_sidecar`-related, grep rustdoc comments for the old wording
  (`grep -rn "skill sidecar" src-tauri/src`) — stale doc refs outside the
  boundary were the defect class of challenge a649e89c.
- No new dependencies. No UI change → no uishot gate.
- Shared checkout: commit via `conclave stage commit` (boundary-scoped).

## Gates (record each via `conclave task gate`)

1. `cargo test --manifest-path src-tauri/Cargo.toml agentctx` — new sweep +
   remove tests green.
2. `cargo test --manifest-path src-tauri/Cargo.toml workspace_agent` —
   `list_all_ids` test green.
3. `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`.

## Risk ledger

- `dirs::data_dir()` differs per platform; sweep must use the SAME path
  construction as `write_skill_sidecar` (hence the shared `skills_dir()`).
- The engine change is live only after the app is rebuilt + relaunched —
  note it in the READY note; the human triggers relaunch.
- 46 live sidecar files exist on this machine post-manual-clean; after the
  sweep ships, count must stay 46 (± roster changes), never regrow unbounded.
