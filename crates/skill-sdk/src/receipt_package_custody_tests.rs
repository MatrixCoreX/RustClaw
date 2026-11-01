use super::*;
use crate::package_custody::{PackageCustodyFailure, PackageVersionCustody};

struct Fixture {
    root: tempfile::TempDir,
    store: InstallReceiptStore,
    version: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = InstallReceiptStore::new(root.path().join("packages"));
        let version = store
            .version_dir("lease_fixture", "1.0.0", &"a".repeat(64))
            .unwrap();
        fs::create_dir_all(&version).unwrap();
        fs::write(version.join("artifact"), b"owned package").unwrap();
        Self {
            root,
            store,
            version,
        }
    }
    fn lease(&self) -> SkillVersionLease {
        self.store
            .acquire_version_lease("lease_fixture", &self.version)
            .unwrap()
    }
    fn collect(&self) -> bool {
        self.store
            .remove_installed_versions("lease_fixture")
            .unwrap()
    }
    fn records(&self) -> PathBuf {
        self.store
            .skill_root("lease_fixture")
            .unwrap()
            .join(DIRECTORY)
            .join(self.version.file_name().unwrap())
    }
}
fn binding() -> PackageCustodyBinding {
    PackageCustodyBinding {
        job_id: Uuid::new_v4(),
        execution_binding_digest: [11; 32],
        owner_reference_digest: [12; 32],
    }
}

#[test]
fn durable_custody_survives_all_open_handles_and_a_new_store_instance() {
    let fixture = Fixture::new();
    let lease = fixture.lease();
    let custody = lease.retain_owner_custody(binding()).unwrap();
    drop(lease);
    drop(custody);
    assert!(!fixture.collect());
    let reopened = InstallReceiptStore::new(fixture.root.path().join("packages"));
    assert!(!reopened.remove_installed_versions("lease_fixture").unwrap());
    assert_eq!(
        fs::read(fixture.version.join("artifact")).unwrap(),
        b"owned package"
    );
}

#[test]
fn explicit_release_is_idempotent_and_collection_waits_for_the_shared_handle() {
    let fixture = Fixture::new();
    let lease = fixture.lease();
    let custody = lease.retain_owner_custody(binding()).unwrap();
    custody.release().unwrap();
    custody.release().unwrap();
    assert!(!fixture.collect());
    drop(lease);
    drop(custody);
    assert!(fixture.collect());
    assert!(!fixture.version.exists());
}

#[test]
fn one_completed_owner_does_not_release_another_owner_of_the_same_version() {
    let fixture = Fixture::new();
    let lease = fixture.lease();
    let first = lease.retain_owner_custody(binding()).unwrap();
    let second_binding = binding();
    let second = lease.retain_owner_custody(second_binding).unwrap();
    first.release().unwrap();
    drop(first);
    drop(second);
    drop(lease);
    assert!(!fixture.collect());
    let lease = fixture.lease();
    let second = lease.retain_owner_custody(second_binding).unwrap();
    second.release().unwrap();
    drop(second);
    drop(lease);
    assert!(
        !fixture.version.exists(),
        "the last SDK guard should finish requested collection"
    );
    assert!(
        !fixture.collect(),
        "collection of an absent skill remains idempotent"
    );
}

#[test]
fn released_custody_cannot_be_rearmed_by_reusing_its_binding() {
    let fixture = Fixture::new();
    let lease = fixture.lease();
    let binding = binding();
    let custody = lease.retain_owner_custody(binding).unwrap();
    custody.release().unwrap();
    assert!(matches!(
        custody.into_process_handles(),
        Err(PackageCustodyFailure::Released)
    ));
    assert_eq!(
        lease.retain_owner_custody(binding).err().unwrap().code,
        "skill_owner_lease_invalid"
    );
}

#[test]
fn empty_or_corrupt_records_never_become_elapsed_retention() {
    for malformed in [
        b"".as_slice(),
        b"{}",
        b"{\"schema_version\":99}",
        &[b'x'; 4097],
    ] {
        let fixture = Fixture::new();
        let lease = fixture.lease();
        let binding = binding();
        let custody = lease.retain_owner_custody(binding).unwrap();
        drop(custody);
        drop(lease);
        fs::write(
            fixture.records().join(binding.filename().unwrap()),
            malformed,
        )
        .unwrap();
        assert!(!fixture.collect());
        assert!(fixture.version.is_dir());
    }
}

#[test]
fn corrupted_scope_or_filename_cannot_release_a_version() {
    for field in [
        "schema_version",
        "skill_name",
        "install_dir",
        "lease_inode",
        "unknown",
        "filename",
    ] {
        let fixture = Fixture::new();
        let lease = fixture.lease();
        let binding = binding();
        let custody = lease.retain_owner_custody(binding).unwrap();
        custody.release().unwrap();
        drop(custody);
        drop(lease);
        let path = fixture.records().join(binding.filename().unwrap());
        if field == "filename" {
            fs::rename(&path, fixture.records().join("foreign.json")).unwrap();
        } else {
            let mut value: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            value[field] = match field {
                "schema_version" | "lease_inode" => serde_json::json!(999),
                _ => serde_json::json!("foreign"),
            };
            fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        }
        assert!(!fixture.collect(), "{field}");
    }
}

