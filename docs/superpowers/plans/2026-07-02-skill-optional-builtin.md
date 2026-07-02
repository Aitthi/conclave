# Optional System Skills Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a system (builtin) skill declare itself `mandatory: false` in its `SKILL.md`, making it a per-`AgentDefinition` pickable item (attach/detach like a custom skill) instead of always-on — while staying read-only content, per ADR 0003.

**Architecture:** Extend the existing folder-based builtin skill reader (`repo/skill.rs`, ADR 0002) with a third frontmatter field, `mandatory` (default `true`). Add one new `agent_definition.selected_builtin_skill_ids` JSON-array column (mirrors the existing `session.launched_skill_ids` pattern) to persist which optional builtins a given agent definition has opted into — NOT the `agent_skill` join table, whose `skill_id` column is FK-enforced against the DB-only `skill` table and would reject a builtin id outright. Add one shared function, `effective_builtin_skills`, that both the launch path (`content_for_agent`) and the Roster-staleness annotation path (`commands::agent::list`) call, so the two can never drift out of sync (this is exactly the class of bug the v1 final review caught and fixed by hand). The frontend reuses its existing `skillIds` attach/detach wiring unchanged in shape — a second checkbox group for optional system skills feeds the same array Custom skills already feed.

**Tech Stack:** Rust (sqlx + chain-builder), React 19 + TypeScript, existing Conclave conventions throughout.

## Global Constraints

