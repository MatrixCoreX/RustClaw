use super::boundary_test_fixtures as setup;
use super::*;

#[test]
fn complete_memory_list_remains_available_on_a_read_only_connection() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    setup::fact(
        &db,
        &fixture.key,
        &fixture.principal,
        "readonly-witness",
        "readable",
        100,
    );
    db.pragma_update(None, "query_only", "ON").unwrap();
    let result = list_memory_page(&db, &fixture.principal, &MemoryListFilter::default(), 101)
        .expect("a complete initialized memory read must not need a writer");
    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].content, "readable");
}

#[test]
fn list_read_does_not_restore_missing_pressure_observation_to_normal() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    setup::fact(
        &db,
        &fixture.key,
        &fixture.principal,
        "pressure-witness",
        "preserved",
        100,
    );
    db.execute(
        "DELETE FROM memory_storage_pressure WHERE singleton_id=1",
        [],
    )
    .unwrap();
    let result = list_memory_page(&db, &fixture.principal, &MemoryListFilter::default(), 101)
        .expect("missing pressure does not prevent reading existing owned memories");
    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].content, "preserved");
    let observations: i64 = db
        .query_row("SELECT COUNT(*) FROM memory_storage_pressure", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        observations, 0,
        "a read must not replace unknown pressure with an unmeasured normal state"
    );
    assert_eq!(
        crate::memory::capacity::automatic_generation_availability(&db, &fixture.principal)
            .unwrap(),
        crate::memory::capacity::GenerationAvailability::StorageStateMissing
    );
}
