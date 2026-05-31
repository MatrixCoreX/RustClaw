use super::{ChannelCommandCatalog, CoreCommandAction};

const SAMPLE: &str = r#"
[[commands]]
name = "start"
kind = "core"
core_action = "start"
channels = ["telegram", "whatsapp"]
menu_channels = ["telegram"]
allow_unbound = true
order = 10

[[commands]]
name = "crypto"
kind = "skill"
skill_name = "crypto"
channels = ["telegram"]
menu_channels = ["telegram"]
order = 20
"#;

#[test]
fn match_command_supports_bot_suffix_and_tail() {
    let catalog = ChannelCommandCatalog::from_toml_str(SAMPLE).expect("parse catalog");
    let matched = catalog
        .match_command("/start@demo_bot hello", "telegram")
        .expect("match command");
    assert_eq!(
        matched.definition.core_action(),
        Some(CoreCommandAction::Start)
    );
    assert_eq!(matched.tail, "hello");
}

#[test]
fn allows_unbound_command_follows_catalog() {
    let catalog = ChannelCommandCatalog::from_toml_str(SAMPLE).expect("parse catalog");
    assert!(catalog.allows_unbound_command("/start", "telegram"));
    assert!(!catalog.allows_unbound_command("/crypto price btc", "telegram"));
}

#[test]
fn menu_commands_filter_by_channel() {
    let catalog = ChannelCommandCatalog::from_toml_str(SAMPLE).expect("parse catalog");
    let telegram = catalog.menu_commands_for_channel("telegram");
    assert_eq!(telegram.len(), 2);
    let whatsapp = catalog.menu_commands_for_channel("whatsapp");
    assert!(whatsapp.is_empty());
}

#[test]
fn duplicate_alias_on_overlapping_channels_is_rejected() {
    let raw = r#"
[[commands]]
name = "start"
kind = "core"
core_action = "start"
channels = ["telegram"]

[[commands]]
name = "begin"
aliases = ["start"]
kind = "core"
core_action = "cancel"
channels = ["telegram", "whatsapp"]
"#;

    let err = ChannelCommandCatalog::from_toml_str(raw).expect_err("duplicate should fail");
    assert!(err.contains("both bind `start`"));
}

#[test]
fn menu_channel_must_be_supported_by_command_channel_set() {
    let raw = r#"
[[commands]]
name = "start"
kind = "core"
core_action = "start"
channels = ["whatsapp"]
menu_channels = ["telegram"]
"#;

    let err = ChannelCommandCatalog::from_toml_str(raw).expect_err("menu channel should fail");
    assert!(err.contains("menu channel `telegram` outside supported channels"));
}

#[test]
fn slash_prefixed_paths_and_non_whitespace_suffixes_are_not_commands() {
    let raw = r#"
[[commands]]
name = "run"
kind = "core"
core_action = "run_skill"
channels = ["telegram", "whatsapp"]

[[commands]]
name = "start"
kind = "core"
core_action = "start"
channels = ["telegram", "whatsapp"]
"#;

    let catalog = ChannelCommandCatalog::from_toml_str(raw).expect("parse catalog");
    assert!(catalog
        .match_command("/home/testuser/project", "telegram")
        .is_none());
    assert!(catalog.match_command("/run/logs", "telegram").is_none());
    assert!(catalog.match_command("/start/docs", "telegram").is_none());
    assert!(catalog.match_command("/run logs", "telegram").is_some());
}
