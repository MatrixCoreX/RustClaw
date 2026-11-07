use super::*;

#[test]
fn unrelated_main_database_growth_does_not_reject_memory_growth_below_domain_budget() {
    let mut fixture = support::fixture(false);
    let (domain, page) = {
        let db = fixture.state().core.db.get().unwrap();
        (
            support::domain(&db, &fixture),
            db.query_row("PRAGMA page_size", [], |row| row.get::<_, u64>(0))
                .unwrap(),
        )
    };
    let budget = domain + page * 32;
    {
        let db = fixture.state().core.db.get().unwrap();
        db.execute_batch("CREATE TABLE unrelated_history(payload BLOB)")
            .unwrap();
        db.execute(
            "INSERT INTO unrelated_history VALUES(zeroblob(?1))",
            [i64::try_from(budget * 2).unwrap()],
        )
        .unwrap();
        assert_eq!(support::domain(&db, &fixture), domain);
    }
    support::limit(&mut fixture, budget);
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        crate::memory::capacity::StoragePressure::load(&db).unwrap(),
        crate::memory::capacity::StoragePressure::ExplicitWriteBlocked
    );
    let result = correct_memory(
        &db,
        &fixture.principal,
        &fixture.principal,
        "witness",
        &MemoryCorrectionRequest {
            expected_revision: 1,
            content: "A small explicit correction".into(),
        },
        103,
    )
    .unwrap();
    assert_eq!(result.status, "corrected");
    assert!(support::domain(&db, &fixture) <= budget);
    assert!(
        !crate::memory::retention::automatic_generation_allowed(&db, &fixture.principal).unwrap()
    );
}

#[test]
fn duplicate_only_and_empty_previews_are_metadata_not_private_payload_growth() {
    let mut fixture = support::fixture(false);
    let export = {
        let db = fixture.state().core.db.get().unwrap();
        export_memory(&db, &fixture.principal, 101).unwrap()
    };
    support::hard(&mut fixture);
    let db = fixture.state().core.db.get().unwrap();
    let before = support::without_sessions(support::state(&db));
    let duplicate = preview_memory_import(
        &db,
        &fixture.principal,
        &MemoryImportPreviewRequest { export },
        103,
    )
    .unwrap();
    assert_eq!(
        (duplicate.accepted_items, duplicate.duplicate_items),
        (0, 1)
    );
    let mut export = export_memory(&db, &fixture.principal, 101).unwrap();
    export.items.clear();
    support::checksum(&mut export);
    let empty = preview_memory_import(
        &db,
        &fixture.principal,
        &MemoryImportPreviewRequest { export },
        104,
    )
    .unwrap();
    assert_eq!(empty.accepted_items, 0);
    assert_eq!(support::without_sessions(support::state(&db)), before);
}

#[test]
fn existing_preview_reuse_under_pressure_does_not_write_or_consume_receipt() {
    let mut fixture = support::fixture(false);
    let (request, first) = {
        let db = fixture.state().core.db.get().unwrap();
        let request = support::request(&db, &fixture.principal);
        let first = preview_memory_import(&db, &fixture.principal, &request, 101).unwrap();
        (request, first)
    };
    support::hard(&mut fixture);
    let db = fixture.state().core.db.get().unwrap();
    let before = support::state(&db);
    let changed = db.total_changes();
    let reused = preview_memory_import(&db, &fixture.principal, &request, 103).unwrap();
    assert_eq!(reused.import_id, first.import_id);
    assert_eq!(reused.payload_digest, first.payload_digest);
    assert_eq!(support::state(&db), before);
    assert_eq!(db.total_changes(), changed);
}

#[test]
fn confirmation_after_competing_import_only_updates_metadata() {
    let mut fixture = support::fixture(false);
    let first = {
        let db = fixture.state().core.db.get().unwrap();
        let first_request = support::request(&db, &fixture.principal);
        let first = preview_memory_import(&db, &fixture.principal, &first_request, 101).unwrap();
        let mut second_request = support::request(&db, &fixture.principal);
        let mut original = export_memory(&db, &fixture.principal, 101).unwrap();
        second_request.export.items.push(original.items.remove(0));
        support::checksum(&mut second_request.export);
        let second = preview_memory_import(&db, &fixture.principal, &second_request, 101).unwrap();
        assert_ne!(first.import_id, second.import_id);
        assert_eq!(
            (
                first.accepted_items,
                second.accepted_items,
                second.duplicate_items
            ),
            (1, 1, 1)
        );
        assert_eq!(
            confirm_memory_import(&db, &fixture.principal, &setup::confirmation(&second), 102)
                .unwrap()
                .imported_items,
            1
        );
        first
    };
    support::hard(&mut fixture);
    let db = fixture.state().core.db.get().unwrap();
    let before = support::without_sessions(support::state(&db));
    let result =
        confirm_memory_import(&db, &fixture.principal, &setup::confirmation(&first), 103).unwrap();
    assert_eq!((result.imported_items, result.existing_items), (0, 1));
    assert_eq!(support::without_sessions(support::state(&db)), before);
}