- The app runs with SQLite `foreign_keys(true)` enforced (`db.rs`) — `agent_skill.skill_id` is `REFERENCES skill(id) ON DELETE CASCADE`; a builtin skill id must NEVER be inserted into `agent_skill` (it isn't a DB row and the insert would fail the FK). Optional-builtin selection uses the new `agent_definition.selected_builtin_skill_ids` column instead.
- `SKILL.md` frontmatter parsing is hand-rolled (`parse_skill_md`, no YAML crate) — extend it, don't replace it.
- `mandatory` is optional in frontmatter; its absence means `true` (today's behavior, unchanged) — this must hold for the already-shipped `src-tauri/skills/example/SKILL.md`, which has no `mandatory` line and must keep resolving to `mandatory: true` after this plan lands.
- `content_for_agent` and `commands::agent::list`'s `skillIds` annotation MUST derive the effective builtin set from the exact same function (`repo::skill::effective_builtin_skills`) — this is a hard requirement, not a style preference; a v1 final review caught a real bug from these two paths drifting.
- No new IPC command and no new top-level request/response field: the frontend continues to send the same `skillIds` array on `agentDef.save` it already sends for custom skills; the backend is responsible for splitting an id into "custom" vs "optional builtin" vs "unknown/mandatory (drop)".
- Migration numbering continues from `0005_drop_skill_kind.sql` → next is `0006`.
- Follow existing patterns exactly: `AgentDefinitionInput`/`AgentDefRow`/`AgentDefListItem` all gain the new field the same way `custom_env`/`secret_env_keys` were added (see `repo/agent_definition.rs`); DB migration gating follows the `if version < N { … }` block pattern in `db.rs`'s `migrate()`.

---

### Task 1: Migration 0006 — `agent_definition.selected_builtin_skill_ids`

**Files:**
- Create: `src-tauri/src/engine/migrations/0006_selected_builtin_skills.sql`
- Modify: `src-tauri/src/engine/db.rs` (add `if version < 6 { … }` gate; extend the migration test)

**Interfaces:**
- Produces: `agent_definition.selected_builtin_skill_ids TEXT` (nullable), a JSON array of optional-builtin skill ids selected for that agent definition. `PRAGMA user_version` reaches `6`.

- [ ] **Step 1: Write the migration SQL**

```sql
-- src-tauri/src/engine/migrations/0006_selected_builtin_skills.sql
-- Per-AgentDefinition selection of OPTIONAL builtin skills (`mandatory: false`
-- in their SKILL.md frontmatter — see ADR 0003). Mandatory builtins are
-- always included and need no persisted selection. Cannot reuse `agent_skill`
-- (its skill_id column is FK-enforced against the `skill` table, and builtin
-- ids are never DB rows — see ADR 0002) so this mirrors the existing
-- `session.launched_skill_ids` JSON-array-column pattern instead.
ALTER TABLE agent_definition ADD COLUMN selected_builtin_skill_ids TEXT;
```

- [ ] **Step 2: Wire the migration into `db.rs`'s `migrate()`**

In `src-tauri/src/engine/db.rs`, immediately after the existing `if version < 5 { … }` block and before `tx.commit().await?;`, add:

```rust
    if version < 6 {
        sqlx::raw_sql(include_str!("migrations/0006_selected_builtin_skills.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 6;")
            .execute(&mut *tx)
            .await?;
    }
```

- [ ] **Step 3: Update the two existing `PRAGMA user_version` assertions**

Two tests in `db.rs` assert the final `user_version` after a full `connect_in_memory()` migration and must both move from `5` to `6`:

In `migrate_is_idempotent`:

```rust
        assert_eq!(version, 5, "user_version should be 5");
```

becomes:

```rust
        assert_eq!(version, 6, "user_version should be 6");
```

In `migrate_adds_skill_system_columns`:

```rust
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .expect("pragma read failed");
        assert_eq!(version, 5);
    }
```

becomes:

```rust
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .expect("pragma read failed");
        assert_eq!(version, 6);
    }
```

Also update `migrate_is_idempotent`'s doc comment (`/// Running migrate twice must not error and must leave user_version == 5.`) to say `== 6`. The `assert_eq!(count, 19, ...)` table-count assertion in the same test is UNCHANGED — this migration adds a column, not a table.

- [ ] **Step 4: Add a new test for the new column**

Add to `db.rs`'s `#[cfg(test)] mod tests`:

```rust
    #[tokio::test]
    async fn migrate_adds_selected_builtin_skill_ids_column() {
        let pool = connect_in_memory().await;

        // Column exists and accepts a JSON array of ids.
        sqlx::query(
            "INSERT INTO agent_definition (id, name, type, harness_mode, selected_builtin_skill_ids) \
             VALUES ('a1', 'A', 'cli', 'own', '[\"opt-1\",\"opt-2\"]')",
        )
        .execute(&pool)
        .await
        .expect("insert with selected_builtin_skill_ids should succeed");

        let stored: String = sqlx::query_scalar(
            "SELECT selected_builtin_skill_ids FROM agent_definition WHERE id = 'a1'",
        )
        .fetch_one(&pool)
        .await
        .expect("select failed");
        assert_eq!(stored, "[\"opt-1\",\"opt-2\"]");

        // NULL (no selection) must also be a valid, common state.
        sqlx::query("INSERT INTO agent_definition (id, name, type, harness_mode) VALUES ('a2', 'B', 'cli', 'own')")
            .execute(&pool)
            .await
            .expect("insert without selected_builtin_skill_ids should succeed");

        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .expect("pragma failed");
        assert_eq!(version, 6);
    }
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::db -- --nocapture`
Expected: all `db.rs` tests pass, including the new one and the updated version-`6` assertion.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/engine/migrations/0006_selected_builtin_skills.sql src-tauri/src/engine/db.rs
git commit -m "feat(db): add agent_definition.selected_builtin_skill_ids (migration 0006)"
```

---

### Task 2: `repo::skill` — `mandatory` frontmatter field, `effective_builtin_skills`, second example fixture

**Files:**
- Modify: `src-tauri/src/engine/repo/skill.rs`
- Create: `src-tauire/skills/example-optional/SKILL.md` — **NOTE the exact path is `src-tauri/skills/example-optional/SKILL.md`** (typo-guard: `src-tauri`, not `src-tauire`)

**Interfaces:**
- Consumes: nothing new from other tasks.
- Produces:
  - `SkillRow` gains `pub mandatory: bool` (after `pub kind: String`, before `pub icon`).
  - `pub fn effective_builtin_skills(selected_optional_ids: &[String]) -> Vec<SkillRow>` — filters `list_builtin()` to `s.mandatory || selected_optional_ids.contains(&s.id)`, preserving `list_builtin()`'s existing id-ascending order.
  - `content_for_agent(pool, agent_def_id)` now reads the agent definition's `selected_builtin_skill_ids` (via `repo::agent_definition::get`, added in Task 3) and calls `effective_builtin_skills` instead of using the raw `list_builtin()` result directly.
  - A second checked-in fixture, `src-tauri/skills/example-optional/SKILL.md`, with `mandatory: false` — both a real shipped example of the optional format and the deterministic test fixture for this task's new tests (same "CARGO_MANIFEST_DIR fallback always resolves in `cargo test`" reasoning as the original `example` fixture — see `repo/skill.rs`'s existing `list_builtin_finds_the_checked_in_example_skill` test).

**IMPORTANT — this task depends on Task 3's `repo::agent_definition::get` returning a `selected_builtin_skill_ids: Option<String>` field.** If Task 3 has not landed yet when this task starts, implement `content_for_agent`'s call to `repo::agent_definition::get` assuming that field already exists on `AgentDefRow` (Task 3's interface, defined below) — do not invent a different lookup mechanism.

- [ ] **Step 1: Ship the second example fixture**

```markdown
---
name: Example Optional Skill
description: Demonstrates the OPTIONAL builtin skill format (mandatory: false) — safe to remove or replace.
mandatory: false
---

This is an example OPTIONAL builtin skill shipped with Conclave (see
docs/adr/0003-optional-system-skills.md). Unlike `example/SKILL.md`
(mandatory, auto-attached to every agent), a skill with `mandatory: false`
in its frontmatter is not attached anywhere by default — a user must pick
it per agent definition, in the Builder's Skills section, the same way a
custom skill is picked. Its content is still read-only.
```

Save as `src-tauri/skills/example-optional/SKILL.md`.

- [ ] **Step 2: Write the failing tests for `mandatory` parsing**

Add to `repo/skill.rs`'s `#[cfg(test)] mod tests`, near the existing `parse_skill_md_*` tests:

```rust
    #[test]
    fn parse_skill_md_mandatory_defaults_to_true_when_absent() {
        let raw = "---\nname: Bare\n---\n\nBody.\n";
        let mandatory = super::parse_skill_md(raw).expect("should parse").3;
        assert!(mandatory, "omitted `mandatory:` must default to true");
    }

    #[test]
    fn parse_skill_md_mandatory_false_is_respected() {
        let raw = "---\nname: Opt\nmandatory: false\n---\n\nBody.\n";
        let mandatory = super::parse_skill_md(raw).expect("should parse").3;
        assert!(!mandatory);
    }

    #[test]
    fn parse_skill_md_mandatory_true_is_respected() {
        let raw = "---\nname: Man\nmandatory: true\n---\n\nBody.\n";
        let mandatory = super::parse_skill_md(raw).expect("should parse").3;
        assert!(mandatory);
    }

    #[test]
    fn parse_skill_md_mandatory_is_case_insensitive() {
        let raw = "---\nname: Opt\nmandatory: FALSE\n---\n\nBody.\n";
        let mandatory = super::parse_skill_md(raw).expect("should parse").3;
        assert!(!mandatory);
    }

    #[test]
    fn parse_skill_md_mandatory_unrecognized_value_defaults_to_true() {
        // An author typo ("nope") must fail safe to the mandatory default,
        // not silently become optional.
        let raw = "---\nname: Weird\nmandatory: nope\n---\n\nBody.\n";
        let mandatory = super::parse_skill_md(raw).expect("should parse").3;
        assert!(mandatory);
    }
```

These will fail to compile because `parse_skill_md`'s return type doesn't yet have a fourth tuple element. That compile failure IS the "run it to see it fail" step here — proceed to Step 3.

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::repo::skill 2>&1 | tail -30`
Expected: compile error, `parse_skill_md_*` tests reference `.3` on a 3-tuple.

- [ ] **Step 3: Extend `parse_skill_md` to return `(String, Option<String>, String, bool)`**

Replace the existing `parse_skill_md` function body in `src-tauri/src/engine/repo/skill.rs`:

```rust
/// Parse a `SKILL.md`'s `---`-delimited frontmatter (flat `key: value` lines
/// — `name`/`description`/`mandatory` recognized) and body. Hand-rolled
/// rather than pulling in a YAML crate: the format is a handful of flat
/// string/bool fields, not general YAML (see ADR 0002). Returns `None` (skip
/// this skill) if the file doesn't start with a frontmatter block or `name`
/// is missing/blank. The fourth element is `mandatory`, defaulting to `true`
/// when the field is absent OR its value isn't recognized as `true`/`false`
/// (case-insensitive) — an author typo must fail safe to mandatory, never
/// silently to optional (see ADR 0003).
fn parse_skill_md(raw: &str) -> Option<(String, Option<String>, String, bool)> {
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let rest = raw.strip_prefix("---")?;
    let rest = rest
        .strip_prefix("\r\n")
        .or_else(|| rest.strip_prefix('\n'))?;
    let end = rest.find("\n---")?;
    let frontmatter = &rest[..end];
    let after_closing = &rest[end + 4..];
    // Skip the closing `---` line's own terminator plus any further blank
    // separator line(s) before the body content starts.
    let body = after_closing.trim_start_matches(['\r', '\n']);

    let mut name = None;
    let mut description = None;
    let mut mandatory = true;
    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("name:") {
            name = Some(v.trim().to_owned());
        } else if let Some(v) = line.strip_prefix("description:") {
            description = Some(v.trim().to_owned());
        } else if let Some(v) = line.strip_prefix("mandatory:") {
            mandatory = !v.trim().eq_ignore_ascii_case("false");
        }
    }
    let name = name.filter(|s| !s.is_empty())?;
    Some((
        name,
        description.filter(|s| !s.is_empty()),
        body.trim_end().to_owned(),
        mandatory,
    ))
}
```

Note the deliberate `!v.trim().eq_ignore_ascii_case("false")` framing: only an explicit, recognized `false` flips it — `true`, an empty value, and any unrecognized string all resolve to `mandatory = true`, matching `parse_skill_md_mandatory_unrecognized_value_defaults_to_true`.

- [ ] **Step 4: Update the two existing `parse_skill_md_*` tests that destructure a 3-tuple**

`parse_skill_md_extracts_frontmatter_and_body` and `parse_skill_md_description_optional` currently do:

```rust
let (name, description, content) = super::parse_skill_md(raw).expect("should parse");
```

Change both to:

```rust
let (name, description, content, mandatory) = super::parse_skill_md(raw).expect("should parse");
```

and add `assert!(mandatory, "no mandatory: line present, must default to true");` at the end of each.

- [ ] **Step 5: Update `read_builtin_skills_from` to thread `mandatory` through**

In `read_builtin_skills_from`, the line:

```rust
        let Some((name, description, content)) = parse_skill_md(&raw) else {
```

becomes:

```rust
        let Some((name, description, content, mandatory)) = parse_skill_md(&raw) else {
```

and the `SkillRow` construction:

```rust
        out.push(SkillRow {
            id,
            name,
            description,
            content,
            kind: "builtin".to_owned(),
            icon: None,
        });
```

becomes:

```rust
        out.push(SkillRow {
            id,
            name,
            description,
            content,
            kind: "builtin".to_owned(),
            mandatory,
            icon: None,
        });
```

- [ ] **Step 6: Add `mandatory: bool` to `SkillRow` and fix every other constructor**

In `SkillRow`'s definition, add the field right after `kind`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SkillRow {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub content: String,
    pub kind: String,
    /// `true` for every custom (DB) skill and every mandatory builtin;
    /// `false` only for a builtin skill whose SKILL.md sets
    /// `mandatory: false` (see ADR 0003). Custom skills are always
    /// attach/detach-able already, so this is always `true` for them —
    /// the field only matters when `kind == "builtin"`.
    pub mandatory: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}
```

Fix every other `SkillRow { .. }` literal in this file to add `mandatory: true` (custom skills are always mandatory in this sense — there's nothing to opt into, they're already opt-in via `agent_skill`):

- `impl From<CustomSkillDbRow> for SkillRow` — add `mandatory: true,`
- `create`'s returned `SkillRow` — add `mandatory: true,`

Run: `cargo build --manifest-path src-tauri/Cargo.toml 2>&1 | grep -A3 "missing field\|error\[" | head -60`
Expected: no more "missing field `mandatory`" errors. Fix any construction site the grep still finds before moving on.

- [ ] **Step 7: Write the failing test for `effective_builtin_skills`**

Add to `repo/skill.rs`'s test module:

```rust
    #[test]
    fn effective_builtin_skills_always_includes_mandatory() {
        let ids = super::effective_builtin_skills(&[])
            .into_iter()
            .map(|s| s.id)
            .collect::<Vec<_>>();
        assert!(
            ids.contains(&"example".to_string()),
            "mandatory builtin must be present even with zero selections"
        );
        assert!(
            !ids.contains(&"example-optional".to_string()),
            "optional builtin must be absent when not selected"
        );
    }

    #[test]
    fn effective_builtin_skills_includes_selected_optional() {
        let ids = super::effective_builtin_skills(&["example-optional".to_string()])
            .into_iter()
            .map(|s| s.id)
            .collect::<Vec<_>>();
        assert!(ids.contains(&"example".to_string()));
        assert!(ids.contains(&"example-optional".to_string()));
    }

    #[test]
    fn effective_builtin_skills_ignores_unknown_selected_id() {
        let ids = super::effective_builtin_skills(&["no-such-skill".to_string()])
            .into_iter()
            .map(|s| s.id)
            .collect::<Vec<_>>();
        assert!(!ids.contains(&"no-such-skill".to_string()));
    }

    #[test]
    fn list_builtin_reports_mandatory_flags_for_both_fixtures() {
        let skills = super::list_builtin();
        let mandatory = skills
            .iter()
            .find(|s| s.id == "example")
            .expect("example fixture must exist");
        assert!(mandatory.mandatory, "example/SKILL.md has no mandatory: line, must default true");

        let optional = skills
            .iter()
            .find(|s| s.id == "example-optional")
            .expect("example-optional fixture must exist");
        assert!(!optional.mandatory, "example-optional/SKILL.md sets mandatory: false");
    }
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::repo::skill 2>&1 | tail -30`
Expected: FAIL — `effective_builtin_skills` does not exist yet.

- [ ] **Step 8: Implement `effective_builtin_skills`**

Add right after `list_builtin`:

```rust
/// The builtin skills that actually apply to an agent definition: every
/// mandatory builtin (always), plus every optional builtin (`mandatory:
/// false`) whose id appears in `selected_optional_ids`. Preserves
/// `list_builtin()`'s id-ascending order. Both `content_for_agent` (what
/// gets injected at launch) and `commands::agent::list`'s `skillIds`
/// annotation (what the Roster compares against the launch snapshot) MUST
/// go through this one function — see ADR 0003's rationale (a v1 final
/// review caught a real bug from two call sites computing "the agent's
/// builtin ids" via separate, silently-drifting logic).
pub fn effective_builtin_skills(selected_optional_ids: &[String]) -> Vec<SkillRow> {
    list_builtin()
        .into_iter()
        .filter(|s| s.mandatory || selected_optional_ids.iter().any(|id| id == &s.id))
        .collect()
}
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::repo::skill 2>&1 | tail -30`
Expected: PASS.

- [ ] **Step 9: Update `content_for_agent` to use `effective_builtin_skills`**

Replace the current body:

```rust
pub async fn content_for_agent(
    pool: &SqlitePool,
    agent_def_id: &str,
) -> sqlx::Result<(String, Vec<String>)> {
    let builtins = list_builtin();
    let customs = attached_to_agent(pool, agent_def_id).await?;

    let mut ids = Vec::with_capacity(builtins.len() + customs.len());
    let mut sections = Vec::with_capacity(builtins.len() + customs.len());
    for s in builtins.iter().chain(customs.iter()) {
        ids.push(s.id.clone());
        sections.push(format!("## Skill: {}\n\n{}", s.name, s.content));
    }
    Ok((sections.join("\n\n"), ids))
}
```

with:

```rust
pub async fn content_for_agent(
    pool: &SqlitePool,
    agent_def_id: &str,
) -> sqlx::Result<(String, Vec<String>)> {
    let selected_optional: Vec<String> = super::agent_definition::get(pool, agent_def_id)
        .await?
        .and_then(|def| def.selected_builtin_skill_ids)
        .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
        .unwrap_or_default();
    let builtins = effective_builtin_skills(&selected_optional);
    let customs = attached_to_agent(pool, agent_def_id).await?;

    let mut ids = Vec::with_capacity(builtins.len() + customs.len());
    let mut sections = Vec::with_capacity(builtins.len() + customs.len());
    for s in builtins.iter().chain(customs.iter()) {
        ids.push(s.id.clone());
        sections.push(format!("## Skill: {}\n\n{}", s.name, s.content));
    }
    Ok((sections.join("\n\n"), ids))
}
```

This calls `super::agent_definition::get` — `repo::skill` and `repo::agent_definition` are sibling modules under `repo::`, so this is an ordinary same-crate call, not a circular dependency. This requires `AgentDefRow.selected_builtin_skill_ids` to exist (Task 3) — if Task 3 hasn't landed, this will not compile; that is expected and covered by this plan's task-dependency note above.

- [ ] **Step 10: Write the new `content_for_agent` test for opted-in optional skills**

Add to the test module, near the existing `content_for_agent_*` tests:

```rust
    #[tokio::test]
    async fn content_for_agent_includes_optional_builtin_only_when_selected() {
        let pool = connect_in_memory().await;
        let def_id = fixture_agent_def(&pool).await;

        // Nothing selected: optional builtin absent.
        let (_, ids) = super::content_for_agent(&pool, &def_id)
            .await
            .expect("query failed");
        assert!(!ids.contains(&"example-optional".to_string()));

        // Select it via the agent_definition column directly (Task 3's
        // setter isn't this task's concern — this test only proves
        // content_for_agent honors whatever is stored there).
        sqlx::query(
            "UPDATE agent_definition SET selected_builtin_skill_ids = ? WHERE id = ?",
        )
        .bind(serde_json::json!(["example-optional"]).to_string())
        .bind(&def_id)
        .execute(&pool)
        .await
        .expect("update failed");

        let (body, ids) = super::content_for_agent(&pool, &def_id)
            .await
            .expect("query failed");
        assert!(ids.contains(&"example-optional".to_string()));
        assert!(body.contains("## Skill: Example Optional Skill"));
    }
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::repo::skill 2>&1 | tail -40`
Expected: PASS (requires Task 3's `selected_builtin_skill_ids` column to exist — if Task 1 hasn't landed yet either, this fails at the raw SQL `UPDATE`; both Task 1 and Task 3 are prerequisites, noted above).

- [ ] **Step 11: Run the full test file once more and commit**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::repo::skill -- --nocapture 2>&1 | tail -50`
Expected: all tests in this file PASS, including every pre-existing test (nothing about their behavior changes — `example-optional` never appears in the effective set unless explicitly selected, so `content_for_agent_still_includes_builtin_when_nothing_custom_attached`'s `ids == vec!["example"]` assertion still holds).

```bash
git add src-tauri/src/engine/repo/skill.rs src-tauri/skills/example-optional/SKILL.md
git commit -m "feat(skill): optional builtin skills via mandatory: false frontmatter"
```

---

### Task 3: `repo::agent_definition` — `selected_builtin_skill_ids` field

**Files:**
- Modify: `src-tauri/src/engine/repo/agent_definition.rs`

**Interfaces:**
- Consumes: migration 0006's `agent_definition.selected_builtin_skill_ids` column (Task 1).
- Produces: `AgentDefRow.selected_builtin_skill_ids: Option<String>` (raw JSON-array text, same shape/convention as `custom_env`/`secret_env_keys`), same field on `AgentDefListItem`, same field on `AgentDefinitionInput`. `create`/`update`/`list_with_counts`/`get` all read/write it. This is the field Task 2's `content_for_agent` reads via `repo::agent_definition::get`, and the field `commands::agent` (Task 4) will write from the split `skillIds` request.

- [ ] **Step 1: Add the field to `AgentDefRow`**

In `AgentDefRow`, add right after `context_window` and before `created_at`:

```rust
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<String>,
    /// JSON array of OPTIONAL builtin skill ids (`mandatory: false`) this
    /// definition has opted into (see ADR 0003). `None`/absent means no
    /// optional builtins selected — distinct from an empty JSON array only
    /// in that both mean the same thing here (no meaningful distinction is
    /// drawn between "never set" and "explicitly cleared").
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_json_text"
    )]
    pub selected_builtin_skill_ids: Option<String>,
    pub created_at: String,
```

(This mirrors `custom_env`'s exact `serialize_json_text` usage a few fields above.)

- [ ] **Step 2: Add the same field to `AgentDefListItem`**

Same placement, same attributes, in `AgentDefListItem` (right after `context_window`, before `created_at`).

- [ ] **Step 3: Add the field to `COLS`**

```rust
const COLS: [&str; 19] = [
    "id",
    "name",
    "role",
    "type",
    "cli_kind",
    "color",
    "provider_id",
    "model",
    "harness_mode",
    "share_blackboard",
    "auto_submit_injected",
    "allowed_senders",
    "permission_mode",
    "custom_args",
    "custom_env",
    "secret_env_keys",
    "context_window",
    "selected_builtin_skill_ids",
    "created_at",
];
```

(Array length `18` → `19`.)

- [ ] **Step 4: Add the field to `list_with_counts`'s raw SQL**

Change:

```rust
        "SELECT d.id, d.name, d.role, d.type, d.cli_kind, d.color, d.provider_id, d.model, \
         d.harness_mode, d.share_blackboard, d.auto_submit_injected, d.allowed_senders, \
         d.permission_mode, d.custom_args, d.custom_env, d.secret_env_keys, d.context_window, \
         d.created_at, \
         (SELECT COUNT(*) FROM workspace_agent wa WHERE wa.agent_def_id = d.id) AS in_workspaces \
         FROM agent_definition d \
         ORDER BY d.created_at, d.id",
```

to:

```rust
        "SELECT d.id, d.name, d.role, d.type, d.cli_kind, d.color, d.provider_id, d.model, \
         d.harness_mode, d.share_blackboard, d.auto_submit_injected, d.allowed_senders, \
         d.permission_mode, d.custom_args, d.custom_env, d.secret_env_keys, d.context_window, \
         d.selected_builtin_skill_ids, \
         d.created_at, \
         (SELECT COUNT(*) FROM workspace_agent wa WHERE wa.agent_def_id = d.id) AS in_workspaces \
         FROM agent_definition d \
         ORDER BY d.created_at, d.id",
```

- [ ] **Step 5: Add the field to `AgentDefinitionInput`**

```rust
    /// "1m" / "200k" — selects the model's context window.
    pub context_window: Option<String>,
    /// JSON array of optional builtin skill ids selected for this agent
    /// definition (see ADR 0003). `None` clears the selection.
    pub selected_builtin_skill_ids: Option<String>,
}
```

(Added as the last field, right before the struct's closing brace — after `context_window`.)

- [ ] **Step 6: Wire it into `create`'s INSERT and returned struct**

In the `.insert([...])` call, add right after the `context_window` entry:

```rust
            (
                "context_window",
                input
                    .context_window
                    .clone()
                    .map(Bind::Text)
                    .unwrap_or(Bind::Null),
            ),
            (
                "selected_builtin_skill_ids",
                input
                    .selected_builtin_skill_ids
                    .clone()
                    .map(Bind::Text)
                    .unwrap_or(Bind::Null),
            ),
            ("created_at", Bind::Text(created_at.clone())),
```

In the `Ok(AgentDefRow { ... })` construction, add right after `context_window: input.context_window,`:

```rust
        context_window: input.context_window,
        selected_builtin_skill_ids: input.selected_builtin_skill_ids,
        created_at,
```

- [ ] **Step 7: Wire it into `update`'s UPDATE**

Add right after the `context_window` entry in `.update([...])`:

```rust
            (
                "context_window",
                input.context_window.map(Bind::Text).unwrap_or(Bind::Null),
            ),
            (
                "selected_builtin_skill_ids",
                input
                    .selected_builtin_skill_ids
                    .map(Bind::Text)
                    .unwrap_or(Bind::Null),
            ),
```

- [ ] **Step 8: Fix the four existing EXHAUSTIVE `AgentDefinitionInput { .. }` literals in this file's tests**

`AgentDefinitionInput` derives `Default`, but this file's own tests never use `..Default::default()` — every existing test constructs the struct exhaustively, field by field. Adding `selected_builtin_skill_ids` to the struct (Step 5) breaks all four of the following at compile time with "missing field" errors. Add `selected_builtin_skill_ids: None,` as the last field in each (right after `context_window: ...,`), in this exact file, `src-tauri/src/engine/repo/agent_definition.rs`:

1. The `minimal_input(name, agent_type, harness_mode)` test helper (in `mod tests`, right before `create_then_get_roundtrip`) — its literal ends `context_window: None,` then the closing brace; add `selected_builtin_skill_ids: None,` before that closing brace.
2. `create_then_get_roundtrip`'s first `create(&pool, &AgentDefinitionInput { ... })` call — its literal ends `context_window: Some("1m".into()),`; add `selected_builtin_skill_ids: None,` right after it.
3. `update_changes_fields`' (or whichever test contains it — the one calling `update(&pool, &row.id, &AgentDefinitionInput { name: "Nova-v2".into(), ... })`) literal ends `context_window: Some("200k".into()),`; add `selected_builtin_skill_ids: None,` right after it.
4. `camel_case_contract`'s `create(&pool, &AgentDefinitionInput { name: "Sol".into(), ... })` literal ends `context_window: Some("1m".into()),`; add `selected_builtin_skill_ids: None,` right after it.

Run: `cargo build --manifest-path src-tauri/Cargo.toml --tests 2>&1 | grep -A3 "missing field"`
Expected: no output (no remaining "missing field `selected_builtin_skill_ids`" errors anywhere in this file). If the grep finds another exhaustive literal this plan didn't list, fix it the same way — the four above are what exist as of this plan's writing, not a guaranteed-exhaustive list if the file has changed since.

- [ ] **Step 9: Write the failing test**

Add to `agent_definition.rs`'s `#[cfg(test)] mod tests`, using the file's existing `use super::*; use crate::engine::db::connect_in_memory;` imports already at the top of the module (no new imports needed):

```rust
    #[tokio::test]
    async fn create_and_update_roundtrip_selected_builtin_skill_ids() {
        let pool = connect_in_memory().await;
        let input = AgentDefinitionInput {
            name: "A".into(),
            agent_type: "cli".into(),
            harness_mode: "own".into(),
            selected_builtin_skill_ids: Some(
                serde_json::json!(["example-optional"]).to_string(),
            ),
            ..Default::default()
        };
        let row = super::create(&pool, &input)
            .await
            .expect("create failed");
        assert_eq!(
            row.selected_builtin_skill_ids.as_deref(),
            Some(r#"["example-optional"]"#)
        );

        let fetched = super::get(&pool, &row.id)
            .await
            .expect("get failed")
            .expect("row should exist");
        assert_eq!(fetched.selected_builtin_skill_ids, row.selected_builtin_skill_ids);

        let cleared_input = AgentDefinitionInput {
            selected_builtin_skill_ids: None,
            ..input
        };
        let updated = super::update(&pool, &row.id, &cleared_input)
            .await
            .expect("update failed")
            .expect("row should exist after update");
        assert!(
            updated.selected_builtin_skill_ids.is_none(),
            "update with None must clear the column"
        );
    }
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::repo::agent_definition 2>&1 | tail -30`
Expected: FAIL to compile until Task 1's migration exists (the column must exist in the DB schema) — if Task 1 hasn't landed, coordinate with the controller; do not invent a workaround migration.

- [ ] **Step 10: Verify it passes**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::repo::agent_definition -- --nocapture 2>&1 | tail -50`
Expected: PASS, including every pre-existing test in this file unchanged (the new column is nullable and additive).

- [ ] **Step 11: Commit**

```bash
git add src-tauri/src/engine/repo/agent_definition.rs
git commit -m "feat(agent_definition): add selected_builtin_skill_ids field"
```

---

### Task 4: `commands::agent` — split `skillIds` into custom vs optional-builtin, use `effective_builtin_skills`

**Files:**
- Modify: `src-tauri/src/engine/commands/agent.rs`

**Interfaces:**
- Consumes: `repo::skill::list_builtin`, `repo::skill::effective_builtin_skills` (Task 2), `AgentDefinitionInput.selected_builtin_skill_ids` (Task 3).
- Produces: `agentDef.save` now persists optional-builtin selections; `agentDef.list`'s `skillIds` annotation reflects the effective (mandatory + selected-optional) builtin set instead of always all builtins.

- [ ] **Step 1: Write the failing test for `save` splitting `skillIds`**

Add to `commands/agent.rs`'s `#[cfg(test)] mod tests`, near the existing `save_persists_and_replaces_skill_attachments` / `save_silently_drops_unknown_or_builtin_skill_ids` tests:

```rust
    #[tokio::test]
    async fn save_splits_skill_ids_into_custom_and_optional_builtin() {
        let state = AppState::for_tests().await;
        let custom = repo::skill::create(&state.db, "Custom", None, "c")
            .await
            .expect("create skill failed");

        let created = save(
            &state,
            serde_json::json!({
                "name": "A",
                "type": "cli",
                "skillIds": [custom.id, "example-optional", "example", "no-such-id"],
            }),
        )
        .await
        .expect("save failed");
        let id = created["id"].as_str().unwrap().to_owned();

        // Custom id -> agent_skill.
        let attached = repo::skill::attached_to_agent(&state.db, &id)
            .await
            .expect("query failed");
        assert_eq!(attached.len(), 1);
        assert_eq!(attached[0].id, custom.id);

        // Optional builtin id -> agent_definition.selected_builtin_skill_ids;
        // the mandatory "example" id and the unknown id are both dropped.
        let def = repo::agent_definition::get(&state.db, &id)
            .await
            .expect("get failed")
            .expect("row should exist");
        let selected: Vec<String> = def
            .selected_builtin_skill_ids
            .as_deref()
            .map(|t| serde_json::from_str(t).expect("valid json"))
            .unwrap_or_default();
        assert_eq!(selected, vec!["example-optional".to_string()]);
    }
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::commands::agent::tests::save_splits_skill_ids_into_custom_and_optional_builtin 2>&1 | tail -30`
Expected: FAIL — today's `save` only ever writes custom ids to `agent_skill`, so `selected` is empty/absent.

- [ ] **Step 2: Update `save` to compute and persist both filtered sets**

In `commands::agent::save`, the existing block:

```rust
    let input = AgentDefinitionInput {
        name: req.name,
        role: req.role,
        agent_type: req.agent_type,
        cli_kind: req.cli_kind,
        color: req.color,
        provider_id: req.provider_id,
        model: req.model,
        harness_mode: req.harness_mode.unwrap_or_else(|| "own".to_owned()),
        share_blackboard: req.share_blackboard,
        auto_submit_injected: req.auto_submit_injected,
        allowed_senders: req.allowed_senders,
        permission_mode,
        custom_args: nonblank(req.custom_args),
        custom_env,
        secret_env_keys,
        context_window: nonblank(req.context_window),
    };
```

Immediately BEFORE this block, add the split logic (it needs `req.skill_ids`, which the block after this doesn't consume, so ordering is safe — `req.skill_ids` is not read anywhere else before this point):

```rust
    // Split the incoming skillIds into three groups: a real custom DB skill
    // id -> agent_skill (unchanged path); an OPTIONAL builtin's id (see ADR
    // 0003) -> agent_definition.selected_builtin_skill_ids; anything else (a
    // mandatory builtin id, an unknown/stale id) -> silently dropped, same
    // "filter to known ids" precedent as the pre-existing custom-only path.
    let requested_skill_ids = req.skill_ids.unwrap_or_default();
    let optional_builtin_ids: std::collections::HashSet<String> = repo::skill::list_builtin()
        .into_iter()
        .filter(|s| !s.mandatory)
        .map(|s| s.id)
        .collect();
    let valid_custom_ids: std::collections::HashSet<String> = repo::skill::list(&state.db)
        .await?
        .into_iter()
        .map(|s| s.id)
        .collect();
    let filtered_custom_skill_ids: Vec<String> = requested_skill_ids
        .iter()
        .filter(|id| valid_custom_ids.contains(*id))
        .cloned()
        .collect();
    let filtered_optional_builtin_ids: Vec<String> = requested_skill_ids
        .iter()
        .filter(|id| optional_builtin_ids.contains(*id))
        .cloned()
        .collect();
    let selected_builtin_skill_ids = (!filtered_optional_builtin_ids.is_empty()).then(|| {
        serde_json::to_string(&filtered_optional_builtin_ids)
            .expect("serializing Vec<String> is infallible")
    });
```

Then add `selected_builtin_skill_ids,` as a new field in the `AgentDefinitionInput { ... }` literal (right after `context_window: nonblank(req.context_window),`):

```rust
        context_window: nonblank(req.context_window),
        selected_builtin_skill_ids,
    };
```

- [ ] **Step 3: Replace the old custom-only filtering block with the new precomputed variable**

Find and DELETE this now-duplicate block (it's superseded by Step 2's `filtered_custom_skill_ids`):

```rust
    // Persist custom skill attachments (replace semantics). Filter to known
    // CUSTOM skill ids so a stale/tampered request can't create an
    // `agent_skill` row for a builtin skill (which is never attached via that
    // table — see `repo::skill::content_for_agent`) or a nonexistent id.
    let valid_custom_ids: std::collections::HashSet<String> = repo::skill::list(&state.db)
        .await?
        .into_iter()
        .map(|s| s.id)
        .collect();
    let filtered_skill_ids: Vec<String> = req
        .skill_ids
        .unwrap_or_default()
        .into_iter()
        .filter(|id| valid_custom_ids.contains(&id))
        .collect();
    repo::skill::set_custom_attachments(&state.db, &row.id, &filtered_skill_ids).await?;
```

Replace it with just:

```rust
    // Persist custom skill attachments (replace semantics) — the filtered
    // set was already computed above, before `row` existed, so both this and
    // the optional-builtin selection (already stored via `input` in the
    // create/update call above) come from one shared split of `skillIds`.
    repo::skill::set_custom_attachments(&state.db, &row.id, &filtered_custom_skill_ids).await?;
```

- [ ] **Step 4: Run the new test**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::commands::agent -- --nocapture 2>&1 | tail -60`
Expected: PASS, including `save_splits_skill_ids_into_custom_and_optional_builtin` and every pre-existing test in this file (`save_persists_and_replaces_skill_attachments`, `save_silently_drops_unknown_or_builtin_skill_ids`, etc. — none of their behavior for CUSTOM ids changes).

- [ ] **Step 5: Write the failing test for `list`'s `skillIds` reflecting selection**

Add to the same test module:

```rust
    #[tokio::test]
    async fn list_skill_ids_include_selected_optional_builtin_but_exclude_unselected() {
        let state = AppState::for_tests().await;
        let created = save(
            &state,
            serde_json::json!({
                "name": "A",
                "type": "cli",
                "skillIds": ["example-optional"],
            }),
        )
        .await
        .expect("save failed");
        let id = created["id"].as_str().unwrap().to_owned();

        let listed = list(&state, Value::Null).await.expect("list failed");
        let item = listed
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == id)
            .expect("item present");
        let ids: Vec<String> = item["skillIds"]
            .as_array()
            .expect("skillIds must be present")
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        assert!(ids.contains(&"example".to_string()), "mandatory builtin always present");
        assert!(
            ids.contains(&"example-optional".to_string()),
            "selected optional builtin must be present"
        );
    }

    #[tokio::test]
    async fn list_skill_ids_exclude_optional_builtin_when_not_selected() {
        let state = AppState::for_tests().await;
        let created = save(&state, serde_json::json!({ "name": "A", "type": "cli" }))
            .await
            .expect("save failed");
        let id = created["id"].as_str().unwrap().to_owned();

        let listed = list(&state, Value::Null).await.expect("list failed");
        let item = listed
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == id)
            .expect("item present");
        let ids: Vec<String> = item["skillIds"]
            .as_array()
            .expect("skillIds must be present (mandatory builtin still applies)")
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        assert!(ids.contains(&"example".to_string()));
        assert!(!ids.contains(&"example-optional".to_string()));
    }
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::commands::agent -- --nocapture 2>&1 | tail -60`
Expected: FAIL — today's `list()` includes ALL builtins unconditionally (`repo::skill::list_builtin()` mapped directly), not the effective/selected set.

- [ ] **Step 6: Update `list` to use `effective_builtin_skills` per-item**

Replace the current body of `commands::agent::list`:

```rust
pub async fn list(state: &AppState, _payload: Value) -> Result<Value, AppError> {
    let items = repo::agent_definition::list_with_counts(&state.db).await?;
    // Same basis as the launch snapshot (`repo::skill::content_for_agent`):
    // builtin ids first (fixed order via `list_builtin`), then custom ids —
    // so `AgentDefinition.skillIds` and `WorkspaceAgent.launchedSkillIds`
    // are directly comparable (see Roster.tsx's `computeSkillsStale`).
    let builtin_ids: Vec<String> = repo::skill::list_builtin()
        .into_iter()
        .map(|s| s.id)
        .collect();
    let skill_map = repo::skill::custom_skill_ids_by_agent(&state.db).await?;

    let mut value = serde_json::to_value(&items).map_err(|e| AppError::Internal(e.to_string()))?;
    if let Some(arr) = value.as_array_mut() {
        for item in arr.iter_mut() {
            let id = item
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();
            let mut ids = builtin_ids.clone();
            if let Some(custom_ids) = skill_map.get(&id) {
                ids.extend(custom_ids.iter().cloned());
            }
            if !ids.is_empty() {
                item["skillIds"] = serde_json::json!(ids);
            }
        }
    }
    Ok(value)
}
```

with:

```rust
pub async fn list(state: &AppState, _payload: Value) -> Result<Value, AppError> {
    let items = repo::agent_definition::list_with_counts(&state.db).await?;
    // Same basis as the launch snapshot (`repo::skill::content_for_agent`):
    // the EFFECTIVE builtin set (mandatory + this item's selected optional
    // ones, via `effective_builtin_skills` — see ADR 0003) first, then
    // custom ids — so `AgentDefinition.skillIds` and
    // `WorkspaceAgent.launchedSkillIds` are directly comparable (see
    // Roster.tsx's `computeSkillsStale`).
    let skill_map = repo::skill::custom_skill_ids_by_agent(&state.db).await?;

    let mut value = serde_json::to_value(&items).map_err(|e| AppError::Internal(e.to_string()))?;
    if let Some(arr) = value.as_array_mut() {
        for item in arr.iter_mut() {
            let id = item
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();
            let selected_optional: Vec<String> = item
                .get("selectedBuiltinSkillIds")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            let mut ids: Vec<String> = repo::skill::effective_builtin_skills(&selected_optional)
                .into_iter()
                .map(|s| s.id)
                .collect();
            if let Some(custom_ids) = skill_map.get(&id) {
                ids.extend(custom_ids.iter().cloned());
            }
            if !ids.is_empty() {
                item["skillIds"] = serde_json::json!(ids);
            }
        }
    }
    Ok(value)
}
```

This reads `selectedBuiltinSkillIds` back out of the JSON `value` array produced a few lines above by `serde_json::to_value(&items)` — Task 3's `serialize_json_text` on `AgentDefListItem.selected_builtin_skill_ids` already converts the raw DB TEXT column into a proper camelCase JSON array (or omits the key when `None`) at that point, so no second DB query is needed.

- [ ] **Step 7: Run all the tests in this file**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::commands::agent -- --nocapture 2>&1 | tail -80`
Expected: PASS, including all new tests and every pre-existing test in this file (in particular `list_annotates_builtin_skill_ids_even_without_attachment`, which must still show the mandatory `example` id present with zero selections/attachments).

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/engine/commands/agent.rs
git commit -m "feat(agent): split skillIds into custom vs optional-builtin on save"
```

---

### Task 5: `commands::skill` — test coverage for `mandatory` in `skill.list`

**Files:**
- Modify: `src-tauri/src/engine/commands/skill.rs`

**Interfaces:**
- Consumes: `repo::skill::list_builtin`'s new `mandatory` field (Task 2) — `SkillRow` already serializes it automatically (no source change needed in this file; `list()`'s existing `serde_json::to_value(s)` call picks up the new struct field for free). This task is pure test coverage confirming the wire contract.

- [ ] **Step 1: Write the test**

Add to `commands/skill.rs`'s `#[cfg(test)] mod tests`, near `list_includes_builtin_and_custom`:

```rust
    #[tokio::test]
    async fn list_reports_mandatory_flag_for_both_builtin_fixtures() {
        let state = AppState::for_tests().await;
        let listed = list(&state, Value::Null).await.expect("list failed");
        let arr = listed.as_array().unwrap();

        let mandatory_item = arr
            .iter()
            .find(|s| s["id"] == "example")
            .expect("mandatory example fixture must be present");
        assert_eq!(mandatory_item["mandatory"], true);

        let optional_item = arr
            .iter()
            .find(|s| s["id"] == "example-optional")
            .expect("optional example fixture must be present");
        assert_eq!(optional_item["mandatory"], false);

        // Custom skills always report mandatory: true (nothing to opt into —
        // they're already opt-in via agent_skill).
        save(
            &state,
            serde_json::json!({ "name": "Custom", "content": "c" }),
        )
        .await
        .expect("create failed");
        let listed2 = list(&state, Value::Null).await.expect("list failed");
        let custom_item = listed2
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == "Custom")
            .expect("custom skill present");
        assert_eq!(custom_item["mandatory"], true);
    }
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::commands::skill -- --nocapture 2>&1 | tail -40`
Expected: PASS immediately (Task 2 already made `SkillRow` carry `mandatory`, and this file's `list()` serializes the whole struct) — this task is verification, not new production code. If it does NOT pass, Task 2 has not fully landed; do not add production code here to make it pass, flag the dependency instead.

- [ ] **Step 2: Run the full file's test suite and commit**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::commands::skill -- --nocapture 2>&1 | tail -60`
Expected: all tests in this file PASS, including every pre-existing one.

```bash
git add src-tauri/src/engine/commands/skill.rs
git commit -m "test(skill): cover mandatory flag in skill.list output"
```

---

### Task 6: Frontend — optional-skill checkboxes in Builder, badges in Library

**Files:**
- Modify: `src/ipc/types.ts`
- Modify: `src/components/Builder.tsx`
- Modify: `src/components/SkillLibrary.tsx`

**Interfaces:**
- Consumes: `Skill.mandatory: boolean` (new), `AgentDefinition.skillIds` (unchanged shape, now correctly includes selected-optional builtin ids per Task 4 — no frontend change needed to read it).
- Produces: no new component props, no new IPC calls — `Builder.tsx`'s existing `skillIds` state and existing `handleSave` payload (`skillIds: agentType === "cli" ? skillIds : undefined`) are reused verbatim; only the JSX rendering a second checkbox group is new.

- [ ] **Step 1: Add `mandatory` to the `Skill` type**

In `src/ipc/types.ts`, `Skill` currently is:

```typescript
export interface Skill {
  id: string;
  name: string;
  description?: string;
  content: string;
  kind: "builtin" | "custom";
  icon?: string;
  /** Annotated by `skill.list`: how many AgentDefinitions have this attached. */
  attachedTo?: number;
}
```

Add `mandatory` right after `kind`:

```typescript
export interface Skill {
  id: string;
  name: string;
  description?: string;
  content: string;
  kind: "builtin" | "custom";
  /** Only meaningful when `kind === "builtin"` — a mandatory builtin is
   *  auto-attached to every AgentDefinition and cannot be detached; an
   *  optional one (`mandatory: false` in its SKILL.md frontmatter) is
   *  picked per agent, like a custom skill, but still read-only content
   *  (see ADR 0003). Always `true` for `kind === "custom"` — there's
   *  nothing to opt into, custom skills are already opt-in via agent_skill. */
  mandatory: boolean;
  icon?: string;
  /** Annotated by `skill.list`: how many AgentDefinitions have this attached. */
  attachedTo?: number;
}
```

- [ ] **Step 2: Add `selectedBuiltinSkillIds` to `AgentDefinition` for wire-shape completeness**

Find `AgentDefinition`'s `skillIds` field (has a multi-line doc comment ending `Matches WorkspaceAgent.launchedSkillIds' basis exactly...`). Add a new optional field right after `skillIds`:

```typescript
  skillIds?: string[];
  /** Raw storage: which OPTIONAL builtin skill ids (`mandatory: false`) this
   *  definition has selected (see ADR 0003). `skillIds` above already
   *  reflects the full effective set (mandatory + this list + custom) — the
   *  Builder's checkboxes read `skillIds`, not this field, directly. */
  selectedBuiltinSkillIds?: string[];
  createdAt: string;
```

- [ ] **Step 3: Split Builder.tsx's "System skills" block into mandatory badges + optional checkboxes**

In `src/components/Builder.tsx`, find the Skills section (search for `System skills — always on`). The current block:

```tsx
                {allSkills.filter((s) => s.kind === "builtin").length > 0 && (
                  <div className="px-3 py-2">
                    <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
                      System skills — always on
                    </div>
                    <div className="flex flex-wrap gap-1.5">
                      {allSkills
                        .filter((s) => s.kind === "builtin")
                        .map((s) => (
                          <span
                            key={s.id}
                            className="text-[11px] font-medium px-2 py-0.5 rounded-md ring-1 ring-overlay/[0.08] text-text-secondary"
                          >
                            {s.name}
                          </span>
                        ))}
                    </div>
                  </div>
                )}
```

Replace it with two blocks — mandatory-only badges, then a new optional-builtin checkbox group (styled identically to the existing custom-skill checkbox group below it):

```tsx
                {allSkills.filter((s) => s.kind === "builtin" && s.mandatory).length > 0 && (
                  <div className="px-3 py-2">
                    <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
                      System skills — always on
                    </div>
                    <div className="flex flex-wrap gap-1.5">
                      {allSkills
                        .filter((s) => s.kind === "builtin" && s.mandatory)
                        .map((s) => (
                          <span
                            key={s.id}
                            className="text-[11px] font-medium px-2 py-0.5 rounded-md ring-1 ring-overlay/[0.08] text-text-secondary"
                          >
                            {s.name}
                          </span>
                        ))}
                    </div>
                  </div>
                )}
                {allSkills.filter((s) => s.kind === "builtin" && !s.mandatory).length > 0 && (
                  <div className="px-3 py-2">
                    <div className="text-[10px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
                      System skills — optional
                    </div>
                    <div className="space-y-1">
                      {allSkills
                        .filter((s) => s.kind === "builtin" && !s.mandatory)
                        .map((s) => {
                          const checked = skillIds.includes(s.id);
                          return (
                            <label
                              key={s.id}
                              className="flex items-center gap-2 text-[12.5px] text-text-secondary cursor-pointer"
                            >
                              <input
                                type="checkbox"
                                checked={checked}
                                onChange={(e) =>
                                  setSkillIds((prev) =>
                                    e.target.checked ? [...prev, s.id] : prev.filter((id) => id !== s.id),
                                  )
                                }
                              />
                              {s.name}
                            </label>
                          );
                        })}
                    </div>
                  </div>
                )}
```

- [ ] **Step 4: Add an "Always on" / "Optional" badge to `SystemSkillCard` in SkillLibrary.tsx**

In `src/components/SkillLibrary.tsx`, find `function SystemSkillCard({ skill }: { skill: Skill }) {`. Read its current JSX body first (`grep -n -A25 "function SystemSkillCard" src/components/SkillLibrary.tsx`) to match its existing layout/className conventions exactly, then add a small badge next to wherever the skill name is rendered:

```tsx
<span
  className={
    skill.mandatory
      ? "text-[10px] font-medium px-1.5 py-0.5 rounded ring-1 ring-overlay/[0.08] text-text-tertiary"
      : "text-[10px] font-medium px-1.5 py-0.5 rounded ring-1 ring-accent/30 text-accent"
  }
>
  {skill.mandatory ? "Always on" : "Optional"}
</span>
```

Place it wherever fits the existing card layout best (e.g. next to the skill name, or in a small metadata row below the description) — match the file's existing spacing/className idioms rather than introducing a new visual pattern. This is a read-only label; no new interaction, no new props, no state.

- [ ] **Step 5: Type-check and build**

Run: `pnpm exec tsc --noEmit`
Expected: no errors.

Run: `pnpm build`
Expected: succeeds.

- [ ] **Step 6: Manual smoke check (documented, not automated — this repo has no frontend test runner)**

Start the dev server (`pnpm tauri dev` or `pnpm dev` per this repo's existing convention) and, in the Builder for a `cli`-type agent:
- Confirm "System skills — always on" still shows `Example Skill` as a plain badge (unchanged from before this plan).
- Confirm a NEW "System skills — optional" section appears showing `Example Optional Skill` as a checkbox, unchecked by default.
- Check it, save, reopen the same agent's Builder — confirm it comes back checked (proves `skillIds` round-trips through `agentDef.save` → `agentDef.list` → the Builder's `initialDef?.skillIds` seed).
- Open the Skill Library, confirm the System section shows "Always on" on `Example Skill` and "Optional" on `Example Optional Skill`.
If a full app build/run isn't available in this environment, say so explicitly rather than claim the smoke check passed — matching this project's established pattern for Tauri-runtime-dependent manual verification gaps.

- [ ] **Step 7: Commit**

```bash
git add src/ipc/types.ts src/components/Builder.tsx src/components/SkillLibrary.tsx
git commit -m "feat(ui): pick optional system skills per agent in Builder"
```

---

## Task Order and Dependencies

Tasks 1, 2, and 3 have a real dependency chain at the Rust compiler level:

- Task 2's `content_for_agent` calls `repo::agent_definition::get(...).selected_builtin_skill_ids` — requires Task 3's field to exist on `AgentDefRow`.
- Task 3's roundtrip test inserts against the `agent_definition.selected_builtin_skill_ids` DB column — requires Task 1's migration to have run.
- Task 4 calls `repo::skill::effective_builtin_skills` (Task 2) and reads/writes `AgentDefinitionInput.selected_builtin_skill_ids` (Task 3).

**Execute strictly in order: Task 1 → Task 2 → Task 3 → Task 4 → Task 5 → Task 6.** Despite Task 2 being listed before Task 3, its Step 9 (the `content_for_agent` change) is written assuming Task 3's field already exists — if running Tasks in strict numeric order, dispatch Task 3 immediately after Task 1 and BEFORE Task 2's Step 9, OR reorder execution to 1 → 3 → 2 → 4 → 5 → 6. **The controller executing this plan should run migration+repo tasks in the dependency order 1 → 3 → 2 → 4 → 5 → 6, not the numeric listing order** — Tasks 2 and 3 are listed in this document in the order that reads most naturally against ADR 0003 (skill logic before the agent-definition column it depends on), not execution order.
