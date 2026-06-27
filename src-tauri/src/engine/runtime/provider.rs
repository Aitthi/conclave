//! Provider clients for the chat-agent backend.
//!
//! This module owns ALL provider HTTP/SSE concerns: it builds a streaming
//! request to a chat-completions API (Anthropic Messages, or any
//! OpenAI-compatible `/chat/completions` endpoint — OpenAI proper, or a local
//! Ollama-style server), parses the Server-Sent-Events stream, and forwards
//! each assistant TEXT delta onto an `mpsc::Sender<String>`. It has ZERO
//! dependency on the database, Tauri, or the runtime registry: the chat loop
//! (`runtime::chat`) drives it and the command handler bridges output onto the
//! bus.
//!
//! API keys come from environment variables for now (`ANTHROPIC_API_KEY`,
//! `OPENAI_API_KEY`); the Keychain/Settings path is a later milestone.
//!
//! # Testing
//!
//! The SSE line-payload parsers ([`anthropic_text_delta`] / [`openai_text_delta`])
//! are pure functions covered by unit tests. The real reqwest HTTP/SSE
//! round-trip is intentionally NOT unit-tested here (it would need a mock HTTP
//! server); that is left to a deferred integration test. Test code drives the
//! chat loop through the `#[cfg(test)] Provider::Mock` variant, which yields
//! canned deltas without any network I/O.

use std::sync::OnceLock;
use tokio::sync::mpsc;

/// Role of a chat message. `System` prompts are handled separately if needed;
/// the conversation history the chat loop accumulates is User/Assistant turns.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChatRole {
    User,
    Assistant,
}

impl ChatRole {
    /// Wire string for both Anthropic and OpenAI request bodies.
    fn as_wire(self) -> &'static str {
        match self {
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
        }
    }
}

/// One message in the chat history.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

/// Failure modes of a provider streaming call.
#[derive(Debug)]
pub enum ProviderError {
    /// No usable API key / no provider configured.
    MissingKey(String),
    /// The HTTP request failed to build / connect / returned a non-success
    /// status.
    Http(String),
    /// The connection succeeded but reading/decoding the SSE body failed.
    Stream(String),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::MissingKey(m) => write!(f, "missing API key: {m}"),
            ProviderError::Http(m) => write!(f, "http error: {m}"),
            ProviderError::Stream(m) => write!(f, "stream error: {m}"),
        }
    }
}

impl std::error::Error for ProviderError {}

const ANTHROPIC_DEFAULT_BASE: &str = "https://api.anthropic.com";
const OPENAI_DEFAULT_BASE: &str = "https://api.openai.com/v1";
const LOCAL_DEFAULT_BASE: &str = "http://localhost:11434/v1";

/// Max tokens requested per Anthropic completion (the API requires the field).
const ANTHROPIC_MAX_TOKENS: u32 = 4096;

/// A resolved provider ready to stream a chat completion.
pub enum Provider {
    /// Anthropic Messages API.
    Anthropic { api_key: String, base_url: String },
    /// OpenAI-compatible `/chat/completions` API. Also used for local/Ollama
    /// servers (which speak the same protocol and often need no key).
    OpenAi { api_key: String, base_url: String },
    /// Test-only provider that yields canned deltas without any HTTP.
    #[cfg(test)]
    Mock { deltas: Vec<String> },
    /// Test-only provider that streams the length of the history it was given —
    /// lets a test prove the chat loop accumulates history across turns.
    #[cfg(test)]
    EchoLen,
}

