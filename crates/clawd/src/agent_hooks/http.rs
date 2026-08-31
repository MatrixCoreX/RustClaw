use reqwest::{redirect::Policy, StatusCode, Url};
use serde_json::Value;
use std::net::IpAddr;
use std::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::public_http_client::{build_public_http_client, is_public_ip};

use super::shared::{
    elapsed_ms, handler_failure, is_env_reference, parse_handler_output, validate_common_handler,
    ExecutedHook, HandlerRunResult, HookHandlerConfig, ValidatedHookHandler,
};

#[derive(Debug)]
pub(super) struct ValidatedHttpHandler {
    common: ValidatedHookHandler,
    url: Url,
    auth_token: Option<String>,
    endpoint_policy: HttpEndpointPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HttpEndpointPolicy {
    PublicHttps,
    ExplicitLoopback,
}

#[derive(Debug)]
struct HttpAttemptError {
    code: &'static str,
    retryable: bool,
    output_truncated: bool,
}

pub(super) async fn run_http_handler(
    handler: HookHandlerConfig,
    event: &Value,
    cancellation: CancellationToken,
) -> Result<ExecutedHook, (String, &'static str)> {
    let handler = validate_http_handler(handler)?;
    let result = execute_http_handler(&handler, event, cancellation).await;
    Ok(ExecutedHook {
        handler: handler.common,
        handler_kind: "http",
        trust_status: "trusted",
        content_sha256: None,
        result,
    })
}

pub(super) fn validate_http_handler(
    handler: HookHandlerConfig,
) -> Result<ValidatedHttpHandler, (String, &'static str)> {
    let common = validate_common_handler(&handler, "http", 3)?;
    let url =
        Url::parse(handler.url.trim()).map_err(|_| (common.id.clone(), "hook_http_url_invalid"))?;
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err((common.id, "hook_http_url_credentials_forbidden"));
    }
    let host = url
        .host_str()
        .ok_or_else(|| (common.id.clone(), "hook_http_host_required"))?;
    let endpoint_policy = if is_literal_loopback_host(host) {
        if !handler.allow_insecure_loopback {
            return Err((common.id, "hook_http_loopback_policy_required"));
        }
        match url.scheme() {
            "http" | "https" => HttpEndpointPolicy::ExplicitLoopback,
            _ => return Err((common.id, "hook_http_scheme_unsupported")),
        }
    } else {
        if url.scheme() != "https" {
            return Err((common.id, "hook_http_https_required"));
        }
        if parse_ip_literal(host).is_some_and(|address| !is_public_ip(address)) {
            return Err((common.id, "hook_http_non_public_address"));
        }
        HttpEndpointPolicy::PublicHttps
    };
    let auth_token = match handler.auth_token_env.as_deref() {
        Some(reference) if is_env_reference(reference) => Some(
            std::env::var(reference)
                .map_err(|_| (common.id.clone(), "hook_http_auth_reference_unavailable"))?,
        ),
        Some(_) => return Err((common.id, "hook_http_auth_reference_invalid")),
        None => None,
    };
    Ok(ValidatedHttpHandler {
        common,
        url,
        auth_token,
        endpoint_policy,
    })
}

pub(super) async fn execute_http_handler(
    handler: &ValidatedHttpHandler,
    event: &Value,
    cancellation: CancellationToken,
) -> HandlerRunResult {
    let started = Instant::now();
    let input = match serde_json::to_vec(event) {
        Ok(input) if input.len() <= handler.common.max_input_bytes => input,
        Ok(_) => {
            return handler_failure(&handler.common, "hook_event_too_large", started, 0, false);
        }
        Err(_) => {
            return handler_failure(
                &handler.common,
                "hook_event_encode_failed",
                started,
                0,
                false,
            );
        }
    };
    let client = match build_http_client(handler) {
        Ok(client) => client,
        Err(_) => {
            return handler_failure(
                &handler.common,
                "hook_http_client_build_failed",
                started,
                0,
                false,
            );
        }
    };
    let deadline = tokio::time::Instant::now() + handler.common.timeout;
    let mut attempts = 0;
    loop {
        attempts += 1;
        let request = execute_http_attempt(handler, &client, input.clone());
        let attempt = tokio::select! {
            _ = cancellation.cancelled() => {
                return handler_failure(
                    &handler.common,
                    "hook_handler_cancelled",
                    started,
                    attempts,
                    false,
                );
            }
            result = tokio::time::timeout_at(deadline, request) => {
                match result {
                    Ok(result) => result,
                    Err(_) => {
                        return handler_failure(
                            &handler.common,
                            "hook_handler_timeout",
                            started,
                            attempts,
                            false,
                        );
                    }
                }
            }
        };
        match attempt {
            Ok(output) => {
                let output = match parse_handler_output(&output, handler.common.blocking) {
                    Ok(output) => output,
                    Err(error_code) => {
                        return handler_failure(
                            &handler.common,
                            error_code,
                            started,
                            attempts,
                            false,
                        );
                    }
                };
                return HandlerRunResult {
                    decision: output.0,
                    reason_code: output.1,
                    status: "ok",
                    error_code: None,
                    duration_ms: elapsed_ms(started),
                    attempts,
                    output_truncated: false,
                };
            }
            Err(error)
                if error.retryable
                    && attempts < handler.common.max_attempts
                    && tokio::time::Instant::now() < deadline =>
            {
                tokio::task::yield_now().await;
            }
            Err(error) => {
                return handler_failure(
                    &handler.common,
                    error.code,
                    started,
                    attempts,
                    error.output_truncated,
                );
            }
        }
    }
}

fn build_http_client(handler: &ValidatedHttpHandler) -> anyhow::Result<reqwest::Client> {
    match handler.endpoint_policy {
        HttpEndpointPolicy::PublicHttps => build_public_http_client(),
        HttpEndpointPolicy::ExplicitLoopback => Ok(reqwest::Client::builder()
            .redirect(Policy::none())
            .timeout(handler.common.timeout)
            .build()?),
    }
}

async fn execute_http_attempt(
    handler: &ValidatedHttpHandler,
    client: &reqwest::Client,
    input: Vec<u8>,
) -> Result<Vec<u8>, HttpAttemptError> {
    let mut request = client
        .post(handler.url.clone())
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(input);
    if let Some(token) = handler.auth_token.as_deref() {
        request = request.bearer_auth(token);
    }
    let mut response = request.send().await.map_err(|_| HttpAttemptError {
        code: "hook_http_request_failed",
        retryable: true,
        output_truncated: false,
    })?;
    if response.status().is_redirection() {
        return Err(HttpAttemptError {
            code: "hook_http_redirect_forbidden",
            retryable: false,
            output_truncated: false,
        });
    }
    if !response.status().is_success() {
        return Err(HttpAttemptError {
            code: "hook_http_status_error",
            retryable: response.status().is_server_error()
                || response.status() == StatusCode::TOO_MANY_REQUESTS,
            output_truncated: false,
        });
    }
    let mut output = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| HttpAttemptError {
        code: "hook_http_response_read_failed",
        retryable: true,
        output_truncated: false,
    })? {
        if output.len().saturating_add(chunk.len()) > handler.common.max_output_bytes {
            return Err(HttpAttemptError {
                code: "hook_handler_output_too_large",
                retryable: false,
                output_truncated: true,
            });
        }
        output.extend_from_slice(&chunk);
    }
    Ok(output)
}

fn is_literal_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || parse_ip_literal(host).is_some_and(|address| address.is_loopback())
}

fn parse_ip_literal(host: &str) -> Option<IpAddr> {
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .parse()
        .ok()
}
