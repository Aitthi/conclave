//! Anthropic summary-generation client for H1 shadow economics.

use std::fmt;
use std::time::Duration;

use serde_json::Value;

use crate::engine::runtime::count_tokens::{
    apply_credential_headers, sanitize_error_type, CountCredential, SanitizedErrorType,
};

pub const SUMMARY_METHOD_VERSION: &str = "h1-shadow-summary-v1";
pub const SUMMARY_PROMPT_VERSION: &str = "aggregate-tool-results-v1";
pub const SUMMARY_MAX_OUTPUT_TOKENS: u32 = 8_192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SummaryUsage {
    pub input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedSummary {
    pub text: String,
    pub response_model: String,
    pub usage: SummaryUsage,
}

/// Content-free generation failure. Construction is private so callers can
/// persist only one of this module's fixed labels, never upstream text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SummaryClientError {
    kind: &'static str,
}

impl SummaryClientError {
    pub fn stage(&self) -> &'static str {
        "generation"
    }

    pub fn kind(&self) -> &'static str {
        self.kind
    }
}

impl fmt::Display for SummaryClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.kind)
    }
}

impl std::error::Error for SummaryClientError {}

fn failure(kind: &'static str) -> SummaryClientError {
    SummaryClientError { kind }
}

/// Dedicated generation client: no redirects, 60-second timeout, no retries.
pub fn summary_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(60))
        .build()
        .expect("summary client builder cannot fail")
}

fn required_usage(usage: &Value, key: &str) -> Result<u64, SummaryClientError> {
    usage
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| failure("missing_usage"))
}

