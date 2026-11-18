use std::fs::{self, File};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::PathBuf;

use super::super::owner_store::tests::binding;
use super::*;

struct Directory(PathBuf);

pub(crate) fn key_file(root: &std::path::Path) -> PathBuf {
    root.join(DIRECTORY).join(KEY_FILE)
}
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("ka-{}", uuid::Uuid::new_v4().simple()));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }

    fn broker(&self) -> OwnerAuthorityBroker {
        OwnerAuthorityBroker::from_admitted_directory(File::open(&self.0).unwrap()).unwrap()
    }

    fn key_path(&self) -> PathBuf {
        key_file(&self.0)
    }
}

#[test]
fn owner_authority_expected_identity_refuses_a_valid_format_replacement_key() {
    let root = Directory::new();
    let broker = root.broker();
    let store = broker.initialize_new().unwrap();
    let fingerprint = store.authority_fingerprint().unwrap();
    let expected = credential(&store, uuid::Uuid::new_v4());
    store.publish(&expected, binding()).unwrap();
    let mut bytes = Zeroizing::new(fs::read(root.key_path()).unwrap());
    bytes[KEY_HEADER.len()] ^= 1;
    fs::write(root.key_path(), &bytes).unwrap();
    let error = broker.load_expected(&fingerprint).err().unwrap();
    assert_eq!(error.operation, AuthorityOperation::VerifyIdentity);
    assert_eq!(error.error_code, AuthorityFailureCode::AuthorityMismatch);
    assert!(fs::read(root.key_path()).unwrap().as_slice() == bytes.as_slice());
    assert!(store
        .load(expected.job_id)
        .unwrap()
        .unwrap()
        .authorizes(&expected, &binding()));
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn credential(
    store: &OwnerReferenceStore,
    job: uuid::Uuid,
) -> super::super::owner_protocol::OwnerCredential {
    store
        .credential_for(
            job,
            super::super::instance::observe_process(std::process::id())
                .unwrap()
                .birth,
            binding(),
        )
        .unwrap()
}

#[test]
fn owner_authority_reopens_stable_key_and_authenticates_preexisting_references() {
    let root = Directory::new();
    let broker = root.broker();
    let store = broker.initialize_new().unwrap();
    let expected = credential(&store, uuid::Uuid::new_v4());
    store.publish(&expected, binding()).unwrap();
    let before = fs::metadata(root.key_path()).unwrap();
    assert_eq!(before.mode() & 0o7777, 0o600);
    assert_eq!(before.len(), KEY_BYTES as u64);
    assert_eq!(
        fs::metadata(root.0.join(DIRECTORY)).unwrap().mode() & 0o7777,
        0o700
    );
    drop(store);
    drop(broker);
    let reopened = root.broker().load_existing().unwrap();
    let proof = reopened.load(expected.job_id).unwrap().unwrap();
    assert!(proof.authorizes(&expected, &binding()));
    assert!(credential(&reopened, expected.job_id) == expected);
    assert_eq!(before.ino(), fs::metadata(root.key_path()).unwrap().ino());
    assert_eq!(fs::read_dir(root.0.join(DIRECTORY)).unwrap().count(), 2);
}

#[test]
fn owner_authority_initialization_is_exclusive_even_when_the_key_is_missing() {
    let root = Directory::new();
    let broker = root.broker();
    let store = broker.initialize_new().unwrap();
    let expected = credential(&store, uuid::Uuid::new_v4());
    store.publish(&expected, binding()).unwrap();
    fs::remove_file(root.key_path()).unwrap();
    let missing = broker.load_existing().err().unwrap();
    assert_eq!(missing.operation, AuthorityOperation::OpenKey);
    assert_eq!(missing.error_code, AuthorityFailureCode::Missing);
    assert_eq!(
        broker.initialize_new().err().unwrap().error_code,
        AuthorityFailureCode::AlreadyInitialized
    );
    assert!(!root.key_path().exists());
    assert!(store
        .load(expected.job_id)
        .unwrap()
        .unwrap()
        .authorizes(&expected, &binding()));
    assert_eq!(fs::read_dir(root.0.join(DIRECTORY)).unwrap().count(), 1);
}

#[test]
fn owner_authority_missing_or_partial_initialization_is_not_automatically_repaired() {
    let root = Directory::new();
    let broker = root.broker();
    let missing = broker.load_existing().err().unwrap();
    assert_eq!(missing.operation, AuthorityOperation::OpenDirectory);
    assert_eq!(missing.error_code, AuthorityFailureCode::Missing);
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 0);
    fs::DirBuilder::new()
        .mode(0o700)
        .create(root.0.join(DIRECTORY))
        .unwrap();
    assert_eq!(
        broker.load_existing().err().unwrap().error_code,
        AuthorityFailureCode::Missing
    );
    assert_eq!(
        broker.initialize_new().err().unwrap().error_code,
        AuthorityFailureCode::AlreadyInitialized
    );
    assert_eq!(fs::read_dir(root.0.join(DIRECTORY)).unwrap().count(), 0);
}

