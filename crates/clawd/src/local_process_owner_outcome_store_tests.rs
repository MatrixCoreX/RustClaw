use std::fs;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;

use super::super::owner_outcome::tests::record;
use super::super::owner_protocol::OwnerFailure;
use super::tests::{binding, credential, key, store};
use super::*;

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("oo-{}", Uuid::new_v4().simple()));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }

    fn outcome(&self, job: Uuid) -> PathBuf {
        self.0.join(outcome_name(job).unwrap().to_str().unwrap())
    }

    fn write(&self, job: Uuid, bytes: &[u8]) {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(self.outcome(job))
            .unwrap()
            .write_all(bytes)
            .unwrap();
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn startup_outcome_recovery_requires_the_exact_reference_and_retains_failure_not_success() {
    let root = Directory::new();
    let expected = credential(Uuid::new_v4());
    let reference = store(&root.0).publish(&expected, binding()).unwrap();
    assert!(store(&root.0).load_outcome(&reference).unwrap().is_none());
    root.write(
        expected.job_id,
        &key()
            .sign_startup_failure(record(&expected, OwnerFailure::SpawnFailed))
            .unwrap(),
    );
    let reopened = store(&root.0);
    let recovered = match reopened.load_outcome(&reference).unwrap().unwrap() {
        DurableOwnerOutcome::StartupFailed(failure) => failure,
        DurableOwnerOutcome::Terminal(_) => panic!("startup failure became scope completion"),
        DurableOwnerOutcome::NotCommitted(_) => panic!("startup attempt became uncommitted"),
    };
    assert_eq!(recovered.error_code(), OwnerFailure::SpawnFailed);
    assert!(recovered.authorizes(&expected, &binding()));
    let mut wrong = expected.clone();
    wrong.nonce = Uuid::new_v4();
    assert!(!recovered.authorizes(&wrong, &binding()));
    assert!(!recovered.authorizes(&expected, &[8; 32]));
    assert!(matches!(
        reopened.load_terminal(&reference),
        Err(OwnerReferenceFailure::Conflict)
    ));
    let bytes = fs::read(root.outcome(expected.job_id)).unwrap();
    fs::remove_file(
        root.0
            .join(reference_name(expected.job_id).unwrap().to_str().unwrap()),
    )
    .unwrap();
    assert!(reopened.load_outcome(&reference).is_err());
    assert_eq!(fs::read(root.outcome(expected.job_id)).unwrap(), bytes);
}

#[test]
fn startup_outcome_recovery_rejects_valid_signatures_for_other_jobs_births_and_bindings() {
    for changed in 0..3 {
        let root = Directory::new();
        let expected = credential(Uuid::new_v4());
        let reference = store(&root.0).publish(&expected, binding()).unwrap();
        let mut value = serde_json::to_value(record(&expected, OwnerFailure::SpawnFailed)).unwrap();
        match changed {
            0 => value["reference"]["job_id"] = serde_json::json!(Uuid::new_v4()),
            1 => value["reference"]["owner"]["boot_id"] = serde_json::json!(Uuid::new_v4()),
            _ => value["reference"]["binding_digest"] = serde_json::json!(vec![8; 32]),
        }
        root.write(
            expected.job_id,
            &key()
                .sign_startup_failure(serde_json::from_value(value).unwrap())
                .unwrap(),
        );
        assert!(matches!(
            store(&root.0).load_outcome(&reference),
            Err(OwnerReferenceFailure::Conflict)
        ));
    }
}

#[test]
fn startup_outcome_recovery_refuses_links_modes_malformed_and_oversize_records() {
    for kind in 0..5 {
        let root = Directory::new();
        let expected = credential(Uuid::new_v4());
        let reference = store(&root.0).publish(&expected, binding()).unwrap();
        let outside = root.0.join("retained");
        fs::write(&outside, b"retained").unwrap();
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o600)).unwrap();
        let path = root.outcome(expected.job_id);
        match kind {
            0 => std::os::unix::fs::symlink(&outside, &path).unwrap(),
            1 => fs::hard_link(&outside, &path).unwrap(),
            2 => {
                root.write(
                    expected.job_id,
                    &key()
                        .sign_startup_failure(record(&expected, OwnerFailure::SpawnFailed))
                        .unwrap(),
                );
                fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            }
            3 => root.write(expected.job_id, b"invalid"),
            _ => root.write(expected.job_id, &vec![b' '; MAX_REFERENCE_BYTES + 1]),
        }
        assert!(store(&root.0).load_outcome(&reference).is_err());
        assert_eq!(fs::read(outside).unwrap(), b"retained");
    }
}

#[test]
fn terminal_and_startup_failure_compete_for_one_immutable_outcome_even_concurrently() {
    let root = Directory::new();
    let expected = credential(Uuid::new_v4());
    let reference = store(&root.0).publish(&expected, binding()).unwrap();
    let terminal = key()
        .sign_terminal(super::super::owner_terminal::tests::record(&expected))
        .unwrap();
    let failure = key()
        .sign_startup_failure(record(&expected, OwnerFailure::SpawnFailed))
        .unwrap();
    let barrier = std::sync::Barrier::new(8);
    std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for index in 0..8 {
            let (barrier, root, reference, terminal, failure) =
                (&barrier, &root, &reference, &terminal, &failure);
            workers.push(scope.spawn(move || {
                let store = store(&root.0);
                barrier.wait();
                store
                    .publish_record(
                        &outcome_name(reference.credential.job_id).unwrap(),
                        if index % 2 == 0 { terminal } else { failure },
                    )
                    .unwrap();
                store.load_outcome(reference).unwrap().is_some()
            }));
        }
        for worker in workers {
            assert!(worker.join().unwrap());
        }
    });
    let path = root.outcome(expected.job_id);
    let winner = fs::read(&path).unwrap();
    assert!(winner == terminal || winner == failure);
    let before = fs::metadata(&path).unwrap().ino();
    for bytes in [&terminal, &failure] {
        store(&root.0)
            .publish_record(&outcome_name(expected.job_id).unwrap(), bytes)
            .unwrap();
        assert_eq!(fs::metadata(&path).unwrap().ino(), before);
        assert_eq!(fs::read(&path).unwrap(), winner);
    }
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 2);
    assert!(store(&root.0).load_outcome(&reference).unwrap().is_some());
}
