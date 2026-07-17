use super::*;
use clap::CommandFactory;

#[test]
fn locale_catalog_selects_chinese_from_machine_locale_tokens() {
    assert_eq!(
        message_for_locale("tui.unknown_key", Some("zh_CN.UTF-8")),
        "无法识别这个操作键。"
    );
    assert_eq!(list_for_locale("tui.help", Some("zh-CN")).len(), 2);
}

#[test]
fn locale_catalog_falls_back_to_english_for_unknown_locale_and_key() {
    assert_eq!(
        message_for_locale("tui.unknown_key", Some("fr-FR")),
        "Unknown command key."
    );
    assert_eq!(
        message_for_locale("missing.machine.key", Some("zh-CN")),
        "missing.machine.key"
    );
}

#[test]
fn locale_catalogs_keep_the_same_interface_keys() {
    let en = catalog(CatalogKind::En);
    let zh = catalog(CatalogKind::ZhCn);
    assert_eq!(
        en.messages.keys().collect::<Vec<_>>(),
        zh.messages.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        en.lists.keys().collect::<Vec<_>>(),
        zh.lists.keys().collect::<Vec<_>>()
    );
}

#[test]
fn locale_catalogs_cover_every_top_level_command() {
    let en = catalog(CatalogKind::En);
    let zh = catalog(CatalogKind::ZhCn);
    for command in crate::Cli::command().get_subcommands() {
        let key = format!("command.{}", command.get_name());
        assert!(en.messages.contains_key(&key), "missing English {key}");
        assert!(zh.messages.contains_key(&key), "missing Chinese {key}");
    }
}