#[test]
fn owner_authority_corrupt_key_is_retained_and_never_replaced() {
    for kind in 0..5 {
        let root = Directory::new();
        let broker = root.broker();
        let store = broker.initialize_new().unwrap();
        let expected = credential(&store, uuid::Uuid::new_v4());
        store.publish(&expected, binding()).unwrap();
        let mut bytes = Zeroizing::new(fs::read(root.key_path()).unwrap());
        match kind {
            0 => bytes.clear(),
            1 => {
                bytes.pop();
            }
            2 => bytes.push(0),
            3 => bytes[0] = b'?',
            _ => bytes[KEY_HEADER.len()..].fill(0),
        }
        fs::write(root.key_path(), &bytes).unwrap();
        let failure = broker.load_existing().err().unwrap();
        assert_eq!(failure.operation, AuthorityOperation::ReadKey);
        assert_eq!(failure.error_code, AuthorityFailureCode::InvalidKey);
        assert_eq!(
            broker.initialize_new().err().unwrap().error_code,
            AuthorityFailureCode::AlreadyInitialized
        );
        assert!(fs::read(root.key_path()).unwrap().as_slice() == bytes.as_slice());
        assert!(store
            .load(expected.job_id)
            .unwrap()
            .unwrap()
            .authorizes(&expected, &binding()));
    }
}

#[test]
fn owner_authority_different_initializations_do_not_share_control_credentials() {
    let first = Directory::new();
    let second = Directory::new();
    let job = uuid::Uuid::new_v4();
    assert!(
        credential(&first.broker().initialize_new().unwrap(), job)
            != credential(&second.broker().initialize_new().unwrap(), job)
    );
}

#[test]
fn owner_authority_concurrent_initializers_have_one_winner() {
    let root = Directory::new();
    let barrier = std::sync::Barrier::new(8);
    let job = uuid::Uuid::new_v4();
    let winner = std::thread::scope(|scope| {
        let mut threads = Vec::new();
        for _ in 0..8 {
            let (barrier, root) = (&barrier, &root);
            threads.push(scope.spawn(move || {
                let broker = root.broker();
                barrier.wait();
                match broker.initialize_new() {
                    Ok(store) => Some(credential(&store, job)),
                    Err(failure) => {
                        assert_eq!(failure.operation, AuthorityOperation::CreateDirectory);
                        assert_eq!(failure.error_code, AuthorityFailureCode::AlreadyInitialized);
                        None
                    }
                }
            }));
        }
        let winners = threads
            .into_iter()
            .filter_map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(winners.len(), 1);
        winners.into_iter().next().unwrap()
    });
    assert!(credential(&root.broker().load_existing().unwrap(), job) == winner);
    assert_eq!(fs::read_dir(root.0.join(DIRECTORY)).unwrap().count(), 1);
}

#[test]
fn owner_authority_parent_directory_is_pinned_across_path_replacement() {
    let root = Directory::new();
    let broker = root.broker();
    let expected = credential(&broker.initialize_new().unwrap(), uuid::Uuid::new_v4());
    let moved = Directory(root.0.with_extension("pinned"));
    fs::rename(&root.0, &moved.0).unwrap();
    fs::DirBuilder::new().mode(0o700).create(&root.0).unwrap();
    assert!(credential(&broker.load_existing().unwrap(), expected.job_id) == expected);
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 0);
}

