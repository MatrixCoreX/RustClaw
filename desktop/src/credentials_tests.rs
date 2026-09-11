use super::LoginSecret;
use serde_json::json;

#[test]
fn prefill_exposes_only_login_display_fields() {
    for mode in ["password", "key"] {
        let input = LoginSecret {
            mode: mode.into(),
            username: "tester".into(),
            secret: "fixture-secret-never-crosses-ipc".into(),
        };
        assert_eq!(
            serde_json::to_value(input.prefill().unwrap()).unwrap(),
            json!({"mode":mode,"username":if mode == "password" {"tester"} else {""}})
        );
    }
}

#[test]
fn unusable_saved_credentials_cannot_fill_a_login_form() {
    for (mode, username, secret) in [
        ("unknown", "tester", "fixture-secret"),
        ("password", "", "fixture-secret"),
        ("key", "", ""),
    ] {
        let input = LoginSecret {
            mode: mode.into(),
            username: username.into(),
            secret: secret.into(),
        };
        assert!(input.prefill().is_err());
    }
}
