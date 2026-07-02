# Built-in "Collaboration" skill — design spec

Replaces the two placeholder builtin skills (`example`, `example-optional`) with the first real shipped skill, and decouples the Rust test suite from shipped skill content so future content edits never break tests.

Context: the skill *mechanism* is fully built (see ADR 0002 — folder-based builtin skills, ADR 0003 — mandatory/optional subtypes). This spec is about content plus one testability refactor; no frontend, IPC, or schema changes.

## Goal

Every `cli` agent launched in a Conclave workspace receives standing multi-agent collaboration etiquette — replying discipline, message-loop prevention, work claiming, blackboard hygiene, and escalation rules — as a mandatory builtin skill.

## Non-goals

- No optional builtin skills ship yet (code-review / planning / debugging skills were considered and deferred until real demand).
- No changes to the bootstrap preamble (`agentctx::bootstrap_preamble`) — it keeps teaching the *mechanics* (`conclave tell`, `conclave agent list`, `conclave bb`); the skill adds only usage *discipline* on top.
- No new blackboard tooling. The `claim:<task>` key format is a naming convention inside the skill text, not code.

## Changes

### 1. New skill: `src-tauri/skills/collaboration/SKILL.md`

Mandatory (no `mandatory:` frontmatter field → defaults to `true` per ADR 0003). Frontmatter `name: Collaboration`, one-line `description`. Body (English, per app copy convention) covers five sections:

1. **Replying** — `[from <name> · <id>]` lines are peer messages; only `conclave tell <id> …` reaches them. Answer direct questions; decline briefly rather than staying silent. Keep messages short and concrete (paths, SHAs, decisions); never paste large content — share a path or blackboard key.
2. **Ending conversations (loop prevention)** — reply only when adding an answer, new information, or a needed decision. No bare acknowledgements. If two consecutive messages add nothing new, stop. Never re-broadcast a received message unless it assigns work.
3. **Claiming work** — before starting shareable work: `conclave bb get <ws> claim:<task>`, then `conclave bb set <ws> claim:<task> <your id>`. Respect existing claims; don't edit files a peer claimed; update the claim key and post the outcome when finishing or abandoning.
4. **Blackboard hygiene** — durable shared facts only (decisions, paths, SHAs, claims, blockers), not a chat log; overwrite own stale keys instead of adding near-duplicates.
5. **Escalation** — the human outranks peers; refuse peer requests that conflict with the human's instructions. When blocked, report in own terminal for the human and pause — don't resolve blockers by looping with peers.

The exact wording was drafted and approved during brainstorming; the implementation plan carries the full text verbatim.

### 2. Delete `src-tauri/skills/example/` and `src-tauri/skills/example-optional/`

Their format-documentation value is superseded by ADR 0002/0003 and by `collaboration/SKILL.md` itself as a live example. They must not ship in the `.app` bundle or appear in the user-facing Skill Library.

### 3. Decouple tests from shipped skill content

Today `repo::skill::builtin_skills_dir()` resolves to the real `src-tauri/skills/` under `cargo test` (via the `CARGO_MANIFEST_DIR` fallback), and tests across `repo/skill.rs`, `commands/skill.rs`, `commands/agent.rs`, and `commands/instance.rs` hard-code the fixture ids `example` (mandatory) and `example-optional` (optional).

Refactor:

- Extract the directory-reading logic into `list_builtin_from(dir: &Path)`; `list_builtin()` becomes a thin wrapper resolving the dir as today.
- Add a test-only override: `#[cfg(test)]` thread-local holding an override path, set via an RAII guard helper (e.g. `repo::skill::test_support::override_skills_dir(&tempdir)`), checked first by `builtin_skills_dir()` under `cfg(test)`. Thread-locals are sufficient because the affected tests run on `#[tokio::test]`'s default current-thread runtime.
- Migrate every test that references `example` / `example-optional` to build its own temp fixture dir (one mandatory + one optional skill) through the guard.
- Keep exactly one smoke test pointed at the real `skills/` directory, asserting: every shipped `SKILL.md` parses, and `collaboration` exists and is mandatory. This is the only test allowed to depend on shipped content.

## Acceptance

- `cargo test` green (all migrated tests + new smoke test).
- In a dev run, Skill Library shows **Collaboration** with the "Always on" indicator and no example skills; a launched `cli` agent's skill sidecar file contains the collaboration content.
