//! Supervisor for the Conclave-native design-canvas host sidecar (`design-host/`, a
//! Vite dev server — see `docs/2026-07-05-plan-design-artifacts-views.md`). Replaces
//! the vendored Arta viewer this module used to supervise (`design-viewer/`); the
//! supervisor machinery itself (registry write-before-spawn, login-shell `node`
//! resolution, first-run `pnpm install`, health-check + capped-backoff respawn,
//! single-flight lock) carries over unchanged — only WHAT is spawned changed.
//!
//! ONE sidecar process serves every Conclave workspace; each workspace is a
//! separate "project" the host multiplexes via a shared `registry.json` (see
//! [`registry_file`] / [`project_id_for`]). This module owns the process's whole
//! lifecycle — resolving `node`, first-run `pnpm install`, spawning, health-checking,
//! crash-restart with capped backoff, and kill-on-app-exit. It has ZERO dependency on
//! the database or Tauri (mirrors `runtime::pty`'s independence) — `commands::design`
//! is the only caller, and owns everything workspace-specific (which folder,
//! scaffolding `design/`, writing this workspace's registry entry, building the
//! iframe URL).
//!
//! # Registry write-before-spawn (load-bearing)
//!
//! Proven empirically while vendoring the predecessor: the underlying `chokidar`
//! watcher does NOT reliably detect `registry.json`'s first-ever creation if the
//! sidecar started watching that path before the file existed, but DOES pick up
//! updates to an already-existing watched file. `commands::design::ensure`
//! MUST create/update `registry.json` (via [`registry_file`]) BEFORE calling
//! [`ensure_running`] — never after — or a workspace registered on the very
//! first `design.ensure` call of the process's lifetime can silently fail to
//! appear in the host.
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

use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex as AsyncMutex;

/// Loopback port the sidecar defaults to when `CONCLAVE_DESIGN_PORT` is
/// unset — must match `design-host/vite.config.ts`'s own default.
const DEFAULT_PORT: u16 = 7343;

/// Stdout marker line `design-host/bin/host.mjs` prints once Vite is
/// actually listening — see that file's header comment.
const READY_PREFIX: &str = "DESIGN_HOST_READY port=";

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
/// load — review finding F3 (Mellow, on the predecessor module) flagged a
/// too-tight budget as a false "dead" verdict; correctness no longer depends
/// on this being long enough (a false negative now kills-and-respawns
/// cleanly instead of orphaning, see the `kill_pid` call in
/// `ensure_running`), so this is a performance/respawn-frequency tradeoff,
/// not a safety one.
const REUSE_HEALTH_BUDGET: Duration = Duration::from_secs(2);

/// Minimum Node major version the `design-host` dependency tree requires at
/// RUNTIME — `react-router-dom`'s `engines` field pins `>=20.0.0` (verified
/// in `design-host/node_modules` 2026-07-06); Vite itself tolerates `^18`,
/// but the dep tree does not, so gate on this rather than Vite's floor.
/// Bump this (and its comment) if the dep tree's floor ever moves.
const MIN_NODE_MAJOR: u32 = 20;

/// What [`ensure_running`] hands back to `commands::design` — everything it
/// needs to build the iframe URL (the project id / query string is the
/// caller's concern, not the sidecar's).
#[derive(Debug, Clone, Copy)]
pub struct HostInfo {
    pub port: u16,
}

/// Typed failure modes so `commands::design` (and the Design view) can
/// distinguish "install Node.js" from a generic startup failure, rather than
/// a panic/hang. Mirrors `runtime::StdinError`'s manual `Display` impl (this
/// crate's convention for runtime-layer errors — `thiserror` is used at the
/// `AppError`/command layer instead).
#[derive(Debug)]
pub enum DesignHostError {
    /// `node` could not be resolved on the user's login-shell PATH.
    NodeNotFound,
    /// `node` was resolved but its major version is below [`MIN_NODE_MAJOR`]
    /// — `design-host`'s dependency tree cannot run on it. `found` is the
    /// raw `node --version` output (e.g. `"v18.20.8"`); `path` is the
    /// resolved node binary so the message can point at exactly which node.
    NodeTooOld { found: String, path: PathBuf },
    /// Spawning the resolver shell, `pnpm install`, or the host itself
    /// failed at the OS level (distinct from a plain non-zero exit, which is
    /// folded into the message too).
    Spawn(String),
    /// The child never printed [`READY_PREFIX`], or never answered
    /// `/__design/health` with 200, inside [`STARTUP_TIMEOUT`].
    HealthTimeout,
}

