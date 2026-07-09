//! Role repository — first-class agent roles (see ADR 0005). A role carries a
//! display name, a one-paragraph job description (used verbatim in the
//! bootstrap preamble and the roster), and a default skill bundle. Builtin
//! roles ship as a bundled `roles/<id>/ROLE.md` folder (mirroring ADR 0002's
//! builtin-skills pattern); custom roles are DB rows.
//!
//! This module mirrors `repo::skill`'s split of responsibility: it is a plain
//! CRUD + folder-reader mirror. Builtin-vs-custom AUTHORIZATION (rejecting a
//! save/delete of a builtin role) is enforced at the COMMAND layer
//! (`commands::role`), not here.

use super::cb_err;
use chain_builder::{Order, QueryBuilder, Sqlite, Value as Bind};
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

/// A role as seen by callers — either a builtin (from a bundled folder, see
/// `list_builtin`) or a custom one (a DB row). `kind` distinguishes them for
/// JSON output; nothing about this struct's SHAPE differs by kind.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleRow {
    pub id: String,
    pub name: String,
    /// The role's one-paragraph job description, baked verbatim into the
    /// agent's preamble and shown in the roster. Always present — a role with
    /// no description is malformed and is skipped by the folder reader.
    pub description: String,
    /// Default skill ids this role attaches (builtin or custom skill ids).
    /// Mandatory skills are NOT listed here — they attach to every agent
    /// regardless. Unknown ids are the caller's problem to filter at use
    /// time (mirrors `effective_builtin_skills`'s ignore-unknown behavior).
    pub skill_ids: Vec<String>,
    pub kind: String,
}

/// Decoded row from the `role` table. `skill_ids` is stored as a JSON-array
/// text column; converted to `RoleRow` via [`from_db_row`]. Never exposed
/// outside this module.
#[derive(Debug, Clone, sqlx::FromRow)]
struct CustomRoleDbRow {
    id: String,
    name: String,
    description: String,
    skill_ids: String,
    #[allow(dead_code)]
    created_at: String,
}

impl From<CustomRoleDbRow> for RoleRow {
    fn from(r: CustomRoleDbRow) -> Self {
        let skill_ids = serde_json::from_str::<Vec<String>>(&r.skill_ids).unwrap_or_default();
        RoleRow {
            id: r.id,
            name: r.name,
            description: r.description,
            skill_ids,
            kind: "custom".to_owned(),
        }
    }
}

const COLS: [&str; 5] = ["id", "name", "description", "skill_ids", "created_at"];

// ── Custom (DB) CRUD ─────────────────────────────────────────────────────────

/// All CUSTOM roles (the only kind stored in the DB), ordered by name.
pub async fn list(pool: &SqlitePool) -> sqlx::Result<Vec<RoleRow>> {
    QueryBuilder::<Sqlite>::table("role")
        .select(COLS)
        .order_by("name", Order::Asc)
        .fetch_all::<CustomRoleDbRow, _>(pool)
        .await
        .map_err(cb_err)
        .map(|rows| rows.into_iter().map(RoleRow::from).collect())
}

/// Fetch a single CUSTOM role by `id`, or `None` if it does not exist. A
/// builtin id (never in the DB) correctly returns `None` here — callers
/// needing to distinguish "unknown id" from "builtin id" must check
/// `list_builtin()` first (see `commands::role::save`/`delete`).
pub async fn get(pool: &SqlitePool, id: &str) -> sqlx::Result<Option<RoleRow>> {
    QueryBuilder::<Sqlite>::table("role")
        .select(COLS)
        .where_eq("id", id)
        .fetch_optional::<CustomRoleDbRow, _>(pool)
        .await
        .map_err(cb_err)
        .map(|opt| opt.map(RoleRow::from))
}

/// Create a new custom role and return the constructed row.
pub async fn create(
    pool: &SqlitePool,
    name: &str,
    description: &str,
    skill_ids: &[String],
) -> sqlx::Result<RoleRow> {
    let id = Uuid::new_v4().to_string();
    let created_at = Utc::now().to_rfc3339();
    let skill_ids_json = serde_json::to_string(skill_ids).unwrap_or_else(|_| "[]".to_owned());

    QueryBuilder::<Sqlite>::table("role")
        .insert([
            ("id", Bind::Text(id.clone())),
            ("name", Bind::Text(name.to_owned())),
            ("description", Bind::Text(description.to_owned())),
            ("skill_ids", Bind::Text(skill_ids_json)),
            ("created_at", Bind::Text(created_at)),
        ])
        .execute(pool)
        .await
        .map_err(cb_err)?;

    Ok(RoleRow {
        id,
        name: name.to_owned(),
        description: description.to_owned(),
        skill_ids: skill_ids.to_vec(),
        kind: "custom".to_owned(),
    })
}

