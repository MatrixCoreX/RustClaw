use rusqlite::Connection;

use super::boundary_test_fixtures as setup;

#[test]
fn missing_schema_readiness_fails_without_creating_tables() {
    let db = Connection::open_in_memory().unwrap();
    let error = crate::memory::ux::require_memory_ux_schema(&db).unwrap_err();
    assert_eq!(error.to_string(), "memory_schema_unavailable");
    let tables: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type='table'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0);
}

#[test]
fn incorrect_migration_digest_is_read_only_and_never_repaired() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    db.execute("UPDATE runtime_schema_migrations SET schema_digest='invalid' WHERE migration_id='012_memory_ux_audit_v1'", []).unwrap();
    db.pragma_update(None, "query_only", "ON").unwrap();
    let error = crate::memory::ux::require_memory_ux_schema(&db).unwrap_err();
    assert_eq!(
        error.to_string(),
        "runtime_schema_migration_digest_mismatch:012_memory_ux_audit_v1"
    );
    let digest: String = db.query_row("SELECT schema_digest FROM runtime_schema_migrations WHERE migration_id='012_memory_ux_audit_v1'", [], |row| row.get(0)).unwrap();
    assert_eq!(digest, "invalid");
}

#[test]
fn explicit_startup_pressure_initialization_measures_without_deleting_content() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    setup::fact(
        &db,
        &fixture.key,
        &fixture.principal,
        "kept-fact",
        "Keep this content",
        100,
    );
    db.execute("DELETE FROM memory_storage_pressure", [])
        .unwrap();
    let mut config = fixture.state().policy.memory.clone();
    config.storage_soft_limit_bytes = 1;
    crate::memory::retention::initialize_storage_pressure(&db, &config, 200).unwrap();
    let (state, bytes, revision, at): (String,u64,i64,i64) = db.query_row(
        "SELECT state,observed_bytes,revision,updated_at_ts FROM memory_storage_pressure WHERE singleton_id=1", [],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).unwrap();
    assert_eq!(state, "explicit_write_blocked");
    assert!(bytes > 0);
    assert_eq!((revision, at), (1, 200));
    let content: String = db
        .query_row(
            "SELECT fact_text FROM memory_facts WHERE memory_id='kept-fact'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(content, "Keep this content");
}

#[test]
fn failed_measurement_keeps_pressure_unknown_without_false_normal_seed() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    setup::fact(
        &db,
        &fixture.key,
        &fixture.principal,
        "kept-fact",
        "Keep this content",
        100,
    );
    db.execute_batch(
        "DELETE FROM memory_storage_pressure;
         CREATE TRIGGER reject_observation BEFORE INSERT ON memory_storage_pressure
         WHEN NEW.observed_bytes > 0 BEGIN SELECT RAISE(ABORT,'fixture'); END;",
    )
    .unwrap();
    assert!(crate::memory::retention::initialize_storage_pressure(
        &db,
        &fixture.state().policy.memory,
        200
    )
    .is_err());
    assert!(
        db.is_autocommit(),
        "failed observation transaction must be released"
    );
    let observations: i64 = db
        .query_row("SELECT COUNT(*) FROM memory_storage_pressure", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(observations, 0);
    assert_eq!(
        crate::memory::capacity::automatic_generation_availability(&db, &fixture.principal)
            .unwrap(),
        crate::memory::capacity::GenerationAvailability::StorageStateMissing
    );
    let content: String = db
        .query_row(
            "SELECT fact_text FROM memory_facts WHERE memory_id='kept-fact'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(content, "Keep this content");
}

#[test]
fn full_exports_are_read_only_and_cannot_recreate_missing_pressure() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    setup::fact(
        &db,
        &fixture.key,
        &fixture.principal,
        "export-witness",
        "Preserved export",
        100,
    );
    db.execute("DELETE FROM memory_storage_pressure", [])
        .unwrap();
    let before = db.total_changes();
    db.pragma_update(None, "query_only", "ON").unwrap();
    let json = super::export_memory(&db, &fixture.principal, 200).unwrap();
    let markdown = super::export_memory_markdown(&db, &fixture.principal, 200).unwrap();
    assert_eq!(json.items.len(), 1);
    assert_eq!(json.items[0].content, "Preserved export");
    assert!(markdown.content.contains("Preserved export"));
    let observations: i64 = db
        .query_row("SELECT COUNT(*) FROM memory_storage_pressure", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(observations, 0);
    assert_eq!(db.total_changes(), before);
}

#[tokio::test]
async fn normal_enqueue_does_not_repair_missing_pressure_or_admit_a_job() {
    let fixture = crate::memory::completion_test_fixtures::GenerationFixture::new().await;
    let db = fixture.fixture.state().core.db.get().unwrap();
    db.execute("DELETE FROM memory_storage_pressure", [])
        .unwrap();
    let before: i64 = db
        .query_row("SELECT COUNT(*) FROM memory_jobs", [], |row| row.get(0))
        .unwrap();
    drop(db);
    let queued = crate::memory::jobs::enqueue_turn_memory_jobs(
        fixture.fixture.state(),
        &fixture.task,
        None,
        None,
        true,
    )
    .unwrap();
    assert!(queued.is_empty());
    let db = fixture.fixture.state().core.db.get().unwrap();
    let observations: i64 = db
        .query_row("SELECT COUNT(*) FROM memory_storage_pressure", [], |row| {
            row.get(0)
        })
        .unwrap();
    let after: i64 = db
        .query_row("SELECT COUNT(*) FROM memory_jobs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(observations, 0);
    assert_eq!(after, before);
    assert_eq!(fixture.provider_request_count(), 0);
}

#[test]
fn observation_writer_contention_keeps_unknown_then_recovers_by_actual_measurement() {
    let fixture = crate::memory::capacity_test_fixtures::Fixture::file();
    let db = fixture.state().core.db.get().unwrap();
    db.execute("DELETE FROM memory_storage_pressure", [])
        .unwrap();
    let writer = fixture.competing_writer();
    assert!(crate::memory::retention::initialize_storage_pressure(
        &db,
        &fixture.state().policy.memory,
        200
    )
    .is_err());
    assert!(db.is_autocommit());
    let observations: i64 = db
        .query_row("SELECT COUNT(*) FROM memory_storage_pressure", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(observations, 0);
    writer.execute_batch("ROLLBACK").unwrap();
    crate::memory::retention::initialize_storage_pressure(&db, &fixture.state().policy.memory, 201)
        .unwrap();
    let (bytes, revision, at): (u64, i64, i64) = db
        .query_row(
            "SELECT observed_bytes,revision,updated_at_ts FROM memory_storage_pressure",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert!(bytes > 0);
    assert_eq!((revision, at), (1, 201));
}

#[test]
fn scheduled_retention_restores_only_a_measured_pressure_observation() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    db.execute("DELETE FROM memory_storage_pressure", [])
        .unwrap();
    let report =
        crate::memory::retention::cleanup_memory_data(&db, &fixture.state().policy.memory, 200)
            .unwrap();
    let (bytes, revision, at): (u64, i64, i64) = db
        .query_row(
            "SELECT observed_bytes,revision,updated_at_ts FROM memory_storage_pressure",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert!(bytes > 0);
    assert_eq!(bytes, report.observed_bytes);
    assert_eq!((revision, at), (1, 200));
}