impl std::fmt::Display for DesignHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DesignHostError::NodeNotFound => write!(
                f,
                "node was not found on PATH — install Node.js to use the Design view"
            ),
            DesignHostError::NodeTooOld { found, path } => write!(
                f,
                "design view requires Node.js {MIN_NODE_MAJOR} or newer; found {found} at {} — install a newer node (e.g. `nvm install {MIN_NODE_MAJOR}` or `brew install node`) and relaunch",
                path.display()
            ),
            DesignHostError::Spawn(msg) => write!(f, "design host sidecar failed to start: {msg}"),
            DesignHostError::HealthTimeout => {
                write!(f, "design host sidecar did not become healthy in time")
            }
        }
    }
}

impl std::error::Error for DesignHostError {}

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
        .join("design-host");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// `<design_home_dir>/registry.json` — the shared project registry
/// `design-host/vite/projects.ts` reads. Callers MUST write/update this
/// file (creating `design_home_dir()` first) BEFORE calling [`ensure_running`]
/// — see the module doc's "registry write-before-spawn" note.
pub fn registry_file() -> std::io::Result<PathBuf> {
    Ok(design_home_dir()?.join("registry.json"))
}

/// Byte-for-byte port of `design-host/vite/projects.ts`'s `idFor()` —
/// FNV-1a over the resolved absolute path, base36-encoded. MUST keep
/// computing the same id for the same directory as the JS side; do not
/// "improve" this without updating both and the cross-language test below.
///
/// The JS side hashes `path.resolve(dir)` — a LEXICAL normalization (`.`/`..`
/// segments, redundant separators), never a symlink-following `realpath`.
/// [`lexical_normalize`] mirrors that here: found in review (on the
/// predecessor module, Mellow F2) that `workspace.link` stores `folder_path`
/// verbatim with no normalization (`commands::workspace::link`), so a
/// workspace linked with a trailing slash or a `..` segment would otherwise
/// hash to a DIFFERENT id than the JS side computes for the same directory —
/// that workspace's canvas would silently render blank (wrong/no registry
/// match) despite `design.ensure` reporting success.
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

/// Resolve `node`'s absolute path via the user's real login-shell PATH, and
/// enforce [`MIN_NODE_MAJOR`] (G2 — a corepack-shimmed `pnpm` crashing under
/// an old node, or the design-host dep tree itself, both fail more
/// confusingly further down; fail fast here with a clear remedy instead).
///
/// Returns the resolved path (spawned DIRECTLY, no shell wrapper) rather than
/// just confirming existence: the long-lived host child is spawned without
/// a shell in between (see [`spawn_node`]) so `child.id()` is `node`'s own
/// pid, not an intermediary shell's — required for [`kill_on_exit`] to
/// reliably reach the right process. `$SHELL -l -i -c "pnpm install"`
/// (one-shot, awaited to completion) has no such requirement and keeps the
/// shell wrapper.
async fn resolve_node() -> Result<PathBuf, DesignHostError> {
    let shell = login_shell();
    let output = Command::new(&shell)
        .args(["-l", "-i", "-c", "command -v node"])
        .output()
        .await
        .map_err(|e| DesignHostError::Spawn(format!("resolving node via {shell}: {e}")))?;
    let path_str = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success() || path_str.is_empty() {
        return Err(DesignHostError::NodeNotFound);
    }
    let path = PathBuf::from(&path_str);

    let version_output = Command::new(&path)
        .arg("--version")
        .output()
        .await
        .map_err(|e| DesignHostError::Spawn(format!("checking `{path_str} --version`: {e}")))?;
    let version_stdout = String::from_utf8_lossy(&version_output.stdout);
    let raw_version = version_stdout.trim().to_owned();
    if !version_output.status.success() {
        return Err(DesignHostError::Spawn(format!(
            "`{path_str} --version` exited {:?}: stdout={raw_version:?} stderr={:?}",
            version_output.status.code(),
            String::from_utf8_lossy(&version_output.stderr).trim()
        )));
    }
    let major = last_version_major(&version_stdout).ok_or_else(|| {
        DesignHostError::Spawn(format!(
            "could not parse a version from `{path_str} --version` output: {raw_version:?}"
        ))
    })?;
    if major < MIN_NODE_MAJOR {
        return Err(DesignHostError::NodeTooOld { found: raw_version, path });
    }
    Ok(path)
}