#[test]
fn owner_authority_rejects_unsafe_parent_directory_descriptors() {
    let root = Directory::new();
    fs::set_permissions(&root.0, fs::Permissions::from_mode(0o755)).unwrap();
    let invalid = OwnerAuthorityBroker::from_admitted_directory(File::open(&root.0).unwrap())
        .err()
        .unwrap();
    assert_eq!(invalid.operation, AuthorityOperation::AdmitParent);
    assert_eq!(invalid.error_code, AuthorityFailureCode::InvalidDirectory);
    fs::set_permissions(&root.0, fs::Permissions::from_mode(0o700)).unwrap();
    let file = File::create(root.0.join("not-a-directory")).unwrap();
    assert_eq!(
        OwnerAuthorityBroker::from_admitted_directory(file)
            .err()
            .unwrap()
            .error_code,
        AuthorityFailureCode::InvalidDirectory
    );
    let file = File::open(&root.0).unwrap();
    assert_eq!(
        unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, 0) },
        0
    );
    assert_eq!(
        OwnerAuthorityBroker::from_admitted_directory(file)
            .err()
            .unwrap()
            .error_code,
        AuthorityFailureCode::InvalidDirectory
    );
}

#[test]
fn owner_authority_revalidates_parent_and_authority_directory_modes() {
    let root = Directory::new();
    let broker = root.broker();
    broker.initialize_new().unwrap();
    fs::set_permissions(&root.0, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        broker.load_existing().err().unwrap().operation,
        AuthorityOperation::AdmitParent
    );
    fs::set_permissions(&root.0, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(root.0.join(DIRECTORY), fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        broker.load_existing().err().unwrap().operation,
        AuthorityOperation::OpenDirectory
    );
}

#[test]
fn owner_authority_refuses_symlinked_directories_and_unsafe_key_files() {
    use std::os::unix::fs::symlink;

    let outside = Directory::new();
    let root = Directory::new();
    symlink(&outside.0, root.0.join(DIRECTORY)).unwrap();
    assert!(root.broker().load_existing().is_err());
    assert_eq!(
        root.broker().initialize_new().err().unwrap().error_code,
        AuthorityFailureCode::AlreadyInitialized
    );
    assert_eq!(fs::read_dir(&outside.0).unwrap().count(), 0);

    for kind in 0..5 {
        let root = Directory::new();
        let broker = root.broker();
        broker.initialize_new().unwrap();
        let retained = root.0.join("retained-key");
        let original = fs::metadata(root.key_path()).unwrap().ino();
        fs::rename(root.key_path(), &retained).unwrap();
        match kind {
            0 => symlink(&retained, root.key_path()).unwrap(),
            1 => fs::hard_link(&retained, root.key_path()).unwrap(),
            2 => {
                fs::copy(&retained, root.key_path()).unwrap();
                fs::set_permissions(root.key_path(), fs::Permissions::from_mode(0o644)).unwrap();
            }
            3 => fs::create_dir(root.key_path()).unwrap(),
            _ => {
                use std::os::unix::ffi::OsStrExt;
                let path = CString::new(root.key_path().as_os_str().as_bytes()).unwrap();
                assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
            }
        }
        assert!(broker.load_existing().is_err());
        assert_eq!(
            broker.initialize_new().err().unwrap().error_code,
            AuthorityFailureCode::AlreadyInitialized
        );
        assert_eq!(fs::metadata(&retained).unwrap().ino(), original);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn owner_authority_real_parent_sync_failure_never_returns_a_store_or_reinitializes() {
    use std::os::unix::fs::OpenOptionsExt;

    let root = Directory::new();
    let parent = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_CLOEXEC)
        .open(&root.0)
        .unwrap();
    let broker = OwnerAuthorityBroker::from_admitted_directory(parent).unwrap();
    let failure = broker.initialize_new().err().unwrap();
    assert_eq!(failure.operation, AuthorityOperation::SyncParent);
    assert_eq!(failure.error_code, AuthorityFailureCode::StorageUnavailable);
    assert_eq!(failure.os_error, Some(libc::EBADF));
    let inode = fs::metadata(root.key_path()).unwrap().ino();
    assert_eq!(
        broker.initialize_new().err().unwrap().error_code,
        AuthorityFailureCode::AlreadyInitialized
    );
    assert_eq!(
        broker.load_existing().err().unwrap().operation,
        AuthorityOperation::SyncParent
    );
    let recovered = root.broker().load_existing().unwrap();
    let expected = credential(&recovered, uuid::Uuid::new_v4());
    assert!(credential(&root.broker().load_existing().unwrap(), expected.job_id) == expected);
    assert_eq!(fs::metadata(root.key_path()).unwrap().ino(), inode);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_authority_actual_fresh_process_reloads_the_private_key_and_reference() {
    let root = Directory::new();
    let store = root.broker().initialize_new().unwrap();
    let expected = credential(&store, uuid::Uuid::new_v4());
    store.publish(&expected, binding()).unwrap();
    drop(store);
    run_process(&root, "authority_reload", expected.job_id);
    assert!(root
        .broker()
        .load_existing()
        .unwrap()
        .load(expected.job_id)
        .unwrap()
        .unwrap()
        .authorizes(&expected, &binding()));
}

#[cfg(target_os = "linux")]
#[test]
fn owner_authority_actual_key_write_failure_preserves_an_unusable_claim() {
    let root = Directory::new();
    run_process(&root, "authority_write_refused", uuid::Uuid::new_v4());
    assert_eq!(fs::metadata(root.key_path()).unwrap().len(), 0);
    let broker = root.broker();
    assert_eq!(
        broker.load_existing().err().unwrap().error_code,
        AuthorityFailureCode::InvalidKey
    );
    assert_eq!(
        broker.initialize_new().err().unwrap().error_code,
        AuthorityFailureCode::AlreadyInitialized
    );
    assert_eq!(fs::metadata(root.key_path()).unwrap().len(), 0);
}

#[cfg(target_os = "linux")]
#[test]
fn owner_authority_actual_reference_store_fd_failure_is_not_a_directory_error() {
    let root = Directory::new();
    root.broker().initialize_new().unwrap();
    run_process(&root, "authority_fd_exhausted", uuid::Uuid::new_v4());
    assert!(root.broker().load_existing().is_ok());
}

#[cfg(target_os = "linux")]
fn run_process(root: &Directory, case: &str, job: uuid::Uuid) {
    let result = super::super::owner_scope::tests::fixture_command(case, None)
        .env("APP_OWNER_REFERENCE_TEST_ROOT", &root.0)
        .env("APP_OWNER_REFERENCE_TEST_JOB", job.to_string())
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    println!("{}", String::from_utf8_lossy(&result.stdout));
}

#[cfg(target_os = "linux")]
pub(crate) fn run_special_case(case: &str) {
    let root = PathBuf::from(std::env::var_os("APP_OWNER_REFERENCE_TEST_ROOT").unwrap());
    let broker = OwnerAuthorityBroker::from_admitted_directory(File::open(&root).unwrap()).unwrap();
    if case == "authority_fd_exhausted" {
        let directory = File::open(root.join(DIRECTORY)).unwrap();
        let key = super::super::owner_store::tests::key();
        let limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) }, 0);
        let failure = broker.finish(directory, key).err().unwrap();
        assert_eq!(failure.operation, AuthorityOperation::OpenReferenceStore);
        assert_eq!(failure.error_code, AuthorityFailureCode::StorageUnavailable);
        // The lower-level store has no preserved errno; do not invent one.
        assert_eq!(failure.os_error, None);
        return;
    }
    if case == "authority_write_refused" {
        // Change limits/signals only in this dedicated helper, not libtest's
        // parent process. Pipe-based test output is not a regular file write.
        assert_ne!(
            unsafe { libc::signal(libc::SIGXFSZ, libc::SIG_IGN) },
            libc::SIG_ERR
        );
        let limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_FSIZE, &limit) }, 0);
        let failure = broker.initialize_new().err().unwrap();
        assert_eq!(failure.operation, AuthorityOperation::WriteKey);
        assert_eq!(failure.error_code, AuthorityFailureCode::StorageUnavailable);
        assert_eq!(failure.os_error, Some(libc::EFBIG));
        return;
    }
    assert_eq!(case, "authority_reload");
    let job =
        uuid::Uuid::parse_str(&std::env::var("APP_OWNER_REFERENCE_TEST_JOB").unwrap()).unwrap();
    let store = broker.load_existing().unwrap();
    let proof = store.load(job).unwrap().unwrap();
    let previous = proof.credential();
    let expected = store
        .credential_for(job, previous.owner, binding())
        .unwrap();
    assert!(proof.authorizes(&expected, &binding()));
}
