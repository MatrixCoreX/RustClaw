use claw_core::types::AuthIdentity;

use super::*;

#[test]
fn ui_auth_failures_expose_stable_machine_codes() {
    let (status, Json(response)) = ui_auth_error("auth_key_required");
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(response.error.as_deref(), Some("auth_key_required"));
    assert_eq!(
        response
            .data
            .as_ref()
            .and_then(|value| value.get("error_code"))
            .and_then(Value::as_str),
        Some("auth_key_required")
    );

    let (status, Json(response)) = ui_auth_code_error::<AuthIdentity>("auth_key_invalid");
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(response.error.as_deref(), Some("auth_key_invalid"));
    assert!(response.data.is_none());
}