/// Parse the major version number out of a single `vMAJOR.MINOR.PATCH` (or
/// plain `MAJOR.MINOR.PATCH`) line — tolerant of a leading `v`, nothing
/// else. Returns `None` for anything that doesn't start with `<digits>.`.
fn parse_version_major(line: &str) -> Option<u32> {
    let core = line.trim().strip_prefix('v').unwrap_or_else(|| line.trim());
    core.split('.').next()?.parse::<u32>().ok()
}

/// Scan `output` (raw multi-line stdout) for the LAST line that parses as a
/// version per [`parse_version_major`], tolerant of login-shell rc noise
/// (nvm/asdf/etc. hooks routinely print banners to stdout before the real
/// command output — the risk ledger for both `node --version` and `pnpm
/// --version` call sites).
fn last_version_major(output: &str) -> Option<u32> {
    output.lines().rev().find_map(parse_version_major)
}

/// Resolve the vendored host package's directory to spawn/install from. In a
/// packaged `.app`, the bundled `Resources/design-host` tree (SOURCE only,
/// no `node_modules` — see D1/`tauri.conf.json`'s `bundle.resources`) is
/// read-only (App Translocation / DMG mounts, and writing into
/// `Contents/Resources` breaks the code signature), so it is synced to a
/// writable copy under `design_home_dir()?/runtime` (D2) via
/// [`sync_bundled_to_runtime`] and THAT path is returned — giving
/// `ensure_deps_installed`'s `pnpm install` somewhere writable to install
/// into. Falls back to the repo-relative source tree
/// (`CARGO_MANIFEST_DIR/../design-host`) for `cargo run`/`tauri dev` (D6: no
/// bundled dir exists there, so this fallback is byte-identical to
/// pre-packaging behavior).
fn design_host_dir() -> Result<PathBuf, DesignHostError> {
    if let Some(bundled) = bundled_design_host_dir() {
        if bundled.is_dir() {
            return sync_bundled_to_runtime(&bundled).map_err(|e| {
                DesignHostError::Spawn(format!(
                    "syncing bundled design-host ({}) to runtime dir: {e}",
                    bundled.display()
                ))
            });
        }
    }
    Ok(PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../design-host")))
}

fn bundled_design_host_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.parent()?.join("Resources").join("design-host"))
}

/// Sync the bundled (read-only) design-host tree to a writable copy at
/// `design_home_dir()?/runtime`, per D2/D3. Thin wrapper resolving the real
/// app-data runtime path around [`sync_to_runtime`], which does the actual
/// work against an arbitrary `runtime` dir so it can be exercised against a
/// tempdir in tests without touching the real app-data directory.
fn sync_bundled_to_runtime(bundled: &Path) -> std::io::Result<PathBuf> {
    let runtime = design_home_dir()?.join("runtime");
    sync_to_runtime(bundled, &runtime)?;
    Ok(runtime)
}

/// Returns immediately WITHOUT copying when [`fingerprint_tree`] proves
/// `runtime` already matches `bundled` — this runs on every `design.ensure`
/// call (via [`design_host_dir`]), not just first boot, so the common case
/// must be a cheap hash-and-compare, not a tree walk + copy.
fn sync_to_runtime(bundled: &Path, runtime: &Path) -> std::io::Result<()> {
    let fingerprint_path = runtime.join(".bundle-fingerprint");
    let current = fingerprint_tree(bundled)?;

    if runtime.is_dir() {
        if std::fs::read_to_string(&fingerprint_path).ok().as_deref() == Some(current.as_str()) {
            return Ok(());
        }
        std::fs::remove_dir_all(runtime)?;
    }

    copy_tree(bundled, runtime)?;
    // Written LAST: an interrupted copy (process killed mid-sync) leaves the
    // runtime dir populated but WITHOUT a fingerprint file, so the read above
    // finds no match on the next call and re-syncs from scratch rather than
    // trusting a half-copied tree.
    std::fs::write(&fingerprint_path, &current)?;
    Ok(())
}

