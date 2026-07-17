use std::sync::Arc;

use serde_json::{json, Value};

use super::run_mcp_tool_observation;

fn task() -> crate::ClaimedTask {
    crate::ClaimedTask {
        task_id: "mcp-dispatch-task".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: Some("mcp-dispatch-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

#[tokio::test]
async fn production_mcp_observation_adapter_preserves_structured_protocol_fields() {
    let runtime = crate::mcp_runtime::test_support::started_fixture_runtime().await;
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.core.mcp_runtime = Arc::clone(&runtime);

    let (raw, extra) = run_mcp_tool_observation(
        &state,
        &task(),
        "mcp.fixture.lookup",
        json!({"query": "dispatch-token"}),
    )
    .await
    .expect("mcp observation");
    let raw: Value = serde_json::from_str(&raw).expect("machine observation JSON");
    assert_eq!(raw["status"], "ok");
    assert_eq!(raw["structured_content"]["query"], "dispatch-token");
    assert_eq!(extra["adapter_kind"], "mcp_tool");
    assert_eq!(extra["mcp_result"]["server_id"], "fixture");

    let (raw, extra) = run_mcp_tool_observation(&state, &task(), "mcp.fixture.fail", json!({}))
        .await
        .expect("tool-level error remains an observation");
    let raw: Value = serde_json::from_str(&raw).expect("machine error observation JSON");
    assert_eq!(raw["status"], "error");
    assert_eq!(raw["error_code"], "mcp_tool_result_error");
    assert_eq!(
        extra["mcp_result"]["structured_content"]["error_code"],
        "fixture_failure"
    );

    runtime.stop().await;
}
