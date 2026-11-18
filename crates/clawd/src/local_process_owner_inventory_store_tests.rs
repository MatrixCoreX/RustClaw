use super::*;
use crate::local_process_owner::owner_resources::OwnerResourceBudget;
use crate::local_process_owner::owner_store::tests::{binding, credential, store};
use std::fs;
use std::os::unix::fs::PermissionsExt;

fn fixture() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("oi-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}
fn budget() -> OwnerResourceBudget {
    OwnerResourceBudget {
        memory_mib: 128,
        cpu_cores: 1,
        network_slots: 0,
        provider_slots: 0,
        browser_slots: 0,
        heavy: false,
        background: true,
    }
}

#[test]
fn inventory_store_enumerates_every_bound_pending_budget_without_public_pid_files() {
    let root = fixture();
    let store = store(root.path());
    let mut expected = std::collections::BTreeSet::new();
    for _ in 0..16 {
        let owner = credential(Uuid::new_v4());
        expected.insert(owner.job_id);
        let reference = store.publish(&owner, binding()).unwrap();
        store.retain_resources(&reference, budget()).unwrap();
    }
    let mut found = std::collections::BTreeSet::new();
    let counts = store
        .visit_pending_resources(|reference, actual| {
            assert_eq!(actual, &budget());
            found.insert(reference.credential().job_id);
            Ok(())
        })
        .unwrap();
    assert_eq!(counts.examined, 16);
    assert_eq!(counts.pending, 16);
    assert_eq!(found, expected);
}

#[test]
fn inventory_store_missing_budget_is_unknown_not_zero_or_omitted() {
    let root = fixture();
    let store = store(root.path());
    store
        .publish(&credential(Uuid::new_v4()), binding())
        .unwrap();
    assert!(matches!(
        store.visit_pending_resources(|_, _| panic!("unexpected_inventory_row")),
        Err(OwnerReferenceFailure::MissingResourceBudget)
    ));
}

#[test]
fn inventory_store_rejects_corrupt_orphan_and_noncanonical_records() {
    for case in ["corrupt", "orphan", "noncanonical", "symlink"] {
        let root = fixture();
        let store = store(root.path());
        let owner = credential(Uuid::new_v4());
        let reference = store.publish(&owner, binding()).unwrap();
        store.retain_resources(&reference, budget()).unwrap();
        match case {
            "corrupt" => fs::write(
                root.path().join(format!("{}.resources.json", owner.job_id)),
                b"unverified",
            )
            .unwrap(),
            "orphan" => fs::write(
                root.path()
                    .join(format!("{}.resources.json", Uuid::new_v4())),
                b"unverified",
            )
            .unwrap(),
            "noncanonical" => {
                fs::write(root.path().join("not-a-job.owner"), b"unverified").unwrap()
            }
            "symlink" => std::os::unix::fs::symlink(
                root.path().join(format!("{}.owner", owner.job_id.simple())),
                root.path()
                    .join(format!("{}.owner", Uuid::new_v4().simple())),
            )
            .unwrap(),
            _ => unreachable!(),
        }
        assert!(
            store.visit_pending_resources(|_, _| Ok(())).is_err(),
            "{case}"
        );
    }
}

#[test]
fn inventory_store_holds_a_fresh_exclusive_description_against_publication() {
    let root = fixture();
    let store = store(root.path());
    let reference = store
        .publish(&credential(Uuid::new_v4()), binding())
        .unwrap();
    store.retain_resources(&reference, budget()).unwrap();
    let later = credential(Uuid::new_v4());
    let counts = store
        .visit_pending_resources(|_, _| {
            // Same store object: a cloned-description flock would accidentally
            // downgrade the inventory lock and allow this concurrent publication.
            assert!(matches!(
                store.publish(&later, binding()),
                Err(OwnerReferenceFailure::Busy)
            ));
            assert!(store.load(later.job_id).unwrap().is_none());
            Ok(())
        })
        .unwrap();
    assert_eq!(counts.pending, 1);
    store.publish(&later, binding()).unwrap();
    assert!(store.load(later.job_id).unwrap().is_some());
}
