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

use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::task::AbortHandle;

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

/// Per-instance bookkeeping for one live session.
struct LiveHandle {
    /// The `session` row id this live handle drives.
    session_id: String,
    /// Sender half of the backend stdin channel; the spawned task owns the rx.
    stdin_tx: UnboundedSender<String>,
    /// Abort handle for the spawned backend task.
    abort: AbortHandle,
}

/// Live-session registry keyed by instance id (`workspace_agent` id).
pub struct Runtime {
    sessions: Mutex<HashMap<String, LiveHandle>>,
}

impl Runtime {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// Register a new live session for `instance_id`.
    ///
    /// If the instance is already live, returns `false` and does nothing
    /// (idempotent — the caller decides what to do). Otherwise creates the
    /// stdin channel, spawns the placeholder backend task (which owns the
    /// receiver), stores the handle, and returns `true`.
    ///
    /// `#[must_use]`: the bool distinguishes "registered" from "already live".
    /// Ignoring it lets a caller racing a concurrent `register` double-persist
    /// status / double-emit events.
    #[must_use]
    pub fn register(&self, instance_id: &str, session_id: &str) -> bool {
        let mut guard = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        if guard.contains_key(instance_id) {
            return false;
        }
        // TODO(M2.2): switch to `mpsc::channel(n)` once the backend applies
        // backpressure (PTY write buffer / rate-limited provider). Unbounded is
        // fine only while the placeholder backend drains immediately.
        let (stdin_tx, rx) = mpsc::unbounded_channel::<String>();
        let abort = spawn_backend(rx);
        guard.insert(
            instance_id.to_owned(),
            LiveHandle {
                session_id: session_id.to_owned(),
                stdin_tx,
                abort,
            },
        );
        true
    }

    /// Remove and abort the live session for `instance_id`.
    ///
    /// Returns `true` if a session existed (and was aborted), `false` if there
    /// was nothing to remove. Dropping the handle also drops `stdin_tx`,
    /// closing the channel.
    ///
    /// `#[must_use]`: the bool reports whether this call (vs a racing one)
    /// performed the teardown; ignoring it can double-emit the idle status.
    #[must_use]
    pub fn unregister(&self, instance_id: &str) -> bool {
        let mut guard = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        match guard.remove(instance_id) {
            Some(handle) => {
                handle.abort.abort();
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
    /// [`StdinError::Closed`] if the backend channel has been closed.
    ///
    /// No production caller yet — message routing arrives in M3 — but the path
    /// is wired and unit-tested now.
    #[allow(dead_code)]
    pub fn send_stdin(&self, instance_id: &str, text: String) -> Result<(), StdinError> {
        let guard = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        let handle = guard.get(instance_id).ok_or(StdinError::NotLive)?;
        handle.stdin_tx.send(text).map_err(|_| StdinError::Closed)
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Runtime {
    /// Abort every in-flight backend task on drop.
    ///
    /// Dropping an `AbortHandle` does NOT abort its task — only `.abort()`
    /// does. Without this, replacing a `Runtime` mid-process (e.g. a test
    /// harness building multiple `AppState`s) would leak the spawned tasks
    /// until the whole tokio runtime shuts down.
    fn drop(&mut self) {
        let mut guard = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        for (_, handle) in guard.drain() {
            handle.abort.abort();
        }
    }
}

/// Spawn the placeholder backend task and return its abort handle.
fn spawn_backend(rx: UnboundedReceiver<String>) -> AbortHandle {
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
        assert!(rt.register("inst-1", "sess-1"));
        assert!(rt.is_live("inst-1"));
        assert_eq!(rt.live_count(), 1);
        assert_eq!(rt.session_id("inst-1"), Some("sess-1".to_owned()));

        // Duplicate register is a no-op and returns false.
        assert!(!rt.register("inst-1", "sess-other"));
        assert_eq!(rt.live_count(), 1);
        assert_eq!(rt.session_id("inst-1"), Some("sess-1".to_owned()));
    }

    #[tokio::test]
    async fn unregister_removes() {
        let rt = Runtime::new();
        assert!(rt.register("inst-1", "sess-1"));

        assert!(rt.unregister("inst-1"));
        assert!(!rt.is_live("inst-1"));
        assert_eq!(rt.live_count(), 0);

        // Unregistering again returns false.
        assert!(!rt.unregister("inst-1"));
    }

    #[tokio::test]
    async fn send_stdin_live_vs_dead() {
        let rt = Runtime::new();

        // Not registered → NotLive.
        let err = rt.send_stdin("inst-1", "hi".to_owned()).unwrap_err();
        assert!(matches!(err, StdinError::NotLive));

        // Registered → Ok.
        assert!(rt.register("inst-1", "sess-1"));
        assert!(rt.send_stdin("inst-1", "hi".to_owned()).is_ok());
    }

    /// Bulk single-threaded registration — 16 distinct instances coexist.
    #[tokio::test]
    async fn bulk_register_many() {
        let rt = Runtime::new();
        for i in 0..16 {
            let inst = format!("inst-{i}");
            let sess = format!("sess-{i}");
            assert!(rt.register(&inst, &sess));
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
                assert!(rt.register(&inst, &format!("sess-{i}")));
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
