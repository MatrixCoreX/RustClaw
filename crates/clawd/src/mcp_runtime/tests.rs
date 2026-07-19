use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use tokio::sync::oneshot;

use super::manager::McpRuntime;
use super::test_support::{fixture_config, started_fixture_runtime};
use super::types::McpLifecycleState;

#[tokio::test]
async fn stdio_runtime_discovers_paginated_tools_calls_bounds_and_stops() {
    let runtime = McpRuntime::new(fixture_config());
    runtime.start().await;

    let lifecycle = runtime.lifecycle_snapshots();
    assert_eq!(lifecycle.len(), 1);
    assert_eq!(lifecycle[0].state, McpLifecycleState::Ready);
    assert_eq!(lifecycle[0].tool_count, 4);
    let probe = runtime.probe("fixture").await.expect("protocol ping");
    assert_eq!(probe.server_id, "fixture");
    assert_eq!(probe.status, "ok");

    let tools = runtime.tools();
    assert_eq!(tools.len(), 4);
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool.capability.as_str())
            .collect::<Vec<_>>(),
        vec![
            "mcp.fixture.fail",
            "mcp.fixture.large",
            "mcp.fixture.lookup",
            "mcp.fixture.slow",
        ]
    );
    let lookup = runtime
        .tool("mcp.fixture.lookup")
        .expect("lookup descriptor");
    assert_eq!(lookup.required_args, vec!["query"]);
    assert_eq!(lookup.policy.effect, "observe");
    assert_eq!(lookup.policy.risk_level, "low");
    assert!(lookup.policy.idempotent);

    let success = runtime
        .call(
            "mcp.fixture.lookup",
            json!({"query": "machine-token"}),
            None,
        )
        .await
        .expect("lookup call");
    assert_eq!(success.status, "ok");
    assert_eq!(
        success.structured_content,
        Some(json!({"query": "machine-token", "source": "fixture"}))
    );
    assert!(!success.truncated);

    let failure = runtime
        .call("mcp.fixture.fail", json!({}), None)
        .await
        .expect("tool-level errors remain protocol results");
    assert_eq!(failure.status, "error");
    assert_eq!(failure.error_code.as_deref(), Some("mcp_tool_result_error"));
    assert_eq!(
        failure.structured_content,
        Some(json!({"error_code": "fixture_failure"}))
    );

    let large = runtime
        .call("mcp.fixture.large", json!({}), None)
        .await
        .expect("oversized result projection");
    assert!(large.truncated);
    assert_eq!(large.content["truncated"], true);
    assert!(large.structured_content.is_none());

    let timeout = runtime
        .call("mcp.fixture.slow", json!({}), None)
        .await
        .expect_err("slow tool must time out");
    assert_eq!(timeout.code(), "mcp_call_timeout");

    runtime.stop().await;
    assert!(runtime.tools().is_empty());
    assert_eq!(
        runtime.lifecycle_snapshots()[0].state,
        McpLifecycleState::Stopped
    );
}