pub async fn generate_summary(
    client: &reqwest::Client,
    upstream: &str,
    credential: &CountCredential,
    body: &Value,
) -> Result<GeneratedSummary, SummaryClientError> {
    if !credential.has_auth() {
        return Err(failure("missing_auth"));
    }
    let url = format!("{}/v1/messages", upstream.trim_end_matches('/'));
    let response = apply_credential_headers(
        client.post(url),
        credential,
        credential.anthropic_beta.as_deref(),
    )
    .body(body.to_string())
    .send()
    .await
    .map_err(|error| {
        if error.is_timeout() {
            failure("timeout")
        } else {
            failure("transport")
        }
    })?;

    if response.status().is_redirection() {
        return Err(failure("redirect"));
    }
    if !response.status().is_success() {
        let error_body = response.text().await.unwrap_or_default();
        return Err(match sanitize_error_type(&error_body) {
            SanitizedErrorType::Known(error_type) => failure(error_type),
            SanitizedErrorType::Unknown => failure("http_unknown_type"),
            SanitizedErrorType::Unparsed => failure("http_unparsed"),
        });
    }

    let response: Value = response.json().await.map_err(|_| failure("decode"))?;
    let response_model = response
        .get("model")
        .and_then(Value::as_str)
        .filter(|model| !model.is_empty())
        .ok_or_else(|| failure("missing_model"))?
        .to_owned();
    let blocks = response
        .get("content")
        .and_then(Value::as_array)
        .filter(|blocks| !blocks.is_empty())
        .ok_or_else(|| failure("missing_content"))?;
    let mut text = Vec::with_capacity(blocks.len());
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            // claude-opus-5 emits a leading `thinking` block even when the
            // request carries NO `thinking` field — adaptive thinking is the
            // model default (verified live 2026-08-16; proxy_summary_metric
            // row 5 died here). Skip the thinking family and keep the text.
            // Turning thinking off in the request is NOT the fix: sending
            // `thinking:{"type":"disabled"}` misses the message-block prompt
            // cache for the whole generation prefix (~200k tokens re-written
            // at cache-write price), while omitting the field is
            // cache-identical to the original request.
            Some("thinking") | Some("redacted_thinking") => continue,
            Some("text") => {
                let block_text = block
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|text| !text.trim().is_empty())
                    .ok_or_else(|| failure("empty_text"))?;
                text.push(block_text);
            }
            // Any OTHER block type is still a hard failure.
            _ => return Err(failure("non_text_content")),
        }
    }
    // A response made up entirely of thinking blocks carries no summary.
    if text.is_empty() {
        return Err(failure("empty_text"));
    }
    let usage = response
        .get("usage")
        .ok_or_else(|| failure("missing_usage"))?;
    Ok(GeneratedSummary {
        text: text.join("\n"),
        response_model,
        usage: SummaryUsage {
            input_tokens: required_usage(usage, "input_tokens")?,
            cache_creation_input_tokens: required_usage(usage, "cache_creation_input_tokens")?,
            cache_read_input_tokens: required_usage(usage, "cache_read_input_tokens")?,
            output_tokens: required_usage(usage, "output_tokens")?,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, Response, StatusCode};
    use axum::routing::post;
    use axum::Router;
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    async fn happy_upstream() -> (
        String,
        Arc<AtomicUsize>,
        Arc<Mutex<Option<(axum::http::HeaderMap, Value)>>>,
        tokio::task::JoinHandle<()>,
    ) {
        let hits = Arc::new(AtomicUsize::new(0));
        let captured = Arc::new(Mutex::new(None));
        let hits2 = hits.clone();
        let captured2 = captured.clone();
        let app = Router::new().route(
            "/v1/messages",
            post(move |request: Request<Body>| {
                let hits = hits2.clone();
                let captured = captured2.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    let headers = request.headers().clone();
                    let bytes = to_bytes(request.into_body(), usize::MAX).await.unwrap();
                    let body = serde_json::from_slice(&bytes).unwrap();
                    *captured.lock().unwrap() = Some((headers, body));
                    Response::builder()
                        .status(StatusCode::OK)
                        .header("content-type", "application/json")
                        .body(Body::from(
                            json!({
                                "model":"claude-test",
                                "content":[
                                    {"type":"text","text":"first"},
                                    {"type":"text","text":"second"}
                                ],
                                "usage":{
                                    "input_tokens":11,
                                    "cache_creation_input_tokens":22,
                                    "cache_read_input_tokens":33,
                                    "output_tokens":44
                                }
                            })
                            .to_string(),
                        ))
                        .unwrap()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), hits, captured, handle)
    }

    fn api_key_credential() -> crate::engine::runtime::count_tokens::CountCredential {
        crate::engine::runtime::count_tokens::CountCredential {
            api_key: Some("secret-key".into()),
            authorization: None,
            anthropic_version: "2023-06-01".into(),
            anthropic_beta: Some("beta-a,beta-b".into()),
        }
    }

    fn bearer_credential() -> crate::engine::runtime::count_tokens::CountCredential {
        crate::engine::runtime::count_tokens::CountCredential {
            api_key: None,
            authorization: Some("Bearer oauth-secret".into()),
            anthropic_version: "2023-06-01".into(),
            anthropic_beta: Some("oauth-2025-04-20,context-1m-2025-08-07".into()),
        }
    }

    async fn fixed_upstream(
        status: StatusCode,
        response_body: String,
        delay: Duration,
    ) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        let hits = Arc::new(AtomicUsize::new(0));
        let hits2 = hits.clone();
        let app = Router::new().route(
            "/v1/messages",
            post(move || {
                let hits = hits2.clone();
                let body = response_body.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    if !delay.is_zero() {
                        tokio::time::sleep(delay).await;
                    }
                    Response::builder()
                        .status(status)
                        .header("content-type", "application/json")
                        .body(Body::from(body))
                        .unwrap()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), hits, handle)
    }

    fn success_body(content: Value, usage: Option<Value>) -> String {
        let mut response = json!({"model":"claude-response","content":content});
        if let Some(usage) = usage {
            response["usage"] = usage;
        }
        response.to_string()
    }

    fn complete_usage() -> Value {
        json!({
            "input_tokens":1,
            "cache_creation_input_tokens":2,
            "cache_read_input_tokens":3,
            "output_tokens":4
        })
    }

    #[tokio::test]
    async fn api_key_call_preserves_beta_and_parses_text_model_and_four_usage_buckets() {
        let (upstream, hits, captured, _server) = happy_upstream().await;
        let body = json!({"model":"claude-test","messages":[]});

        let generated =
            generate_summary(&summary_client(), &upstream, &api_key_credential(), &body)
                .await
                .unwrap();

        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(generated.text, "first\nsecond");
        assert_eq!(generated.response_model, "claude-test");
        assert_eq!(generated.usage.input_tokens, 11);
        assert_eq!(generated.usage.cache_creation_input_tokens, 22);
        assert_eq!(generated.usage.cache_read_input_tokens, 33);
        assert_eq!(generated.usage.output_tokens, 44);
        let (headers, sent_body) = captured.lock().unwrap().take().unwrap();
        assert_eq!(headers["x-api-key"], "secret-key");
        assert_eq!(headers["anthropic-version"], "2023-06-01");
        assert_eq!(headers["anthropic-beta"], "beta-a,beta-b");
        assert!(!headers["anthropic-beta"]
            .to_str()
            .unwrap()
            .contains("token-counting"));
        assert_eq!(sent_body, body);
    }

    #[tokio::test]
    async fn bearer_call_preserves_authorization_and_beta_order_without_counting_beta() {
        let (upstream, hits, captured, _server) = happy_upstream().await;
        generate_summary(
            &summary_client(),
            &upstream,
            &bearer_credential(),
            &json!({"model":"claude-test","messages":[]}),
        )
        .await
        .unwrap();

        assert_eq!(hits.load(Ordering::SeqCst), 1);
        let (headers, _) = captured.lock().unwrap().take().unwrap();
        assert_eq!(headers["authorization"], "Bearer oauth-secret");
        assert_eq!(
            headers["anthropic-beta"],
            "oauth-2025-04-20,context-1m-2025-08-07"
        );
    }

    #[tokio::test]
    async fn missing_auth_returns_fixed_label_without_network_call() {
        let (upstream, hits, _captured, _server) = happy_upstream().await;
        let credential = crate::engine::runtime::count_tokens::CountCredential {
            api_key: None,
            authorization: None,
            anthropic_version: "2023-06-01".into(),
            anthropic_beta: None,
        };
        let error = generate_summary(&summary_client(), &upstream, &credential, &json!({}))
            .await
            .unwrap_err();
        assert_eq!(error.stage(), "generation");
        assert_eq!(error.kind(), "missing_auth");
        assert_eq!(error.to_string(), "missing_auth");
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn redirect_is_refused_and_credentials_never_reach_target() {
        let target_hits = Arc::new(AtomicUsize::new(0));
        let target_hits2 = target_hits.clone();
        let target_app = Router::new().fallback(axum::routing::any(move || {
            let hits = target_hits2.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                StatusCode::OK
            }
        }));
        let target_listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let target_addr = target_listener.local_addr().unwrap();
        let _target =
            tokio::spawn(async move { axum::serve(target_listener, target_app).await.unwrap() });

        let redirect_hits = Arc::new(AtomicUsize::new(0));
        let redirect_hits2 = redirect_hits.clone();
        let location = format!("http://{target_addr}/stolen");
        let redirect_app = Router::new().route(
            "/v1/messages",
            post(move || {
                let hits = redirect_hits2.clone();
                let location = location.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    Response::builder()
                        .status(StatusCode::FOUND)
                        .header("location", location)
                        .body(Body::empty())
                        .unwrap()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let _redirect =
            tokio::spawn(async move { axum::serve(listener, redirect_app).await.unwrap() });

        let error = generate_summary(
            &summary_client(),
            &format!("http://{addr}"),
            &api_key_credential(),
            &json!({}),
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind(), "redirect");
        assert_eq!(redirect_hits.load(Ordering::SeqCst), 1);
        assert_eq!(target_hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn timeout_is_fixed_label_and_request_is_not_retried() {
        let (upstream, hits, _server) = fixed_upstream(
            StatusCode::OK,
            success_body(
                json!([{"type":"text","text":"late"}]),
                Some(complete_usage()),
            ),
            Duration::from_millis(200),
        )
        .await;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(50))
            .build()
            .unwrap();
        let error = generate_summary(&client, &upstream, &api_key_credential(), &json!({}))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), "timeout");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn http_error_allows_only_known_type_and_never_hostile_message() {
        let marker = "PRIVATE_TOOL_OUTPUT_MARKER";
        let body = json!({
            "error":{"type":"invalid_request_error","message":marker},
            "extra":marker
        })
        .to_string();
        let (upstream, hits, _server) =
            fixed_upstream(StatusCode::BAD_REQUEST, body, Duration::ZERO).await;
        let error = generate_summary(
            &summary_client(),
            &upstream,
            &api_key_credential(),
            &json!({}),
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind(), "invalid_request_error");
        assert!(!error.to_string().contains(marker));
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn unknown_and_unparsed_http_bodies_degrade_to_fixed_labels() {
        for (body, expected) in [
            (
                json!({"error":{"type":"PRIVATE_TOOL_OUTPUT_MARKER"}}).to_string(),
                "http_unknown_type",
            ),
            ("PRIVATE_TOOL_OUTPUT_MARKER".into(), "http_unparsed"),
        ] {
            let (upstream, hits, _server) =
                fixed_upstream(StatusCode::BAD_GATEWAY, body, Duration::ZERO).await;
            let error = generate_summary(
                &summary_client(),
                &upstream,
                &api_key_credential(),
                &json!({}),
            )
            .await
            .unwrap_err();
            assert_eq!(error.kind(), expected);
            assert!(!error.to_string().contains("PRIVATE_TOOL_OUTPUT_MARKER"));
            assert_eq!(hits.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn tool_use_and_other_non_text_blocks_are_rejected_without_retry() {
        for content in [
            json!([{"type":"tool_use","id":"tu","name":"Read","input":{}}]),
            json!([{"type":"image","source":{"type":"base64","data":"AAAA"}}]),
        ] {
            let (upstream, hits, _server) = fixed_upstream(
                StatusCode::OK,
                success_body(content, Some(complete_usage())),
                Duration::ZERO,
            )
            .await;
            let error = generate_summary(
                &summary_client(),
                &upstream,
                &api_key_credential(),
                &json!({}),
            )
            .await
            .unwrap_err();
            assert_eq!(error.kind(), "non_text_content");
            assert_eq!(hits.load(Ordering::SeqCst), 1);
        }
    }

    /// claude-opus-5 answers the generation request with `[thinking, text]`
    /// even though `gen_body` sets no `thinking` field — adaptive thinking is
    /// the model default (live-verified 2026-08-16, the defect behind
    /// `proxy_summary_metric` row 5).
    #[tokio::test]
    async fn thinking_family_blocks_are_skipped_and_the_text_survives() {
        let cases = [
            (
                json!([
                    {"type":"thinking","thinking":"PRIVATE_REASONING","signature":"sig"},
                    {"type":"text","text":"first"}
                ]),
                "first",
            ),
            (
                json!([
                    {"type":"redacted_thinking","data":"PRIVATE_REASONING"},
                    {"type":"text","text":"first"},
                    {"type":"thinking","thinking":"PRIVATE_REASONING","signature":"sig"},
                    {"type":"text","text":"second"}
                ]),
                "first\nsecond",
            ),
        ];
        for (content, expected) in cases {
            let (upstream, hits, _server) = fixed_upstream(
                StatusCode::OK,
                success_body(content, Some(complete_usage())),
                Duration::ZERO,
            )
            .await;
            let generated = generate_summary(
                &summary_client(),
                &upstream,
                &api_key_credential(),
                &json!({}),
            )
            .await
            .unwrap();
            assert_eq!(generated.text, expected);
            assert!(
                !generated.text.contains("PRIVATE_REASONING"),
                "thinking payloads must never reach the summary text"
            );
            assert_eq!(hits.load(Ordering::SeqCst), 1);
        }
    }

    /// Tolerating the thinking family must not tolerate anything else, and a
    /// response with no text block at all still has no summary in it.
    #[tokio::test]
    async fn thinking_tolerance_does_not_leak_to_other_types_or_textless_responses() {
        let cases = [
            (
                json!([
                    {"type":"thinking","thinking":"t","signature":"sig"},
                    {"type":"tool_use","id":"tu","name":"Read","input":{}},
                    {"type":"text","text":"first"}
                ]),
                "non_text_content",
            ),
            (
                json!([
                    {"type":"thinking","thinking":"t","signature":"sig"},
                    {"type":"redacted_thinking","data":"d"}
                ]),
                "empty_text",
            ),
            (
                json!([
                    {"type":"thinking","thinking":"t","signature":"sig"},
                    {"type":"text","text":"  \n "}
                ]),
                "empty_text",
            ),
        ];
        for (content, expected) in cases {
            let (upstream, hits, _server) = fixed_upstream(
                StatusCode::OK,
                success_body(content, Some(complete_usage())),
                Duration::ZERO,
            )
            .await;
            let error = generate_summary(
                &summary_client(),
                &upstream,
                &api_key_credential(),
                &json!({}),
            )
            .await
            .unwrap_err();
            assert_eq!(error.kind(), expected);
            assert_eq!(hits.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn empty_text_missing_usage_and_decode_fail_with_fixed_labels() {
        let cases = [
            (
                success_body(json!([{"type":"text","text":""}]), Some(complete_usage())),
                "empty_text",
            ),
            (
                success_body(
                    json!([{"type":"text","text":"  \n "}]),
                    Some(complete_usage()),
                ),
                "empty_text",
            ),
            (
                success_body(json!([{"type":"text","text":"ok"}]), None),
                "missing_usage",
            ),
            ("not-json".into(), "decode"),
        ];
        for (body, expected) in cases {
            let (upstream, hits, _server) =
                fixed_upstream(StatusCode::OK, body, Duration::ZERO).await;
            let error = generate_summary(
                &summary_client(),
                &upstream,
                &api_key_credential(),
                &json!({}),
            )
            .await
            .unwrap_err();
            assert_eq!(error.kind(), expected);
            assert_eq!(hits.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn missing_model_content_or_any_usage_bucket_is_rejected_once() {
        let cases = [
            (
                json!({"content":[{"type":"text","text":"ok"}],"usage":complete_usage()})
                    .to_string(),
                "missing_model",
            ),
            (
                json!({"model":"m","usage":complete_usage()}).to_string(),
                "missing_content",
            ),
            (
                json!({
                    "model":"m",
                    "content":[{"type":"text","text":"ok"}],
                    "usage":{
                        "input_tokens":1,
                        "cache_creation_input_tokens":2,
                        "cache_read_input_tokens":3
                    }
                })
                .to_string(),
                "missing_usage",
            ),
        ];
        for (body, expected) in cases {
            let (upstream, hits, _server) =
                fixed_upstream(StatusCode::OK, body, Duration::ZERO).await;
            let error = generate_summary(
                &summary_client(),
                &upstream,
                &api_key_credential(),
                &json!({}),
            )
            .await
            .unwrap_err();
            assert_eq!(error.kind(), expected);
            assert_eq!(hits.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn versions_and_output_cap_are_pinned() {
        assert_eq!(SUMMARY_METHOD_VERSION, "h1-shadow-summary-v1");
        assert_eq!(SUMMARY_PROMPT_VERSION, "aggregate-tool-results-v1");
        assert_eq!(SUMMARY_MAX_OUTPUT_TOKENS, 8_192);
    }
}
