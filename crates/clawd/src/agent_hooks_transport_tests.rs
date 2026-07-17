use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use claw_core::config::{McpToolEffectConfig, McpToolPolicyConfig, McpToolRiskConfig};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::http::{execute_http_handler, validate_http_handler};
use super::mcp::{execute_mcp_handler, validate_mcp_handler};
use super::shared::HookHandlerConfig;
use crate::policy_decision::PolicyDecision;

fn http_handler(url: String) -> HookHandlerConfig {
    HookHandlerConfig {
        id: "fixture_http_guard".to_string(),
        stage: "pre_tool_use".to_string(),
        kind: "http".to_string(),
        enabled: true,
        trusted: true,
        blocking: true,
        url,
        allow_insecure_loopback: true,
        timeout_ms: 1_000,
        max_input_bytes: 4096,
        max_output_bytes: 4096,
        max_attempts: 2,
        failure_policy: "deny".to_string(),
        ..HookHandlerConfig::default()
    }
}

async fn spawn_http_fixture(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind hook HTTP fixture");
    let address = listener.local_addr().expect("fixture address");
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("serve hook HTTP fixture");
    });
    (format!("http://{address}"), server)
}

#[tokio::test]
async fn trusted_loopback_http_hook_retries_and_returns_machine_decision() {
    async fn hook(State(attempts): State<Arc<AtomicUsize>>) -> impl IntoResponse {
        if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"error_code": "retry"})),
            );
        }
        (
            StatusCode::OK,
            Json(json!({
                "schema_version": 1,
                "decision": "require_confirmation",
                "reason_code": "fixture_http_review"
            })),
        )
    }

    let attempts = Arc::new(AtomicUsize::new(0));
    let router = Router::new()
        .route("/hook", post(hook))
        .with_state(attempts.clone());
    let (base, server) = spawn_http_fixture(router).await;
    let handler =
        validate_http_handler(http_handler(format!("{base}/hook"))).expect("validated HTTP hook");
    let result = execute_http_handler(
        &handler,
        &json!({"schema_version": 1, "event_type": "pre_tool_use"}),
        CancellationToken::new(),
    )
    .await;
    server.abort();

    assert_eq!(result.status, "ok");
    assert_eq!(result.decision, PolicyDecision::RequireConfirmation);
    assert_eq!(result.reason_code, "fixture_http_review");
    assert_eq!(result.attempts, 2);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn http_hook_rejects_external_plaintext_and_redirects() {
    let mut external = http_handler("http://example.com/hook".to_string());
    external.allow_insecure_loopback = false;
    assert_eq!(
        validate_http_handler(external)
            .expect_err("external plaintext must fail")
            .1,
        "hook_http_https_required"
    );

    async fn redirect() -> impl IntoResponse {
        (
            StatusCode::TEMPORARY_REDIRECT,
            [(header::LOCATION, "/elsewhere")],
        )
    }
    let (base, server) = spawn_http_fixture(Router::new().route("/hook", post(redirect))).await;
    let handler = validate_http_handler(http_handler(format!("{base}/hook")))
        .expect("validated loopback hook");
    let result = execute_http_handler(
        &handler,
        &json!({"schema_version": 1, "event_type": "pre_tool_use"}),
        CancellationToken::new(),
    )
    .await;
    server.abort();

    assert_eq!(result.decision, PolicyDecision::Deny);
    assert_eq!(result.error_code, Some("hook_http_redirect_forbidden"));
}

#[tokio::test]
async fn trusted_observation_only_mcp_hook_uses_structured_content_only() {
    let mut config = crate::mcp_runtime::test_support::fixture_config();
    let server = config
        .servers
        .get_mut("fixture")
        .expect("fixture MCP server");
    server
        .env
        .insert("MCP_FIXTURE_MODE".to_string(), "hook".to_string());
    server.allowed_tools.push("hook_decision".to_string());
    server.tool_policies.insert(
        "hook_decision".to_string(),
        McpToolPolicyConfig {
            effect: McpToolEffectConfig::Observe,
            risk_level: McpToolRiskConfig::Low,
            idempotent: true,
            ..McpToolPolicyConfig::default()
        },
    );
    let runtime = crate::mcp_runtime::McpRuntime::new(config);
    runtime.start().await;
    let handler = HookHandlerConfig {
        id: "fixture_mcp_guard".to_string(),
        stage: "pre_tool_use".to_string(),
        kind: "mcp".to_string(),
        enabled: true,
        trusted: true,
        blocking: true,
        capability: "mcp.fixture.hook_decision".to_string(),
        event_argument: "hook_event".to_string(),
        timeout_ms: 1_000,
        max_input_bytes: 4096,
        max_output_bytes: 4096,
        max_attempts: 1,
        failure_policy: "deny".to_string(),
        ..HookHandlerConfig::default()
    };
    let handler = validate_mcp_handler(&runtime, handler).expect("validated MCP hook");
    let result = execute_mcp_handler(
        &runtime,
        &handler,
        &json!({
            "schema_version": 1,
            "event_type": "pre_tool_use",
            "argument_fields": ["path"]
        }),
        CancellationToken::new(),
    )
    .await;
    runtime.stop().await;

    assert_eq!(result.status, "ok");
    assert_eq!(result.decision, PolicyDecision::Deny);
    assert_eq!(result.reason_code, "fixture_mcp_denied");
    assert!(result.error_code.is_none());
}

#[tokio::test]
async fn mcp_hook_rejects_unavailable_or_unsafe_capabilities() {
    let runtime =
        crate::mcp_runtime::McpRuntime::new(crate::mcp_runtime::test_support::fixture_config());
    let handler = HookHandlerConfig {
        id: "fixture_mcp_guard".to_string(),
        stage: "pre_tool_use".to_string(),
        kind: "mcp".to_string(),
        enabled: true,
        trusted: true,
        capability: "mcp.fixture.lookup".to_string(),
        ..HookHandlerConfig::default()
    };
    assert_eq!(
        validate_mcp_handler(&runtime, handler)
            .expect_err("runtime has not exposed the capability")
            .1,
        "hook_mcp_capability_unavailable"
    );

    let mut unsafe_config = crate::mcp_runtime::test_support::fixture_config();
    unsafe_config
        .servers
        .get_mut("fixture")
        .expect("fixture server")
        .tool_policies = HashMap::new();
    let unsafe_runtime = crate::mcp_runtime::McpRuntime::new(unsafe_config);
    unsafe_runtime.start().await;
    let unsafe_handler = HookHandlerConfig {
        id: "fixture_mcp_guard".to_string(),
        stage: "pre_tool_use".to_string(),
        kind: "mcp".to_string(),
        enabled: true,
        trusted: true,
        capability: "mcp.fixture.lookup".to_string(),
        ..HookHandlerConfig::default()
    };
    assert_eq!(
        validate_mcp_handler(&unsafe_runtime, unsafe_handler)
            .expect_err("unknown-risk MCP hook must fail")
            .1,
        "hook_mcp_policy_unsafe"
    );
    unsafe_runtime.stop().await;
}