#[tokio::test]
async fn duplicate_namespaces_fail_closed_before_connecting() {
    let mut config = fixture_config();
    let mut second = config
        .servers
        .get("fixture")
        .expect("fixture config")
        .clone();
    second.capability_prefix = Some("fixture".to_string());
    config
        .servers
        .get_mut("fixture")
        .expect("fixture config")
        .capability_prefix = Some("fixture".to_string());
    config.servers.insert("fixture_two".to_string(), second);
    let runtime = McpRuntime::new(config);

    runtime.start().await;

    assert!(runtime.tools().is_empty());
    let lifecycle = runtime.lifecycle_snapshots();
    assert_eq!(lifecycle.len(), 2);
    assert!(lifecycle.iter().all(|snapshot| {
        snapshot.state == McpLifecycleState::Degraded
            && snapshot.last_error_code.as_deref() == Some("mcp_capability_prefix_duplicate")
    }));
    runtime.stop().await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn duplicate_tool_failure_cleans_up_stdio_process() {
    let pid_file =
        std::env::temp_dir().join(format!("rustclaw-mcp-fixture-pid-{}", uuid::Uuid::new_v4()));
    let mut config = fixture_config();
    let fixture = config.servers.get_mut("fixture").expect("fixture config");
    fixture
        .env
        .insert("MCP_FIXTURE_MODE".to_string(), "duplicate_tool".to_string());
    fixture.env.insert(
        "MCP_FIXTURE_PID_FILE".to_string(),
        pid_file.to_string_lossy().into_owned(),
    );
    let runtime = McpRuntime::new(config);

    runtime.start().await;

    let lifecycle = runtime.lifecycle_snapshots();
    assert_eq!(lifecycle[0].state, McpLifecycleState::Degraded);
    assert_eq!(
        lifecycle[0].last_error_code.as_deref(),
        Some("mcp_tool_name_duplicate")
    );
    assert!(runtime.tools().is_empty());
    let pid: u32 = std::fs::read_to_string(&pid_file)
        .expect("fixture pid")
        .parse()
        .expect("numeric fixture pid");
    let mut exited = false;
    for _ in 0..50 {
        if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
            exited = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        exited,
        "fixture process must exit after discovery rejection"
    );
    runtime.stop().await;
    let _ = std::fs::remove_file(pid_file);
}

#[tokio::test]
async fn untrusted_and_invalid_schema_servers_fail_closed() {
    let mut untrusted = fixture_config();
    untrusted.servers.get_mut("fixture").unwrap().trusted = false;
    untrusted.servers.get_mut("fixture").unwrap().command = Some("missing-command".to_string());
    let runtime = McpRuntime::new(untrusted);
    runtime.start().await;
    let lifecycle = runtime.lifecycle_snapshots();
    assert_eq!(lifecycle[0].state, McpLifecycleState::Degraded);
    assert_eq!(
        lifecycle[0].last_error_code.as_deref(),
        Some("mcp_server_untrusted")
    );
    assert!(runtime.tools().is_empty());
    assert!(runtime.reconnect_retry_blocked("fixture"));
    runtime.health_tick().await;
    assert!(runtime.reconnect_retry_blocked("fixture"));

    let mut invalid = fixture_config();
    invalid
        .servers
        .get_mut("fixture")
        .unwrap()
        .env
        .insert("MCP_FIXTURE_MODE".to_string(), "invalid_schema".to_string());
    let runtime = McpRuntime::new(invalid);
    runtime.start().await;
    let lifecycle = runtime.lifecycle_snapshots();
    assert_eq!(lifecycle[0].state, McpLifecycleState::Degraded);
    assert_eq!(
        lifecycle[0].last_error_code.as_deref(),
        Some("mcp_tool_schema_invalid")
    );
    assert!(runtime.tools().is_empty());
    runtime.stop().await;
}

#[tokio::test]
async fn conflicting_http_auth_is_blocked_without_background_retry() {
    let mut config = fixture_config();
    let fixture = config.servers.get_mut("fixture").expect("fixture config");
    fixture.transport = claw_core::config::McpTransportConfig::StreamableHttp;
    fixture.command = None;
    fixture.url = Some("http://127.0.0.1:1/mcp".to_string());
    fixture.auth_token_env = Some("RUSTCLAW_MCP_BEARER".to_string());
    fixture.oauth_client_id_env = Some("RUSTCLAW_MCP_CLIENT_ID".to_string());
    fixture.oauth_client_secret_env = Some("RUSTCLAW_MCP_CLIENT_SECRET".to_string());
    let runtime = McpRuntime::new(config);
    runtime.start().await;
    let lifecycle = runtime.lifecycle_snapshots();
    assert_eq!(
        lifecycle[0].last_error_code.as_deref(),
        Some("mcp_http_auth_conflict")
    );
    assert!(runtime.reconnect_retry_blocked("fixture"));
    runtime.health_tick().await;
    assert!(runtime.reconnect_retry_blocked("fixture"));
}

#[test]
fn configuration_validation_rejects_invalid_secret_reference_names() {
    let mut config = fixture_config();
    config
        .servers
        .get_mut("fixture")
        .expect("fixture config")
        .auth_token_env = Some("literal bearer value".to_string());
    assert_eq!(
        McpRuntime::validate_configuration(&config),
        Err("mcp_auth_token_ref_invalid")
    );

    let mut config = fixture_config();
    config
        .servers
        .get_mut("fixture")
        .expect("fixture config")
        .env_refs
        .insert("CHILD_TOKEN".to_string(), "not-an-env-ref".to_string());
    assert_eq!(
        McpRuntime::validate_configuration(&config),
        Err("mcp_stdio_env_ref_invalid")
    );
}

#[tokio::test]
async fn health_tick_reconnects_closed_transport_without_replaying_a_tool() {
    let marker =
        std::env::temp_dir().join(format!("rustclaw-mcp-reconnect-{}", uuid::Uuid::new_v4()));
    let mut config = fixture_config();
    config
        .servers
        .get_mut("fixture")
        .expect("fixture config")
        .env
        .insert(
            "MCP_FIXTURE_EXIT_ONCE_MARKER".to_string(),
            marker.to_string_lossy().into_owned(),
        );
    let runtime = McpRuntime::new(config);
    runtime.start().await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    runtime.health_tick().await;

    assert_eq!(
        runtime.lifecycle_snapshots()[0].state,
        McpLifecycleState::Ready
    );
    let result = runtime
        .call(
            "mcp.fixture.lookup",
            json!({"query": "after-reconnect"}),
            None,
        )
        .await
        .expect("call after reconnect");
    assert_eq!(result.status, "ok");
    runtime.stop().await;
    let _ = std::fs::remove_file(marker);
}

#[tokio::test]
async fn dynamic_mcp_capability_reaches_resolver_prompt_and_verifier_policy() {
    let runtime = started_fixture_runtime().await;
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.core.mcp_runtime = Arc::clone(&runtime);
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "mcp-policy-task".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: Some("mcp-policy-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };

    let capability_map =
        crate::capability_map::build_compact_capability_map_for_task(&state, &task);
    assert!(capability_map.contains("mcp.fixture.lookup"));
    assert!(capability_map.contains("required=query"));
    assert!(capability_map.contains("effect=observe"));

    let plan = crate::agent_engine::direct_capability_plan(
        &state,
        "mcp.fixture.lookup",
        json!({"query": "machine-token"}),
    );
    assert_eq!(plan.steps[0].action_type, "call_capability");
    assert_eq!(plan.steps[0].skill, "mcp.fixture.lookup");
    let verified = crate::verifier::verify_plan(
        &state,
        &task,
        crate::verifier::VerifyInput {
            output_contract: None,
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        crate::verifier::VerifyMode::Enforce,
    );
    assert!(verified.approved, "issues={:?}", verified.issues);
    assert!(!verified.needs_confirmation);
    assert_eq!(verified.approved_steps[0].action_type, "call_tool");
    assert_eq!(verified.approved_steps[0].skill, "mcp.fixture.lookup");
    assert_eq!(
        verified.capability_resolutions[0]
            .record
            .canonical_capability_ref
            .as_deref(),
        Some("mcp.fixture.lookup")
    );
    let permission = &verified.permission_decision["steps"][0];
    assert_eq!(permission["decision"], "allow");
    assert_eq!(permission["risk_level"], "low");
    assert_eq!(permission["registry_policy"]["source"], "mcp_config");

    let missing =
        crate::agent_engine::direct_capability_plan(&state, "mcp.fixture.lookup", json!({}));
    let missing = crate::verifier::verify_plan(
        &state,
        &task,
        crate::verifier::VerifyInput {
            output_contract: None,
            request_text: None,
            context_bundle_summary: None,
            plan_result: &missing,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        crate::verifier::VerifyMode::Enforce,
    );
    assert!(!missing.approved);
    assert!(missing.issues.iter().any(|issue| {
        issue.kind == crate::verifier::VerifyIssueKind::MissingRequiredArg
            && issue.missing_fields == vec!["query"]
    }));
    runtime.stop().await;
}

#[tokio::test]
async fn mutating_mcp_tool_requires_shared_permission_confirmation() {
    let mut config = fixture_config();
    let fixture = config.servers.get_mut("fixture").expect("fixture config");
    fixture.tool_policies.insert(
        "lookup".to_string(),
        claw_core::config::McpToolPolicyConfig {
            effect: claw_core::config::McpToolEffectConfig::Mutate,
            risk_level: claw_core::config::McpToolRiskConfig::High,
            idempotent: false,
            filesystem_write: true,
            ..claw_core::config::McpToolPolicyConfig::default()
        },
    );
    let runtime = Arc::new(McpRuntime::new(config));
    runtime.start().await;
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.core.mcp_runtime = Arc::clone(&runtime);
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "mcp-confirm-task".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: Some("mcp-confirm-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let plan = crate::agent_engine::direct_capability_plan(
        &state,
        "mcp.fixture.lookup",
        json!({"query": "machine-token"}),
    );

    let verified = crate::verifier::verify_plan(
        &state,
        &task,
        crate::verifier::VerifyInput {
            output_contract: None,
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        crate::verifier::VerifyMode::Enforce,
    );

    assert!(verified.approved);
    assert!(verified.needs_confirmation);
    assert_eq!(
        verified.permission_decision["decision"],
        "require_confirmation"
    );
    let permission = &verified.permission_decision["steps"][0];
    assert_eq!(permission["decision"], "require_confirmation");
    assert_eq!(permission["risk_level"], "high");
    assert_eq!(permission["action_effect"]["mutates"], true);
    assert_eq!(permission["action_effect"]["observes"], false);
    assert_eq!(permission["registry_policy"]["filesystem_write"], true);
    assert_eq!(permission["registry_policy"]["idempotent"], false);
    runtime.stop().await;
}

#[tokio::test]
async fn large_catalog_uses_bounded_search_then_discloses_matching_schema() {
    let mut config = fixture_config();
    config.planner_visible_tools = 3;
    config.catalog_search_max_results = 2;
    let runtime = Arc::new(McpRuntime::new(config));
    runtime.start().await;

    let planner_tools = runtime.planner_tools();
    assert_eq!(planner_tools.len(), 3);
    assert!(planner_tools
        .iter()
        .any(|tool| tool.capability == "mcp.catalog.search"));
    assert!(!planner_tools
        .iter()
        .any(|tool| tool.capability == "mcp.fixture.slow"));

    let search = runtime
        .call(
            "mcp.catalog.search",
            json!({"query": "slow", "limit": 1}),
            None,
        )
        .await
        .expect("catalog search");
    assert_eq!(search.status, "ok");
    let result = search.structured_content.expect("structured search result");
    assert_eq!(result["match_count"], 1);
    assert_eq!(result["returned_count"], 1);
    assert_eq!(result["tools"][0]["capability"], "mcp.fixture.slow");
    assert_eq!(result["tools"][0]["input_schema"]["type"], "object");

    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.core.mcp_runtime = Arc::clone(&runtime);
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "mcp-search-task".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: Some("mcp-search-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let capability_map =
        crate::capability_map::build_compact_capability_map_for_task(&state, &task);
    assert!(capability_map.contains("mcp.catalog.search"));
    assert!(!capability_map.contains("mcp.fixture.slow"));
    runtime.stop().await;
}

async fn streamable_http_fixture(Json(message): Json<serde_json::Value>) -> Response {
    let Some(request_id) = message.get("id").cloned() else {
        return StatusCode::ACCEPTED.into_response();
    };
    let method = message
        .get("method")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let result = match method {
        "initialize" => json!({
            "protocolVersion": message
                .pointer("/params/protocolVersion")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("2025-11-25"),
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {"name": "rustclaw-http-fixture", "version": "1"},
        }),
        "tools/list" => json!({
            "tools": [
                {
                    "name": "lookup",
                    "description": "fixture_lookup",
                    "inputSchema": {
                        "type": "object",
                        "properties": {"query": {"type": "string"}},
                        "required": ["query"],
                    },
                },
                {
                    "name": "slow",
                    "description": "fixture_slow",
                    "inputSchema": {
                        "type": "object",
                        "properties": {},
                        "required": [],
                    },
                }
            ],
        }),
        "tools/call" => {
            if message
                .pointer("/params/name")
                .and_then(serde_json::Value::as_str)
                == Some("slow")
            {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
            json!({
                "content": [{"type": "text", "text": "fixture_http_text"}],
                "structuredContent": {
                    "query": message.pointer("/params/arguments/query").cloned(),
                    "transport": "streamable_http",
                },
                "isError": false,
            })
        }
        "ping" => json!({}),
        _ => {
            return (
                StatusCode::OK,
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "error": {"code": -32601, "message": "method_not_found"},
                })),
            )
                .into_response();
        }
    };
    (
        StatusCode::OK,
        Json(json!({"jsonrpc": "2.0", "id": request_id, "result": result})),
    )
        .into_response()
}

#[derive(Default)]
struct OAuthFixtureState {
    token_requests: AtomicUsize,
}

fn oauth_fixture_base_url(headers: &HeaderMap) -> String {
    format!(
        "http://{}",
        headers
            .get("host")
            .and_then(|value| value.to_str().ok())
            .expect("fixture host")
    )
}

async fn oauth_resource_metadata(headers: HeaderMap) -> Json<serde_json::Value> {
    let base_url = oauth_fixture_base_url(&headers);
    Json(json!({
        "resource": format!("{base_url}/mcp"),
        "authorization_servers": [base_url],
        "scopes_supported": ["read"],
    }))
}

async fn oauth_authorization_metadata(headers: HeaderMap) -> Json<serde_json::Value> {
    let base_url = oauth_fixture_base_url(&headers);
    Json(json!({
        "issuer": base_url,
        "authorization_endpoint": format!("{base_url}/authorize"),
        "token_endpoint": format!("{base_url}/token"),
        "token_endpoint_auth_methods_supported": ["client_secret_post"],
        "grant_types_supported": ["client_credentials", "refresh_token"],
        "scopes_supported": ["read"],
    }))
}

async fn oauth_token(State(state): State<Arc<OAuthFixtureState>>, body: String) -> Response {
    state.token_requests.fetch_add(1, Ordering::SeqCst);
    if body.contains("grant_type=client_credentials")
        && body.contains("client_id=fixture-client")
        && body.contains("client_secret=fixture-secret")
    {
        return Json(json!({
            "access_token": "expired-fixture-token",
            "refresh_token": "fixture-refresh-token",
            "token_type": "Bearer",
            "expires_in": 0,
            "scope": "read",
        }))
        .into_response();
    }
    if body.contains("grant_type=refresh_token")
        && body.contains("refresh_token=fixture-refresh-token")
    {
        return Json(json!({
            "access_token": "active-fixture-token",
            "token_type": "Bearer",
            "expires_in": 3600,
            "scope": "read",
        }))
        .into_response();
    }
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({"error": "invalid_client"})),
    )
        .into_response()
}

async fn oauth_mcp_fixture(
    State(_state): State<Arc<OAuthFixtureState>>,
    headers: HeaderMap,
    message: Json<serde_json::Value>,
) -> Response {
    if headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        != Some("Bearer active-fixture-token")
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    streamable_http_fixture(message).await
}

#[tokio::test]
async fn oauth_client_credentials_discovers_refreshes_and_redacts_tokens() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind OAuth fixture");
    let address = listener.local_addr().expect("fixture address");
    let state = Arc::new(OAuthFixtureState::default());
    let server_state = Arc::clone(&state);
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/.well-known/oauth-protected-resource/mcp",
                    get(oauth_resource_metadata),
                )
                .route(
                    "/.well-known/oauth-protected-resource",
                    get(oauth_resource_metadata),
                )
                .route(
                    "/.well-known/oauth-authorization-server",
                    get(oauth_authorization_metadata),
                )
                .route("/token", post(oauth_token))
                .route("/mcp", post(oauth_mcp_fixture))
                .with_state(server_state),
        )
        .with_graceful_shutdown(async {
            let _ = stop_rx.await;
        })
        .await
        .expect("serve OAuth fixture");
    });

    let suffix = uuid::Uuid::new_v4()
        .simple()
        .to_string()
        .to_ascii_uppercase();
    let client_id_env = format!("RUSTCLAW_MCP_OAUTH_CLIENT_ID_{suffix}");
    let client_secret_env = format!("RUSTCLAW_MCP_OAUTH_CLIENT_SECRET_{suffix}");
    std::env::set_var(&client_id_env, "fixture-client");
    std::env::set_var(&client_secret_env, "fixture-secret");

    let mut config = fixture_config();
    let fixture = config.servers.get_mut("fixture").expect("fixture config");
    fixture.transport = claw_core::config::McpTransportConfig::StreamableHttp;
    fixture.command = None;
    fixture.args.clear();
    fixture.url = Some(format!("http://{address}/mcp"));
    fixture.allowed_tools = vec!["lookup".to_string()];
    fixture.oauth_client_id_env = Some(client_id_env.clone());
    fixture.oauth_client_secret_env = Some(client_secret_env.clone());
    fixture.oauth_scopes = vec!["read".to_string()];
    let runtime = McpRuntime::new(config);
    runtime.start().await;

    let lifecycle = runtime.lifecycle_snapshots();
    assert_eq!(lifecycle[0].state, McpLifecycleState::Ready);
    assert_eq!(lifecycle[0].auth_mode, "oauth_client_credentials");
    let lifecycle_json = serde_json::to_string(&lifecycle).expect("serialize lifecycle");
    assert!(!lifecycle_json.contains("fixture-client"));
    assert!(!lifecycle_json.contains("fixture-secret"));
    assert!(!lifecycle_json.contains("active-fixture-token"));
    assert!(state.token_requests.load(Ordering::SeqCst) >= 2);

    let outcome = runtime
        .call("mcp.fixture.lookup", json!({"query": "oauth-token"}), None)
        .await
        .expect("OAuth MCP call");
    assert_eq!(outcome.status, "ok");
    runtime.stop().await;
    std::env::remove_var(client_id_env);
    std::env::remove_var(client_secret_env);
    let _ = stop_tx.send(());
    server.await.expect("join OAuth fixture");
}

