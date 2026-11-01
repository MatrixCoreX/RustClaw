use std::fs;
#[cfg(target_os = "linux")]
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

use super::*;

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("or-{}", Uuid::new_v4().simple()));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

pub(crate) fn key() -> OwnerAuthorityKey {
    OwnerAuthorityKey::new([7; 32]).unwrap()
}
pub(crate) fn binding() -> [u8; 32] {
    [9; 32]
}
pub(crate) fn credential(job: Uuid) -> OwnerCredential {
    key()
        .credential_for(
            job,
            super::super::instance::observe_process(std::process::id())
                .unwrap()
                .birth,
            binding(),
        )
        .unwrap()
}
pub(crate) fn store(path: &Path) -> OwnerReferenceStore {
    OwnerReferenceStore::from_admitted_directory(File::open(path).unwrap(), key()).unwrap()
}

#[test]
fn owner_store_reopens_same_claim_without_replacing_its_inode_or_persisting_credentials() {
    let root = Directory::new();
    let credential = credential(Uuid::new_v4());
    let first = store(&root.0).publish(&credential, binding()).unwrap();
    let path = root
        .0
        .join(reference_name(credential.job_id).unwrap().to_str().unwrap());
    let before = fs::metadata(&path).unwrap();
    let restored = store(&root.0).load(credential.job_id).unwrap().unwrap();
    assert!(restored.credential() == credential);
    assert!(first.authorizes(&credential, &binding()));
    assert!(store(&root.0)
        .publish(&credential, binding())
        .unwrap()
        .authorizes(&credential, &binding()));
    assert_eq!(before.ino(), fs::metadata(&path).unwrap().ino());
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.contains(&credential.nonce.to_string()));
    assert_eq!(before.mode() & 0o7777, 0o600);
}

#[test]
fn owner_store_one_job_cannot_switch_to_another_birth_or_binding() {
    let root = Directory::new();
    let original = credential(Uuid::new_v4());
    let store = store(&root.0);
    store.publish(&original, binding()).unwrap();
    for changed_binding in [false, true] {
        let mut owner = original.owner.clone();
        let digest = if changed_binding {
            [8; 32]
        } else {
            owner.boot_id = Uuid::new_v4();
            binding()
        };
        let different = key()
            .credential_for(original.job_id, owner, digest)
            .unwrap();
        assert!(matches!(
            store.publish(&different, digest),
            Err(OwnerReferenceFailure::Conflict)
        ));
    }
    assert!(store.load(original.job_id).unwrap().unwrap().credential() == original);
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
}

#[test]
fn owner_store_concurrent_publishers_have_one_durable_job_winner() {
    let root = Directory::new();
    let job = Uuid::new_v4();
    let owner = credential(job).owner;
    let barrier = std::sync::Barrier::new(8);
    std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for index in 1..=8 {
            let (barrier, root, owner) = (&barrier, &root.0, &owner);
            workers.push(scope.spawn(move || {
                let store = store(root);
                let digest = [index; 32];
                let claim = key().credential_for(job, owner.clone(), digest).unwrap();
                barrier.wait();
                match store.publish(&claim, digest) {
                    Ok(proof) => {
                        assert!(proof.authorizes(&claim, &digest));
                        true
                    }
                    Err(error) => {
                        assert_eq!(error, OwnerReferenceFailure::Conflict);
                        false
                    }
                }
            }));
        }
        assert_eq!(
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .filter(|won| *won)
                .count(),
            1
        );
    });
    assert!(store(&root.0).load(job).unwrap().is_some());
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
}

#[test]
fn owner_store_rejects_changed_keys_corrupt_records_and_does_not_overwrite_them() {
    let root = Directory::new();
    let credential = credential(Uuid::new_v4());
    store(&root.0).publish(&credential, binding()).unwrap();
    let wrong = OwnerReferenceStore::from_admitted_directory(
        File::open(&root.0).unwrap(),
        OwnerAuthorityKey::new([8; 32]).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        wrong.load(credential.job_id),
        Err(OwnerReferenceFailure::AuthenticationFailed)
    ));
    let path = root
        .0
        .join(reference_name(credential.job_id).unwrap().to_str().unwrap());
    fs::write(&path, b"partial").unwrap();
    assert!(matches!(
        store(&root.0).load(credential.job_id),
        Err(OwnerReferenceFailure::InvalidReference)
    ));
    assert!(store(&root.0).publish(&credential, binding()).is_err());
    assert_eq!(fs::read(path).unwrap(), b"partial");
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
}

