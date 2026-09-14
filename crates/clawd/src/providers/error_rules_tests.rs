use super::*;

#[test]
fn builtin_rules_are_valid_and_profiles_are_isolated() {
    let rules = ErrorRules::parse(BUILTIN).unwrap();
    assert!(rules.profiles.iter().any(|profile| profile.id == "default"));
    assert_eq!(
        rules.profile("custom", "https://api.minimaxi.com/v1").id,
        "minimax"
    );
    assert_eq!(
        rules.profile("vendor-minimax", "http://127.0.0.1:1234").id,
        "minimax"
    );
    assert_eq!(
        rules.profile("vendor-minimax", "https://api.openai.com").id,
        "openai"
    );
    assert_eq!(
        rules
            .profile("custom", "https://api.minimaxi.com.attacker.invalid")
            .id,
        "default"
    );
    assert_eq!(
        rules.profile("custom", "https://notminimaxi.com").id,
        "default"
    );
    assert_eq!(
        rules
            .profile("custom", "https://api.openai.com@attacker.invalid")
            .id,
        "default"
    );
}

#[test]
fn invalid_configuration_is_rejected_not_silently_defaulted() {
    for raw in [
        BUILTIN.replace("schema_version = 1", "schema_version = 999"),
        BUILTIN.replace("revision =", "misspelled_revision ="),
        BUILTIN.replace("id = \"minimax\"", "id = \"default\""),
        BUILTIN.replace("class = \"quota_exhausted\"", "class = \"retry_forever\""),
        BUILTIN.replace("\"/error/code\"", "\"error/code\""),
        BUILTIN.replace("\"/error/code\"", "\"/error/~9code\""),
        BUILTIN.replace(
            "message_code_http_statuses = [429]",
            "message_code_http_statuses = []",
        ),
        format!("{BUILTIN}\n[[bindings]]\nprovider_name = 'custom'\nprofile = 'missing'"),
    ] {
        assert!(ErrorRules::parse(&raw).is_err());
    }
}

#[test]
fn an_operator_can_bind_a_relay_or_add_a_provider_without_rust_changes() {
    let raw = format!(
        "{BUILTIN}\n\
        [[bindings]]\nprovider_name = 'custom'\nprofile = 'new_vendor'\n\
        [[profiles]]\nid = 'new_vendor'\nhosts = ['llm.example.invalid']\n\
        sources = ['https://docs.example.invalid/errors']\n\
        code_paths = ['/problem/reason']\nfailure_code_paths = ['/problem/reason']\n\
        success_codes = ['DONE']\n\
        [[profiles.rules]]\nid = 'new_quota'\nclass = 'quota_exhausted'\n\
        codes = ['NEW_BALANCE_EMPTY']\nhttp_statuses = [200, 403]\n"
    );
    let rules = ErrorRules::parse(&raw).unwrap();
    assert!(super::super::error_classification::classify(
        &rules,
        "custom",
        "http://localhost",
        200,
        &serde_json::json!({"problem":{"reason":"DONE"}}),
    )
    .is_none());
    assert_eq!(
        rules.profile("custom", "https://api.openai.com").id,
        "new_vendor"
    );
    let found = super::super::error_classification::classify(
        &rules,
        "custom",
        "http://localhost",
        200,
        &serde_json::json!({"problem":{"reason":"NEW_BALANCE_EMPTY"}}),
    )
    .unwrap();
    assert_eq!(found.kind, ProviderErrorKind::QuotaExhausted);
    let found = super::super::error_classification::classify(
        &rules,
        "custom",
        "http://localhost",
        401,
        &serde_json::json!({"problem":{"reason":"NEW_BALANCE_EMPTY"}}),
    )
    .unwrap();
    assert_eq!(found.kind, ProviderErrorKind::ProviderNonRetryableBusiness);
}

#[test]
fn missing_explicit_configuration_fails_but_standalone_has_embedded_rules() {
    let temp = std::env::temp_dir().join(format!("agent-error-rules-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&temp).unwrap();
    let missing = temp.join("missing.toml");
    assert!(load(&missing, true).is_err());
    assert_eq!(load(&missing, false).unwrap().revision, "2026-09-14.1");
    let invalid = temp.join("invalid.toml");
    std::fs::write(&invalid, "not valid toml").unwrap();
    assert!(load(&invalid, false).is_err());
    std::fs::remove_dir_all(temp).unwrap();
}
