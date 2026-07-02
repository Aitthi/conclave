//! Skill repository — reusable prompt modules attached to `AgentDefinition`s.
//! Mirrors `agent_definition.rs`'s pattern. Builtin-vs-custom AUTHORIZATION
//! (rejecting an edit/delete of a `kind = 'builtin'` row) is enforced at the
//! COMMAND layer (`commands::skill`), not here — this module is a plain CRUD
//! mirror, same division of responsibility as `agent_definition`'s
//! `permission_mode` allowlist check living in `commands::agent`, not the repo.

use super::cb_err;
use chain_builder::{Order, QueryBuilder, Sqlite, Value as Bind};
use serde::Serialize;
use sqlx::SqlitePool;
use std::collections::HashMap;
use uuid::Uuid;

/// A skill as seen by callers — either a builtin (from a bundled folder, see
/// `list_builtin`) or a custom one (a DB row). `kind` distinguishes them for
/// JSON output; nothing about this struct's SHAPE differs by kind.
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

/// Decoded row from the `skill` table, which (as of migration 0005) has no
/// `kind` column at all — every row is, structurally, a custom skill. Never
/// exposed outside this module; converted to `SkillRow` via `From` below.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
struct CustomSkillDbRow {
    id: String,
    name: String,
    description: Option<String>,
    content: String,
    icon: Option<String>,
}

impl From<CustomSkillDbRow> for SkillRow {
    fn from(r: CustomSkillDbRow) -> Self {
        SkillRow {
            id: r.id,
            name: r.name,
            description: r.description,
            content: r.content,
            kind: "custom".to_owned(),
            mandatory: true,
            icon: r.icon,
        }
    }
}

const COLS: [&str; 5] = ["id", "name", "description", "content", "icon"];

/// All CUSTOM skills (the only kind stored in the DB), ordered by name.
pub async fn list(pool: &SqlitePool) -> sqlx::Result<Vec<SkillRow>> {
    QueryBuilder::<Sqlite>::table("skill")
        .select(COLS)
        .order_by("name", Order::Asc)
        .fetch_all::<CustomSkillDbRow, _>(pool)
        .await
        .map_err(cb_err)
        .map(|rows| rows.into_iter().map(SkillRow::from).collect())
}

/// Fetch a single CUSTOM skill by `id`, or `None` if it does not exist. A
/// builtin id (never in the DB) correctly returns `None` here — callers
/// needing to distinguish "unknown id" from "builtin id" must check
/// `list_builtin()` first (see `commands::skill::save`/`delete`).
pub async fn get(pool: &SqlitePool, id: &str) -> sqlx::Result<Option<SkillRow>> {
    QueryBuilder::<Sqlite>::table("skill")
        .select(COLS)
        .where_eq("id", id)
        .fetch_optional::<CustomSkillDbRow, _>(pool)
        .await
        .map_err(cb_err)
        .map(|opt| opt.map(SkillRow::from))
}

/// Create a new custom skill and return the constructed row.
pub async fn create(
    pool: &SqlitePool,
    name: &str,
    description: Option<&str>,
    content: &str,
) -> sqlx::Result<SkillRow> {
    let id = Uuid::new_v4().to_string();

    QueryBuilder::<Sqlite>::table("skill")
        .insert([
            ("id", Bind::Text(id.clone())),
            ("name", Bind::Text(name.to_owned())),
            (
                "description",
                description
                    .map(|d| Bind::Text(d.to_owned()))
                    .unwrap_or(Bind::Null),
            ),
            ("content", Bind::Text(content.to_owned())),
            ("icon", Bind::Null),
        ])
        .execute(pool)
        .await
        .map_err(cb_err)?;

    Ok(SkillRow {
        id,
        name: name.to_owned(),
        description: description.map(str::to_owned),
        content: content.to_owned(),
        kind: "custom".to_owned(),
        mandatory: true,
        icon: None,
    })
}

/// Update an existing custom skill's mutable fields and return the updated
/// row, or `None` if no row with `id` exists. Does NOT check whether `id` is
/// a builtin — a builtin id is simply never in the DB, so `get(id)` after
/// this returns `None` for one; the command layer must reject a builtin
/// mutation attempt BEFORE calling this (see `commands::skill::save`).
pub async fn update(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    description: Option<&str>,
    content: &str,
) -> sqlx::Result<Option<SkillRow>> {
    QueryBuilder::<Sqlite>::table("skill")
        .update([
            ("name", Bind::Text(name.to_owned())),
            (
                "description",
                description
                    .map(|d| Bind::Text(d.to_owned()))
                    .unwrap_or(Bind::Null),
            ),
            ("content", Bind::Text(content.to_owned())),
        ])
        .where_eq("id", id)
        .execute(pool)
        .await
        .map_err(cb_err)?;

    get(pool, id).await
}

