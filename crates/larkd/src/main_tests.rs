use super::{extract_bind_key_candidate, is_unbound_allowed_command};

#[test]
fn unbound_plain_text_requires_binding_prompt() {
    assert!(!is_unbound_allowed_command("hello"));
    assert_eq!(extract_bind_key_candidate("hello", false), None);
}

#[test]
fn unbound_key_command_keeps_binding_flow_available() {
    assert_eq!(
        extract_bind_key_candidate("/key rk_live_123", false).as_deref(),
        Some("rk_live_123")
    );
}

#[test]
fn unbound_help_and_start_are_allowed() {
    assert!(is_unbound_allowed_command("/start"));
    assert!(is_unbound_allowed_command("/help"));
    assert!(!is_unbound_allowed_command("/start/docs"));
    assert!(!is_unbound_allowed_command("/help.md"));
}

#[test]
fn waiting_key_state_accepts_plain_key_reply() {
    assert_eq!(
        extract_bind_key_candidate("rk_live_abc", true).as_deref(),
        Some("rk_live_abc")
    );
}

#[test]
fn waiting_key_state_rejects_non_binding_commands() {
    assert_eq!(
        extract_bind_key_candidate("/run image_vision {}", true),
        None
    );
    assert_eq!(extract_bind_key_candidate("/crypto btc", true), None);
}

#[test]
fn unbound_media_like_empty_text_requires_binding_prompt() {
    assert!(!is_unbound_allowed_command(""));
    assert_eq!(extract_bind_key_candidate("", false), None);
}
