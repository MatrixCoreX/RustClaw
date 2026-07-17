use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::json;
use tokio::sync::oneshot;

use super::manager::McpRuntime;
use super::test_support::{fixture_config, started_fixture_runtime};
use super::types::McpLifecycleState;

#[tokio::test]
async fn stdio_runtime_discovers_calls_bounds_and_stops() {
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
async fn dynamic_mcp_capability_reaches_resolver_prompt_and_verifier_policy() {
    let runtime = started_fixture_runtime().await;
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.core.mcp_runtime = Arc::clone(&runtime);
    let task = crate::ClaimedTask {
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

    let capability_map = crate::capability_map::build_capability_map_for_task(&state, &task);
    assert!(capability_map.contains("mcp.fixture.lookup"));
    assert!(capability_map.contains("required=query"));
    assert!(capability_map.contains("effect=observe"));

    let plan = crate::agent_engine::direct_capability_plan(
        &state,
        "mcp.fixture.lookup",
        json!({"query": "machine-token"}),
    );
    assert_eq!(plan.steps[0].action_type, "call_tool");
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
