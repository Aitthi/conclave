//! In-memory agent runtime registry.
//!
//! [`Runtime`] tracks live agent sessions keyed by **instance id**
//! (= `workspace_agent` id). It is a pure concurrency primitive with ZERO
//! dependency on the database or Tauri: status persistence and event emission
//! live in the command handlers (`commands::instance`), not here.
//!
//! Future milestones plug into this registry:
//! - M2.2 replaces the placeholder backend task with a real PTY driver.
//! - M2.4 replaces it with the provider chat loop.
//!
//! Both feed stdin into the child / provider and stream output back over the
//! bus; the registry itself does not change.
//!
//! # Concurrency contract
//!
//! Every method is synchronous and only performs quick map operations plus a
//! non-blocking `tokio::spawn` / `tx.send`. The internal `Mutex` guard is
//! **never** held across an `.await`. A poisoned mutex is recovered with
//! `unwrap_or_else(|e| e.into_inner())` so a panicked holder cannot cascade.

pub mod chat;
pub mod design_host;
// The memory-v1 embedder seam lands before its T3/T4 consumers.
#[allow(dead_code)]
pub mod embedder;
pub mod provider;
pub mod pty;
pub mod sandbox_config;
pub mod task_timer;
// T2 lands the codec before the T4 exact-search consumer.
#[allow(dead_code)]
pub mod vec_codec;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

/// Error returned by [`Runtime::send_stdin`].
#[derive(Debug)]
pub enum StdinError {
    /// No live session is registered for the given instance id.
    NotLive,
    /// The session exists but its stdin channel is closed (backend gone).
    ///
    /// Unreachable with the current placeholder backend (it only exits via
    /// `unregister`, which also removes the map entry, so a registered-but-
    /// closed channel cannot occur). Becomes reachable in M2.2/M2.4 when a
    /// backend can self-terminate (PTY child exits / provider stream ends)
    /// while the instance is still registered.
    Closed,
}

impl std::fmt::Display for StdinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StdinError::NotLive => write!(f, "no live session for instance"),
            StdinError::Closed => write!(f, "session stdin channel is closed"),
        }
    }
}

impl std::error::Error for StdinError {}

/// Rolling echo-suppression horizon (`bb plan:working-false-positive`, human
/// smoke report 2026-07-04: roster shows "working…" for idle agents). A
/// PTY/CLI chunk arriving within this window after a `send_stdin` / `resize`
/// call on the SAME instance is presumed to be the terminal's own echo of that
/// input — a mount-jiggle repaint, a wheel-scroll arrow-key echo, or plain
/// keystroke echo — not genuine agent activity, so it must not extend the
/// working indicator. Proven empirically: an idle `claude` provoked 2 repaint
/// chunks from a resize jiggle alone, no stdin
/// (`runtime::pty::tests::idle_claude_repaints_on_resize_jiggle`).
const ECHO_SUPPRESS_MS: u64 = 500;

/// Per-instance bookkeeping for one live session.
///
/// Backend-agnostic: a session may be driven by the placeholder drain task
/// (chat / orchestrator until M2.4) or a real PTY child (CLI agents, M2.2).
/// Teardown is captured as a boxed `FnOnce` closure rather than tied to a
/// single tokio `AbortHandle`, so a PTY backend can kill its child + drop its
/// channels while the placeholder backend simply aborts its drain task.
pub struct LiveHandle {
    /// The `session` row id this live handle drives.
    session_id: String,
    /// Registration generation, stamped by [`Runtime::register`] (0 until then).
    /// Lets a backend's detached cleanup (`unregister_epoch`) tear down ONLY the
    /// registration it belongs to: after a programmatic kill → respawn on the
    /// same instance id, the OLD forwarder's late EOF must not unregister (and
    /// kill) the NEW generation's backend.
    epoch: u64,
    /// Sender half of the backend stdin channel; the spawned task / PTY writer
    /// owns the rx. This is the universal stdin path used by [`Runtime::send_stdin`].
    stdin_tx: UnboundedSender<String>,
    /// Teardown closure run exactly once on `unregister` / `Drop`. Kills the
    /// child / aborts the backend task and drops any owned channels.
    shutdown: Box<dyn FnOnce() + Send>,
    /// Resize the backing PTY to `(cols, rows)`. A no-op for backends without a
    /// PTY (chat / placeholder). Built in `pty.rs` so this module stays free of
    /// any `portable-pty` dependency (mirrors `shutdown`).
    resize: Box<dyn Fn(u16, u16) + Send>,
}

