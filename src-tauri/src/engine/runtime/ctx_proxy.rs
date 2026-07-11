//! App-global loopback proxy for Anthropic API traffic.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use axum::body::{to_bytes, Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, Request, Response, StatusCode};
use axum::routing::any;
use axum::Router;
use futures_util::{stream, StreamExt};
use serde_json::Value;

use crate::engine::state::AppState;

const DEFAULT_PROXY_PORT: u16 = 18_787;
const DEFAULT_UPSTREAM: &str = "https://api.anthropic.com";
pub const MODE_OFF: u8 = 0;
pub const MODE_LOG: u8 = 1;
pub const MODE_REWRITE: u8 = 2;

struct RewriteOutcome {
    body: Vec<u8>,
    elisions: usize,
    bytes_saved: usize,
    model: String,
    decision: &'static str,
    conversation: Option<Value>,
    request_bytes_in: usize,
    mode: &'static str,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct UsageTotals {
    input_tokens: Option<u64>,
    cache_read: Option<u64>,
    cache_creation: Option<u64>,
    output_tokens: Option<u64>,
}

pub struct ProxyRuntime {
    pub port: u16,
    pub mode: AtomicU8,
    /// Elision high-water ratio, f32 stored as bits. Runtime-settable via
    /// `proxy.threshold`; resets to `ctxopt::DEFAULT_HIGH_WATER` on restart.
    pub threshold: AtomicU32,
    pub upstream: RwLock<String>,
    pub ledger: Mutex<ctxopt::ledger::Ledger>,
    pub active: AtomicBool,
    client: reqwest::Client,
}

impl ProxyRuntime {
    pub fn new() -> Self {
        let port = std::env::var("CONCLAVE_PROXY_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(DEFAULT_PROXY_PORT);
        Self::with_port(port)
    }

    fn with_port(port: u16) -> Self {
        Self {
            port,
            mode: AtomicU8::new(MODE_LOG),
            threshold: AtomicU32::new(ctxopt::DEFAULT_HIGH_WATER.to_bits()),
            upstream: RwLock::new(DEFAULT_UPSTREAM.to_owned()),
            ledger: Mutex::new(ctxopt::ledger::Ledger::new(ctxopt::LEDGER_CAP)),
            active: AtomicBool::new(false),
            client: reqwest::Client::new(),
        }
    }

    pub fn active_port(&self) -> Option<u16> {
        self.active.load(Ordering::Acquire).then_some(self.port)
    }
}

impl Default for ProxyRuntime {
    fn default() -> Self {
        Self::new()
    }
}

/// Serve the proxy for the lifetime of the engine. Bind failures are retried so
/// a transient port conflict cannot permanently disable an opted-in agent.
pub async fn serve(state: Arc<AppState>) {
    let address = (std::net::Ipv4Addr::LOCALHOST, state.ctx_proxy.port);
    let listener = loop {
        match tokio::net::TcpListener::bind(address).await {
            Ok(listener) => break listener,
            Err(error) => {
                eprintln!("[ctx-proxy] failed to bind loopback listener: {error}");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    };

    state.ctx_proxy.active.store(true, Ordering::Release);
    let app = Router::new()
        .fallback(any(forward))
        .with_state(state.clone());

    if let Err(error) = axum::serve(listener, app).await {
        state.ctx_proxy.active.store(false, Ordering::Release);
        eprintln!("[ctx-proxy] listener stopped: {error}");
    }
}

async fn forward(State(state): State<Arc<AppState>>, request: Request<Body>) -> Response<Body> {
    match forward_inner(state, request).await {
        Ok(response) => response,
        Err(error) => Response::builder()
            .status(StatusCode::BAD_GATEWAY)
            .header("content-type", "text/plain; charset=utf-8")
            .body(Body::from(format!("upstream request failed: {error}")))
            .expect("static proxy error response is valid"),
    }
}

async fn forward_inner(
    state: Arc<AppState>,
    request: Request<Body>,
) -> Result<Response<Body>, String> {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, usize::MAX)
        .await
        .map_err(|error| error.to_string())?;
    let should_optimize = parts.method == axum::http::Method::POST
        && parts.uri.path() == "/v1/messages"
        && state.ctx_proxy.mode.load(Ordering::Acquire) != MODE_OFF;
    let rewrite = should_optimize.then(|| rewrite_body(&state.ctx_proxy, &body));
    let upstream_body = rewrite
        .as_ref()
        .map_or_else(|| body.to_vec(), |outcome| outcome.body.clone());
    let upstream = state
        .ctx_proxy
        .upstream
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .trim_end_matches('/')
        .to_owned();
    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/");

    let upstream_request = state
        .ctx_proxy
        .client
        .request(parts.method, format!("{upstream}{path_and_query}"))
        .body(upstream_body);
    let upstream_request = with_upstream_headers(upstream_request, &parts.headers);

    let upstream_response = upstream_request
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let status = upstream_response.status();
    let response_headers = upstream_response.headers().clone();
    let response_stream = if let Some(outcome) = rewrite {
        tee_response_stream(upstream_response.bytes_stream(), state, outcome)
    } else {
        Body::from_stream(
            upstream_response
                .bytes_stream()
                .map(|chunk| chunk.map_err(std::io::Error::other)),
        )
    };

    let mut response = Response::builder().status(status);
    for (name, value) in filtered_headers(&response_headers) {
        response = response.header(name, value);
    }
    response
        .body(response_stream)
        .map_err(|error| error.to_string())
}

fn rewrite_body(rt: &ProxyRuntime, body: &[u8]) -> RewriteOutcome {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        rewrite_body_inner(rt, body)
    })) {
        Ok(outcome) => outcome,
        Err(_) => {
            eprintln!("[ctx-proxy] rewrite pipeline panicked; forwarding original request");
            RewriteOutcome::original(body, "validate-reject", String::new(), None, rt)
        }
    }
}

