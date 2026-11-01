use super::super::owner_outcome::{
    DurableOwnerOutcome, OwnerObservationRecord, OwnerOutcomeRecord,
};
use super::super::owner_reference::{OwnerAuthorityKey, MAX_REFERENCE_BYTES};
use super::super::owner_store::OwnerReferenceStore;
use super::*;
use std::fs::{self, File};
use std::os::unix::fs::MetadataExt;
use uuid::Uuid;

#[path = "local_process_owner_no_commit_test_support.rs"]
mod support;
use support::{key, Fixture};

#[test]
fn no_commit_signature_has_its_own_domain_and_contains_no_control_nonce() {
    use hmac::{Hmac, Mac};
    let fixture = Fixture::new();
    let record = fixture.record();
    let bytes = key().sign_no_commit(record.clone()).unwrap();
    assert!(
        matches!(key().verify_outcome(&bytes).unwrap(), OwnerOutcomeRecord::NotCommitted(actual) if actual == record)
    );
    assert!(matches!(
        key().verify_terminal(&bytes),
        Err(OwnerReferenceFailure::Conflict)
    ));
    assert!(key().verify(&bytes).is_err());
    assert!(!String::from_utf8_lossy(&bytes)
        .contains(&fixture.reference.credential().nonce.to_string()));
    assert!(OwnerAuthorityKey::new([8; 32])
        .unwrap()
        .verify_outcome(&bytes)
        .is_err());
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(&[7; 32]).unwrap();
    mac.update(b"owner-terminal-v1\0");
    mac.update(&serde_json::to_vec(&record).unwrap());
    value["authenticator"] = serde_json::json!(mac.finalize().into_bytes().to_vec());
    assert!(matches!(
        key().verify_outcome(&serde_json::to_vec(&value).unwrap()),
        Err(OwnerReferenceFailure::AuthenticationFailed)
    ));
}

#[test]
fn no_commit_signature_rejects_tampering_unknown_fields_and_invalid_schema() {
    let fixture = Fixture::new();
    let bytes = key().sign_no_commit(fixture.record()).unwrap();
    for pointer in [
        "",
        "/receipt",
        "/receipt/reference",
        "/receipt/reference/owner",
        "/receipt/reference/owner/started",
    ] {
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value.pointer_mut(pointer).unwrap()["unexpected"] = serde_json::json!(true);
        assert!(key()
            .verify_outcome(&serde_json::to_vec(&value).unwrap())
            .is_err());
    }
    for field in ["job", "birth", "binding", "mac", "kind"] {
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        match field {
            "job" => value["receipt"]["reference"]["job_id"] = serde_json::json!(Uuid::new_v4()),
            "birth" => {
                value["receipt"]["reference"]["owner"]["boot_id"] =
                    serde_json::json!(Uuid::new_v4())
            }
            "binding" => {
                value["receipt"]["reference"]["binding_digest"] =
                    serde_json::to_value([8_u8; 32]).unwrap()
            }
            "kind" => value["kind"] = serde_json::json!("terminal"),
            _ => {
                value["authenticator"][0] =
                    serde_json::json!(value["authenticator"][0].as_u64().unwrap() ^ 1)
            }
        }
        assert!(key()
            .verify_outcome(&serde_json::to_vec(&value).unwrap())
            .is_err());
    }
    let mut invalid = fixture.record();
    invalid.schema_version = 2;
    assert!(key().sign_no_commit(invalid).is_err());
    for bytes in [vec![], vec![b' '; MAX_REFERENCE_BYTES + 1]] {
        assert!(key().verify_outcome(&bytes).is_err());
    }
}

#[test]
fn no_commit_store_reopens_idempotently_and_never_becomes_scope_terminal() {
    let fixture = Fixture::new();
    fixture
        .store
        .publish_no_commit(&fixture.reference, &fixture.evidence)
        .unwrap();
    let before = fs::metadata(fixture.outcome_path()).unwrap();
    let bytes = fs::read(fixture.outcome_path()).unwrap();
    fixture
        .store
        .publish_no_commit(&fixture.reference, &fixture.evidence)
        .unwrap();
    assert_eq!(
        before.ino(),
        fs::metadata(fixture.outcome_path()).unwrap().ino()
    );
    assert_eq!(before.mode() & 0o7777, 0o600);
    assert_eq!(bytes, fs::read(fixture.outcome_path()).unwrap());
    let reopened = OwnerReferenceStore::from_admitted_directory(
        File::open(fixture.root.path()).unwrap(),
        key(),
    )
    .unwrap();
    let reference = reopened
        .load(fixture.reference.credential().job_id)
        .unwrap()
        .unwrap();
    let recovered = reopened.load_outcome(&reference).unwrap().unwrap();
    assert!(
        matches!(&recovered, DurableOwnerOutcome::NotCommitted(proof) if proof.authorizes(&reference.credential(), &[9; 32]))
    );
    assert_eq!(
        OwnerObservationRecord::from_verified_outcome(Some(&recovered)),
        OwnerObservationRecord::NotCommitted {}
    );
    assert!(reopened.load_terminal(&reference).is_err());
}

