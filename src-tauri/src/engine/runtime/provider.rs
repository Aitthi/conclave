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
//! API keys come from environment variables (`ANTHROPIC_API_KEY`,
//! `OPENAI_API_KEY`) **or** from the macOS Keychain when configured via
//! Settings (`engine::secrets`). Environment variables take precedence so
//! developers can override the Keychain value without touching the UI.
//!
//! # Testing
//!
//! The SSE line-payload parsers ([`anthropic_text_delta`] / [`openai_text_delta`])
//! are pure functions covered by unit tests. The real reqwest HTTP/SSE
//! round-trip is intentionally NOT unit-tested here (it would need a mock HTTP
//! server); that is left to a deferred integration test. Test code drives the
//! chat loop through the `#[cfg(test)] Provider::Mock` variant, which yields
//! canned deltas without any network I/O.

use super::usage::{checked_sum, counter_tracked, MeasuredUsage};
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
    /// Test-only provider that streams `text` as one delta and returns exactly
    /// this completion — for collector tests that need terminal metadata.
    #[cfg(test)]
    MockCompletion(ProviderCompletion),
}

// ── Terminal completion metadata ─────────────────────────────────────────────

/// What a completed provider call reports about itself, beside the text.
///
/// `completed` is `true` ONLY when the provider's own terminal marker arrived
/// (Anthropic `message_stop`, OpenAI `[DONE]`). Receiver loss, a transport
/// error or an EOF before that marker leave it `false`: the text gathered so
/// far may still be shown, but it is not completed activity and the usage
/// collector never records it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderCompletion {
    pub text: String,
    pub completed: bool,
    /// The provider's own response/message id — evidence for reconciliation,
    /// never the identity a record is keyed by.
    pub response_id: Option<String>,
    /// The model the PROVIDER said served the response (Anthropic
    /// `message_start.message.model`, OpenAI chunk `model`).
    pub served_model: Option<String>,
    pub usage: MeasuredUsage,
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
                let api_key = non_empty_env("ANTHROPIC_API_KEY")
                    .or_else(|| {
                        // Fall back to the Keychain when the env var is absent.
                        // `get_key` is sync; errors are silenced so a Keychain
                        // hiccup degrades gracefully to MissingKey below.
                        provider_id
                            .and_then(|id| crate::engine::secrets::get_key(id).ok().flatten())
                    })
                    .ok_or_else(|| {
                        ProviderError::MissingKey("ANTHROPIC_API_KEY is not set".into())
                    })?;
                let base_url = non_empty_env("CONCLAVE_ANTHROPIC_BASE_URL")
                    .unwrap_or_else(|| ANTHROPIC_DEFAULT_BASE.to_owned());
                Ok(Provider::Anthropic { api_key, base_url })
            }
            Some("openai") => {
                let api_key = non_empty_env("OPENAI_API_KEY")
                    .or_else(|| {
                        provider_id
                            .and_then(|id| crate::engine::secrets::get_key(id).ok().flatten())
                    })
                    .ok_or_else(|| ProviderError::MissingKey("OPENAI_API_KEY is not set".into()))?;
                let base_url = non_empty_env("CONCLAVE_OPENAI_BASE_URL")
                    .unwrap_or_else(|| OPENAI_DEFAULT_BASE.to_owned());
                Ok(Provider::OpenAi { api_key, base_url })
            }
            Some("local") => {
                // Local servers (Ollama et al.) are OpenAI-compatible and often
                // need no key — default to empty rather than erroring.
                let api_key = non_empty_env("OPENAI_API_KEY")
                    .or_else(|| {
                        provider_id
                            .and_then(|id| crate::engine::secrets::get_key(id).ok().flatten())
                    })
                    .unwrap_or_default();
                let base_url = non_empty_env("CONCLAVE_LOCAL_BASE_URL")
                    .unwrap_or_else(|| LOCAL_DEFAULT_BASE.to_owned());
                Ok(Provider::OpenAi { api_key, base_url })
            }
            _ => Err(ProviderError::MissingKey("no provider configured".into())),
        }
    }

    /// Override the base URL with a user-configured value (the provider's
    /// Settings `base_url` from the DB) when present and non-empty. `None` /
    /// empty leaves the env-or-default base URL chosen by [`from_config`]
    /// untouched, so a user's explicit Settings endpoint takes precedence.
    pub fn with_base_url(self, base_url: Option<&str>) -> Self {
        let url = match base_url.map(str::trim).filter(|s| !s.is_empty()) {
            Some(u) => u.to_owned(),
            None => return self,
        };
        match self {
            Provider::Anthropic { api_key, .. } => Provider::Anthropic {
                api_key,
                base_url: url,
            },
            Provider::OpenAi { api_key, .. } => Provider::OpenAi {
                api_key,
                base_url: url,
            },
            // Test-only variants (Mock / EchoLen) carry no base URL.
            #[cfg(test)]
            other => other,
        }
    }

    /// Stream a chat completion for `messages`, forwarding each assistant TEXT
    /// delta to `tx`, and return the full accumulated assistant text.
    ///
    /// Text-only view of [`Self::stream_chat_measured`], kept for callers that
    /// want no metadata; every in-process caller now takes the measured form.
    #[allow(dead_code)]
    pub async fn stream_chat(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tx: &mpsc::Sender<String>,
    ) -> Result<String, ProviderError> {
        self.stream_chat_measured(model, messages, tx)
            .await
            .map(|c| c.text)
    }

    /// Stream a chat completion and return it WITH its terminal metadata
    /// ([`ProviderCompletion`]): whether the provider's terminal marker was
    /// seen, its response id, the served model and the usage it reported.
    ///
    /// `tx` send errors (the receiver was dropped) stop the stream early — the
    /// accumulated text so far is still returned, marked NOT completed.
    pub async fn stream_chat_measured(
        &self,
        model: &str,
        messages: &[ChatMessage],
        tx: &mpsc::Sender<String>,
    ) -> Result<ProviderCompletion, ProviderError> {
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
                let mut completed = true;
                for d in deltas {
                    acc.push_str(d);
                    if tx.send(d.clone()).await.is_err() {
                        completed = false;
                        break; // receiver gone
                    }
                }
                Ok(ProviderCompletion {
                    text: acc,
                    completed,
                    ..ProviderCompletion::default()
                })
            }
            #[cfg(test)]
            Provider::EchoLen => {
                let s = messages.len().to_string();
                let _ = tx.send(s.clone()).await;
                Ok(ProviderCompletion {
                    text: s,
                    completed: true,
                    ..ProviderCompletion::default()
                })
            }
            #[cfg(test)]
            Provider::MockCompletion(completion) => {
                let _ = tx.send(completion.text.clone()).await;
                Ok(completion.clone())
            }
        }
    }

    /// Perform a NON-streaming completion of a single user `prompt`, returning
    /// the full accumulated assistant text. Text-only view of
    /// [`Self::complete_chat_measured`]; see [`Self::stream_chat`].
    #[allow(dead_code)]
    pub async fn complete_chat(&self, model: &str, prompt: &str) -> Result<String, ProviderError> {
        self.complete_chat_measured(model, prompt)
            .await
            .map(|c| c.text)
    }

    /// Non-streaming completion WITH terminal metadata.
    ///
    /// Thin wrapper over [`Self::stream_chat_measured`]: it builds one `User`
    /// message, creates an internal `mpsc` channel, and drains the receiver on a
    /// spawned task so a long answer can't deadlock by filling the channel while
    /// the stream is still producing. Reuses the existing provider plumbing — no
    /// new HTTP code.
    pub async fn complete_chat_measured(
        &self,
        model: &str,
        prompt: &str,
    ) -> Result<ProviderCompletion, ProviderError> {
        let msg = ChatMessage {
            role: ChatRole::User,
            content: prompt.to_owned(),
        };
        let (tx, mut rx) = mpsc::channel::<String>(1024);
        // Drain deltas concurrently so the sender never blocks on a full channel.
        let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let result = self.stream_chat_measured(model, &[msg], &tx).await;
        drop(tx); // close the channel so the drain task terminates
                  // The drain body can't panic, so a JoinError is unreachable here; we still
                  // await it to guarantee the task is gone before returning (no leak).
        let _ = drain.await;
        result
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

// ── SSE accumulators (terminal metadata) ─────────────────────────────────────

/// Per-stream state that turns SSE payloads into text deltas AND remembers the
/// terminal metadata. One implementation per wire protocol; both are pure and
/// unit-tested by feeding payloads directly.
trait SseAccumulator {
    /// Ingest one `data:` JSON payload, returning the text delta it carries.
    fn ingest(&mut self, payload: &str) -> Option<String>;
    /// The `[DONE]` sentinel arrived.
    fn done(&mut self);
    /// Has the provider's terminal marker been accepted? Once true the body
    /// is not read further: a tail transport failure or a server that keeps
    /// the connection open must not lose or delay a completed response
    /// (review a12f77f2 C8).
    fn is_terminal(&self) -> bool;
    /// The completion as seen so far, `text` being the accumulated deltas.
    fn finish(&self, text: String) -> ProviderCompletion;
}

/// Anthropic Messages stream: `message_start` carries the id, the served model
/// and the INPUT usage (uncached + cache-create + cache-read, all three needed
/// for a cache-inclusive input); `message_delta` carries the cumulative OUTPUT
/// count; `message_stop` is the terminal marker. Nothing else is trusted — a
/// stream that ends without `message_stop` is not completed.
#[derive(Debug, Default)]
struct AnthropicAcc {
    response_id: Option<String>,
    served_model: Option<String>,
    uncached_input: Option<i64>,
    cache_write: Option<i64>,
    cache_read: Option<i64>,
    output: Option<i64>,
    invalid: u32,
    stopped: bool,
}

impl SseAccumulator for AnthropicAcc {
    fn ingest(&mut self, payload: &str) -> Option<String> {
        let v: serde_json::Value = serde_json::from_str(payload).ok()?;
        match v.get("type")?.as_str()? {
            "message_start" => {
                let message = &v["message"];
                self.response_id = message["id"].as_str().map(str::to_owned);
                self.served_model = message["model"].as_str().map(str::to_owned);
                let usage = &message["usage"];
                self.uncached_input = counter_tracked(&usage["input_tokens"], &mut self.invalid);
                self.cache_write =
                    counter_tracked(&usage["cache_creation_input_tokens"], &mut self.invalid);
                self.cache_read =
                    counter_tracked(&usage["cache_read_input_tokens"], &mut self.invalid);
                None
            }
            "message_delta" => {
                // Cumulative for the whole message; the last one wins.
                if let Some(output) =
                    counter_tracked(&v["usage"]["output_tokens"], &mut self.invalid)
                {
                    self.output = Some(output);
                }
                None
            }
            "message_stop" => {
                self.stopped = true;
                None
            }
            "content_block_delta" => anthropic_text_delta(payload),
            _ => None,
        }
    }

    fn done(&mut self) {
        // Anthropic has no `[DONE]` sentinel; only `message_stop` completes.
    }

    fn is_terminal(&self) -> bool {
        self.stopped
    }

    fn finish(&self, text: String) -> ProviderCompletion {
        let mut invalid = self.invalid;
        let input_tokens = checked_sum(
            &[self.uncached_input, self.cache_write, self.cache_read],
            &mut invalid,
        );
        ProviderCompletion {
            text,
            completed: self.stopped,
            response_id: self.response_id.clone(),
            served_model: self.served_model.clone(),
            usage: MeasuredUsage {
                input_tokens,
                output_tokens: self.output,
                cache_read_input_tokens: self.cache_read,
                cache_write_input_tokens: self.cache_write,
                reasoning_output_tokens: None,
                invalid_counters: invalid,
            },
        }
    }
}

/// OpenAI-compatible `/chat/completions` stream: every chunk repeats `id` and
/// `model`; text rides `choices[0].delta.content`; with
/// `stream_options.include_usage` the LAST chunk before `[DONE]` carries
/// `usage` (`prompt_tokens` already includes cached input,
/// `completion_tokens` already includes reasoning; the `*_details` are
/// subsets). `[DONE]` is the terminal marker.
#[derive(Debug, Default)]
struct OpenAiAcc {
    response_id: Option<String>,
    served_model: Option<String>,
    usage: MeasuredUsage,
    done: bool,
}

impl SseAccumulator for OpenAiAcc {
    fn ingest(&mut self, payload: &str) -> Option<String> {
        let v: serde_json::Value = serde_json::from_str(payload).ok()?;
        if self.response_id.is_none() {
            self.response_id = v["id"].as_str().map(str::to_owned);
        }
        if self.served_model.is_none() {
            self.served_model = v["model"]
                .as_str()
                .filter(|m| !m.is_empty())
                .map(str::to_owned);
        }
        if let Some(usage) = v.get("usage").filter(|u| u.is_object()) {
            let mut invalid = 0;
            self.usage = MeasuredUsage {
                input_tokens: counter_tracked(&usage["prompt_tokens"], &mut invalid),
                output_tokens: counter_tracked(&usage["completion_tokens"], &mut invalid),
                cache_read_input_tokens: counter_tracked(
                    &usage["prompt_tokens_details"]["cached_tokens"],
                    &mut invalid,
                ),
                cache_write_input_tokens: None,
                reasoning_output_tokens: counter_tracked(
                    &usage["completion_tokens_details"]["reasoning_tokens"],
                    &mut invalid,
                ),
                invalid_counters: invalid,
            };
        }
        openai_text_delta(payload)
    }

    fn done(&mut self) {
        self.done = true;
    }

    fn is_terminal(&self) -> bool {
        self.done
    }

    fn finish(&self, text: String) -> ProviderCompletion {
        ProviderCompletion {
            text,
            completed: self.done,
            response_id: self.response_id.clone(),
            served_model: self.served_model.clone(),
            usage: self.usage.clone(),
        }
    }
}

/// Whether to ask an OpenAI-compatible server for `stream_options.include_usage`.
///
/// Only the official endpoint is known to accept the field; an unknown
/// compatible server (a proxy, Ollama, a lab gateway) may reject the whole
/// request with 400, and a usage flag must never cost the user their answer.
/// Elsewhere the usage stays unknown unless the server volunteers it.
fn openai_requests_usage(base_url: &str) -> bool {
    base_url
        .trim_end_matches('/')
        .starts_with(OPENAI_DEFAULT_BASE.trim_end_matches('/'))
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
) -> Result<ProviderCompletion, ProviderError> {
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

    consume_sse(resp, tx, AnthropicAcc::default()).await
}

/// Drive an OpenAI-compatible `/chat/completions` streaming request.
async fn stream_openai(
    api_key: &str,
    base_url: &str,
    model: &str,
    messages: &[ChatMessage],
    tx: &mpsc::Sender<String>,
) -> Result<ProviderCompletion, ProviderError> {
    let body = openai_request_body(base_url, model, messages);

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

    consume_sse(resp, tx, OpenAiAcc::default()).await
}

/// The `/chat/completions` request body; `stream_options.include_usage` only
/// where [`openai_requests_usage`] says the server accepts it.
fn openai_request_body(base_url: &str, model: &str, messages: &[ChatMessage]) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "stream": true,
        "messages": messages
            .iter()
            .map(|m| serde_json::json!({ "role": m.role.as_wire(), "content": m.content }))
            .collect::<Vec<_>>(),
    });
    if openai_requests_usage(base_url) {
        body["stream_options"] = serde_json::json!({ "include_usage": true });
    }
    body
}

