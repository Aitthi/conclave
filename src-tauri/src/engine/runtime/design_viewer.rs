//! Supervisor for the vendored design-canvas viewer sidecar (`design-viewer/`,
//! a Vite dev server — see `docs/2026-07-05-plan-design-artifacts-views.md`).
//!
//! ONE sidecar process serves every Conclave workspace; each workspace is a
//! separate "project" the vendored viewer multiplexes via a shared
//! `registry.json` (see [`registry_file`] / [`project_id_for`]). This module
//! owns the process's whole lifecycle — resolving `node`, first-run `pnpm
//! install`, spawning, health-checking, crash-restart with capped backoff,
//! and kill-on-app-exit. It has ZERO dependency on the database or Tauri
//! (mirrors `runtime::pty`'s independence) — `commands::design` is the only
//! caller, and owns everything workspace-specific (which folder, scaffolding
//! `.arta/`, writing this workspace's registry entry, building the iframe
//! URL).
//!
//! # Registry write-before-spawn (load-bearing)
//!
//! Proven empirically while vendoring: the underlying `chokidar` watcher does
//! NOT reliably detect `registry.json`'s first-ever creation if the sidecar
//! started watching that path before the file existed, but DOES pick up
//! updates to an already-existing watched file. `commands::design::ensure`
//! MUST create/update `registry.json` (via [`registry_file`]) BEFORE calling
//! [`ensure_running`] — never after — or a workspace registered on the very
//! first `design.ensure` call of the process's lifetime can silently fail to
//! appear in the viewer.
//!
//! # Concurrency
//!
//! A single process-wide [`Supervisor`] (module-level `OnceLock`, like
//! `agentctx`'s shim caching) rather than an `AppState` field: this service
//! needs no DB access and no per-instance keying, so threading it through
//! `AppState` would only add an unused dependency for every other handler.
//! [`ensure_running`] holds the supervisor's lock for its entire respawn
//! sequence (health check → resolve → install → spawn → health-poll) so
//! concurrent `design.ensure` calls serialize instead of racing into a
//! double-spawn — calls are rare (one per ipc round trip) so this is cheap.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex as AsyncMutex;

/// Loopback port the sidecar defaults to when `CONCLAVE_DESIGN_PORT` is
/// unset — must match `design-viewer/vite.config.ts`'s own default.
const DEFAULT_PORT: u16 = 7343;

/// Stdout marker line `design-viewer/bin/viewer.mjs` prints once Vite is
/// actually listening — see that file's header comment.
const READY_PREFIX: &str = "DESIGN_VIEWER_READY port=";

/// Total time budget for spawn + first-run `pnpm install` + health poll.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

/// Capped exponential backoff for crash-restart, doubling from 1s, capped at
/// 30s; gives up after [`MAX_RESTART_ATTEMPTS`] consecutive failures (the
/// slot is left empty — the next explicit `design.ensure` call starts a
/// fresh attempt sequence).
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const MAX_RESTART_ATTEMPTS: u32 = 5;

/// Budget for [`ensure_running`]'s reuse check on an EXISTING slot. Kept
/// short (this runs on every `design.ensure` call, not just at spawn time)
/// but long enough to tolerate a momentarily-slow-but-alive dev server under
/// load — review finding F3 (Mellow) flagged a too-tight budget as a false
/// "dead" verdict; correctness no longer depends on this being long enough
/// (a false negative now kills-and-respawns cleanly instead of orphaning,
/// see the `kill_pid` call in `ensure_running`), so this is a performance/
/// respawn-frequency tradeoff, not a safety one.
const REUSE_HEALTH_BUDGET: Duration = Duration::from_secs(2);

/// What [`ensure_running`] hands back to `commands::design` — everything it
/// needs to build the iframe URL (the project id / query string is the
/// caller's concern, not the sidecar's).
#[derive(Debug, Clone, Copy)]
pub struct ViewerInfo {
    pub port: u16,
}

