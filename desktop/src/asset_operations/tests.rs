use super::{protocol::*, *};
use serde_json::{json, Value};

fn fixture() -> (Capabilities, String, Uuid, Intent, Value) {
    let cap = Capabilities {
        schema_version: 1,
        protocol: PROTOCOL.into(),
        ledger_id: "ledger-fixture-v1".into(),
        node_url: "https://ledger.example.test".into(),
        service: Service::Bancor,
        actions: vec!["balances".into(), "bancor_trade".into()],
    };
    let account = crate::wallet::keys::public(&[1; 32]).unwrap();
    let id = Uuid::new_v4();
    let intent = Intent::BancorTrade {
        side: "buy".into(),
        input_units: "100000000".into(),
        slippage_bps: 300,
        max_fee_bps: 100,
    };
    let payload = json!({"schema_version":1,"protocol":PROTOCOL,"ledger_id":cap.ledger_id,"node_url":cap.node_url,"service":"bancor","account":account,"operation_id":id,"challenge_id":Uuid::new_v4(),"nonce":"ab".repeat(32),"expires_at_unix":1100,"terms":{"kind":"bancor_trade","side":"buy","input_units":"100000000","slippage_bps":300,"max_fee_bps":100,"fee_units":"1000000","quoted_output_units":"200000000","min_output_units":"194000000"}});
    (cap, account, id, intent, payload)
}

#[test]
fn exact_challenge_binding_rejects_mutations_and_ambiguous_json() {
    let (cap, account, id, intent, p) = fixture();
    validate_challenge(&p.to_string(), &cap, &account, id, &intent, 1000).unwrap();
    for (field, value) in [
        ("ledger_id", json!("other-ledger")),
        ("node_url", json!("https://other.example.test")),
        (
            "account",
            json!(crate::wallet::keys::public(&[2; 32]).unwrap()),
        ),
        ("service", json!("assets")),
        ("operation_id", json!(Uuid::new_v4())),
        ("expires_at_unix", json!(1000)),
        ("expires_at_unix", json!(1301)),
        ("nonce", json!("AB".repeat(32))),
        ("unknown", json!(true)),
    ] {
        let mut changed = p.clone();
        changed[field] = value;
        assert!(
            validate_challenge(&changed.to_string(), &cap, &account, id, &intent, 1000).is_err(),
            "{field}"
        );
    }
    for (field, value) in [
        ("side", json!("sell")),
        ("input_units", json!("200000000")),
        ("fee_units", json!("1000001")),
        ("min_output_units", json!("193999999")),
        ("kind", json!("transfer")),
        ("extra", json!(true)),
    ] {
        let mut changed = p.clone();
        changed["terms"][field] = value;
        assert!(
            validate_challenge(&changed.to_string(), &cap, &account, id, &intent, 1000).is_err(),
            "{field}"
        );
    }
    let duplicate = p.to_string().replacen('{', "{\"schema_version\":1,", 1);
    assert!(validate_challenge(&duplicate, &cap, &account, id, &intent, 1000).is_err());
}

#[test]
fn amounts_transfer_limits_and_service_boundaries() {
    for invalid in ["", "0.1", "01", "-1", "1e8", "9223372036854775808"] {
        assert!(units(invalid).is_err());
    }
    assert_eq!(units("9223372036854775807").unwrap(), i64::MAX as u64);
    assert!(positive_units("0").is_err());
    let recipient = crate::wallet::keys::public(&[2; 32]).unwrap();
    let intent = Intent::Transfer {
        asset: "AIC".into(),
        amount_units: "100".into(),
        recipient: recipient.clone(),
        memo: "memo".into(),
        max_fee_bps: 0,
    };
    intent.validate(Service::Assets, "different-owner").unwrap();
    assert!(intent.validate(Service::Bancor, "different-owner").is_err());
    assert!(intent.validate(Service::Assets, &recipient).is_err());
    let (mut cap, _, _, _, _) = fixture();
    cap.node_url = "http://192.168.1.2".into();
    assert!(cap.validate(Service::Bancor, "balances").is_err());
    cap.node_url = "http://127.0.0.1:8080".into();
    cap.validate(Service::Bancor, "balances").unwrap();
    cap.node_url = "https://user:secret@example.test".into();
    assert!(cap.validate(Service::Bancor, "balances").is_err());
}

