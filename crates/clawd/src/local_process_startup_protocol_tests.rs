use super::*;

fn credential() -> OwnerCredential {
    super::super::owner_runtime::OwnerRuntime::prepare_credential(
        uuid::Uuid::new_v4(),
        uuid::Uuid::new_v4(),
    )
    .unwrap()
}

fn messages() -> [StartupMessage; 7] {
    [
        StartupMessage::Ready {},
        StartupMessage::Commit {},
        StartupMessage::Acknowledgment {
            result: StartupAck::Started {},
        },
        StartupMessage::Acknowledgment {
            result: StartupAck::RejectedBeforeSpawn {
                reason: StartupRejection::MissingReference {},
            },
        },
        StartupMessage::Acknowledgment {
            result: StartupAck::RejectedBeforeSpawn {
                reason: StartupRejection::Reference {
                    error_code: OwnerReferenceFailure::AuthenticationFailed,
                },
            },
        },
        StartupMessage::Acknowledgment {
            result: StartupAck::RejectedBeforeSpawn {
                reason: StartupRejection::StartupBudgetExpired {},
            },
        },
        StartupMessage::Acknowledgment {
            result: StartupAck::AttemptFailed {
                error_code: OwnerFailure::SpawnFailed,
            },
        },
    ]
}

#[test]
fn startup_protocol_preserves_phase_and_separate_rejection_or_attempt_failure() {
    let expected = credential();
    for message in messages() {
        let bytes = encode(&expected, [9; 32], message).unwrap();
        assert!(bytes.len() < MAX_CONTROL_BYTES);
        assert!(decode(&bytes, &expected, &[9; 32]).unwrap() == message);
    }
}

#[test]
fn startup_protocol_binds_exact_job_nonce_birth_and_policy_digest() {
    let expected = credential();
    let bytes = encode(&expected, [9; 32], StartupMessage::Commit {}).unwrap();
    for field in 0..5 {
        let mut changed = expected.clone();
        let mut binding = [9; 32];
        match field {
            0 => changed.job_id = uuid::Uuid::new_v4(),
            1 => changed.nonce = uuid::Uuid::new_v4(),
            2 => changed.owner.boot_id = uuid::Uuid::new_v4(),
            3 => changed.owner.pid = if changed.owner.pid == 2 { 3 } else { 2 },
            _ => binding = [8; 32],
        }
        assert!(matches!(
            decode(&bytes, &changed, &binding),
            Err(OwnerFailure::CredentialMismatch)
        ));
    }
}

#[test]
fn startup_protocol_rejects_unknown_fields_schemas_and_unbounded_frames() {
    let expected = credential();
    let bytes = encode(&expected, [9; 32], StartupMessage::Ready {}).unwrap();
    for field in 0..3 {
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        match field {
            0 => value["schema_version"] = serde_json::json!(2),
            1 => value["command"] = serde_json::json!("unexpected"),
            _ => value["message"]["text"] = serde_json::json!("unexpected"),
        }
        assert!(decode(&serde_json::to_vec(&value).unwrap(), &expected, &[9; 32]).is_err());
    }
    for bytes in [vec![], vec![b' '; MAX_CONTROL_BYTES + 1], b"{}".to_vec()] {
        assert!(decode(&bytes, &expected, &[9; 32]).is_err());
    }
    assert!(encode(&expected, [0; 32], StartupMessage::Ready {}).is_err());
}

#[test]
fn startup_protocol_rejects_unexpected_fields_in_every_message_variant_and_nested_record() {
    let expected = credential();
    for (index, message) in messages().into_iter().enumerate() {
        let bytes = encode(&expected, [9; 32], message).unwrap();
        for path in [
            "",
            "/credential",
            "/credential/owner",
            "/message",
            "/message/result",
            "/message/result/reason",
        ] {
            let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            let Some(record) = value.pointer_mut(path) else {
                continue;
            };
            record["unexpected_field"] = serde_json::json!(true);
            assert!(
                matches!(
                    decode(&serde_json::to_vec(&value).unwrap(), &expected, &[9; 32]),
                    Err(OwnerFailure::InvalidFrame)
                ),
                "message {index} accepted extra field at {path}"
            );
        }
    }
}
