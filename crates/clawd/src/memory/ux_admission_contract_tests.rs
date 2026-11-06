use super::boundary_test_fixtures as setup;
use super::*;
use crate::memory::capacity_test_fixtures::pressure;

#[path = "ux_admission_test_fixtures.rs"]
mod support;
use support::{correction, fixture, import_request, private_state};

#[test]
fn blocked_correction_rejects_without_superseding_or_appending_private_content() {
    let fixture = fixture();
    let db = fixture.state().core.db.get().unwrap();
    pressure(&db, "explicit_write_blocked");
    let before = private_state(&db, &fixture.principal);
    let error = correct_memory(
        &db,
        &fixture.principal,
        &fixture.principal,
        "witness",
        &correction(),
        102,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "memory_storage_write_blocked");
    assert_eq!(private_state(&db, &fixture.principal), before);
    assert!(db.is_autocommit());
}

#[test]
fn missing_pressure_correction_fails_closed_without_repairing_the_observation() {
    let fixture = fixture();
    let db = fixture.state().core.db.get().unwrap();
    db.execute("DELETE FROM memory_storage_pressure", [])
        .unwrap();
    let before = private_state(&db, &fixture.principal);
    let error = correct_memory(
        &db,
        &fixture.principal,
        &fixture.principal,
        "witness",
        &correction(),
        102,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "memory_storage_state_missing");
    assert_eq!(private_state(&db, &fixture.principal), before);
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM memory_storage_pressure", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
}

#[test]
fn blocked_import_preview_does_not_persist_an_unadmitted_private_payload() {
    let fixture = fixture();
    let db = fixture.state().core.db.get().unwrap();
    let request = import_request(&db, &fixture.principal);
    pressure(&db, "explicit_write_blocked");
    let before = private_state(&db, &fixture.principal);
    let error = preview_memory_import(&db, &fixture.principal, &request, 102).unwrap_err();
    assert_eq!(error.to_string(), "memory_storage_write_blocked");
    assert_eq!(private_state(&db, &fixture.principal), before);
}

#[test]
fn blocked_import_confirmation_preserves_preview_for_an_explicit_later_retry() {
    let fixture = fixture();
    let db = fixture.state().core.db.get().unwrap();
    let request = import_request(&db, &fixture.principal);
    let preview = preview_memory_import(&db, &fixture.principal, &request, 101).unwrap();
    let confirmation = setup::confirmation(&preview);
    pressure(&db, "explicit_write_blocked");
    let before = private_state(&db, &fixture.principal);
    let error = confirm_memory_import(&db, &fixture.principal, &confirmation, 102).unwrap_err();
    assert_eq!(error.to_string(), "memory_storage_write_blocked");
    assert_eq!(private_state(&db, &fixture.principal), before);
    assert!(db.is_autocommit());
    pressure(&db, "normal");
    assert_eq!(
        confirm_memory_import(&db, &fixture.principal, &confirmation, 103)
            .unwrap()
            .imported_items,
        1
    );
}

#[test]
fn blocked_undo_cannot_restore_content_or_consume_the_existing_undo_receipt() {
    let fixture = fixture();
    let db = fixture.state().core.db.get().unwrap();
    let request = setup::delete(&db, &fixture.principal, "witness");
    pressure(&db, "explicit_write_blocked");
    let before = private_state(&db, &fixture.principal);
    let error = undo_memory_mutation(&db, &fixture.principal, &request, 102).unwrap_err();
    assert_eq!(error.to_string(), "memory_storage_write_blocked");
    assert_eq!(private_state(&db, &fixture.principal), before);
    pressure(&db, "normal");
    assert_eq!(
        undo_memory_mutation(&db, &fixture.principal, &request, 103)
            .unwrap()
            .status,
        "restored"
    );
}

#[test]
fn automatic_pause_does_not_block_an_explicit_revision_checked_correction() {
    let fixture = fixture();
    let db = fixture.state().core.db.get().unwrap();
    pressure(&db, "automatic_generation_paused");
    assert_eq!(
        correct_memory(
            &db,
            &fixture.principal,
            &fixture.principal,
            "witness",
            &correction(),
            102
        )
        .unwrap()
        .status,
        "corrected"
    );
}

#[test]
fn hard_pressure_does_not_block_a_user_requested_content_deletion() {
    let fixture = fixture();
    let db = fixture.state().core.db.get().unwrap();
    pressure(&db, "explicit_write_blocked");
    assert_eq!(
        delete_memory_with_revision(
            &db,
            &fixture.principal,
            "witness",
            &MemoryMutationRequest {
                expected_revision: 1
            },
            102
        )
        .unwrap()
        .status,
        "deleted"
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memory_facts WHERE memory_id='witness'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn hard_pressure_preserves_read_only_export_of_all_existing_owned_content() {
    let fixture = fixture();
    let db = fixture.state().core.db.get().unwrap();
    pressure(&db, "explicit_write_blocked");
    let before = private_state(&db, &fixture.principal);
    db.pragma_update(None, "query_only", "ON").unwrap();
    let export = export_memory(&db, &fixture.principal, 102).unwrap();
    assert_eq!(export.items.len(), 1);
    assert_eq!(export.items[0].content, "Keep the original fact");
    assert_eq!(private_state(&db, &fixture.principal), before);
}

#[tokio::test]
async fn operational_enqueue_does_not_recreate_an_existing_principals_missing_quota() {
    let fixture = crate::memory::completion_test_fixtures::GenerationFixture::new().await;
    let db = fixture.fixture.state().core.db.get().unwrap();
    db.execute(
        "DELETE FROM memory_principal_quotas WHERE principal_id=?1",
        [&fixture.fixture.principal],
    )
    .unwrap();
    let before = private_state(&db, &fixture.fixture.principal);
    drop(db);
    let queued = crate::memory::jobs::enqueue_turn_memory_jobs(
        fixture.fixture.state(),
        &fixture.task,
        None,
        None,
        true,
    )
    .unwrap();
    let db = fixture.fixture.state().core.db.get().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memory_principal_quotas WHERE principal_id=?1",
            [&fixture.fixture.principal],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0,
        "enqueue is not policy initialization or an authorized repair"
    );
    assert!(queued.is_empty());
    assert_eq!(private_state(&db, &fixture.fixture.principal), before);
    assert_eq!(fixture.provider_request_count(), 0);
}

#[tokio::test]
async fn periodic_usage_refresh_does_not_recreate_a_missing_principal_policy() {
    let fixture = crate::memory::completion_test_fixtures::GenerationFixture::new().await;
    let db = fixture.fixture.state().core.db.get().unwrap();
    db.execute(
        "DELETE FROM memory_principal_quotas WHERE principal_id=?1",
        [&fixture.fixture.principal],
    )
    .unwrap();
    crate::memory::retention::cleanup_memory_data(
        &db,
        &fixture.fixture.state().policy.memory,
        crate::now_ts_u64() as i64,
    )
    .unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memory_principal_quotas WHERE principal_id=?1",
            [&fixture.fixture.principal],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0,
        "usage maintenance cannot silently create policy for an already initialized account"
    );
    assert_eq!(
        crate::memory::capacity::automatic_generation_availability(&db, &fixture.fixture.principal)
            .unwrap(),
        crate::memory::capacity::GenerationAvailability::PrincipalQuotaMissing
    );
    assert_eq!(fixture.provider_request_count(), 0);
}
