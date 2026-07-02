---
status: accepted
supersedes: partially supersedes ADR 0001's "Builtin skills are DB rows" decision
---

# Builtin skills are read from a bundled folder, not DB rows

ADR 0001 shipped builtin skills as DB rows (`skill.kind = 'builtin'`), seeded via migration. In practice this meant every builtin skill needed a migration + code change to add, and had no natural authoring format — a real content author would be hand-writing SQL `INSERT` strings for markdown prompt text.

We're replacing that with a filesystem convention mirroring Claude Code's own Skills: a `skills/` directory containing one subdirectory per skill, each holding a `SKILL.md` file with two-field YAML-ish frontmatter (`name`, `description`) followed by the markdown body as `content`. This directory is bundled into the app at build time via Tauri's `bundle.resources`, so builtin skills ship inside the `.app` like any other static asset — authored as plain files in the repo, reviewed in PRs like code, no migration required to add or edit one.

Key decisions:

- **The DB `skill` table becomes custom-only.** `kind` is dropped from the schema (migration `0005_drop_skill_kind.sql`) — there is no longer a `kind='builtin'` DB path at all. Every remaining row in `skill` is, structurally, a user-authored custom skill.
- **Builtin skill ids are the folder name, not a UUID.** They're static, versioned-in-git content, not created through any runtime `create()` call — the folder name (`skills/code-reviewer/SKILL.md` → id `code-reviewer`) is a natural, stable, human-readable identifier, unlike custom skills' generated UUIDs.
- **Resource-directory resolution avoids the Tauri `AppHandle`/`Manager` API, mirroring `ensure_conclave_shim`'s existing pattern.** `repo::skill`'s builtin-reading logic needs to work from a plain `&SqlitePool`-free, `AppHandle`-free context (it's called from ordinary async command handlers, and must stay testable via `AppState::for_tests()`, which never sets an `AppHandle`). Rather than threading a `Manager`/`AppHandle` through the repo layer, we resolve the skills directory the same way `agentctx::ensure_conclave_shim` already resolves the `conclave-cli` binary: relative to `std::env::current_exe()` in a packaged `.app` (`Contents/MacOS/conclave` → `Contents/Resources/skills`), falling back to `CARGO_MANIFEST_DIR/skills` (a compile-time constant pointing at the source tree) when that bundled path doesn't exist — which is always true in a `cargo run`/`tauri dev` build, since dev builds have no `.app` bundle structure at all.
- **No new YAML dependency.** The frontmatter format needed here is two flat string fields (`name:`, `description:`), not general YAML — a hand-rolled `---`-delimited parser is added instead of pulling in `serde_yaml`/`gray_matter`, consistent with this codebase's existing preference for small hand-rolled parsers over heavy dependencies for narrow formats (e.g. `agentctx::sanitize_field`).
- **A skill folder with no `SKILL.md`, or one with unparsable frontmatter, is silently skipped, not a hard error.** A single malformed builtin skill file must not take down every `cli` agent's launch (which depends on `content_for_agent` succeeding) — the reader logs (dev-only) and continues past a bad entry rather than propagating a parse error up through `content_for_agent`.
- **No change to any consumer of the `Skill` type.** `SkillLibrary.tsx`, `SkillEditor.tsx`, `Builder.tsx`, and `Roster.tsx` all consume `Skill`/`AgentDefinition.skillIds`/`WorkspaceAgent.launchedSkillIds` purely by `kind` and shape, with no dependency on WHERE a builtin row's data physically comes from — this is a pure backend swap, and the frontend needs zero changes.
