use serde_json::{json, Value};

use super::super::owner_reference::{OwnerAuthorityKey, MAX_REFERENCE_BYTES};
use super::super::owner_store::tests::{binding, credential, key};
use super::*;

pub(crate) fn record(
    expected: &OwnerCredential,
    error_code: OwnerFailure,
) -> OwnerStartupFailureRecord {
    OwnerStartupFailureRecord {
        schema_version: 1,
        reference: OwnerReference::new(expected.job_id, expected.owner.clone(), binding()).unwrap(),
        error_code,
    }
}

#[test]
fn startup_outcome_is_authenticated_separately_from_reference_and_scope_terminal() {
    let expected = credential(uuid::Uuid::new_v4());
    let record = record(&expected, OwnerFailure::SpawnFailed);
    let bytes = key().sign_startup_failure(record.clone()).unwrap();
    assert!(matches!(key().verify_outcome(&bytes).unwrap(),
        OwnerOutcomeRecord::StartupFailed(actual) if actual == record));
    assert!(matches!(
        key().verify_terminal(&bytes),
        Err(OwnerReferenceFailure::Conflict)
    ));
    assert!(key().verify(&bytes).is_err());
    assert!(!String::from_utf8_lossy(&bytes).contains(&expected.nonce.to_string()));
    assert!(key()
        .verify_outcome(&key().sign(record.reference.clone()).unwrap())
        .is_err());

    let mut value: Value = serde_json::from_slice(&bytes).unwrap();
    use hmac::{Hmac, Mac};
    let mut wrong_domain = Hmac::<sha2::Sha256>::new_from_slice(&[7; 32]).unwrap();
    wrong_domain.update(b"owner-terminal-v1\0");
    wrong_domain.update(&serde_json::to_vec(&record).unwrap());
    value["authenticator"] = json!(wrong_domain.finalize().into_bytes().to_vec());
    assert!(matches!(
        key().verify_outcome(&serde_json::to_vec(&value).unwrap()),
        Err(OwnerReferenceFailure::AuthenticationFailed)
    ));
}

#[test]
fn startup_outcome_rejects_tampered_identity_binding_error_kind_and_authenticator() {
    let expected = credential(uuid::Uuid::new_v4());
    let bytes = key()
        .sign_startup_failure(record(&expected, OwnerFailure::SpawnFailed))
        .unwrap();
    for field in 0..6 {
        let mut value: Value = serde_json::from_slice(&bytes).unwrap();
        match field {
            0 => value["receipt"]["reference"]["job_id"] = json!(uuid::Uuid::new_v4()),
            1 => value["receipt"]["reference"]["owner"]["boot_id"] = json!(uuid::Uuid::new_v4()),
            2 => value["receipt"]["reference"]["binding_digest"] = json!(vec![8; 32]),
            3 => value["receipt"]["error_code"] = json!("unsupported"),
            4 => value["kind"] = json!("terminal"),
            _ => value["authenticator"][0] = json!(value["authenticator"][0].as_u64().unwrap() ^ 1),
        }
        assert!(key()
            .verify_outcome(&serde_json::to_vec(&value).unwrap())
            .is_err());
    }
    assert!(matches!(
        OwnerAuthorityKey::new([8; 32])
            .unwrap()
            .verify_outcome(&bytes),
        Err(OwnerReferenceFailure::AuthenticationFailed)
    ));
}

#[test]
fn startup_outcome_rejects_unknown_nested_fields_schema_and_unbounded_input() {
    let expected = credential(uuid::Uuid::new_v4());
    let bytes = key()
        .sign_startup_failure(record(&expected, OwnerFailure::SpawnFailed))
        .unwrap();
    for pointer in [
        "",
        "/receipt",
        "/receipt/reference",
        "/receipt/reference/owner",
        "/receipt/reference/owner/started",
    ] {
        let mut value: Value = serde_json::from_slice(&bytes).unwrap();
        value.pointer_mut(pointer).unwrap()["unexpected"] = json!(true);
        assert!(matches!(
            key().verify_outcome(&serde_json::to_vec(&value).unwrap()),
            Err(OwnerReferenceFailure::InvalidReference)
        ));
    }
    for bytes in [vec![], vec![b' '; MAX_REFERENCE_BYTES + 1], b"{}".to_vec()] {
        assert!(matches!(
            key().verify_outcome(&bytes),
            Err(OwnerReferenceFailure::InvalidReference)
        ));
    }
    let mut invalid = record(&expected, OwnerFailure::SpawnFailed);
    invalid.schema_version = 2;
    assert!(matches!(
        key().sign_startup_failure(invalid),
        Err(OwnerReferenceFailure::InvalidReference)
    ));
}