/// Update an existing custom role's mutable fields and return the updated row,
/// or `None` if no row with `id` exists. Does NOT check whether `id` is a
/// builtin — the command layer must reject a builtin mutation BEFORE calling
/// this (see `commands::role::save`).
pub async fn update(
    pool: &SqlitePool,
    id: &str,
    name: &str,
    description: &str,
    skill_ids: &[String],
) -> sqlx::Result<Option<RoleRow>> {
    let skill_ids_json = serde_json::to_string(skill_ids).unwrap_or_else(|_| "[]".to_owned());
    QueryBuilder::<Sqlite>::table("role")
        .update([
            ("name", Bind::Text(name.to_owned())),
            ("description", Bind::Text(description.to_owned())),
            ("skill_ids", Bind::Text(skill_ids_json)),
        ])
        .where_eq("id", id)
        .execute(pool)
        .await
        .map_err(cb_err)?;

    get(pool, id).await
}

/// Delete a custom role. Returns `true` if a row was deleted. Any
/// `agent_definition.role_id` referencing it is NULLed first (the display-text
/// `role` column is left intact so nothing loses its label) — see
/// `agent_definition::clear_role_references`.
pub async fn delete(pool: &SqlitePool, id: &str) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;

    sqlx::query("UPDATE agent_definition SET role_id = NULL WHERE role_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    let res = sqlx::query("DELETE FROM role WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

// ── Builtin (bundled folder) reading ─────────────────────────────────────────

/// Every role shipped with the app (see ADR 0005 / ADR 0002). Sync and
/// infallible — a missing/unreadable directory just yields zero roles rather
/// than propagating an error, mirroring `skill::list_builtin`.
pub fn list_builtin() -> Vec<RoleRow> {
    read_builtin_roles_from(&roles_dir())
}

/// Look up a single role by id across builtin + custom sources. Builtin ids
/// (folder names) are checked first, then the DB. Returns `None` for an
/// unknown id. Phase B (`agentctx::bootstrap_preamble` baking the agent's own
/// role description) is the first production caller; exercised by tests now.
#[allow(dead_code)]
pub async fn find_any(pool: &SqlitePool, id: &str) -> sqlx::Result<Option<RoleRow>> {
    if let Some(builtin) = list_builtin().into_iter().find(|r| r.id == id) {
        return Ok(Some(builtin));
    }
    get(pool, id).await
}

/// Parse every role in `dir` — each direct subdirectory containing a `ROLE.md`
/// file becomes one builtin `RoleRow` (`id` = the subdirectory's name). A
/// subdirectory with no `ROLE.md`, or one with unparsable frontmatter, is
/// silently skipped rather than failing the whole read.
fn read_builtin_roles_from(dir: &std::path::Path) -> Vec<RoleRow> {
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
        let Ok(raw) = std::fs::read_to_string(path.join("ROLE.md")) else {
            #[cfg(debug_assertions)]
            eprintln!("[role] {}: no readable ROLE.md, skipping", path.display());
            continue;
        };
        let Some((name, description, skill_ids)) = parse_role_md(&raw) else {
            #[cfg(debug_assertions)]
            eprintln!(
                "[role] {}: unparsable ROLE.md frontmatter, skipping",
                path.display()
            );
            continue;
        };
        out.push(RoleRow {
            id,
            name,
            description,
            skill_ids,
            kind: "builtin".to_owned(),
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// Parse a `ROLE.md`'s `---`-delimited frontmatter (flat `key: value` lines —
/// `name`/`description`/`skills` recognized). Hand-rolled rather than pulling
/// in a YAML crate (see ADR 0002). Returns `None` (skip this role) if the file
/// doesn't start with a frontmatter block, or `name`/`description` is
/// missing/blank — a role's whole purpose is its verbatim job description, so
/// one lacking either is malformed. `skills` is comma-separated; absent or
/// blank yields an empty bundle.
fn parse_role_md(raw: &str) -> Option<(String, String, Vec<String>)> {
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let rest = raw.strip_prefix("---")?;
    let rest = rest
        .strip_prefix("\r\n")
        .or_else(|| rest.strip_prefix('\n'))?;
    let end = rest.find("\n---")?;
    let frontmatter = &rest[..end];

    let mut name = None;
    let mut description = None;
    let mut skill_ids = Vec::new();
    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("name:") {
            name = Some(v.trim().to_owned());
        } else if let Some(v) = line.strip_prefix("description:") {
            description = Some(v.trim().to_owned());
        } else if let Some(v) = line.strip_prefix("skills:") {
            skill_ids = v
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
        }
    }
    let name = name.filter(|s| !s.is_empty())?;
    let description = description.filter(|s| !s.is_empty())?;
    Some((name, description, skill_ids))
}

/// Resolve the real builtin-roles directory: the bundled `Resources/roles`
/// sibling of the running executable inside a packaged `.app` (mirrors
/// `skill::skills_dir`), falling back to the source tree's `roles/` directory
/// (`CARGO_MANIFEST_DIR`) for a `cargo run`/`cargo test`/`tauri dev` build.
fn roles_dir() -> std::path::PathBuf {
    #[cfg(test)]
    if let Some(dir) = test_support::override_dir() {
        return dir;
    }
    if let Some(bundled) = bundled_roles_dir() {
        if bundled.is_dir() {
            return bundled;
        }
    }
    std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/roles"))
}

fn bundled_roles_dir() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    // macOS .app layout: Contents/MacOS/<exe> ; Contents/Resources/<...>
    Some(exe.parent()?.parent()?.join("Resources").join("roles"))
}

/// Test-only override of the builtin-roles directory, so tests never depend on
/// the SHIPPED role content (which is product copy, free to change). Mirrors
/// `skill::test_support`.
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

    /// RAII guard from [`fixture_roles_dir`]. While alive, `roles_dir()` on
    /// THIS thread resolves to the guard's temp fixture dir. `Drop` restores
    /// the real resolution and deletes the fixture dir.
    pub struct FixtureRolesDir {
        dir: PathBuf,
    }

    impl Drop for FixtureRolesDir {
        fn drop(&mut self) {
            OVERRIDE.with(|c| *c.borrow_mut() = None);
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Create a two-role fixture and point this thread's builtin resolution at
    /// it: `fix-lead` (name "Fixture Lead", skills `alpha, beta`) and
    /// `fix-bare` (name "Fixture Bare", no skills). `tag` must be unique per
    /// test — it names the temp dir.
    pub fn fixture_roles_dir(tag: &str) -> FixtureRolesDir {
        let dir = std::env::temp_dir().join(format!("conclave-role-fixture-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        write_role(
            &dir,
            "fix-lead",
            "---\nname: Fixture Lead\ndescription: Leads the fixture team.\nskills: alpha, beta\n---\n\nBody.\n",
        );
        write_role(
            &dir,
            "fix-bare",
            "---\nname: Fixture Bare\ndescription: Carries only mandatory skills.\nskills:\n---\n\nBody.\n",
        );
        OVERRIDE.with(|c| *c.borrow_mut() = Some(dir.clone()));
        FixtureRolesDir { dir }
    }

    fn write_role(root: &Path, id: &str, raw: &str) {
        let d = root.join(id);
        std::fs::create_dir_all(&d).expect("fixture mkdir failed");
        std::fs::write(d.join("ROLE.md"), raw).expect("fixture ROLE.md write failed");
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::db::connect_in_memory;

    // ── Folder reader / parser ───────────────────────────────────────────────

    #[test]
    fn parse_role_md_extracts_frontmatter_and_skills() {
        let raw =
            "---\nname: Lead\ndescription: Leads the work.\nskills: leadership, agent-loop\n---\n\nBody.\n";
        let (name, description, skills) = super::parse_role_md(raw).expect("should parse");
        assert_eq!(name, "Lead");
        assert_eq!(description, "Leads the work.");
        assert_eq!(skills, vec!["leadership", "agent-loop"]);
    }

    #[test]
    fn parse_role_md_empty_skills_yields_empty_bundle() {
        let raw = "---\nname: Researcher\ndescription: Investigates.\nskills:\n---\n\nBody.\n";
        let (_, _, skills) = super::parse_role_md(raw).expect("should parse");
        assert!(skills.is_empty());
    }

    #[test]
    fn parse_role_md_missing_skills_line_yields_empty_bundle() {
        let raw = "---\nname: Researcher\ndescription: Investigates.\n---\n\nBody.\n";
        let (_, _, skills) = super::parse_role_md(raw).expect("should parse");
        assert!(skills.is_empty());
    }

    #[test]
    fn parse_role_md_rejects_missing_frontmatter() {
        assert!(super::parse_role_md("Just a body, no frontmatter.").is_none());
    }

    #[test]
    fn parse_role_md_rejects_missing_name() {
        let raw = "---\ndescription: no name here\n---\n\nBody.\n";
        assert!(super::parse_role_md(raw).is_none());
    }

    #[test]
    fn parse_role_md_rejects_missing_description() {
        // A role with no verbatim job description is malformed — it would bake
        // an empty paragraph into the preamble.
        let raw = "---\nname: Nameless Purpose\nskills: implementer\n---\n\nBody.\n";
        assert!(super::parse_role_md(raw).is_none());
    }

    #[test]
    fn read_builtin_roles_from_parses_one_role_per_subdir_skips_bad_ones() {
        let dir = std::env::temp_dir().join("conclave-role-test-read-builtin-roles");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("good")).expect("mkdir failed");
        std::fs::write(
            dir.join("good").join("ROLE.md"),
            "---\nname: Good One\ndescription: Works.\nskills: implementer\n---\n\nBody.\n",
        )
        .expect("write failed");
        // A subdir with no ROLE.md at all must be silently skipped.
        std::fs::create_dir_all(dir.join("empty-dir")).expect("mkdir failed");
        // A subdir with unparsable frontmatter must be silently skipped.
        std::fs::create_dir_all(dir.join("bad")).expect("mkdir failed");
        std::fs::write(dir.join("bad").join("ROLE.md"), "no frontmatter here")
            .expect("write failed");
        // A plain FILE directly under dir (not a subdir) must be ignored.
        std::fs::write(dir.join("stray.txt"), "ignore me").expect("write failed");

        let roles = super::read_builtin_roles_from(&dir);

        assert_eq!(
            roles.len(),
            1,
            "only the well-formed 'good' role should survive"
        );
        assert_eq!(roles[0].id, "good");
        assert_eq!(roles[0].name, "Good One");
        assert_eq!(roles[0].description, "Works.");
        assert_eq!(roles[0].skill_ids, vec!["implementer"]);
        assert_eq!(roles[0].kind, "builtin");

        std::fs::remove_dir_all(&dir).expect("cleanup failed");
    }

    #[test]
    fn read_builtin_roles_from_missing_dir_returns_empty() {
        let dir = std::env::temp_dir().join("conclave-role-test-does-not-exist-xyz");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(super::read_builtin_roles_from(&dir).is_empty());
    }

    #[test]
    fn list_builtin_reads_from_fixture_override() {
        let _fx = super::test_support::fixture_roles_dir("override-basic");
        let ids: Vec<String> = super::list_builtin().into_iter().map(|r| r.id).collect();
        assert_eq!(ids, vec!["fix-bare".to_string(), "fix-lead".to_string()]);
    }

    /// The ONLY test allowed to depend on shipped role content. Guards two
    /// invariants: every shipped `roles/*/ROLE.md` parses (a broken one would
    /// be silently skipped in production), and the five builtins exist with
    /// `lead` bundling `leadership`. Reads the source-tree path directly so a
    /// concurrent test's thread-local override can never leak in.
    #[test]
    fn shipped_roles_all_parse_and_lead_includes_leadership() {
        let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/roles"));
        let role_md_count = std::fs::read_dir(&dir)
            .expect("shipped roles dir must exist")
            .flatten()
            .filter(|e| e.path().join("ROLE.md").is_file())
            .count();
        let roles = super::read_builtin_roles_from(&dir);
        assert_eq!(
            roles.len(),
            role_md_count,
            "every shipped ROLE.md must parse — a failing one is silently dropped in production"
        );

        let ids: Vec<&str> = roles.iter().map(|r| r.id.as_str()).collect();
        for expected in ["lead", "implementer", "reviewer", "researcher", "designer"] {
            assert!(
                ids.contains(&expected),
                "builtin role '{expected}' must ship"
            );
        }

        let lead = roles.iter().find(|r| r.id == "lead").expect("lead ships");
        assert_eq!(lead.name, "Lead");
        assert_eq!(lead.kind, "builtin");
        assert!(
            !lead.description.is_empty(),
            "lead must carry a job description"
        );
        assert!(
            lead.skill_ids.contains(&"leadership".to_string()),
            "lead must bundle the leadership skill"
        );
        assert!(lead.skill_ids.contains(&"agent-loop".to_string()));
    }

    #[test]
    fn shipped_role_skill_ids_reference_real_skills() {
        // Every skill id a builtin role names must resolve to a shipped skill —
        // a typo'd id would silently attach nothing.
        let role_dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/roles"));
        let roles = super::read_builtin_roles_from(&role_dir);
        let skills = crate::engine::repo::skill::list_builtin();
        let skill_ids: std::collections::HashSet<String> =
            skills.into_iter().map(|s| s.id).collect();
        for role in &roles {
            for sid in &role.skill_ids {
                assert!(
                    skill_ids.contains(sid),
                    "role '{}' references unknown skill id '{}'",
                    role.id,
                    sid
                );
            }
        }
    }

    // ── Custom (DB) CRUD ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn create_then_get_roundtrip() {
        let pool = connect_in_memory().await;
        let row = super::create(
            &pool,
            "My Reviewer",
            "Reviews everything.",
            &["implementer".to_string()],
        )
        .await
        .expect("create failed");
        assert_eq!(row.name, "My Reviewer");
        assert_eq!(row.description, "Reviews everything.");
        assert_eq!(row.skill_ids, vec!["implementer"]);
        assert_eq!(row.kind, "custom");

        let fetched = super::get(&pool, &row.id)
            .await
            .expect("get failed")
            .expect("row should exist");
        assert_eq!(fetched, row);
    }

    #[tokio::test]
    async fn update_changes_fields() {
        let pool = connect_in_memory().await;
        let row = super::create(&pool, "Old", "old desc", &[])
            .await
            .expect("create failed");
        let updated = super::update(
            &pool,
            &row.id,
            "New",
            "new desc",
            &["leadership".to_string(), "agent-loop".to_string()],
        )
        .await
        .expect("update failed")
        .expect("row should exist after update");
        assert_eq!(updated.name, "New");
        assert_eq!(updated.description, "new desc");
        assert_eq!(updated.skill_ids, vec!["leadership", "agent-loop"]);
    }

    #[tokio::test]
    async fn update_unknown_id_returns_none() {
        let pool = connect_in_memory().await;
        let result = super::update(&pool, "no-such-id", "X", "Y", &[])
            .await
            .expect("update should not error");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn list_returns_only_custom_ordered_by_name() {
        let pool = connect_in_memory().await;
        super::create(&pool, "Zeta", "z", &[])
            .await
            .expect("create failed");
        super::create(&pool, "Alpha", "a", &[])
            .await
            .expect("create failed");

        let all = super::list(&pool).await.expect("list failed");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].name, "Alpha", "must be name-ordered");
        assert_eq!(all[1].name, "Zeta");
        assert!(all.iter().all(|r| r.kind == "custom"));
    }

    #[tokio::test]
    async fn delete_removes_then_second_is_noop() {
        let pool = connect_in_memory().await;
        let row = super::create(&pool, "Gone", "g", &[])
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
    async fn find_any_prefers_builtin_then_falls_back_to_db() {
        let _fx = super::test_support::fixture_roles_dir("find-any");
        let pool = connect_in_memory().await;
        // Builtin id resolves from the folder.
        let builtin = super::find_any(&pool, "fix-lead")
            .await
            .expect("find_any failed")
            .expect("builtin role should resolve");
        assert_eq!(builtin.kind, "builtin");
        assert_eq!(builtin.skill_ids, vec!["alpha", "beta"]);

        // Custom id resolves from the DB.
        let custom = super::create(&pool, "Custom", "c", &[])
            .await
            .expect("create failed");
        let found = super::find_any(&pool, &custom.id)
            .await
            .expect("find_any failed")
            .expect("custom role should resolve");
        assert_eq!(found.kind, "custom");

        // Unknown id → None.
        assert!(super::find_any(&pool, "no-such-role")
            .await
            .expect("find_any failed")
            .is_none());
    }

    /// JSON must use camelCase keys (`skillIds`, not `skill_ids`).
    #[tokio::test]
    async fn camel_case_contract() {
        let pool = connect_in_memory().await;
        let row = super::create(&pool, "Sol", "desc", &["implementer".to_string()])
            .await
            .expect("create failed");
        let json = serde_json::to_value(&row).expect("serialize failed");
        assert!(json.get("skillIds").is_some(), "must have skillIds");
        assert!(json.get("skill_ids").is_none(), "must NOT have skill_ids");
        assert!(json.get("kind").is_some());
        assert_eq!(json["description"], "desc");
    }
}
