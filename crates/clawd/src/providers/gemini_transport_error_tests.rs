use std::{collections::BTreeMap, sync::Arc};

use claw_core::model_turn::{ModelMessage, ModelRole, ModelToolChoice, ModelTurnRequest};
use serde_json::Value;

use super::client::{ChatRequestHints, ProviderError, ProviderErrorKind};
use crate::LlmProviderRuntime;

#[path = "gemini_transport_error_test_support.rs"]
mod support;
use support::{assert_request, runtime, server, Mode, KEY, PROMPT};

async fn call(
    native: bool,
    provider: Arc<LlmProviderRuntime>,
) -> Result<(String, Value), ProviderError> {
    if native {
        let request = ModelTurnRequest {
            messages: vec![ModelMessage::text(ModelRole::User, PROMPT)],
            tools: vec![],
            tool_choice: ModelToolChoice::Auto,
            response_schema: None,
            stream: false,
            metadata: BTreeMap::new(),
        };
        super::gemini_model_turn::call_gemini_model_turn(
            provider,
            &request,
            &ChatRequestHints::default(),
            None,
        )
        .await
        .map(|response| (response.raw_response, response.request_payload))
    } else {
        super::google_gemini::call_google_gemini(provider, PROMPT, &ChatRequestHints::default())
            .await
            .map(|response| (response.raw_response, response.request_payload))
    }
}

fn assert_safe_failure(error: ProviderError, expected: ProviderErrorKind, stage: &str) {
    assert_eq!(error.kind, expected);
    assert!(error.retryable);
    assert!(error.should_trip_breaker());
    assert_eq!(error.attempts, 1);
    assert_eq!(error.retryable_error_count, 0);
    assert!(error.message.starts_with(stage));
    assert!(error.raw_response.is_none());
    assert!(error.usage.is_none());
    assert_eq!(
        error.request_payload["contents"][0]["parts"][0]["text"],
        PROMPT
    );
    assert!(
        !format!("{error:?}").contains(KEY),
        "fixture_query_key_in_error"
    );
    assert!(
        !error.message.contains("?key="),
        "fixture_query_url_in_error"
    );
    assert!(error.kind.terminal_notice_text().is_none());
}

#[tokio::test]
async fn gemini_connection_failure_keeps_query_key_out_of_diagnostics() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    for native in [false, true] {
        let provider = runtime(&url);
        let error = call(native, provider.clone()).await.unwrap_err();
        assert_eq!(provider.semaphore.available_permits(), 1);
        assert_safe_failure(
            error,
            ProviderErrorKind::TransportRetryable,
            "request failed:",
        );
    }
}

#[tokio::test]
async fn gemini_header_timeout_keeps_query_key_out_of_diagnostics() {
    for native in [false, true] {
        let server = server(Mode::HeaderTimeout).await;
        let provider = runtime(&server.url);
        let error = call(native, provider.clone()).await.unwrap_err();
        assert_request(&server);
        assert_eq!(provider.semaphore.available_permits(), 1);
        assert_safe_failure(error, ProviderErrorKind::Timeout, "timeout:");
    }
}

#[tokio::test]
async fn gemini_body_abort_keeps_query_key_out_of_diagnostics() {
    for native in [false, true] {
        let server = server(Mode::BodyAbort).await;
        let error = call(native, runtime(&server.url)).await.unwrap_err();
        assert_request(&server);
        assert_safe_failure(
            error,
            ProviderErrorKind::TransportRetryable,
            "read response failed:",
        );
    }
}

#[tokio::test]
async fn gemini_body_timeout_keeps_existing_retry_class_without_query_key() {
    for native in [false, true] {
        let server = server(Mode::BodyTimeout).await;
        let error = call(native, runtime(&server.url)).await.unwrap_err();
        assert_request(&server);
        assert_safe_failure(
            error,
            ProviderErrorKind::TransportRetryable,
            "read response failed:",
        );
    }
}

#[tokio::test]
async fn gemini_direct_success_keeps_query_auth_and_payload_unchanged() {
    for native in [false, true] {
        let server = server(Mode::Success).await;
        let (raw, body) = call(native, runtime(&server.url)).await.unwrap();
        assert_request(&server);
        assert!(raw.contains("fixture-answer"));
        assert!(!raw.contains(KEY));
        assert_eq!(body["contents"][0]["parts"][0]["text"], PROMPT);
    }
}

#[tokio::test]
async fn reqwest_fixture_control_proves_query_key_is_in_unsanitized_error_url() {
    let server = server(Mode::HeaderTimeout).await;
    let provider = runtime(&server.url);
    let error = provider
        .client
        .post(format!(
            "{}/models/fixture-model:generateContent?key={KEY}",
            server.url
        ))
        .json(&serde_json::json!({"contents":[{"parts":[{"text":PROMPT}]}]}))
        .send()
        .await
        .unwrap_err();
    assert!(error.is_timeout());
    assert!(error.url().is_some());
    assert!(error.to_string().contains(KEY));
    let safe = error.without_url();
    assert!(safe.is_timeout());
    assert!(safe.url().is_none());
    assert!(!safe.to_string().contains(KEY));
    assert_request(&server);
}