/// Consume an SSE response: accumulate RAW BYTES, split on the `\n` byte, and
/// for each `data:` line feed the JSON payload to the accumulator, forwarding
/// any text delta to `tx`. Keeps a leftover partial-line buffer across network
/// chunks. Returns the completion with its terminal metadata.
///
/// Decoding happens per COMPLETE LINE, not per network chunk: `0x0A` never
/// appears inside a multi-byte UTF-8 sequence, so line boundaries are safe to
/// find in raw bytes, and each whole line decodes as valid UTF-8. Decoding a raw
/// chunk directly would split a multi-byte codepoint at a chunk boundary into
/// `U+FFFD` replacement chars — corrupting non-ASCII (CJK/Thai/emoji) deltas.
///
/// A dropped receiver stops the stream early; the text so far comes back but
/// the completion is marked NOT completed — nobody saw the terminal marker.
async fn consume_sse<A: SseAccumulator>(
    resp: reqwest::Response,
    tx: &mpsc::Sender<String>,
    mut acc: A,
) -> Result<ProviderCompletion, ProviderError> {
    use futures_util::StreamExt;

    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    let mut text = String::new();

    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| ProviderError::Stream(e.to_string()))?;
        buf.extend_from_slice(&bytes);

        for line in drain_lines(&mut buf) {
            if let Some(delta) = ingest_line(&mut acc, &line) {
                text.push_str(&delta);
                if tx.send(delta).await.is_err() {
                    let mut completion = acc.finish(text);
                    completion.completed = false; // receiver gone → not completed
                    return Ok(completion);
                }
            }
            if acc.is_terminal() {
                // The response is complete: stop reading. Whatever the
                // transport does after this point cannot take it back or
                // hold it hostage.
                return Ok(acc.finish(text));
            }
        }
    }

    // Flush any trailing line without a final newline.
    let tail = String::from_utf8_lossy(&buf);
    if let Some(delta) = ingest_line(&mut acc, tail.trim_end_matches(['\r', '\n'])) {
        text.push_str(&delta);
        let _ = tx.send(delta).await;
    }

    Ok(acc.finish(text))
}

