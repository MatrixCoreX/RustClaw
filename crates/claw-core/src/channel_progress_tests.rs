use super::*;

#[test]
fn chat_progress_is_deduplicated_and_stops_after_terminal() {
    let capabilities = ChannelProgressCapabilities::for_channel(ChannelKind::Telegram);
    let mut state = ChannelProgressProjectionState::default();
    assert!(!state.should_emit_progress(1, 1, 10, capabilities));
    assert!(state.should_emit_progress(2, 10, 10, capabilities));
    assert!(!state.should_emit_progress(3, 11, 10, capabilities));
    state.mark_terminal();
    assert!(!state.should_emit_progress(4, 20, 10, capabilities));
}

#[test]
fn resource_waiting_notice_is_machine_derived_once_and_is_not_failure() {
    let task: crate::types::TaskQueryResponse = serde_json::from_value(serde_json::json!({
        "task_id": "00000000-0000-0000-0000-000000000001",
        "status": "running",
        "execution_state": "waiting",
        "result_json": null,
        "error_text": null,
        "lifecycle": {
            "state": "waiting",
            "resume_reason": "resource_admission_wait",
            "message_key": "clawd.task.resource_waiting"
        }
    }))
    .expect("resource waiting task");
    let mut state = ChannelProgressProjectionState::default();
    assert!(task_is_waiting_for_resources(&task));
    assert!(state.should_emit_resource_wait_notice(&task));
    assert!(!state.should_emit_resource_wait_notice(&task));
    assert!(state.notice_sent());
    state.mark_terminal();
    assert!(!state.should_emit_resource_wait_notice(&task));
}

#[test]
fn unrelated_waiting_reason_does_not_emit_resource_notice() {
    let task: crate::types::TaskQueryResponse = serde_json::from_value(serde_json::json!({
        "task_id": "00000000-0000-0000-0000-000000000002",
        "status": "running",
        "execution_state": "waiting",
        "result_json": null,
        "error_text": null,
        "lifecycle": {"state": "waiting", "resume_reason": "provider_backoff"}
    }))
    .expect("provider waiting task");
    let mut state = ChannelProgressProjectionState::default();
    assert!(!task_is_waiting_for_resources(&task));
    assert!(!state.should_emit_resource_wait_notice(&task));
}
