use std::{sync::Arc, time::Duration};

use serde_json::{json, Value};

use super::{record_mcp_tool_execution_observation, run_mcp_tool_observation, LoopState};

fn task() -> crate::ClaimedTask {
    crate::ClaimedTask {
        claim_attempt: 0,
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

#[tokio::test]
async fn mcp_execution_records_auditable_teaching_observation() {
    let runtime = crate::mcp_runtime::test_support::started_fixture_runtime().await;
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.core.mcp_runtime = Arc::clone(&runtime);
    let task = task();
    let (raw, extra) = run_mcp_tool_observation(
        &state,
        &task,
        "mcp.fixture.lookup",
        json!({"query": "teaching-token"}),
    )
    .await
    .expect("mcp observation");
    let descriptor = state
        .mcp_tool("mcp.fixture.lookup")
        .expect("fixture descriptor");
    let step = crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: descriptor.capability.clone(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(raw),
        error: None,
        started_at: 1,
        finished_at: 2,
    };
    let mut loop_state = LoopState::new(2);

    record_mcp_tool_execution_observation(
        &state,
        &task,
        &mut loop_state,
        &descriptor,
        &step,
        Some(&extra),
        Duration::from_millis(17),
    );

    let observation = loop_state.task_observations.last().expect("observation");
    assert_eq!(observation["owner_layer"], "mcp_runtime");
    assert_eq!(observation["capability"], "mcp.fixture.lookup");
    assert_eq!(observation["lifecycle_state"], "ready");
    assert_eq!(observation["policy_decision"], "allow");
    assert_eq!(observation["latency_ms"], 17);
    assert_eq!(observation["status"], "ok");
    assert_eq!(observation["truncated"], false);
    assert!(observation.get("structured_content").is_none());

    let connection = state.core.audit_db.get().expect("audit db");
    let detail: String = connection
        .query_row(
            "SELECT detail_json FROM audit_logs WHERE action = 'mcp.tool_call' ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("mcp audit row");
    let detail: Value = serde_json::from_str(&detail).expect("audit json");
    assert_eq!(detail["task_id"], task.task_id);
    assert_eq!(detail["server_id"], "fixture");
    assert_eq!(detail["output_bytes"], observation["output_bytes"]);
    assert!(!detail.to_string().contains("teaching-token"));

    runtime.stop().await;
}

#[tokio::test]
async fn mcp_transport_failure_records_machine_error_without_raw_detail() {
    let runtime = crate::mcp_runtime::test_support::started_fixture_runtime().await;
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.core.mcp_runtime = Arc::clone(&runtime);
    let task = task();
    let descriptor = state
        .mcp_tool("mcp.fixture.lookup")
        .expect("fixture descriptor");
    let step = crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: descriptor.capability.clone(),
        status: crate::executor::StepExecutionStatus::Error,
        output: None,
        error: Some(
            json!({
                "error_code": "mcp_transport_closed",
                "message_key": "mcp_transport_closed",
                "adapter_kind": "mcp_tool",
                "internal_detail": "sensitive transport detail",
            })
            .to_string(),
        ),
        started_at: 1,
        finished_at: 2,
    };
    let mut loop_state = LoopState::new(2);

    record_mcp_tool_execution_observation(
        &state,
        &task,
        &mut loop_state,
        &descriptor,
        &step,
        None,
        Duration::from_millis(23),
    );

    let observation = loop_state.task_observations.last().expect("observation");
    assert_eq!(observation["status"], "error");
    assert_eq!(observation["error_code"], "mcp_transport_closed");
    assert_eq!(observation["output_bytes"], 0);
    assert!(!observation
        .to_string()
        .contains("sensitive transport detail"));

    let connection = state.core.audit_db.get().expect("audit db");
    let detail: String = connection
        .query_row(
            "SELECT detail_json FROM audit_logs WHERE action = 'mcp.tool_call' ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("mcp audit row");
    assert!(!detail.contains("sensitive transport detail"));

    runtime.stop().await;
}
