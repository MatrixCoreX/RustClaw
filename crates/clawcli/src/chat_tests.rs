use super::{chat_control, follow_outcome, ChatControl, FollowOutcome};

#[test]
fn chat_controls_are_explicit_slash_protocol_not_natural_language() {
    assert_eq!(chat_control("exit"), None);
    assert_eq!(chat_control("quit"), None);
    assert_eq!(chat_control("/exit"), Some(ChatControl::Exit));
    assert_eq!(chat_control("/new"), Some(ChatControl::New));
    assert_eq!(chat_control("/detach"), Some(ChatControl::Detach));
    assert_eq!(chat_control("/cancel"), Some(ChatControl::Cancel));
    assert_eq!(chat_control("/status"), Some(ChatControl::Status));
    assert_eq!(
        chat_control("/attach task-123"),
        Some(ChatControl::Attach("task-123"))
    );
    assert_eq!(
        chat_control("/attach task-1 extra"),
        Some(ChatControl::Unknown("/attach"))
    );
}

#[test]
fn chat_event_follow_stops_on_terminal_or_background_machine_state() {
    let terminal = serde_json::json!({"event_type": "task_final", "payload": {}});
    assert_eq!(follow_outcome(&terminal), Some(FollowOutcome::Terminal));

    let background = serde_json::json!({
        "event_type": "task_lifecycle",
        "payload": {"execution_state": "background"}
    });
    assert_eq!(follow_outcome(&background), Some(FollowOutcome::Background));

    let running = serde_json::json!({
        "event_type": "tool_started",
        "payload": {"state": "running"}
    });
    assert_eq!(follow_outcome(&running), Some(FollowOutcome::StreamEnded));
}