impl Provider {
    /// Resolve a provider from a `provider_id` (the agent definition's
    /// `provider_id` column) plus environment variables for keys / base URLs.
    ///
    /// Returns [`ProviderError::MissingKey`] when the required key env var is
    /// unset/empty (or when no provider is configured at all).
    pub fn from_config(provider_id: Option<&str>) -> Result<Provider, ProviderError> {
        match provider_id {
            Some("anthropic") => {
                let api_key = non_empty_env("ANTHROPIC_API_KEY").ok_or_else(|| {
                    ProviderError::MissingKey("ANTHROPIC_API_KEY is not set".into())
                })?;
                let base_url = non_empty_env("CONCLAVE_ANTHROPIC_BASE_URL")
                    .unwrap_or_else(|| ANTHROPIC_DEFAULT_BASE.to_owned());
                Ok(Provider::Anthropic { api_key, base_url })
            }
            Some("openai") => {
                let api_key = non_empty_env("OPENAI_API_KEY")
                    .ok_or_else(|| ProviderError::MissingKey("OPENAI_API_KEY is not set".into()))?;
                let base_url = non_empty_env("CONCLAVE_OPENAI_BASE_URL")
                    .unwrap_or_else(|| OPENAI_DEFAULT_BASE.to_owned());
                Ok(Provider::OpenAi { api_key, base_url })
            }
            Some("local") => {
                // Local servers (Ollama et al.) are OpenAI-compatible and often
                // need no key — default to empty rather than erroring.
                let api_key = non_empty_env("OPENAI_API_KEY").unwrap_or_default();
                let base_url = non_empty_env("CONCLAVE_LOCAL_BASE_URL")
                    .unwrap_or_else(|| LOCAL_DEFAULT_BASE.to_owned());
                Ok(Provider::OpenAi { api_key, base_url })
            }
            _ => Err(ProviderError::MissingKey("no provider configured".into())),
        }
    }

    /// Stream a chat completion for `messages`, forwarding each assistant TEXT
    /// delta to `tx`, and return the full accumulated assistant text.
    ///
    /// `tx` send errors (the receiver was dropped) stop the stream early — the
    /// accumulated text so far is still returned.
    pub async fn stream_chat(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tx: &mpsc::Sender<String>,
    ) -> Result<String, ProviderError> {
        match self {
            Provider::Anthropic { api_key, base_url } => {
                stream_anthropic(api_key, base_url, model, messages, tx).await
            }
            Provider::OpenAi { api_key, base_url } => {
                stream_openai(api_key, base_url, model, messages, tx).await
            }
            #[cfg(test)]
            Provider::Mock { deltas } => {
                let mut acc = String::new();
                for d in deltas {
                    acc.push_str(d);
                    if tx.send(d.clone()).await.is_err() {
                        break; // receiver gone
                    }
                }
                Ok(acc)
            }
            #[cfg(test)]
            Provider::EchoLen => {
                let s = messages.len().to_string();
                let _ = tx.send(s.clone()).await;
                Ok(s)
            }
        }
    }
}

/// Read an env var, returning `None` if it is unset OR empty.
fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

// ── SSE payload parsers (pure, unit-testable) ────────────────────────────────

/// Parse one Anthropic SSE `data:` JSON payload, returning `Some(text)` for a
/// `content_block_delta` carrying a `text_delta`, else `None` (other event
/// types: `message_start`, `ping`, `content_block_start`, etc.).
pub(crate) fn anthropic_text_delta(data_json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(data_json).ok()?;
    if v.get("type")?.as_str()? != "content_block_delta" {
        return None;
    }
    let delta = v.get("delta")?;
    if delta.get("type")?.as_str()? != "text_delta" {
        return None;
    }
    Some(delta.get("text")?.as_str()?.to_owned())
}

/// Parse one OpenAI SSE `data:` JSON payload, returning `Some(text)` for a
/// `choices[0].delta.content` string, else `None` (role-only deltas, finish
/// chunks, the `[DONE]` sentinel handled by the caller, etc.).
pub(crate) fn openai_text_delta(data_json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(data_json).ok()?;
    let content = v
        .get("choices")?
        .get(0)?
        .get("delta")?
        .get("content")?
        .as_str()?;
    Some(content.to_owned())
}

// ── HTTP streaming drivers ───────────────────────────────────────────────────

/// Shared HTTP client (reqwest's client is internally `Arc`, cheap to clone and
/// designed to be reused — a per-call client would re-handshake TLS every turn).
///
/// `connect_timeout` bounds a dead endpoint; `read_timeout` is an IDLE timeout
/// between successive body bytes — it aborts a stalled SSE stream (server sent
/// headers then went silent) WITHOUT capping a legitimately long, still-flowing
/// generation, so a hung stream can't strand the instance in `running` forever.
fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .read_timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_default()
    })
}