#[test]
fn no_commit_store_refuses_another_prepared_owner_and_binding() {
    let fixture = Fixture::new();
    let other = Fixture::new();
    assert!(fixture
        .store
        .publish_no_commit(&fixture.reference, &other.evidence)
        .is_err());
    assert!(!fixture.outcome_path().exists());
    let mut changed = fixture.reference.record().clone();
    changed.binding_digest = [8; 32];
    assert!(OwnerNoCommitRecord::from_evidence(changed, &fixture.evidence).is_err());
}

#[test]
fn no_commit_store_rejects_valid_mac_for_another_reference() {
    for field in ["job", "birth", "binding"] {
        let fixture = Fixture::new();
        let mut record = fixture.record();
        match field {
            "job" => record.reference.job_id = Uuid::new_v4(),
            "birth" => record.reference.owner.boot_id = Uuid::new_v4(),
            _ => record.reference.binding_digest = [8; 32],
        }
        fixture.write_outcome(&key().sign_no_commit(record).unwrap());
        let before = fs::read(fixture.outcome_path()).unwrap();
        assert!(fixture.store.load_outcome(&fixture.reference).is_err());
        assert!(fixture
            .store
            .publish_no_commit(&fixture.reference, &fixture.evidence)
            .is_err());
        assert_eq!(before, fs::read(fixture.outcome_path()).unwrap());
    }
}

#[test]
fn no_commit_cannot_replace_attempted_startup_failure_or_damage_its_evidence() {
    let fixture = Fixture::new();
    let record: super::super::owner_outcome::OwnerStartupFailureRecord = serde_json::from_value(serde_json::json!({
        "schema_version": 1, "reference": fixture.reference.record(), "error_code": "spawn_failed"
    })).unwrap();
    let before = key().sign_startup_failure(record).unwrap();
    fixture.write_outcome(&before);
    assert!(fixture
        .store
        .publish_no_commit(&fixture.reference, &fixture.evidence)
        .is_err());
    assert_eq!(before, fs::read(fixture.outcome_path()).unwrap());
    assert!(matches!(
        fixture.store.load_outcome(&fixture.reference).unwrap(),
        Some(DurableOwnerOutcome::StartupFailed(_))
    ));
}

#[test]
fn no_commit_closure_refuses_new_resource_budget() {
    let fixture = Fixture::new();
    fixture
        .store
        .publish_no_commit(&fixture.reference, &fixture.evidence)
        .unwrap();
    let budget = super::super::owner_resources::OwnerResourceBudget {
        memory_mib: 128,
        cpu_cores: 1,
        network_slots: 0,
        provider_slots: 0,
        browser_slots: 0,
        heavy: false,
        background: true,
    };
    assert!(fixture
        .store
        .retain_resources(&fixture.reference, budget)
        .is_err());
    assert!(fixture
        .store
        .load_resources(&fixture.reference)
        .unwrap()
        .is_none());
}

#[test]
fn protected_no_commit_observation_preserves_exact_credential_and_role() {
    let fixture = Fixture::new();
    // Synthetic conversion only; executable SDK cases prove the private channel.
    let proof = unsafe {
        OwnerObservationRecord::NotCommitted {}.into_protected_outcome(&fixture.reference)
    }
    .unwrap()
    .unwrap();
    let DurableOwnerOutcome::NotCommitted(proof) = proof else {
        panic!("no_commit_role_changed")
    };
    let credential = fixture.reference.credential();
    assert!(proof.authorizes(&credential, &[9; 32]));
    let mut wrong = credential.clone();
    wrong.nonce = Uuid::new_v4();
    assert!(!proof.authorizes(&wrong, &[9; 32]));
    assert!(!proof.authorizes(&credential, &[8; 32]));
    assert!(serde_json::from_value::<OwnerObservationRecord>(
        serde_json::json!({"kind": "not_committed", "snapshot": null})
    )
    .is_err());
}