/// Typed failure modes so `commands::design` (and eventually the Design view)
/// can distinguish "install Node.js" from a generic startup failure, rather
/// than a panic/hang. Mirrors `runtime::StdinError`'s manual `Display` impl
/// (this crate's convention for runtime-layer errors — `thiserror` is used at
/// the `AppError`/command layer instead).
#[derive(Debug)]
pub enum DesignViewerError {
    /// `node` could not be resolved on the user's login-shell PATH.
    NodeNotFound,
    /// Spawning the resolver shell, `pnpm install`, or the viewer itself
    /// failed at the OS level (distinct from a plain non-zero exit, which is
    /// folded into the message too).
    Spawn(String),
    /// The child never printed [`READY_PREFIX`], or never answered
    /// `/__arta/state` with 200, inside [`STARTUP_TIMEOUT`].
    HealthTimeout,
}

impl std::fmt::Display for DesignViewerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DesignViewerError::NodeNotFound => write!(
                f,
                "node was not found on PATH — install Node.js to use the Design view"
            ),
            DesignViewerError::Spawn(msg) => write!(f, "design viewer sidecar failed to start: {msg}"),
            DesignViewerError::HealthTimeout => {
                write!(f, "design viewer sidecar did not become healthy in time")
            }
        }
    }
}

impl std::error::Error for DesignViewerError {}

/// One live generation of the spawned child. `epoch` lets a crash-monitor
/// task (see [`monitor`]) tell whether it still owns the current generation
/// before mutating shared state — the same guard `runtime::Runtime` uses for
/// its PTY backends (`unregister_epoch`), for the identical reason: an old
/// generation's late cleanup must never clobber a newer one.
struct Slot {
    port: u16,
    /// The spawned node process's OWN pid (not a wrapping shell's — see
    /// [`spawn_node`]) so [`kill_on_exit`] can reliably signal it.
    pid: u32,
    epoch: u64,
}

struct Supervisor {
    slot: AsyncMutex<Option<Slot>>,
    next_epoch: AtomicU64,
}

static SUPERVISOR: OnceLock<Arc<Supervisor>> = OnceLock::new();

fn supervisor() -> Arc<Supervisor> {
    Arc::clone(SUPERVISOR.get_or_init(|| {
        Arc::new(Supervisor {
            slot: AsyncMutex::new(None),
            next_epoch: AtomicU64::new(0),
        })
    }))
}

/// The engine's app-data subdir the sidecar reads/writes `registry.json`
/// under (`CONCLAVE_DESIGN_HOME`) — created if missing. Mirrors
/// `repo::skill::new_draft_dir`'s `dirs::data_dir()`-based Conclave app-data
/// convention.
pub fn design_home_dir() -> std::io::Result<PathBuf> {
    let dir = dirs::data_dir()
        .ok_or_else(|| std::io::Error::new(ErrorKind::NotFound, "no user data directory"))?
        .join("Conclave")
        .join("design-viewer");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// `<design_home_dir>/registry.json` — the shared project registry
/// `design-viewer/vite/projects.ts` reads. Callers MUST write/update this
/// file (creating `design_home_dir()` first) BEFORE calling [`ensure_running`]
/// — see the module doc's "registry write-before-spawn" note.
pub fn registry_file() -> std::io::Result<PathBuf> {
    Ok(design_home_dir()?.join("registry.json"))
}

/// Byte-for-byte port of `design-viewer/vite/projects.ts`'s `idFor()` —
/// FNV-1a over the resolved absolute path, base36-encoded. MUST keep
/// computing the same id for the same directory as the JS side; do not
/// "improve" this without updating both and the cross-language test below.
///
/// The JS side hashes `path.resolve(dir)` — a LEXICAL normalization (`.`/`..`
/// segments, redundant separators), never a symlink-following `realpath`.
/// [`lexical_normalize`] mirrors that here: found in review (Mellow, F2)
/// that `workspace.link` stores `folder_path` verbatim with no
/// normalization (`commands::workspace::link`), so a workspace linked with a
/// trailing slash or a `..` segment would otherwise hash to a DIFFERENT id
/// than the JS side computes for the same directory — that workspace's
/// canvas would silently render blank (wrong/no registry match) despite
/// `design.ensure` reporting success.
pub fn project_id_for(dir: &Path) -> String {
    let resolved = lexical_normalize(dir);
    let s = resolved.to_string_lossy();
    let mut h: u32 = 2166136261;
    for b in s.as_bytes() {
        // JS's `charCodeAt` is a UTF-16 code unit, not a byte — for ASCII
        // paths (the overwhelmingly common case, and the only one the
        // cross-language test can assert against without a JS runtime) a
        // UTF-16 code unit and a UTF-8 byte are numerically identical, so
        // iterating raw bytes agrees with the JS implementation. Genuinely
        // non-ASCII workspace paths are astonishingly rare on top of already
        // being astonishingly rare for THIS specific hash's collision
        // surface (36^N ids) to matter — this module note is the single
        // point to revisit if that ever needs closing more tightly.
        h ^= u32::from(*b);
        h = h.wrapping_mul(16777619);
    }
    to_base36(h)
}

/// Lexical path normalization matching Node's `path.resolve()` semantics:
/// collapses `.` segments and pops the preceding component on `..` (when
/// there is one to pop), WITHOUT touching the filesystem — no symlink
/// resolution, no existence check (`std::fs::canonicalize` does both, which
/// would make this disagree with the JS side the moment a path traverses a
/// symlink; `path.resolve` never does either). Trailing separators are
/// dropped as a side effect of rebuilding from `Path::components()`.
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                if !matches!(out.components().next_back(), None | Some(std::path::Component::RootDir)) {
                    out.pop();
                } else if out.components().next_back().is_none() {
                    out.push("..");
                }
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn to_base36(mut n: u32) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".to_owned();
    }
    let mut buf = Vec::new();
    while n > 0 {
        buf.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    buf.reverse();
    String::from_utf8(buf).expect("base36 digits are always valid UTF-8")
}

