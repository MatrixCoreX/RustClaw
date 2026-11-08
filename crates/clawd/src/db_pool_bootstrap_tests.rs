use super::{configure_pooled_sqlite_connection, sqlite_file_manager};
use crate::runtime_memory::SqliteResourceProfile;
use r2d2::ManageConnection;
use rusqlite::{ffi, Connection};
use std::ffi::{c_char, c_int, c_void, CStr};
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::time::Duration;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "sqlite-pool-bootstrap-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn database(&self) -> PathBuf {
        self.0.join("fixture.db")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn journal_mode(db: &Connection) -> String {
    db.query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap()
}

fn integer_pragma(db: &Connection, name: &str) -> i64 {
    db.pragma_query_value(None, name, |row| row.get(0)).unwrap()
}

#[test]
fn fresh_file_is_wal_before_parallel_pool_connections() {
    let fixture = Fixture::new();
    let _manager = sqlite_file_manager(&fixture.database(), 0).unwrap();
    assert!(
        fixture.database().is_file(),
        "file initialization must finish before pooled connections begin"
    );
    let db = Connection::open(fixture.database()).unwrap();
    assert_eq!(journal_mode(&db), "wal");
}

#[test]
fn rollback_database_is_upgraded_before_pool_connections_without_losing_rows() {
    let fixture = Fixture::new();
    let db = Connection::open(fixture.database()).unwrap();
    db.execute_batch("CREATE TABLE witness(value TEXT); INSERT INTO witness VALUES ('retained');")
        .unwrap();
    assert_eq!(journal_mode(&db), "delete");
    drop(db);
    let _manager = sqlite_file_manager(&fixture.database(), 0).unwrap();
    let db = Connection::open(fixture.database()).unwrap();
    assert_eq!(journal_mode(&db), "wal");
    let value: String = db
        .query_row("SELECT value FROM witness", [], |row| row.get(0))
        .unwrap();
    assert_eq!(value, "retained");
}

// Reject only journal-mode assignments on this connection. Queries and all
// other PRAGMAs remain real SQLite operations; no global callback is installed.
unsafe extern "C" fn deny_journal_assignment(
    _context: *mut c_void,
    action: c_int,
    name: *const c_char,
    value: *const c_char,
    _database: *const c_char,
    _trigger: *const c_char,
) -> c_int {
    if action == ffi::SQLITE_PRAGMA && !name.is_null() && !value.is_null() {
        // SQLite owns this NUL-terminated callback argument for the call.
        let name = unsafe { CStr::from_ptr(name) };
        if name.to_bytes().eq_ignore_ascii_case(b"journal_mode") {
            return ffi::SQLITE_DENY;
        }
    }
    ffi::SQLITE_OK
}

#[test]
fn pooled_connection_initialization_does_not_repeat_journal_assignment() {
    let fixture = Fixture::new();
    let db = Connection::open(fixture.database()).unwrap();
    db.pragma_update(None, "journal_mode", "WAL").unwrap();
    // The callback stores no pointers and is scoped to this owned connection.
    let result = unsafe {
        ffi::sqlite3_set_authorizer(
            db.handle(),
            Some(deny_journal_assignment),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(result, ffi::SQLITE_OK);
    configure_pooled_sqlite_connection(&db, 73, SqliteResourceProfile::Standard)
        .expect("a pooled connection must not repeat the database-wide WAL transition");
    assert_eq!(journal_mode(&db), "wal");
    assert_eq!(integer_pragma(&db, "busy_timeout"), 73);
    assert_eq!(integer_pragma(&db, "foreign_keys"), 1);
}

#[test]
fn verified_wal_bootstrap_preserves_an_active_writer_and_rows() {
    let fixture = Fixture::new();
    let writer = Connection::open(fixture.database()).unwrap();
    writer.busy_timeout(Duration::ZERO).unwrap();
    writer.pragma_update(None, "journal_mode", "WAL").unwrap();
    writer
        .execute_batch(
            "CREATE TABLE witness(value); INSERT INTO witness VALUES(1); BEGIN IMMEDIATE",
        )
        .unwrap();
    let manager = sqlite_file_manager(&fixture.database(), 0).unwrap();
    let reader = manager.connect().unwrap();
    assert!(!writer.is_autocommit());
    let value: i64 = reader
        .query_row("SELECT value FROM witness", [], |row| row.get(0))
        .unwrap();
    assert_eq!(value, 1);
    let error = reader.execute("DELETE FROM witness", []).unwrap_err();
    assert_eq!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy)
    );
    writer.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn pooled_settings_keep_timeout_foreign_keys_and_constrained_memory_policy() {
    let db = Connection::open_in_memory().unwrap();
    configure_pooled_sqlite_connection(&db, 73, SqliteResourceProfile::Constrained).unwrap();
    assert_eq!(integer_pragma(&db, "busy_timeout"), 73);
    assert_eq!(integer_pragma(&db, "synchronous"), 1);
    assert_eq!(integer_pragma(&db, "foreign_keys"), 1);
    assert_eq!(integer_pragma(&db, "cache_size"), -1024);
    assert_eq!(integer_pragma(&db, "temp_store"), 1);
    assert_ne!(integer_pragma(&db, "cache_spill"), 0);
    db.execute_batch(
        "CREATE TABLE parent(id PRIMARY KEY); CREATE TABLE child(id REFERENCES parent(id));",
    )
    .unwrap();
    let error = db.execute("INSERT INTO child VALUES(1)", []).unwrap_err();
    assert_eq!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::ConstraintViolation)
    );
}

#[test]
fn corrupt_file_is_rejected_before_any_pooled_connection_without_rewriting_it() {
    let fixture = Fixture::new();
    let bytes = b"invalid sqlite fixture; preserve this exact evidence";
    std::fs::write(fixture.database(), bytes).unwrap();
    let result = sqlite_file_manager(&fixture.database(), 0);
    assert_eq!(std::fs::read(fixture.database()).unwrap(), bytes);
    assert!(
        result.is_err(),
        "invalid storage must fail before creating a pool"
    );
}

#[test]
fn parallel_connections_keep_wal_and_each_connections_local_settings() {
    let fixture = Fixture::new();
    let keeper = Connection::open(fixture.database()).unwrap();
    keeper.pragma_update(None, "journal_mode", "WAL").unwrap();
    let manager = Arc::new(sqlite_file_manager(&fixture.database(), 73).unwrap());
    let barrier = Arc::new(Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let manager = Arc::clone(&manager);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                let db = manager.connect().unwrap();
                assert_eq!(journal_mode(&db), "wal");
                assert_eq!(integer_pragma(&db, "busy_timeout"), 73);
                assert_eq!(integer_pragma(&db, "foreign_keys"), 1);
                assert_eq!(integer_pragma(&db, "synchronous"), 1);
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
}