#[test]
fn owner_store_pins_the_admitted_directory_across_path_replacement() {
    let root = Directory::new();
    let current = root.0.join("current");
    fs::DirBuilder::new().mode(0o700).create(&current).unwrap();
    let store = store(&current);
    fs::rename(&current, root.0.join("pinned")).unwrap();
    fs::DirBuilder::new().mode(0o700).create(&current).unwrap();
    let credential = credential(Uuid::new_v4());
    store.publish(&credential, binding()).unwrap();
    assert_eq!(fs::read_dir(current).unwrap().count(), 0);
    assert_eq!(fs::read_dir(root.0.join("pinned")).unwrap().count(), 1);
}

#[test]
fn owner_store_refuses_symlinks_hardlinks_and_special_files_without_blocking() {
    for kind in 0..3 {
        let root = Directory::new();
        let outside = Directory::new();
        let credential = credential(Uuid::new_v4());
        let path = root
            .0
            .join(reference_name(credential.job_id).unwrap().to_str().unwrap());
        let retained = outside.0.join("retained");
        fs::write(&retained, b"outside").unwrap();
        fs::set_permissions(&retained, fs::Permissions::from_mode(0o600)).unwrap();
        match kind {
            0 => std::os::unix::fs::symlink(&retained, &path).unwrap(),
            1 => fs::hard_link(&retained, &path).unwrap(),
            _ => {
                let name = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
                assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
            }
        }
        assert!(store(&root.0).load(credential.job_id).is_err());
        assert!(store(&root.0).publish(&credential, binding()).is_err());
        assert_eq!(fs::read(retained).unwrap(), b"outside");
        assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
    }
}

#[test]
fn owner_store_rechecks_private_directory_and_record_modes() {
    let root = Directory::new();
    let credential = credential(Uuid::new_v4());
    let store = store(&root.0);
    store.publish(&credential, binding()).unwrap();
    fs::set_permissions(&root.0, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        store.load(credential.job_id),
        Err(OwnerReferenceFailure::InvalidDirectory)
    ));
    fs::set_permissions(&root.0, fs::Permissions::from_mode(0o700)).unwrap();
    let path = root
        .0
        .join(reference_name(credential.job_id).unwrap().to_str().unwrap());
    fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        store.load(credential.job_id),
        Err(OwnerReferenceFailure::InvalidReference)
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn owner_store_directory_sync_failure_never_grants_a_start_token_or_deletes_the_claim() {
    let root = Directory::new();
    let credential = credential(Uuid::new_v4());
    let descriptor = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(&root.0)
        .unwrap();
    let store = OwnerReferenceStore::from_admitted_directory(descriptor, key()).unwrap();
    assert!(matches!(
        store.publish(&credential, binding()),
        Err(OwnerReferenceFailure::StorageUnavailable)
    ));
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
    assert!(matches!(
        store.load(credential.job_id),
        Err(OwnerReferenceFailure::StorageUnavailable)
    ));
    assert!(super::tests::store(&root.0)
        .load(credential.job_id)
        .unwrap()
        .unwrap()
        .authorizes(&credential, &binding()));
}