fn rewrite_body_inner(rt: &ProxyRuntime, body: &[u8]) -> RewriteOutcome {
    let original: Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("[ctx-proxy] request JSON parse failed: {error}");
            return RewriteOutcome::original(body, "parse-error", String::new(), None, rt);
        }
    };
    let model = original
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let messages = original.get("messages").unwrap_or(&Value::Null);
    let conversation = Some(messages.clone());

    let mut ledger = rt.ledger.lock().unwrap_or_else(|error| error.into_inner());
    let conv_idx = ledger.observe(messages);
    let conv = ledger.conv_mut(conv_idx);
    let est = conv
        .last_input_tokens
        .map(|tokens| tokens as usize)
        .unwrap_or_else(|| ctxopt::estimate::est_tokens(body.len()));
    let window = ctxopt::estimate::context_window_for_model(&model);
    let ratio = f32::from_bits(rt.threshold.load(Ordering::Acquire));
    let decision = ctxopt::policy::decide(est, window, ratio, conv);

    let decision_name = match decision {
        ctxopt::policy::Decision::Passthrough => {
            return RewriteOutcome::original(body, "passthrough", model, conversation, rt);
        }
        ctxopt::policy::Decision::ApplyFrozen => "apply-frozen",
        ctxopt::policy::Decision::Reevaluate => {
            let analyzed = ctxopt::analyze::analyze(messages);
            for elision in analyzed {
                if !conv
                    .frozen
                    .iter()
                    .any(|frozen| frozen.tool_use_id == elision.tool_use_id)
                {
                    conv.frozen.push(elision);
                }
            }
            conv.last_eval_est = est;
            "reevaluate"
        }
    };

    let (_, results) = ctxopt::request::index_tools(messages);
    let applied: Vec<_> = conv
        .frozen
        .iter()
        .filter(|elision| {
            results
                .iter()
                .any(|result| result.tool_use_id == elision.tool_use_id)
        })
        .cloned()
        .collect();
    drop(ledger);

    if applied.is_empty() {
        return RewriteOutcome::original(body, decision_name, model, conversation, rt);
    }

    let mut rewritten = original.clone();
    let bytes_saved = ctxopt::apply::apply(&mut rewritten["messages"], &applied);
    if let Err(error) = ctxopt::validate::validate(
        original.get("messages").unwrap_or(&Value::Null),
        rewritten.get("messages").unwrap_or(&Value::Null),
        &applied,
    ) {
        eprintln!("[ctx-proxy] rewrite validation rejected: {error}");
        return RewriteOutcome::original(body, "validate-reject", model, conversation, rt);
    }

    let rewritten_body = serde_json::to_vec(&rewritten).unwrap_or_else(|_| body.to_vec());
    let mode = rt.mode.load(Ordering::Acquire);
    RewriteOutcome {
        body: if mode == MODE_REWRITE {
            rewritten_body
        } else {
            body.to_vec()
        },
        elisions: applied.len(),
        bytes_saved,
        model,
        decision: decision_name,
        conversation,
        request_bytes_in: body.len(),
        mode: mode_name(mode),
    }
}