#[tokio::test]
async fn streamable_http_runtime_initializes_discovers_and_calls() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind HTTP fixture");
    let address = listener.local_addr().expect("fixture address");
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/mcp", post(streamable_http_fixture)),
        )
        .with_graceful_shutdown(async {
            let _ = stop_rx.await;
        })
        .await
        .expect("serve HTTP fixture");
    });

    let mut config = fixture_config();
    let fixture = config.servers.get_mut("fixture").expect("fixture config");
    fixture.transport = claw_core::config::McpTransportConfig::StreamableHttp;
    fixture.command = None;
    fixture.args.clear();
    fixture.url = Some(format!("http://{address}/mcp"));
    fixture.allowed_tools = vec!["lookup".to_string(), "slow".to_string()];
    let runtime = McpRuntime::new(config);
    runtime.start().await;
    assert_eq!(
        runtime.lifecycle_snapshots()[0].state,
        McpLifecycleState::Ready
    );
    assert_eq!(
        runtime.probe("fixture").await.expect("HTTP ping").status,
        "ok"
    );
    let outcome = runtime
        .call("mcp.fixture.lookup", json!({"query": "http-token"}), None)
        .await
        .expect("HTTP MCP call");
    assert_eq!(outcome.status, "ok");
    assert_eq!(
        outcome.structured_content,
        Some(json!({
            "query": "http-token",
            "transport": "streamable_http",
        }))
    );

    let runtime = Arc::new(runtime);
    let cancellation = tokio_util::sync::CancellationToken::new();
    let slow_runtime = Arc::clone(&runtime);
    let slow_cancellation = cancellation.clone();
    let slow = tokio::spawn(async move {
        slow_runtime
            .call("mcp.fixture.slow", json!({}), Some(slow_cancellation))
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    cancellation.cancel();
    let cancelled = slow
        .await
        .expect("join cancelled call")
        .expect_err("slow call must cancel");
    assert_eq!(cancelled.code(), "mcp_call_cancelled");
    let unrelated = runtime
        .call("mcp.fixture.lookup", json!({"query": "after-cancel"}), None)
        .await
        .expect("unrelated call remains available");
    assert_eq!(unrelated.status, "ok");
    runtime.stop().await;
    let _ = stop_tx.send(());
    server.await.expect("join HTTP fixture");
}

#[test]
fn lifecycle_tokens_and_machine_errors_are_language_neutral() {
    for (state, token) in [
        (McpLifecycleState::Disabled, "disabled"),
        (McpLifecycleState::Starting, "starting"),
        (McpLifecycleState::Ready, "ready"),
        (McpLifecycleState::Degraded, "degraded"),
        (McpLifecycleState::Stopped, "stopped"),
    ] {
        assert_eq!(state.as_token(), token);
    }
}
