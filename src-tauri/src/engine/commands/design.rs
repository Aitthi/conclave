//! `design.ensure` / `design.status` — the vendor-neutral bridge between a
//! Conclave workspace and the design-canvas host sidecar
//! (`runtime::design_host`). Scaffolds a workspace's `design/` on first use,
//! keeps the sidecar's shared `registry.json` in sync, and reports the
//! iframe URL the Design view embeds.
//!
//! Neither command touches the agent-facing file CONTRACT itself (writing
//! `design/screens/*.tsx` is any agent's ordinary Write/Edit tool) — this
//! module only ensures the host CAN show whatever is already on disk.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::engine::runtime::design_host;
use crate::engine::{repo, AppError, AppState};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceReq {
    workspace_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DesignInfo {
    project_id: String,
    running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
}

/// `<folder_path>/design` for a workspace — the canonical project dir both
/// `project_id_for` and the registry entry key off. Fully separate from
/// `.arta` (D1 — the Conclave-native design view never reads or writes it).
fn design_dir(folder_path: &str) -> PathBuf {
    Path::new(folder_path).join("design")
}

fn build_info(dir: &Path, host: Option<design_host::HostInfo>) -> DesignInfo {
    let project_id = design_host::project_id_for(dir);
    match host {
        Some(v) => DesignInfo {
            url: Some(format!("http://127.0.0.1:{}/?project={}", v.port, project_id)),
            port: Some(v.port),
            running: true,
            project_id,
        },
        None => DesignInfo {
            project_id,
            running: false,
            url: None,
            port: None,
        },
    }
}

/// `design.ensure { workspaceId }` → `{ url, port, projectId, running: true }`.
///
/// 1. Scaffold `design/` in the workspace's linked folder if missing.
/// 2. Upsert this workspace into the shared `registry.json` — BEFORE
///    starting the sidecar (load-bearing ordering, see
///    `runtime::design_host`'s module doc: the sidecar's file watcher can
///    miss the registry file's first-ever creation, never an update to an
///    already-existing one).
/// 3. Ensure the sidecar is running (spawns/respawns as needed).
pub async fn ensure(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req = serde_json::from_value::<WorkspaceReq>(payload)
        .map_err(|e| AppError::Invalid(e.to_string()))?;
    let ws = repo::workspace::get(&state.db, &req.workspace_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("workspace {}", req.workspace_id)))?;

    let dir = design_dir(&ws.folder_path);
    scaffold_if_missing(&dir).map_err(|e| AppError::Internal(format!("scaffold design/: {e}")))?;

    let project_id = design_host::project_id_for(&dir);
    upsert_registry(&dir, &ws.name, &project_id)
        .map_err(|e| AppError::Internal(format!("registering with design host: {e}")))?;

    let info = design_host::ensure_running()
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    serde_json::to_value(build_info(&dir, Some(info))).map_err(|e| AppError::Internal(e.to_string()))
}