/// Feed one SSE line to the accumulator: non-`data:` lines are ignored, the
/// `[DONE]` sentinel is reported to it, a payload is ingested. Returns the
/// text delta, if any.
fn ingest_line<A: SseAccumulator>(acc: &mut A, line: &str) -> Option<String> {
    match classify_line(line) {
        SseLine::Skip => None,
        SseLine::Done => {
            acc.done();
            None
        }
        SseLine::Payload(payload) => acc.ingest(payload),
    }
}

/// One SSE line, classified.
#[derive(Debug, PartialEq, Eq)]
enum SseLine<'a> {
    /// Not a `data:` line, or an empty payload.
    Skip,
    /// The OpenAI-style `[DONE]` sentinel.
    Done,
    /// A `data:` JSON payload.
    Payload(&'a str),
}

fn classify_line(line: &str) -> SseLine<'_> {
    let Some(payload) = line.strip_prefix("data:") else {
        return SseLine::Skip;
    };
    let payload = payload.trim();
    if payload.is_empty() {
        SseLine::Skip
    } else if payload == "[DONE]" {
        SseLine::Done
    } else {
        SseLine::Payload(payload)
    }
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
    fn classify_line_skips_non_data_and_reports_done() {
        assert_eq!(classify_line("event: message"), SseLine::Skip);
        assert_eq!(classify_line("data: [DONE]"), SseLine::Done);
        assert_eq!(classify_line(""), SseLine::Skip);
        assert_eq!(classify_line("data:   "), SseLine::Skip);
        assert_eq!(
            classify_line(r#"data: {"choices":[{"delta":{"content":"x"}}]}"#),
            SseLine::Payload(r#"{"choices":[{"delta":{"content":"x"}}]}"#)
        );
    }

    // ── Terminal metadata ────────────────────────────────────────────────

    /// Feed SSE lines through the same path `consume_sse` uses.
    fn run_lines<A: SseAccumulator>(mut acc: A, lines: &[&str]) -> ProviderCompletion {
        let mut text = String::new();
        for line in lines {
            if let Some(delta) = ingest_line(&mut acc, line) {
                text.push_str(&delta);
            }
            if acc.is_terminal() {
                break; // exactly what consume_sse does
            }
        }
        acc.finish(text)
    }

    /// Once the terminal marker is accepted the rest of the body is ignored:
    /// a garbage or error tail after `message_stop` / `[DONE]` neither
    /// retracts the completion nor changes its numbers (review C8).
    #[test]
    fn a_tail_after_the_terminal_marker_cannot_touch_the_completion() {
        let mut lines: Vec<&str> = ANTHROPIC_STREAM.to_vec();
        lines.push(r#"data: {"type":"message_delta","usage":{"output_tokens":-999}}"#);
        lines.push("data: this is not json");
        let c = run_lines(AnthropicAcc::default(), &lines);
        assert!(c.completed);
        assert_eq!(c.usage.output_tokens, Some(15));
        assert_eq!(c.usage.invalid_counters, 0);

        let mut lines: Vec<&str> = OPENAI_STREAM.to_vec();
        lines.push(r#"data: {"id":"x","choices":[{"delta":{"content":"LATE"}}]}"#);
        let c = run_lines(OpenAiAcc::default(), &lines);
        assert!(c.completed);
        assert_eq!(c.text, "Hello", "nothing after [DONE] is text");
    }

    /// A provider that reports nonsense counters leaves evidence, a provider
    /// that omits them does not (review C7).
    #[test]
    fn invalid_provider_counters_are_evidence_and_absent_ones_are_not() {
        let lines = [
            r#"data: {"type":"message_start","message":{"id":"m","model":"claude-opus-5","usage":{"input_tokens":-1,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3.5}}"#,
            r#"data: {"type":"message_stop"}"#,
        ];
        let c = run_lines(AnthropicAcc::default(), &lines);
        assert!(c.completed);
        assert_eq!(c.usage.input_tokens, None);
        assert_eq!(c.usage.output_tokens, None);
        assert_eq!(c.usage.invalid_counters, 2);
        assert_eq!(c.usage.diagnostic_code(), Some("counter_out_of_range"));

        let absent = [
            r#"data: {"type":"message_start","message":{"id":"m","model":"claude-opus-5"}}"#,
            r#"data: {"type":"message_stop"}"#,
        ];
        let c = run_lines(AnthropicAcc::default(), &absent);
        assert_eq!(
            c.usage.invalid_counters, 0,
            "never reported is plain unknown"
        );
        assert_eq!(c.usage.diagnostic_code(), None);
    }

    const ANTHROPIC_STREAM: &[&str] = &[
        "event: message_start",
        r#"data: {"type":"message_start","message":{"id":"msg_01X","type":"message","role":"assistant","model":"claude-opus-5","content":[],"stop_reason":null,"usage":{"input_tokens":25,"cache_creation_input_tokens":1000,"cache_read_input_tokens":400,"output_tokens":1}}}"#,
        r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
        r#"data: {"type":"ping"}"#,
        r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}"#,
        r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"lo"}}"#,
        r#"data: {"type":"content_block_stop","index":0}"#,
        r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":15}}"#,
        r#"data: {"type":"message_stop"}"#,
    ];

    #[test]
    fn anthropic_stream_yields_cache_inclusive_input_and_terminal_output() {
        let c = run_lines(AnthropicAcc::default(), ANTHROPIC_STREAM);
        assert_eq!(c.text, "Hello");
        assert!(c.completed, "message_stop seen");
        assert_eq!(c.response_id.as_deref(), Some("msg_01X"));
        assert_eq!(c.served_model.as_deref(), Some("claude-opus-5"));
        assert_eq!(c.usage.input_tokens, Some(25 + 1000 + 400));
        assert_eq!(
            c.usage.output_tokens,
            Some(15),
            "message_delta, not message_start's 1"
        );
        assert_eq!(c.usage.cache_write_input_tokens, Some(1000));
        assert_eq!(c.usage.cache_read_input_tokens, Some(400));
        assert_eq!(c.usage.reasoning_output_tokens, None);
    }

    /// EOF before `message_stop` is not completed activity, even with text.
    #[test]
    fn anthropic_stream_without_message_stop_is_not_completed() {
        let cut = &ANTHROPIC_STREAM[..ANTHROPIC_STREAM.len() - 1];
        let c = run_lines(AnthropicAcc::default(), cut);
        assert_eq!(c.text, "Hello");
        assert!(!c.completed);
        assert_eq!(
            c.usage.output_tokens,
            Some(15),
            "what was seen is still reported"
        );
    }

    /// A message_start missing a cache component makes the INPUT unknown; the
    /// output is independent. `[DONE]` means nothing to Anthropic.
    #[test]
    fn anthropic_stream_with_partial_input_components_is_unknown_input() {
        let lines = [
            r#"data: {"type":"message_start","message":{"id":"m","model":"claude-opus-5","usage":{"input_tokens":25}}}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}"#,
            "data: [DONE]",
        ];
        let c = run_lines(AnthropicAcc::default(), &lines);
        assert_eq!(c.usage.input_tokens, None);
        assert_eq!(c.usage.output_tokens, Some(3));
        assert!(!c.completed, "[DONE] is not Anthropic's terminal marker");
    }

    const OPENAI_STREAM: &[&str] = &[
        r#"data: {"id":"chatcmpl-9","object":"chat.completion.chunk","model":"gpt-5.5","choices":[{"index":0,"delta":{"role":"assistant","content":""},"finish_reason":null}]}"#,
        r#"data: {"id":"chatcmpl-9","object":"chat.completion.chunk","model":"gpt-5.5","choices":[{"index":0,"delta":{"content":"Hel"},"finish_reason":null}]}"#,
        r#"data: {"id":"chatcmpl-9","object":"chat.completion.chunk","model":"gpt-5.5","choices":[{"index":0,"delta":{"content":"lo"},"finish_reason":null}]}"#,
        r#"data: {"id":"chatcmpl-9","object":"chat.completion.chunk","model":"gpt-5.5","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#,
        r#"data: {"id":"chatcmpl-9","object":"chat.completion.chunk","model":"gpt-5.5","choices":[],"usage":{"prompt_tokens":120,"completion_tokens":40,"total_tokens":160,"prompt_tokens_details":{"cached_tokens":100},"completion_tokens_details":{"reasoning_tokens":8}}}"#,
        "data: [DONE]",
    ];

    #[test]
    fn openai_stream_yields_usage_from_the_final_chunk_and_completes_on_done() {
        let c = run_lines(OpenAiAcc::default(), OPENAI_STREAM);
        assert_eq!(c.text, "Hello");
        assert!(c.completed);
        assert_eq!(c.response_id.as_deref(), Some("chatcmpl-9"));
        assert_eq!(c.served_model.as_deref(), Some("gpt-5.5"));
        assert_eq!(
            c.usage.input_tokens,
            Some(120),
            "prompt_tokens already includes cached"
        );
        assert_eq!(
            c.usage.output_tokens,
            Some(40),
            "completion_tokens already includes reasoning"
        );
        assert_eq!(c.usage.cache_read_input_tokens, Some(100));
        assert_eq!(c.usage.reasoning_output_tokens, Some(8));
        assert_eq!(c.usage.cache_write_input_tokens, None);
    }

    /// A compatible server that never sends `usage` still yields a completed
    /// answer with unknown usage; a stream cut before `[DONE]` is not completed.
    #[test]
    fn openai_stream_without_usage_or_done_is_honest() {
        let no_usage: Vec<&str> = OPENAI_STREAM
            .iter()
            .copied()
            .filter(|l| !l.contains("\"usage\""))
            .collect();
        let c = run_lines(OpenAiAcc::default(), &no_usage);
        assert!(c.completed);
        assert_eq!(c.usage, MeasuredUsage::default());
        assert_eq!(c.text, "Hello");

        let cut = &OPENAI_STREAM[..OPENAI_STREAM.len() - 1];
        let c = run_lines(OpenAiAcc::default(), cut);
        assert!(!c.completed);
        assert_eq!(
            c.usage.input_tokens,
            Some(120),
            "seen usage is still reported"
        );
    }

    /// `include_usage` is requested only where it is known to be accepted.
    #[test]
    fn include_usage_is_requested_only_from_the_official_openai_endpoint() {
        let msgs = [ChatMessage {
            role: ChatRole::User,
            content: "hi".into(),
        }];
        let official = openai_request_body("https://api.openai.com/v1", "gpt-5.5", &msgs);
        assert_eq!(
            official["stream_options"]["include_usage"],
            serde_json::json!(true)
        );
        for other in [
            "http://localhost:11434/v1",
            "https://proxy.example.com/v1",
            "https://api.openai.com.evil.example/v1",
        ] {
            let body = openai_request_body(other, "gpt-5.5", &msgs);
            assert!(
                body.get("stream_options").is_none(),
                "{other} must not get the flag"
            );
        }
    }

    #[tokio::test]
    async fn mock_completion_returns_its_metadata_and_streams_its_text() {
        let completion = ProviderCompletion {
            text: "done".into(),
            completed: true,
            response_id: Some("r-1".into()),
            served_model: Some("m-served".into()),
            usage: MeasuredUsage {
                input_tokens: Some(3),
                output_tokens: Some(4),
                ..MeasuredUsage::default()
            },
        };
        let provider = Provider::MockCompletion(completion.clone());
        let (tx, mut rx) = mpsc::channel::<String>(8);
        let got = provider.stream_chat_measured("m", &[], &tx).await.unwrap();
        assert_eq!(got, completion);
        assert_eq!(rx.recv().await.as_deref(), Some("done"));
        assert_eq!(
            provider.complete_chat("m", "p").await.unwrap(),
            "done",
            "the text-only view still works"
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

    /// `with_base_url` overrides only when given a non-empty value, and only
    /// the base URL — the api key is preserved.
    #[test]
    fn with_base_url_overrides_when_present() {
        let p = Provider::OpenAi {
            api_key: "k".into(),
            base_url: "http://default".into(),
        }
        .with_base_url(Some("http://custom:9000/v1"));
        match p {
            Provider::OpenAi { api_key, base_url } => {
                assert_eq!(api_key, "k");
                assert_eq!(base_url, "http://custom:9000/v1");
            }
            _ => panic!("variant changed"),
        }
    }

    /// `with_base_url(None)` and an all-whitespace value leave the base URL intact.
    #[test]
    fn with_base_url_noop_on_empty() {
        for arg in [None, Some(""), Some("   ")] {
            let p = Provider::Anthropic {
                api_key: "k".into(),
                base_url: "http://keep".into(),
            }
            .with_base_url(arg);
            match p {
                Provider::Anthropic { base_url, .. } => assert_eq!(base_url, "http://keep"),
                _ => panic!("variant changed"),
            }
        }
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
