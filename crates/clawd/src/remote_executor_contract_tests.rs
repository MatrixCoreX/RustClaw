use super::*;

fn digest(fill: char) -> String {
    std::iter::repeat_n(fill, 64).collect()
}

fn assignment() -> RemoteExecutorAssignment {
    RemoteExecutorAssignment {
        schema_version: 1,
        assignment_id: "assignment-1".to_string(),
        task_id: "task-1".to_string(),
        idempotency_key: "effect-1".to_string(),
        code_revision: "revision-1".to_string(),
        registry_generation: "generation-1".to_string(),
        policy_digest: digest('a'),
        capability_digest: digest('b'),
        skill_receipt_digest: digest('c'),
        workspace_snapshot: None,
        granted_capabilities: vec!["code.build".to_string()],
        credential_refs: vec![RemoteCredentialRef {
            reference: "credential-ref-1".to_string(),
            audience: "executor-1".to_string(),
            expires_at_unix: 150,
        }],
        lease: RemoteExecutorLease {
            lease_id: "lease-1".to_string(),
            owner_id: "executor-1".to_string(),
            issued_at_unix: 90,
            expires_at_unix: 200,
            heartbeat_seq: 3,
        },
    }
}

#[test]
fn assignment_pins_revision_policy_receipt_and_short_lived_credentials() {
    validate_assignment(&assignment(), 100).expect("assignment should validate");
}

#[test]
fn expired_lease_and_long_lived_credentials_are_rejected() {
    let mut value = assignment();
    value.lease.expires_at_unix = 99;
    assert!(validate_assignment(&value, 100)
        .expect_err("expired lease must fail")
        .to_string()
        .contains("lease_invalid"));
    let mut value = assignment();
    value.credential_refs[0].expires_at_unix = 201;
    assert!(validate_assignment(&value, 100)
        .expect_err("credential cannot outlive lease")
        .to_string()
        .contains("credential_lease_invalid"));
}

#[test]
fn stale_or_wrong_owner_events_are_rejected() {
    let value = assignment();
    let mut event = RemoteExecutorEvent {
        schema_version: 1,
        assignment_id: value.assignment_id.clone(),
        lease_id: value.lease.lease_id.clone(),
        sequence: 2,
        state: RemoteExecutorState::Running,
        progress_digest: digest('d'),
        heartbeat_at_unix: 110,
        artifact_refs: vec![],
    };
    assert!(validate_event(&value, &event).is_err());
    event.sequence = 4;
    event.lease_id = "wrong-lease".to_string();
    assert!(validate_event(&value, &event).is_err());
}

#[test]
fn network_loss_never_blindly_replays_external_effects() {
    assert_eq!(
        state_after_transport_loss(true),
        RemoteExecutorState::Ambiguous
    );
    assert_eq!(
        state_after_transport_loss(false),
        RemoteExecutorState::Waiting
    );
}