/// `design.status { workspaceId }` → same shape as `ensure`, `running: bool`,
/// NO side effects (no scaffold, no registry write, no spawn).
pub async fn status(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req = serde_json::from_value::<WorkspaceReq>(payload)
        .map_err(|e| AppError::Invalid(e.to_string()))?;
    let ws = repo::workspace::get(&state.db, &req.workspace_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("workspace {}", req.workspace_id)))?;

    let dir = design_dir(&ws.folder_path);
    let host = design_host::current().await;
    serde_json::to_value(build_info(&dir, host)).map_err(|e| AppError::Internal(e.to_string()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewReq {
    workspace_id: String,
    /// Data mode vs gate mode (lead-ratified exit-code contract — see below).
    #[serde(default)]
    json: bool,
}

/// `design.review { workspaceId, json? }` → runs the deterministic design-review
/// grader (`design-host/review/review.mjs`) against the workspace's `design/` dir
/// and returns its `{ pass, findings, assertions }` report.
///
/// Exit-code contract (ruled by the lead, ledger note on `design-review`): the
/// PLAIN form (`json: false`, i.e. `conclave design review <ws>`) is the GATE —
/// it must fail loudly on serious findings so it drops into `conclave task gate`
/// and `… && next` scripting. Since the CLI maps any handler `Err` to a non-zero
/// process exit and an `Ok` to exit 0, "not passing" is surfaced as `Err` here.
/// The `--json` form (`json: true`) is DATA retrieval — it always returns `Ok`
/// with the full report so a caller can read `pass`/`findings` itself; using it as
/// a gate would silently always pass, which the CLI help line warns against.
pub async fn review(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req = serde_json::from_value::<ReviewReq>(payload)
        .map_err(|e| AppError::Invalid(e.to_string()))?;
    let ws = repo::workspace::get(&state.db, &req.workspace_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("workspace {}", req.workspace_id)))?;

    let dir = design_dir(&ws.folder_path);
    if !dir.is_dir() {
        return Err(AppError::Invalid(format!(
            "design review: no design/ directory for workspace {} — open the Design view or write design/screens/ first",
            req.workspace_id
        )));
    }

    let report = design_host::review(&dir)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let pass = report.get("pass").and_then(Value::as_bool).unwrap_or(false);

    // Data mode, or a clean pass → return the report (CLI exits 0). Gate mode with
    // findings → Err (CLI exits non-zero) carrying a one-line summary + a pointer to
    // the full JSON. `AppError` has no dedicated "check failed" variant and `error.rs`
    // is outside this lane's boundary, so `Invalid` carries it — the message leads
    // with "design review:" so the intent is unambiguous in a gate tail.
    if req.json || pass {
        Ok(report)
    } else {
        let n = report
            .get("findings")
            .and_then(Value::as_array)
            .map(|a| a.len())
            .unwrap_or(0);
        Err(AppError::Invalid(format!(
            "design review: not passing — {n} serious finding(s); run `conclave design review {} --json` for the full report",
            req.workspace_id
        )))
    }
}

// ── design/ scaffold ────────────────────────────────────────────────────
//
// Minimal starter so a brand-new workspace's canvas is never empty: one
// `screens/welcome.tsx` plus a `lib/` dir for whatever an agent wants to
// share across screens. Every write is idempotent (skipped if the file
// already exists).

// `design/theme.css` — a real Tailwind v4 CSS-first sheet (R4 + R6). `@import
// "tailwindcss"` makes `@tailwindcss/vite` (wired in design-host/vite.config.ts)
// compile it; `@source` globs (relative to this file = the workspace `design/` dir)
// tell Tailwind which files to scan for utility classes; `@theme` declares the token
// set the grader's A1a asserts exists and the welcome screen's utilities are built
// from (so A1b sees no hardcoded hex). This is the Arta-parity authoring contract the
// `design-craft` skills teach — edit tokens here, use `bg-canvas`/`text-ink`/… in
// screens.
const THEME_CSS: &str = r#"@import "tailwindcss";

@source "./screens/**/*.tsx";
@source "./components/**/*.tsx";
@source "./lib/**/*.{ts,tsx}";

@theme {
  --color-canvas: #0b0b0f;
  --color-surface: #17171d;
  --color-ink: #ececf1;
  --color-muted: #9a9aa7;
  --color-accent: #6d6df0;
  --color-border: #262630;

  --radius-panel: 0.875rem;

  --font-sans: ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif;
  --font-mono: ui-monospace, "SF Mono", "JetBrains Mono", Menlo, monospace;
}
"#;

const WELCOME_TSX: &str = r#"export default function Welcome() {
  return (
    <main className="min-h-screen grid place-items-center bg-canvas font-sans">
      <div className="max-w-md px-8 text-center">
        <h1 className="text-2xl font-semibold text-ink">Your canvas is live</h1>
        <p className="mt-3 text-sm leading-relaxed text-muted">
          Ask your agent to design something. Screens are React files in{" "}
          <span className="text-ink">design/screens/</span>, styled with Tailwind
          utilities built from the tokens in{" "}
          <span className="text-ink">design/theme.css</span>.
        </p>
      </div>
    </main>
  );
}
"#;

fn scaffold_if_missing(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir.join("screens"))?;
    std::fs::create_dir_all(dir.join("lib"))?;
    write_if_missing(&dir.join("theme.css"), THEME_CSS)?;
    write_if_missing(&dir.join("screens").join("welcome.tsx"), WELCOME_TSX)?;
    Ok(())
}