/// The login shell to resolve `node`/run `pnpm install` through — same
/// rationale as `commands::instance`'s CLI-agent spawn: a Tauri app launched
/// from Finder inherits a bare environment that never sourced `~/.zshrc` /
/// `~/.zprofile`, so an nvm- or Homebrew-installed `node` would otherwise be
/// invisible.
fn login_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string())
}

/// Resolve `node`'s absolute path via the user's real login-shell PATH.
///
/// Returns the resolved path (spawned DIRECTLY, no shell wrapper) rather than
/// just confirming existence: the long-lived viewer child is spawned without
/// a shell in between (see [`spawn_node`]) so `child.id()` is `node`'s own
/// pid, not an intermediary shell's — required for [`kill_on_exit`] to
/// reliably reach the right process. `$SHELL -l -i -c "pnpm install"`
/// (one-shot, awaited to completion) has no such requirement and keeps the
/// shell wrapper.
async fn resolve_node() -> Result<PathBuf, DesignViewerError> {
    let shell = login_shell();
    let output = Command::new(&shell)
        .args(["-l", "-i", "-c", "command -v node"])
        .output()
        .await
        .map_err(|e| DesignViewerError::Spawn(format!("resolving node via {shell}: {e}")))?;
    let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if output.status.success() && !path.is_empty() {
        Ok(PathBuf::from(path))
    } else {
        Err(DesignViewerError::NodeNotFound)
    }
}

/// Resolve the vendored viewer package's directory: the bundled
/// `Resources/design-viewer` sibling of the running executable inside a
/// packaged `.app`, falling back to the repo-relative source tree
/// (`CARGO_MANIFEST_DIR/../design-viewer`) for `cargo run`/`tauri dev`.
/// Mirrors `repo::skill::skills_dir()` exactly. Packaged-app resource
/// bundling for `design-viewer/` itself (node_modules et al.) is OUT OF
/// SCOPE for Phase 1 (risk ledger) — this fn only makes dev mode and a
/// future packaged build resolve consistently once that bundling exists.
fn design_viewer_dir() -> PathBuf {
    if let Some(bundled) = bundled_design_viewer_dir() {
        if bundled.is_dir() {
            return bundled;
        }
    }
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../design-viewer"))
}

fn bundled_design_viewer_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.parent()?.join("Resources").join("design-viewer"))
}

