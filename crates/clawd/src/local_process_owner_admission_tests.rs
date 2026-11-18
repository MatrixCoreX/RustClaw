use std::fs::{self, File};
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::super::owner_reference::AuthorityFingerprint;
use super::super::owner_store::tests::binding;
use super::*;

#[path = "local_process_owner_admission_recovery_tests.rs"]
mod recovery;

#[path = "local_process_owner_host_pin_tests.rs"]
mod host_pin;

fn absent_pin() -> AuthorityFingerprint {
    AuthorityFingerprint::from_slice(&[1; 32]).unwrap()
}

struct Fixture {
    root: PathBuf,
    database: PathBuf,
    keys: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("aj-{}", uuid::Uuid::new_v4().simple()));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let keys = root.join("keys");
        fs::DirBuilder::new().mode(0o700).create(&keys).unwrap();
        let database = root.join("journal.sqlite");
        let db = Self::open(&database);
        db.pragma_update(None, "journal_mode", "WAL").unwrap();
        db.pragma_update(None, "synchronous", "NORMAL").unwrap();
        drop(db);
        Self {
            root,
            database,
            keys,
        }
    }

    fn open(path: &Path) -> Connection {
        let db = Connection::open(path).unwrap();
        db.busy_timeout(Duration::from_secs(8)).unwrap();
        db
    }

    fn journal(&self) -> OwnerAuthorityJournal {
        OwnerAuthorityJournal::from_connection(Self::open(&self.database)).unwrap()
    }

    fn broker(&self) -> OwnerAuthorityBroker {
        OwnerAuthorityBroker::from_admitted_directory(File::open(&self.keys).unwrap()).unwrap()
    }

    fn key_file(&self) -> PathBuf {
        super::super::owner_authority::tests::key_file(&self.keys)
    }

    fn sequence(&self) -> i64 {
        Self::open(&self.database).query_row(
            "SELECT admission_sequence FROM local_process_authority_bootstrap WHERE singleton = 1",
            [], |row| row.get(0),
        ).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn owner_admission_refuses_memory_or_rollback_journals_without_creating_key_state() {
    let fixture = Fixture::new();
    for db in [
        Connection::open_in_memory().unwrap(),
        Fixture::open(&fixture.root.join("rollback.sqlite")),
    ] {
        assert_eq!(
            OwnerAuthorityJournal::from_connection(db).err().unwrap(),
            AdmissionFailure::DurabilityUnavailable
        );
    }
    assert_eq!(fs::read_dir(&fixture.keys).unwrap().count(), 0);
}

#[test]
fn owner_admission_dedicated_full_connection_preserves_ordinary_pool_policy() {
    let fixture = Fixture::new();
    let ordinary = Fixture::open(&fixture.database);
    ordinary
        .pragma_update(None, "synchronous", "NORMAL")
        .unwrap();
    let mut journal = fixture.journal();
    assert_eq!(
        journal
            .db
            .pragma_query_value(None, "synchronous", |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
    journal.initialize(&fixture.broker()).unwrap();
    assert_eq!(
        ordinary
            .pragma_query_value(None, "synchronous", |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(fixture.sequence(), 1);
    println!("owner_admission_sqlite_version={}", rusqlite::version());
}

#[test]
fn owner_admission_reopens_the_same_authority_and_authenticates_its_existing_job() {
    let fixture = Fixture::new();
    let mut journal = fixture.journal();
    let store = journal.initialize(&fixture.broker()).unwrap();
    let expected = store
        .credential_for(
            uuid::Uuid::new_v4(),
            super::super::instance::observe_process(std::process::id())
                .unwrap()
                .birth,
            binding(),
        )
        .unwrap();
    store.publish(&expected, binding()).unwrap();
    let fingerprint = store.authority_fingerprint().unwrap();
    let inode = fs::metadata(fixture.key_file()).unwrap().ino();
    drop(store);
    drop(journal);
    let mut journal = fixture.journal();
    let store = journal.resume(&fixture.broker(), &fingerprint).unwrap();
    assert_eq!(store.authority_fingerprint().unwrap(), fingerprint);
    assert!(store
        .load(expected.job_id)
        .unwrap()
        .unwrap()
        .authorizes(&expected, &binding()));
    assert_eq!(fixture.sequence(), 2);
    assert_eq!(
        journal.initialize(&fixture.broker()).err().unwrap(),
        AdmissionFailure::AlreadyInitialized
    );
    assert_eq!(fs::metadata(fixture.key_file()).unwrap().ino(), inode);
}

#[test]
fn owner_admission_missing_journal_is_not_first_install_permission() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture
            .journal()
            .resume(&fixture.broker(), &absent_pin())
            .err()
            .unwrap(),
        AdmissionFailure::MissingJournal
    );
    assert_eq!(fs::read_dir(&fixture.keys).unwrap().count(), 0);
    let tables: i64 = Fixture::open(&fixture.database)
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name = 'local_process_authority_bootstrap'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0);
}

#[test]
fn owner_admission_reserved_but_unfinished_bootstrap_is_never_retried() {
    let fixture = Fixture::new();
    let mut journal = fixture.journal();
    drop(journal::reserve(&mut journal.db).unwrap());
    drop(journal);
    let mut journal = fixture.journal();
    assert_eq!(
        journal.initialize(&fixture.broker()).err().unwrap(),
        AdmissionFailure::AlreadyInitialized
    );
    assert_eq!(
        journal
            .resume(&fixture.broker(), &absent_pin())
            .err()
            .unwrap(),
        AdmissionFailure::IncompleteInitialization
    );
    assert_eq!(fixture.sequence(), 0);
    assert_eq!(fs::read_dir(&fixture.keys).unwrap().count(), 0);
}

#[test]
fn owner_admission_does_not_adopt_a_key_without_a_matching_journal() {
    let fixture = Fixture::new();
    fixture.broker().initialize_new().unwrap();
    let before = fs::metadata(fixture.key_file()).unwrap().ino();
    let mut journal = fixture.journal();
    assert!(matches!(
        journal.initialize(&fixture.broker()),
        Err(AdmissionFailure::Authority(_))
    ));
    assert_eq!(
        journal
            .resume(&fixture.broker(), &absent_pin())
            .err()
            .unwrap(),
        AdmissionFailure::IncompleteInitialization
    );
    assert_eq!(fixture.sequence(), 0);
    assert_eq!(fs::metadata(fixture.key_file()).unwrap().ino(), before);
}

#[test]
fn owner_admission_missing_key_or_epoch_cannot_trigger_reinitialization() {
    for entire_epoch in [false, true] {
        let fixture = Fixture::new();
        let mut journal = fixture.journal();
        let store = journal.initialize(&fixture.broker()).unwrap();
        let fingerprint = store.authority_fingerprint().unwrap();
        if entire_epoch {
            fs::remove_dir_all(fixture.key_file().parent().unwrap()).unwrap();
        } else {
            fs::remove_file(fixture.key_file()).unwrap();
        }
        assert!(matches!(
            journal.resume(&fixture.broker(), &fingerprint),
            Err(AdmissionFailure::Authority(_))
        ));
        assert_eq!(
            journal.initialize(&fixture.broker()).err().unwrap(),
            AdmissionFailure::AlreadyInitialized
        );
        assert!(!fixture.key_file().exists());
        let bytes: Vec<u8> = journal
            .db
            .query_row(
                "SELECT fingerprint FROM local_process_authority_bootstrap",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            AuthorityFingerprint::from_slice(&bytes).unwrap(),
            fingerprint
        );
    }
}

#[test]
fn owner_admission_valid_format_key_replacement_is_refused_using_the_journal_identity() {
    let fixture = Fixture::new();
    let mut journal = fixture.journal();
    let store = journal.initialize(&fixture.broker()).unwrap();
    let fingerprint = store.authority_fingerprint().unwrap();
    let mut key = zeroize::Zeroizing::new(fs::read(fixture.key_file()).unwrap());
    let last = key.len() - 1;
    key[last] ^= 1;
    fs::write(fixture.key_file(), &key).unwrap();
    assert!(matches!(
        journal.resume(&fixture.broker(), &fingerprint),
        Err(AdmissionFailure::Authority(AuthorityFailure {
            error_code: super::super::owner_authority::AuthorityFailureCode::AuthorityMismatch,
            ..
        }))
    ));
    assert!(fs::read(fixture.key_file()).unwrap().as_slice() == key.as_slice());
    let bytes: Vec<u8> = journal
        .db
        .query_row(
            "SELECT fingerprint FROM local_process_authority_bootstrap",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        AuthorityFingerprint::from_slice(&bytes).unwrap(),
        fingerprint
    );
}

#[test]
fn owner_admission_rechecks_durability_and_rejects_an_active_transaction() {
    let fixture = Fixture::new();
    let mut journal = fixture.journal();
    journal
        .db
        .pragma_update(None, "synchronous", "NORMAL")
        .unwrap();
    assert_eq!(
        journal.initialize(&fixture.broker()).err().unwrap(),
        AdmissionFailure::DurabilityUnavailable
    );
    assert_eq!(fs::read_dir(&fixture.keys).unwrap().count(), 0);
    let db = Fixture::open(&fixture.database);
    db.execute_batch("BEGIN").unwrap();
    assert_eq!(
        OwnerAuthorityJournal::from_connection(db).err().unwrap(),
        AdmissionFailure::InvalidJournal
    );
}

#[test]
fn owner_admission_corrupted_bounded_row_fields_never_grant_or_repair_authority() {
    for sql in [
        "UPDATE local_process_authority_bootstrap SET schema_version = 2",
        "UPDATE local_process_authority_bootstrap SET reservation_id = zeroblob(16)",
        "UPDATE local_process_authority_bootstrap SET reservation_id = zeroblob(65536)",
        "UPDATE local_process_authority_bootstrap SET phase = 'unknown'",
        "UPDATE local_process_authority_bootstrap SET fingerprint = zeroblob(32)",
        "UPDATE local_process_authority_bootstrap SET fingerprint = zeroblob(65536)",
        "UPDATE local_process_authority_bootstrap SET admission_sequence = 0",
    ] {
        let fixture = Fixture::new();
        let mut journal = fixture.journal();
        let fingerprint = journal
            .initialize(&fixture.broker())
            .unwrap()
            .authority_fingerprint()
            .unwrap();
        let inode = fs::metadata(fixture.key_file()).unwrap().ino();
        journal
            .db
            .pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        journal.db.execute_batch(sql).unwrap();
        assert!(journal.resume(&fixture.broker(), &fingerprint).is_err());
        assert_eq!(fs::metadata(fixture.key_file()).unwrap().ino(), inode);
    }
}

#[test]
fn owner_admission_counter_exhaustion_is_explicit_and_does_not_wrap() {
    let fixture = Fixture::new();
    let mut journal = fixture.journal();
    let fingerprint = journal
        .initialize(&fixture.broker())
        .unwrap()
        .authority_fingerprint()
        .unwrap();
    journal
        .db
        .execute(
            "UPDATE local_process_authority_bootstrap SET admission_sequence = ?1",
            [i64::MAX],
        )
        .unwrap();
    assert_eq!(
        journal
            .resume(&fixture.broker(), &fingerprint)
            .err()
            .unwrap(),
        AdmissionFailure::SequenceExhausted
    );
    assert_eq!(fixture.sequence(), i64::MAX);
}

#[cfg(target_os = "linux")]
pub(crate) fn run_special_case(case: &str) {
    let root = PathBuf::from(std::env::var_os("APP_OWNER_REFERENCE_TEST_ROOT").unwrap());
    let mut journal =
        OwnerAuthorityJournal::from_connection(Fixture::open(&root.join("journal.sqlite")))
            .unwrap();
    match case {
        "admission_reserve_exit" => drop(journal::reserve(&mut journal.db).unwrap()),
        "admission_ready_exit" => {
            let broker = OwnerAuthorityBroker::from_admitted_directory(
                File::open(root.join("keys")).unwrap(),
            )
            .unwrap();
            journal.initialize(&broker).unwrap();
        }
        _ => panic!("unknown admission fixture"),
    }
}