impl LiveHandle {
    /// Build a placeholder handle that holds the session "live" without a real
    /// backend: it spawns a drain task that consumes stdin and drops it. The
    /// shutdown closure aborts that task.
    ///
    /// Used for `chat` / `orchestrator` agents until M2.4 attaches the provider
    /// chat loop, and by the runtime unit tests.
    pub fn placeholder(session_id: &str) -> LiveHandle {
        // TODO(M2.4): replace the placeholder drain with the provider chat loop.
        let (stdin_tx, rx) = mpsc::unbounded_channel::<String>();
        let abort = spawn_placeholder_backend(rx);
        LiveHandle {
            session_id: session_id.to_owned(),
            epoch: 0,
            stdin_tx,
            shutdown: Box::new(move || abort.abort()),
            resize: Box::new(|_, _| {}),
        }
    }

    /// Test-only: like [`Self::placeholder`], but with NO drain task consuming
    /// the channel — the paired receiver is handed back so a test can
    /// `try_recv()`/`recv()` on it to observe (or prove the absence of)
    /// anything sent via [`Runtime::send_stdin`]. `placeholder`'s drain task
    /// swallows everything, making it impossible to tell "nothing was
    /// written" from "something was written and silently dropped" — which is
    /// exactly the distinction ADR 0006's self-triggered restart needs to
    /// prove (it must NOT inject into a mid-turn agent's own TUI).
    #[cfg(test)]
    pub(crate) fn for_test(session_id: &str) -> (LiveHandle, UnboundedReceiver<String>) {
        let (stdin_tx, rx) = mpsc::unbounded_channel::<String>();
        (
            LiveHandle {
                session_id: session_id.to_owned(),
                epoch: 0,
                stdin_tx,
                shutdown: Box::new(|| {}),
                resize: Box::new(|_, _| {}),
            },
            rx,
        )
    }
}

/// Live-session registry keyed by instance id (`workspace_agent` id).
pub struct Runtime {
    sessions: Mutex<HashMap<String, LiveHandle>>,
    /// Monotonic registration counter — the source of [`LiveHandle::epoch`].
    next_epoch: AtomicU64,
    /// Last-activity timestamp per instance — a NEW, orthogonal, in-memory-only
    /// signal (R-act-1): "working" iff a PTY/stream chunk arrived within the
    /// last `WORKING_WINDOW` (commands::instance). Independent of `sessions`
    /// (no session ⇒ no activity, but a live, quiet session has no entry
    /// either), so it gets its own mutex rather than living on `LiveHandle`.
    activity: Mutex<HashMap<String, std::time::SystemTime>>,
    /// Per-instance echo-suppression deadline, stamped by `send_stdin`/`resize`
    /// and consulted by [`Self::mark_activity_gated`] — see [`ECHO_SUPPRESS_MS`].
    /// A monotonic `Instant` (not `SystemTime`): this is a short rolling
    /// horizon compared purely against elapsed wall-clock time, never
    /// persisted or displayed, so it doesn't need to survive a clock jump.
    echo_suppress: Mutex<HashMap<String, std::time::Instant>>,
}

