use super::*;

#[test]
fn claim_contract_validates_machine_fields() {
    let request = ChannelEventClaimRequest::new(
        ChannelKind::Feishu,
        "app-id",
        "event-id",
        br#"{"event":"value"}"#,
    );
    assert!(request.validate().is_ok());

    let mut invalid = request.clone();
    invalid.provider_event_id = "\n".to_string();
    assert_eq!(
        invalid.validate(),
        Err(ChannelEventAdmissionValidationError::Identifier)
    );

    let mut invalid = request;
    invalid.payload_sha256 = "not-a-digest".to_string();
    assert_eq!(
        invalid.validate(),
        Err(ChannelEventAdmissionValidationError::Digest)
    );
}

#[test]
fn signed_admission_body_detects_tampering() {
    let body = br#"{"provider_event_id":"event-1"}"#;
    let signature = sign_admission_request("secret", 100, body).expect("sign");
    assert!(verify_admission_request_signature(
        "secret", 100, body, &signature
    ));
    assert!(!verify_admission_request_signature(
        "secret",
        100,
        br#"{"provider_event_id":"event-2"}"#,
        &signature
    ));
    assert!(!verify_admission_request_signature(
        "secret", 101, body, &signature
    ));
    assert!(!verify_admission_request_signature(
        "wrong", 100, body, &signature
    ));
}

#[test]
fn finish_contract_rejects_unknown_json_fields() {
    let value = serde_json::json!({
        "schema_version": CHANNEL_EVENT_ADMISSION_SCHEMA_VERSION,
        "channel": "feishu",
        "account_id": "app-id",
        "provider_event_id": "event-id",
        "payload_sha256": "0".repeat(64),
        "lease_token": "lease",
        "outcome": "completed",
        "unexpected": true
    });
    assert!(serde_json::from_value::<ChannelEventFinishRequest>(value).is_err());
}
