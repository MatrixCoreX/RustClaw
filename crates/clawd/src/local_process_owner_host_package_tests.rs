use super::*;
use crate::local_process_owner::owner_runtime::OwnerRuntime;
use crate::local_process_owner::package_test_support::custody_fixture;
use std::fs::{self, OpenOptions};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

const BINDING: [u8; 32] = [91; 32];

fn credential() -> OwnerCredential {
    OwnerRuntime::prepare_credential(uuid::Uuid::new_v4(), uuid::Uuid::new_v4()).unwrap()
}

fn package(root: &Path, owner: &OwnerCredential) -> PackageVersionCustody {
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
    let lease = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(root.join("version.lock"))
        .unwrap();
    custody_fixture(lease, &root.join("custody.json"), owner, BINDING)
}

fn record(root: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(root.join("custody.json")).unwrap()).unwrap()
}

#[test]
fn exact_untransferred_custody_is_released_before_transport_entry() {
    let root = tempfile::tempdir().unwrap();
    let owner = credential();
    let custody = package(root.path(), &owner);
    let guarded = UntransferredOwnerPackage::new(Some(custody), &owner, BINDING).unwrap();
    assert_eq!(record(root.path())["released"], false);
    drop(guarded);
    assert_eq!(record(root.path())["released"], true);
    drop(UntransferredOwnerPackage::new(None, &owner, BINDING).unwrap());
}

#[test]
fn entering_transport_disarms_release_even_when_the_capsule_is_dropped() {
    let root = tempfile::tempdir().unwrap();
    let owner = credential();
    let custody = package(root.path(), &owner);
    let guarded = UntransferredOwnerPackage::new(Some(custody), &owner, BINDING).unwrap();
    drop(guarded.into_transfer());
    assert_eq!(record(root.path())["released"], false);
}

#[test]
fn another_owner_or_execution_binding_cannot_release_custody() {
    for different_owner in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let owner = credential();
        let custody = package(root.path(), &owner);
        let before = fs::read(root.path().join("custody.json")).unwrap();
        let mut expected = owner.clone();
        let binding = if different_owner {
            expected.nonce = uuid::Uuid::new_v4();
            BINDING
        } else {
            [92; 32]
        };
        assert!(matches!(
            UntransferredOwnerPackage::new(Some(custody), &expected, binding),
            Err(LaunchChannelFailure::CredentialMismatch)
        ));
        assert_eq!(fs::read(root.path().join("custody.json")).unwrap(), before);
        assert_eq!(record(root.path())["released"], false);
    }
}

#[test]
fn corrupted_bound_record_is_preserved_without_release_or_repair() {
    let root = tempfile::tempdir().unwrap();
    let owner = credential();
    let custody = package(root.path(), &owner);
    let guarded = UntransferredOwnerPackage::new(Some(custody), &owner, BINDING).unwrap();
    let mut changed = record(root.path());
    changed["binding"]["execution_binding_digest"] = serde_json::to_value([93u8; 32]).unwrap();
    let bytes = serde_json::to_vec(&changed).unwrap();
    fs::write(root.path().join("custody.json"), &bytes).unwrap();
    drop(guarded);
    assert_eq!(fs::read(root.path().join("custody.json")).unwrap(), bytes);
    assert_eq!(record(root.path())["released"], false);
}
