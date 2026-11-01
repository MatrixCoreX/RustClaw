use serde_json::{json, Value};

use super::super::exit::ChildExit;
use super::super::owner_protocol::OwnerCompletion;
use super::super::owner_reference::MAX_REFERENCE_BYTES;
use super::super::owner_store::tests::{binding, credential, key};
use super::*;

pub(crate) fn record(credential: &OwnerCredential) -> OwnerTerminalRecord {
    OwnerTerminalRecord {
        schema_version: 1,
        reference: OwnerReference::new(credential.job_id, credential.owner.clone(), binding())
            .unwrap(),
        snapshot: OwnerSnapshot {
            phase: OwnerPhase::Terminal,
            completion: Some(OwnerCompletion::Completed),
            root_exit: Some(ChildExit {
                code: Some(7),
                signal: None,
            }),
        },
    }
}

#[test]
fn owner_terminal_authentication_is_separate_from_start_authority_and_persists_no_nonce() {
    let expected = credential(uuid::Uuid::new_v4());
    let record = record(&expected);
    let bytes = key().sign_terminal(record.clone()).unwrap();
    assert!(key().verify_terminal(&bytes).unwrap() == record);
    assert!(!String::from_utf8_lossy(&bytes).contains(&expected.nonce.to_string()));
    assert!(key().verify(&bytes).is_err());
    assert!(key()
        .verify_terminal(&key().sign(record.reference.clone()).unwrap())
        .is_err());
    let mut value: Value = serde_json::from_slice(&bytes).unwrap();
    use hmac::{Hmac, Mac};
    let mut wrong_domain = Hmac::<sha2::Sha256>::new_from_slice(&[7; 32]).unwrap();
    wrong_domain.update(b"owner-reference-v1\0");
    wrong_domain.update(&serde_json::to_vec(&record).unwrap());
    value["authenticator"] = json!(wrong_domain.finalize().into_bytes().to_vec());
    assert!(matches!(
        key().verify_terminal(&serde_json::to_vec(&value).unwrap()),
        Err(OwnerReferenceFailure::AuthenticationFailed)
    ));
}

#[test]
fn owner_terminal_modified_job_birth_binding_exit_or_completion_cannot_verify() {
    let expected = credential(uuid::Uuid::new_v4());
    let bytes = key().sign_terminal(record(&expected)).unwrap();
    for field in 0..6 {
        let mut value: Value = serde_json::from_slice(&bytes).unwrap();
        match field {
            0 => value["receipt"]["reference"]["job_id"] = json!(uuid::Uuid::new_v4()),
            1 => value["receipt"]["reference"]["owner"]["boot_id"] = json!(uuid::Uuid::new_v4()),
            2 => value["receipt"]["reference"]["binding_digest"] = json!(vec![8; 32]),
            3 => value["receipt"]["snapshot"]["root_exit"]["code"] = json!(8),
            4 => value["receipt"]["snapshot"]["completion"] = json!("cancelled"),
            _ => value["authenticator"][0] = json!(value["authenticator"][0].as_u64().unwrap() ^ 1),
        }
        assert!(key()
            .verify_terminal(&serde_json::to_vec(&value).unwrap())
            .is_err());
    }
    assert!(matches!(
        super::super::owner_reference::OwnerAuthorityKey::new([8; 32])
            .unwrap()
            .verify_terminal(&bytes),
        Err(OwnerReferenceFailure::AuthenticationFailed)
    ));
}

#[test]
fn owner_terminal_closed_schema_refuses_unknown_nested_fields_and_unbounded_input() {
    let bytes = key()
        .sign_terminal(record(&credential(uuid::Uuid::new_v4())))
        .unwrap();
    for pointer in [
        "",
        "/receipt",
        "/receipt/reference",
        "/receipt/reference/owner",
        "/receipt/reference/owner/started",
        "/receipt/snapshot",
        "/receipt/snapshot/root_exit",
    ] {
        let mut value: Value = serde_json::from_slice(&bytes).unwrap();
        value.pointer_mut(pointer).unwrap()["unexpected"] = json!(true);
        assert!(matches!(
            key().verify_terminal(&serde_json::to_vec(&value).unwrap()),
            Err(OwnerReferenceFailure::InvalidReference)
        ));
    }
    for bytes in [vec![], vec![b' '; MAX_REFERENCE_BYTES + 1], b"{}".to_vec()] {
        assert!(matches!(
            key().verify_terminal(&bytes),
            Err(OwnerReferenceFailure::InvalidReference)
        ));
    }
}

#[test]
fn owner_terminal_running_cancelling_incomplete_or_invalid_exit_is_not_a_terminal_record() {
    let expected = credential(uuid::Uuid::new_v4());
    for field in 0..8 {
        let mut record = record(&expected);
        match field {
            0 => record.schema_version = 2,
            1 => record.snapshot.phase = OwnerPhase::Running,
            2 => record.snapshot.phase = OwnerPhase::Cancelling,
            3 => record.snapshot.completion = None,
            4 => record.snapshot.root_exit = None,
            5 => record.snapshot.root_exit.as_mut().unwrap().code = Some(256),
            6 => record.snapshot.root_exit.as_mut().unwrap().signal = Some(15),
            _ => record.reference.binding_digest = [0; 32],
        }
        assert!(matches!(
            key().sign_terminal(record),
            Err(OwnerReferenceFailure::InvalidReference)
        ));
    }
}
