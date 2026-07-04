//! Fixture-repo integration tests for the Lane C commit guard (ADR 0008).
//!
//! These drive a throwaway git repo via `std::process::Command`, installing the
//! EXACT hook bytes that `conclave lane guard install` writes (pinned here via
//! `include_str!` of the same source file), and assert the guard:
//!   1. blocks the b9ab709 replay — a staged out-of-scope path is rejected,
//!   2. allows a commit whose staged paths are all inside scope,
//!   3. rejects when `CONCLAVE_COMMIT_SCOPE` is unset,
//!   4. self-skips inside a linked (lane) worktree — the load-bearing skip
//!      without which the shared hook would brick every lane commit.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Same bytes `lane guard install` embeds — one source of truth for the hook.
const GUARD_HOOK: &str = include_str!("../src/bin/pre_commit_guard.sh");

/// A unique temp directory for one fixture repo (no external crates).
fn unique_tmpdir() -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "conclave-lane-guard-{}-{nanos}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Run a git command in `cwd` and assert it succeeded.
fn git_ok(cwd: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Initialise a fixture repo with an identity, no signing, and a seed commit.
fn init_repo() -> PathBuf {
    let dir = unique_tmpdir();
    git_ok(&dir, &["init", "-q"]);
    git_ok(&dir, &["config", "user.email", "t@t.co"]);
    git_ok(&dir, &["config", "user.name", "t"]);
    git_ok(&dir, &["config", "commit.gpgsign", "false"]);
    // A seed commit so `git worktree add` has a HEAD to branch from.
    std::fs::write(dir.join("seed.txt"), "seed\n").unwrap();
    git_ok(&dir, &["add", "seed.txt"]);
    git_ok(&dir, &["commit", "-q", "-m", "seed"]);
    dir
}

/// Install the guard into `<repo>/.git/hooks/pre-commit`, executable.
fn install_hook(repo: &Path) {
    let hook = repo.join(".git").join("hooks").join("pre-commit");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, GUARD_HOOK).unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Run the real CLI installer in `repo`.
fn install_hook_with_cli(repo: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_conclave-cli"))
        .args(["lane", "guard", "install"])
        .current_dir(repo)
        .output()
        .expect("spawn conclave-cli lane guard install")
}

/// Attempt `git commit -m <msg>` in `cwd` with an optional commit scope.
/// Returns (success, combined stderr).
fn try_commit(cwd: &Path, scope: Option<&str>, msg: &str) -> (bool, String) {
    let mut cmd = Command::new("git");
    cmd.args(["commit", "-m", msg]).current_dir(cwd);
    match scope {
        Some(s) => {
            cmd.env("CONCLAVE_COMMIT_SCOPE", s);
        }
        None => {
            cmd.env_remove("CONCLAVE_COMMIT_SCOPE");
        }
    }
    let out = cmd.output().expect("spawn git commit");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn guard_blocks_b9ab709_replay_out_of_scope_path() {
    let repo = init_repo();
    install_hook(&repo);

    std::fs::write(repo.join("owned.txt"), "mine\n").unwrap();
    std::fs::write(repo.join("foreign.txt"), "not mine\n").unwrap();
    // Stage ONLY the foreign file, declare scope over a path we don't touch.
    git_ok(&repo, &["add", "foreign.txt"]);

    let (ok, stderr) = try_commit(&repo, Some("owned.txt"), "sweep");
    assert!(!ok, "guard must reject an out-of-scope staged path");
    assert!(
        stderr.contains("foreign.txt"),
        "rejection must name the offending path; got: {stderr}"
    );

    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn guard_allows_fully_scoped_commit() {
    let repo = init_repo();
    install_hook(&repo);

    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/a.rs"), "a\n").unwrap();
    git_ok(&repo, &["add", "src/a.rs"]);

    // Directory-prefix scope: `src` covers `src/a.rs`.
    let (ok, stderr) = try_commit(&repo, Some("src"), "scoped");
    assert!(ok, "guard must allow an in-scope commit; stderr: {stderr}");

    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn guard_rejects_when_scope_unset() {
    let repo = init_repo();
    install_hook(&repo);

    std::fs::write(repo.join("owned.txt"), "mine\n").unwrap();
    git_ok(&repo, &["add", "owned.txt"]);

    let (ok, stderr) = try_commit(&repo, None, "noscope");
    assert!(!ok, "guard must reject when CONCLAVE_COMMIT_SCOPE is unset");
    assert!(
        stderr.contains("CONCLAVE_COMMIT_SCOPE"),
        "message must explain the missing scope; got: {stderr}"
    );

    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn guard_self_skips_inside_lane_worktree() {
    let repo = init_repo();
    install_hook(&repo);

    // Add a linked worktree — hooks are shared with it via the common git dir.
    let wt = repo.join("wt");
    git_ok(
        &repo,
        &["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "lane/x"],
    );

    std::fs::write(wt.join("z.txt"), "z\n").unwrap();
    git_ok(&wt, &["add", "z.txt"]);

    // A scope that matches NOTHING would block in the shared checkout; inside
    // the lane worktree the guard must self-skip and let the commit through.
    let (ok, stderr) = try_commit(&wt, Some("does-not-match"), "in-worktree");
    assert!(
        ok,
        "guard must self-skip in a lane worktree, else every lane commit breaks; stderr: {stderr}"
    );

    std::fs::remove_dir_all(&repo).ok();
}

#[test]
fn install_warns_when_core_hooks_path_redirects_git() {
    let repo = init_repo();
    git_ok(&repo, &["config", "core.hooksPath", ".husky"]);

    let out = install_hook_with_cli(&repo);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "installer must still succeed when core.hooksPath is set; stderr: {stderr}"
    );
    assert!(
        stderr.contains("core.hooksPath")
            && stderr.contains(".husky")
            && stderr.contains(".git/hooks/pre-commit")
            && stderr.contains("will not fire"),
        "warning must name the config, configured value, installed hook, and consequence; got: \
         {stderr}"
    );
    assert!(
        repo.join(".git/hooks/pre-commit").is_file(),
        "installer must still write the shared pre-commit hook"
    );

    std::fs::remove_dir_all(&repo).ok();
}