/// Delete a custom skill. Returns `true` if a row was deleted. `agent_skill`
/// rows referencing it cascade away (`ON DELETE CASCADE`, `0001_init.sql:144`).
pub async fn delete(pool: &SqlitePool, id: &str) -> sqlx::Result<bool> {
    let res = sqlx::query("DELETE FROM skill WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// All skills attached to `agent_def_id` via `agent_skill`, ordered by
/// `sort_order` then `id` as a stable tie-breaker. `agent_skill` only ever
/// references CUSTOM (DB) skills — builtins are never attached via this
/// table, see `content_for_agent`.
pub async fn attached_to_agent(
    pool: &SqlitePool,
    agent_def_id: &str,
) -> sqlx::Result<Vec<SkillRow>> {
    let rows: Vec<CustomSkillDbRow> = sqlx::query_as(
        "SELECT s.id, s.name, s.description, s.content, s.icon \
         FROM agent_skill a JOIN skill s ON s.id = a.skill_id \
         WHERE a.agent_def_id = ? \
         ORDER BY a.sort_order ASC, s.id ASC",
    )
    .bind(agent_def_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(SkillRow::from).collect())
}

/// Replace an agent definition's CUSTOM skill attachments with exactly
/// `skill_ids`, in that order (index becomes `sort_order`). Delete-then-insert
/// inside one transaction. An empty slice clears all attachments.
pub async fn set_custom_attachments(
    pool: &SqlitePool,
    agent_def_id: &str,
    skill_ids: &[String],
) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM agent_skill WHERE agent_def_id = ?")
        .bind(agent_def_id)
        .execute(&mut *tx)
        .await?;

    for (idx, skill_id) in skill_ids.iter().enumerate() {
        sqlx::query(
            "INSERT INTO agent_skill (agent_def_id, skill_id, sort_order) VALUES (?, ?, ?)",
        )
        .bind(agent_def_id)
        .bind(skill_id)
        .bind(idx as i64)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// Every agent definition's custom skill ids, grouped by `agent_def_id` and
/// ordered by `sort_order` within each group. Used by `commands::agent::list`.
pub async fn custom_skill_ids_by_agent(
    pool: &SqlitePool,
) -> sqlx::Result<HashMap<String, Vec<String>>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT agent_def_id, skill_id FROM agent_skill ORDER BY agent_def_id, sort_order",
    )
    .fetch_all(pool)
    .await?;

    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for (agent_def_id, skill_id) in rows {
        map.entry(agent_def_id).or_default().push(skill_id);
    }
    Ok(map)
}

/// Count of `agent_skill` rows per skill id. Used by `commands::skill::list`
/// for the Library's "attached to N agents" label (custom skills only).
pub async fn attached_counts(pool: &SqlitePool) -> sqlx::Result<HashMap<String, i64>> {
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT skill_id, COUNT(*) FROM agent_skill GROUP BY skill_id")
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().collect())
}

/// Every skill shipped with the app (see
/// docs/adr/0002-builtin-skills-from-bundled-folder.md). Sync and infallible
/// — a missing/unreadable directory just yields zero skills rather than
/// propagating an error, so one bad file can never take down a `cli` agent's
/// launch (which depends on `content_for_agent` succeeding).
pub fn list_builtin() -> Vec<SkillRow> {
    read_builtin_skills_from(&skills_dir())
}

/// The actual filter behind "the builtin skills that apply to an agent
/// definition": every mandatory builtin (always), plus every optional
/// builtin (`mandatory: false`) whose id appears in `selected_optional_ids`.
/// Preserves `builtins`' incoming order (callers pass `list_builtin()`'s
/// id-ascending output). Pure and filesystem-free — takes a pre-fetched
/// `builtins` slice instead of reading the skills folder itself, so a caller
/// that needs the effective set for MANY agent definitions in a loop (e.g.
/// `commands::agent::list`) can call `list_builtin()` ONCE outside the loop
/// and then call this function per item, instead of re-scanning the
/// filesystem per item.
///
/// This is the single source of truth for the filter logic itself: both
/// `content_for_agent` (via the `effective_builtin_skills` wrapper below) and
/// `commands::agent::list` (which calls this function directly, passing its
/// own once-fetched `builtins`) MUST derive "the agent's effective builtin
/// ids" through this one filter — see ADR 0003's rationale (a v1 final
/// review caught a real bug from two call sites computing that set via
/// separate, silently-drifting logic).
pub fn effective_from(builtins: &[SkillRow], selected_optional_ids: &[String]) -> Vec<SkillRow> {
    builtins
        .iter()
        .filter(|s| s.mandatory || selected_optional_ids.iter().any(|id| id == &s.id))
        .cloned()
        .collect()
}

