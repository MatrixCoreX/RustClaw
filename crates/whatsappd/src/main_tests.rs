use super::{extract_bind_key_candidate, is_unbound_allowed_command};
use claw_core::channel_commands::ChannelCommandCatalog;

fn default_catalog() -> ChannelCommandCatalog {
    ChannelCommandCatalog::default()
}

fn unbound_allowed(text: &str) -> bool {
    is_unbound_allowed_command(&default_catalog(), "whatsapp", text)
}

#[test]
fn unbound_plain_text_requires_binding_prompt() {
    assert!(!unbound_allowed("hello"));
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
fn unbound_start_and_help_are_allowed_without_task_submission() {
    assert!(unbound_allowed("/start"));
    assert!(unbound_allowed("/help"));
}

#[test]
fn waiting_key_state_accepts_plain_key_reply() {
    assert_eq!(
        extract_bind_key_candidate("rk_live_abc", true).as_deref(),
        Some("rk_live_abc")
    );
}

#[test]
fn waiting_key_state_does_not_treat_business_commands_as_key() {
    assert_eq!(extract_bind_key_candidate("/run weather {}", true), None);
    assert_eq!(extract_bind_key_candidate("/crypto btc", true), None);
}

#[test]
fn unbound_media_like_empty_text_requires_binding_prompt() {
    assert!(!unbound_allowed(""));
    assert_eq!(extract_bind_key_candidate("", false), None);
}
