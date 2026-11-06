use super::boundary_test_fixtures as setup;
use super::*;

#[path = "ux_import_credential_race.rs"]
mod race;
#[path = "ux_import_credential_test_fixtures.rs"]
mod support;
use support::{fixture, request, snapshot};

#[test]
fn preview_rejects_a_revoked_credential_without_reactivating_or_writing() {
    let fixture = fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    let request = request(&db, &fixture.principal);
    db.execute(
        "UPDATE credential_bindings SET status='revoked' WHERE principal_id=?1",
        [&fixture.principal],
    )
    .unwrap();
    let before = snapshot(&db, &fixture.principal);
    let error = preview_memory_import(&db, &fixture.principal, &request, 101).unwrap_err();
    assert_eq!(error.to_string(), "credential_binding_missing");
    assert_eq!(snapshot(&db, &fixture.principal), before);
}

#[test]
fn preview_rejects_a_disabled_key_without_creating_a_new_binding() {
    let fixture = fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    let request = request(&db, &fixture.principal);
    db.execute(
        "UPDATE auth_keys SET enabled=0 WHERE principal_id=?1",
        [&fixture.principal],
    )
    .unwrap();
    let before = snapshot(&db, &fixture.principal);
    assert_eq!(
        preview_memory_import(&db, &fixture.principal, &request, 101)
            .unwrap_err()
            .to_string(),
        "credential_binding_missing"
    );
    assert_eq!(snapshot(&db, &fixture.principal), before);
}

#[test]
fn preview_rejects_an_inactive_principal_with_an_otherwise_live_binding() {
    let fixture = fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    let request = request(&db, &fixture.principal);
    for status in ["frozen", "merged", "deleted"] {
        db.execute(
            "UPDATE principals SET status=?2 WHERE principal_id=?1",
            params![fixture.principal, status],
        )
        .unwrap();
        let before = snapshot(&db, &fixture.principal);
        assert_eq!(
            preview_memory_import(&db, &fixture.principal, &request, 101)
                .unwrap_err()
                .to_string(),
            "credential_binding_missing"
        );
        assert_eq!(snapshot(&db, &fixture.principal), before);
    }
}

#[test]
fn confirm_rechecks_a_binding_revoked_after_the_valid_preview() {
    let fixture = fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    let preview = setup::preview(&db, &fixture.principal, "A distinct imported fact");
    db.execute(
        "UPDATE credential_bindings SET status='revoked' WHERE principal_id=?1",
        [&fixture.principal],
    )
    .unwrap();
    let before = snapshot(&db, &fixture.principal);
    assert_eq!(
        confirm_memory_import(&db, &fixture.principal, &setup::confirmation(&preview), 101)
            .unwrap_err()
            .to_string(),
        "credential_binding_missing"
    );
    assert_eq!(snapshot(&db, &fixture.principal), before);
    assert!(db.is_autocommit());
}

#[test]
fn preview_cannot_authorize_another_owners_reassigned_binding() {
    let fixture = fixture(false);
    let (_, other) = setup::another_owner(&fixture);
    let db = fixture.state().core.db.get().unwrap();
    let request = request(&db, &fixture.principal);
    db.execute(
        "UPDATE credential_bindings SET principal_id=?2 WHERE principal_id=?1",
        params![fixture.principal, other],
    )
    .unwrap();
    let before = snapshot(&db, &fixture.principal);
    assert_eq!(
        preview_memory_import(&db, &fixture.principal, &request, 101)
            .unwrap_err()
            .to_string(),
        "credential_binding_missing"
    );
    assert_eq!(snapshot(&db, &fixture.principal), before);
}

#[test]
fn an_unrelated_missing_binding_does_not_block_the_valid_import_target() {
    let fixture = fixture(false);
    let (_, other) = setup::another_owner(&fixture);
    let db = fixture.state().core.db.get().unwrap();
    let request = request(&db, &fixture.principal);
    db.execute(
        "DELETE FROM credential_bindings WHERE principal_id=?1",
        [&other],
    )
    .unwrap();
    let bindings = db.total_changes();
    let preview = preview_memory_import(&db, &fixture.principal, &request, 101).unwrap();
    assert_eq!(preview.accepted_items, 1);
    assert_eq!(
        db.total_changes(),
        bindings + 1,
        "only this owner's import session is written"
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM credential_bindings WHERE principal_id=?1",
            [&other],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn live_target_credential_validation_is_a_read_only_operation() {
    let fixture = fixture(false);
    let db = fixture.state().core.db.get().unwrap();
    let before = db.total_changes();
    db.pragma_update(None, "query_only", "ON").unwrap();
    assert_eq!(
        crate::repo::auth::require_active_principal_credential(&db, &fixture.principal).unwrap(),
        fixture.key
    );
    assert_eq!(db.total_changes(), before);
}

#[test]
fn preview_revalidates_revocation_committed_during_its_actual_writer_wait() {
    let fixture = fixture(true);
    let owner = fixture.principal.clone();
    let request = request(&fixture.state().core.db.get().unwrap(), &owner);
    let error = race::revoke_during_write(&fixture, move |state| {
        let db = state.core.db.get()?;
        preview_memory_import(&db, &owner, &request, 101)
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "credential_binding_missing");
    let db = fixture.state().core.db.get().unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM memory_import_sessions", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn confirm_revalidates_revocation_committed_during_its_actual_writer_wait() {
    let fixture = fixture(true);
    let owner = fixture.principal.clone();
    let confirmation = {
        let db = fixture.state().core.db.get().unwrap();
        setup::confirmation(&setup::preview(&db, &owner, "A distinct imported fact"))
    };
    let before = snapshot(&fixture.state().core.db.get().unwrap(), &owner);
    let error = race::revoke_during_write(&fixture, move |state| {
        let db = state.core.db.get()?;
        confirm_memory_import(&db, &owner, &confirmation, 101)
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "credential_binding_missing");
    let db = fixture.state().core.db.get().unwrap();
    let after = snapshot(&db, &fixture.principal);
    assert_eq!(
        after.1, before.1,
        "private canonical/import state is unchanged"
    );
}

#[test]
fn confirmation_persists_the_validated_key_not_an_earlier_revoked_key() {
    let fixture = fixture(false);
    let (live_key, other) = setup::another_owner(&fixture);
    let db = fixture.state().core.db.get().unwrap();
    let original_digest: String = db
        .query_row(
            "SELECT credential_digest FROM credential_bindings WHERE principal_id=?1",
            [&fixture.principal],
            |row| row.get(0),
        )
        .unwrap();
    db.execute(
        "UPDATE auth_keys SET principal_id=?2 WHERE user_key=?1",
        params![live_key, fixture.principal],
    )
    .unwrap();
    assert_eq!(
        db.execute(
            "UPDATE credential_bindings SET principal_id=?2 WHERE principal_id=?1",
            params![other, fixture.principal]
        )
        .unwrap(),
        1
    );
    // Arrange two credentials for the same account, with the original key still
    // enabled but its binding revoked. No production identity repair is called.
    db.execute(
        "UPDATE credential_bindings SET status='revoked' WHERE credential_digest=?1",
        [original_digest],
    )
    .unwrap();
    assert_eq!(
        crate::repo::auth::require_active_principal_credential(&db, &fixture.principal).unwrap(),
        live_key
    );
    let preview = setup::preview(&db, &fixture.principal, "A distinct imported fact");
    confirm_memory_import(&db, &fixture.principal, &setup::confirmation(&preview), 101).unwrap();
    let stored: String = db
        .query_row(
            "SELECT user_key FROM memory_facts WHERE source_kind='memory_import'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, live_key);
}
