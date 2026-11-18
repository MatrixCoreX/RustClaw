use rusqlite::Connection;
use std::{path::PathBuf, time::Duration};

struct Directory(PathBuf);

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct WalSchema {
    db: Connection,
    writer: Connection,
    _directory: Directory,
}

fn initialized() -> WalSchema {
    let directory = Directory(std::env::temp_dir().join(format!(
        "memory-schema-read-{}",
        uuid::Uuid::new_v4().simple()
    )));
    std::fs::create_dir(&directory.0).unwrap();
    let path = directory.0.join("fixture.sqlite");
    let db = Connection::open(&path).unwrap();
    db.busy_timeout(Duration::ZERO).unwrap();
    db.pragma_update(None, "journal_mode", "WAL").unwrap();
    db.execute_batch(
        "CREATE TABLE runtime_schema_migrations (
            migration_id TEXT PRIMARY KEY,
            schema_digest TEXT NOT NULL,
            applied_at TEXT NOT NULL
        );",
    )
    .unwrap();
    super::ensure_embedding_control_schema(&db).unwrap();
    let writer = Connection::open(path).unwrap();
    writer.busy_timeout(Duration::ZERO).unwrap();
    WalSchema {
        db,
        writer,
        _directory: directory,
    }
}

fn hold_writer(fixture: &WalSchema) {
    fixture.writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(!fixture.writer.is_autocommit());
    let error = fixture
        .db
        .execute("DELETE FROM memory_embedding_circuits", [])
        .unwrap_err();
    assert_eq!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy)
    );
}

fn assert_still_read_only(fixture: &WalSchema, changes: u64) {
    assert_eq!(fixture.db.total_changes(), changes);
    assert!(!fixture.writer.is_autocommit());
    assert_eq!(
        fixture
            .db
            .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
}

#[test]
fn initialized_vector_schema_does_not_acquire_writer() {
    let fixture = initialized();
    hold_writer(&fixture);
    let changes = fixture.db.total_changes();
    crate::memory::vector_store::ensure_vector_pipeline_schema(&fixture.db)
        .expect("an already verified vector migration must not reacquire a writer");
    assert_still_read_only(&fixture, changes);
}

#[test]
fn initialized_embedding_controls_do_not_acquire_writer() {
    let fixture = initialized();
    hold_writer(&fixture);
    let changes = fixture.db.total_changes();
    super::ensure_embedding_control_schema(&fixture.db)
        .expect("verified embedding migrations must remain read only");
    assert_still_read_only(&fixture, changes);
}

#[test]
fn circuit_status_read_does_not_acquire_writer() {
    let fixture = initialized();
    fixture
        .db
        .execute(
            "INSERT INTO memory_embedding_circuits
             VALUES ('principal', 'profile', 3, 100, 'fixture', 1)",
            [],
        )
        .unwrap();
    hold_writer(&fixture);
    let changes = fixture.db.total_changes();
    assert!(super::provider_circuit_open(&fixture.db, "principal", "profile", 50).unwrap());
    assert!(!super::provider_circuit_open(&fixture.db, "principal", "profile", 100).unwrap());
    assert!(!super::provider_circuit_open(&fixture.db, "other", "profile", 50).unwrap());
    assert_still_read_only(&fixture, changes);
}

#[test]
fn vector_migration_digest_mismatch_is_rejected_without_writing() {
    let fixture = initialized();
    fixture
        .db
        .execute(
            "UPDATE runtime_schema_migrations SET schema_digest='invalid'
         WHERE migration_id='014_memory_vector_pipeline_v1'",
            [],
        )
        .unwrap();
    hold_writer(&fixture);
    let changes = fixture.db.total_changes();
    let error =
        crate::memory::vector_store::ensure_vector_pipeline_schema(&fixture.db).unwrap_err();
    assert_eq!(
        error.to_string(),
        "runtime_schema_migration_digest_mismatch:014_memory_vector_pipeline_v1"
    );
    assert_still_read_only(&fixture, changes);
}

#[test]
fn control_migration_digest_mismatch_is_rejected_without_writing() {
    let fixture = initialized();
    fixture
        .db
        .execute(
            "UPDATE runtime_schema_migrations SET schema_digest='invalid'
         WHERE migration_id='016_memory_embedding_controls_v1'",
            [],
        )
        .unwrap();
    hold_writer(&fixture);
    let changes = fixture.db.total_changes();
    let error = super::ensure_embedding_control_schema(&fixture.db).unwrap_err();
    assert_eq!(
        error.to_string(),
        "memory_embedding_control_digest_mismatch"
    );
    assert_still_read_only(&fixture, changes);
}

#[test]
fn missing_migration_still_requires_real_write_admission() {
    let fixture = initialized();
    fixture.db.execute(
        "DELETE FROM runtime_schema_migrations WHERE migration_id='017_memory_embedding_circuit_v1'",
        [],
    ).unwrap();
    hold_writer(&fixture);
    let changes = fixture.db.total_changes();
    let error = super::ensure_embedding_control_schema(&fixture.db).unwrap_err();
    assert_eq!(
        error
            .downcast_ref::<rusqlite::Error>()
            .unwrap()
            .sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy)
    );
    assert_still_read_only(&fixture, changes);
}

#[test]
fn missing_migration_is_applied_after_writer_release() {
    let fixture = initialized();
    fixture.db.execute(
        "DELETE FROM runtime_schema_migrations WHERE migration_id='017_memory_embedding_circuit_v1'",
        [],
    ).unwrap();
    hold_writer(&fixture);
    fixture.writer.execute_batch("ROLLBACK").unwrap();
    super::ensure_embedding_control_schema(&fixture.db).unwrap();
    let digest: String = fixture
        .db
        .query_row(
            "SELECT schema_digest FROM runtime_schema_migrations
         WHERE migration_id='017_memory_embedding_circuit_v1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(digest.starts_with("sha256:"));
    assert_eq!(digest.len(), 71);
}
