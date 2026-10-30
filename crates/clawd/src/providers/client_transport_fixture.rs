use std::sync::{Arc, Mutex};

use axum::{body::to_bytes, extract::Request, http::StatusCode};
use claw_core::config::{LlmProviderConfig, LlmProviderParams};
use serde_json::{json, Value};

use crate::LlmProviderRuntime;

pub(super) const FIXTURE_KEY: &str = "fixture-only-model-key";
pub(super) const FIXTURE_PROMPT: &str = "fixture-private-prompt";
pub(super) const PROFILES: [(&str, &str); 4] = [
    ("fixture-openai", "openai_compat"),
    ("fixture-anthropic", "anthropic_claude"),
    ("vendor-minimax", "anthropic_claude"),
    ("fixture-gemini", "google_gemini"),
];

#[derive(Debug)]
pub(super) struct Captured {
    pub method: String,
    pub uri: String,
    pub authorization: Option<String>,
    pub api_key: Option<String>,
    pub body: Value,
}

pub(super) struct Server {
    pub url: String,
    pub seen: Arc<Mutex<Vec<Captured>>>,
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

fn success(streaming: bool) -> String {
    if streaming {
        return concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"fixture-answer\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":2,\"total_tokens\":3}}\n\n",
            "data: [DONE]\n\n"
        ).into();
    }
    json!({
        "choices":[{"message":{"role":"assistant","content":"fixture-answer"},"finish_reason":"stop"}],
        "content":[{"type":"text","text":"fixture-answer"}], "stop_reason":"end_turn",
        "candidates":[{"content":{"role":"model","parts":[{"text":"fixture-answer"}]},"finishReason":"STOP"}]
    }).to_string()
}

pub(super) async fn server(redirect: Option<(StatusCode, String)>, streaming: bool) -> Server {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let capture = seen.clone();
    let app = axum::Router::new().fallback(move |request: Request| {
        let seen = capture.clone();
        let redirect = redirect.clone();
        async move {
            let (parts, body) = request.into_parts();
            let body = to_bytes(body, 64 * 1024).await.unwrap();
            let is_target = parts.uri.path() == "/redirect-target";
            let header = |name: &str| {
                parts
                    .headers
                    .get(name)
                    .map(|v| v.to_str().unwrap().to_string())
            };
            seen.lock().unwrap().push(Captured {
                method: parts.method.to_string(),
                uri: parts.uri.to_string(),
                authorization: header("authorization"),
                api_key: header("x-api-key"),
                body: serde_json::from_slice(&body).unwrap_or(Value::Null),
            });
            if !is_target {
                if let Some((status, target)) = redirect {
                    return (
                        status,
                        [("location", target)],
                        json!({"fixture":"redirect"}).to_string(),
                    );
                }
            }
            let content_type = if streaming {
                "text/event-stream"
            } else {
                "application/json"
            };
            (
                StatusCode::OK,
                [("content-type", content_type.into())],
                success(streaming),
            )
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Server { url, seen, handle }
}

pub(super) fn runtime(
    name: &str,
    protocol: &str,
    url: &str,
    stream: bool,
) -> Arc<LlmProviderRuntime> {
    let provider = Arc::new(LlmProviderRuntime {
        config: LlmProviderConfig {
            name: name.into(),
            provider_type: protocol.into(),
            base_url: url.into(),
            api_key: FIXTURE_KEY.into(),
            model: "fixture-model".into(),
            context_window_tokens: None,
            input_modalities: vec!["text".into()],
            supports_tools: true,
            expected_latency_ms: None,
            priority: 1,
            timeout_seconds: 3,
            max_concurrency: 1,
            params: LlmProviderParams {
                stream: Some(stream),
                ..LlmProviderParams::default()
            },
        },
        pricing: None,
        latency: Arc::new(crate::providers::LlmProviderLatencyTracker::default()),
        client: super::build_llm_http_client(3).unwrap(),
        semaphore: Arc::new(tokio::sync::Semaphore::new(1)),
        breaker: Arc::new(crate::providers::CircuitBreaker::new()),
    });
    assert!(
        provider.api_key() == FIXTURE_KEY,
        "fixture_credential_override"
    );
    provider
}
