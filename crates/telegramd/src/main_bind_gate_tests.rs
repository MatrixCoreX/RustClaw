use super::{extract_bind_key_candidate, TextCatalog};
use std::path::Path;

#[test]
fn unbound_plain_text_requires_key_binding() {
    assert_eq!(
        extract_bind_key_candidate("hello agent-runtime", false),
        None
    );
}

#[test]
fn unbound_key_command_is_accepted_for_binding() {
    assert_eq!(
        extract_bind_key_candidate("/key rk_live_123", false).as_deref(),
        Some("rk_live_123")
    );
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
    assert_eq!(extract_bind_key_candidate("/status now", true), None);
    assert_eq!(extract_bind_key_candidate("/cancel all", true), None);
}

#[test]
fn unbound_media_like_empty_text_requires_binding_prompt() {
    assert_eq!(extract_bind_key_candidate("", false), None);
}

#[test]
fn binding_i18n_keys_are_locale_specific_with_safe_fallback() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let zh = TextCatalog::load(
        root.join("configs/i18n/telegramd.zh-CN.toml")
            .to_string_lossy()
            .as_ref(),
    )
    .expect("load zh telegram i18n");
    let en = TextCatalog::load(
        root.join("configs/i18n/telegramd.en-US.toml")
            .to_string_lossy()
            .as_ref(),
    )
    .expect("load en telegram i18n");

    assert!(zh.t("telegram.msg.bind_success").contains("绑定成功"));
    assert!(!zh
        .t("telegram.msg.bind_key_required_for_chat")
        .contains("Please send"));
    assert!(en.t("telegram.msg.bind_success").contains("Key bound"));
    assert!(!en
        .t("telegram.msg.bind_key_required_for_chat")
        .contains("请先"));
    let missing_path = root.join("configs/i18n/telegramd.zh-CN.missing.toml");
    let fallback = TextCatalog::fallback(missing_path.to_string_lossy().as_ref())
        .t("telegram.msg.bind_success");
    assert_eq!(
        fallback,
        claw_core::channel_i18n::safe_generic_text_for_locale("zh-CN")
    );
    assert!(!fallback.contains("telegram.msg.bind_success"));
}
