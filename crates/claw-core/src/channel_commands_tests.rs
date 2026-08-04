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
name = "cancel"
kind = "core"
core_action = "cancel"
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
    assert!(!catalog.allows_unbound_command("/cancel", "telegram"));
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
fn default_telegram_commands_only_expose_transport_controls() {
    let catalog = ChannelCommandCatalog::default();
    let menu_names = catalog
        .menu_commands_for_channel("telegram")
        .into_iter()
        .map(|command| command.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(menu_names, vec!["help", "cancel"]);

    for removed in [
        "/ask hello",
        "/run weather {}",
        "/skills",
        "/sendfile report.txt",
        "/agent-runtime show",
        "/cryptoapi show",
    ] {
        assert!(
            catalog.match_command(removed, "telegram").is_none(),
            "removed Telegram command unexpectedly matched: {removed}"
        );
    }
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
name = "cancel"
kind = "core"
core_action = "cancel"
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
    assert!(catalog.match_command("/cancel/logs", "telegram").is_none());
    assert!(catalog.match_command("/start/docs", "telegram").is_none());
    assert!(catalog.match_command("/cancel now", "telegram").is_some());
}

#[test]
fn skill_command_schema_is_permanently_rejected() {
    for raw in [
        r#"[[commands]]
name = "run"
kind = "skill"
core_action = "cancel"
channels = ["telegram"]
"#,
        r#"[[commands]]
name = "run"
kind = "core"
core_action = "cancel"
skill_name = "weather"
channels = ["telegram"]
"#,
        r#"[[commands]]
name = "run"
kind = "core"
core_action = "run_skill"
channels = ["telegram"]
"#,
    ] {
        assert!(
            ChannelCommandCatalog::from_toml_str(raw).is_err(),
            "skill-linked command schema unexpectedly accepted: {raw}"
        );
    }
}

#[test]
fn command_examples_inside_ordinary_text_do_not_enter_the_control_plane() {
    let catalog = ChannelCommandCatalog::from_toml_str(SAMPLE).expect("parse catalog");

    for ordinary_text in [
        "Example: /start",
        "please type /start",
        "> /start",
        "```text\n/start\n```",
        "first read this\n/start",
    ] {
        assert!(
            catalog.match_command(ordinary_text, "telegram").is_none(),
            "ordinary text unexpectedly matched as a command: {ordinary_text:?}"
        );
    }
    assert!(catalog.match_command("/start", "telegram").is_some());
}

#[test]
fn command_catalog_digest_is_stable_and_transport_only() {
    let first = ChannelCommandCatalog::default();
    let second = ChannelCommandCatalog::default();
    assert_eq!(first.digest(), second.digest());
    assert_eq!(first.digest().len(), 64);
    assert!(first
        .commands()
        .iter()
        .all(|command| command.core_action().is_some()));
}