#[test]
fn fact_correction_undo_releases_replacement_and_preserves_original_history() {
    let mut fixture = support::fixture(false);
    let (undo, replacement) = {
        let db = fixture.state().core.db.get().unwrap();
        let corrected = correct_memory(
            &db,
            &fixture.principal,
            &fixture.principal,
            "witness",
            &support::large_correction(),
            101,
        )
        .unwrap();
        (
            MemoryUndoRequest {
                revision_id: corrected.revision_id.unwrap(),
            },
            corrected.replacement_memory_id.unwrap(),
        )
    };
    support::hard(&mut fixture);
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        undo_memory_mutation(&db, &fixture.principal, &undo, 103)
            .unwrap()
            .status,
        "restored"
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memory_facts WHERE memory_id=?1",
            [replacement],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let original: String = db
        .query_row(
            "SELECT fact_text FROM memory_facts WHERE memory_id='witness' AND status='active'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(original, support::ORIGINAL);
    assert_eq!(db.query_row("SELECT json_extract(previous_snapshot_json,'$.content') FROM memory_revisions WHERE revision_id=?1",[undo.revision_id],|row|row.get::<_,String>(0)).unwrap(),support::ORIGINAL);
}

#[test]
fn preference_undo_to_smaller_original_is_releasing() {
    let mut fixture = support::fixture(false);
    support::preference(&fixture, "small original");
    let undo = support::correct_preference(&fixture, &"large replacement ".repeat(4096));
    support::hard(&mut fixture);
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        undo_memory_mutation(&db, &fixture.principal, &undo, 103)
            .unwrap()
            .status,
        "restored"
    );
    assert_eq!(
        db.query_row(
            "SELECT pref_value FROM user_preferences WHERE memory_id='budget-pref'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "small original"
    );
}

#[test]
fn unknown_state_still_allows_export_and_user_requested_content_deletion() {
    let fixture = support::fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    db.execute_batch("PRAGMA ignore_check_constraints=ON; UPDATE memory_storage_pressure SET state='future_state';").unwrap();
    let before = support::state(&db);
    db.pragma_update(None, "query_only", true).unwrap();
    let export = export_memory(&db, &fixture.principal, 103).unwrap();
    assert_eq!(export.items.len(), 1);
    assert_eq!(export.items[0].content, support::ORIGINAL);
    assert_eq!(support::state(&db), before);
    db.pragma_update(None, "query_only", false).unwrap();
    assert_eq!(
        delete_memory_with_revision(
            &db,
            &fixture.principal,
            "witness",
            &MemoryMutationRequest {
                expected_revision: 1
            },
            104
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
fn host_available_observation_is_independent_of_main_pages_and_domain_budget() {
    let mut fixture = support::fixture(true);
    support::hard(&mut fixture);
    let (mut report, path) = {
        let db = fixture.state().core.db.get().unwrap();
        let tx = db.unchecked_transaction().unwrap();
        let result = support::report(&tx, &fixture);
        tx.rollback().unwrap();
        result
    };
    assert_eq!(
        report.measurements.database_path_filesystem_available_bytes,
        crate::memory::availability::ByteMeasurement::Unavailable {
            reason: crate::memory::availability::MeasurementUnavailable::NotSampled
        }
    );
    report.measurements.database_path_filesystem_available_bytes =
        crate::memory::availability::filesystem_available(path.as_deref());
    assert!(support::bytes(&report.measurements.database_path_filesystem_available_bytes) > 0);
    assert_eq!(
        support::bytes(&report.pressure.configured_soft_limit_bytes),
        fixture.state().policy.memory.storage_soft_limit_bytes
    );
    assert!(
        support::bytes(&report.measurements.main_sqlite_allocated_page_bytes)
            >= support::bytes(&report.measurements.memory_domain_assigned_btree_bytes)
    );
}
