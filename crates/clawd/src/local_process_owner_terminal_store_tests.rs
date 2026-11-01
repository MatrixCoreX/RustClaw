use std::fs;
#[cfg(target_os = "linux")]
use std::fs::File;
#[cfg(target_os = "linux")]
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
use super::super::owner_authority::OwnerAuthorityBroker;
use super::super::owner_store::tests as store_tests;
use super::tests::record;
use super::*;

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("ot-{}", uuid::Uuid::new_v4().simple()));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn terminal_path(root: &Path, job: uuid::Uuid) -> PathBuf {
    root.join(format!("{}.outcome", job.simple()))
}

fn write_record(root: &Path, expected: &OwnerCredential) {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(terminal_path(root, expected.job_id))
        .unwrap();
    file.write_all(&store_tests::key().sign_terminal(record(expected)).unwrap())
        .unwrap();
}

#[test]
fn owner_terminal_missing_record_is_not_completion_and_never_creates_a_file() {
    let root = Directory::new();
    let expected = store_tests::credential(uuid::Uuid::new_v4());
    let store = store_tests::store(&root.0);
    let reference = store.publish(&expected, store_tests::binding()).unwrap();
    assert!(store.load_terminal(&reference).unwrap().is_none());
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
}

#[test]
fn owner_terminal_cannot_be_recovered_without_its_exact_original_claim() {
    for missing in [false, true] {
        let root = Directory::new();
        let expected = store_tests::credential(uuid::Uuid::new_v4());
        let store = store_tests::store(&root.0);
        let reference = store.publish(&expected, store_tests::binding()).unwrap();
        write_record(&root.0, &expected);
        let claim = root.0.join(format!("{}.owner", expected.job_id.simple()));
        if missing {
            fs::remove_file(claim).unwrap();
        } else {
            fs::write(claim, b"corrupt").unwrap();
        }
        let before = fs::read(terminal_path(&root.0, expected.job_id)).unwrap();
        assert!(store.load_terminal(&reference).is_err());
        assert_eq!(
            fs::read(terminal_path(&root.0, expected.job_id)).unwrap(),
            before
        );
    }
}

