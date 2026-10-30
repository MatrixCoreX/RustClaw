use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    body::{to_bytes, Body, Bytes},
    extract::Request,
    response::Response,
    Router,
};
use claw_core::config::{LlmProviderConfig, LlmProviderParams};
use futures_util::{stream, StreamExt};
use serde_json::{json, Value};

use crate::LlmProviderRuntime;

pub(super) const KEY: &str = "fixture-only-gemini-query-key";
pub(super) const PROMPT: &str = "fixture-prompt";

#[derive(Clone, Copy)]
pub(super) enum Mode {
    Success,
    HeaderTimeout,
    BodyAbort,
    BodyTimeout,
}

pub(super) struct Server {
    pub url: String,
    pub requests: Arc<Mutex<Vec<(String, Value)>>>,
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

pub(super) async fn server(mode: Mode) -> Server {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let capture = requests.clone();
    let app = Router::new().fallback(move |request: Request| {
        let capture = capture.clone();
        async move {
            let (parts, body) = request.into_parts();
            let raw = to_bytes(body, 64 * 1024).await.unwrap();
            capture.lock().unwrap().push((parts.uri.to_string(), serde_json::from_slice(&raw).unwrap()));
            if matches!(mode, Mode::HeaderTimeout) {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            if matches!(mode, Mode::BodyAbort | Mode::BodyTimeout) {
                let first = stream::once(async { Ok::<_, io::Error>(Bytes::from_static(b"{")) });
                let last = stream::once(async move {
                    if matches!(mode, Mode::BodyAbort) {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        Err(io::Error::new(io::ErrorKind::UnexpectedEof, "fixture_body_abort"))
                    } else {
                        std::future::pending::<Result<Bytes, io::Error>>().await
                    }
                });
                return Response::builder().header("content-type", "application/json")
                    .body(Body::from_stream(first.chain(last))).unwrap();
            }
            Response::builder().header("content-type", "application/json").body(Body::from(json!({
                "candidates":[{"content":{"role":"model","parts":[{"text":"fixture-answer"}]},"finishReason":"STOP"}]
            }).to_string())).unwrap()
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Server {
        url,
        requests,
        handle,
    }
}

pub(super) fn runtime(url: &str) -> Arc<LlmProviderRuntime> {
    let provider = Arc::new(LlmProviderRuntime {
        config: LlmProviderConfig {
            name: "fixture-gemini".into(),
            provider_type: "google_gemini".into(),
            base_url: url.into(),
            api_key: KEY.into(),
            model: "fixture-model".into(),
            context_window_tokens: None,
            input_modalities: vec!["text".into()],
            supports_tools: true,
            expected_latency_ms: None,
            priority: 1,
            timeout_seconds: 1,
            max_concurrency: 1,
            params: LlmProviderParams::default(),
        },
        pricing: None,
        latency: Arc::new(crate::providers::LlmProviderLatencyTracker::default()),
        client: reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(500))
            .build()
            .unwrap(),
        semaphore: Arc::new(tokio::sync::Semaphore::new(1)),
        breaker: Arc::new(crate::providers::CircuitBreaker::new()),
    });
    assert!(provider.api_key() == KEY, "fixture_credential_override");
    provider
}

pub(super) fn assert_request(server: &Server) {
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let (uri, body) = &requests[0];
    let url = reqwest::Url::parse(&format!("http://fixture.invalid{uri}")).unwrap();
    assert_eq!(url.path(), "/models/fixture-model:generateContent");
    assert_eq!(
        url.query_pairs()
            .collect::<BTreeMap<_, _>>()
            .get("key")
            .map(|v| v.as_ref()),
        Some(KEY)
    );
    assert_eq!(body["contents"][0]["parts"][0]["text"], PROMPT);
}
