use super::*;
use axum::{body::Body, http::Response, routing::post, Json, Router};
use claw_core::config::{LlmProviderConfig, LlmProviderParams};
use futures_util::{stream, StreamExt};
use std::{convert::Infallible, sync::Mutex, time::Duration};

struct Fixture {
    provider: Arc<LlmProviderRuntime>,
    requests: Arc<Mutex<Vec<Value>>>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn fixture(streaming: bool, status: u16, media_type: &str, body: String) -> Fixture {
    serve(streaming, status, media_type, body, false).await
}

async fn serve(
    streaming: bool,
    status: u16,
    media_type: &str,
    body: String,
    stall: bool,
) -> Fixture {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    let media_type = media_type.to_string();
    let app = Router::new().route(
        "/chat/completions",
        post(move |Json(request): Json<Value>| {
            seen.lock().unwrap().push(request);
            let chunks = body
                .as_bytes()
                .chunks(7)
                .map(|bytes| Ok::<_, Infallible>(axum::body::Bytes::copy_from_slice(bytes)))
                .collect::<Vec<_>>();
            let media_type = media_type.clone();
            async move {
                let tail = if stall {
                    stream::pending().boxed()
                } else {
                    stream::empty().boxed()
                };
                Response::builder()
                    .status(status)
                    .header("content-type", media_type)
                    .header("retry-after", "17")
                    .body(Body::from_stream(stream::iter(chunks).chain(tail)))
                    .unwrap()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let provider = Arc::new(LlmProviderRuntime {
        config: LlmProviderConfig {
            name: "fixture-provider".into(),
            provider_type: "openai_compat".into(),
            base_url,
            api_key: "fixture-local".into(),
            model: "fixture-model".into(),
            context_window_tokens: None,
            input_modalities: vec!["text".into()],
            supports_tools: true,
            expected_latency_ms: None,
            priority: 1,
            timeout_seconds: 10,
            max_concurrency: 1,
            params: LlmProviderParams {
                stream: Some(streaming),
                ..Default::default()
            },
        },
        pricing: None,
        latency: Arc::new(super::super::LlmProviderLatencyTracker::default()),
        client: reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
        semaphore: Arc::new(tokio::sync::Semaphore::new(1)),
        breaker: Arc::new(super::super::CircuitBreaker::new()),
    });
    Fixture {
        provider,
        requests,
        server,
    }
}

fn frame(value: Value) -> String {
    format!("data: {value}\n\n")
}

fn delta(text: &str) -> String {
    frame(json!({"choices":[{"delta":{"content":text}}]}))
}

fn terminal(reason: &str) -> String {
    frame(json!({"choices":[{"delta":{},"finish_reason":reason}]}))
}

fn usage_frame() -> String {
    frame(
        json!({"choices":[],"usage":{"prompt_tokens":11,"completion_tokens":7,"total_tokens":18}}),
    )
}

async fn call(fixture: &Fixture) -> Result<LlmProviderResponse, ProviderError> {
    call_openai_compat(
        fixture.provider.clone(),
        "fixture prompt",
        &ChatRequestHints::default(),
    )
    .await
}

#[tokio::test]
async fn configured_stream_reads_fragmented_text_and_terminal_usage() {
    let body =
        delta("hello ") + &delta("world") + &terminal("stop") + &usage_frame() + "data: [DONE]\n\n";
    let f = fixture(true, 200, "text/event-stream", body).await;
    let response = call(&f).await.expect("configured stream must decode SSE");
    assert_eq!(response.text, "hello world");
    assert_eq!(response.usage.unwrap().total_tokens, Some(18));
    assert_eq!(response.request_payload["stream"], true);
    assert!(response.request_payload.get("tools").is_none());
    assert_eq!(f.requests.lock().unwrap().len(), 1);
    assert_eq!(response.attempts, 1);
}

#[tokio::test]
async fn streamed_verifier_json_is_preserved_exactly() {
    let expected = "{\"pass\":true,\"missing_evidence_fields\":[]}";
    let body = delta(&expected[..12]) + &delta(&expected[12..]) + &terminal("stop");
    let f = fixture(true, 200, "text/event-stream", body).await;
    let response = call(&f).await.unwrap();
    assert_eq!(response.text, expected);
    assert_eq!(
        serde_json::from_str::<Value>(&response.text).unwrap()["pass"],
        true
    );
    assert!(response.usage.is_none());
}

#[tokio::test]
async fn streaming_ndjson_uses_the_same_terminal_contract() {
    let body = format!(
        "{}\n{}\n{}\n",
        json!({"choices":[{"delta":{"content":"public text"}}]}),
        json!({"choices":[{"delta":{},"finish_reason":"stop"}]}),
        json!({"choices":[],"usage":{"prompt_tokens":11,"completion_tokens":7,"total_tokens":18}})
    );
    let f = fixture(true, 200, "application/x-ndjson", body).await;
    let response = call(&f).await.unwrap();
    assert_eq!(response.text, "public text");
    assert_eq!(response.usage.unwrap().total_tokens, Some(18));
}

#[tokio::test]
async fn incomplete_stream_never_publishes_partial_success() {
    let f = fixture(true, 200, "text/event-stream", delta("partial")).await;
    let error = call(&f).await.unwrap_err();
    assert!(error.retryable);
    assert_eq!(error.message, "model_turn_stream_disconnected");
    assert_eq!(f.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn malformed_stream_frame_is_an_explicit_error() {
    let f = fixture(true, 200, "text/event-stream", "data: not-json\n\n".into()).await;
    let error = call(&f).await.unwrap_err();
    assert!(!error.retryable);
    assert!(error.message.starts_with("model_turn_stream_"));
}

#[tokio::test]
async fn missing_terminal_frame_cannot_be_replaced_by_unterminated_json() {
    let body = delta("partial") + &terminal("stop").trim_end();
    let f = fixture(true, 200, "text/event-stream", body).await;
    let error = call(&f).await.unwrap_err();
    assert!(error.retryable);
    assert_eq!(error.message, "model_turn_stream_disconnected");
}

#[tokio::test]
async fn stream_error_frame_preserves_rate_limit_and_retry_after() {
    let body = delta("partial")
        + &frame(json!({"error":{"type":"rate_limit_error","code":"rate_limit_exceeded"}}));
    let f = fixture(true, 200, "text/event-stream", body).await;
    let error = call(&f).await.unwrap_err();
    assert!(error.is_rate_limited());
    assert_eq!(error.retry_after_seconds, Some(17));
}

#[tokio::test]
async fn http_error_is_not_misparsed_as_a_success_stream() {
    let body =
        json!({"error":{"type":"rate_limit_error","code":"rate_limit_exceeded"}}).to_string();
    let f = fixture(true, 429, "application/json", body).await;
    let error = call(&f).await.unwrap_err();
    assert!(error.is_rate_limited());
    assert_eq!(error.retry_after_seconds, Some(17));
}

#[tokio::test]
async fn text_only_request_rejects_unsolicited_tool_calls_even_with_text() {
    let tool = frame(
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"fixture-call",
        "function":{"name":"fixture_tool","arguments":"{}"}}]}}]}),
    );
    for prefix in [String::new(), delta("not a plain text answer")] {
        let f = fixture(
            true,
            200,
            "text/event-stream",
            prefix + &tool + &terminal("tool_calls"),
        )
        .await;
        let error = call(&f).await.unwrap_err();
        assert_eq!(error.message, "openai_text_response_tool_calls_unexpected");
        assert!(!error.retryable);
        assert_eq!(f.requests.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn stream_logs_exclude_hidden_reasoning_fields() {
    let body =
        frame(json!({"choices":[{"delta":{"reasoning_content":"private-fixture-reasoning"}}]}))
            + &delta("public answer")
            + &terminal("stop");
    let f = fixture(true, 200, "text/event-stream", body).await;
    let response = call(&f).await.unwrap();
    assert_eq!(response.text, "public answer");
    assert!(!response.raw_response.contains("private-fixture-reasoning"));
    assert!(!response.raw_response.contains("reasoning_content"));
}

#[tokio::test]
async fn canceling_a_stalled_stream_releases_provider_concurrency() {
    let f = serve(true, 200, "text/event-stream", delta("partial"), true).await;
    let provider = f.provider.clone();
    let task = tokio::spawn(async move {
        call_openai_compat(provider, "fixture prompt", &ChatRequestHints::default()).await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while f.requests.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("request reached the loopback server");
    assert_eq!(f.provider.semaphore.available_permits(), 0);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(f.requests.lock().unwrap().len(), 1);
    assert_eq!(f.provider.semaphore.available_permits(), 1);
}

#[tokio::test]
async fn non_stream_response_and_request_hints_remain_unchanged() {
    let body = json!({"choices":[{"message":{"content":"whole response"},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":11,"completion_tokens":7,"total_tokens":18}})
    .to_string();
    let f = fixture(false, 200, "application/json", body).await;
    let hints = ChatRequestHints {
        temperature: Some(0.2),
        max_tokens: Some(123),
        reasoning_effort: Some("high".into()),
        ..Default::default()
    };
    let response = call_openai_compat(f.provider.clone(), "fixture prompt", &hints)
        .await
        .unwrap();
    assert_eq!(response.text, "whole response");
    assert_eq!(response.request_payload["stream"], false);
    assert_eq!(response.request_payload["temperature"], 0.2);
    assert_eq!(response.request_payload["max_tokens"], 123);
    assert_eq!(response.request_payload["reasoning_effort"], "high");
    assert_eq!(response.usage.unwrap().total_tokens, Some(18));
    assert_eq!(f.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn stream_length_termination_preserves_the_existing_text_contract() {
    let f = fixture(
        true,
        200,
        "text/event-stream",
        delta("truncated") + &terminal("length"),
    )
    .await;
    let response = call(&f).await.unwrap();
    assert_eq!(response.text, "truncated");
    assert!(response.usage.is_none());
}
