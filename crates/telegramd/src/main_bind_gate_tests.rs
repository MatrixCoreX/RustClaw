use super::{extract_bind_key_candidate, is_unbound_allowed_command};
use claw_core::channel_commands::ChannelCommandCatalog;

fn default_catalog() -> ChannelCommandCatalog {
    ChannelCommandCatalog::default()
}

#[test]
fn unbound_plain_text_requires_key_binding() {
    let catalog = default_catalog();
    assert!(!is_unbound_allowed_command(
        &catalog,
        "telegram",
        "hello rustclaw"
    ));
    assert_eq!(extract_bind_key_candidate("hello rustclaw", false), None);
}

#[test]
fn unbound_key_command_is_accepted_for_binding() {
    assert_eq!(
        extract_bind_key_candidate("/key rk_live_123", false).as_deref(),
        Some("rk_live_123")
    );
}

#[test]
fn bound_gate_allows_help_commands() {
    let catalog = default_catalog();
    assert!(is_unbound_allowed_command(&catalog, "telegram", "/start"));
    assert!(is_unbound_allowed_command(&catalog, "telegram", "/help"));
}

#[test]
fn waiting_bind_state_accepts_plain_key_reply() {
    assert_eq!(
        extract_bind_key_candidate("rk_live_abc", true).as_deref(),
        Some("rk_live_abc")
    );
}

#[test]
fn waiting_bind_state_does_not_treat_other_commands_as_key() {
    assert_eq!(extract_bind_key_candidate("/run weather {}", true), None);
    assert_eq!(extract_bind_key_candidate("/crypto btc", true), None);
}

#[test]
fn unbound_media_like_empty_text_requires_binding_prompt() {
    let catalog = default_catalog();
    assert!(!is_unbound_allowed_command(&catalog, "telegram", ""));
    assert_eq!(extract_bind_key_candidate("", false), None);
}