#[test]
fn results_and_persistent_unknown_operations_keep_owner_binding() {
    let (cap, account, id, intent, value) = fixture();
    let payload =
        validate_challenge(&value.to_string(), &cap, &account, id, &intent, 1000).unwrap();
    let mut outcome = Outcome {
        operation_id: id,
        account: account.clone(),
        ledger_id: cap.ledger_id.clone(),
        status: "pending".into(),
        receipt_id: None,
    };
    outcome.validate(id, &account, &cap.ledger_id).unwrap();
    outcome.account = "wrong-owner".into();
    assert!(outcome.validate(id, &account, &cap.ledger_id).is_err());
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("operations.json");
    let pending = Pending {
        generation: 1,
        session_id: Uuid::new_v4(),
        profile_id: Uuid::new_v4(),
        account: Account {
            id: Uuid::new_v4(),
            name: "test".into(),
            public_key: account,
            backed_up: true,
        },
        cap,
        intent,
        payload,
        bytes: value.to_string(),
        device_label: "fixture".into(),
        origin: "https://localhost".into(),
    };
    let mut ops = Operations::new(path.clone()).unwrap();
    ops.mark_submitted(&pending).unwrap();
    let reopened = Operations::new(path.clone()).unwrap();
    assert_eq!(reopened.records[0].status, "pending");
    assert!(reopened.pending.is_none());
    let stored = std::fs::read_to_string(path).unwrap();
    assert!(!stored.contains("signature"));
    assert!(!stored.contains("nonce"));
}

#[test]
fn typed_errors_keep_submitted_operations_unknown_and_never_suggest_duplicate_submission() {
    use super::client::{backend_error, submission_error};
    assert_eq!(backend_error(429, None), "wallet_rate_limited");
    assert_eq!(
        backend_error(503, Some("asset_owner_unavailable")),
        "wallet_network_failed"
    );
    assert_eq!(
        backend_error(404, Some("asset_owner_operation_not_found")),
        "wallet_backend_rejected"
    );
    assert_eq!(backend_error(404, None), "wallet_backend_unsupported");
    assert_eq!(
        backend_error(404, Some("unsupported")),
        "wallet_backend_unsupported"
    );
    assert_eq!(
        backend_error(409, Some("asset_owner_market_unavailable")),
        "wallet_market_unavailable"
    );
    assert_eq!(
        backend_error(409, Some("asset_owner_fee_changed")),
        "wallet_quote_changed"
    );
    for error in [
        "wallet_network_failed",
        "wallet_operation_expired",
        "wallet_response_invalid",
        "wallet_insufficient_balance",
    ] {
        assert_eq!(submission_error(error), "wallet_outcome_unknown");
    }
    assert_eq!(
        submission_error("wallet_rate_limited"),
        "wallet_outcome_rate_limited"
    );
}

#[test]
fn capabilities_and_receipts_cannot_misrepresent_execution_or_node() {
    let (mut cap, account, id, _, _) = fixture();
    cap.actions.push("balances".into());
    assert!(cap.validate(Service::Bancor, "balances").is_err());
    cap.actions.pop();
    for url in [
        "https://ledger.example.test/path",
        "https://ledger.example.test/",
        "https://ledger.example.test?x=1",
    ] {
        cap.node_url = url.into();
        assert!(cap.validate(Service::Bancor, "balances").is_err());
    }
    let mut outcome = Outcome {
        operation_id: id,
        account: account.clone(),
        ledger_id: cap.ledger_id.clone(),
        status: "succeeded".into(),
        receipt_id: None,
    };
    assert!(outcome.validate(id, &account, &cap.ledger_id).is_err());
    outcome.receipt_id = Some(id.to_string());
    outcome.validate(id, &account, &cap.ledger_id).unwrap();
    outcome.status = "pending".into();
    assert!(outcome.validate(id, &account, &cap.ledger_id).is_err());
}