fn published_reference() -> (tempfile::TempDir, DurableOwnerReference) {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let expected = credential(uuid::Uuid::new_v4());
    let reference = super::super::owner_store::tests::store(root.path())
        .publish(&expected, binding())
        .unwrap();
    (root, reference)
}

#[test]
fn protected_observation_conversion_keeps_pending_terminal_and_startup_failure_distinct() {
    use super::super::exit::ChildExit;
    use super::super::owner_protocol::OwnerCompletion;
    let (_root, reference) = published_reference();
    let expected = reference.credential();
    // Synthetic trusted-boundary inputs test conversion only. Actual channel
    // authentication is covered by the source-pinned executable tests.
    assert!(
        unsafe { OwnerObservationRecord::Pending {}.into_protected_outcome(&reference) }
            .unwrap()
            .is_none()
    );
    let snapshot = OwnerSnapshot {
        phase: OwnerPhase::Terminal,
        completion: Some(OwnerCompletion::Completed),
        root_exit: Some(ChildExit {
            code: Some(7),
            signal: None,
        }),
    };
    let terminal =
        unsafe { OwnerObservationRecord::Terminal { snapshot }.into_protected_outcome(&reference) }
            .unwrap()
            .unwrap();
    assert!(matches!(&terminal, DurableOwnerOutcome::Terminal(proof)
        if proof.authorizes(&expected, &binding()) && proof.snapshot() == snapshot));
    assert_eq!(
        OwnerObservationRecord::from_verified_outcome(Some(&terminal)),
        OwnerObservationRecord::Terminal { snapshot }
    );
    let failure = unsafe {
        OwnerObservationRecord::StartupFailed {
            error_code: OwnerFailure::SpawnFailed,
        }
        .into_protected_outcome(&reference)
    }
    .unwrap()
    .unwrap();
    assert!(matches!(&failure, DurableOwnerOutcome::StartupFailed(proof)
        if proof.authorizes(&expected, &binding()) && proof.error_code() == OwnerFailure::SpawnFailed));
    assert_eq!(
        OwnerObservationRecord::from_verified_outcome(Some(&failure)),
        OwnerObservationRecord::StartupFailed {
            error_code: OwnerFailure::SpawnFailed
        }
    );
    let encoded = serde_json::to_string(&OwnerObservationRecord::from_verified_outcome(Some(
        &terminal,
    )))
    .unwrap();
    assert!(!encoded.contains(&expected.nonce.to_string()));
    assert!(!encoded.contains("credential"));
}

#[test]
fn protected_terminal_conversion_refuses_nonterminal_and_incomplete_snapshots() {
    use super::super::exit::ChildExit;
    use super::super::owner_protocol::OwnerCompletion;
    let (_root, reference) = published_reference();
    for snapshot in [
        OwnerSnapshot {
            phase: OwnerPhase::Running,
            completion: None,
            root_exit: None,
        },
        OwnerSnapshot {
            phase: OwnerPhase::Running,
            completion: None,
            root_exit: Some(ChildExit {
                code: Some(0),
                signal: None,
            }),
        },
        OwnerSnapshot {
            phase: OwnerPhase::Terminal,
            completion: Some(OwnerCompletion::Completed),
            root_exit: None,
        },
        OwnerSnapshot {
            phase: OwnerPhase::Terminal,
            completion: None,
            root_exit: Some(ChildExit {
                code: Some(0),
                signal: None,
            }),
        },
    ] {
        let result = unsafe {
            OwnerObservationRecord::Terminal { snapshot }.into_protected_outcome(&reference)
        };
        assert!(matches!(
            result,
            Err(OwnerReferenceFailure::InvalidReference)
        ));
    }
}

#[test]
fn private_observation_wire_variants_are_closed_machine_records() {
    for valid in [
        json!({"kind":"pending"}),
        json!({"kind":"startup_failed","error_code":"spawn_failed"}),
    ] {
        let observation: OwnerObservationRecord = serde_json::from_value(valid.clone()).unwrap();
        observation.validate().unwrap();
        let mut extra = valid;
        extra["unexpected"] = json!(true);
        assert!(serde_json::from_value::<OwnerObservationRecord>(extra).is_err());
    }
    for invalid in [
        json!({"kind":"missing"}),
        json!({"kind":"startup_failed","error_code":"unknown"}),
        json!({"kind":"terminal"}),
    ] {
        assert!(serde_json::from_value::<OwnerObservationRecord>(invalid).is_err());
    }
}