#[test]
fn owner_terminal_valid_signature_for_another_job_birth_or_binding_is_refused() {
    for changed in 0..3 {
        let root = Directory::new();
        let expected = store_tests::credential(uuid::Uuid::new_v4());
        let store = store_tests::store(&root.0);
        let reference = store.publish(&expected, store_tests::binding()).unwrap();
        let mut receipt = serde_json::to_value(record(&expected)).unwrap();
        match changed {
            0 => receipt["reference"]["job_id"] = serde_json::json!(uuid::Uuid::new_v4()),
            1 => receipt["reference"]["owner"]["boot_id"] = serde_json::json!(uuid::Uuid::new_v4()),
            _ => receipt["reference"]["binding_digest"] = serde_json::json!(vec![8; 32]),
        }
        let receipt = serde_json::from_value(receipt).unwrap();
        fs::write(
            terminal_path(&root.0, expected.job_id),
            store_tests::key().sign_terminal(receipt).unwrap(),
        )
        .unwrap();
        fs::set_permissions(
            terminal_path(&root.0, expected.job_id),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        assert!(matches!(
            store.load_terminal(&reference),
            Err(OwnerReferenceFailure::Conflict)
        ));
    }
}

#[test]
fn owner_terminal_recovery_rejects_wrong_nonce_and_preserves_nonzero_exit() {
    let root = Directory::new();
    let expected = store_tests::credential(uuid::Uuid::new_v4());
    let store = store_tests::store(&root.0);
    let reference = store.publish(&expected, store_tests::binding()).unwrap();
    write_record(&root.0, &expected);
    let receipt = store.load_terminal(&reference).unwrap().unwrap();
    assert!(receipt.authorizes(&expected, &store_tests::binding()));
    let mut wrong = expected.clone();
    wrong.nonce = uuid::Uuid::new_v4();
    assert!(!receipt.authorizes(&wrong, &store_tests::binding()));
    assert!(!receipt.authorizes(&expected, &[8; 32]));
    assert_eq!(receipt.snapshot().root_exit.unwrap().code, Some(7));
}

#[test]
fn owner_terminal_recovery_refuses_links_special_files_modes_and_oversize_records() {
    for kind in 0..5 {
        let root = Directory::new();
        let expected = store_tests::credential(uuid::Uuid::new_v4());
        let store = store_tests::store(&root.0);
        let reference = store.publish(&expected, store_tests::binding()).unwrap();
        let path = terminal_path(&root.0, expected.job_id);
        let outside = root.0.join("retained");
        fs::write(&outside, b"retained").unwrap();
        fs::set_permissions(&outside, fs::Permissions::from_mode(0o600)).unwrap();
        match kind {
            0 => std::os::unix::fs::symlink(&outside, &path).unwrap(),
            1 => fs::hard_link(&outside, &path).unwrap(),
            2 => {
                let name = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
                assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
            }
            3 => {
                write_record(&root.0, &expected);
                fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            }
            _ => {
                fs::write(
                    &path,
                    vec![b' '; super::super::owner_reference::MAX_REFERENCE_BYTES + 1],
                )
                .unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            }
        }
        assert!(store.load_terminal(&reference).is_err());
        assert_eq!(fs::read(&outside).unwrap(), b"retained");
    }
}

#[cfg(target_os = "linux")]
fn isolated(case: &str) {
    use std::io::Write;
    let root = Directory::new();
    let broker =
        OwnerAuthorityBroker::from_admitted_directory(File::open(&root.0).unwrap()).unwrap();
    let store = broker.initialize_new().unwrap();
    let job = uuid::Uuid::new_v4();
    let (child, mut control) = super::super::launch_channel::tests::spawn_fixture(
        case,
        &root.0,
        job,
        Some(store.authority_fingerprint().unwrap()),
    );
    let expected = store
        .credential_for(
            job,
            super::super::instance::observe_process(child.id())
                .unwrap()
                .birth,
            store_tests::binding(),
        )
        .unwrap();
    store.publish(&expected, store_tests::binding()).unwrap();
    control.write_all(b"G").unwrap();
    drop(control);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    print!("{}", String::from_utf8_lossy(&output.stdout));
    let reopened = broker
        .load_expected(&store.authority_fingerprint().unwrap())
        .unwrap();
    let reference = reopened.load(job).unwrap().unwrap();
    if case == "terminal_corrupt" {
        assert!(reopened.load_terminal(&reference).is_err());
    } else if matches!(case, "terminal_lost" | "terminal_write_failure") {
        assert!(reopened.load_terminal(&reference).unwrap().is_none());
    } else {
        let receipt = reopened.load_terminal(&reference).unwrap().unwrap();
        assert!(receipt.authorizes(&expected, &store_tests::binding()));
        assert_eq!(receipt.snapshot().phase, OwnerPhase::Terminal);
        assert_eq!(receipt.snapshot().root_exit.unwrap().code, Some(7));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn owner_terminal_real_completion_is_recoverable_after_the_owner_process_exits() {
    isolated("terminal_complete");
}
#[cfg(target_os = "linux")]
#[test]
fn owner_terminal_root_exit_does_not_complete_an_adopted_live_descendant() {
    isolated("terminal_descendant");
}
#[cfg(target_os = "linux")]
#[test]
fn owner_terminal_cancellation_ack_requires_descendants_to_exit_before_publication() {
    isolated("terminal_cancel");
}
#[cfg(target_os = "linux")]
#[test]
fn owner_terminal_lost_child_authority_never_publishes_completion() {
    isolated("terminal_lost");
}
#[cfg(target_os = "linux")]
#[test]
fn owner_terminal_concurrent_republication_is_immutable_and_idempotent() {
    isolated("terminal_concurrent");
}
#[cfg(target_os = "linux")]
#[test]
fn owner_terminal_real_write_failure_never_returns_a_durable_proof() {
    isolated("terminal_write_failure");
}

#[cfg(target_os = "linux")]
#[test]
fn owner_terminal_another_job_cannot_publish_using_the_completed_scopes_evidence() {
    isolated("terminal_wrong_proof");
}

#[cfg(target_os = "linux")]
#[test]
fn owner_terminal_corrupt_existing_publication_is_preserved_and_never_replaced() {
    isolated("terminal_corrupt");
}

#[cfg(target_os = "linux")]
pub(crate) fn run_case(case: &str) {
    use super::super::owner_protocol::{OwnerAction, OwnerRequest};
    use super::super::owner_runtime::OwnerRuntime;
    use std::io::Read;
    use std::os::fd::FromRawFd;
    use std::os::unix::net::UnixStream;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let root = PathBuf::from(std::env::var_os("APP_OWNER_REFERENCE_TEST_ROOT").unwrap());
    let job =
        uuid::Uuid::parse_str(&std::env::var("APP_OWNER_REFERENCE_TEST_JOB").unwrap()).unwrap();
    let fingerprint: [u8; 32] =
        serde_json::from_str(&std::env::var("APP_OWNER_AUTHORITY_TEST_FINGERPRINT").unwrap())
            .unwrap();
    let fingerprint =
        super::super::owner_reference::AuthorityFingerprint::from_slice(&fingerprint).unwrap();
    let broker = OwnerAuthorityBroker::from_admitted_directory(File::open(&root).unwrap()).unwrap();
    let store = broker.load_expected(&fingerprint).unwrap();
    let mut control = unsafe { UnixStream::from_raw_fd(198) };
    control
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut ready = [0; 1];
    control.read_exact(&mut ready).unwrap();
    assert_eq!(&ready, b"G");
    drop(control);
    let reference = store.load(job).unwrap().unwrap();
    let mut command = Command::new("/bin/sh");
    let script = if matches!(case, "terminal_descendant" | "terminal_cancel") {
        "exec 3<&0; /bin/sh -c 'printf ready > \"$1\"; read -r value' child \"$1\" <&3 & exit 7"
    } else {
        "exit 7"
    };
    command
        .args(["-c", script, "fixture"])
        .arg(root.join("ready"))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut runtime = unsafe {
        OwnerRuntime::spawn_bound(&mut command, reference.credential(), Duration::from_secs(1))
    }
    .unwrap();
    let input = runtime.take_stdin().unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    if case == "terminal_lost" {
        let mut status = 0;
        assert!(unsafe { libc::waitpid(-1, &mut status, 0) } > 1);
        assert!(matches!(
            runtime.terminal_evidence(),
            Err(super::super::owner_protocol::OwnerFailure::OwnershipLost)
        ));
        assert!(store.load_terminal(&reference).unwrap().is_none());
        return;
    }
    if matches!(case, "terminal_descendant" | "terminal_cancel") {
        loop {
            let snapshot = runtime.tick().unwrap();
            if snapshot.root_exit.is_some() && root.join("ready").exists() {
                break;
            }
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(runtime.terminal_evidence().unwrap().is_none());
        assert!(store.load_terminal(&reference).unwrap().is_none());
        if case == "terminal_cancel" {
            runtime
                .handle(&OwnerRequest {
                    schema_version: 1,
                    credential: reference.credential(),
                    request_id: 1,
                    action: OwnerAction::Cancel,
                })
                .unwrap();
        }
    }
    drop(input);
    let evidence = loop {
        if let Some(evidence) = runtime.terminal_evidence().unwrap() {
            break evidence;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(2));
    };
    if case == "terminal_write_failure" {
        let mut previous = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        assert_eq!(
            unsafe { libc::getrlimit(libc::RLIMIT_FSIZE, &mut previous) },
            0
        );
        let limit = libc::rlimit {
            rlim_cur: 1,
            rlim_max: previous.rlim_max,
        };
        assert_ne!(
            unsafe { libc::signal(libc::SIGXFSZ, libc::SIG_IGN) },
            libc::SIG_ERR
        );
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_FSIZE, &limit) }, 0);
        let result = store.publish_terminal(&reference, &evidence);
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_FSIZE, &previous) }, 0);
        assert!(matches!(
            result,
            Err(OwnerReferenceFailure::StorageUnavailable)
        ));
        assert!(store.load_terminal(&reference).unwrap().is_none());
        assert!(store.load(job).unwrap().is_some());
        let directory = super::super::owner_authority::tests::key_file(&root)
            .parent()
            .unwrap()
            .to_path_buf();
        assert_eq!(fs::read_dir(directory).unwrap().count(), 2);
        return;
    }
    let directory = super::super::owner_authority::tests::key_file(&root)
        .parent()
        .unwrap()
        .to_path_buf();
    if case == "terminal_corrupt" {
        let path = terminal_path(&directory, job);
        fs::write(&path, b"retained").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let inode = fs::metadata(&path).unwrap().ino();
        assert!(matches!(
            store.publish_terminal(&reference, &evidence),
            Err(OwnerReferenceFailure::InvalidReference)
        ));
        assert_eq!(fs::read(&path).unwrap(), b"retained");
        assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 3);
        return;
    }
    if case == "terminal_wrong_proof" {
        let wrong = store
            .credential_for(
                uuid::Uuid::new_v4(),
                reference.credential().owner,
                store_tests::binding(),
            )
            .unwrap();
        let wrong = store.publish(&wrong, store_tests::binding()).unwrap();
        assert!(matches!(
            store.publish_terminal(&wrong, &evidence),
            Err(OwnerReferenceFailure::AuthenticationFailed)
        ));
        assert!(store.load_terminal(&wrong).unwrap().is_none());
        assert!(store.load_terminal(&reference).unwrap().is_none());
    }
    if case == "terminal_concurrent" {
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let (store, reference, evidence, barrier) =
                    (&store, &reference, &evidence, &barrier);
                scope.spawn(move || {
                    barrier.wait();
                    store.publish_terminal(reference, evidence).unwrap();
                });
            }
        });
    }
    let before = store.publish_terminal(&reference, &evidence).unwrap();
    let path = terminal_path(&directory, job);
    let inode = fs::metadata(&path).unwrap().ino();
    for _ in 0..3 {
        store.publish_terminal(&reference, &evidence).unwrap();
    }
    assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
    assert_eq!(
        fs::read_dir(&directory).unwrap().count(),
        if case == "terminal_wrong_proof" { 4 } else { 3 }
    );
    assert!(!String::from_utf8_lossy(&fs::read(path).unwrap())
        .contains(&reference.credential().nonce.to_string()));
    assert_eq!(
        before.snapshot().completion,
        Some(if case == "terminal_cancel" {
            super::super::owner_protocol::OwnerCompletion::Cancelled
        } else {
            super::super::owner_protocol::OwnerCompletion::Completed
        })
    );
}
