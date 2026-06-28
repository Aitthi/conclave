//! PTY-backed CLI agent backend.
//!
//! This module owns ALL `portable-pty` usage. It spawns a CLI process (e.g.
//! `claude`, `codex`) inside a pseudo-terminal rooted at the workspace folder,
//! and exposes the child's output and stdin purely as channels + a teardown
//! closure via [`CliBackend`] / [`LiveHandle`]. It has ZERO dependency on the
//! database or Tauri: the command handler bridges `output_rx` onto the event
//! bus and the registry tracks the [`LiveHandle`].
//!
//! # Threading
//!
//! `portable-pty`'s reader/writer are blocking, synchronous handles:
//! - The **reader** is drained on a dedicated OS thread (`std::thread::spawn`)
//!   that pushes UTF-8 chunks onto `output_tx`. When the child exits the read
//!   hits EOF, the loop ends, and `output_tx` is dropped — signalling EOF to
//!   the forwarder draining `output_rx`.
//! - The **writer** is driven by a tokio task that awaits stdin lines and
//!   writes them under `block_in_place` (hence the multi-thread runtime).
//!
//! Teardown kills the child (forcing reader EOF) and aborts the writer task.

use super::LiveHandle;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use tokio::sync::mpsc::{self, Receiver};

/// Bound on buffered output chunks. The reader thread `blocking_send`s onto this
/// channel, so a slow forwarder applies natural backpressure (the reader thread
/// stalls, and behind it the PTY's own kernel buffer) instead of letting output
/// accumulate unbounded in heap. ~4 KB/chunk → ~4 MB ceiling.
const OUTPUT_CHANNEL_CAP: usize = 1024;

/// A spawned CLI PTY backend: the [`LiveHandle`] to register in the runtime
/// plus the output stream the handler forwards onto the bus.
pub struct CliBackend {
    /// Register this in the [`crate::engine::runtime::Runtime`].
    pub handle: LiveHandle,
    /// Drained by the forwarder task; closes (recv → None) when the child exits.
    pub output_rx: Receiver<String>,
}

/// Spawn `command` with `args` inside a PTY rooted at `cwd`, streaming its
/// combined stdout/stderr back through [`CliBackend::output_rx`].
///
/// Returns an `io::Error` if the PTY cannot be opened or the command fails to
/// spawn (e.g. binary not found).
pub fn spawn_cli(
    session_id: &str,
    command: &str,
    args: &[String],
    cwd: &str,
) -> std::io::Result<CliBackend> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(to_io_err)?;

    let mut cmd = CommandBuilder::new(command);
    cmd.args(args);
    cmd.cwd(cwd);

    let child = pair.slave.spawn_command(cmd).map_err(to_io_err)?;
    // The slave is no longer needed once the child owns it; dropping it lets the
    // master see EOF once the child exits.
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().map_err(to_io_err)?;
    let mut writer = pair.master.take_writer().map_err(to_io_err)?;
    // Keep the master alive for the lifetime of the backend, behind Arc<Mutex>
    // so the resize closure (below) and the shutdown closure can both hold it.
    // `MasterPty::resize` only needs `&self`; the Mutex guards concurrent access.
    let master = std::sync::Arc::new(std::sync::Mutex::new(pair.master));

    let (output_tx, output_rx) = mpsc::channel::<String>(OUTPUT_CHANNEL_CAP);
    let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<String>();

    // ── Reader OS thread ────────────────────────────────────────────────────
    // Blocking reads off the PTY master; pushes decoded chunks onto output_tx.
    // `blocking_send` (this is a plain OS thread, never inside the async runtime)
    // applies backpressure when the forwarder lags. When the loop ends, output_tx
    // is dropped at scope exit — that close signals EOF to the forwarder.
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
                    if output_tx.blocking_send(chunk).is_err() {
                        break; // receiver gone
                    }
                }
            }
        }
    });

    // ── Stdin writer task ───────────────────────────────────────────────────
    // Awaits stdin lines, writes them to the PTY master under block_in_place
    // (the write is blocking and requires the multi-thread runtime).
    let writer_task = tokio::spawn(async move {
        while let Some(s) = stdin_rx.recv().await {
            tokio::task::block_in_place(|| {
                if let Err(e) = writer.write_all(s.as_bytes()).and_then(|()| writer.flush()) {
                    // Fire-and-forget stdin: a write failure usually means the
                    // child closed its end. Surface it in debug builds so a
                    // "my input isn't reaching the agent" bug is diagnosable.
                    #[cfg(debug_assertions)]
                    eprintln!("[pty] stdin write failed: {e}");
                    let _ = &e;
                }
            });
        }
    });
    let writer_abort = writer_task.abort_handle();

    // ── Resize ────────────────────────────────────────────────────────────────
    // Forward (cols, rows) from the frontend's xterm fit to the PTY master so a
    // full-screen TUI (Claude Code, etc.) lays out at the real on-screen size
    // instead of the 80×24 default — without a matching size the redraws garble.
    let resize_master = std::sync::Arc::clone(&master);
    let resize: Box<dyn Fn(u16, u16) + Send> = Box::new(move |cols, rows| {
        if let Ok(m) = resize_master.lock() {
            let _ = m.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    });

    // ── Teardown ────────────────────────────────────────────────────────────
    // Killing the child makes the reader hit EOF and exit its thread on its own;
    // aborting the writer task stops stdin. We deliberately do NOT join the
    // reader thread here (that could deadlock if called from async). The master
    // is moved in so it outlives the reader/writer and is dropped on teardown.
    let mut child = child;
    let shutdown = Box::new(move || {
        // kill() is a no-op (ESRCH, ignored) if the child already exited on its
        // own. wait() reaps it so a dead child doesn't linger as a zombie in the
        // process table until app exit — on the natural-exit path the child is
        // already dead so this returns immediately; on a fresh SIGKILL it returns
        // in microseconds.
        let _ = child.kill();
        let _ = child.wait();
        writer_abort.abort();
        drop(master);
    });

    let handle = LiveHandle {
        session_id: session_id.to_owned(),
        stdin_tx,
        shutdown,
        resize,
    };
    Ok(CliBackend { handle, output_rx })
}

/// Map a `portable-pty` error (boxed `anyhow`/`std::error::Error`) into an
/// `io::Error` so `spawn_cli` exposes a `std::io::Result`.
fn to_io_err<E: std::fmt::Display>(e: E) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A short-lived command writes to the PTY; we collect everything until the
    /// channel closes (the child exits → reader EOF → output_tx dropped) and
    /// assert the payload made it through. Termination is bounded by channel
    /// close, not a timer (the tokio `time` feature is intentionally off).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spawn_cli_streams_output() {
        let mut backend = spawn_cli(
            "s1",
            "sh",
            &["-c".into(), "printf 'hello-pty'".into()],
            "/tmp",
        )
        .expect("spawn_cli failed");

        // `sh -c printf` exits promptly, closing output_rx — this recv loop ends
        // on its own without any timer.
        let mut collected = String::new();
        while let Some(c) = backend.output_rx.recv().await {
            collected.push_str(&c);
        }

        assert!(
            collected.contains("hello-pty"),
            "expected output to contain 'hello-pty', got: {collected:?}"
        );
    }

    /// A bogus command must fail to spawn with an `Err` (binary not found),
    /// deterministically and without any external dependency.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spawn_cli_bad_command_errors() {
        let result = spawn_cli("s2", "definitely-not-a-real-binary-xyz", &[], "/tmp");
        assert!(result.is_err(), "bogus command should fail to spawn");
    }
}
