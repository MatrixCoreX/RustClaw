use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use claw_core::{
    config::{LlmProviderConfig, LlmProviderParams},
    model_turn::{ModelMessage, ModelRole, ModelToolChoice, ModelTurnRequest},
};
use serde_json::json;

use super::super::client::{
    call_model_turn_with_retry, call_provider_with_retry, ChatRequestHints,
};
use super::*;

fn runtime(name: &str, protocol: &str, base_url: &str) -> Arc<LlmProviderRuntime> {
    Arc::new(LlmProviderRuntime {
        config: LlmProviderConfig {
            name: name.into(),
            provider_type: protocol.into(),
            base_url: base_url.into(),
            api_key: "fixture-only".into(),
            model: "fixture-model".into(),
            context_window_tokens: None,
            input_modalities: vec!["text".into()],
            supports_tools: true,
            expected_latency_ms: None,
            priority: 1,
            timeout_seconds: 10,
            max_concurrency: 1,
            params: LlmProviderParams::default(),
        },
        pricing: None,
        latency: Arc::new(crate::providers::LlmProviderLatencyTracker::default()),
        client: reqwest::Client::builder().no_proxy().build().unwrap(),
        semaphore: Arc::new(tokio::sync::Semaphore::new(1)),
        breaker: Arc::new(crate::providers::CircuitBreaker::new()),
    })
}

fn request(stream: bool) -> ModelTurnRequest {
    ModelTurnRequest {
        messages: vec![ModelMessage::text(ModelRole::User, "protocol fixture")],
        tools: vec![],
        tool_choice: ModelToolChoice::Auto,
        response_schema: None,
        stream,
        metadata: BTreeMap::new(),
    }
}

struct FixtureServer {
    url: String,
    calls: Arc<AtomicUsize>,
    handle: tokio::task::JoinHandle<()>,
}
impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn server(status: StatusCode, headers: HeaderMap, body: String) -> FixtureServer {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let app = axum::Router::new().fallback(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        let headers = headers.clone();
        let body = body.clone();
        async move { (status, headers, body) }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    FixtureServer { url, calls, handle }
}

#[test]
fn retry_after_supports_seconds_dates_and_google_rpc_without_overflow() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-09-14T00:00:00Z")
        .unwrap()
        .to_utc();
    for (raw, expected) in [
        ("120", Some(120)),
        ("0.2", Some(1)),
        ("0", Some(0)),
        ("Mon, 14 Sep 2026 00:02:00 GMT", Some(120)),
        ("-1", None),
        ("NaN", None),
        ("inf", None),
        ("1e9", None),
        ("18446744073709551615", None),
    ] {
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", raw.parse().unwrap());
        assert_eq!(retry_after(&headers, &Value::Null, now), expected, "{raw}");
    }
    let mut headers = HeaderMap::new();
    headers.insert("retry-after", "2".parse().unwrap());
    let details = json!({"error":{"details":[
        {"@type":"type.googleapis.com/google.rpc.RetryInfo","retryDelay":"12.1s"},
        {"@type":"untrusted","retryDelay":"999999s"}
    ]}});
    assert_eq!(retry_after(&headers, &details, now), Some(13));
}

#[test]
fn normalized_error_retains_evidence_but_not_raw_body_in_message() {
    let provider = runtime(
        "vendor-minimax",
        "anthropic_claude",
        "https://example.invalid",
    );
    let body = json!({"error":{"type":"rate_limit_error","message":"opaque (2056)"}}).to_string();
    let error = response_error(
        &provider,
        StatusCode::TOO_MANY_REQUESTS,
        &HeaderMap::new(),
        &body,
        &json!({}),
    )
    .unwrap();
    assert_eq!(error.kind, ProviderErrorKind::QuotaExhausted);
    assert!(!error.retryable);
    assert_eq!(error.background_wait_seconds(), Some(10_800));
    assert!(!error.message.contains("opaque"));
    assert!(error.raw_response.unwrap().contains("2056"));
}

#[tokio::test]
async fn six_http_entrypoints_classify_quota_and_do_not_retry() {
    let body =
        json!({"type":"error","error":{"type":"rate_limit_error","message":"opaque (2056)"}})
            .to_string();
    let server = server(StatusCode::TOO_MANY_REQUESTS, HeaderMap::new(), body).await;
    for protocol in ["openai_compat", "anthropic_claude", "google_gemini"] {
        let provider = runtime("vendor-minimax", protocol, &server.url);
        let error = call_provider_with_retry(provider.clone(), "fixture")
            .await
            .unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::QuotaExhausted, "{protocol}");
        assert_eq!(error.attempts, 1);
        let error = call_model_turn_with_retry(
            provider,
            &request(false),
            &ChatRequestHints::default(),
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::QuotaExhausted, "{protocol}");
        assert_eq!(error.attempts, 1);
    }
    assert_eq!(server.calls.load(Ordering::SeqCst), 6);
}

