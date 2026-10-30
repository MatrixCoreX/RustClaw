use axum::http::StatusCode;
use claw_core::model_turn::ModelTurnRequest;
use serde_json::json;
use std::sync::Arc;

use super::{
    build_llm_http_client, call_model_turn_with_retry, call_provider_with_retry, ChatRequestHints,
    ProviderError, ProviderErrorKind,
};
use crate::LlmProviderRuntime;

#[path = "client_transport_fixture.rs"]
mod fixture;
use fixture::{runtime, server, FIXTURE_KEY, FIXTURE_PROMPT, PROFILES};

async fn call(provider: Arc<LlmProviderRuntime>, native: bool) -> Result<String, ProviderError> {
    if native {
        let mut request = ModelTurnRequest::text(FIXTURE_PROMPT);
        request.stream = provider.config.params.stream.unwrap_or(false);
        call_model_turn_with_retry(provider, &request, &ChatRequestHints::default(), None)
            .await
            .map(|r| r.turn.text)
    } else {
        call_provider_with_retry(provider, FIXTURE_PROMPT)
            .await
            .map(|r| r.text)
    }
}

#[tokio::test]
async fn provider_http_redirects_do_not_forward_credentials_or_payloads() {
    for status in [301, 302, 303, 307, 308] {
        for cross_origin in [false, true] {
            for path in ["device-key/request", "device-key/verify"] {
                let target = server(None, false).await;
                let location = if cross_origin {
                    format!("{}/redirect-target", target.url)
                } else {
                    "/redirect-target".into()
                };
                let origin = server(
                    Some((StatusCode::from_u16(status).unwrap(), location)),
                    false,
                )
                .await;
                let payload = json!({"prompt":FIXTURE_PROMPT,"signature":"fixture-signature"});
                let response = build_llm_http_client(3)
                    .unwrap()
                    .post(format!("{}/{path}", origin.url))
                    .bearer_auth(FIXTURE_KEY)
                    .header("x-api-key", FIXTURE_KEY)
                    .json(&payload)
                    .send()
                    .await
                    .unwrap();
                let target_requests = target.seen.lock().unwrap();
                assert!(
                    target_requests.is_empty(),
                    "redirect target received sensitive request: {target_requests:?}"
                );
                let source_requests = origin.seen.lock().unwrap();
                assert_eq!(
                    source_requests.len(),
                    1,
                    "same-origin redirect was followed"
                );
                assert_eq!(source_requests[0].body, payload);
                assert_eq!(source_requests[0].api_key.as_deref(), Some(FIXTURE_KEY));
                assert_eq!(response.status().as_u16(), status);
            }
        }
    }
}

#[tokio::test]
async fn all_model_entrypoints_reject_redirects_without_retries_or_quota_notice() {
    for status in [301, 302, 303, 307, 308] {
        for cross_origin in [false, true] {
            for (name, protocol) in PROFILES {
                for native in [false, true] {
                    for stream in [false, true] {
                        if stream && protocol != "openai_compat" {
                            continue;
                        }
                        let target = server(None, false).await;
                        let location = if cross_origin {
                            format!("{}/redirect-target", target.url)
                        } else {
                            "/redirect-target".into()
                        };
                        let origin = server(
                            Some((StatusCode::from_u16(status).unwrap(), location)),
                            false,
                        )
                        .await;
                        let result =
                            call(runtime(name, protocol, &origin.url, stream), native).await;
                        assert!(target.seen.lock().unwrap().is_empty(), "redirect target received model data: {protocol}/{native}/{stream}/{status}");
                        assert_eq!(
                            origin.seen.lock().unwrap().len(),
                            1,
                            "redirect/retry: {protocol}/{native}/{stream}/{status}"
                        );
                        let error = result.unwrap_err();
                        assert_eq!(error.kind, ProviderErrorKind::ProviderNonRetryableBusiness);
                        assert_eq!(error.attempts, 1);
                        assert!(!error.retryable);
                        assert!(error.kind.terminal_notice_text().is_none());
                        assert!(error.request_payload.to_string().contains(FIXTURE_PROMPT));
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn direct_model_entrypoints_keep_protocol_credentials_and_payloads() {
    for (name, protocol) in PROFILES {
        for native in [false, true] {
            let origin = server(None, false).await;
            let answer = call(runtime(name, protocol, &origin.url, false), native)
                .await
                .unwrap();
            assert_eq!(answer, "fixture-answer");
            let seen = origin.seen.lock().unwrap();
            assert_eq!(seen.len(), 1);
            let request = &seen[0];
            assert_eq!(request.method, "POST");
            assert!(request.body.to_string().contains(FIXTURE_PROMPT));
            match (protocol, name) {
                ("anthropic_claude", "fixture-anthropic") => {
                    assert_eq!(request.api_key.as_deref(), Some(FIXTURE_KEY))
                }
                ("google_gemini", _) => {
                    assert!(request.uri.contains(&format!("key={FIXTURE_KEY}")))
                }
                _ => assert_eq!(
                    request.authorization.as_deref(),
                    Some(format!("Bearer {FIXTURE_KEY}").as_str())
                ),
            }
        }
    }
}

#[tokio::test]
async fn direct_streaming_keeps_text_native_output_and_usage() {
    for native in [false, true] {
        let origin = server(None, true).await;
        let provider = runtime("fixture-openai", "openai_compat", &origin.url, true);
        let (text, usage) = if native {
            let mut request = ModelTurnRequest::text(FIXTURE_PROMPT);
            request.stream = true;
            let response =
                call_model_turn_with_retry(provider, &request, &ChatRequestHints::default(), None)
                    .await
                    .unwrap();
            (response.turn.text, response.turn.usage)
        } else {
            let response = call_provider_with_retry(provider, FIXTURE_PROMPT)
                .await
                .unwrap();
            (response.text, response.usage)
        };
        assert_eq!(text, "fixture-answer");
        assert_eq!(usage.unwrap().total_tokens, Some(3));
        let requests = origin.seen.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].body["stream"], true);
    }
}

#[tokio::test]
async fn unrestricted_control_really_forwards_fixture_key_and_body() {
    for cross_origin in [false, true] {
        let target = server(None, false).await;
        let location = if cross_origin {
            format!("{}/redirect-target", target.url)
        } else {
            "/redirect-target".into()
        };
        let origin = server(Some((StatusCode::TEMPORARY_REDIRECT, location)), false).await;
        let payload = json!({"prompt":FIXTURE_PROMPT,"signature":"fixture-signature"});
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!("{}/device-key/verify", origin.url))
            .header("x-api-key", FIXTURE_KEY)
            .json(&payload)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let seen = if cross_origin {
            target.seen.lock().unwrap()
        } else {
            origin.seen.lock().unwrap()
        };
        assert_eq!(seen.len(), if cross_origin { 1 } else { 2 });
        let forwarded = seen.last().unwrap();
        assert_eq!(forwarded.method, "POST");
        assert_eq!(forwarded.api_key.as_deref(), Some(FIXTURE_KEY));
        assert_eq!(forwarded.body, payload);
        println!(
            "unrestricted_fixture_control cross_origin={cross_origin} forwarded={forwarded:?}"
        );
    }
}
