//! Anthropic count_tokens client + credential preflight for the ctx-proxy
//! checkpoint gate. Runs OFF the forwarding path; returns provider ESTIMATES.
//!
//! SECURITY: the count client follows NO redirects (Policy::none) so a 3xx can
//! never trigger a second, credential-bearing request to another host (reqwest
//! 0.12 does not strip x-api-key cross-host). Auth values are applied as
//! set_sensitive HeaderValues from a fixed allowlist; missing auth makes no call.

use std::time::Duration;

use serde_json::{json, Value};

pub const CHECKPOINT_METHOD_VERSION: &str = "m1-count_tokens-2023-06-01";

/// Anthropic API error `type` enum (docs.anthropic.com/en/api/errors). On a
/// count_tokens failure, the upstream-supplied `error.type` is persisted into a
/// metric row ONLY when it EXACTLY matches one of these fixed, content-free
/// literals — so a hostile or nonstandard upstream cannot smuggle response
/// content through the field (rulings e376fec5 + f8651210, challenge fda10918).
/// Any other value degrades to a status-only label.
const KNOWN_ERROR_TYPES: &[&str] = &[
    "invalid_request_error",
    "authentication_error",
    "permission_error",
    "not_found_error",
    "request_too_large",
    "rate_limit_error",
    "api_error",
    "overloaded_error",
];

/// Auth headers lifted from the forwarded /v1/messages request. Used in-flight
/// for count_tokens and dropped; never logged or persisted. Clone-only by
/// design — no Debug/Serialize/Deserialize so a credential can never be
/// accidentally formatted into a log or serialized into a metric row.
#[derive(Clone)]
pub struct CountCredential {
    pub api_key: Option<String>,       // x-api-key
    pub authorization: Option<String>, // Authorization: Bearer …
    pub anthropic_version: String,     // anthropic-version header
    pub anthropic_beta: Option<String>, // anthropic-beta (required by OAuth Bearer + [1m] context)
}

impl CountCredential {
    fn has_auth(&self) -> bool {
        self.api_key.is_some() || self.authorization.is_some()
    }
}

/// The dedicated count client: NO redirects, explicit timeout, no retries.
pub fn count_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()
        .expect("count_tokens client builder cannot fail")
}

/// Build a count_tokens body: model + the given messages + optional
/// system/tools/tool_choice (max_tokens/stream/metadata dropped).
pub fn count_tokens_body(request: &Value, messages: &Value) -> Value {
    let mut body = json!({
        "model": request.get("model").cloned().unwrap_or(Value::Null),
        "messages": messages.clone(),
    });
    for key in ["system", "tools", "tool_choice"] {
        if let Some(v) = request.get(key) {
            body[key] = v.clone();
        }
    }
    body
}

/// Structurally valid prefix ending at the message boundary BEFORE `earliest_changed_msg_index`.
pub fn prefix_messages(messages: &Value, earliest_changed_msg_index: usize) -> Value {
    match messages.as_array() {
        Some(arr) => {
            let end = earliest_changed_msg_index.min(arr.len());
            Value::Array(arr[..end].to_vec())
        }
        None => Value::Array(Vec::new()),
    }
}

fn sensitive_header(value: &str) -> Option<reqwest::header::HeaderValue> {
    let mut hv = reqwest::header::HeaderValue::from_str(value).ok()?;
    hv.set_sensitive(true);
    Some(hv)
}

/// Apply ONLY the allowlisted auth headers, each marked sensitive.
fn apply_cred(mut req: reqwest::RequestBuilder, cred: &CountCredential) -> reqwest::RequestBuilder {
    if let Some(key) = cred.api_key.as_deref().and_then(sensitive_header) {
        req = req.header("x-api-key", key);
    }
    if let Some(auth) = cred.authorization.as_deref().and_then(sensitive_header) {
        req = req.header("authorization", auth);
    }
    // anthropic-beta is not a secret (ruling ea3df57c amendment) but is required
    // for OAuth Bearer auth to be accepted and for [1m] 1M-context. Forward the
    // whole value verbatim — it may be a comma-separated list; never split it.
    if let Some(beta) = cred.anthropic_beta.as_deref() {
        if let Ok(hv) = reqwest::header::HeaderValue::from_str(beta) {
            req = req.header("anthropic-beta", hv);
        }
    }
    req.header("anthropic-version", cred.anthropic_version.as_str())
        .header("content-type", "application/json")
}

