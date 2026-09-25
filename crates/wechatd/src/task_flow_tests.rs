use super::*;

fn task_response(
    status: TaskStatus,
    execution_state: Option<TaskExecutionState>,
    lifecycle: Option<Value>,
) -> TaskQueryResponse {
    serde_json::from_value(json!({
        "task_id": "00000000-0000-0000-0000-000000000000",
        "status": status,
        "execution_state": execution_state,
        "result_json": null,
        "error_text": null,
        "lifecycle": lifecycle,
    }))
    .expect("task response fixture")
}

#[test]
fn needs_user_lifecycle_stops_channel_polling_and_requires_attention() {
    let task = task_response(
        TaskStatus::Running,
        Some(TaskExecutionState::NeedsConfirmation),
        Some(json!({"state": "needs_user"})),
    );
    assert_eq!(
        wechat_task_poll_disposition(&task),
        WechatTaskPollDisposition::RequiresAttention
    );
}

#[test]
fn background_lifecycle_keeps_channel_polling() {
    let task = task_response(
        TaskStatus::Running,
        Some(TaskExecutionState::Background),
        Some(json!({"state": "background"})),
    );
    assert_eq!(
        wechat_task_poll_disposition(&task),
        WechatTaskPollDisposition::Continue
    );
}

#[test]
fn terminal_database_status_uses_unified_delivery() {
    let task = task_response(
        TaskStatus::Failed,
        Some(TaskExecutionState::Failed),
        Some(json!({"state": "failed"})),
    );
    assert_eq!(
        wechat_task_poll_disposition(&task),
        WechatTaskPollDisposition::DeliverTerminal
    );
}

#[test]
fn failed_execution_projection_never_polls_forever_under_running_database_status() {
    let task = task_response(
        TaskStatus::Running,
        Some(TaskExecutionState::Failed),
        Some(json!({"state": "failed"})),
    );
    assert_eq!(
        wechat_task_poll_disposition(&task),
        WechatTaskPollDisposition::RequiresAttention
    );
}

#[tokio::test]
async fn durable_handoff_requires_the_submitter_to_confirm_acceptance() {
    let (accepted, receipt) = tokio::sync::oneshot::channel();
    accepted.send(()).expect("handoff receiver");
    assert!(durable_handoff_received(receipt).await);

    let (dropped, receipt) = tokio::sync::oneshot::channel::<()>();
    drop(dropped);
    assert!(!durable_handoff_received(receipt).await);
}
