//! `design.ensure` / `design.status` — the vendor-neutral bridge between a
//! Conclave workspace and the design-canvas viewer sidecar
//! (`runtime::design_viewer`). Scaffolds a workspace's `.arta/` on first use,
//! keeps the sidecar's shared `registry.json` in sync, and reports the
//! iframe URL the Design view (Phase 2 Lane D) will embed.
//!
//! Neither command touches the agent-facing file CONTRACT itself (writing
//! `state.json` / `proto/*.tsx` is any agent's ordinary Write/Edit tool, per
//! the `design-canvas` skill) — this module only ensures the viewer CAN show
//! whatever is already on disk.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::engine::runtime::design_viewer;
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

/// `<folder_path>/.arta` for a workspace — the canonical project dir both
/// `project_id_for` and the registry entry key off.
fn arta_dir(folder_path: &str) -> PathBuf {
    Path::new(folder_path).join(".arta")
}

fn build_info(dir: &Path, viewer: Option<design_viewer::ViewerInfo>) -> DesignInfo {
    let project_id = design_viewer::project_id_for(dir);
    match viewer {
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
/// 1. Scaffold `.arta/` in the workspace's linked folder if missing.
/// 2. Upsert this workspace into the shared `registry.json` — BEFORE
///    starting the sidecar (load-bearing ordering, see
///    `runtime::design_viewer`'s module doc: the sidecar's file watcher can
///    miss the registry file's first-ever creation, never an update to an
///    already-existing one).
/// 3. Ensure the sidecar is running (spawns/respawns as needed).
pub async fn ensure(state: &AppState, payload: Value) -> Result<Value, AppError> {
    let req = serde_json::from_value::<WorkspaceReq>(payload)
        .map_err(|e| AppError::Invalid(e.to_string()))?;
    let ws = repo::workspace::get(&state.db, &req.workspace_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("workspace {}", req.workspace_id)))?;

    let dir = arta_dir(&ws.folder_path);
    scaffold_if_missing(&dir, &ws.name).map_err(|e| AppError::Internal(format!("scaffold .arta/: {e}")))?;

    let project_id = design_viewer::project_id_for(&dir);
    upsert_registry(&dir, &ws.name, &project_id)
        .map_err(|e| AppError::Internal(format!("registering with design viewer: {e}")))?;

    let info = design_viewer::ensure_running()
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

    let dir = arta_dir(&ws.folder_path);
    let viewer = design_viewer::current().await;
    serde_json::to_value(build_info(&dir, viewer)).map_err(|e| AppError::Internal(e.to_string()))
}

// ── .arta/ scaffold (Rust port of design-viewer/vite/scaffold.ts) ──────────
//
// Kept as a Rust-side port rather than calling the sidecar's own
// `/__arta/scaffold` HTTP route (which shares the same templates) so
// scaffolding works even when `node` is missing or the sidecar has not
// started yet — `design.ensure` must create `.arta/state.json` (with this
// workspace's name) and `.arta/proto/` REGARDLESS of whether the viewer
// itself can currently boot. Every write is idempotent (skipped if the file
// already exists), matching `scaffoldProto`'s own contract exactly.

const THEME_CSS: &str = r#"@import "tailwindcss";
@source "./";
@custom-variant dark (&:where(.dark, .dark *));

@theme {
  --color-primary: oklch(0.66 0.2 250);
  --color-bg: #ffffff;
  --color-fg: #18181b;
  --font-sans: "Geist", "Noto Sans Thai", system-ui, sans-serif;
  --radius-lg: 1rem;
}

.dark {
  --color-bg: #0b0b0c;
  --color-fg: #fafafa;
}

body { background: var(--color-bg); color: var(--color-fg); font-family: var(--font-sans); }
"#;

const HOME_TSX: &str = r#"export const meta = { title: "Home" };

export default function Home() {
  return (
    <main className="min-h-screen grid place-items-center bg-bg text-fg">
      <div className="text-center space-y-3">
        <h1 className="text-4xl font-semibold">Your canvas is live</h1>
        <p className="opacity-70">Ask your agent to design something — screens are React files in .arta/proto/screens/.</p>
      </div>
    </main>
  );
}
"#;

const CONFIG_JSON: &str = "{ \"start\": \"home\" }\n";

fn scaffold_if_missing(dir: &Path, workspace_name: &str) -> std::io::Result<()> {
    let proto_dir = dir.join("proto");
    std::fs::create_dir_all(proto_dir.join("screens"))?;
    std::fs::create_dir_all(proto_dir.join("lib"))?;
    std::fs::create_dir_all(proto_dir.join("components"))?;

    write_if_missing(&proto_dir.join("config.json"), CONFIG_JSON)?;
    write_if_missing(&proto_dir.join("theme.css"), THEME_CSS)?;
    write_if_missing(&proto_dir.join("screens").join("home.tsx"), HOME_TSX)?;

    let state_file = dir.join("state.json");
    if !state_file.exists() {
        let state = serde_json::json!({ "meta": { "name": workspace_name } });
        write_if_missing(&state_file, &serde_json::to_string_pretty(&state).expect("json"))?;
    }
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
    let registry_path = design_viewer::registry_file()?;
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
    /// `.arta/proto/` scaffolded → real `node`/Vite sidecar spawned →
    /// `/__arta/state` and the scratch screen's `/@fs/` module both 200 →
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
            tmp.join(".arta/proto/screens/home.tsx").is_file(),
            "scaffold must create the starter screen"
        );
        assert!(
            tmp.join(".arta/state.json").is_file(),
            "scaffold must create state.json with meta.name"
        );

        let client = reqwest::Client::new();
        let resp = client.get(&url).send().await.expect("GET viewer URL");
        assert!(resp.status().is_success(), "viewer URL must 200: {}", resp.status());

        let port = res["port"].as_u64().expect("port present when running");
        let screen_path = tmp.join(".arta/proto/screens/home.tsx");
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
