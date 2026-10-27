use super::*;
use crate::AppState;

fn fixture() -> (AppState, String, claw_core::types::AuthIdentity) {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.seed_test_auth_identity("delivery-owner", "admin");
    let identity = crate::resolve_auth_identity_by_key(&state, "delivery-owner")
        .unwrap()
        .unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let db = state.core.db.get().unwrap();
    crate::repo::channel_delivery_outbox::ensure_channel_delivery_outbox_schema(&db).unwrap();
    ensure_channel_delivery_receipt_schema(&db).unwrap();
    db.execute("INSERT INTO tasks(task_id,user_id,chat_id,user_key,principal_id,channel,kind,payload_json,status,created_at,updated_at) VALUES (?1,1,2,'delivery-owner',?2,'wechat','ask','{}','succeeded','1','1')",params![id,identity.principal_id]).unwrap();
    db.execute(
        "UPDATE tasks SET external_user_id='one',external_chat_id='two' WHERE task_id=?1",
        [&id],
    )
    .unwrap();
    db.execute("INSERT INTO channel_bindings(channel,external_user_id,external_chat_id,user_key,bound_at,updated_at) VALUES ('wechat','one','two','delivery-owner','1','1')",[]).unwrap();
    for (key, delivery_id, status, error) in [
        (
            "parent".to_string(),
            format!("delivery:{id}:terminal"),
            ChannelDeliveryStatus::Partial,
            Some("channel.provider.transport"),
        ),
        (
            "part-0".to_string(),
            "delivery-part:one".to_string(),
            ChannelDeliveryStatus::Accepted,
            None,
        ),
        (
            "part-1".to_string(),
            "delivery-part:two".to_string(),
            ChannelDeliveryStatus::Failed,
            Some("channel.provider.transport"),
        ),
    ] {
        let r = ChannelDeliveryReceipt {
            schema_version: 1,
            delivery_id: delivery_id.clone(),
            idempotency_key: key.clone(),
            channel: claw_core::types::ChannelKind::Wechat,
            adapter: "wechat_ilink".into(),
            status,
            provider_message_ids: vec![],
            parts: if status == ChannelDeliveryStatus::Partial {
                vec![
                    claw_core::channel_delivery::ChannelDeliveryPartReceipt {
                        part_index: 0,
                        status: ChannelDeliveryStatus::Accepted,
                        provider_message_id: None,
                        error_code: None,
                    },
                    claw_core::channel_delivery::ChannelDeliveryPartReceipt {
                        part_index: 1,
                        status: ChannelDeliveryStatus::Failed,
                        provider_message_id: None,
                        error_code: Some("channel.provider.transport".into()),
                    },
                ]
            } else {
                vec![]
            },
            error_code: error.map(str::to_owned),
            message_key: error.map(|_| "channel.error.delivery_failed".into()),
            diagnostic_id: error.map(|_| "delivery:fixture".into()),
            provider_error_code: None,
            retryable: false,
            updated_at_ts: 1,
        };
        save_receipt(&db, &r).unwrap();
        db.execute("INSERT INTO channel_delivery_dispatch_claims VALUES (?1,?2,'wechat','wechat_ilink','receipt_recorded',NULL,0,1,1,1)",params![key,delivery_id]).unwrap();
    }
    db.execute_batch("CREATE TABLE channel_delivery_part_plans(idempotency_key TEXT PRIMARY KEY,plan_json TEXT,task_id TEXT,part_keys_json TEXT);").unwrap();
    db.execute("INSERT INTO channel_delivery_part_plans VALUES ('parent','{}',?1,'[\"part-0\",\"part-1\"]')",[&id]).unwrap();
    drop(db);
    (state, id, identity)
}

fn decision(db: &Connection, id: &str, operation: Operation) -> Decision {
    Decision {
        operation,
        snapshot_digest: inspect(db, id, 10).unwrap()["snapshot_digest"]
            .as_str()
            .unwrap()
            .into(),
        decision_id: uuid::Uuid::new_v4(),
        confirmed_duplicate_risk: true,
    }
}

#[test]
fn inspection_is_bounded_private_and_does_not_grant_or_retry() {
    let (state, id, _) = fixture();
    let db = state.core.db.get().unwrap();
    let view = inspect(&db, &id, 10).unwrap();
    assert_eq!(view["status"], "query_required");
    assert_eq!(view["unknown_count"], 1);
    assert_eq!(view["accepted_count"], 1);
    assert!(!view.to_string().contains("part-0"));
    assert!(!exists(&db, "channel_delivery_reconciliations").unwrap());
}

