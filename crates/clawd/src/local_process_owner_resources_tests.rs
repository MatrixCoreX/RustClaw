use super::*;
use crate::local_process_owner::instance::observe_process;
use crate::local_process_owner::owner_reference::OwnerAuthorityKey;
use crate::local_process_owner::owner_store::OwnerReferenceStore;
use std::fs::File;
use std::os::unix::fs::PermissionsExt;
use uuid::Uuid;

fn budget() -> OwnerResourceBudget {
    OwnerResourceBudget {
        memory_mib: 512,
        cpu_cores: 1,
        network_slots: 0,
        provider_slots: 0,
        browser_slots: 0,
        heavy: true,
        background: true,
    }
}

fn fixture() -> (
    tempfile::TempDir,
    OwnerReferenceStore,
    DurableOwnerReference,
) {
    let directory = tempfile::Builder::new()
        .prefix("rr-")
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let store = OwnerReferenceStore::from_admitted_directory(
        File::open(directory.path()).unwrap(),
        OwnerAuthorityKey::new([37; 32]).unwrap(),
    )
    .unwrap();
    let credential = store
        .credential_for(
            Uuid::new_v4(),
            observe_process(std::process::id()).unwrap().birth,
            [62; 32],
        )
        .unwrap();
    let reference = store.publish(&credential, [62; 32]).unwrap();
    (directory, store, reference)
}

#[test]
fn resource_budget_bounds_reject_zero_and_excessive_accounting() {
    assert!(budget().validate().is_ok());
    for field in [
        "memory_zero",
        "memory",
        "cpu_zero",
        "cpu",
        "network",
        "provider",
        "browser",
    ] {
        let mut value = budget();
        match field {
            "memory_zero" => value.memory_mib = 0,
            "memory" => value.memory_mib = u64::MAX,
            "cpu_zero" => value.cpu_cores = 0,
            "cpu" => value.cpu_cores = 4_097,
            "network" => value.network_slots = 4_097,
            "provider" => value.provider_slots = 4_097,
            "browser" => value.browser_slots = 65,
            _ => unreachable!(),
        }
        assert!(value.validate().is_err());
    }
}

#[test]
fn resource_budget_wire_rejects_unknown_fields() {
    let mut value = serde_json::to_value(budget()).unwrap();
    value["unrecognized"] = serde_json::json!(true);
    assert!(serde_json::from_value::<OwnerResourceBudget>(value).is_err());
}

#[test]
fn resource_signature_rejects_tampering_and_wrong_authority() {
    let (_directory, _store, reference) = fixture();
    let key = OwnerAuthorityKey::new([37; 32]).unwrap();
    let record = OwnerResourceRecord {
        schema_version: 1,
        reference: reference.record().clone(),
        budget: budget(),
    };
    let bytes = key.sign_resources(record.clone()).unwrap();
    assert!(key.verify_resources(&bytes).unwrap() == record);
    assert_eq!(
        OwnerAuthorityKey::new([38; 32])
            .unwrap()
            .verify_resources(&bytes)
            .err(),
        Some(OwnerReferenceFailure::AuthenticationFailed)
    );
    let mut changed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    changed["record"]["budget"]["memory_mib"] = serde_json::json!(513);
    assert_eq!(
        key.verify_resources(&serde_json::to_vec(&changed).unwrap())
            .err(),
        Some(OwnerReferenceFailure::AuthenticationFailed)
    );
    assert!(key.verify(&bytes).is_err());
}

#[test]
fn resource_signature_rejects_schema_and_unknown_record_fields() {
    let (_directory, _store, reference) = fixture();
    let key = OwnerAuthorityKey::new([37; 32]).unwrap();
    let bytes = key
        .sign_resources(OwnerResourceRecord {
            schema_version: 1,
            reference: reference.record().clone(),
            budget: budget(),
        })
        .unwrap();
    for path in ["", "/record", "/record/budget"] {
        let mut changed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        changed.pointer_mut(path).unwrap()["extra"] = serde_json::json!(true);
        assert!(key
            .verify_resources(&serde_json::to_vec(&changed).unwrap())
            .is_err());
    }
    let mut changed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    changed["record"]["schema_version"] = serde_json::json!(2);
    assert!(key
        .verify_resources(&serde_json::to_vec(&changed).unwrap())
        .is_err());
}

#[test]
fn resource_store_is_immutable_idempotent_and_reopens() {
    let (directory, store, reference) = fixture();
    assert!(store.load_resources(&reference).unwrap().is_none());
    store.retain_resources(&reference, budget()).unwrap();
    store.retain_resources(&reference, budget()).unwrap();
    let mut changed = budget();
    changed.memory_mib += 1;
    assert_eq!(
        store.retain_resources(&reference, changed).err(),
        Some(OwnerReferenceFailure::Conflict)
    );
    drop(store);
    let reopened = OwnerReferenceStore::from_admitted_directory(
        File::open(directory.path()).unwrap(),
        OwnerAuthorityKey::new([37; 32]).unwrap(),
    )
    .unwrap();
    let reloaded = reopened
        .load(reference.credential().job_id)
        .unwrap()
        .unwrap();
    assert_eq!(reopened.load_resources(&reloaded).unwrap(), Some(budget()));
}

#[test]
fn resource_store_rejects_modified_private_record() {
    let (directory, store, reference) = fixture();
    store.retain_resources(&reference, budget()).unwrap();
    let path = directory
        .path()
        .join(format!("{}.resources.json", reference.credential().job_id));
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    value["record"]["budget"]["provider_slots"] = serde_json::json!(1);
    std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(
        store.load_resources(&reference).err(),
        Some(OwnerReferenceFailure::AuthenticationFailed)
    );
}
