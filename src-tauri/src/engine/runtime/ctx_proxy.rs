//! App-global loopback proxy for Anthropic API traffic.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, Request, Response, StatusCode};
use axum::routing::any;
use axum::Router;
use futures_util::StreamExt;

use crate::engine::state::AppState;

const DEFAULT_PROXY_PORT: u16 = 18_787;
const DEFAULT_UPSTREAM: &str = "https://api.anthropic.com";
const MODE_LOG: u8 = 1;

pub struct ProxyRuntime {
    pub port: u16,
    pub mode: AtomicU8,
    pub upstream: RwLock<String>,
    pub ledger: Mutex<ctxopt::ledger::Ledger>,
    pub active: AtomicBool,
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
            upstream: RwLock::new(DEFAULT_UPSTREAM.to_owned()),
            ledger: Mutex::new(ctxopt::ledger::Ledger::new(ctxopt::LEDGER_CAP)),
            active: AtomicBool::new(false),
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
    match forward_inner(&state, request).await {
        Ok(response) => response,
        Err(error) => Response::builder()
            .status(StatusCode::BAD_GATEWAY)
            .header("content-type", "text/plain; charset=utf-8")
            .body(Body::from(format!("upstream request failed: {error}")))
            .expect("static proxy error response is valid"),
    }
}

async fn forward_inner(state: &AppState, request: Request<Body>) -> Result<Response<Body>, String> {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, usize::MAX)
        .await
        .map_err(|error| error.to_string())?;
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

    let mut upstream_request = reqwest::Client::new()
        .request(parts.method, format!("{upstream}{path_and_query}"))
        .body(body);
    for (name, value) in filtered_headers(&parts.headers) {
        upstream_request = upstream_request.header(name, value);
    }

    let upstream_response = upstream_request
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let status = upstream_response.status();
    let response_headers = upstream_response.headers().clone();
    let response_stream = upstream_response
        .bytes_stream()
        .map(|chunk| chunk.map_err(std::io::Error::other));

    let mut response = Response::builder().status(status);
    for (name, value) in filtered_headers(&response_headers) {
        response = response.header(name, value);
    }
    response
        .body(Body::from_stream(response_stream))
        .map_err(|error| error.to_string())
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

    use axum::body::Bytes;
    use axum::http::Method;
    use axum::response::IntoResponse;
    use futures_util::stream;
    use serde_json::json;

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
}