/// Thin, filesystem-reading convenience wrapper around [`effective_from`] for
/// callers that only need the effective set for a SINGLE agent definition
/// (e.g. `content_for_agent`, called once per `cli` launch — no per-item-loop
/// regression risk). Fetches `list_builtin()` fresh on every call, so a
/// caller iterating over many agent definitions should call
/// [`list_builtin`] once and use [`effective_from`] directly instead (see
/// `commands::agent::list`).
pub fn effective_builtin_skills(selected_optional_ids: &[String]) -> Vec<SkillRow> {
    effective_from(&list_builtin(), selected_optional_ids)
}

/// Parse every skill in `dir` — each direct subdirectory containing a
/// `SKILL.md` file becomes one builtin `SkillRow` (`id` = the subdirectory's
/// name). A subdirectory with no `SKILL.md`, or one with unparsable
/// frontmatter, is silently skipped rather than failing the whole read.
fn read_builtin_skills_from(dir: &std::path::Path) -> Vec<SkillRow> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(id) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            continue;
        };
        let Ok(raw) = std::fs::read_to_string(path.join("SKILL.md")) else {
            #[cfg(debug_assertions)]
            eprintln!("[skill] {}: no readable SKILL.md, skipping", path.display());
            continue;
        };
        let Some((name, description, content, mandatory)) = parse_skill_md(&raw) else {
            #[cfg(debug_assertions)]
            eprintln!(
                "[skill] {}: unparsable SKILL.md frontmatter, skipping",
                path.display()
            );
            continue;
        };
        out.push(SkillRow {
            id,
            name,
            description,
            content,
            kind: "builtin".to_owned(),
            mandatory,
            icon: None,
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

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

/// Resolve the real builtin-skills directory: the bundled `Resources/skills`
/// sibling of the running executable inside a packaged `.app` (mirrors
/// `agentctx::ensure_conclave_shim`'s exe-relative resolution for
/// `conclave-cli`), falling back to the source tree's `skills/` directory
/// (`CARGO_MANIFEST_DIR`, a compile-time constant) for a `cargo
/// run`/`cargo test`/`tauri dev` build, none of which have a `.app` bundle
/// structure.
fn skills_dir() -> std::path::PathBuf {
    #[cfg(test)]
    if let Some(dir) = test_support::override_dir() {
        return dir;
    }
    if let Some(bundled) = bundled_skills_dir() {
        if bundled.is_dir() {
            return bundled;
        }
    }
    std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/skills"))
}

fn bundled_skills_dir() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // macOS .app layout: Contents/MacOS/<exe> ; Contents/Resources/<...>
    Some(exe.parent()?.parent()?.join("Resources").join("skills"))
}

/// Test-only override of the builtin-skills directory, so tests never depend
/// on the SHIPPED skill content (which is product copy, free to change).
/// Thread-local because `cargo test` runs tests on parallel threads and every
/// affected test uses `#[tokio::test]`'s default current-thread runtime — the
/// whole test, including awaited repo calls, stays on one thread.
#[cfg(test)]
pub mod test_support {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    thread_local! {
        static OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }

    pub(super) fn override_dir() -> Option<PathBuf> {
        OVERRIDE.with(|c| c.borrow().clone())
    }

    /// RAII guard from [`fixture_skills_dir`]. While alive, `skills_dir()` on
    /// THIS thread resolves to the guard's temp fixture dir. `Drop` restores
    /// the real resolution and deletes the fixture dir.
    pub struct FixtureSkillsDir {
        dir: PathBuf,
    }

    impl Drop for FixtureSkillsDir {
        fn drop(&mut self) {
            OVERRIDE.with(|c| *c.borrow_mut() = None);
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Create the standard two-skill fixture and point this thread's builtin
    /// resolution at it: `fix-mandatory` (name "Fixture Mandatory", mandatory
    /// by omission) and `fix-optional` (name "Fixture Optional",
    /// `mandatory: false`). `tag` must be unique per test — it names the temp
    /// dir, and tests run concurrently under one shared temp root.
    pub fn fixture_skills_dir(tag: &str) -> FixtureSkillsDir {
        let dir = std::env::temp_dir().join(format!("conclave-skill-fixture-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        write_skill(
            &dir,
            "fix-mandatory",
            "---\nname: Fixture Mandatory\ndescription: Mandatory test fixture\n---\n\nMandatory fixture content.\n",
        );
        write_skill(
            &dir,
            "fix-optional",
            "---\nname: Fixture Optional\ndescription: Optional test fixture\nmandatory: false\n---\n\nOptional fixture content.\n",
        );
        OVERRIDE.with(|c| *c.borrow_mut() = Some(dir.clone()));
        FixtureSkillsDir { dir }
    }

    fn write_skill(root: &Path, id: &str, raw: &str) {
        let d = root.join(id);
        std::fs::create_dir_all(&d).expect("fixture mkdir failed");
        std::fs::write(d.join("SKILL.md"), raw).expect("fixture SKILL.md write failed");
    }
}

/// Allocate a fresh scratch directory for one skill-draft agent-assist
/// session (see docs/specs/2026-07-02-skill-editor-agent-assist-design.md),
/// under the same per-user Conclave data dir `agentctx::write_skill_sidecar`
/// uses, in its own `skill-drafts/<uuid>` subdirectory so concurrent
/// sessions never collide.
#[allow(dead_code)]
pub fn new_draft_dir() -> std::io::Result<std::path::PathBuf> {
    let dir = dirs::data_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no user data directory"))?
        .join("Conclave")
        .join("skill-drafts")
        .join(Uuid::new_v4().to_string());
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Serialize a skill draft's `SKILL.md` in the same frontmatter format
/// `parse_skill_md` reads back — so the file an agent-assist session's CLI
/// agent edits looks identical to a real builtin `SKILL.md`, and the same
/// parser handles both.
#[allow(dead_code)]
fn render_draft_skill_md(name: &str, description: Option<&str>, content: &str) -> String {
    let mut out = format!("---\nname: {name}\n");
    if let Some(d) = description {
        out.push_str(&format!("description: {d}\n"));
    }
    out.push_str("---\n\n");
    out.push_str(content);
    out.push('\n');
    out
}

/// Write a skill draft's current `name`/`description`/`content` to `dir`'s
/// `SKILL.md` (creating `dir` if missing), for an agent-assist session's CLI
/// agent to edit directly.
#[allow(dead_code)]
pub fn write_draft(
    dir: &std::path::Path,
    name: &str,
    description: Option<&str>,
    content: &str,
) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(
        dir.join("SKILL.md"),
        render_draft_skill_md(name, description, content),
    )
}

/// Read a skill draft's current `SKILL.md` back out of `dir`, reusing
/// `parse_skill_md` (its `mandatory` element is irrelevant for a custom
/// skill draft and is dropped). `None` if the file is missing or its
/// frontmatter is currently unparsable (e.g. the agent is mid-write) — the
/// caller (`commands::skill_draft::sync`) leaves the editor's last
/// successfully synced fields untouched in that case rather than erroring.
#[allow(dead_code)]
pub fn read_draft(dir: &std::path::Path) -> Option<(String, Option<String>, String)> {
    let raw = std::fs::read_to_string(dir.join("SKILL.md")).ok()?;
    let (name, description, content, _mandatory) = parse_skill_md(&raw)?;
    Some((name, description, content))
}

/// Build the concatenated skill body for one agent definition's `cli` launch,
/// plus the ordered list of skill ids used (for `session.launched_skill_ids`).
/// Builtin skills come first (fixed `id` order, via `list_builtin`), then
/// custom skills by `sort_order` (via `attached_to_agent`). Each skill renders
/// as a `## Skill: {name}` header followed by its `content`, sections
/// separated by a blank line.
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

#[cfg(test)]
mod tests {
    use crate::engine::db::connect_in_memory;

    #[tokio::test]
    async fn create_then_get_roundtrip() {
        let pool = connect_in_memory().await;
        let row = super::create(
            &pool,
            "Reviewer",
            Some("Reviews diffs"),
            "Always check for X",
        )
        .await
        .expect("create failed");
        assert_eq!(row.name, "Reviewer");
        assert_eq!(row.description.as_deref(), Some("Reviews diffs"));
        assert_eq!(row.content, "Always check for X");
        assert_eq!(row.kind, "custom");
        assert!(row.icon.is_none());

        let fetched = super::get(&pool, &row.id)
            .await
            .expect("get failed")
            .expect("row should exist");
        assert_eq!(fetched, row);
    }

    #[tokio::test]
    async fn create_with_no_description_stores_none() {
        let pool = connect_in_memory().await;
        let row = super::create(&pool, "Bare", None, "content")
            .await
            .expect("create failed");
        assert!(row.description.is_none());
    }

    #[tokio::test]
    async fn update_changes_fields() {
        let pool = connect_in_memory().await;
        let row = super::create(&pool, "Old", Some("old desc"), "old content")
            .await
            .expect("create failed");

        let updated = super::update(&pool, &row.id, "New", Some("new desc"), "new content")
            .await
            .expect("update failed")
            .expect("row should exist after update");
        assert_eq!(updated.name, "New");
        assert_eq!(updated.description.as_deref(), Some("new desc"));
        assert_eq!(updated.content, "new content");
        assert_eq!(updated.kind, "custom");
    }

    #[tokio::test]
    async fn update_unknown_id_returns_none() {
        let pool = connect_in_memory().await;
        let result = super::update(&pool, "no-such-id", "X", None, "Y")
            .await
            .expect("update should not error");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn delete_removes_then_second_is_noop() {
        let pool = connect_in_memory().await;
        let row = super::create(&pool, "Gone", None, "c")
            .await
            .expect("create failed");
        assert!(super::delete(&pool, &row.id).await.expect("delete failed"));
        assert!(super::get(&pool, &row.id)
            .await
            .expect("get failed")
            .is_none());
        assert!(!super::delete(&pool, &row.id)
            .await
            .expect("second delete failed"));
    }

    #[tokio::test]
    async fn list_returns_only_custom_ordered_by_name() {
        let pool = connect_in_memory().await;
        super::create(&pool, "Zeta", None, "z")
            .await
            .expect("create failed");
        super::create(&pool, "Alpha", None, "a")
            .await
            .expect("create failed");

        let all = super::list(&pool).await.expect("list failed");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, "Alpha", "must be name-ordered");
        assert_eq!(all[1].name, "Zeta");
        assert!(all.iter().all(|s| s.kind == "custom"));
    }

    /// JSON must use camelCase keys and omit `description`/`icon` when None.
    #[tokio::test]
    async fn camel_case_contract() {
        let pool = connect_in_memory().await;
        let row = super::create(&pool, "Sol", None, "content")
            .await
            .expect("create failed");
        let json = serde_json::to_value(&row).expect("serialize failed");
        assert!(json.get("kind").is_some());
        assert!(json.get("content").is_some());
        assert!(json.get("description").is_none(), "None must be omitted");
        assert!(json.get("icon").is_none(), "None must be omitted");
    }

    async fn fixture_agent_def(pool: &sqlx::SqlitePool) -> String {
        crate::engine::repo::agent_definition::create(
            pool,
            &crate::engine::repo::agent_definition::AgentDefinitionInput {
                name: "Fixture".into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed")
        .id
    }

    #[tokio::test]
    async fn attached_to_agent_respects_sort_order() {
        let pool = connect_in_memory().await;
        let def_id = fixture_agent_def(&pool).await;
        let s1 = super::create(&pool, "First", None, "1")
            .await
            .expect("create failed");
        let s2 = super::create(&pool, "Second", None, "2")
            .await
            .expect("create failed");

        // Attach in REVERSE order to prove sort_order (not insertion order) wins.
        super::set_custom_attachments(&pool, &def_id, &[s2.id.clone(), s1.id.clone()])
            .await
            .expect("set_custom_attachments failed");

        let attached = super::attached_to_agent(&pool, &def_id)
            .await
            .expect("query failed");
        assert_eq!(attached.len(), 2);
        assert_eq!(attached[0].id, s2.id, "sort_order 0 must come first");
        assert_eq!(attached[1].id, s1.id);
    }

    #[tokio::test]
    async fn set_custom_attachments_replaces_not_appends() {
        let pool = connect_in_memory().await;
        let def_id = fixture_agent_def(&pool).await;
        let s1 = super::create(&pool, "A", None, "a")
            .await
            .expect("create failed");
        let s2 = super::create(&pool, "B", None, "b")
            .await
            .expect("create failed");

        super::set_custom_attachments(&pool, &def_id, std::slice::from_ref(&s1.id))
            .await
            .expect("first set failed");
        super::set_custom_attachments(&pool, &def_id, std::slice::from_ref(&s2.id))
            .await
            .expect("second set failed");

        let attached = super::attached_to_agent(&pool, &def_id)
            .await
            .expect("query failed");
        assert_eq!(attached.len(), 1, "second call must REPLACE, not append");
        assert_eq!(attached[0].id, s2.id);
    }

    #[tokio::test]
    async fn set_custom_attachments_empty_clears() {
        let pool = connect_in_memory().await;
        let def_id = fixture_agent_def(&pool).await;
        let s1 = super::create(&pool, "A", None, "a")
            .await
            .expect("create failed");
        super::set_custom_attachments(&pool, &def_id, std::slice::from_ref(&s1.id))
            .await
            .expect("set failed");
        super::set_custom_attachments(&pool, &def_id, &[])
            .await
            .expect("clear failed");

        let attached = super::attached_to_agent(&pool, &def_id)
            .await
            .expect("query failed");
        assert!(attached.is_empty());
    }

    #[tokio::test]
    async fn custom_skill_ids_by_agent_groups_correctly() {
        let pool = connect_in_memory().await;
        let def1 = fixture_agent_def(&pool).await;
        let def2 = fixture_agent_def(&pool).await;
        let s1 = super::create(&pool, "A", None, "a")
            .await
            .expect("create failed");
        let s2 = super::create(&pool, "B", None, "b")
            .await
            .expect("create failed");
        super::set_custom_attachments(&pool, &def1, &[s1.id.clone(), s2.id.clone()])
            .await
            .expect("set failed");
        super::set_custom_attachments(&pool, &def2, std::slice::from_ref(&s1.id))
            .await
            .expect("set failed");

        let map = super::custom_skill_ids_by_agent(&pool)
            .await
            .expect("query failed");
        assert_eq!(map.get(&def1).cloned().unwrap_or_default().len(), 2);
        assert_eq!(map.get(&def2).cloned().unwrap_or_default(), vec![s1.id]);
    }

    #[tokio::test]
    async fn attached_counts_counts_across_agents() {
        let pool = connect_in_memory().await;
        let def1 = fixture_agent_def(&pool).await;
        let def2 = fixture_agent_def(&pool).await;
        let s1 = super::create(&pool, "Shared", None, "s")
            .await
            .expect("create failed");
        super::set_custom_attachments(&pool, &def1, std::slice::from_ref(&s1.id))
            .await
            .expect("set failed");
        super::set_custom_attachments(&pool, &def2, std::slice::from_ref(&s1.id))
            .await
            .expect("set failed");

        let counts = super::attached_counts(&pool).await.expect("query failed");
        assert_eq!(counts.get(&s1.id).copied(), Some(2));
    }

    #[tokio::test]
    async fn delete_skill_cascades_agent_skill_rows() {
        let pool = connect_in_memory().await;
        let def_id = fixture_agent_def(&pool).await;
        let s1 = super::create(&pool, "Doomed", None, "d")
            .await
            .expect("create failed");
        super::set_custom_attachments(&pool, &def_id, std::slice::from_ref(&s1.id))
            .await
            .expect("set failed");

        super::delete(&pool, &s1.id).await.expect("delete failed");

        let attached = super::attached_to_agent(&pool, &def_id)
            .await
            .expect("query failed");
        assert!(attached.is_empty(), "agent_skill row must cascade away");
        assert!(
            crate::engine::repo::agent_definition::exists(&pool, &def_id)
                .await
                .expect("exists check failed"),
            "agent_definition itself must be untouched"
        );
    }

    #[test]
    fn parse_skill_md_extracts_frontmatter_and_body() {
        let raw = "---\nname: Reviewer\ndescription: Reviews diffs\n---\n\nAlways check X.\n";
        let (name, description, content, mandatory) =
            super::parse_skill_md(raw).expect("should parse");
        assert_eq!(name, "Reviewer");
        assert_eq!(description.as_deref(), Some("Reviews diffs"));
        assert_eq!(content, "Always check X.");
        assert!(
            mandatory,
            "no mandatory: line present, must default to true"
        );
    }

    #[test]
    fn parse_skill_md_description_optional() {
        let raw = "---\nname: Bare\n---\n\nBody only.\n";
        let (name, description, content, mandatory) =
            super::parse_skill_md(raw).expect("should parse");
        assert_eq!(name, "Bare");
        assert!(description.is_none());
        assert_eq!(content, "Body only.");
        assert!(
            mandatory,
            "no mandatory: line present, must default to true"
        );
    }

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

    #[test]
    fn parse_skill_md_rejects_missing_frontmatter() {
        assert!(super::parse_skill_md("Just a body, no frontmatter.").is_none());
    }

    #[test]
    fn parse_skill_md_rejects_missing_name() {
        let raw = "---\ndescription: no name here\n---\n\nBody.\n";
        assert!(super::parse_skill_md(raw).is_none());
    }

    /// Real filesystem test (no tempfile crate in this codebase — mirrors
    /// `agentctx.rs`'s existing sidecar test pattern of writing to a real
    /// path under `std::env::temp_dir()` and cleaning up manually).
    #[test]
    fn read_builtin_skills_from_parses_one_skill_per_subdir_skips_bad_ones() {
        let dir = std::env::temp_dir().join("conclave-skill-test-read-builtin-skills");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("good")).expect("mkdir failed");
        std::fs::write(
            dir.join("good").join("SKILL.md"),
            "---\nname: Good One\ndescription: Works\n---\n\nDo the thing.\n",
        )
        .expect("write failed");
        // A subdir with no SKILL.md at all must be silently skipped.
        std::fs::create_dir_all(dir.join("empty-dir")).expect("mkdir failed");
        // A subdir with unparsable frontmatter must be silently skipped.
        std::fs::create_dir_all(dir.join("bad")).expect("mkdir failed");
        std::fs::write(dir.join("bad").join("SKILL.md"), "no frontmatter here")
            .expect("write failed");
        // A plain FILE directly under dir (not a subdirectory) must be ignored.
        std::fs::write(dir.join("stray.txt"), "ignore me").expect("write failed");

        let skills = super::read_builtin_skills_from(&dir);

        assert_eq!(
            skills.len(),
            1,
            "only the well-formed 'good' skill should survive"
        );
        assert_eq!(skills[0].id, "good");
        assert_eq!(skills[0].name, "Good One");
        assert_eq!(skills[0].description.as_deref(), Some("Works"));
        assert_eq!(skills[0].content, "Do the thing.");
        assert_eq!(skills[0].kind, "builtin");

        std::fs::remove_dir_all(&dir).expect("cleanup failed");
    }

    #[test]
    fn read_builtin_skills_from_missing_dir_returns_empty() {
        let dir = std::env::temp_dir().join("conclave-skill-test-does-not-exist-xyz");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(super::read_builtin_skills_from(&dir).is_empty());
    }

    /// The real, checked-in `src-tauri/skills/example/` fixture must be
    /// discoverable via the production `list_builtin()` entry point in a
    /// `cargo test` run (its CARGO_MANIFEST_DIR fallback resolves to
    /// `src-tauri/skills` regardless of CWD).
    #[test]
    fn list_builtin_finds_the_checked_in_example_skill() {
        let skills = super::list_builtin();
        let example = skills
            .iter()
            .find(|s| s.id == "example")
            .expect("the checked-in example skill must be discoverable");
        assert_eq!(example.name, "Example Skill");
        assert_eq!(example.kind, "builtin");
    }

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

    /// Fixture builtins for `effective_from` tests — deliberately NOT read
    /// from the filesystem, so these tests exercise the pure filter logic in
    /// isolation from `list_builtin()`.
    fn fixture_builtins() -> Vec<super::SkillRow> {
        vec![
            super::SkillRow {
                id: "mandatory-one".to_string(),
                name: "Mandatory One".to_string(),
                description: None,
                content: "mandatory-one content".to_string(),
                kind: "builtin".to_string(),
                mandatory: true,
                icon: None,
            },
            super::SkillRow {
                id: "optional-one".to_string(),
                name: "Optional One".to_string(),
                description: None,
                content: "optional-one content".to_string(),
                kind: "builtin".to_string(),
                mandatory: false,
                icon: None,
            },
            super::SkillRow {
                id: "optional-two".to_string(),
                name: "Optional Two".to_string(),
                description: None,
                content: "optional-two content".to_string(),
                kind: "builtin".to_string(),
                mandatory: false,
                icon: None,
            },
        ]
    }

    #[test]
    fn effective_from_always_includes_mandatory() {
        let ids = super::effective_from(&fixture_builtins(), &[])
            .into_iter()
            .map(|s| s.id)
            .collect::<Vec<_>>();
        assert!(
            ids.contains(&"mandatory-one".to_string()),
            "mandatory builtin must be present even with zero selections"
        );
        assert!(
            !ids.contains(&"optional-one".to_string()),
            "optional builtin must be absent when not selected"
        );
        assert!(!ids.contains(&"optional-two".to_string()));
    }

    #[test]
    fn effective_from_includes_selected_optional() {
        let ids = super::effective_from(&fixture_builtins(), &["optional-two".to_string()])
            .into_iter()
            .map(|s| s.id)
            .collect::<Vec<_>>();
        assert!(ids.contains(&"mandatory-one".to_string()));
        assert!(ids.contains(&"optional-two".to_string()));
        assert!(!ids.contains(&"optional-one".to_string()));
    }

    #[test]
    fn effective_from_ignores_unknown_selected_id() {
        let ids = super::effective_from(&fixture_builtins(), &["no-such-skill".to_string()])
            .into_iter()
            .map(|s| s.id)
            .collect::<Vec<_>>();
        assert!(!ids.contains(&"no-such-skill".to_string()));
        assert!(ids.contains(&"mandatory-one".to_string()));
    }

    #[test]
    fn list_builtin_reports_mandatory_flags_for_both_fixtures() {
        let skills = super::list_builtin();
        let mandatory = skills
            .iter()
            .find(|s| s.id == "example")
            .expect("example fixture must exist");
        assert!(
            mandatory.mandatory,
            "example/SKILL.md has no mandatory: line, must default true"
        );

        let optional = skills
            .iter()
            .find(|s| s.id == "example-optional")
            .expect("example-optional fixture must exist");
        assert!(
            !optional.mandatory,
            "example-optional/SKILL.md sets mandatory: false"
        );
    }

    #[tokio::test]
    async fn content_for_agent_orders_builtin_then_custom_with_headers() {
        let pool = connect_in_memory().await;
        let def_id = fixture_agent_def(&pool).await;
        let custom = super::create(&pool, "Extra", None, "Do X")
            .await
            .expect("create failed");
        super::set_custom_attachments(&pool, &def_id, std::slice::from_ref(&custom.id))
            .await
            .expect("set failed");

        let (body, ids) = super::content_for_agent(&pool, &def_id)
            .await
            .expect("query failed");

        assert_eq!(
            ids,
            vec!["example".to_string(), custom.id.clone()],
            "builtin (the checked-in example) must come first"
        );
        let base_pos = body
            .find("## Skill: Example Skill")
            .expect("Example header missing");
        let extra_pos = body.find("## Skill: Extra").expect("Extra header missing");
        assert!(
            base_pos < extra_pos,
            "builtin section must precede custom section"
        );
        assert!(body.contains("Do X"));
    }

    #[tokio::test]
    async fn content_for_agent_still_includes_builtin_when_nothing_custom_attached() {
        let pool = connect_in_memory().await;
        let def_id = fixture_agent_def(&pool).await;
        let (body, ids) = super::content_for_agent(&pool, &def_id)
            .await
            .expect("query failed");
        assert_eq!(ids, vec!["example".to_string()]);
        assert!(body.contains("## Skill: Example Skill"));
    }

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
        sqlx::query("UPDATE agent_definition SET selected_builtin_skill_ids = ? WHERE id = ?")
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

    #[test]
    fn write_draft_then_read_draft_round_trips() {
        let dir = std::env::temp_dir().join("conclave-skill-test-draft-roundtrip");
        let _ = std::fs::remove_dir_all(&dir);

        super::write_draft(&dir, "My Draft", Some("A test draft"), "Body text here.")
            .expect("write_draft failed");
        let (name, description, content) =
            super::read_draft(&dir).expect("read_draft should parse what write_draft wrote");

        assert_eq!(name, "My Draft");
        assert_eq!(description.as_deref(), Some("A test draft"));
        assert_eq!(content, "Body text here.");

        std::fs::remove_dir_all(&dir).expect("cleanup failed");
    }

    #[test]
    fn write_draft_with_no_description_round_trips() {
        let dir = std::env::temp_dir().join("conclave-skill-test-draft-no-desc");
        let _ = std::fs::remove_dir_all(&dir);

        super::write_draft(&dir, "Bare", None, "Content.").expect("write_draft failed");
        let (name, description, content) = super::read_draft(&dir).expect("should parse");

        assert_eq!(name, "Bare");
        assert!(description.is_none());
        assert_eq!(content, "Content.");

        std::fs::remove_dir_all(&dir).expect("cleanup failed");
    }

    #[test]
    fn read_draft_missing_file_returns_none() {
        let dir = std::env::temp_dir().join("conclave-skill-test-draft-missing-xyz");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir failed");

        assert!(super::read_draft(&dir).is_none());

        std::fs::remove_dir_all(&dir).expect("cleanup failed");
    }

    #[test]
    fn new_draft_dir_creates_a_fresh_empty_directory() {
        let dir = super::new_draft_dir().expect("new_draft_dir failed");
        assert!(dir.is_dir());
        assert!(
            std::fs::read_dir(&dir)
                .expect("read_dir failed")
                .next()
                .is_none(),
            "a freshly allocated draft dir must start empty"
        );
        std::fs::remove_dir_all(&dir).expect("cleanup failed");
    }

    /// While a `fixture_skills_dir` guard is alive, `list_builtin()` must read
    /// ONLY the fixture dir — the shipped `skills/` folder must not leak in.
    #[test]
    fn list_builtin_reads_from_fixture_override() {
        let _fx = super::test_support::fixture_skills_dir("override-basic");
        let ids: Vec<String> = super::list_builtin().into_iter().map(|s| s.id).collect();
        assert_eq!(
            ids,
            vec!["fix-mandatory".to_string(), "fix-optional".to_string()],
            "override dir must fully replace the shipped skills folder"
        );
    }

    /// Dropping the guard must restore the real (shipped) skills folder.
    #[test]
    fn fixture_override_is_restored_on_drop() {
        {
            let _fx = super::test_support::fixture_skills_dir("override-drop");
            assert!(super::list_builtin()
                .iter()
                .any(|s| s.id == "fix-mandatory"));
        }
        assert!(
            !super::list_builtin()
                .iter()
                .any(|s| s.id == "fix-mandatory"),
            "after Drop, list_builtin must be back on the real skills dir"
        );
    }
}
