//! App-global loopback proxy for Anthropic API traffic.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use axum::body::{to_bytes, Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, Request, Response, StatusCode};
use axum::routing::any;
use axum::Router;
use futures_util::{stream, StreamExt};
use serde_json::Value;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::engine::runtime::count_tokens::CountCredential;
use crate::engine::state::AppState;

/// Minimum wall-clock gap between two checkpoint samples (global rate cap, on
/// top of the concurrency semaphore). Sustained eligible traffic cannot fan out
/// unbounded count_tokens calls (spec §7.1 / security containment ea3df57c).
const CHECKPOINT_SAMPLE_COOLDOWN_MS: u64 = 60_000;

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
    /// GLOBAL log-mode checkpoint toggle (spec §6), default off.
    pub checkpoint: AtomicBool,
    /// C: evaluate a checkpoint above this effective-context estimate (tokens).
    pub ceiling: AtomicU32,
    /// M: floor on projected net token saving to proceed to sampling.
    pub min_net_saving: AtomicU32,
    /// L: projected post-checkpoint tokens must land at/below this.
    pub low_water: AtomicU32,
    /// Recent-tail messages kept verbatim (never frozen).
    pub tail_msgs: AtomicU32,
    /// Count of eligible samples dropped by the rate/concurrency caps. Observable
    /// telemetry so a drop is never silent (security containment ea3df57c).
    pub samples_dropped: AtomicU64,
    /// UNIX-millis of the last scheduled sample; 0 = never (global cooldown gate).
    last_sample_at_ms: AtomicU64,
    /// Bounds concurrent off-path count_tokens sampling (never blocks forwarding).
    sample_permits: Arc<Semaphore>,
    /// Dedicated, redirect-contained client for count_tokens (never the forwarding
    /// client): follows NO redirects so an x-api-key cannot leak cross-host.
    count_client: reqwest::Client,
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
            checkpoint: AtomicBool::new(false),
            ceiling: AtomicU32::new(ctxopt::DEFAULT_CEILING_TOKENS as u32),
            min_net_saving: AtomicU32::new(ctxopt::MIN_NET_SAVING_TOKENS as u32),
            low_water: AtomicU32::new(ctxopt::LOW_WATER_TOKENS as u32),
            tail_msgs: AtomicU32::new(ctxopt::RECENT_TAIL_MSGS as u32),
            samples_dropped: AtomicU64::new(0),
            last_sample_at_ms: AtomicU64::new(0),
            sample_permits: Arc::new(Semaphore::new(2)),
            count_client: crate::engine::runtime::count_tokens::count_client(),
        }
    }

    fn client_for_count(&self) -> reqwest::Client {
        self.count_client.clone()
    }

    /// Decide whether an eligible checkpoint sample may run NOW: the global
    /// cooldown must have elapsed AND a concurrency permit must be free. Any
    /// refusal increments `samples_dropped` so the drop is recorded, never
    /// silent. Returns the held permit on success (released when the sample task
    /// finishes).
    fn try_begin_sample(&self) -> Option<OwnedSemaphorePermit> {
        let now = now_ms();
        let last = self.last_sample_at_ms.load(Ordering::Acquire);
        if last != 0 && now.saturating_sub(last) < CHECKPOINT_SAMPLE_COOLDOWN_MS {
            self.samples_dropped.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        match self.sample_permits.clone().try_acquire_owned() {
            Ok(permit) => {
                self.last_sample_at_ms.store(now, Ordering::Release);
                Some(permit)
            }
            Err(_) => {
                self.samples_dropped.fetch_add(1, Ordering::Relaxed);
                None
            }
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
    let is_messages_post =
        parts.method == axum::http::Method::POST && parts.uri.path() == "/v1/messages";
    let should_optimize =
        is_messages_post && state.ctx_proxy.mode.load(Ordering::Acquire) != MODE_OFF;
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

    // Log-mode checkpoint measurement (spec §7.1). Runs ONLY after the original
    // forward returns a success status, reads `body`/headers by reference, and
    // spawns off the forwarding path — it never derives `upstream_body`, so the
    // forwarded bytes are byte-identical whether or not this fires. The job
    // captures the exact `upstream` used for THIS forward (no async TOCTOU).
    if is_messages_post
        && status.is_success()
        && state.ctx_proxy.checkpoint.load(Ordering::Acquire)
    {
        if let Some(job) = checkpoint_gate(&state.ctx_proxy, &body, &upstream) {
            let cred = credential_from_headers(&parts.headers);
            if let Some(permit) = state.ctx_proxy.try_begin_sample() {
                let sample_state = state.clone();
                tokio::spawn(async move {
                    let _permit = permit; // released on completion
                    sample_checkpoint(sample_state, cred, job).await;
                });
            }
            // try_begin_sample() already recorded the drop if it returned None.
        }
    }

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

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// A projected checkpoint the measurement path will sample OFF the forwarding
/// path. It carries the ORIGINAL request (for count `a`), the projected message
/// list (count `b`), and — critically — the exact `upstream` captured for the
/// original forward, so a later change to the global upstream can never retarget
/// the in-flight credential (security containment ea3df57c).
struct CheckpointJob {
    model: String,
    upstream: String,
    original: Value,
    projected_messages: Value,
    earliest_changed_msg_index: usize,
    earliest_changed_byte: usize,
    gross_candidate_bytes: usize,
    stub_overhead_bytes: usize,
    non_recoverable_kept_bytes: usize,
    projected_post_tokens: usize,
    est_whole_tokens: usize,
}

/// Sync pre-gate: parse the body, plan + project via ctxopt, and return an
/// eligible job or None. Reads `body` by reference and produces NO forwardable
/// bytes — it is structurally incapable of altering the forwarded request
/// (Global Constraint: NEVER alter forwarded bytes in M1).
fn checkpoint_gate(rt: &ProxyRuntime, body: &[u8], upstream: &str) -> Option<CheckpointJob> {
    if !rt.checkpoint.load(Ordering::Acquire) {
        return None;
    }
    let original: Value = serde_json::from_slice(body).ok()?;
    let messages = original.get("messages")?.clone();
    let model = original
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let est_whole_tokens = ctxopt::estimate::est_tokens(body.len());
    let ceiling = rt.ceiling.load(Ordering::Acquire) as usize;
    let tail = rt.tail_msgs.load(Ordering::Acquire) as usize;
    let plan = ctxopt::checkpoint::plan_checkpoint(&messages, est_whole_tokens, ceiling, tail)?;
    let m = rt.min_net_saving.load(Ordering::Acquire) as usize;
    let l = rt.low_water.load(Ordering::Acquire) as usize;
    match ctxopt::checkpoint::project(&messages, &plan, est_whole_tokens, m, l) {
        ctxopt::checkpoint::CheckpointOutcome::Saturated => None,
        ctxopt::checkpoint::CheckpointOutcome::Eligible(p) => Some(CheckpointJob {
            model,
            upstream: upstream.to_owned(),
            original,
            projected_messages: p.projected_messages,
            earliest_changed_msg_index: plan.earliest_changed_msg_index,
            earliest_changed_byte: plan.candidates.first().map_or(0, |c| c.gross_bytes),
            gross_candidate_bytes: p.gross_candidate_bytes,
            stub_overhead_bytes: p.stub_overhead_bytes,
            non_recoverable_kept_bytes: plan.non_recoverable_kept_bytes,
            projected_post_tokens: p.projected_post_tokens,
            est_whole_tokens,
        }),
    }
}

/// Lift ONLY the allowlisted auth headers off the forwarded request. Values are
/// used in-flight for count_tokens and dropped; never logged or persisted.
fn credential_from_headers(headers: &HeaderMap) -> CountCredential {
    let get = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    CountCredential {
        api_key: get("x-api-key"),
        authorization: get("authorization"),
        anthropic_version: get("anthropic-version").unwrap_or_else(|| "2023-06-01".to_owned()),
    }
}

/// Off-path sampler: counts a=original, b=projected, c=prefix via the dedicated
/// count client against the job's CAPTURED upstream, derives S_net=a−b, R=a−c,
/// q=S_net/R, folds plateau state, and persists one metric row. Fail-open: any
/// count error records `count_failure = 1` with the bytes/4 diagnostic instead of
/// failing (it cannot affect the already-forwarded request).
async fn sample_checkpoint(state: Arc<AppState>, cred: CountCredential, job: CheckpointJob) {
    use crate::engine::runtime::count_tokens as ct;
    // AMENDMENT ea3df57c: use the upstream captured for the original forward,
    // NEVER re-read state.ctx_proxy.upstream (which may have changed since).
    let upstream = job.upstream.trim_end_matches('/').to_owned();
    let client = state.ctx_proxy.client_for_count();
    let body_a = ct::count_tokens_body(&job.original, &job.original["messages"]);
    let body_b = ct::count_tokens_body(&job.original, &job.projected_messages);
    let prefix = ct::prefix_messages(&job.original["messages"], job.earliest_changed_msg_index);
    let body_c = ct::count_tokens_body(&job.original, &prefix);

    let counts = async {
        let a = ct::count_tokens(&client, &upstream, &cred, &body_a).await?;
        let b = ct::count_tokens(&client, &upstream, &cred, &body_b).await?;
        let c = ct::count_tokens(&client, &upstream, &cred, &body_c).await?;
        Ok::<_, String>((a, b, c))
    }
    .await;

    let plateau = {
        let mut ledger = state
            .ctx_proxy
            .ledger
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let idx = ledger.observe(&job.original["messages"]);
        ctxopt::ledger::record_plateau(ledger.conv_mut(idx), job.earliest_changed_msg_index)
    };

    let bytes_est = ctxopt::estimate::est_tokens(
        job.gross_candidate_bytes
            .saturating_sub(job.stub_overhead_bytes),
    );
    let mut row = crate::engine::repo::proxy_checkpoint_metric::CheckpointMetricInsert {
        created_at: chrono::Utc::now().to_rfc3339(),
        model: job.model.clone(),
        earliest_changed_byte: saturating_i64(job.earliest_changed_byte as u64),
        earliest_changed_msg: saturating_i64(job.earliest_changed_msg_index as u64),
        r_tokens: 0,
        gross_candidate_tokens: saturating_i64(
            ctxopt::estimate::est_tokens(job.gross_candidate_bytes) as u64,
        ),
        stub_overhead_tokens: saturating_i64(
            ctxopt::estimate::est_tokens(job.stub_overhead_bytes) as u64,
        ),
        s_net_tokens: 0,
        q: 0.0,
        projected_break_even: f64::INFINITY,
        projected_post_tokens: saturating_i64(job.projected_post_tokens as u64),
        plateau_turns: i64::from(plateau),
        non_recoverable_kept_tokens: saturating_i64(
            ctxopt::estimate::est_tokens(job.non_recoverable_kept_bytes) as u64,
        ),
        provider_estimate: 1,
        count_failure: 0,
        method_version: ct::CHECKPOINT_METHOD_VERSION.to_owned(),
        bytes_est_tokens: saturating_i64(bytes_est as u64),
    };
    match counts {
        Ok((a, b, c)) => {
            let r = a.saturating_sub(c);
            let s_net = a.saturating_sub(b);
            let q = if r == 0 { 0.0 } else { s_net as f64 / r as f64 };
            row.r_tokens = saturating_i64(r);
            row.s_net_tokens = saturating_i64(s_net);
            row.q = q;
            row.projected_break_even = if q > 0.0 { 11.5 / q - 12.5 } else { f64::INFINITY };
            row.projected_post_tokens = saturating_i64(b);
        }
        Err(error) => {
            eprintln!("[ctx-proxy] checkpoint count_tokens failed: {error}");
            row.count_failure = 1;
        }
    }
    let _ = job.est_whole_tokens; // captured for diagnostics; not persisted directly
    if let Err(error) = crate::engine::repo::proxy_checkpoint_metric::insert(&state.db, row).await {
        eprintln!("[ctx-proxy] failed to record checkpoint metric: {error}");
    }
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

        {
            let mut ledger = runtime.ledger.lock().unwrap();
            let index = ledger.observe(&request["messages"]);
            assert_eq!(ledger.conv_mut(index).last_input_tokens, Some(125));
        } // guard scoped out before the await below (clippy await_holding_lock)
        let report = crate::engine::repo::proxy_metric::report(&state.db, 24)
            .await
            .unwrap();
        assert_eq!(report.requests, 1);
        assert_eq!(report.input_tokens, 100);
        assert_eq!(report.cache_read_tokens, 20);
    }

    // ---- Checkpoint (log-mode measurement) ----------------------------------

    use crate::engine::runtime::count_tokens::CountCredential;

    fn test_cred() -> CountCredential {
        CountCredential {
            api_key: Some("k".into()),
            authorization: None,
            anthropic_version: "2023-06-01".into(),
        }
    }

    fn eligible_runtime(port: u16) -> ProxyRuntime {
        let rt = ProxyRuntime::with_port(port);
        rt.checkpoint.store(true, Ordering::Release);
        rt.ceiling.store(1, Ordering::Release); // force above-ceiling
        rt.min_net_saving.store(1, Ordering::Release); // trivial M
        rt.low_water.store(u32::MAX, Ordering::Release); // trivial L
        rt.tail_msgs.store(2, Ordering::Release); // push the reads out of the tail
        rt
    }

    /// Fake upstream that answers count_tokens with `{input_tokens: bodyLength}`
    /// and streams a success SSE for /v1/messages.
    async fn start_checkpoint_upstream() -> (String, tokio::task::JoinHandle<()>) {
        async fn handler(request: Request<Body>) -> Response<Body> {
            if request.uri().path() == "/v1/messages/count_tokens" {
                let body = to_bytes(request.into_body(), usize::MAX).await.unwrap();
                return axum::Json(json!({ "input_tokens": body.len() })).into_response();
            }
            let chunks = stream::unfold(0, |index| async move {
                if index == 3 {
                    return None;
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

    /// Fake upstream that FAILS /v1/messages with 500 (count path still answers).
    async fn start_failing_checkpoint_upstream() -> (String, tokio::task::JoinHandle<()>) {
        async fn handler(request: Request<Body>) -> Response<Body> {
            if request.uri().path() == "/v1/messages/count_tokens" {
                let body = to_bytes(request.into_body(), usize::MAX).await.unwrap();
                return axum::Json(json!({ "input_tokens": body.len() })).into_response();
            }
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("nope"))
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

    /// Bring up a real proxy with checkpoint enabled + eligible thresholds; hand
    /// back the shared state so the test can read the metric table.
    async fn start_checkpoint_proxy(
        upstream: &str,
    ) -> (String, Arc<AppState>, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut state = AppState::for_tests().await;
        let rt = eligible_runtime(port);
        *rt.upstream.write().unwrap() = upstream.to_owned();
        state.ctx_proxy = Arc::new(rt);
        let state = Arc::new(state);
        let handle = tokio::spawn(serve(state.clone()));
        tokio::time::timeout(Duration::from_secs(2), async {
            while state.ctx_proxy.active_port().is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        (format!("http://127.0.0.1:{port}"), state, handle)
    }

    #[test]
    fn checkpoint_gate_off_by_default_and_never_yields_a_forward_body() {
        let rt = ProxyRuntime::with_port(0);
        let body = serde_json::to_vec(&high_water_request(560_000)).unwrap();
        assert!(checkpoint_gate(&rt, &body, "http://up").is_none());
    }

    #[test]
    fn checkpoint_gate_when_enabled_projects_and_captures_the_upstream() {
        let rt = eligible_runtime(0);
        let body = serde_json::to_vec(&high_water_request(1_000)).unwrap();
        let job = checkpoint_gate(&rt, &body, "http://captured-upstream").expect("eligible job");
        // The job carries the ORIGINAL request unchanged (for count `a`) and the
        // exact upstream captured for THIS forward; the projected list is separate.
        assert_eq!(job.upstream, "http://captured-upstream");
        assert_eq!(serde_json::to_vec(&job.original).unwrap().len(), body.len());
        assert!(job.projected_messages != job.original["messages"]);
        assert!(
            job.earliest_changed_msg_index <= job.original["messages"].as_array().unwrap().len()
        );
    }

    #[tokio::test]
    async fn sample_checkpoint_persists_a_metric_row() {
        let (upstream, up) = start_checkpoint_upstream().await;
        let mut state = AppState::for_tests().await;
        let rt = Arc::new(eligible_runtime(0));
        state.ctx_proxy = rt.clone();
        let state = Arc::new(state);
        let body = serde_json::to_vec(&high_water_request(1_000)).unwrap();
        let job = checkpoint_gate(&rt, &body, &upstream).unwrap();
        sample_checkpoint(state.clone(), test_cred(), job).await;
        let report = crate::engine::repo::proxy_checkpoint_metric::report(&state.db, 24)
            .await
            .unwrap();
        assert_eq!(report.samples, 1);
        assert_eq!(report.count_failures, 0); // counts succeeded against the fake
        up.abort();
    }

    // CONTAINMENT (a): mutating the global upstream between capture and sampling
    // cannot retarget the in-flight credential — the job holds the captured host.
    #[tokio::test]
    async fn mutating_global_upstream_cannot_retarget_the_captured_credential() {
        let (good_upstream, up) = start_checkpoint_upstream().await;
        let mut state = AppState::for_tests().await;
        let rt = Arc::new(eligible_runtime(0));
        // Global upstream is a dead host BEFORE capture …
        *rt.upstream.write().unwrap() = "http://127.0.0.1:1".to_owned();
        state.ctx_proxy = rt.clone();
        let state = Arc::new(state);
        let body = serde_json::to_vec(&high_water_request(1_000)).unwrap();
        // … capture against the GOOD upstream (as forward_inner does) …
        let job = checkpoint_gate(&rt, &body, &good_upstream).unwrap();
        // … then mutate the global upstream again to another dead host.
        *rt.upstream.write().unwrap() = "http://127.0.0.1:2".to_owned();
        sample_checkpoint(state.clone(), test_cred(), job).await;
        let report = crate::engine::repo::proxy_checkpoint_metric::report(&state.db, 24)
            .await
            .unwrap();
        assert_eq!(report.samples, 1);
        // Counts SUCCEEDED because the job used the captured good upstream, never
        // the mutated dead global one.
        assert_eq!(report.count_failures, 0);
        up.abort();
    }

    // CONTAINMENT (b): the cooldown + permit cap bound fan-out and every refusal
    // is recorded in `samples_dropped`, never silently lost.
    #[test]
    fn try_begin_sample_enforces_cooldown_and_records_drops() {
        let rt = ProxyRuntime::with_port(0);
        let first = rt.try_begin_sample();
        assert!(first.is_some(), "first sample passes (no prior cooldown)");
        // Immediate follow-ups are inside the 60s cooldown → dropped + recorded.
        assert!(rt.try_begin_sample().is_none());
        assert!(rt.try_begin_sample().is_none());
        assert_eq!(rt.samples_dropped.load(Ordering::Acquire), 2);
        drop(first);
    }

    // CONTAINMENT (c) negative: a non-success upstream response schedules nothing.
    #[tokio::test]
    async fn no_sample_scheduled_when_upstream_fails() {
        let (upstream, up) = start_failing_checkpoint_upstream().await;
        let (proxy, state, ph) = start_checkpoint_proxy(&upstream).await;
        let client = reqwest::Client::new();
        let body = serde_json::to_vec(&high_water_request(1_000)).unwrap();
        let resp = client
            .post(format!("{proxy}/v1/messages"))
            .body(body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let _ = resp.bytes().await; // ensure forward_inner ran past the gate
        // The scheduling decision is synchronous in forward_inner; a failed status
        // never spawns a sampler.
        let report = crate::engine::repo::proxy_checkpoint_metric::report(&state.db, 24)
            .await
            .unwrap();
        assert_eq!(report.samples, 0);
        ph.abort();
        up.abort();
    }

    // CONTAINMENT (c) positive: a successful response DOES schedule a sample.
    #[tokio::test]
    async fn sample_scheduled_after_successful_response() {
        let (upstream, up) = start_checkpoint_upstream().await;
        let (proxy, state, ph) = start_checkpoint_proxy(&upstream).await;
        let client = reqwest::Client::new();
        let body = serde_json::to_vec(&high_water_request(1_000)).unwrap();
        let resp = client
            .post(format!("{proxy}/v1/messages"))
            .header("x-api-key", "k") // credential lifted for the off-path count
            .body(body)
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());
        let _ = resp.bytes().await;
        // The sample spawns off-path; poll the metric table with a generous deadline.
        let report = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let r = crate::engine::repo::proxy_checkpoint_metric::report(&state.db, 24)
                    .await
                    .unwrap();
                if r.samples >= 1 {
                    return r;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("a checkpoint sample must be recorded after a successful forward");
        assert_eq!(report.samples, 1);
        assert_eq!(report.count_failures, 0);
        ph.abort();
        up.abort();
    }
}
