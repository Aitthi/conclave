# Skill Editor: Full-Panel + Agent-Assisted Writing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the custom-skill editor's small centered modal with a full-panel editor using a real code editor widget, and add an "Ask agent to help" panel that spawns one of the user's own CLI `AgentDefinition`s against the skill's scratch `SKILL.md`, syncing its edits back into the editor.

**Architecture:** Reuse the existing session/PTY/streaming machinery end-to-end via a HIDDEN, single-purpose `Workspace` pointed at a scratch directory — `instance::spawn`, `instance::stop`, `message::send`, `workspace::delete`, `session:output`/`session:status` events, and `parse_skill_md` are all reused unmodified. See `docs/specs/2026-07-02-skill-editor-agent-assist-design.md` for the approved design.

**Tech Stack:** Tauri v2, Rust (sqlx + chain-builder), React 19 + TypeScript strict, `@uiw/react-codemirror`.

## Global Constraints

- Builtin (system) skills are completely untouched by this plan — no file in `src-tauri/skills/`, `repo/skill.rs`'s builtin-reading functions, or the mandatory/optional mechanism (ADR 0002/0003) changes.
- New migration is `0007` (current latest is `0006_selected_builtin_skills.sql`). Every `if version < N { … }` gate in `db.rs::migrate` and every `assert_eq!(version, 6, ...)` in its tests that asserts "fully migrated" must become `7` — grep for all three before considering Task 1 done.
- `repo::workspace::create`'s existing 4-arg public signature and behavior (`hidden` always `false`) MUST NOT change — there are 24 existing call sites across the crate (mostly test fixtures in unrelated files: `fusion.rs`, `blackboard.rs`, `snapshot.rs`, `session.rs`, `inter_agent_message.rs`, etc.) that must not need touching. Add a NEW `create_hidden` function instead; share the INSERT logic via a private helper.
- No new Tauri event / `bus.rs` type. The frontend re-uses the EXISTING `session:status` event (`useSessionStatus`, already exported from `../ipc`) to detect when to re-sync a draft session's scratch file — do not add a `skill:draft-synced` push event.
- A skill-draft agent-assist session's `AgentDefinition` MUST have `type == "cli"` and `cliKind` exactly `"claude-code"` or `"codex"` — matches the only two branches `instance::spawn`'s `cli` dispatch actually launches. Reject anything else with `AppError::Invalid` BEFORE creating any scratch resources.
- Do not write a unit test that spawns a real `claude`/`codex` process — this repeats the exact "binary-free" boundary `commands/instance.rs`'s own `fixture_instance` doc comment already documents (search that file for "binary-free" before writing Task 3's tests). Test the validation-rejection paths and the sync/stop logic against hand-built fixtures instead of going through a real `start()` → `instance::spawn` → PTY spawn path.
- Frontend verification in every frontend task is `pnpm exec tsc --noEmit` (strict) and `pnpm build` — this codebase has no frontend unit test runner configured. Do not add one; it is out of scope.
- All new user-facing UI copy is English (existing project convention — see `CONTEXT.md`).
- Rust formatting/lint gates: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` must stay clean after every task, same as prior arcs in this codebase.

---

### Task 1: Migration 0007 — `workspace.hidden` column + repo support

**Files:**
- Create: `src-tauri/src/engine/migrations/0007_workspace_hidden.sql`
- Modify: `src-tauri/src/engine/db.rs` (migration gate + 3 version assertions)
- Modify: `src-tauri/src/engine/repo/workspace.rs` (`WorkspaceRow.hidden`, `list`/`get` column lists, `create`/`create_hidden` refactor)

**Interfaces:**
- Produces: `repo::workspace::create_hidden(pool, name, folder_path) -> sqlx::Result<WorkspaceRow>` — a hidden (`hidden: true`, `color: None`) workspace, excluded from `repo::workspace::list`. Later tasks (Task 3) depend on this exact signature.
- Produces: `WorkspaceRow.hidden: bool` (not serialized — `#[serde(skip)]`).

- [ ] **Step 1: Write the migration file**

```sql
-- Create: src-tauri/src/engine/migrations/0007_workspace_hidden.sql
-- A hidden ephemeral workspace backs an agent-assisted skill-draft session
-- (see docs/specs/2026-07-02-skill-editor-agent-assist-design.md). It is a
-- completely normal `workspace` row otherwise — every existing spawn/session
-- code path works against it unmodified — it is simply excluded from
-- `workspace.list` so it never appears in the normal workspace switcher.
ALTER TABLE workspace ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0;
```

- [ ] **Step 2: Wire the migration into `db.rs::migrate`**

In `src-tauri/src/engine/db.rs`, immediately after the existing `if version < 6 { … }` block (ends around line 123), add:

```rust
    if version < 7 {
        sqlx::raw_sql(include_str!("migrations/0007_workspace_hidden.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("PRAGMA user_version = 7;")
            .execute(&mut *tx)
            .await?;
    }
```

- [ ] **Step 3: Bump the 3 existing "fully migrated" version assertions from 6 to 7**

In `src-tauri/src/engine/db.rs`'s `#[cfg(test)] mod tests`, change:
- `migrate_is_idempotent`: `assert_eq!(version, 6, "user_version should be 6");` → `assert_eq!(version, 7, "user_version should be 7");`
- `migrate_adds_skill_system_columns`: `assert_eq!(version, 6);` → `assert_eq!(version, 7);`
- `migrate_adds_selected_builtin_skill_ids_column`: `assert_eq!(version, 6);` → `assert_eq!(version, 7);`

Leave every `assert_eq!(count, 19, ...)` table-count assertion UNCHANGED — this migration adds a column, not a table.

- [ ] **Step 4: Run the full db.rs test module to verify the migration lands cleanly**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::db::tests`
Expected: all pass, including the 3 just-updated assertions.

- [ ] **Step 5: Add `hidden` to `WorkspaceRow` and thread it through `list`/`get`**

In `src-tauri/src/engine/repo/workspace.rs`, change the struct:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRow {
    pub id: String,
    pub name: String,
    pub folder_path: String, // serializes to "folderPath"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// `true` for a single-purpose scratch workspace backing a skill-draft
    /// agent-assist session (see
    /// docs/specs/2026-07-02-skill-editor-agent-assist-design.md). Excluded
    /// from `list()` so it never appears in the normal workspace switcher;
    /// every other repo function treats it exactly like a normal workspace.
    /// Never serialized — the frontend `Workspace` TS type has no `hidden`
    /// field and nothing outside this module needs to know a row is hidden.
    #[serde(skip)]
    pub hidden: bool,
    pub created_at: String, // serializes to "createdAt"
}
```