#[test]
fn owner_store_missing_reference_is_not_a_start_permission() {
    let root = Directory::new();
    assert!(store(&root.0).load(Uuid::new_v4()).unwrap().is_none());
    assert!(store(&root.0).load(Uuid::nil()).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn owner_store_two_actual_helpers_never_start_the_same_job_twice() {
    use std::process::{Child, Stdio};
    struct Children(Vec<Child>);
    impl Drop for Children {
        fn drop(&mut self) {
            for child in &mut self.0 {
                if child.try_wait().unwrap().is_none() {
                    let _ = child.kill();
                }
                let _ = child.wait();
            }
        }
    }
    let root = Directory::new();
    let job = Uuid::new_v4();
    let mut children = Children(Vec::new());
    for _ in 0..2 {
        let child = super::super::owner_scope::tests::fixture_command("owner_ref_spawn", None)
            .env("APP_OWNER_REFERENCE_TEST_ROOT", &root.0)
            .env("APP_OWNER_REFERENCE_TEST_JOB", job.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        children.0.push(child);
    }
    let ids = children
        .0
        .iter()
        .map(|child| child.id())
        .collect::<Vec<_>>();
    for id in &ids {
        wait_for(&root.0.join(format!("ready-{id}")));
    }
    fs::write(root.0.join("go"), b"ready").unwrap();
    while let Some(child) = children.0.pop() {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        println!("{}", String::from_utf8_lossy(&output.stdout));
    }
    assert_eq!(fs::read(root.0.join("effect")).unwrap(), b"x");
    let restored = store(&root.0).load(job).unwrap().unwrap();
    assert!(ids.contains(&restored.credential().owner.pid));
}

#[cfg(target_os = "linux")]
#[test]
fn owner_store_published_handoff_keeps_work_running_after_the_actual_frontend_exits() {
    let root = Directory::new();
    let job = Uuid::new_v4();
    let output = super::super::owner_scope::tests::fixture_command("owner_ref_frontend", None)
        .env("APP_OWNER_REFERENCE_TEST_ROOT", &root.0)
        .env("APP_OWNER_REFERENCE_TEST_JOB", job.to_string())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
    assert!(root.0.join("started").exists());
    assert!(!root.0.join("effect").exists());
    let reopened = store(&root.0).load(job).unwrap().unwrap();
    assert_eq!(reopened.credential().job_id, job);
    fs::write(root.0.join("release"), b"continue").unwrap();
    wait_for(&root.0.join("owner-complete"));
    assert_eq!(fs::read(root.0.join("effect")).unwrap(), b"x");
}

#[cfg(target_os = "linux")]
fn wait_for(path: &Path) {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(8);
    while !path.exists() {
        assert!(
            std::time::Instant::now() < end,
            "fixture barrier did not arrive"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn run_special_case(case: &str) {
    use super::super::owner_protocol::OwnerPhase;
    use super::super::owner_start::OwnerStart;
    use super::super::process_launch::{PayloadIo, ProcessLaunch};
    use crate::process_invocation::ProcessInvocation;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let root = PathBuf::from(std::env::var_os("APP_OWNER_REFERENCE_TEST_ROOT").unwrap());
    let job = Uuid::parse_str(&std::env::var("APP_OWNER_REFERENCE_TEST_JOB").unwrap()).unwrap();
    if case == "owner_ref_frontend" {
        let child = super::super::owner_scope::tests::fixture_command("owner_ref_detached", None)
            .env("APP_OWNER_REFERENCE_TEST_ROOT", &root)
            .env("APP_OWNER_REFERENCE_TEST_JOB", job.to_string())
            .stdin(Stdio::null())
            .stdout(File::create(root.join("owner.stdout")).unwrap())
            .stderr(File::create(root.join("owner.stderr")).unwrap())
            .spawn()
            .unwrap();
        let expected = key()
            .credential_for(
                job,
                super::super::instance::observe_process(child.id())
                    .unwrap()
                    .birth,
                binding(),
            )
            .unwrap();
        store(&root).publish(&expected, binding()).unwrap();
        fs::write(root.join("go"), b"ready").unwrap();
        wait_for(&root.join("started"));
        // Drop only the frontend's observation handle. Its independent owner
        // continues until the outer test releases the controlled payload.
        drop(child);
        return;
    }
    let expected = credential(job);
    let mut invocation = ProcessInvocation::new("/bin/sh");
    let script = if case == "owner_ref_detached" {
        "i=0; while [ ! -e release ]; do i=$((i+1)); [ \"$i\" -le 500 ] || exit 3; sleep 0.01; done; printf x >> effect"
    } else {
        "printf x >> effect"
    };
    invocation.arg("-c").arg(script);
    let launch = ProcessLaunch::new(
        invocation,
        root.clone(),
        vec![],
        PayloadIo {
            stdin: None,
            stdout: None,
            stderr: None,
        },
    )
    .unwrap();
    let mut start = OwnerStart::prepare(
        launch,
        expected.job_id,
        expected.nonce,
        binding(),
        Duration::from_secs(1),
    )
    .unwrap();
    fs::write(root.join(format!("ready-{}", std::process::id())), b"ready").unwrap();
    wait_for(&root.join("go"));
    let reference = if case == "owner_ref_detached" {
        store(&root).load(job).unwrap().unwrap()
    } else {
        match store(&root).publish(&expected, binding()) {
            Ok(reference) => reference,
            Err(error) => {
                assert_eq!(error, OwnerReferenceFailure::Conflict);
                let previous = store(&root).load(job).unwrap().unwrap();
                assert!(matches!(
                    unsafe { start.commit(&previous) },
                    Err(super::super::owner_protocol::OwnerFailure::CredentialMismatch)
                ));
                return;
            }
        }
    };
    assert!(unsafe { start.commit(&reference) }.unwrap() == expected);
    assert!(unsafe { start.commit(&reference) }.unwrap() == expected);
    if case == "owner_ref_detached" {
        fs::write(root.join("started"), b"started").unwrap();
    }
    let end = Instant::now() + Duration::from_secs(8);
    loop {
        let snapshot = start.runtime().unwrap().tick().unwrap();
        if snapshot.phase == OwnerPhase::Terminal {
            assert_eq!(snapshot.root_exit.unwrap().code, Some(0));
            break;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(2));
    }
    if case == "owner_ref_detached" {
        fs::write(root.join("owner-complete"), b"complete").unwrap();
    }
}