/// Map a reqwest error into the right [`ProviderError`] variant.
fn http_err(e: reqwest::Error) -> ProviderError {
    ProviderError::Http(e.to_string())
}

/// Drive an Anthropic Messages streaming request.
async fn stream_anthropic(
    api_key: &str,
    base_url: &str,
    model: &str,
    messages: &[ChatMessage],
    tx: &mpsc::Sender<String>,
) -> Result<String, ProviderError> {
    let body = serde_json::json!({
        "model": model,
        "max_tokens": ANTHROPIC_MAX_TOKENS,
        "stream": true,
        "messages": messages
            .iter()
            .map(|m| serde_json::json!({ "role": m.role.as_wire(), "content": m.content }))
            .collect::<Vec<_>>(),
    });

    let url = format!("{}/v1/messages", base_url.trim_end_matches('/'));
    let resp = http_client()
        .post(url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(http_err)?
        .error_for_status()
        .map_err(http_err)?;

    consume_sse(resp, tx, anthropic_text_delta).await
}

/// Drive an OpenAI-compatible `/chat/completions` streaming request.
async fn stream_openai(
    api_key: &str,
    base_url: &str,
    model: &str,
    messages: &[ChatMessage],
    tx: &mpsc::Sender<String>,
) -> Result<String, ProviderError> {
    let body = serde_json::json!({
        "model": model,
        "stream": true,
        "messages": messages
            .iter()
            .map(|m| serde_json::json!({ "role": m.role.as_wire(), "content": m.content }))
            .collect::<Vec<_>>(),
    });

    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let mut req = http_client()
        .post(url)
        .header("content-type", "application/json");
    if !api_key.is_empty() {
        req = req.header("authorization", format!("Bearer {api_key}"));
    }
    let resp = req
        .json(&body)
        .send()
        .await
        .map_err(http_err)?
        .error_for_status()
        .map_err(http_err)?;

    consume_sse(resp, tx, openai_text_delta).await
}

/// Consume an SSE response: accumulate RAW BYTES, split on the `\n` byte, and
/// for each `data:` line feed the JSON payload to `parse` and forward any
/// `Some(text)` to `tx`. Keeps a leftover partial-line buffer across network
/// chunks. The `[DONE]` sentinel is skipped. Returns the full accumulated text.
///
/// Decoding happens per COMPLETE LINE, not per network chunk: `0x0A` never
/// appears inside a multi-byte UTF-8 sequence, so line boundaries are safe to
/// find in raw bytes, and each whole line decodes as valid UTF-8. Decoding a raw
/// chunk directly would split a multi-byte codepoint at a chunk boundary into
/// `U+FFFD` replacement chars — corrupting non-ASCII (CJK/Thai/emoji) deltas.
async fn consume_sse(
    resp: reqwest::Response,
    tx: &mpsc::Sender<String>,
    parse: fn(&str) -> Option<String>,
) -> Result<String, ProviderError> {
    use futures_util::StreamExt;

    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    let mut acc = String::new();

    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| ProviderError::Stream(e.to_string()))?;
        buf.extend_from_slice(&bytes);

        for line in drain_lines(&mut buf) {
            if let Some(text) = parse_data_line(&line, parse) {
                acc.push_str(&text);
                if tx.send(text).await.is_err() {
                    return Ok(acc); // receiver gone → stop early
                }
            }
        }
    }

    // Flush any trailing line without a final newline.
    let tail = String::from_utf8_lossy(&buf);
    if let Some(text) = parse_data_line(tail.trim_end_matches(['\r', '\n']), parse) {
        acc.push_str(&text);
        let _ = tx.send(text).await;
    }

    Ok(acc)
}

/// Drain complete `\n`-terminated lines from `buf`, each decoded as a whole and
/// trimmed of trailing `\r`/`\n`. Leftover partial bytes (an incomplete line,
/// possibly mid-codepoint) stay in `buf` for the next chunk. Splitting on the
/// raw `\n` byte is safe because `0x0A` never occurs inside a multi-byte UTF-8
/// sequence, so each returned line is whole and decodes without corruption.
fn drain_lines(buf: &mut Vec<u8>) -> Vec<String> {
    let mut lines = Vec::new();
    while let Some(nl) = buf.iter().position(|&b| b == b'\n') {
        let line_bytes: Vec<u8> = buf.drain(..=nl).collect();
        lines.push(
            String::from_utf8_lossy(&line_bytes)
                .trim_end_matches(['\r', '\n'])
                .to_owned(),
        );
    }
    lines
}

