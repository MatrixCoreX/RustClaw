use super::{chat_control, ChatControl};

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