#[test]
fn explicit_retry_preserves_accepted_prefix_and_is_idempotent_and_fenced() {
    let (state, id, identity) = fixture();
    let mut db = state.core.db.get().unwrap();
    let d = decision(&db, &id, Operation::RetryUnknown);
    let before = load_channel_delivery_receipt_from_db(&db, "part-0").unwrap();
    assert_eq!(
        reconcile(&mut db, &id, &identity, &d, 10).unwrap()["status"],
        "queued"
    );
    assert_eq!(
        load_channel_delivery_receipt_from_db(&db, "part-0").unwrap(),
        before
    );
    assert!(
        load_channel_delivery_receipt_from_db(&db, "part-1")
            .unwrap()
            .unwrap()
            .retryable
    );
    assert_eq!(
        reconcile(&mut db, &id, &identity, &d, 11).unwrap()["status"],
        "already_recorded"
    );
    let mut stale = d;
    stale.decision_id = uuid::Uuid::new_v4();
    assert_eq!(
        reconcile(&mut db, &id, &identity, &stale, 12)
            .unwrap_err()
            .to_string(),
        "delivery_snapshot_changed"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM channel_delivery_reconciliations",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn user_confirmation_is_not_provider_delivery_proof() {
    let (state, id, identity) = fixture();
    let mut db = state.core.db.get().unwrap();
    let d = decision(&db, &id, Operation::ConfirmReceived);
    let result = reconcile(&mut db, &id, &identity, &d, 10).unwrap();
    assert_eq!(result["provider_delivery_proof"], false);
    assert_eq!(inspect(&db, &id, 11).unwrap()["status"], "user_confirmed");
    let receipt = load_channel_delivery_receipt_from_db(&db, "part-1")
        .unwrap()
        .unwrap();
    assert_eq!(receipt.status, ChannelDeliveryStatus::Accepted);
    assert!(receipt.provider_message_ids.is_empty());
    assert_eq!(
        db.query_row(
            "SELECT part_indexes_json FROM channel_delivery_reconciliations",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "[1]"
    );
}

#[test]
fn ambiguous_transport_is_never_an_ordinary_retry_even_when_marked_retryable() {
    let (state, id, identity) = fixture();
    let mut db = state.core.db.get().unwrap();
    let mut receipt = load_channel_delivery_receipt_from_db(&db, "part-1")
        .unwrap()
        .unwrap();
    receipt.retryable = true;
    save_receipt(&db, &receipt).unwrap();
    assert_eq!(inspect(&db, &id, 10).unwrap()["retryable_count"], 0);
    let d = decision(&db, &id, Operation::RetryFailed);
    assert_eq!(
        reconcile(&mut db, &id, &identity, &d, 10)
            .unwrap_err()
            .to_string(),
        "delivery_no_eligible_parts"
    );
}

#[test]
fn provider_proof_and_all_accepted_parts_do_not_offer_parent_only_retries() {
    let (state, id, identity) = fixture();
    let mut db = state.core.db.get().unwrap();
    for key in ["part-0", "part-1"] {
        let mut r = load_channel_delivery_receipt_from_db(&db, key)
            .unwrap()
            .unwrap();
        r.status = ChannelDeliveryStatus::Delivered;
        r.error_code = None;
        r.message_key = None;
        r.diagnostic_id = None;
        r.provider_message_ids = vec![format!("provider-{key}")];
        save_receipt(&db, &r).unwrap();
    }
    let view = inspect(&db, &id, 10).unwrap();
    assert_eq!(view["status"], "delivered");
    assert_eq!(view["can_reconcile"], false);
    let d = decision(&db, &id, Operation::RetryUnknown);
    assert_eq!(
        reconcile(&mut db, &id, &identity, &d, 10)
            .unwrap_err()
            .to_string(),
        "delivery_no_eligible_parts"
    );
}

#[test]
fn risk_confirmation_and_current_authorization_are_required() {
    let (state, id, mut identity) = fixture();
    let mut db = state.core.db.get().unwrap();
    let mut d = decision(&db, &id, Operation::RetryUnknown);
    d.confirmed_duplicate_risk = false;
    assert_eq!(
        reconcile(&mut db, &id, &identity, &d, 10)
            .unwrap_err()
            .to_string(),
        "delivery_duplicate_risk_confirmation_required"
    );
    d.confirmed_duplicate_risk = true;
    identity.principal_id = "other".into();
    assert_eq!(
        reconcile(&mut db, &id, &identity, &d, 10)
            .unwrap_err()
            .to_string(),
        "task_delivery_unauthorized"
    );
    db.execute(
        "UPDATE auth_keys SET enabled=0 WHERE user_key='delivery-owner'",
        [],
    )
    .unwrap();
    identity.principal_id = db
        .query_row(
            "SELECT principal_id FROM tasks WHERE task_id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        reconcile(&mut db, &id, &identity, &d, 10)
            .unwrap_err()
            .to_string(),
        "task_delivery_unauthorized"
    );
}

#[test]
fn channel_unbinding_between_inspection_and_decision_revokes_recovery() {
    let (state, id, identity) = fixture();
    let mut db = state.core.db.get().unwrap();
    let d = decision(&db, &id, Operation::RetryUnknown);
    db.execute("DELETE FROM channel_bindings", []).unwrap();
    assert_eq!(
        reconcile(&mut db, &id, &identity, &d, 10)
            .unwrap_err()
            .to_string(),
        "task_delivery_unauthorized"
    );
}

#[test]
fn an_active_send_cannot_be_reconciled_and_deletion_cleans_the_audit() {
    let (state, id, identity) = fixture();
    let mut db = state.core.db.get().unwrap();
    db.execute("UPDATE channel_delivery_dispatch_claims SET state='dispatching',lease_token='fixture',lease_expires_at_ts=99 WHERE idempotency_key='parent'",[]).unwrap();
    let d = decision(&db, &id, Operation::RetryUnknown);
    assert_eq!(
        reconcile(&mut db, &id, &identity, &d, 10)
            .unwrap_err()
            .to_string(),
        "delivery_dispatch_active"
    );
    db.execute("UPDATE channel_delivery_dispatch_claims SET state='query_required',lease_expires_at_ts=0 WHERE idempotency_key='parent'",[]).unwrap();
    let d = decision(&db, &id, Operation::ConfirmReceived);
    reconcile(&mut db, &id, &identity, &d, 10).unwrap();
    db.execute("DELETE FROM tasks WHERE task_id=?1", [id])
        .unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM channel_delivery_reconciliations",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