/// First-run `pnpm install` inside `dir` if `node_modules` is missing —
/// dev-mode behavior, accepted by the plan's risk ledger. Logs progress to
/// stderr (this crate has no structured logger yet; mirrors `bus.rs`'s
/// plain `eprintln!` convention) since an install can take real wall-clock
/// time and a silent multi-second hang would look like a stuck spawn.
async fn ensure_deps_installed(dir: &Path) -> Result<(), DesignViewerError> {
    if dir.join("node_modules").is_dir() {
        return Ok(());
    }
    eprintln!("[design-viewer] installing dependencies (first run)…");
    let shell = login_shell();
    let cmd = format!("cd {} && pnpm install --frozen-lockfile=false", shell_quote(dir));
    let output = Command::new(&shell)
        .args(["-l", "-i", "-c", &cmd])
        .output()
        .await
        .map_err(|e| DesignViewerError::Spawn(format!("pnpm install: {e}")))?;
    if !output.status.success() {
        return Err(DesignViewerError::Spawn(format!(
            "pnpm install failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    eprintln!("[design-viewer] dependencies installed");
    Ok(())
}

/// Minimal single-quoting for the one path we ever interpolate into a shell
/// `-c` string (the vendored package dir) — not a general-purpose shell
/// escaper, just enough to survive a path containing spaces.
fn shell_quote(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"))
}

/// Spawn `node <dir>/bin/viewer.mjs --port <requested_port>` directly (no
/// shell wrapper — see [`resolve_node`]), piping stdout/stderr. Returns once
/// [`READY_PREFIX`] is seen on stdout (parsing the ACTUAL bound port — Vite
/// bumps to the next free port when `requested_port` is busy, so the ready
/// line, never the request, is the source of truth), or [`DesignViewerError`]
/// on spawn failure / timeout. Drains both pipes for the child's whole
/// lifetime in background tasks so Vite's ordinary dev-server chatter can
/// never fill the OS pipe buffer and stall the process.
async fn spawn_node(
    node: &Path,
    dir: &Path,
    design_home: &Path,
    requested_port: u16,
) -> Result<(Child, u16), DesignViewerError> {
    let mut child = Command::new(node)
        .arg(dir.join("bin/viewer.mjs"))
        .arg("--port")
        .arg(requested_port.to_string())
        .current_dir(dir)
        .env("CONCLAVE_DESIGN_HOME", design_home)
        .env("CONCLAVE_DESIGN_PORT", requested_port.to_string())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| DesignViewerError::Spawn(format!("spawn node: {e}")))?;

    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<u16>();

    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        let mut ready_tx = Some(ready_tx);
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(rest) = line.strip_prefix(READY_PREFIX) {
                if let (Some(tx), Ok(port)) = (ready_tx.take(), rest.trim().parse::<u16>()) {
                    let _ = tx.send(port);
                }
            }
            eprintln!("[design-viewer] {line}");
        }
    });
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            eprintln!("[design-viewer:stderr] {line}");
        }
    });

    match tokio::time::timeout(STARTUP_TIMEOUT, ready_rx).await {
        Ok(Ok(port)) => Ok((child, port)),
        _ => {
            let _ = child.start_kill();
            Err(DesignViewerError::HealthTimeout)
        }
    }
}

