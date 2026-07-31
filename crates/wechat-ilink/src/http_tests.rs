use claw_core::channel_provider_error::ChannelProviderFailureClass;
use serde_json::json;

use super::decode_ilink_provider_failure;

#[test]
fn http_200_session_timeout_is_a_typed_authentication_failure() {
    let error = decode_ilink_provider_failure(
        "sendmessage",
        &json!({"ret": -14, "errmsg": "must not escape"}),
    )
    .expect("provider failure");

    assert_eq!(
        error.failure_class,
        ChannelProviderFailureClass::Authentication
    );
    assert_eq!(error.provider_error_code.as_deref(), Some("-14"));
    assert!(!error.to_string().contains("must not escape"));
}

#[test]
fn zero_or_absent_provider_codes_are_success() {
    for value in [json!({}), json!({"ret": 0}), json!({"errcode": 0})] {
        assert!(decode_ilink_provider_failure("sendmessage", &value).is_none());
    }
}
