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

/// Auth headers lifted from the forwarded /v1/messages request. Used in-flight
/// for count_tokens and dropped; never logged or persisted. Clone-only by
/// design — no Debug/Serialize/Deserialize so a credential can never be
/// accidentally formatted into a log or serialized into a metric row.
#[derive(Clone)]
pub struct CountCredential {
    pub api_key: Option<String>,       // x-api-key
    pub authorization: Option<String>, // Authorization: Bearer …
    pub anthropic_version: String,     // anthropic-version header
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
        return Err(format!("count_tokens HTTP {}", resp.status()));
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
