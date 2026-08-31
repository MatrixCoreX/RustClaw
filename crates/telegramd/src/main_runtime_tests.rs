use super::*;

#[test]
fn telegram_media_path_is_stable_for_provider_message() {
    let first = build_telegram_inbox_rel_path("data/inbox", "primary", 10, 20, "message-30", "jpg");
    let replay =
        build_telegram_inbox_rel_path("data/inbox", "primary", 10, 20, "message-30", "jpg");
    assert_eq!(first, replay);
    assert!(first.ends_with("/message-30.jpg"));
}

#[test]
fn telegram_api_retry_delay_uses_bounded_exponential_backoff() {
    let seconds = (0..=6)
        .map(|attempt| telegram_api_retry_delay(attempt).as_secs())
        .collect::<Vec<_>>();
    assert_eq!(seconds, vec![5, 10, 20, 40, 60, 60, 60]);
}

#[test]
fn telegram_menu_language_code_uses_iso_639_primary_subtag() {
    assert_eq!(telegram_menu_language_code("zh-CN").as_deref(), Some("zh"));
    assert_eq!(telegram_menu_language_code("EN_us").as_deref(), Some("en"));
    assert_eq!(telegram_menu_language_code("  ja  ").as_deref(), Some("ja"));
    assert_eq!(
        telegram_menu_language_code("zh-Hans").as_deref(),
        Some("zh")
    );
    assert_eq!(telegram_menu_language_code("eng").as_deref(), None);
    assert_eq!(telegram_menu_language_code("").as_deref(), None);
}

#[test]
fn telegram_menu_registers_default_and_localized_command_scopes() {
    let mut current = HashMap::new();
    current.insert(
        "telegram.menu.help_desc".to_string(),
        "Localized help".to_string(),
    );
    let i18n = TextCatalog {
        current,
        safe_fallback: "Safe fallback".to_string(),
    };
    let catalog = ChannelCommandCatalog::default();

    let payloads = telegram_command_payloads("zh-CN", &i18n, &catalog);
    assert_eq!(payloads.len(), 2);
    assert!(payloads[0].get("language_code").is_none());
    assert_eq!(payloads[1]["language_code"], "zh");
    assert_eq!(payloads[0]["commands"], payloads[1]["commands"]);
    assert_eq!(payloads[0]["commands"][0]["command"], "help");
    assert_eq!(payloads[0]["commands"][0]["description"], "Localized help");
}