impl RewriteOutcome {
    fn original(
        body: &[u8],
        decision: &'static str,
        model: String,
        conversation: Option<Value>,
        rt: &ProxyRuntime,
    ) -> Self {
        let mode = rt.mode.load(Ordering::Acquire);
        Self {
            body: body.to_vec(),
            elisions: 0,
            bytes_saved: 0,
            model,
            decision,
            conversation,
            request_bytes_in: body.len(),
            mode: mode_name(mode),
        }
    }
}

fn mode_name(mode: u8) -> &'static str {
    match mode {
        MODE_REWRITE => "rewrite",
        MODE_LOG => "log",
        _ => "off",
    }
}

fn tee_response_stream(
    mut upstream: impl futures_util::Stream<Item = Result<Bytes, reqwest::Error>>
        + Send
        + Unpin
        + 'static,
    state: Arc<AppState>,
    outcome: RewriteOutcome,
) -> Body {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(8);
    tokio::spawn(async move {
        let mut parser = SseUsageParser::default();
        while let Some(item) = upstream.next().await {
            match item {
                Ok(chunk) => {
                    if tx.send(Ok(chunk.clone())).await.is_err() {
                        return;
                    }
                    parser.push(&chunk);
                }
                Err(error) => {
                    let _ = tx.send(Err(std::io::Error::other(error))).await;
                    return;
                }
            }
        }
        parser.finish();
        drop(tx);
        on_request_complete(&state, &outcome, parser.usage).await;
    });
    Body::from_stream(stream::unfold(rx, |mut receiver| async move {
        receiver.recv().await.map(|item| (item, receiver))
    }))
}

#[derive(Default)]
struct SseUsageParser {
    pending: Vec<u8>,
    usage: UsageTotals,
}

impl SseUsageParser {
    fn push(&mut self, chunk: &[u8]) {
        self.pending.extend_from_slice(chunk);
        while let Some(newline) = self.pending.iter().position(|byte| *byte == b'\n') {
            let mut line: Vec<u8> = self.pending.drain(..=newline).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.parse_line(&line);
        }
    }

    fn finish(&mut self) {
        if !self.pending.is_empty() {
            let line = std::mem::take(&mut self.pending);
            self.parse_line(&line);
        }
    }

    fn parse_line(&mut self, line: &[u8]) {
        let Some(data) = line.strip_prefix(b"data: ") else {
            return;
        };
        let Ok(value) = serde_json::from_slice::<Value>(data) else {
            return;
        };
        if let Some(usage) = value.pointer("/message/usage") {
            self.usage.input_tokens = token_value(usage, "input_tokens");
            self.usage.cache_read = token_value(usage, "cache_read_input_tokens")
                .or_else(|| token_value(usage, "cache_read"));
            self.usage.cache_creation = token_value(usage, "cache_creation_input_tokens")
                .or_else(|| token_value(usage, "cache_creation"));
        }
        if let Some(usage) = value.get("usage") {
            if let Some(output) = token_value(usage, "output_tokens") {
                self.usage.output_tokens = Some(output);
            }
        }
    }
}

fn token_value(usage: &Value, key: &str) -> Option<u64> {
    usage.get(key).and_then(Value::as_u64)
}