#[tokio::test]
async fn http_200_business_failure_is_not_mistaken_for_empty_model_output() {
    let server = server(
        StatusCode::OK,
        HeaderMap::new(),
        json!({"base_resp":{"status_code":1008}}).to_string(),
    )
    .await;
    for protocol in ["openai_compat", "anthropic_claude", "google_gemini"] {
        let provider = runtime("vendor-minimax", protocol, &server.url);
        let error = call_provider_with_retry(provider.clone(), "fixture")
            .await
            .unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::QuotaExhausted);
        let error = call_model_turn_with_retry(
            provider,
            &request(false),
            &ChatRequestHints::default(),
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::QuotaExhausted);
    }
    assert_eq!(server.calls.load(Ordering::SeqCst), 6);
}

#[tokio::test]
async fn retry_after_yields_to_background_without_spending_more_requests() {
    let mut headers = HeaderMap::new();
    headers.insert("retry-after", "180".parse().unwrap());
    let server = server(StatusCode::TOO_MANY_REQUESTS, headers, "{}".into()).await;
    let provider = runtime("custom", "openai_compat", &server.url);
    let error = call_provider_with_retry(provider.clone(), "fixture")
        .await
        .unwrap_err();
    assert_eq!(error.background_wait_seconds(), Some(180));
    assert_eq!(error.attempts, 1);
    let error = call_model_turn_with_retry(
        provider,
        &request(false),
        &ChatRequestHints::default(),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(error.background_wait_seconds(), Some(180));
    assert_eq!(error.attempts, 1);
    assert_eq!(server.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn stream_error_after_text_is_not_accepted_as_success() {
    for trailing_newline in ["\n\n", ""] {
        let body = format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"partial\"}}}}]}}\n\n\
            data: {{\"error\":{{\"code\":402,\"message\":\"opaque\"}}}}{trailing_newline}"
        );
        let server = server(StatusCode::OK, HeaderMap::new(), body).await;
        let provider = runtime("openrouter", "openai_compat", &server.url);
        let error = call_model_turn_with_retry(
            provider,
            &request(true),
            &ChatRequestHints::default(),
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::QuotaExhausted);
        assert_eq!(error.attempts, 1);
        let raw = error.raw_response.unwrap();
        assert!(raw.contains("402"));
        assert!(raw.contains("partial"));
        assert_eq!(server.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn successful_http_and_streaming_protocols_still_work() {
    for (protocol, body) in [
        (
            "openai_compat",
            json!({"choices":[{"message":{"content":"fixture ok"},"finish_reason":"stop"}]}),
        ),
        (
            "anthropic_claude",
            json!({"type":"message","content":[{"type":"text","text":"fixture ok"}],"stop_reason":"end_turn"}),
        ),
        (
            "google_gemini",
            json!({"candidates":[{"content":{"parts":[{"text":"fixture ok"}]},"finishReason":"STOP"}]}),
        ),
    ] {
        let server = server(StatusCode::OK, HeaderMap::new(), body.to_string()).await;
        let provider = runtime("custom", protocol, &server.url);
        assert!(
            call_provider_with_retry(provider.clone(), "fixture")
                .await
                .is_ok(),
            "{protocol}"
        );
        assert!(
            call_model_turn_with_retry(
                provider,
                &request(false),
                &ChatRequestHints::default(),
                None
            )
            .await
            .is_ok(),
            "{protocol}"
        );
    }
    let server = server(StatusCode::OK, HeaderMap::new(), "data: {\"choices\":[{\"delta\":{\"content\":\"fixture ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".into()).await;
    assert!(call_model_turn_with_retry(
        runtime("custom", "openai_compat", &server.url),
        &request(true),
        &ChatRequestHints::default(),
        None
    )
    .await
    .is_ok());
}

#[tokio::test]
async fn incomplete_success_tail_is_not_promoted_to_a_complete_turn() {
    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n\
        data: {\"choices\":[{\"delta\":{\"content\":\"tail\"},\"finish_reason\":\"stop\"}]}";
    let server = server(StatusCode::OK, HeaderMap::new(), body.into()).await;
    let error = super::super::openai_model_turn::call_openai_model_turn(
        runtime("custom", "openai_compat", &server.url),
        &request(true),
        &ChatRequestHints::default(),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::ProviderRetryableResponse);
    assert_eq!(server.calls.load(Ordering::SeqCst), 1);
}