/// SHA-256 over `dir`'s tree per plan D3: walk files sorted by relative
/// path, hashing `relpath bytes + 0x00 + file contents` for each. Sorted
/// order makes the result independent of filesystem iteration order; the
/// NUL separator between path and contents prevents two different trees
/// from hashing equal via boundary-shifted concatenation (e.g. a file named
/// "ab" with contents "c" vs. a file named "a" with contents "bc").
fn fingerprint_tree(dir: &Path) -> std::io::Result<String> {
    let mut files = Vec::new();
    collect_files(dir, dir, &mut files)?;
    files.sort();

    let mut hasher = Sha256::new();
    for rel in &files {
        hasher.update(rel.to_string_lossy().as_bytes());
        hasher.update([0u8]);
        hasher.update(std::fs::read(dir.join(rel))?);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, out)?;
        } else {
            out.push(
                path.strip_prefix(root)
                    .expect("walked entry is a descendant of root")
                    .to_path_buf(),
            );
        }
    }
    Ok(())
}

fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let dest_path = dst.join(entry.file_name());
        if path.is_dir() {
            copy_tree(&path, &dest_path)?;
        } else {
            std::fs::copy(&path, &dest_path)?;
        }
    }
    Ok(())
}

/// Run the deterministic design-review grader (`review/review.mjs`) against a
/// workspace `design/` dir and return its parsed `{ pass, findings, assertions }`
/// report.
///
/// Resolves `node` via the login-shell PATH exactly like the host sidecar (so an
/// nvm-/Homebrew-installed node is found), then spawns
/// `node review/review.mjs <dir> --json`. In `--json` mode `review.mjs` prints
/// EXACTLY that one JSON object on stdout and nothing else, so we parse stdout
/// directly. A NON-ZERO child exit is EXPECTED and benign when the design has
/// findings — the report's own `pass` field is the signal the caller acts on, not
/// the exit code — so a non-zero exit is not itself an error here; only failing to
/// spawn node or to parse the report is.
pub async fn review(design_dir: &Path) -> Result<serde_json::Value, DesignHostError> {
    let node = resolve_node().await?;
    let host_dir = design_host_dir()?;
    // `review.mjs` imports esbuild (A4) + ./slop-detect.mjs from the host package's
    // node_modules; a fresh checkout that has never started the host sidecar has none
    // yet. Mirror `ensure_running`'s first-run `pnpm install` (F2) so
    // `conclave design review` works standalone, not only after the viewer booted.
    ensure_deps_installed(&host_dir).await?;
    let script = host_dir.join("review").join("review.mjs");
    let output = Command::new(&node)
        .arg(&script)
        .arg(design_dir)
        .arg("--json")
        .output()
        .await
        .map_err(|e| DesignHostError::Spawn(format!("spawning design review: {e}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str::<serde_json::Value>(stdout.trim()).map_err(|e| {
        DesignHostError::Spawn(format!(
            "design review did not produce a parseable report ({e}); stderr: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    })
}

/// First-run install inside `dir` if `node_modules` is missing — dev-mode
/// behavior, accepted by the plan's risk ledger. Prefers `pnpm`, but only
/// when it is actually WORKABLE (G1 — a `command -v pnpm` hit is not enough:
/// a corepack shim can exist on PATH and still crash outright under an old
/// node, e.g. `TypeError: Invalid host defined options` under Node 18), so
/// this runs `pnpm --version` through the login shell and only trusts it on
/// a successful exit with a parseable version on stdout. Falls back to `npm
/// install --no-audit --no-fund` otherwise (D4 — npm ships with node, so
/// this only trades away `pnpm-lock.yaml` version pinning on pnpm-less
/// machines, an accepted tradeoff). Logs progress to stderr (this crate has
/// no structured logger yet; mirrors `bus.rs`'s plain `eprintln!`
/// convention) since an install can take real wall-clock time and a silent
/// multi-second hang would look like a stuck spawn.
async fn ensure_deps_installed(dir: &Path) -> Result<(), DesignHostError> {
    if dir.join("node_modules").is_dir() {
        return Ok(());
    }
    let shell = login_shell();
    let pnpm_check = Command::new(&shell).args(["-l", "-i", "-c", "pnpm --version"]).output().await;
    let (manager, install_cmd) = match &pnpm_check {
        Ok(out) if out.status.success() && last_version_major(&String::from_utf8_lossy(&out.stdout)).is_some() => {
            ("pnpm", "pnpm install --frozen-lockfile=false")
        }
        other => {
            let reason = match other {
                Ok(out) => format!(
                    "exit {:?}, stdout={:?}, stderr={:?}",
                    out.status.code(),
                    String::from_utf8_lossy(&out.stdout).trim(),
                    String::from_utf8_lossy(&out.stderr).trim()
                ),
                Err(e) => format!("failed to run: {e}"),
            };
            eprintln!("[design-host] pnpm not workable ({reason}) — falling back to npm");
            ("npm", "npm install --no-audit --no-fund")
        }
    };

    eprintln!("[design-host] installing dependencies via {manager} (first run)…");
    let cmd = format!("cd {} && {install_cmd}", shell_quote(dir));
    let output = Command::new(&shell)
        .args(["-l", "-i", "-c", &cmd])
        .output()
        .await
        .map_err(|e| DesignHostError::Spawn(format!("{manager} install: {e}")))?;
    if !output.status.success() {
        return Err(DesignHostError::Spawn(format!(
            "{manager} install failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    eprintln!("[design-host] dependencies installed via {manager}");
    Ok(())
}

/// Minimal single-quoting for the one path we ever interpolate into a shell
/// `-c` string (the vendored package dir) — not a general-purpose shell
/// escaper, just enough to survive a path containing spaces.
fn shell_quote(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"))
}

/// Spawn `node <dir>/bin/host.mjs --port <requested_port>` directly (no
/// shell wrapper — see [`resolve_node`]), piping stdout/stderr. Returns once
/// [`READY_PREFIX`] is seen on stdout (parsing the ACTUAL bound port — Vite
/// bumps to the next free port when `requested_port` is busy, so the ready
/// line, never the request, is the source of truth), or [`DesignHostError`]
/// on spawn failure / timeout. Drains both pipes for the child's whole
/// lifetime in background tasks so Vite's ordinary dev-server chatter can
/// never fill the OS pipe buffer and stall the process.
async fn spawn_node(
    node: &Path,
    dir: &Path,
    design_home: &Path,
    requested_port: u16,
) -> Result<(Child, u16), DesignHostError> {
    let mut child = Command::new(node)
        .arg(dir.join("bin/host.mjs"))
        .arg("--port")
        .arg(requested_port.to_string())
        .current_dir(dir)
        .env("CONCLAVE_DESIGN_HOME", design_home)
        .env("CONCLAVE_DESIGN_PORT", requested_port.to_string())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| DesignHostError::Spawn(format!("spawn node: {e}")))?;

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
            eprintln!("[design-host] {line}");
        }
    });
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            eprintln!("[design-host:stderr] {line}");
        }
    });

    match tokio::time::timeout(STARTUP_TIMEOUT, ready_rx).await {
        Ok(Ok(port)) => Ok((child, port)),
        _ => {
            let _ = child.start_kill();
            Err(DesignHostError::HealthTimeout)
        }
    }
}

/// Poll `GET /__design/health` on `port` until it answers 200, up to
/// `budget`. A fresh `reqwest::Client` per call is fine — this runs at most
/// once per spawn/respawn, never per request.
async fn wait_state_ok(port: u16, budget: Duration) -> bool {
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}/__design/health");
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
pub async fn ensure_running() -> Result<HostInfo, DesignHostError> {
    let sup = supervisor();
    let mut guard = sup.slot.lock().await;

    if let Some(s) = guard.as_ref() {
        if wait_state_ok(s.port, REUSE_HEALTH_BUDGET).await {
            return Ok(HostInfo { port: s.port });
        }
        // A merely-slow-but-alive sidecar that outlasts the budget above must
        // not become an orphan. The stale slot's monitor task (if still
        // running) will find its epoch superseded once we overwrite the slot
        // below and back off WITHOUT killing its child (that's the epoch
        // guard's whole point — an old generation must never kill a NEW one
        // it raced against). So if we are about to supersede it, we — not the
        // monitor — must be the one to kill it, unconditionally, before
        // spawning a replacement.
        kill_pid(s.pid);
    }

    let node = resolve_node().await?;
    let dir = design_host_dir()?;
    ensure_deps_installed(&dir).await?;
    let design_home = design_home_dir()
        .map_err(|e| DesignHostError::Spawn(format!("resolving app-data dir: {e}")))?;
    let requested_port = std::env::var("CONCLAVE_DESIGN_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    let (child, port) = spawn_node(&node, &dir, &design_home, requested_port).await?;
    if !wait_state_ok(port, STARTUP_TIMEOUT).await {
        return Err(DesignHostError::HealthTimeout);
    }

    let epoch = sup.next_epoch.fetch_add(1, Ordering::Relaxed) + 1;
    let pid = child.id().unwrap_or_default();
    *guard = Some(Slot { port, pid, epoch });
    drop(guard);

    tokio::spawn(monitor(sup, child, epoch, node, dir, design_home));

    Ok(HostInfo { port })
}

/// Report the sidecar's current port if it is up and healthy, WITHOUT
/// spawning or scaffolding anything — the health-check-only half of
/// `design.status`. `None` covers both "never started" and "crashed".
pub async fn current() -> Option<HostInfo> {
    let sup = supervisor();
    let guard = sup.slot.lock().await;
    let s = guard.as_ref()?;
    wait_state_ok(s.port, Duration::from_millis(500))
        .await
        .then_some(HostInfo { port: s.port })
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
        eprintln!("[design-host] sidecar exited ({status:?})");

        attempts += 1;
        if attempts > MAX_RESTART_ATTEMPTS {
            eprintln!(
                "[design-host] giving up after {MAX_RESTART_ATTEMPTS} restart attempts — next design.ensure will retry fresh"
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
                eprintln!("[design-host] restart attempt {attempts} failed to spawn: {e}");
                continue;
            }
        };
        if !wait_state_ok(port, STARTUP_TIMEOUT).await {
            eprintln!("[design-host] restart attempt {attempts} never became healthy");
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
        eprintln!("[design-host] restarted on port {port}");
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
/// [`ensure_running`]'s stale-slot fallthrough. A plain external `kill` (not
/// `Child::start_kill`) because callers here only ever have the pid, never
/// the owning `Child` handle (that's owned by a `monitor` task that may
/// already consider itself superseded and never touch it again).
fn kill_pid(pid: u32) {
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill").args(["-9", &pid.to_string()]).status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_major_handles_v_prefix_and_plain_form() {
        assert_eq!(parse_version_major("v18.20.8"), Some(18));
        assert_eq!(parse_version_major("v22.23.1"), Some(22));
        assert_eq!(parse_version_major("9.1.4"), Some(9)); // pnpm's plain form, no `v`
        assert_eq!(parse_version_major("  v20.0.0  "), Some(20)); // surrounding whitespace
        assert_eq!(parse_version_major("garbage"), None);
        assert_eq!(parse_version_major(""), None);
        assert_eq!(parse_version_major("vX.Y.Z"), None);
    }

    #[test]
    fn last_version_major_skips_login_shell_banner_noise() {
        // nvm/asdf-style shell hooks print banners to stdout before the real
        // `node --version`/`pnpm --version` output — the real line is last.
        let noisy = "Now using node v20.11.0\nnvm: cache updated\nv18.20.8";
        assert_eq!(last_version_major(noisy), Some(18));

        let clean = "v22.23.1";
        assert_eq!(last_version_major(clean), Some(22));

        assert_eq!(last_version_major("no version here\nnope"), None);
    }

    #[test]
    fn node_too_old_display_names_version_path_and_remedy() {
        let err = DesignHostError::NodeTooOld {
            found: "v18.20.8".to_string(),
            path: PathBuf::from("/Users/tharadon/.nvm/versions/node/v18.20.8/bin/node"),
        };
        let msg = err.to_string();
        assert!(msg.contains("v18.20.8"), "must name the found version: {msg}");
        assert!(
            msg.contains("/Users/tharadon/.nvm/versions/node/v18.20.8/bin/node"),
            "must name the node path: {msg}"
        );
        assert!(msg.contains("20"), "must name the minimum required major: {msg}");
        assert!(
            msg.to_lowercase().contains("install") || msg.to_lowercase().contains("nvm"),
            "must suggest a remedy: {msg}"
        );
    }

    /// Cross-language guard: these ids MUST match
    /// `design-host/vite/projects.ts`'s `idFor()` byte-for-byte for the
    /// SAME three fixed paths — computed once via `node -e` against the
    /// unmodified `idFor` and hardcoded here so a future edit to either side
    /// that breaks agreement fails `cargo test`, not a live project
    /// switching to the wrong canvas.
    #[test]
    fn project_id_matches_js_idfor() {
        // Computed via: node -e 'const p=require("path");function idFor(d){const
        // s=p.resolve(d);let h=2166136261>>>0;for(const c of s){h^=c.charCodeAt(0);
        // h=Math.imul(h,16777619)>>>0}return h.toString(36)}console.log(idFor(dir))'
        assert_eq!(project_id_for(Path::new("/Users/dev/app")), "1p4sq9m");
        assert_eq!(project_id_for(Path::new("/tmp/scratch-project")), "1fxfqiy");
        assert_eq!(project_id_for(Path::new("/")), "bo0mse");
    }

    /// `workspace.link` stores `folder_path` verbatim (no normalization) — an
    /// unnormalized path must still hash to the SAME id as its clean form,
    /// matching `path.resolve`'s lexical normalization on the JS side.
    /// Expected value verified via `node -e
    /// 'console.log(path.resolve("/Users/dev/app/"))'` (and the `..`/`.`
    /// variants) → all three resolve to `/Users/dev/app`, i.e. the SAME id as
    /// `project_id_matches_js_idfor`'s first case.
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

    /// Unique scratch dir under `std::env::temp_dir()`, this crate's existing
    /// convention (see e.g. `repo::skill`'s tests) for filesystem-backed unit
    /// tests — no external tempdir crate is a dependency here.
    fn scratch_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("conclave-design-host-test-{tag}-{}", uuid::Uuid::new_v4()))
    }

    fn write_file(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    #[test]
    fn fingerprint_stable_across_two_walks() {
        let dir = scratch_dir("fp-stable");
        write_file(&dir.join("bin/host.mjs"), "console.log(1)");
        write_file(&dir.join("package.json"), "{}");

        let a = fingerprint_tree(&dir).unwrap();
        let b = fingerprint_tree(&dir).unwrap();
        assert_eq!(a, b);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fingerprint_changes_when_a_file_changes() {
        let dir = scratch_dir("fp-changes");
        write_file(&dir.join("bin/host.mjs"), "console.log(1)");

        let before = fingerprint_tree(&dir).unwrap();
        write_file(&dir.join("bin/host.mjs"), "console.log(2)");
        let after = fingerprint_tree(&dir).unwrap();

        assert_ne!(before, after);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sync_copies_tree_and_skips_recopy_when_fingerprint_matches() {
        let bundled = scratch_dir("sync-bundled");
        let runtime = scratch_dir("sync-runtime");
        write_file(&bundled.join("bin/host.mjs"), "console.log(1)");
        write_file(&bundled.join("package.json"), "{}");

        sync_to_runtime(&bundled, &runtime).unwrap();
        assert_eq!(
            std::fs::read_to_string(runtime.join("bin/host.mjs")).unwrap(),
            "console.log(1)"
        );
        assert!(runtime.join(".bundle-fingerprint").is_file());

        // Drop a marker file into the runtime copy that is NOT in `bundled`.
        // If a second sync with a matching fingerprint were to delete+recopy
        // (rather than skip), this marker would disappear.
        write_file(&runtime.join("marker.txt"), "still here");
        sync_to_runtime(&bundled, &runtime).unwrap();
        assert!(runtime.join("marker.txt").is_file(), "matching fingerprint must skip recopy");

        std::fs::remove_dir_all(&bundled).ok();
        std::fs::remove_dir_all(&runtime).ok();
    }

    #[test]
    fn sync_resyncs_when_fingerprint_file_is_missing() {
        let bundled = scratch_dir("sync-interrupted-bundled");
        let runtime = scratch_dir("sync-interrupted-runtime");
        write_file(&bundled.join("bin/host.mjs"), "console.log(1)");

        // Simulate an interrupted prior copy: the runtime dir exists with
        // some (possibly stale/partial) content but no `.bundle-fingerprint`.
        write_file(&runtime.join("bin/host.mjs"), "stale partial content");

        sync_to_runtime(&bundled, &runtime).unwrap();
        assert_eq!(
            std::fs::read_to_string(runtime.join("bin/host.mjs")).unwrap(),
            "console.log(1)",
            "missing fingerprint must force a full re-sync from bundled"
        );
        assert!(runtime.join(".bundle-fingerprint").is_file());

        std::fs::remove_dir_all(&bundled).ok();
        std::fs::remove_dir_all(&runtime).ok();
    }
}