/// Extract the text delta from a single SSE line, or `None` for non-`data:`
/// lines, the `[DONE]` sentinel, and payloads with no text.
fn parse_data_line(line: &str, parse: fn(&str) -> Option<String>) -> Option<String> {
    let payload = line.strip_prefix("data:")?.trim();
    if payload.is_empty() || payload == "[DONE]" {
        return None;
    }
    parse(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_text_delta_parses() {
        let text = anthropic_text_delta(
            r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hello"}}"#,
        );
        assert_eq!(text, Some("Hello".to_owned()));

        // message_start / ping events carry no text delta.
        assert_eq!(
            anthropic_text_delta(r#"{"type":"message_start","message":{"id":"m"}}"#),
            None
        );
        assert_eq!(anthropic_text_delta(r#"{"type":"ping"}"#), None);
    }

    #[test]
    fn openai_text_delta_parses() {
        assert_eq!(
            openai_text_delta(r#"{"choices":[{"delta":{"content":"Hi"}}]}"#),
            Some("Hi".to_owned())
        );
        // role-only delta (first chunk) has no content.
        assert_eq!(
            openai_text_delta(r#"{"choices":[{"delta":{"role":"assistant"}}]}"#),
            None
        );
        // The terminal sentinel is not valid JSON → None.
        assert_eq!(openai_text_delta("[DONE]"), None);
    }

    #[test]
    fn parse_data_line_skips_non_data_and_done() {
        let p = openai_text_delta;
        assert_eq!(parse_data_line("event: message", p), None);
        assert_eq!(parse_data_line("data: [DONE]", p), None);
        assert_eq!(parse_data_line("", p), None);
        assert_eq!(
            parse_data_line(r#"data: {"choices":[{"delta":{"content":"x"}}]}"#, p),
            Some("x".to_owned())
        );
    }

    /// `from_config(None)` is deterministic and needs no env manipulation.
    #[test]
    fn from_config_none_missing_key() {
        assert!(matches!(
            Provider::from_config(None),
            Err(ProviderError::MissingKey(_))
        ));
    }

    /// Regression for the SSE byte-framing fix: a multi-byte UTF-8 codepoint
    /// split across two network chunks must reassemble, not corrupt into
    /// `U+FFFD`. "日" (e6 97 a5) is fed one byte then two, then the newline.
    #[test]
    fn drain_lines_reassembles_multibyte_split_across_chunks() {
        let mut buf: Vec<u8> = Vec::new();

        buf.extend_from_slice(&[0xe6]); // first byte of 日
        assert!(drain_lines(&mut buf).is_empty(), "no line until a newline");

        buf.extend_from_slice(&[0x97, 0xa5]); // remaining bytes of 日
        buf.extend_from_slice(b"\r\n");
        let lines = drain_lines(&mut buf);
        assert_eq!(lines, vec!["日".to_owned()]);
        assert!(buf.is_empty(), "no leftover after the terminated line");
    }

    /// Two lines in one buffer drain in order; a trailing partial line stays.
    #[test]
    fn drain_lines_keeps_partial_tail() {
        let mut buf: Vec<u8> = b"data: a\ndata: b\ndata: c".to_vec();
        let lines = drain_lines(&mut buf);
        assert_eq!(lines, vec!["data: a".to_owned(), "data: b".to_owned()]);
        assert_eq!(buf, b"data: c"); // partial (no newline) retained
    }

    #[tokio::test]
    async fn mock_provider_streams() {
        let provider = Provider::Mock {
            deltas: vec!["a".into(), "b".into(), "c".into()],
        };
        let (tx, mut rx) = mpsc::channel::<String>(8);
        let full = provider
            .stream_chat("m", &[], &tx)
            .await
            .expect("mock stream");
        drop(tx);

        assert_eq!(full, "abc");
        let mut got = Vec::new();
        while let Some(d) = rx.recv().await {
            got.push(d);
        }
        assert_eq!(got, vec!["a", "b", "c"]);
    }
}