#[test]
fn symlinked_custody_parent_is_refused_before_creating_foreign_files() {
    let fixture = Fixture::new();
    let lease = fixture.lease();
    let foreign = fixture.root.path().join("foreign");
    fs::create_dir(&foreign).unwrap();
    let owners = fixture
        .store
        .skill_root("lease_fixture")
        .unwrap()
        .join(DIRECTORY);
    std::os::unix::fs::symlink(&foreign, owners).unwrap();
    assert!(lease.retain_owner_custody(binding()).is_err());
    assert_eq!(fs::read_dir(foreign).unwrap().count(), 0);
    drop(lease);
    assert!(!fixture.collect());
}

#[test]
fn private_record_descriptors_reject_readonly_append_and_foreign_scope() {
    use std::os::unix::fs::OpenOptionsExt;
    for mode in ["readonly", "append", "scope"] {
        let fixture = Fixture::new();
        let lease = fixture.lease();
        let mut expected = binding();
        let custody = lease.retain_owner_custody(expected).unwrap();
        let [version, record] = custody.into_process_handles().unwrap();
        let path = fixture.records().join(expected.filename().unwrap());
        let record = match mode {
            "readonly" => File::open(&path).unwrap(),
            "append" => OpenOptions::new()
                .read(true)
                .write(true)
                .append(true)
                .custom_flags(libc::O_CLOEXEC)
                .open(&path)
                .unwrap(),
            _ => {
                expected.execution_binding_digest = [29; 32];
                record
            }
        };
        assert!(matches!(
            PackageVersionCustody::from_process_handles([version, record], expected),
            Err(PackageCustodyFailure::InvalidDescriptor | PackageCustodyFailure::BindingMismatch)
        ));
    }
}

#[test]
fn release_preserves_shared_file_offsets_and_status_flags() {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::FileExt;
    let fixture = Fixture::new();
    let lease = fixture.lease();
    let expected = binding();
    let custody = lease.retain_owner_custody(expected).unwrap();
    let [version, record] = custody.into_process_handles().unwrap();
    let observer = record.try_clone().unwrap();
    assert_eq!(
        unsafe { libc::lseek(observer.as_raw_fd(), 3, libc::SEEK_SET) },
        3
    );
    let flags = unsafe { libc::fcntl(observer.as_raw_fd(), libc::F_GETFL) };
    let custody = PackageVersionCustody::from_process_handles([version, record], expected).unwrap();
    custody.release().unwrap();
    assert_eq!(
        unsafe { libc::lseek(observer.as_raw_fd(), 0, libc::SEEK_CUR) },
        3
    );
    assert_eq!(
        unsafe { libc::fcntl(observer.as_raw_fd(), libc::F_GETFL) },
        flags
    );
    let mut head = [0; 1];
    observer.read_exact_at(&mut head, 0).unwrap();
    assert_eq!(head, *b"{");
}

#[test]
fn pending_record_publication_cannot_be_admitted_by_another_open() {
    let fixture = Fixture::new();
    let lease = fixture.lease();
    let expected = binding();
    let custody = lease.retain_owner_custody(expected).unwrap();
    let [version, record] = custody.into_process_handles().unwrap();
    // An owned publisher barrier, not an assertion of an observed power-loss incident.
    fs2::FileExt::lock_exclusive(&record).unwrap();
    let premature = lease.retain_owner_custody(expected);
    let refused = premature.err().map(|error| error.code);
    fs2::FileExt::unlock(&record).unwrap();
    let recovered = lease.retain_owner_custody(expected).unwrap();
    recovered.release().unwrap();
    drop(recovered);
    drop(version);
    drop(record);
    drop(lease);
    assert_eq!(refused.as_deref(), Some("skill_owner_lease_initializing"));
}

#[test]
fn missing_version_directory_does_not_erase_unknown_owner_custody() {
    let fixture = Fixture::new();
    let lease = fixture.lease();
    let expected = binding();
    let custody = lease.retain_owner_custody(expected).unwrap();
    drop(custody);
    drop(lease);
    let record = fixture.records().join(expected.filename().unwrap());
    let before = fs::read(&record).unwrap();
    fs::remove_dir_all(&fixture.version).unwrap();
    let removed = fixture.collect();
    let retained = fs::read(record).ok();
    assert!(
        !removed && retained.as_deref() == Some(before.as_slice()),
        "a missing package directory is not completion evidence for its owner"
    );
}
