use super::*;
use crate::memory::ux;

pub(super) fn correction_rollback() {
    let fixture = fixtures::Fixture::new();
    let db = fixture.state.core.db.get().unwrap();
    db.execute_batch(
        "CREATE TABLE parent_effect(value TEXT);
        CREATE TRIGGER correction_fault BEFORE INSERT ON memory_revisions
        WHEN NEW.operation='correct' BEGIN SELECT RAISE(ABORT,'nested_correction_fault'); END;
        BEGIN IMMEDIATE; INSERT INTO parent_effect VALUES('before');",
    )
    .unwrap();
    let before = fixtures::private_state(&db);
    assert!(ux::correct_memory(
        &db,
        &fixture.actor.principal_id,
        &fixture.actor.principal_id,
        "actor-witness",
        &ux::MemoryCorrectionRequest {
            expected_revision: 1,
            content: "nested revised content".into()
        },
        crate::now_ts_u64() as i64
    )
    .is_err());
    assert!(!db.is_autocommit());
    assert_eq!(fixtures::private_state(&db), before);
    db.execute_batch("INSERT INTO parent_effect VALUES('after'); COMMIT;")
        .unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM parent_effect", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

pub(super) fn import_rollback() {
    let fixture = fixtures::Fixture::new();
    let (_, body) = fixture.request(fixtures::Action::Confirm);
    let request: ux::MemoryImportConfirmRequest = serde_json::from_value(body).unwrap();
    let db = fixture.state.core.db.get().unwrap();
    db.execute_batch(
        "CREATE TABLE parent_effect(value TEXT);
        CREATE TRIGGER import_fault BEFORE UPDATE OF status ON memory_import_sessions
        WHEN NEW.status='confirmed' BEGIN SELECT RAISE(ABORT,'nested_import_fault'); END;
        BEGIN IMMEDIATE; INSERT INTO parent_effect VALUES('before');",
    )
    .unwrap();
    let before = fixtures::private_state(&db);
    assert!(ux::confirm_memory_import(
        &db,
        &fixture.actor.principal_id,
        &request,
        crate::now_ts_u64() as i64
    )
    .is_err());
    assert!(!db.is_autocommit());
    assert_eq!(fixtures::private_state(&db), before);
    db.execute_batch("INSERT INTO parent_effect VALUES('after'); COMMIT;")
        .unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM parent_effect", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}