/// POST <upstream>/v1/messages/count_tokens; returns provider input_tokens estimate.
/// Missing auth → Err with NO network call.
pub async fn count_tokens(
    client: &reqwest::Client,
    upstream: &str,
    cred: &CountCredential,
    body: &Value,
) -> Result<u64, String> {
    if !cred.has_auth() {
        return Err("count_tokens: missing auth credential; no request issued".to_string());
    }
    let url = format!(
        "{}/v1/messages/count_tokens",
        upstream.trim_end_matches('/')
    );
    let resp = apply_cred(client.post(url), cred)
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        let status = resp.status();
        // SECURITY (rulings e376fec5 + f8651210 / invariant
        // proxy_checkpoint_metric.rs:3-5): the raw upstream body must NEVER enter
        // this repository. Persist ONLY the http status plus an `error.type` that
        // EXACTLY matches a known Anthropic enum literal (KNOWN_ERROR_TYPES). A
        // length cap is NOT a content guarantee (challenge fda10918) — the
        // allowlist is. `error.message` may echo request content, so it is never
        // surfaced; an unknown/absent type or a non-JSON body degrades to a
        // status-only label, never raw bytes.
        let body = resp.text().await.unwrap_or_default();
        return Err(match serde_json::from_str::<Value>(&body) {
            // Parseable JSON: keep error.type ONLY if it is an allowlisted enum
            // literal; any other value (hostile, novel, or absent) → status-only.
            Ok(v) => match v
                .get("error")
                .and_then(|e| e.get("type"))
                .and_then(Value::as_str)
            {
                Some(t) if KNOWN_ERROR_TYPES.contains(&t) => {
                    format!("count_tokens HTTP {status}: {t}")
                }
                _ => format!("count_tokens HTTP {status} (unknown type)"),
            },
            // Body was not JSON at all → status only, never raw bytes.
            Err(_) => format!("count_tokens HTTP {status} (unparsed)"),
        });
    }
    let value: Value = resp.json().await.map_err(|e| e.to_string())?;
    value
        .get("input_tokens")
        .and_then(Value::as_u64)
        .ok_or_else(|| "count_tokens: missing input_tokens".to_string())
}