Change `list` and `get` to select the new column (both need it — `sqlx::FromRow`'s derive fetches every struct field by column name):

```rust
pub async fn list(pool: &SqlitePool) -> sqlx::Result<Vec<WorkspaceRow>> {
    let rows = QueryBuilder::<Sqlite>::table("workspace")
        .select(["id", "name", "folder_path", "color", "hidden", "created_at"])
        .order_by("created_at", Order::Asc)
        .order_by("id", Order::Asc)
        .fetch_all::<WorkspaceRow, _>(pool)
        .await
        .map_err(cb_err)?;
    // Filtered in Rust rather than a chain-builder `.where_eq("hidden", ...)`
    // predicate — this table is always small (dozens of rows at most), and
    // filtering here avoids depending on chain-builder's bool-bind behavior
    // in a WHERE clause, which nothing else in this codebase exercises yet.
    Ok(rows.into_iter().filter(|w| !w.hidden).collect())
}

pub async fn get(pool: &SqlitePool, id: &str) -> sqlx::Result<Option<WorkspaceRow>> {
    QueryBuilder::<Sqlite>::table("workspace")
        .select(["id", "name", "folder_path", "color", "hidden", "created_at"])
        .where_eq("id", id)
        .fetch_optional::<WorkspaceRow, _>(pool)
        .await
        .map_err(cb_err)
}
```

- [ ] **Step 6: Refactor `create` to share an `insert_row` helper, and add `create_hidden`**

Replace the existing `create` function body with:

```rust
/// Insert a new (non-hidden) workspace and return the constructed row.
///
/// Generates a UUID v4 `id` and ISO-8601 UTC `created_at` timestamp.
pub async fn create(
    pool: &SqlitePool,
    name: &str,
    folder_path: &str,
    color: Option<&str>,
) -> sqlx::Result<WorkspaceRow> {
    insert_row(pool, name, folder_path, color, false).await
}

/// Create a HIDDEN, single-purpose workspace — used only to back an
/// agent-assist skill-draft session (see
/// docs/specs/2026-07-02-skill-editor-agent-assist-design.md). Always
/// `color: None`. Excluded from `list()`; every other repo function treats
/// it exactly like a normal workspace (in particular, `instance::spawn`'s
/// prerequisites — workspace_agent → session → agent_definition → workspace
/// — are satisfied without any change to `instance::spawn` itself).
pub async fn create_hidden(
    pool: &SqlitePool,
    name: &str,
    folder_path: &str,
) -> sqlx::Result<WorkspaceRow> {
    insert_row(pool, name, folder_path, None, true).await
}

async fn insert_row(
    pool: &SqlitePool,
    name: &str,
    folder_path: &str,
    color: Option<&str>,
    hidden: bool,
) -> sqlx::Result<WorkspaceRow> {
    let id = Uuid::new_v4().to_string();
    let created_at = Utc::now().to_rfc3339();
    let name = name.to_owned();
    let folder_path = folder_path.to_owned();
    let color = color.map(str::to_owned);

    QueryBuilder::<Sqlite>::table("workspace")
        .insert([
            ("id", Bind::Text(id.clone())),
            ("name", Bind::Text(name.clone())),
            ("folder_path", Bind::Text(folder_path.clone())),
            ("color", color.clone().map(Bind::Text).unwrap_or(Bind::Null)),
            ("hidden", Bind::Bool(hidden)),
            ("created_at", Bind::Text(created_at.clone())),
        ])
        .execute(pool)
        .await
        .map_err(cb_err)?;

    Ok(WorkspaceRow {
        id,
        name,
        folder_path,
        color,
        hidden,
        created_at,
    })
}
```

- [ ] **Step 7: Add tests to `repo/workspace.rs`'s existing `#[cfg(test)] mod tests`**

```rust
    /// A hidden workspace round-trips through get() with hidden state intact,
    /// but is completely excluded from list().
    #[tokio::test]
    async fn create_hidden_excluded_from_list_but_gettable() {
        let pool = connect_in_memory().await;
        create(&pool, "Visible", "/tmp/visible", None)
            .await
            .expect("create failed");
        let hidden = create_hidden(&pool, "Scratch", "/tmp/scratch")
            .await
            .expect("create_hidden failed");
        assert!(hidden.hidden);
        assert!(hidden.color.is_none());

        let listed = list(&pool).await.expect("list failed");
        assert_eq!(listed.len(), 1, "only the visible workspace must be listed");
        assert_eq!(listed[0].name, "Visible");

        let fetched = get(&pool, &hidden.id)
            .await
            .expect("get failed")
            .expect("hidden row must still be gettable by id");
        assert_eq!(fetched, hidden);
    }

    /// A normal create() always yields hidden: false.
    #[tokio::test]
    async fn create_is_never_hidden() {
        let pool = connect_in_memory().await;
        let row = create(&pool, "Normal", "/tmp/n", None)
            .await
            .expect("create failed");
        assert!(!row.hidden);
    }

    /// `hidden` must never appear in serialized JSON (skip, not
    /// skip_serializing_if — there is no frontend-visible state for it).
    #[tokio::test]
    async fn hidden_field_is_never_serialized() {
        let pool = connect_in_memory().await;
        let row = create_hidden(&pool, "Scratch", "/tmp/scratch2")
            .await
            .expect("create_hidden failed");
        let json = serde_json::to_value(&row).expect("serialize failed");
        assert!(json.get("hidden").is_none(), "hidden must never serialize");
    }
```

- [ ] **Step 8: Run the workspace repo tests, then the full crate test/fmt/clippy baseline**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::repo::workspace`
Expected: all pass (existing tests + 3 new ones).

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib && cargo fmt --manifest-path src-tauri/Cargo.toml --check && cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
Expected: full crate green (no test in any OTHER file should have broken — `create`'s public signature/behavior is unchanged).

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/engine/migrations/0007_workspace_hidden.sql src-tauri/src/engine/db.rs src-tauri/src/engine/repo/workspace.rs
git commit -m "feat(workspace): add hidden workspaces for skill-draft agent-assist sessions"
```

---

### Task 2: Skill-draft scratch file helpers (`repo/skill.rs`)

**Files:**
- Modify: `src-tauri/src/engine/repo/skill.rs`

**Interfaces:**
- Consumes: the private `fn parse_skill_md(raw: &str) -> Option<(String, Option<String>, String, bool)>` already in this file (Task 3's command layer never needs to touch it directly — only `read_draft` does, since it lives in the same module).
- Produces: `pub fn new_draft_dir() -> std::io::Result<std::path::PathBuf>`, `pub fn write_draft(dir: &Path, name: &str, description: Option<&str>, content: &str) -> std::io::Result<()>`, `pub fn read_draft(dir: &Path) -> Option<(String, Option<String>, String)>` — Task 3 depends on these exact signatures.

- [ ] **Step 1: Write the failing tests**

Add to `repo/skill.rs`'s existing `#[cfg(test)] mod tests` (near the other real-filesystem tests, e.g. next to `read_builtin_skills_from_parses_one_skill_per_subdir_skips_bad_ones`):

```rust
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
```

- [ ] **Step 2: Run to verify these fail to compile (the functions don't exist yet)**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::repo::skill::tests::write_draft`
Expected: compile error — `write_draft`/`read_draft`/`new_draft_dir` not found.

- [ ] **Step 3: Implement the three functions**

Add to `repo/skill.rs`, after `list_builtin`/`skills_dir`/`bundled_skills_dir` and before `content_for_agent`:

```rust
/// Allocate a fresh scratch directory for one skill-draft agent-assist
/// session (see docs/specs/2026-07-02-skill-editor-agent-assist-design.md),
/// under the same per-user Conclave data dir `agentctx::write_skill_sidecar`
/// uses, in its own `skill-drafts/<uuid>` subdirectory so concurrent
/// sessions never collide.
pub fn new_draft_dir() -> std::io::Result<std::path::PathBuf> {
    let dir = dirs::data_dir()
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "no user data directory")
        })?
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
pub fn read_draft(dir: &std::path::Path) -> Option<(String, Option<String>, String)> {
    let raw = std::fs::read_to_string(dir.join("SKILL.md")).ok()?;
    let (name, description, content, _mandatory) = parse_skill_md(&raw)?;
    Some((name, description, content))
}
```

- [ ] **Step 4: Run to verify the tests pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::repo::skill::tests -- --test-threads=1`
Expected: all pass, including the 4 new tests (single-threaded avoids any theoretical collision on the shared `std::env::temp_dir()` fixture paths — matches this file's existing real-filesystem tests' convention).

- [ ] **Step 5: Run the full crate baseline**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib && cargo fmt --manifest-path src-tauri/Cargo.toml --check && cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
Expected: green.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/engine/repo/skill.rs
git commit -m "feat(skill): add scratch SKILL.md write/read helpers for agent-assist drafts"
```

---

### Task 3: `commands/skill_draft.rs` — start / sync / stop handlers + router wiring

**Files:**
- Create: `src-tauri/src/engine/commands/skill_draft.rs`
- Modify: `src-tauri/src/engine/commands/mod.rs` (add `pub mod skill_draft;`)
- Modify: `src-tauri/src/engine/router.rs` (3 new dispatch arms)
- Modify: `src-tauri/src/engine/commands/instance.rs` (drop the now-stale `#[allow(dead_code)]` on `stop` — it gets a real caller in this task)

**Interfaces:**
- Consumes: `repo::workspace::create_hidden` (Task 1), `repo::skill::new_draft_dir`/`write_draft`/`read_draft` (Task 2), `repo::workspace_agent::instantiate` (existing), `commands::instance::spawn`/`stop` (existing, called as plain function calls — not through the JSON router), `commands::workspace::delete` (existing).
- Produces: `pub async fn start(state, payload) -> Result<Value, AppError>` → `{ workspaceAgentId, sessionId }`; `pub async fn sync(state, payload) -> Result<Value, AppError>` → `{ name, description, content }`; `pub async fn stop(state, payload) -> Result<Value, AppError>` → `null`. Task 4 (frontend IPC types) depends on these exact request/response shapes.

- [ ] **Step 1: Create the command module**

```rust
// Create: src-tauri/src/engine/commands/skill_draft.rs
//! Skill-draft agent-assist sessions — spawn a real CLI agent (reusing the
//! existing workspace/session/instance machinery via a HIDDEN, single-purpose
//! `Workspace`) against a scratch `SKILL.md` so it can write a custom skill's
//! name/description/content directly. See
//! docs/specs/2026-07-02-skill-editor-agent-assist-design.md.

use crate::engine::{repo, AppError, AppState};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartDraftReq {
    name: String,
    description: Option<String>,
    content: String,
    agent_def_id: String,
}

/// Start an agent-assist draft session: materialize the skill's current
/// fields as a scratch `SKILL.md`, create a hidden `Workspace` +
/// `workspace_agent` + `session` pointed at it, and spawn the chosen
/// `AgentDefinition` exactly as a real workspace would (reuses
/// `instance::spawn` unmodified). Maps to `skill.startDraftSession`.
///
/// Rejects a non-`cli` or unconfigured/unsupported-`cli_kind` agent
/// definition BEFORE creating any resources — `instance::spawn`'s `chat` and
/// `orchestrator` branches don't give the agent real file tool access, and
/// only `claude-code`/`codex` are launchable `cli_kind`s today (see
/// `instance::spawn`'s own dispatch) — failing fast here avoids leaving an
/// orphaned hidden workspace + scratch dir behind.
pub async fn start(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: StartDraftReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    let def = repo::agent_definition::get(&state.db, &req.agent_def_id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "agent_definition id={} not found",
                req.agent_def_id
            ))
        })?;
    if def.r#type != "cli" || !matches!(def.cli_kind.as_deref(), Some("claude-code") | Some("codex"))
    {
        return Err(AppError::Invalid(
            "skill-assist agent must be a configured CLI agent (Claude Code or Codex)".into(),
        ));
    }

    let dir = repo::skill::new_draft_dir()
        .map_err(|e| AppError::Internal(format!("create skill draft scratch dir: {e}")))?;
    repo::skill::write_draft(&dir, &req.name, req.description.as_deref(), &req.content)
        .map_err(|e| AppError::Internal(format!("write skill draft: {e}")))?;

    let ws = repo::workspace::create_hidden(&state.db, &req.name, &dir.to_string_lossy()).await?;
    let wa = repo::workspace_agent::instantiate(&state.db, &ws.id, &req.agent_def_id).await?;

    match super::instance::spawn(state, json!({ "workspaceAgentId": wa.id })).await {
        Ok(session) => {
            let session_id = session
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::Internal("spawn returned no session id".into()))?
                .to_owned();
            Ok(json!({ "workspaceAgentId": wa.id, "sessionId": session_id }))
        }
        Err(e) => {
            // Best-effort rollback: a doomed spawn must not leave a hidden
            // workspace + scratch dir behind for the user to never see again.
            let _ = super::workspace::delete(state, json!({ "workspaceId": ws.id })).await;
            let _ = std::fs::remove_dir_all(&dir);
            Err(e)
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DraftSessionReq {
    workspace_agent_id: String,
}

/// Re-read + parse the draft session's scratch `SKILL.md`, returning the
/// latest `{name, description, content}`. Maps to `skill.syncDraft`.
///
/// If the file is currently missing or unparsable (e.g. the agent is
/// mid-write), returns `AppError::Invalid` — the FRONTEND simply keeps
/// showing the last successfully synced values on a failed sync (it only
/// applies a sync's result when the call succeeds), so no "last good" state
/// needs to be tracked here.
pub async fn sync(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: DraftSessionReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    let wa = repo::workspace_agent::get(&state.db, &req.workspace_agent_id)
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "workspace_agent id={} not found",
                req.workspace_agent_id
            ))
        })?;
    let ws = repo::workspace::get(&state.db, &wa.workspace_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("workspace id={} not found", wa.workspace_id)))?;

    let (name, description, content) =
        repo::skill::read_draft(std::path::Path::new(&ws.folder_path)).ok_or_else(|| {
            AppError::Invalid("skill draft file is missing or unparsable right now".into())
        })?;

    // `description` is OMITTED (not `null`) when absent, matching this
    // codebase's `skip_serializing_if = "Option::is_none"` convention
    // elsewhere (e.g. `SkillRow.description`) and the frontend's `description?:
    // string` (not `string | null`) contract exactly.
    let mut res = json!({ "name": name, "content": content });
    if let Some(d) = description {
        res["description"] = json!(d);
    }
    Ok(res)
}

/// Stop a draft session's live agent, tear down its hidden workspace (and
/// everything that hangs off it, via `workspace::delete`), and remove the
/// scratch directory from disk. Maps to `skill.stopDraftSession`.
///
/// Idempotent: an unknown `workspaceAgentId` (already cleaned up) is a no-op,
/// matching `instance::stop`'s idempotent-on-not-live precedent — the
/// frontend calls this defensively on unmount, and an uncaught rejection
/// there shouldn't surface as an error to the user.
pub async fn stop(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req: DraftSessionReq =
        serde_json::from_value(payload).map_err(|e| AppError::Invalid(e.to_string()))?;

    let Some(wa) = repo::workspace_agent::get(&state.db, &req.workspace_agent_id).await? else {
        return Ok(Value::Null);
    };
    let folder_path = repo::workspace::get(&state.db, &wa.workspace_id)
        .await?
        .map(|ws| ws.folder_path);

    super::instance::stop(state, json!({ "workspaceAgentId": req.workspace_agent_id })).await?;
    super::workspace::delete(state, json!({ "workspaceId": wa.workspace_id })).await?;

    if let Some(path) = folder_path {
        let _ = std::fs::remove_dir_all(path);
    }

    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fixture_cli_agent_def(state: &AppState, cli_kind: Option<&str>) -> String {
        repo::agent_definition::create(
            &state.db,
            &repo::agent_definition::AgentDefinitionInput {
                name: "Assistant".into(),
                agent_type: "cli".into(),
                cli_kind: cli_kind.map(str::to_owned),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed")
        .id
    }

    /// Builds a hidden workspace + workspace_agent + session directly,
    /// bypassing `start`'s real CLI spawn (see this file's
    /// `start_rejects_*` tests and this plan's Global Constraints — spawning
    /// a real `claude`/`codex` process in a unit test repeats the exact
    /// "binary-free" boundary `commands::instance`'s own tests already
    /// respect). Returns `(workspace_agent_id, scratch_dir)`.
    async fn fixture_draft_session(
        state: &AppState,
        name: &str,
        description: Option<&str>,
        content: &str,
    ) -> (String, std::path::PathBuf) {
        let dir = repo::skill::new_draft_dir().expect("new_draft_dir failed");
        repo::skill::write_draft(&dir, name, description, content).expect("write_draft failed");

        let ws = repo::workspace::create_hidden(&state.db, name, &dir.to_string_lossy())
            .await
            .expect("create_hidden failed");
        let def_id = fixture_cli_agent_def(state, Some("claude-code")).await;
        let wa = repo::workspace_agent::instantiate(&state.db, &ws.id, &def_id)
            .await
            .expect("instantiate failed");
        (wa.id, dir)
    }

    #[tokio::test]
    async fn start_rejects_non_cli_agent_def() {
        let state = AppState::for_tests().await;
        let def = repo::agent_definition::create(
            &state.db,
            &repo::agent_definition::AgentDefinitionInput {
                name: "Orch".into(),
                agent_type: "orchestrator".into(),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed");

        let err = start(
            &state,
            json!({ "name": "X", "content": "c", "agentDefId": def.id }),
        )
        .await
        .expect_err("must reject a non-cli agent def");
        assert!(matches!(err, AppError::Invalid(_)));
    }

    #[tokio::test]
    async fn start_rejects_cli_agent_with_no_cli_kind() {
        let state = AppState::for_tests().await;
        let def_id = fixture_cli_agent_def(&state, None).await;

        let err = start(
            &state,
            json!({ "name": "X", "content": "c", "agentDefId": def_id }),
        )
        .await
        .expect_err("must reject a cli agent def with no cli_kind configured");
        assert!(matches!(err, AppError::Invalid(_)));
    }

    #[tokio::test]
    async fn start_rejects_unknown_agent_def() {
        let state = AppState::for_tests().await;
        let err = start(
            &state,
            json!({ "name": "X", "content": "c", "agentDefId": "no-such-def" }),
        )
        .await
        .expect_err("must reject an unknown agent_def id");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn sync_reads_back_the_current_scratch_file() {
        let state = AppState::for_tests().await;
        let (wa_id, dir) = fixture_draft_session(&state, "Reviewer", Some("desc"), "Body").await;

        let result = sync(&state, json!({ "workspaceAgentId": wa_id }))
            .await
            .expect("sync failed");
        assert_eq!(result["name"], "Reviewer");
        assert_eq!(result["description"], "desc");
        assert_eq!(result["content"], "Body");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn sync_reflects_a_file_edited_after_start() {
        let state = AppState::for_tests().await;
        let (wa_id, dir) = fixture_draft_session(&state, "Reviewer", None, "Original").await;

        // Simulate the agent having edited the scratch file.
        repo::skill::write_draft(
            &dir,
            "Reviewer v2",
            Some("now with a description"),
            "Updated body",
        )
        .expect("simulated agent edit failed");

        let result = sync(&state, json!({ "workspaceAgentId": wa_id }))
            .await
            .expect("sync failed");
        assert_eq!(result["name"], "Reviewer v2");
        assert_eq!(result["content"], "Updated body");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn sync_unknown_workspace_agent_not_found() {
        let state = AppState::for_tests().await;
        let err = sync(&state, json!({ "workspaceAgentId": "nope" }))
            .await
            .expect_err("should fail for unknown id");
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn stop_deletes_hidden_workspace_and_scratch_dir() {
        let state = AppState::for_tests().await;
        let (wa_id, dir) = fixture_draft_session(&state, "Doomed", None, "c").await;
        assert!(dir.is_dir());

        stop(&state, json!({ "workspaceAgentId": wa_id }))
            .await
            .expect("stop failed");

        assert!(
            !repo::workspace_agent::exists(&state.db, &wa_id)
                .await
                .expect("exists check failed"),
            "workspace_agent must be gone"
        );
        assert!(!dir.exists(), "scratch dir must be removed");
    }

    #[tokio::test]
    async fn stop_unknown_workspace_agent_is_idempotent_noop() {
        let state = AppState::for_tests().await;
        let result = stop(&state, json!({ "workspaceAgentId": "nope" })).await;
        assert_eq!(result.expect("must not error"), Value::Null);
    }

    #[tokio::test]
    async fn hidden_draft_workspace_never_appears_in_workspace_list() {
        let state = AppState::for_tests().await;
        let (_wa_id, dir) = fixture_draft_session(&state, "Invisible", None, "c").await;

        let listed = repo::workspace::list(&state.db).await.expect("list failed");
        assert!(
            listed.is_empty(),
            "a hidden draft-session workspace must never appear in workspace.list"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
```

- [ ] **Step 2: Register the module**

In `src-tauri/src/engine/commands/mod.rs`, add (alphabetical, matching the existing order):

```rust
pub mod skill_draft;
```
(immediately after `pub mod skill;`)

- [ ] **Step 3: Wire the router**

In `src-tauri/src/engine/router.rs`:
- Add `skill_draft` to the `use crate::engine::commands::{...}` import list (alphabetical).
- Add, in the `// ── skill ──` section, right after the existing `"skill.delete"` arm:

```rust
        "skill.startDraftSession" => skill_draft::start(state, payload).await,
        "skill.syncDraft" => skill_draft::sync(state, payload).await,
        "skill.stopDraftSession" => skill_draft::stop(state, payload).await,
```

- [ ] **Step 4: Drop the now-stale `#[allow(dead_code)]` on `instance::stop`**

In `src-tauri/src/engine/commands/instance.rs`, `stop`'s doc comment currently ends with:

```rust
/// `#[allow(dead_code)]`: routed in a later milestone — UI stop button /
/// app teardown.
#[allow(dead_code)]
pub async fn stop(state: &AppState, payload: Value) -> Result<Value, AppError> {
```

Remove the `#[allow(dead_code)]` attribute line only (leave the doc comment's prose as-is — `stop` now has a real caller in `skill_draft::stop`, so the attribute would trip clippy's `unfulfilled_lint_expectations`-style dead-code-allow lints going forward if left in place once nothing marks it dead).

- [ ] **Step 5: Run this task's tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib engine::commands::skill_draft`
Expected: all 9 tests pass.

- [ ] **Step 6: Run the full crate baseline**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib && cargo fmt --manifest-path src-tauri/Cargo.toml --check && cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
Expected: green. In particular, confirm removing `#[allow(dead_code)]` on `instance::stop` does not trigger a clippy warning (it shouldn't — it now has a real caller).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/engine/commands/skill_draft.rs src-tauri/src/engine/commands/mod.rs src-tauri/src/engine/router.rs src-tauri/src/engine/commands/instance.rs
git commit -m "feat(skill): add skill.startDraftSession/syncDraft/stopDraftSession commands"
```

---

### Task 4: Frontend IPC types for the 3 new commands

**Files:**
- Modify: `src/ipc/commands.ts`

**Interfaces:**
- Consumes: Task 3's exact request/response shapes.
- Produces: `ipc.skill.startDraftSession`, `ipc.skill.syncDraft`, `ipc.skill.stopDraftSession`. Tasks 7–8 (frontend components) depend on these exact call signatures.

- [ ] **Step 1: Add the 3 command entries to the `Commands` interface**

In `src/ipc/commands.ts`, inside the `Commands` interface, immediately after the existing `"skill.delete"` entry:

```typescript
  "skill.startDraftSession": {
    req: { name: string; description?: string; content: string; agentDefId: string };
    res: { workspaceAgentId: string; sessionId: string };
  };
  "skill.syncDraft": {
    req: { workspaceAgentId: string };
    res: { name: string; description?: string; content: string };
  };
  "skill.stopDraftSession": {
    req: { workspaceAgentId: string };
    res: void;
  };
```

- [ ] **Step 2: Add the wrapper methods**

In `src/ipc/commands.ts`, inside the `ipc.skill` object, immediately after the existing `delete` entry:

```typescript
    startDraftSession: (req: Commands["skill.startDraftSession"]["req"]) =>
      call("skill.startDraftSession", req),
    syncDraft: (req: Commands["skill.syncDraft"]["req"]) => call("skill.syncDraft", req),
    stopDraftSession: (req: Commands["skill.stopDraftSession"]["req"]) =>
      call("skill.stopDraftSession", req),
```

- [ ] **Step 3: Verify**

Run: `pnpm exec tsc --noEmit`
Expected: clean (no other file references these yet, so this is a pure additive type-check).

- [ ] **Step 4: Commit**

```bash
git add src/ipc/commands.ts
git commit -m "feat(ipc): add skill.startDraftSession/syncDraft/stopDraftSession types"
```

---

### Task 5: Add the CodeMirror dependency

**Files:**
- Modify: `package.json`, `pnpm-lock.yaml`

- [ ] **Step 1: Install**

Run: `pnpm add @uiw/react-codemirror @codemirror/lang-markdown`

- [ ] **Step 2: Verify it resolves cleanly against React 19 and the existing strict TS config**

Run: `pnpm exec tsc --noEmit`
Expected: clean. If `@uiw/react-codemirror`'s peer dependency range rejects React 19 (`pnpm add` warns or fails), retry with `pnpm add @uiw/react-codemirror @codemirror/lang-markdown --force` and note the peer-range mismatch in the commit message; do not downgrade React to satisfy it.

Run: `pnpm build`
Expected: succeeds.

- [ ] **Step 3: Commit**

```bash
git add package.json pnpm-lock.yaml
git commit -m "chore: add @uiw/react-codemirror + @codemirror/lang-markdown"
```

---

### Task 6: `SkillEditor.tsx` — full-panel layout + CodeMirror content field

No assist panel yet — this task only changes the editor's chrome (small centered modal → full-viewport panel) and swaps the `content` `<textarea>` for CodeMirror. `name`/`description`/`content` state, `handleSave`, and the component's public props (`onClose`, `onSaved`, `initialSkill`) are unchanged.

**Files:**
- Modify: `src/components/SkillEditor.tsx`

- [ ] **Step 1: Replace the file**

```tsx
import { useState } from "react";
import { X, Wand2 } from "lucide-react";
import CodeMirror from "@uiw/react-codemirror";
import { markdown } from "@codemirror/lang-markdown";
import { ipc } from "../ipc";
import type { Skill } from "../ipc";

export interface SkillEditorProps {
  onClose: () => void;
  onSaved: (skill: Skill) => void;
  /** Pre-fill the form for editing an existing CUSTOM skill. Never a builtin
   *  one — the Library never opens the editor for builtin cards. */
  initialSkill?: Skill;
}

/**
 * Create or edit a CUSTOM skill: name, short description (shown in Library
 * lists), and the full markdown `content` injected into a cli agent's skill
 * sidecar file at launch (see docs/adr/0001-skill-system-v1.md). Builtin
 * skills are never edited here. Full-panel (not a small modal) so there's
 * room for a real code editor and, alongside it, an agent-assist panel (see
 * docs/specs/2026-07-02-skill-editor-agent-assist-design.md).
 */
export function SkillEditor({ onClose, onSaved, initialSkill }: SkillEditorProps) {
  const isEditing = initialSkill !== undefined;
  const [name, setName] = useState(initialSkill?.name ?? "");
  const [description, setDescription] = useState(initialSkill?.description ?? "");
  const [content, setContent] = useState(initialSkill?.content ?? "");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSave() {
    if (!name.trim()) {
      setError("Name is required");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const skill = await ipc.skill.save({
        id: initialSkill?.id,
        name: name.trim(),
        description: description.trim() || undefined,
        content,
      });
      onSaved(skill);
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex flex-col bg-surface">
      <div className="h-11 flex items-center justify-between px-4 border-b border-overlay/[0.06] shrink-0">
        <div className="flex items-center gap-2">
          <Wand2 className="w-4 h-4 text-accent" />
          <span className="text-[13px] font-semibold tracking-tight">
            {isEditing ? "Edit skill" : "New skill"}
          </span>
        </div>
        <button
          onClick={onClose}
          disabled={saving}
          className="w-7 h-7 grid place-items-center rounded-md hover:bg-overlay/[0.05] text-text-secondary disabled:opacity-50"
          aria-label="Close"
        >
          <X className="w-[15px] h-[15px]" />
        </button>
      </div>

      <div className="flex-1 flex min-h-0">
        <div className="flex-1 flex flex-col min-w-0 p-5 overflow-y-auto scroll-thin">
          <div className="mb-4">
            <div className="text-[11px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
              Name
            </div>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Code Reviewer"
              className="w-full rounded-lg ring-1 ring-overlay/[0.10] bg-fill-softer px-3 h-9 text-[13px] outline-none focus:ring-accent/50"
            />
          </div>

          <div className="mb-4">
            <div className="text-[11px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
              Description
            </div>
            <input
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="Shown in the Skill Library list"
              className="w-full rounded-lg ring-1 ring-overlay/[0.10] bg-fill-softer px-3 h-9 text-[13px] outline-none focus:ring-accent/50"
            />
          </div>

          <div className="flex-1 min-h-0 flex flex-col">
            <div className="text-[11px] font-bold tracking-wider text-text-tertiary uppercase mb-1.5">
              Content
            </div>
            <div className="flex-1 min-h-0 rounded-lg ring-1 ring-overlay/[0.10] overflow-hidden">
              <CodeMirror
                value={content}
                onChange={(value) => setContent(value)}
                extensions={[markdown()]}
                height="100%"
                className="h-full text-[12.5px]"
              />
            </div>
          </div>

          {error && <p className="text-[12px] text-danger mt-3">{error}</p>}
        </div>
      </div>

      <div className="border-t border-overlay/[0.07] px-5 py-3 bg-surface shrink-0 flex items-center gap-2">
        <button
          onClick={onClose}
          disabled={saving}
          className="flex-1 text-[12.5px] font-medium text-text-secondary bg-surface ring-1 ring-overlay/[0.08] rounded-lg py-2.5 hover:bg-overlay/[0.02] disabled:opacity-50"
        >
          Cancel
        </button>
        <button
          onClick={handleSave}
          disabled={saving}
          className="flex-[1.4] text-[12.5px] font-semibold text-white bg-accent rounded-lg py-2.5 hover:brightness-105 disabled:opacity-50"
        >
          {saving ? "Saving…" : "Save skill"}
        </button>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Verify**

Run: `pnpm exec tsc --noEmit && pnpm build`
Expected: clean. `SkillLibrary.tsx` needs no change — it still renders `<SkillEditor initialSkill={...} onClose={...} onSaved={...} />` exactly as before; the component now happens to render full-viewport instead of a centered card, and since it's `position: fixed`, this works regardless of it staying nested inside `SkillLibrary`'s JSX tree.

- [ ] **Step 3: Commit**

```bash
git add src/components/SkillEditor.tsx
git commit -m "feat(skill-editor): full-panel layout with a CodeMirror content editor"
```

---

### Task 7: `SkillAssistPanel.tsx` — new component (not wired in yet)

Standalone component: agent picker, start/stop, streamed output, message input, sync. Not imported anywhere yet (Task 8 wires it into `SkillEditor`), so this task is independently reviewable and still passes `tsc`/`build` (an unused-but-exported component is not a TS error).

**Files:**
- Create: `src/components/SkillAssistPanel.tsx`

**Interfaces:**
- Consumes: `ipc.agentDef.list`, `ipc.skill.startDraftSession/syncDraft/stopDraftSession` (Task 4), `ipc.message.send`, `useSessionOutput`/`useSessionStatus` (existing, from `../ipc`).
- Produces: `export interface DraftSession { workspaceAgentId: string; sessionId: string }` and `export function SkillAssistPanel(props: SkillAssistPanelProps)`. Task 8 depends on this exact prop shape.

- [ ] **Step 1: Write the component**

```tsx
// Create: src/components/SkillAssistPanel.tsx
import { useEffect, useState } from "react";
import { Play, Square, RefreshCw } from "lucide-react";
import { ipc } from "../ipc";
import { useSessionOutput, useSessionStatus } from "../ipc";
import type { AgentDefinition } from "../ipc";

export interface DraftSession {
  workspaceAgentId: string;
  sessionId: string;
}

export interface SkillAssistPanelProps {
  name: string;
  description: string;
  content: string;
  /** Non-null while a session is active — owned by the parent editor so it
   *  can lock its fields for the duration. */
  draft: DraftSession | null;
  onStarted: (draft: DraftSession) => void;
  onSynced: (v: { name: string; description?: string; content: string }) => void;
  onStopped: () => void;
}

/**
 * "Ask agent to help" panel for the skill editor: pick one of the user's own
 * CLI `AgentDefinition`s, start a real agent-assist session against the
 * skill's scratch file (see docs/specs/2026-07-02-skill-editor-agent-assist-design.md),
 * chat with it, and sync its edits back into the editor — either on the
 * session's next idle transition or via the manual "Sync now" button.
 */
export function SkillAssistPanel({
  name,
  description,
  content,
  draft,
  onStarted,
  onSynced,
  onStopped,
}: SkillAssistPanelProps) {
  const [agents, setAgents] = useState<AgentDefinition[]>([]);
  const [agentDefId, setAgentDefId] = useState<string>("");
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lines, setLines] = useState<string[]>([]);
  const [draftText, setDraftText] = useState("");
  const [sending, setSending] = useState(false);

  useEffect(() => {
    ipc.agentDef
      .list()
      .then((defs) => {
        const cliDefs = defs.filter(
          (d) => d.type === "cli" && (d.cliKind === "claude-code" || d.cliKind === "codex"),
        );
        setAgents(cliDefs);
        setAgentDefId((prev) => prev || (cliDefs[0]?.id ?? ""));
      })
      .catch(() => setAgents([]));
  }, []);

  async function handleStart() {
    if (!agentDefId) return;
    setStarting(true);
    setError(null);
    try {
      const res = await ipc.skill.startDraftSession({
        name: name.trim() || "Untitled skill",
        description: description.trim() || undefined,
        content,
        agentDefId,
      });
      setLines([]);
      onStarted(res);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setStarting(false);
    }
  }

  async function handleStop() {
    if (!draft) return;
    await ipc.skill.stopDraftSession({ workspaceAgentId: draft.workspaceAgentId }).catch(() => {});
    onStopped();
  }

  async function handleSync() {
    if (!draft) return;
    try {
      const v = await ipc.skill.syncDraft({ workspaceAgentId: draft.workspaceAgentId });
      onSynced(v);
    } catch {
      // Leave the editor's current fields untouched — a failed sync must not
      // destroy the last successfully synced state (see design spec).
    }
  }

  useSessionOutput(draft?.sessionId ?? "", (e) => {
    if (!draft) return;
    setLines((prev) => [...prev, e.chunk]);
  });

  useSessionStatus(draft?.sessionId ?? "", (e) => {
    if (!draft) return;
    if (e.status === "idle") void handleSync();
  });

  async function handleSend() {
    if (!draft || draftText.trim().length === 0) return;
    setSending(true);
    try {
      await ipc.message.send({ sessionId: draft.sessionId, text: draftText });
      setDraftText("");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSending(false);
    }
  }

  return (
    <div className="w-[360px] shrink-0 border-l border-overlay/[0.07] flex flex-col bg-sidebar">
      <div className="h-11 flex items-center px-3 border-b border-overlay/[0.06] shrink-0">
        <span className="text-[12.5px] font-semibold tracking-tight">Ask agent to help</span>
      </div>

      {!draft ? (
        <div className="p-3 space-y-2">
          <select
            value={agentDefId}
            onChange={(e) => setAgentDefId(e.target.value)}
            disabled={agents.length === 0}
            className="w-full rounded-lg ring-1 ring-overlay/[0.10] bg-fill-softer px-2.5 h-8 text-[12.5px] outline-none"
          >
            {agents.length === 0 ? (
              <option>No CLI agents configured</option>
            ) : (
              agents.map((a) => (
                <option key={a.id} value={a.id}>
                  {a.name}
                </option>
              ))
            )}
          </select>
          <button
            onClick={() => void handleStart()}
            disabled={starting || !agentDefId}
            className="w-full flex items-center justify-center gap-1.5 rounded-lg bg-accent text-white py-2 text-[12.5px] font-semibold disabled:opacity-50"
          >
            <Play className="w-3.5 h-3.5" />
            {starting ? "Starting…" : "Start"}
          </button>
          {error && <p className="text-[11.5px] text-danger">{error}</p>}
        </div>
      ) : (
        <>
          <div className="flex-1 overflow-y-auto scroll-thin px-3 py-2 text-[11.5px] font-mono whitespace-pre-wrap break-words">
            {lines.join("")}
          </div>
          <div className="border-t border-overlay/[0.06] p-2 space-y-1.5 shrink-0">
            <div className="flex items-center gap-1.5">
              <button
                onClick={() => void handleSync()}
                className="flex-1 flex items-center justify-center gap-1 text-[11px] font-medium text-text-secondary bg-surface ring-1 ring-overlay/[0.08] rounded-lg py-1.5 hover:bg-overlay/[0.02]"
              >
                <RefreshCw className="w-3 h-3" />
                Sync now
              </button>
              <button
                onClick={() => void handleStop()}
                className="flex-1 flex items-center justify-center gap-1 text-[11px] font-medium text-danger bg-danger/[0.06] rounded-lg py-1.5 hover:bg-danger/10"
              >
                <Square className="w-3 h-3" />
                Stop agent
              </button>
            </div>
            <textarea
              value={draftText}
              onChange={(e) => setDraftText(e.target.value)}
              disabled={sending}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  void handleSend();
                }
              }}
              rows={2}
              placeholder="Message the agent…"
              className="w-full rounded-lg ring-1 ring-overlay/[0.10] bg-fill-softer px-2.5 py-1.5 text-[12px] outline-none resize-none disabled:opacity-50"
            />
          </div>
        </>
      )}
    </div>
  );
}
```

- [ ] **Step 2: Verify**

Run: `pnpm exec tsc --noEmit && pnpm build`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src/components/SkillAssistPanel.tsx
git commit -m "feat(skill-editor): add standalone SkillAssistPanel component"
```

---

### Task 8: Wire `SkillAssistPanel` into `SkillEditor` — field lock, sync, cleanup

**Files:**
- Modify: `src/components/SkillEditor.tsx`

- [ ] **Step 1: Add the assist-session state, lock fields while active, render the panel, and clean up on unmount**

In `src/components/SkillEditor.tsx`:

Add imports:
```tsx
import { useEffect, useRef, useState } from "react";
import { ipc } from "../ipc";
import type { Skill } from "../ipc";
import { SkillAssistPanel, type DraftSession } from "./SkillAssistPanel";
```
(`useEffect`/`useRef` replace the current `useState`-only import line; `SkillAssistPanel` import is new.)

Inside the component, after the existing `error` state:

```tsx
  // Agent-assist session — non-null while the assist panel has a live
  // session. Locks the editor fields (single writer at a time — see design
  // spec's conflict-avoidance decision).
  const [draft, setDraft] = useState<DraftSession | null>(null);
  const draftRef = useRef(draft);
  useEffect(() => {
    draftRef.current = draft;
  }, [draft]);

  // Best-effort cleanup: if the editor unmounts (closed, or the app
  // navigates away) while a session is still active, stop it rather than
  // leaking a hidden workspace + live agent process.
  useEffect(() => {
    return () => {
      const active = draftRef.current;
      if (active) void ipc.skill.stopDraftSession({ workspaceAgentId: active.workspaceAgentId });
    };
  }, []);

  const locked = draft !== null;
```

Change the Name and Description `<input>`s to add `disabled={locked}` and `disabled:opacity-60` to their className (mirroring the existing `disabled:opacity-50` idiom used elsewhere in this file):

```tsx
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              disabled={locked}
              placeholder="e.g. Code Reviewer"
              className="w-full rounded-lg ring-1 ring-overlay/[0.10] bg-fill-softer px-3 h-9 text-[13px] outline-none focus:ring-accent/50 disabled:opacity-60"
            />
```
```tsx
            <input
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              disabled={locked}
              placeholder="Shown in the Skill Library list"
              className="w-full rounded-lg ring-1 ring-overlay/[0.10] bg-fill-softer px-3 h-9 text-[13px] outline-none focus:ring-accent/50 disabled:opacity-60"
            />
```

Add a lock banner right before the "Content" label block, and pass `editable={!locked}` to `CodeMirror`:

```tsx
          {locked && (
            <div className="mb-3 rounded-lg bg-accent/[0.08] text-accent text-[11.5px] px-3 py-2">
              Agent is editing — stop the session to edit manually.
            </div>
          )}

```
```tsx
              <CodeMirror
                value={content}
                onChange={(value) => setContent(value)}
                editable={!locked}
                extensions={[markdown()]}
                height="100%"
                className="h-full text-[12.5px]"
              />
```

Render `SkillAssistPanel` as a sibling of the existing fields column, inside the `<div className="flex-1 flex min-h-0">` wrapper, right after that column's closing `</div>`:

```tsx
        <SkillAssistPanel
          name={name}
          description={description}
          content={content}
          draft={draft}
          onStarted={setDraft}
          onSynced={(v) => {
            setName(v.name);
            setDescription(v.description ?? "");
            setContent(v.content);
          }}
          onStopped={() => setDraft(null)}
        />
```

Disable the "Save skill" button while locked (a save must not race the agent's own writes):

```tsx
        <button
          onClick={handleSave}
          disabled={saving || locked}
          className="flex-[1.4] text-[12.5px] font-semibold text-white bg-accent rounded-lg py-2.5 hover:brightness-105 disabled:opacity-50"
        >
          {saving ? "Saving…" : "Save skill"}
        </button>
```

- [ ] **Step 2: Verify**

Run: `pnpm exec tsc --noEmit && pnpm build`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src/components/SkillEditor.tsx
git commit -m "feat(skill-editor): wire the agent-assist panel into the editor"
```

---

## Final verification (after all 8 tasks, done by the orchestrator — not a subagent task)

Run, in order:
1. `cargo test --manifest-path src-tauri/Cargo.toml --lib` — full Rust suite green.
2. `cargo fmt --manifest-path src-tauri/Cargo.toml --check` — clean.
3. `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` — clean.
4. `pnpm exec tsc --noEmit` — clean.
5. `pnpm build` — succeeds.

Then dispatch the final whole-branch review per `superpowers:subagent-driven-development` (base = the commit before Task 1, head = the last commit above), on the most capable available model. Known, intentionally out-of-scope-for-automated-verification gaps to disclose regardless of review outcome: the actual `pnpm add`'d CodeMirror rendering, and the full spawn → chat → file-sync → stop happy path, both require a running `.app` to smoke-test manually (per this plan's Global Constraints on the binary-free test boundary) and were not run in this environment unless explicitly stated otherwise.
