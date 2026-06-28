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
pub mod provider;
pub mod pty;

use std::collections::HashMap;
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
            stdin_tx,
            shutdown: Box::new(move || abort.abort()),
            resize: Box::new(|_, _| {}),
        }
    }
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

    /// Register a pre-built live session [`LiveHandle`] for `instance_id`.
    ///
    /// If the instance is already live, returns `false` and does nothing
    /// (idempotent — the caller decides what to do, and the passed `handle` is
    /// dropped, running its shutdown closure to tear down the now-orphaned
    /// backend). Otherwise stores the handle and returns `true`.
    ///
    /// The caller builds the backend (PTY child via [`pty::spawn_cli`], or the
    /// placeholder via [`LiveHandle::placeholder`]) and hands the resulting
    /// handle here, keeping this registry free of backend-specific concerns.
    ///
    /// `#[must_use]`: the bool distinguishes "registered" from "already live".
    /// Ignoring it lets a caller racing a concurrent `register` double-persist
    /// status / double-emit events.
    #[must_use]
    pub fn register(&self, instance_id: &str, handle: LiveHandle) -> bool {
        // Decide under the guard, but run the orphaned handle's shutdown closure
        // (which may kill a PTY child) AFTER releasing the lock — the same
        // never-hold-the-mutex-across-teardown discipline as `unregister`.
        let displaced = {
            let mut guard = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            if guard.contains_key(instance_id) {
                Some(handle)
            } else {
                guard.insert(instance_id.to_owned(), handle);
                None
            }
        };
        match displaced {
            // Already live: tear down the orphaned backend the caller just built
            // (its own `Drop` does not run the shutdown closure).
            Some(orphan) => {
                (orphan.shutdown)();
                false
            }
            None => true,
        }
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
        assert!(rt.register("inst-1", LiveHandle::placeholder("sess-1")));
        assert!(rt.is_live("inst-1"));
        assert_eq!(rt.live_count(), 1);
        assert_eq!(rt.session_id("inst-1"), Some("sess-1".to_owned()));

        // Duplicate register is a no-op and returns false.
        assert!(!rt.register("inst-1", LiveHandle::placeholder("sess-other")));
        assert_eq!(rt.live_count(), 1);
        assert_eq!(rt.session_id("inst-1"), Some("sess-1".to_owned()));
    }

    #[tokio::test]
    async fn unregister_removes() {
        let rt = Runtime::new();
        assert!(rt.register("inst-1", LiveHandle::placeholder("sess-1")));

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
        let err = rt.send_stdin("inst-1", "hi").unwrap_err();
        assert!(matches!(err, StdinError::NotLive));

        // Registered → Ok.
        assert!(rt.register("inst-1", LiveHandle::placeholder("sess-1")));
        assert!(rt.send_stdin("inst-1", "hi").is_ok());
    }

    /// Bulk single-threaded registration — 16 distinct instances coexist.
    #[tokio::test]
    async fn bulk_register_many() {
        let rt = Runtime::new();
        for i in 0..16 {
            let inst = format!("inst-{i}");
            assert!(rt.register(&inst, LiveHandle::placeholder(&format!("sess-{i}"))));
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
                assert!(rt.register(&inst, LiveHandle::placeholder(&format!("sess-{i}"))));
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