/// Credential preflight (plan prerequisite): a trivial count_tokens call proving
/// the live credential is authorized. Ok(()) iff HTTP 200.
///
/// M1 has no non-test caller yet: the live count path in `sample_checkpoint`
/// already surfaces auth failures via `count_failure`, so this standalone probe
/// is retained for an explicit live check (no CLI entry point wired in M1 — see
/// the READY note / escalation to the lead).
#[allow(dead_code)]
pub async fn preflight(
    client: &reqwest::Client,
    upstream: &str,
    cred: &CountCredential,
) -> Result<(), String> {
    let body = json!({ "model": "claude-3-5-haiku-20241022",
        "messages": [{ "role": "user", "content": "ok" }] });
    count_tokens(client, upstream, cred, &body)
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, Response, StatusCode};
    use axum::routing::any;
    use axum::Router;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    // Fake upstream: counts requests, 401s on x-api-key "bad", else echoes a token count.
    async fn fake_upstream() -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        let hits = Arc::new(AtomicUsize::new(0));
        let hits2 = hits.clone();
        let app = Router::new().fallback(any(move |request: Request<Body>| {
            let hits = hits2.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                let unauth = request
                    .headers()
                    .get("x-api-key")
                    .map(|v| v == "bad")
                    .unwrap_or(false);
                if unauth {
                    return Response::builder()
                        .status(StatusCode::UNAUTHORIZED)
                        .body(Body::from("no"))
                        .unwrap();
                }
                let body = to_bytes(request.into_body(), usize::MAX).await.unwrap();
                let count = body.len() as u64;
                Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "application/json")
                    .body(Body::from(json!({ "input_tokens": count }).to_string()))
                    .unwrap()
            }
        }));
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), hits, handle)
    }

    fn cred(key: &str) -> CountCredential {
        CountCredential {
            api_key: Some(key.into()),
            authorization: None,
            anthropic_version: "2023-06-01".into(),
            anthropic_beta: None,
        }
    }

    #[test]
    fn body_builder_keeps_model_and_swaps_messages_and_drops_max_tokens() {
        let req = json!({ "model": "claude-x", "max_tokens": 999, "stream": true,
            "system": "s", "tools": [{"name":"Read"}], "messages": [{"role":"user","content":"a"}] });
        let msgs = json!([{"role":"user","content":"b"}]);
        let body = count_tokens_body(&req, &msgs);
        assert_eq!(body["model"], "claude-x");
        assert_eq!(body["system"], "s");
        assert_eq!(body["tools"][0]["name"], "Read");
        assert_eq!(body["messages"], msgs);
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("stream").is_none());
    }

    #[test]
    fn prefix_truncates_at_message_boundary() {
        let msgs = json!([{"role":"user","content":"0"},{"role":"assistant","content":"1"},{"role":"user","content":"2"}]);
        let p = prefix_messages(&msgs, 2);
        assert_eq!(p.as_array().unwrap().len(), 2);
        assert_eq!(p[1]["content"], "1");
    }

    #[tokio::test]
    async fn count_tokens_returns_provider_estimate() {
        let (upstream, _hits, h) = fake_upstream().await;
        let client = count_client();
        let body = json!({ "model": "claude-x", "messages": [{"role":"user","content":"hello"}] });
        let n = count_tokens(&client, &upstream, &cred("good"), &body)
            .await
            .unwrap();
        assert!(n > 0);
        h.abort();
    }

    // Fake upstream that REQUIRES anthropic-beta: 400s with a JSON error body when
    // absent, echoes a token count when the expected beta value is present. Models
    // the OAuth/[1m] requirement the M1 sampler was blind to.
    async fn beta_required_upstream(expected: &'static str) -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new().fallback(any(move |request: Request<Body>| async move {
            let ok = request
                .headers()
                .get("anthropic-beta")
                .and_then(|v| v.to_str().ok())
                .map(|v| v == expected)
                .unwrap_or(false);
            if !ok {
                return Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"type":"error","error":{"type":"invalid_request_error",
                            "message":"anthropic-beta header required"}})
                        .to_string(),
                    ))
                    .unwrap();
            }
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/json")
                .body(Body::from(json!({ "input_tokens": 7 }).to_string()))
                .unwrap()
        }));
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn count_tokens_forwards_anthropic_beta_and_surfaces_error_type() {
        let (upstream, h) = beta_required_upstream("oauth-2025-04-20,context-1m-2025-08-07").await;
        let client = count_client();
        let body = json!({ "model": "claude-x", "messages": [{"role":"user","content":"x"}] });

        // With beta forwarded, the strict upstream accepts and returns its count.
        let mut with_beta = cred("good");
        with_beta.anthropic_beta = Some("oauth-2025-04-20,context-1m-2025-08-07".into());
        let n = count_tokens(&client, &upstream, &with_beta, &body)
            .await
            .expect("beta forwarded → success");
        assert_eq!(n, 7);

        // Without beta, the upstream 400s. The failure is diagnosable via the
        // bounded error.TYPE, but the error.MESSAGE (here "anthropic-beta header
        // required") is NEVER surfaced — repo invariant, ruling e376fec5.
        let err = count_tokens(&client, &upstream, &cred("good"), &body)
            .await
            .expect_err("missing beta → error");
        assert!(err.contains("HTTP 400"), "status must be surfaced: {err}");
        assert!(
            err.contains("invalid_request_error"),
            "error.type must be surfaced: {err}"
        );
        assert!(
            !err.contains("anthropic-beta header required"),
            "error.message must NOT be surfaced (repo invariant): {err}"
        );
        h.abort();
    }

    // Fake upstream returning the Anthropic error shape whose `message` carries a
    // distinctive marker. Proves the non-success arm keeps error.TYPE and drops
    // error.MESSAGE (invariant: response bodies never enter this repository).
    async fn error_message_upstream() -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new().fallback(any(|_req: Request<Body>| async move {
            Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"type":"error","error":{
                        "type":"invalid_request_error",
                        "message":"SECRET_PROMPT_LEAK_MARKER user said hunter2"}})
                    .to_string(),
                ))
                .unwrap()
        }));
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn count_tokens_error_keeps_type_drops_message() {
        let (upstream, h) = error_message_upstream().await;
        let client = count_client();
        let body = json!({ "model": "claude-x", "messages": [{"role":"user","content":"x"}] });
        let err = count_tokens(&client, &upstream, &cred("good"), &body)
            .await
            .expect_err("400 → error");
        assert!(err.contains("HTTP 400"), "status must be surfaced: {err}");
        assert!(
            err.contains("invalid_request_error"),
            "error.type must be surfaced: {err}"
        );
        assert!(
            !err.contains("SECRET_PROMPT_LEAK_MARKER"),
            "error.message must NOT leak into the persisted snippet: {err}"
        );
        assert!(
            !err.contains("hunter2"),
            "no message content may leak: {err}"
        );
        h.abort();
    }

    // A non-JSON / unparseable error body → status only, never raw bytes.
    #[tokio::test]
    async fn count_tokens_unparseable_error_is_status_only() {
        let app = Router::new().fallback(any(|_req: Request<Body>| async move {
            Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Body::from("RAW_HTML_LEAK <html>gateway down user-secret</html>"))
                .unwrap()
        }));
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let sh = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let up = format!("http://{addr}");
        let client = count_client();
        let body = json!({ "model": "claude-x", "messages": [{"role":"user","content":"x"}] });
        let err = count_tokens(&client, &up, &cred("good"), &body)
            .await
            .expect_err("502 → error");
        assert!(err.contains("HTTP 502"), "status must be surfaced: {err}");
        assert!(err.contains("(unparsed)"), "must mark unparsed: {err}");
        assert!(
            !err.contains("RAW_HTML_LEAK"),
            "raw body must NOT leak: {err}"
        );
        sh.abort();
    }

    // A valid Anthropic error envelope whose `type` is NOT an allowlisted enum
    // literal — a hostile/nonstandard upstream trying to smuggle content through
    // the type field. The value must never be persisted (challenge fda10918).
    async fn unknown_type_upstream() -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new().fallback(any(|_req: Request<Body>| async move {
            Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"type":"error","error":{
                        "type":"SECRET_TYPE_LEAK_MARKER","message":"unused"}})
                    .to_string(),
                ))
                .unwrap()
        }));
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn count_tokens_unknown_error_type_is_not_persisted() {
        let (upstream, h) = unknown_type_upstream().await;
        let client = count_client();
        let body = json!({ "model": "claude-x", "messages": [{"role":"user","content":"x"}] });
        let err = count_tokens(&client, &upstream, &cred("good"), &body)
            .await
            .expect_err("unlisted type → error");
        assert!(
            !err.contains("SECRET_TYPE_LEAK_MARKER"),
            "hostile error.type must NOT be persisted: {err}"
        );
        assert!(err.contains("HTTP 400"), "status must be surfaced: {err}");
        assert!(
            err.contains("(unknown type)"),
            "unlisted type must degrade to the status-only label: {err}"
        );
        // Prove it is status-ONLY: the type-appended form is "HTTP {status}: {t}",
        // so no ": " may appear.
        assert!(
            !err.contains(": "),
            "must not append any type value after the status: {err}"
        );
        h.abort();
    }

    #[tokio::test]
    async fn preflight_rejects_unauthorized_credential() {
        let (upstream, _hits, h) = fake_upstream().await;
        let client = count_client();
        assert!(preflight(&client, &upstream, &cred("good")).await.is_ok());
        assert!(preflight(&client, &upstream, &cred("bad")).await.is_err());
        h.abort();
    }

    // CONTAINMENT (a): a cross-host 3xx is NOT followed; the redirect target receives no credential.
    #[tokio::test]
    async fn count_client_does_not_follow_cross_host_redirect() {
        let (target, target_hits, th) = fake_upstream().await;
        // Redirector 302s to the target host.
        let target_for_redir = target.clone();
        let app = Router::new().fallback(any(move |_req: Request<Body>| {
            let loc = format!("{target_for_redir}/v1/messages/count_tokens");
            async move {
                Response::builder()
                    .status(StatusCode::FOUND)
                    .header("location", loc)
                    .body(Body::from(""))
                    .unwrap()
            }
        }));
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let rh = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let redirector = format!("http://{addr}");

        let client = count_client();
        let body = json!({ "model": "claude-x", "messages": [{"role":"user","content":"x"}] });
        // Policy::none() → the 302 is returned as-is (not success) → Err, and target is never hit.
        let result = count_tokens(&client, &redirector, &cred("good"), &body).await;
        assert!(
            result.is_err(),
            "3xx must be surfaced as an error, not followed"
        );
        assert_eq!(
            target_hits.load(Ordering::SeqCst),
            0,
            "credential must never reach the redirect target"
        );
        rh.abort();
        th.abort();
    }

    // CONTAINMENT (b): a slow upstream trips the client timeout without hanging.
    #[tokio::test]
    async fn count_client_times_out_on_slow_upstream() {
        let app = Router::new().fallback(any(|_req: Request<Body>| async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Response::builder()
                .status(StatusCode::OK)
                .body(Body::from("{}"))
                .unwrap()
        }));
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let sh = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let slow = format!("http://{addr}");
        // A short-timeout client stands in for the production 20s client; assert it returns fast.
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(300))
            .build()
            .unwrap();
        let body = json!({ "model": "claude-x", "messages": [{"role":"user","content":"x"}] });
        let started = tokio::time::Instant::now();
        let result = count_tokens(&client, &slow, &cred("good"), &body).await;
        assert!(result.is_err(), "timeout must surface as Err");
        assert!(started.elapsed() < Duration::from_secs(5), "must not hang");
        sh.abort();
    }

    // CONTAINMENT (c): missing auth → zero remote calls.
    #[tokio::test]
    async fn missing_auth_makes_no_remote_call() {
        let (upstream, hits, h) = fake_upstream().await;
        let client = count_client();
        let no_auth = CountCredential {
            api_key: None,
            authorization: None,
            anthropic_version: "2023-06-01".into(),
            anthropic_beta: None,
        };
        let body = json!({ "model": "claude-x", "messages": [{"role":"user","content":"x"}] });
        let result = count_tokens(&client, &upstream, &no_auth, &body).await;
        assert!(result.is_err(), "missing auth must fail");
        assert_eq!(
            hits.load(Ordering::SeqCst),
            0,
            "no remote call may be made without auth"
        );
        h.abort();
    }
}