fn write_if_missing(path: &Path, contents: &str) -> std::io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    std::fs::write(path, contents)
}

// ── registry.json read-modify-write ─────────────────────────────────────────

#[derive(Deserialize, Serialize)]
struct RegistryEntry {
    id: String,
    name: String,
    dir: String,
}

/// Upsert `{id: project_id, name, dir}` into the shared registry, atomically
/// (write to a sibling temp file, then rename over the real one) so a reader
/// never observes a half-written file. Creates `design_home_dir()` and an
/// empty registry if neither exists yet.
fn upsert_registry(dir: &Path, name: &str, project_id: &str) -> std::io::Result<()> {
    let registry_path = design_host::registry_file()?;
    let mut entries: Vec<RegistryEntry> = std::fs::read_to_string(&registry_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let dir_str = dir.to_string_lossy().into_owned();
    match entries.iter_mut().find(|e| e.id == project_id) {
        Some(existing) => {
            existing.name = name.to_owned();
            existing.dir = dir_str;
        }
        None => entries.push(RegistryEntry {
            id: project_id.to_owned(),
            name: name.to_owned(),
            dir: dir_str,
        }),
    }

    let tmp_path = registry_path.with_extension("json.tmp");
    std::fs::write(&tmp_path, serde_json::to_string_pretty(&entries)?)?;
    std::fs::rename(&tmp_path, &registry_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Full functional round trip: a scratch workspace → `design.ensure` →
    /// `design/` scaffolded → real `node`/Vite sidecar spawned →
    /// `/__design/health` and the scratch screen's `/@fs/` module both 200 →
    /// `design.status` (no side effects) agrees.
    ///
    /// Genuinely spawns a process and polls real HTTP (requires `node`/`pnpm`
    /// on PATH), so this is `#[ignore]`d like the fastembed spike tests — run
    /// manually: `cargo test --lib design::tests::ensure_round_trip --
    /// --ignored --nocapture`. Side effect: writes a real entry into this
    /// machine's `design_home_dir()` registry.json (there is no test-mode
    /// override, matching `design_home_dir`'s plain `dirs::data_dir()`
    /// convention) — harmless (it points at a since-deleted temp dir) but
    /// worth knowing before wondering why registry.json has a stray row.
    #[tokio::test]
    #[ignore]
    async fn ensure_round_trip() {
        let state = AppState::for_tests().await;
        let tmp = std::env::temp_dir().join(format!("conclave-design-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).expect("mkdir scratch workspace");
        let ws = repo::workspace::create(&state.db, "Design Test Scratch", tmp.to_str().unwrap(), None)
            .await
            .expect("insert scratch workspace");

        let res = ensure(&state, serde_json::json!({ "workspaceId": ws.id }))
            .await
            .expect("design.ensure failed");
        assert_eq!(res["running"], serde_json::json!(true));
        let url = res["url"].as_str().expect("url present when running").to_owned();

        assert!(
            tmp.join("design/screens/welcome.tsx").is_file(),
            "scaffold must create the starter screen"
        );

        let client = reqwest::Client::new();
        let resp = client.get(&url).send().await.expect("GET host URL");
        assert!(resp.status().is_success(), "host URL must 200: {}", resp.status());

        let port = res["port"].as_u64().expect("port present when running");
        let screen_path = tmp.join("design/screens/welcome.tsx");
        let fs_url = format!("http://127.0.0.1:{port}/@fs{}", screen_path.display());
        let fs_resp = client.get(&fs_url).send().await.expect("GET /@fs/ screen");
        assert!(
            fs_resp.status().is_success(),
            "the scratch project's screen must compile through /@fs/ (fs.strict must stay disabled): {}",
            fs_resp.status()
        );

        // design.status must agree, with no side effects of its own.
        let status_res = status(&state, serde_json::json!({ "workspaceId": ws.id }))
            .await
            .expect("design.status failed");
        assert_eq!(status_res["running"], serde_json::json!(true));
        assert_eq!(status_res["projectId"], res["projectId"]);
    }
}
