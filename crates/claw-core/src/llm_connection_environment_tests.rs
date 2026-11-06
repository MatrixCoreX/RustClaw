use super::*;

fn scope(
    vendor: Option<&str>,
    protocol: Option<&str>,
    relay: Option<&str>,
) -> ConnectionEnvironment {
    ConnectionEnvironment::from_env_with(|name| {
        match name {
            SELECTED_VENDOR_ENV => vendor,
            SELECTED_PROTOCOL_ENV => protocol,
            SELECTED_RELAY_ENV => relay,
            _ => panic!("identity parser must never request credential values"),
        }
        .map(str::to_string)
    })
}

#[test]
fn standalone_environment_keeps_canonical_credentials_without_relay_authority() {
    let value = scope(None, None, None);
    assert_eq!(value, ConnectionEnvironment::Standalone);
    assert!(value.permits_vendor_credential("minimax"));
    assert!(value.permits_vendor_credential("openai"));
    assert!(!value.is_enrolled_relay());
}

#[test]
fn selected_protocol_alias_is_not_another_vendor_identity() {
    for vendor in ["minimax", "qwen", "mimo", "custom"] {
        let value = scope(Some(vendor), Some("openai_compat"), Some("false"));
        assert!(value.permits_vendor_credential(vendor));
        assert!(!value.permits_vendor_credential("openai"));
        assert!(!value.is_enrolled_relay());
    }
    assert!(scope(Some("openai"), Some("openai_compat"), None).permits_vendor_credential("openai"));
}

#[test]
fn relay_identity_allows_only_its_declared_native_media_alias() {
    let value = scope(Some("custom"), Some("openai_compat"), Some("true"));
    assert!(value.is_enrolled_relay());
    assert!(value.permits_vendor_credential("custom"));
    assert!(value.permits_vendor_credential("minimax"));
    for vendor in ["openai", "qwen", "google", "anthropic"] {
        assert!(!value.permits_vendor_credential(vendor));
    }
}

#[test]
fn incomplete_or_unknown_metadata_never_becomes_standalone() {
    for value in [
        scope(Some("minimax"), None, None),
        scope(None, Some("openai_compat"), None),
        scope(None, None, Some("true")),
        scope(Some("minimax"), Some("unknown"), Some("false")),
        scope(Some("minimax"), Some("openai_compat"), Some("unknown")),
        scope(Some("minimax"), Some("anthropic_claude"), Some("true")),
    ] {
        assert_eq!(value, ConnectionEnvironment::Invalid);
        assert!(!value.permits_vendor_credential("minimax"));
        assert!(!value.permits_vendor_credential("openai"));
        assert!(!value.is_enrolled_relay());
    }
}

#[test]
fn supported_native_protocols_do_not_imply_enrollment() {
    for (vendor, protocol) in [
        ("google", "google_gemini"),
        ("anthropic", "anthropic_claude"),
    ] {
        let value = scope(Some(vendor), Some(protocol), Some("false"));
        assert!(value.permits_vendor_credential(vendor));
        assert!(!value.permits_vendor_credential("minimax"));
        assert!(!value.is_enrolled_relay());
    }
}

#[test]
fn identity_parsing_normalizes_whitespace_without_reading_keys_or_urls() {
    let value = scope(Some(" MiniMax "), Some(" openai_compat "), Some(" true "));
    assert!(value.permits_vendor_credential("minimax"));
    assert!(value.is_enrolled_relay());
    assert_eq!(
        scope(Some(""), Some(" "), None),
        ConnectionEnvironment::Standalone
    );
}

fn receipt_command() -> std::process::Command {
    let mut command = std::process::Command::new("unused-test-command");
    for name in SELECTED_IDENTITY_ENV {
        command.env(name, "untrusted-receipt-value");
    }
    command
}

#[test]
fn declared_identity_uses_host_values_instead_of_receipt_fixed_values() {
    let mut command = receipt_command();
    let declared = SELECTED_IDENTITY_ENV.map(str::to_string);
    apply_host_identity_environment(&mut command, &declared, |name| Some(name.into()));
    let values: std::collections::BTreeMap<_, _> = command.get_envs().collect();
    for name in SELECTED_IDENTITY_ENV {
        assert_eq!(
            values[std::ffi::OsStr::new(name)],
            Some(std::ffi::OsStr::new(name))
        );
    }
}

#[test]
fn undeclared_identity_is_not_forwarded_even_when_host_and_receipt_supply_it() {
    let mut command = receipt_command();
    apply_host_identity_environment(&mut command, &[], |_| Some("host-value".into()));
    for (_, value) in command.get_envs() {
        assert!(value.is_none());
    }
}

#[test]
fn absent_host_identity_cannot_be_replaced_by_a_receipt() {
    let mut command = receipt_command();
    let declared = SELECTED_IDENTITY_ENV.map(str::to_string);
    apply_host_identity_environment(&mut command, &declared, |_| None);
    for (_, value) in command.get_envs() {
        assert!(value.is_none());
    }
}
