use serde_json::Value;

use super::build_hook_admin_status;
use crate::agent_hooks::shared::{HookHandlerConfig, LoadedHookConfiguration};

fn handler(id: &str, enabled: bool) -> HookHandlerConfig {
    HookHandlerConfig {
        id: id.to_string(),
        stage: "pre_tool_use".to_string(),
        kind: "command".to_string(),
        enabled,
        trusted: enabled,
        blocking: true,
        path: "hooks/fixture".to_string(),
        args: vec!["must-not-reach-api".to_string()],
        content_sha256: "sha256:fixture".to_string(),
        url: "https://secret.example.invalid/token".to_string(),
        auth_token_env: Some("HOOK_TOKEN_ENV".to_string()),
        ..HookHandlerConfig::default()
    }
}

#[test]
fn admin_status_defaults_safe_and_redacts_handler_values() {
    let status = build_hook_admin_status(
        LoadedHookConfiguration {
            handlers: vec![handler("disabled_fixture", false)],
            error_code: None,
        },
        |_| panic!("disabled handler must not be validated"),
    );

    assert_eq!(status["setup_state"], "configured_disabled");
    assert_eq!(status["enabled"], false);
    assert_eq!(status["handler_count"], 1);
    assert_eq!(status["enabled_handler_count"], 0);
    assert_eq!(status["invalid_handler_count"], 0);
    assert_eq!(status["setup"]["ui_enable_supported"], false);
    assert_eq!(status["handlers"][0]["status"], "disabled");
    assert_eq!(
        status["handlers"][0]["redacted_config"]["argument_count"],
        1
    );
    assert_eq!(
        status["handlers"][0]["redacted_config"]["arguments_redacted"],
        true
    );
    assert_eq!(
        status["handlers"][0]["redacted_config"]["url_configured"],
        true
    );
    let encoded = serde_json::to_string(&status).expect("encode status");
    assert!(!encoded.contains("must-not-reach-api"));
    assert!(!encoded.contains("secret.example.invalid"));
    assert!(!encoded.contains("HOOK_TOKEN_ENV"));
}

#[test]
fn admin_status_reports_enabled_validation_and_config_errors() {
    let status = build_hook_admin_status(
        LoadedHookConfiguration {
            handlers: vec![handler("ready", true), handler("invalid", true)],
            error_code: None,
        },
        |handler| {
            if handler.id == "ready" {
                Ok(())
            } else {
                Err("hook_handler_hash_mismatch")
            }
        },
    );

    assert_eq!(status["setup_state"], "configured_invalid");
    assert_eq!(status["fail_closed"], true);
    assert_eq!(status["enabled_handler_count"], 2);
    assert_eq!(status["valid_handler_count"], 1);
    assert_eq!(status["invalid_handler_count"], 1);
    assert_eq!(
        status["handlers"][1]["error_code"],
        "hook_handler_hash_mismatch"
    );
    assert_eq!(
        status["supported_stages"]
            .as_array()
            .map(Vec::len)
            .unwrap_or_default(),
        11
    );

    let error = build_hook_admin_status(
        LoadedHookConfiguration {
            handlers: Vec::new(),
            error_code: Some("hook_config_parse_failed"),
        },
        |_: &HookHandlerConfig| -> Result<(), &'static str> { Ok(()) },
    );
    assert_eq!(error["setup_state"], "configuration_error");
    assert_eq!(error["fail_closed"], true);
    assert_eq!(error["config_error_code"], "hook_config_parse_failed");
    assert!(matches!(error["handlers"], Value::Array(_)));
}