async fn on_request_complete(state: &AppState, outcome: &RewriteOutcome, usage: UsageTotals) {
    if let (Some(conversation), Some(input)) = (outcome.conversation.as_ref(), usage.input_tokens) {
        let total = input
            .saturating_add(usage.cache_read.unwrap_or(0))
            .saturating_add(usage.cache_creation.unwrap_or(0));
        let mut ledger = state
            .ctx_proxy
            .ledger
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let conv_idx = ledger.observe(conversation);
        ledger.conv_mut(conv_idx).last_input_tokens = Some(total);
    }

    let metric = crate::engine::repo::proxy_metric::MetricInsert {
        created_at: chrono::Utc::now().to_rfc3339(),
        model: outcome.model.clone(),
        mode: outcome.mode.to_owned(),
        decision: outcome.decision.to_owned(),
        request_bytes_in: saturating_i64(outcome.request_bytes_in as u64),
        request_bytes_out: saturating_i64(outcome.body.len() as u64),
        elisions: saturating_i64(outcome.elisions as u64),
        bytes_saved: saturating_i64(outcome.bytes_saved as u64),
        input_tokens: usage.input_tokens.map(saturating_i64),
        cache_read_tokens: usage.cache_read.map(saturating_i64),
        cache_creation_tokens: usage.cache_creation.map(saturating_i64),
        output_tokens: usage.output_tokens.map(saturating_i64),
    };
    if let Err(error) = crate::engine::repo::proxy_metric::insert(&state.db, metric).await {
        eprintln!("[ctx-proxy] failed to record request metric: {error}");
    }
}

fn saturating_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn filtered_headers(
    headers: &HeaderMap,
) -> impl Iterator<Item = (&HeaderName, &axum::http::HeaderValue)> {
    headers.iter().filter(move |(name, _)| {
        !is_hop_by_hop(name)
            && !headers
                .get_all(axum::http::header::CONNECTION)
                .iter()
                .filter_map(|value| value.to_str().ok())
                .flat_map(|value| value.split(','))
                .any(|token| token.trim().eq_ignore_ascii_case(name.as_str()))
    })
}

fn with_upstream_headers(
    mut request: reqwest::RequestBuilder,
    headers: &HeaderMap,
) -> reqwest::RequestBuilder {
    for (name, value) in filtered_headers(headers) {
        // The SSE usage tee parses the upstream bytes directly. Do not negotiate
        // compressed bytes that it cannot parse (reqwest compression is disabled).
        if name == axum::http::header::ACCEPT_ENCODING {
            continue;
        }
        request = request.header(name, value);
    }
    request
}

fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "host"
            | "content-length"
            | "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;

    use axum::http::Method;
    use axum::response::IntoResponse;
    use futures_util::stream;
    use serde_json::{json, Value};

    fn tool_pair(id: &str, text: &str) -> [Value; 2] {
        [
            json!({"role":"assistant","content":[{
                "type":"tool_use","id":id,"name":"Read",
                "input":{"file_path":"/tmp/a.rs"}
            }]}),
            json!({"role":"user","content":[{
                "type":"tool_result","tool_use_id":id,
                "content":[{"type":"text","text":text}]
            }]}),
        ]
    }

    fn high_water_request(padding: usize) -> Value {
        let text = "x".repeat(700);
        let mut messages: Vec<Value> = tool_pair("tu_1", &text)
            .into_iter()
            .chain(tool_pair("tu_2", &text))
            .collect();
        messages.extend((0..12).map(|index| {
            json!({"role":"user","content":[{"type":"text","text":format!("filler {index}")}]})
        }));
        json!({
            "model":"claude-3-5-sonnet-20241022",
            "messages":messages,
            "padding":"z".repeat(padding)
        })
    }

    async fn start_fake_upstream() -> (String, tokio::task::JoinHandle<()>) {
        async fn handler(request: Request<Body>) -> Response<Body> {
            if request.uri().path() == "/v1/messages/count_tokens" {
                let method = request.method().to_string();
                let path = request.uri().path().to_owned();
                let body = to_bytes(request.into_body(), usize::MAX).await.unwrap();
                return axum::Json(json!({
                    "method": method,
                    "path": path,
                    "bodyLength": body.len(),
                }))
                .into_response();
            }

            let chunks = stream::unfold(0, |index| async move {
                if index == 3 {
                    return None;
                }
                if index > 0 {
                    tokio::time::sleep(Duration::from_millis(40)).await;
                }
                Some((
                    Ok::<_, Infallible>(Bytes::from(format!("data: {index}\n\n"))),
                    index + 1,
                ))
            });
            Response::builder()
                .status(StatusCode::ACCEPTED)
                .header("content-type", "text/event-stream")
                .body(Body::from_stream(chunks))
                .unwrap()
        }

        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().fallback(any(handler));
        let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{address}"), handle)
    }

    async fn start_proxy(upstream: &str) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let mut state = AppState::for_tests().await;
        state.ctx_proxy = Arc::new(ProxyRuntime::with_port(port));
        *state.ctx_proxy.upstream.write().unwrap() = upstream.to_owned();
        let state = Arc::new(state);
        let handle = tokio::spawn(serve(state.clone()));
        tokio::time::timeout(Duration::from_secs(2), async {
            while state.ctx_proxy.active_port().is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        (format!("http://127.0.0.1:{port}"), handle)
    }

    #[tokio::test]
    async fn forwards_arbitrary_paths_and_streams_responses() {
        let (upstream, upstream_handle) = start_fake_upstream().await;
        let (proxy, proxy_handle) = start_proxy(&upstream).await;
        let client = reqwest::Client::new();

        let body = br#"{"messages":[]}"#;
        let echo = client
            .request(Method::POST, format!("{proxy}/v1/messages/count_tokens"))
            .body(body.as_slice())
            .send()
            .await
            .unwrap();
        assert_eq!(echo.status(), StatusCode::OK);
        assert_eq!(
            echo.json::<serde_json::Value>().await.unwrap(),
            json!({"method":"POST", "path":"/v1/messages/count_tokens", "bodyLength":body.len()})
        );

        let response = client
            .post(format!("{proxy}/v1/messages"))
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let mut stream = response.bytes_stream();
        let mut chunks = Vec::new();
        while let Some(chunk) = stream.next().await {
            chunks.push(chunk.unwrap());
        }
        assert!(
            chunks.len() >= 3,
            "expected streaming chunks, got {}",
            chunks.len()
        );
        assert_eq!(chunks.concat(), b"data: 0\n\ndata: 1\n\ndata: 2\n\n");

        proxy_handle.abort();
        upstream_handle.abort();
    }

    #[test]
    fn upstream_request_does_not_copy_accept_encoding() {
        let mut inbound = HeaderMap::new();
        inbound.insert(
            axum::http::header::ACCEPT_ENCODING,
            axum::http::HeaderValue::from_static("gzip, br, zstd"),
        );
        inbound.insert(
            axum::http::header::HeaderName::from_static("x-test-header"),
            axum::http::HeaderValue::from_static("preserved"),
        );

        let outbound = with_upstream_headers(
            reqwest::Client::new().post("https://api.anthropic.com/v1/messages"),
            &inbound,
        )
        .build()
        .unwrap();

        assert!(outbound
            .headers()
            .get(axum::http::header::ACCEPT_ENCODING)
            .is_none());
        assert_eq!(
            outbound.headers().get("x-test-header").unwrap(),
            "preserved"
        );
    }

    #[test]
    fn rewrite_body_reevaluates_and_elides_identical_read() {
        let rt = ProxyRuntime::with_port(0);
        rt.mode.store(MODE_REWRITE, Ordering::Release);
        let original = serde_json::to_vec(&high_water_request(560_000)).unwrap();

        let outcome = rewrite_body(&rt, &original);

        assert_eq!(outcome.decision, "reevaluate");
        assert_eq!(outcome.elisions, 1);
        assert!(outcome.bytes_saved > 0);
        assert!(outcome.body.len() < original.len());
        let rewritten: Value = serde_json::from_slice(&outcome.body).unwrap();
        assert!(rewritten["messages"][1]["content"][0]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("[ctxopt] elided:"));
    }

    #[test]
    fn rewrite_body_keeps_small_and_invalid_requests_byte_identical() {
        let rt = ProxyRuntime::with_port(0);
        rt.mode.store(MODE_REWRITE, Ordering::Release);
        let small = br#"{"model":"claude-3-5-sonnet","messages":[]}"#;
        let outcome = rewrite_body(&rt, small);
        assert_eq!(outcome.decision, "passthrough");
        assert_eq!(outcome.body, small);

        let invalid = b"not json";
        let outcome = rewrite_body(&rt, invalid);
        assert_eq!(outcome.decision, "parse-error");
        assert_eq!(outcome.body, invalid);
    }

    #[test]
    fn reevaluation_never_rewrites_a_previously_frozen_stub() {
        let rt = ProxyRuntime::with_port(0);
        rt.mode.store(MODE_REWRITE, Ordering::Release);
        let first = high_water_request(560_000);
        rewrite_body(&rt, &serde_json::to_vec(&first).unwrap());
        let first_stub = {
            let mut ledger = rt.ledger.lock().unwrap();
            let index = ledger.observe(&first["messages"]);
            ledger.conv_mut(index).frozen[0].stub.clone()
        };

        let mut second = first;
        let text = "x".repeat(700);
        second["messages"]
            .as_array_mut()
            .unwrap()
            .extend(tool_pair("tu_3", &text));
        second["padding"] = Value::String("z".repeat(630_000));
        let outcome = rewrite_body(&rt, &serde_json::to_vec(&second).unwrap());
        assert_eq!(outcome.decision, "reevaluate");

        let mut ledger = rt.ledger.lock().unwrap();
        let index = ledger.observe(&second["messages"]);
        let frozen = &ledger.conv_mut(index).frozen;
        assert_eq!(
            frozen
                .iter()
                .find(|elision| elision.tool_use_id == "tu_1")
                .unwrap()
                .stub,
            first_stub
        );
    }

    #[test]
    fn sse_usage_parser_handles_awkward_chunk_boundaries() {
        let sse = concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{",
            "\"input_tokens\":100,\"cache_read_input_tokens\":20,",
            "\"cache_creation_input_tokens\":5}}}\n\n",
            "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":42}}\n\n"
        )
        .as_bytes();
        let mut parser = SseUsageParser::default();
        for range in [0..7, 7..53, 53..119, 119..sse.len()] {
            parser.push(&sse[range]);
        }
        parser.finish();
        assert_eq!(
            parser.usage,
            UsageTotals {
                input_tokens: Some(100),
                cache_read: Some(20),
                cache_creation: Some(5),
                output_tokens: Some(42),
            }
        );
    }

    #[tokio::test]
    async fn completion_updates_the_matching_conversation_usage() {
        let runtime = Arc::new(ProxyRuntime::with_port(0));
        runtime.mode.store(MODE_REWRITE, Ordering::Release);
        let request = high_water_request(560_000);
        let outcome = rewrite_body(&runtime, &serde_json::to_vec(&request).unwrap());
        let mut state = AppState::for_tests().await;
        state.ctx_proxy = runtime.clone();

        on_request_complete(
            &state,
            &outcome,
            UsageTotals {
                input_tokens: Some(100),
                cache_read: Some(20),
                cache_creation: Some(5),
                output_tokens: Some(42),
            },
        )
        .await;

        let mut ledger = runtime.ledger.lock().unwrap();
        let index = ledger.observe(&request["messages"]);
        assert_eq!(ledger.conv_mut(index).last_input_tokens, Some(125));
        drop(ledger);
        let report = crate::engine::repo::proxy_metric::report(&state.db, 24)
            .await
            .unwrap();
        assert_eq!(report.requests, 1);
        assert_eq!(report.input_tokens, 100);
        assert_eq!(report.cache_read_tokens, 20);
    }
}
