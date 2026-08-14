use super::*;

fn request_for(action: &str, extra: &str) -> String {
    format!(
        r#"{{"request_id":"nni-test","args":{{"action":"{action}"{extra}}},"user_id":1,"chat_id":2,"context":null}}"#
    )
}

#[test]
fn all_supported_actions_have_stable_tokens() {
    let actions = [
        "status",
        "device_status",
        "heartbeat_status",
        "heartbeat_enable",
        "heartbeat_disable",
        "heartbeat_now",
        "network_stats",
        "my_rewards",
        "bancor_market",
        "bancor_account",
        "bancor_market_trades",
        "bancor_candles",
        "bancor_quote",
    ];
    for token in actions {
        let request: Request = serde_json::from_str(&request_for(token, ""))
            .unwrap_or_else(|error| panic!("decode {token}: {error}"));
        assert_eq!(request.args.action.as_str(), token);
    }
}

#[test]
fn arguments_reject_unknown_fields_and_actions() {
    assert!(
        serde_json::from_str::<Request>(&request_for("status", r#", "language":"zh""#)).is_err()
    );
    assert!(serde_json::from_str::<Request>(&request_for("trade", "")).is_err());
}

#[test]
fn runtime_memory_context_is_accepted_but_not_forwarded() {
    let request: Request = serde_json::from_str(&request_for(
        "status",
        r#", "_memory":{"context":"runtime-owned"}"#,
    ))
    .expect("decode runtime memory context");
    let forwarded = serde_json::to_value(&request.args).expect("serialize gateway arguments");
    assert_eq!(forwarded["action"], "status");
    assert!(forwarded.get("_memory").is_none());
}

#[test]
fn quote_arguments_preserve_decimal_strings() {
    let request: Request = serde_json::from_str(&request_for(
        "bancor_quote",
        r#", "side":"buy", "pay_asset":"USD", "pay_amount":"12.3400", "slippage_bps":50"#,
    ))
    .expect("decode quote request");
    assert_eq!(request.args.pay_amount.as_deref(), Some("12.3400"));
    assert_eq!(request.args.slippage_bps, Some(50));
    assert!(request.args.validate().is_ok());
}

#[test]
fn action_specific_fields_are_enforced() {
    let status: Request = serde_json::from_str(&request_for("status", r#", "limit":1"#))
        .expect("decode known but irrelevant field");
    let error = status
        .args
        .validate()
        .expect_err("status must reject limit");
    assert_eq!(error.code, "nni_argument_invalid");

    let quote: Request =
        serde_json::from_str(&request_for("bancor_quote", "")).expect("decode incomplete quote");
    let error = quote
        .args
        .validate()
        .expect_err("quote requires side and amount");
    assert_eq!(
        error.details["missing_fields"],
        json!(["side", "pay_amount"])
    );
}

#[test]
fn success_envelope_requires_matching_action_and_source() {
    let valid = json!({
        "schema_version": 1,
        "source_skill": "nni",
        "status": "ok",
        "action": "device_status",
        "observed_at_ts": 1,
        "data": {"signer_kind": "unavailable"},
    });
    assert!(validate_success_envelope(&valid, Action::DeviceStatus).is_ok());
    assert!(validate_success_envelope(&valid, Action::Status).is_err());
    let wrong_source = json!({
        "schema_version": 1,
        "source_skill": "other",
        "status": "ok",
        "action": "device_status",
        "data": {},
    });
    assert!(validate_success_envelope(&wrong_source, Action::DeviceStatus).is_err());
}

#[test]
fn errors_use_canonical_machine_fields() {
    let response = SkillError::new(
        "nni_signature_device_unavailable",
        false,
        json!({"signer_kind": "unavailable"}),
    )
    .into_response("request-1".to_string(), Some(Action::HeartbeatEnable));
    let extra = response.extra.expect("error extra");
    assert_eq!(response.status, "error");
    assert_eq!(
        extra.get("error_code").and_then(Value::as_str),
        Some("nni_signature_device_unavailable")
    );
    assert_eq!(extra.get("retryable").and_then(Value::as_bool), Some(false));
    assert_eq!(extra["failure_phase"], "pre_dispatch");
    assert_eq!(extra["side_effect_applied"], false);
    assert_eq!(extra["recovery_action"], "replan_arguments");
    assert!(extra.get("error_kind").is_none());
}

#[test]
fn gateway_effect_evidence_is_forwarded_without_interpreting_error_text() {
    let envelope = json!({
        "error_code": "nni_device_not_authorized",
        "retryable": false,
        "failure_phase": "provider_rejected",
        "side_effect_applied": false,
        "recovery_action": null,
        "details": {"runtime_error_code": "nni_pubkey_not_allowlisted"},
    });
    let response = SkillError::from_gateway(
        "nni_device_not_authorized",
        false,
        envelope["details"].clone(),
        &envelope,
    )
    .into_response("request-2".to_string(), Some(Action::HeartbeatEnable));
    let extra = response.extra.expect("error extra");
    assert_eq!(extra["failure_phase"], "provider_rejected");
    assert_eq!(extra["side_effect_applied"], false);
    assert!(extra["recovery_action"].is_null());
}

#[test]
fn gateway_prose_never_becomes_a_machine_error_code() {
    let error = gateway_error_from_response(
        reqwest::StatusCode::BAD_GATEWAY,
        ApiResponse {
            ok: false,
            data: None,
            error: Some("localized gateway detail".to_string()),
        },
        Action::BancorMarket,
    );
    assert_eq!(error.code, "nni_internal_request_failed");
    assert!(error.retryable);
    assert_eq!(error.details["gateway_error"], "localized gateway detail");
    assert_eq!(error.failure_phase.as_deref(), Some("pre_dispatch"));
    assert_eq!(error.side_effect_applied, Some(false));
}

#[test]
fn mutating_transport_failures_remain_uncertain() {
    let error = response_contract_error(
        Action::HeartbeatNow,
        "nni_internal_gateway_unavailable",
        true,
        json!({}),
    );
    assert_eq!(error.failure_phase, None);
    assert_eq!(error.side_effect_applied, None);
    assert_eq!(
        error.recovery_action.as_deref(),
        Some("reconcile_before_retry")
    );
}

#[tokio::test]
async fn invalid_args_preserve_the_protocol_request_id() {
    let response = handle_line(
        r#"{"request_id":"smoke-request","args":{},"user_id":1,"chat_id":2,"context":null}"#,
    )
    .await;
    assert_eq!(response.request_id, "smoke-request");
    assert_eq!(response.status, "error");
    assert_eq!(
        response.extra.expect("error envelope")["error_code"],
        "nni_invalid_input"
    );
}
