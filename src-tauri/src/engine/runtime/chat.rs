//! Chat-agent backend: the provider chat loop.
//!
//! This is the chat counterpart to [`super::pty`]'s PTY/CLI backend. Instead of
//! a child process inside a pseudo-terminal, it drives a provider streaming API
//! (`super::provider`) in a loop: each user turn arriving on the stdin channel
//! is appended to an in-memory history, sent to the provider, and the streamed
//! assistant-text deltas are pushed onto the bounded `output_rx` — exactly the
//! channel shape the shared output forwarder already bridges onto the bus.
//!
//! It has ZERO dependency on the database or Tauri: the command handler bridges
//! `output_rx` onto the event bus and the registry tracks the [`LiveHandle`].
//!
//! # Lifecycle
//!
//! - User input flows `message.send` → `Runtime::send_stdin` → `stdin_tx` →
//!   this loop's `stdin_rx` (the SAME path the placeholder/PTY backends use).
//! - When the session is stopped, the registry drops the [`LiveHandle`] (and
//!   its `stdin_tx`); `stdin_rx.recv()` then returns `None`, the loop ends, and
//!   `output_tx` drops → the forwarder sees EOF and marks the instance idle.
//! - The `shutdown` closure aborts the loop task directly (dropping its
//!   channels) for an immediate teardown without waiting on an in-flight stream.

use super::provider::{ChatMessage, ChatRole, Provider};
use super::LiveHandle;
use tokio::sync::mpsc::{self, Receiver};

/// Bound on buffered output chunks, mirroring [`super::pty`]. The provider
/// stream `send`s deltas onto this channel, so a slow forwarder applies natural
/// backpressure (the stream loop awaits the send) instead of buffering deltas
/// unbounded in heap.
const OUTPUT_CHANNEL_CAP: usize = 1024;

/// A spawned chat backend: the [`LiveHandle`] to register in the runtime plus
/// the output stream the handler forwards onto the bus. Structurally parallel
/// to [`super::pty::CliBackend`].
pub struct ChatBackend {
    /// Register this in the [`crate::engine::runtime::Runtime`].
    pub handle: LiveHandle,
    /// Drained by the forwarder task; closes (recv → None) when the loop ends.
    pub output_rx: Receiver<String>,
}

/// Spawn the chat loop for `session_id`, driving `provider`/`model`.
///
/// Returns immediately; the loop runs as a detached tokio task that owns the
/// stdin receiver, output sender, provider, model, and conversation history.
pub fn spawn_chat(session_id: &str, provider: Provider, model: String) -> ChatBackend {
    let (output_tx, output_rx) = mpsc::channel::<String>(OUTPUT_CHANNEL_CAP);
    let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<String>();

    let loop_task = tokio::spawn(async move {
        let mut history: Vec<ChatMessage> = Vec::new();
        while let Some(user_text) = stdin_rx.recv().await {
            // Strip the terminal-style trailing newline send_stdin lines carry.
            let user_text = user_text.trim_end().to_string();
            if user_text.is_empty() {
                continue;
            }
            history.push(ChatMessage {
                role: ChatRole::User,
                content: user_text,
            });
            match provider.stream_chat(&model, &history, &output_tx).await {
                Ok(assistant) => history.push(ChatMessage {
                    role: ChatRole::Assistant,
                    content: assistant,
                }),
                // Surface provider errors as an output chunk so the user sees
                // them; keep the loop alive so they can retry.
                Err(e) => {
                    let _ = output_tx.send(format!("\n[provider error] {e}\n")).await;
                }
            }
        }
    });
    let loop_abort = loop_task.abort_handle();

    let handle = LiveHandle {
        session_id: session_id.to_owned(),
        stdin_tx,
        shutdown: Box::new(move || loop_abort.abort()),
    };
    ChatBackend { handle, output_rx }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::runtime::Runtime;

    /// The chat loop forwards a mock provider's deltas onto `output_rx` for a
    /// single user turn. We register the backend in a `Runtime` and feed input
    /// through the SAME `send_stdin` path the real handler uses, then drop the
    /// runtime (dropping `stdin_tx` + aborting the loop) so `output_rx` closes
    /// and the drain terminates without a timer.
    #[tokio::test]
    async fn chat_loop_streams_and_accumulates() {
        let backend = spawn_chat(
            "s1",
            Provider::Mock {
                deltas: vec!["Hel".into(), "lo".into()],
            },
            "m".into(),
        );
        let mut output_rx = backend.output_rx;

        let rt = Runtime::new();
        assert!(rt.register("inst-1", backend.handle));
        rt.send_stdin("inst-1", "hi").expect("send_stdin");

        // Collect exactly the two deltas this single turn produces, then tear
        // down the runtime to close the output channel.
        let mut collected = String::new();
        for _ in 0..2 {
            let d = output_rx.recv().await.expect("delta");
            collected.push_str(&d);
        }
        assert_eq!(collected, "Hello");

        drop(rt); // aborts the loop task, closing output_rx.
        assert!(output_rx.recv().await.is_none());
    }

    /// History accumulates ACROSS turns: the `EchoLen` provider streams the
    /// history length it was handed, so turn 1 sees `[user]` (len 1) and turn 2
    /// sees `[user, assistant, user]` (len 3). A regression that cleared history
    /// between turns would make turn 2 report "1" and fail this test.
    #[tokio::test]
    async fn chat_loop_accumulates_history_across_turns() {
        let backend = spawn_chat("s1", Provider::EchoLen, "m".into());
        let mut output_rx = backend.output_rx;

        let rt = Runtime::new();
        assert!(rt.register("inst-1", backend.handle));

        rt.send_stdin("inst-1", "first").expect("send_stdin 1");
        assert_eq!(output_rx.recv().await.as_deref(), Some("1"));

        rt.send_stdin("inst-1", "second").expect("send_stdin 2");
        assert_eq!(
            output_rx.recv().await.as_deref(),
            Some("3"),
            "turn 2 must see user+assistant+user — history carried forward"
        );

        drop(rt);
    }
}