impl Runtime {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            next_epoch: AtomicU64::new(0),
            activity: Mutex::new(HashMap::new()),
            echo_suppress: Mutex::new(HashMap::new()),
        }
    }

    /// Record that `instance_id` just emitted output, timestamped now.
    /// Ungated — used directly by chat backends (no PTY, so no terminal echo
    /// to suppress: every streamed delta is genuine assistant output) and by
    /// the unit tests below. CLI/PTY backends go through
    /// [`Self::mark_activity_gated`] instead.
    pub fn mark_activity(&self, instance_id: &str) {
        let mut guard = self.activity.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(instance_id.to_owned(), std::time::SystemTime::now());
    }

    /// Gated variant of [`Self::mark_activity`] for PTY/CLI backends: a chunk
    /// arriving inside the rolling echo-suppression horizon armed by the most
    /// recent `send_stdin`/`resize` on this instance ([`ECHO_SUPPRESS_MS`]) is
    /// presumed to be the terminal's own echo of that input rather than
    /// genuine agent activity, so it is dropped WITHOUT stamping
    /// `last_activity` — stamping it would extend the working window on our
    /// own input and read the agent as "working" while it did nothing.
    /// Returns `true` if the chunk was stamped, `false` if it was suppressed.
    pub fn mark_activity_gated(&self, instance_id: &str) -> bool {
        let now = std::time::Instant::now();
        let suppressed = {
            let guard = self.echo_suppress.lock().unwrap_or_else(|e| e.into_inner());
            matches!(guard.get(instance_id), Some(until) if now < *until)
        };
        if suppressed {
            return false;
        }
        self.mark_activity(instance_id);
        true
    }

    /// Arm the echo-suppression horizon for `instance_id`: any chunk arriving
    /// before `now + ECHO_SUPPRESS_MS` will be dropped by
    /// [`Self::mark_activity_gated`]. Called from `send_stdin`/`resize` — the
    /// two ways OUR OWN input reaches the backend and could provoke an echo.
    fn arm_echo_suppress(&self, instance_id: &str) {
        let until = std::time::Instant::now() + std::time::Duration::from_millis(ECHO_SUPPRESS_MS);
        let mut guard = self.echo_suppress.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(instance_id.to_owned(), until);
    }

    /// The last-activity timestamp recorded for `instance_id`, if any.
    pub fn last_activity(&self, instance_id: &str) -> Option<std::time::SystemTime> {
        let guard = self.activity.lock().unwrap_or_else(|e| e.into_inner());
        guard.get(instance_id).copied()
    }

    /// Drop the activity entry for `instance_id` — a dead session must read as
    /// not-working immediately, not linger until its last stamp ages out.
    fn clear_activity(&self, instance_id: &str) {
        let mut guard = self.activity.lock().unwrap_or_else(|e| e.into_inner());
        guard.remove(instance_id);
        let mut es_guard = self.echo_suppress.lock().unwrap_or_else(|e| e.into_inner());
        es_guard.remove(instance_id);
    }

    /// Register a pre-built live session [`LiveHandle`] for `instance_id`.
    ///
    /// If the instance is already live, returns `None` and does nothing
    /// (idempotent — the caller decides what to do, and the passed `handle` is
    /// dropped, running its shutdown closure to tear down the now-orphaned
    /// backend). Otherwise stamps the handle with a fresh registration epoch,
    /// stores it, and returns `Some(epoch)`. The caller hands that epoch to the
    /// backend's detached cleanup so it can [`Self::unregister_epoch`] ONLY its
    /// own generation (see [`LiveHandle::epoch`]).
    ///
    /// The caller builds the backend (PTY child via [`pty::spawn_cli`], or the
    /// placeholder via [`LiveHandle::placeholder`]) and hands the resulting
    /// handle here, keeping this registry free of backend-specific concerns.
    ///
    /// `#[must_use]`: `Some`/`None` distinguishes "registered" from "already
    /// live". Ignoring it lets a caller racing a concurrent `register`
    /// double-persist status / double-emit events.
    #[must_use]
    pub fn register(&self, instance_id: &str, mut handle: LiveHandle) -> Option<u64> {
        // Decide under the guard, but run the orphaned handle's shutdown closure
        // (which may kill a PTY child) AFTER releasing the lock — the same
        // never-hold-the-mutex-across-teardown discipline as `unregister`.
        let (registered, displaced) = {
            let mut guard = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            if guard.contains_key(instance_id) {
                (None, Some(handle))
            } else {
                let epoch = self.next_epoch.fetch_add(1, Ordering::Relaxed) + 1;
                handle.epoch = epoch;
                guard.insert(instance_id.to_owned(), handle);
                (Some(epoch), None)
            }
        };
        // Already live: tear down the orphaned backend the caller just built
        // (its own `Drop` does not run the shutdown closure).
        if let Some(orphan) = displaced {
            (orphan.shutdown)();
        }
        registered
    }

    /// Remove and tear down the live session for `instance_id`.
    ///
    /// Returns `true` if a session existed (and its shutdown closure ran),
    /// `false` if there was nothing to remove. Dropping the handle also drops
    /// `stdin_tx`, closing the channel.
    ///
    /// `#[must_use]`: the bool reports whether this call (vs a racing one)
    /// performed the teardown; ignoring it can double-emit the idle status.
    #[must_use]
    pub fn unregister(&self, instance_id: &str) -> bool {
        // Take ownership of the handle under the lock, but run its (possibly
        // non-trivial) shutdown closure AFTER releasing the guard so we never
        // hold the mutex across backend teardown.
        let removed = {
            let mut guard = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            guard.remove(instance_id)
        };
        match removed {
            Some(handle) => {
                self.clear_activity(instance_id);
                (handle.shutdown)();
                true
            }
            None => false,
        }
    }

    /// Like [`Self::unregister`], but tears down the live session ONLY if its
    /// registration epoch matches `epoch` — i.e. it is still the same generation
    /// the caller belongs to. This is the safe cleanup for a backend's detached
    /// forwarder: after a kill → respawn on the same instance id, the OLD
    /// forwarder's late EOF finds a NEWER epoch registered and backs off instead
    /// of killing the fresh backend.
    ///
    /// `#[must_use]`: same double-teardown / double-emit rationale as
    /// [`Self::unregister`].
    #[must_use]
    pub fn unregister_epoch(&self, instance_id: &str, epoch: u64) -> bool {
        let removed = {
            let mut guard = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            match guard.get(instance_id) {
                Some(h) if h.epoch == epoch => guard.remove(instance_id),
                _ => None,
            }
        };
        match removed {
            Some(handle) => {
                self.clear_activity(instance_id);
                (handle.shutdown)();
                true
            }
            None => false,
        }
    }

    /// Return `true` if a session is currently live for `instance_id`.
    pub fn is_live(&self, instance_id: &str) -> bool {
        let guard = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        guard.contains_key(instance_id)
    }

    /// Number of live sessions in the registry.
    ///
    /// `#[allow(dead_code)]`: exercised by tests and future callers (M3 roster
    /// concurrency metrics); no production call site yet.
    #[allow(dead_code)]
    pub fn live_count(&self) -> usize {
        let guard = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        guard.len()
    }

    /// The live session id for `instance_id`, if any.
    pub fn session_id(&self, instance_id: &str) -> Option<String> {
        let guard = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        guard.get(instance_id).map(|h| h.session_id.clone())
    }

    /// Send a line of stdin to the live backend for `instance_id`.
    ///
    /// Returns [`StdinError::NotLive`] if the instance is not registered, or
    /// [`StdinError::Closed`] if the backend channel has been closed. Takes
    /// `&str` (the owned copy the channel needs is made here) so callers like
    /// `commands::message::send` don't clone just to satisfy this signature.
    pub fn send_stdin(&self, instance_id: &str, text: &str) -> Result<(), StdinError> {
        let guard = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let handle = guard.get(instance_id).ok_or(StdinError::NotLive)?;
        // Arm the echo-suppression horizon: this write itself can provoke a
        // PTY echo (keystroke echo, TUI redraw) that must not read as activity.
        self.arm_echo_suppress(instance_id);
        handle
            .stdin_tx
            .send(text.to_owned())
            .map_err(|_| StdinError::Closed)
    }

    /// Resize the live session's PTY to `(cols, rows)`. A no-op for backends
    /// without a PTY (chat / placeholder). Returns [`StdinError::NotLive`] if
    /// the instance is not registered. The resize runs under the registry lock,
    /// which is fine — `MasterPty::resize` is a fast, non-blocking ioctl.
    pub fn resize(&self, instance_id: &str, cols: u16, rows: u16) -> Result<(), StdinError> {
        let guard = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let handle = guard.get(instance_id).ok_or(StdinError::NotLive)?;
        // Arm the echo-suppression horizon: a resize raises SIGWINCH, which is
        // exactly the mount-jiggle / real-resize repaint that must not read as
        // activity (the proven root cause — see ECHO_SUPPRESS_MS's doc comment).
        self.arm_echo_suppress(instance_id);
        (handle.resize)(cols, rows);
        Ok(())
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Runtime {
    /// Tear down every in-flight backend on drop by running each handle's
    /// shutdown closure.
    ///
    /// For the placeholder backend this aborts its drain task; for a PTY
    /// backend it kills the child + aborts the writer task. Without this,
    /// replacing a `Runtime` mid-process (e.g. a test harness building multiple
    /// `AppState`s) would leak the spawned tasks / child processes until the
    /// whole tokio runtime shuts down.
    fn drop(&mut self) {
        // Take ownership of all handles under the lock, release it, THEN run the
        // shutdown closures (a PTY shutdown kills a child) — never under the
        // mutex. `Drop` has exclusive access so there's no contention, but this
        // keeps the "shutdown runs outside the lock" invariant uniform.
        let handles: Vec<LiveHandle> = {
            let mut guard = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            guard.drain().map(|(_, handle)| handle).collect()
        };
        {
            let mut guard = self.activity.lock().unwrap_or_else(|e| e.into_inner());
            guard.clear();
        }
        {
            let mut guard = self.echo_suppress.lock().unwrap_or_else(|e| e.into_inner());
            guard.clear();
        }
        for handle in handles {
            (handle.shutdown)();
        }
    }
}

/// Spawn the placeholder backend task and return its abort handle.
fn spawn_placeholder_backend(rx: UnboundedReceiver<String>) -> tokio::task::AbortHandle {
    let handle = tokio::spawn(async move {
        let mut rx = rx;
        // M2.2 (PTY) / M2.4 (chat loop) replace this body with the real driver
        // that feeds stdin into the child process / provider and streams output
        // back over the bus. For now we hold the channel open and drain — this
        // keeps the session "live" so send_stdin() succeeds, without emitting any
        // fake agent output. Messages received here are intentionally dropped
        // until a real backend is attached.
        while rx.recv().await.is_some() {}
    });
    handle.abort_handle()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn register_then_live() {
        let rt = Runtime::new();
        assert!(rt
            .register("inst-1", LiveHandle::placeholder("sess-1"))
            .is_some());
        assert!(rt.is_live("inst-1"));
        assert_eq!(rt.live_count(), 1);
        assert_eq!(rt.session_id("inst-1"), Some("sess-1".to_owned()));

        // Duplicate register is a no-op and returns None.
        assert!(rt
            .register("inst-1", LiveHandle::placeholder("sess-other"))
            .is_none());
        assert_eq!(rt.live_count(), 1);
        assert_eq!(rt.session_id("inst-1"), Some("sess-1".to_owned()));
    }

    #[tokio::test]
    async fn unregister_removes() {
        let rt = Runtime::new();
        assert!(rt
            .register("inst-1", LiveHandle::placeholder("sess-1"))
            .is_some());

        assert!(rt.unregister("inst-1"));
        assert!(!rt.is_live("inst-1"));
        assert_eq!(rt.live_count(), 0);

        // Unregistering again returns false.
        assert!(!rt.unregister("inst-1"));
    }

    /// The kill → respawn race guard: an OLD generation's epoch must not be able
    /// to unregister (and kill) the NEW generation registered under the same
    /// instance id, while the matching epoch still tears down normally.
    #[tokio::test]
    async fn unregister_epoch_only_matches_own_generation() {
        let rt = Runtime::new();
        let old = rt
            .register("inst-1", LiveHandle::placeholder("sess-1"))
            .expect("first register");

        // Kill + respawn (same instance id, same session id — a restart).
        assert!(rt.unregister("inst-1"));
        let new = rt
            .register("inst-1", LiveHandle::placeholder("sess-1"))
            .expect("respawn register");
        assert_ne!(old, new, "each registration gets a fresh epoch");

        // The old forwarder's late EOF: must back off, the new backend survives.
        assert!(!rt.unregister_epoch("inst-1", old));
        assert!(rt.is_live("inst-1"));

        // The new generation's own cleanup still works.
        assert!(rt.unregister_epoch("inst-1", new));
        assert!(!rt.is_live("inst-1"));
    }

    #[tokio::test]
    async fn send_stdin_live_vs_dead() {
        let rt = Runtime::new();

        // Not registered → NotLive.
        let err = rt.send_stdin("inst-1", "hi").unwrap_err();
        assert!(matches!(err, StdinError::NotLive));

        // Registered → Ok.
        assert!(rt
            .register("inst-1", LiveHandle::placeholder("sess-1"))
            .is_some());
        assert!(rt.send_stdin("inst-1", "hi").is_ok());
    }

    #[tokio::test]
    async fn activity_mark_then_read_roundtrip() {
        let rt = Runtime::new();
        assert_eq!(rt.last_activity("inst-1"), None);
        rt.mark_activity("inst-1");
        assert!(rt.last_activity("inst-1").is_some());
    }

    #[tokio::test]
    async fn activity_unknown_instance_is_none() {
        let rt = Runtime::new();
        assert_eq!(rt.last_activity("never-registered"), None);
    }

    #[tokio::test]
    async fn activity_cleared_on_unregister() {
        let rt = Runtime::new();
        rt.register("inst-1", LiveHandle::placeholder("sess-1"))
            .expect("register");
        rt.mark_activity("inst-1");
        assert!(rt.last_activity("inst-1").is_some());

        assert!(rt.unregister("inst-1"));
        assert_eq!(rt.last_activity("inst-1"), None);
    }

    #[tokio::test]
    async fn activity_cleared_on_unregister_epoch() {
        let rt = Runtime::new();
        let epoch = rt
            .register("inst-1", LiveHandle::placeholder("sess-1"))
            .expect("register");
        rt.mark_activity("inst-1");

        assert!(rt.unregister_epoch("inst-1", epoch));
        assert_eq!(rt.last_activity("inst-1"), None);
    }

    /// `bb plan:working-false-positive`: a chunk arriving right after our own
    /// `send_stdin` must be suppressed (it's presumed to be an echo), but the
    /// SAME instance stamps normally once the horizon elapses.
    #[tokio::test]
    async fn mark_activity_gated_suppresses_echo_after_send_stdin() {
        let rt = Runtime::new();
        rt.register("inst-1", LiveHandle::placeholder("sess-1"))
            .expect("register");

        rt.send_stdin("inst-1", "hi").expect("send_stdin");
        assert!(
            !rt.mark_activity_gated("inst-1"),
            "a chunk immediately after send_stdin must be suppressed as an echo"
        );
        assert_eq!(
            rt.last_activity("inst-1"),
            None,
            "a suppressed chunk must NOT stamp last_activity"
        );

        tokio::time::sleep(std::time::Duration::from_millis(ECHO_SUPPRESS_MS + 50)).await;
        assert!(
            rt.mark_activity_gated("inst-1"),
            "a chunk after the horizon elapses is genuine activity"
        );
        assert!(rt.last_activity("inst-1").is_some());
    }

    /// The same horizon, armed by `resize` instead of `send_stdin` — the
    /// mount-jiggle / real-window-resize path, proven empirically to provoke
    /// repaint chunks from an otherwise-idle agent
    /// (`runtime::pty::tests::idle_claude_repaints_on_resize_jiggle`).
    #[tokio::test]
    async fn mark_activity_gated_suppresses_echo_after_resize() {
        let rt = Runtime::new();
        rt.register("inst-1", LiveHandle::placeholder("sess-1"))
            .expect("register");

        rt.resize("inst-1", 80, 24).expect("resize");
        assert!(
            !rt.mark_activity_gated("inst-1"),
            "a repaint chunk immediately after resize must be suppressed"
        );

        tokio::time::sleep(std::time::Duration::from_millis(ECHO_SUPPRESS_MS + 50)).await;
        assert!(rt.mark_activity_gated("inst-1"));
    }

    /// Chat backends bypass the gate entirely — no PTY, so no terminal echo;
    /// `mark_activity` (ungated) always stamps regardless of any recent
    /// `send_stdin` on the same instance (the message that triggered the
    /// assistant's reply).
    #[tokio::test]
    async fn mark_activity_ungated_always_stamps_even_right_after_send_stdin() {
        let rt = Runtime::new();
        rt.register("inst-1", LiveHandle::placeholder("sess-1"))
            .expect("register");

        rt.send_stdin("inst-1", "hi").expect("send_stdin");
        rt.mark_activity("inst-1");
        assert!(rt.last_activity("inst-1").is_some());
    }

    /// Bulk single-threaded registration — 16 distinct instances coexist.
    #[tokio::test]
    async fn bulk_register_many() {
        let rt = Runtime::new();
        for i in 0..16 {
            let inst = format!("inst-{i}");
            assert!(rt
                .register(&inst, LiveHandle::placeholder(&format!("sess-{i}")))
                .is_some());
        }
        assert_eq!(rt.live_count(), 16);
        for i in 0..16 {
            assert!(rt.is_live(&format!("inst-{i}")));
        }
    }

    /// Real contention: many tasks hit the shared `Mutex` concurrently. Each
    /// task registers a distinct instance, then half of them unregister. The
    /// final `live_count` must be exact — proving the registry is race-free
    /// under the ≥8-concurrent-session NFR.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_register_unregister() {
        let rt = Arc::new(Runtime::new());
        let mut handles = Vec::new();

        for i in 0..32 {
            let rt = Arc::clone(&rt);
            handles.push(tokio::spawn(async move {
                let inst = format!("inst-{i}");
                assert!(rt
                    .register(&inst, LiveHandle::placeholder(&format!("sess-{i}")))
                    .is_some());
                // Odd-numbered instances tear themselves down again.
                if i % 2 == 1 {
                    assert!(rt.unregister(&inst));
                }
            }));
        }
        for h in handles {
            h.await.expect("task panicked");
        }

        // 32 registered, 16 odd ones unregistered → 16 even ones remain.
        assert_eq!(rt.live_count(), 16);
        for i in (0..32).step_by(2) {
            assert!(rt.is_live(&format!("inst-{i}")));
        }
        for i in (1..32).step_by(2) {
            assert!(!rt.is_live(&format!("inst-{i}")));
        }
    }
}