/// Poll `GET /__arta/state` on `port` until it answers 200, up to `budget`.
/// A fresh `reqwest::Client` per call is fine — this runs at most once per
/// spawn/respawn, never per request.
async fn wait_state_ok(port: u16, budget: Duration) -> bool {
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}/__arta/state");
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        if let Ok(resp) = client.get(&url).send().await {
            if resp.status().is_success() {
                return true;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Ensure the sidecar is running and healthy, spawning/respawning it if not.
/// Idempotent: a healthy existing child is reused as-is (its port returned
/// unchanged). Callers MUST have already written this workspace's
/// `registry.json` entry (see the module doc) before calling this.
pub async fn ensure_running() -> Result<ViewerInfo, DesignViewerError> {
    let sup = supervisor();
    let mut guard = sup.slot.lock().await;

    if let Some(s) = guard.as_ref() {
        if wait_state_ok(s.port, REUSE_HEALTH_BUDGET).await {
            return Ok(ViewerInfo { port: s.port });
        }
        // Review finding F3 (Mellow): a merely-slow-but-alive sidecar that
        // outlasts the budget above must not become an orphan. The stale
        // slot's monitor task (if still running) will find its epoch
        // superseded once we overwrite the slot below and back off WITHOUT
        // killing its child (that's the epoch guard's whole point — an old
        // generation must never kill a NEW one it raced against). So if we
        // are about to supersede it, we — not the monitor — must be the one
        // to kill it, unconditionally, before spawning a replacement.
        kill_pid(s.pid);
    }

    let node = resolve_node().await?;
    let dir = design_viewer_dir();
    ensure_deps_installed(&dir).await?;
    let design_home = design_home_dir()
        .map_err(|e| DesignViewerError::Spawn(format!("resolving app-data dir: {e}")))?;
    let requested_port = std::env::var("CONCLAVE_DESIGN_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    let (child, port) = spawn_node(&node, &dir, &design_home, requested_port).await?;
    if !wait_state_ok(port, STARTUP_TIMEOUT).await {
        return Err(DesignViewerError::HealthTimeout);
    }

    let epoch = sup.next_epoch.fetch_add(1, Ordering::Relaxed) + 1;
    let pid = child.id().unwrap_or_default();
    *guard = Some(Slot { port, pid, epoch });
    drop(guard);

    tokio::spawn(monitor(sup, child, epoch, node, dir, design_home));

    Ok(ViewerInfo { port })
}

/// Report the sidecar's current port if it is up and healthy, WITHOUT
/// spawning or scaffolding anything — the health-check-only half of
/// `design.status`. `None` covers both "never started" and "crashed".
pub async fn current() -> Option<ViewerInfo> {
    let sup = supervisor();
    let guard = sup.slot.lock().await;
    let s = guard.as_ref()?;
    wait_state_ok(s.port, Duration::from_millis(500))
        .await
        .then_some(ViewerInfo { port: s.port })
}

/// Crash-restart with capped backoff. Owns `child` for its whole lifetime
/// (nothing else can `.wait()` on it) so pipe-drain + exit-detection never
/// race a second owner. On each unexpected exit, checks whether `epoch` is
/// still the current generation before touching shared state — an ensure
/// call that already respawned (superseding this generation) must not have
/// its fresh slot clobbered by this task's now-stale cleanup.
async fn monitor(
    sup: Arc<Supervisor>,
    mut child: Child,
    mut epoch: u64,
    node: PathBuf,
    dir: PathBuf,
    design_home: PathBuf,
) {
    let mut backoff = INITIAL_BACKOFF;
    let mut attempts = 0u32;
    loop {
        let status = child.wait().await;
        {
            let mut guard = sup.slot.lock().await;
            match guard.as_ref() {
                Some(s) if s.epoch == epoch => *guard = None,
                _ => return, // superseded by a newer generation — not our slot to clear
            }
        }
        eprintln!("[design-viewer] sidecar exited ({status:?})");

        attempts += 1;
        if attempts > MAX_RESTART_ATTEMPTS {
            eprintln!(
                "[design-viewer] giving up after {MAX_RESTART_ATTEMPTS} restart attempts — next design.ensure will retry fresh"
            );
            return;
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);

        let requested_port = std::env::var("CONCLAVE_DESIGN_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_PORT);
        let spawned = spawn_node(&node, &dir, &design_home, requested_port).await;
        let (new_child, port) = match spawned {
            Ok(pair) => pair,
            Err(e) => {
                eprintln!("[design-viewer] restart attempt {attempts} failed to spawn: {e}");
                continue;
            }
        };
        if !wait_state_ok(port, STARTUP_TIMEOUT).await {
            eprintln!("[design-viewer] restart attempt {attempts} never became healthy");
            let _ = new_child.id(); // child dropped here, kill_on_drop tears it down
            continue;
        }

        let mut guard = sup.slot.lock().await;
        // Only claim the slot if nothing newer has taken it while we were
        // respawning (an explicit design.ensure could have raced us).
        if guard.is_some() {
            return;
        }
        epoch = sup.next_epoch.fetch_add(1, Ordering::Relaxed) + 1;
        let pid = new_child.id().unwrap_or_default();
        *guard = Some(Slot { port, pid, epoch });
        drop(guard);
        child = new_child;
        attempts = 0;
        backoff = INITIAL_BACKOFF;
        eprintln!("[design-viewer] restarted on port {port}");
    }
}

/// Best-effort synchronous kill for the app-exit path (`lib.rs`'s
/// `RunEvent::Exit` handler runs on the main thread, not guaranteed another
/// chance to poll the async monitor task before the process exits) — signals
/// the node child directly by pid rather than depending on the monitor task
/// getting scheduled again. `try_lock` is a plain sync method on
/// `tokio::sync::Mutex` (no runtime required), safe to call from a non-async
/// context.
pub fn kill_on_exit() {
    let Some(sup) = SUPERVISOR.get() else { return };
    let Ok(guard) = sup.slot.try_lock() else { return };
    let Some(slot) = guard.as_ref() else { return };
    kill_pid(slot.pid);
}

/// Best-effort `kill -9` by pid — shared by [`kill_on_exit`] and
/// [`ensure_running`]'s stale-slot fallthrough (F3). A plain external `kill`
/// (not `Child::start_kill`) because callers here only ever have the pid,
/// never the owning `Child` handle (that's owned by a `monitor` task that
/// may already consider itself superseded and never touch it again).
fn kill_pid(pid: u32) {
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill").args(["-9", &pid.to_string()]).status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cross-language guard: these ids MUST match
    /// `design-viewer/vite/projects.ts`'s `idFor()` byte-for-byte for the
    /// SAME three fixed paths — computed once via `node -e` against the
    /// vendored, unmodified `idFor` and hardcoded here so a future edit to
    /// either side that breaks agreement fails `cargo test`, not a live
    /// project switching to the wrong canvas.
    #[test]
    fn project_id_matches_js_idfor() {
        // Computed via: node -e 'const p=require("path");function idFor(d){const
        // s=p.resolve(d);let h=2166136261>>>0;for(const c of s){h^=c.charCodeAt(0);
        // h=Math.imul(h,16777619)>>>0}return h.toString(36)}console.log(idFor(dir))'
        assert_eq!(project_id_for(Path::new("/Users/dev/app")), "1p4sq9m");
        assert_eq!(project_id_for(Path::new("/tmp/scratch-project")), "1fxfqiy");
        assert_eq!(project_id_for(Path::new("/")), "bo0mse");
    }

    /// Review finding F2 (Mellow): `workspace.link` stores `folder_path`
    /// verbatim (no normalization) — an unnormalized path must still hash to
    /// the SAME id as its clean form, matching `path.resolve`'s lexical
    /// normalization on the JS side. Expected value verified via
    /// `node -e 'console.log(path.resolve("/Users/dev/app/"))'` (and the
    /// `..`/`.` variants) → all three resolve to `/Users/dev/app`, i.e. the
    /// SAME id as `project_id_matches_js_idfor`'s first case.
    #[test]
    fn project_id_normalizes_before_hashing() {
        let clean = project_id_for(Path::new("/Users/dev/app"));
        assert_eq!(project_id_for(Path::new("/Users/dev/app/")), clean, "trailing slash");
        assert_eq!(
            project_id_for(Path::new("/Users/dev/other/../app")),
            clean,
            ".. segment"
        );
        assert_eq!(project_id_for(Path::new("/Users/dev/./app")), clean, ". segment");
    }

    #[test]
    fn lexical_normalize_matches_path_resolve_semantics() {
        assert_eq!(lexical_normalize(Path::new("/a/b/")), PathBuf::from("/a/b"));
        assert_eq!(lexical_normalize(Path::new("/a/b/../c")), PathBuf::from("/a/c"));
        assert_eq!(lexical_normalize(Path::new("/a/./b")), PathBuf::from("/a/b"));
        assert_eq!(lexical_normalize(Path::new("/a/b/..")), PathBuf::from("/a"));
        // `..` past root is a no-op (matches `path.resolve`'s clamping — it
        // never escapes above the filesystem root).
        assert_eq!(lexical_normalize(Path::new("/../a")), PathBuf::from("/a"));
    }
}
