use super::boundary_support::*;
use super::*;
use crate::memory::indexing;

#[test]
fn missing_feedback_ledger_rejects_index_refresh_without_effects_or_read_repair() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    seed(
        &db,
        &fixture.key,
        &fixture.principal,
        "preference",
        "missing-ledger",
    );
    db.execute_batch("DROP TABLE memory_retrieval_feedback")
        .unwrap();
    let before = state_without_feedback(&db);
    let result = indexing::index_preference_entries(
        &db,
        1,
        7,
        &fixture.key,
        &[(
            "response_format".into(),
            "markdown".into(),
            1.0,
            "user_message".into(),
        )],
        201,
    );
    assert!(
        result.is_err(),
        "an admitted but missing feedback ledger must not imply unrestricted recall"
    );
    assert_eq!(
        state_without_feedback(&db),
        before,
        "failed policy validation must not delete or rewrite any projection"
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='memory_retrieval_feedback'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0,
        "index refresh must not repair missing policy state"
    );
}

#[test]
fn failed_fts_insert_rolls_back_feedback_canonical_revision_and_projection() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    seed(&db, &fixture.key, &fixture.principal, "fact", "failed-fts");
    db.execute_batch(
        "CREATE TRIGGER fail_projection_fts BEFORE INSERT ON memory_retrieval_index_fts_content
        BEGIN SELECT RAISE(ABORT,'projection_fts_fixture_failed'); END;",
    )
    .unwrap();
    let before = state(&db);
    let result = record_feedback(
        &db,
        &fixture.principal,
        "failed-fts",
        &feedback(MemoryFeedbackKind::Incorrect),
        200,
    );
    assert!(
        result.is_err(),
        "present but failing FTS must not commit a corrected canonical row"
    );
    assert_eq!(
        state(&db),
        before,
        "FTS failure must roll back feedback,canonical,revision,index and outbox together"
    );
}

#[test]
fn absent_optional_fts_does_not_block_correction_or_initialize_a_table() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    seed(&db, &fixture.key, &fixture.principal, "fact", "no-fts");
    db.execute_batch("DROP TABLE memory_retrieval_index_fts")
        .unwrap();
    let result = correct_memory(
        &db,
        &fixture.principal,
        &fixture.principal,
        "no-fts",
        &correction(),
        200,
    )
    .unwrap();
    let replacement = result.replacement_memory_id.unwrap();
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index i JOIN memory_facts f
        ON i.source_ref=?3||CAST(f.id AS TEXT) AND i.principal_id=f.principal_id
        WHERE f.memory_id=?1 AND f.principal_id=?2 AND i.search_text='Corrected private content'",
            params![
                replacement,
                fixture.principal,
                crate::memory::MEMORY_FACT_SOURCE_REF_PREFIX
            ],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='memory_retrieval_index_fts'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn correcting_preference_preserves_another_scope_of_the_same_principal() {
    let fixture = setup::fixture();
    let db = fixture.state().core.db.get().unwrap();
    setup::preference(&db, &fixture.key, &fixture.principal, "other-scope");
    db.execute(
        "UPDATE user_preferences SET chat_id=8,scope_kind='conversation',scope_ref='conversation-b'
        WHERE memory_id='other-scope'",
        [],
    )
    .unwrap();
    indexing::index_preference_entries(
        &db,
        1,
        8,
        &fixture.key,
        &[(
            "response_format".into(),
            "markdown".into(),
            1.0,
            "user_message".into(),
        )],
        100,
    )
    .unwrap();
    seed(
        &db,
        &fixture.key,
        &fixture.principal,
        "preference",
        "own-scope",
    );
    let foreign = scope_rows(&db, &fixture.principal, "conversation-b");
    assert_eq!(
        foreign.len(),
        1,
        "fixture creates a real independent scoped index and FTS entry"
    );
    correct_memory(
        &db,
        &fixture.principal,
        &fixture.principal,
        "own-scope",
        &correction(),
        200,
    )
    .unwrap();
    assert_eq!(
        scope_rows(&db, &fixture.principal, "conversation-b"),
        foreign,
        "same owner must not merge distinct conversations"
    );
    let own = scope_rows(&db, &fixture.principal, &fixture.principal);
    assert_eq!(own.len(), 1);
    assert_eq!(
        own[0].1,
        "Preference response_format: Corrected private content"
    );
    assert_eq!(own[0].3.as_deref(), Some(own[0].1.as_str()));
    assert_no_orphan_fts(&db);
}

#[test]
fn rotating_admin_credential_does_not_make_owned_canonical_facts_uncorrectable() {
    let fixture = setup::fixture();
    let key = crate::repo::auth::create_auth_key(fixture.state(), "admin").unwrap();
    let owner = crate::resolve_auth_identity_by_key(fixture.state(), &key)
        .unwrap()
        .unwrap()
        .principal_id;
    {
        let db = fixture.state().core.db.get().unwrap();
        seed(&db, &key, &owner, "fact", "rotated-fact");
    }
    let rotated = crate::repo::auth::create_auth_key(fixture.state(), "admin")
        .expect("normal administrator credential rotation");
    let identity = crate::resolve_auth_identity_by_key(fixture.state(), &rotated)
        .unwrap()
        .unwrap();
    assert_eq!(
        identity.principal_id, owner,
        "credential rotation retains stable principal ownership"
    );
    assert!(crate::resolve_auth_identity_by_key(fixture.state(), &key)
        .unwrap()
        .is_none());
    let db = fixture.state().core.db.get().unwrap();
    let result = correct_memory(&db, &owner, &owner, "rotated-fact", &correction(), 200)
        .expect("active stable owner can correct a fact after the source credential was rotated");
    assert_replacement(
        &db,
        &owner,
        "fact",
        result.replacement_memory_id.as_deref().unwrap(),
    );
}

#[test]
fn rebuild_after_credential_rotation_preserves_every_owned_canonical_kind() {
    let fixture = setup::fixture();
    let key = crate::repo::auth::create_auth_key(fixture.state(), "admin").unwrap();
    let owner = crate::resolve_auth_identity_by_key(fixture.state(), &key)
        .unwrap()
        .unwrap()
        .principal_id;
    let before = {
        let db = fixture.state().core.db.get().unwrap();
        seed(&db, &key, &owner, "fact", "rotation-rebuild-fact");
        seed(&db, &key, &owner, "preference", "rotation-rebuild-pref");
        seed(&db, &key, &owner, "recent", "rotation-rebuild-recent");
        rows(&db, &owner)
    };
    assert_eq!(
        before.len(),
        4,
        "fact,preference and both transcript projections exist"
    );
    let rotated = crate::repo::auth::create_auth_key(fixture.state(), "admin").unwrap();
    assert_eq!(
        crate::resolve_auth_identity_by_key(fixture.state(), &rotated)
            .unwrap()
            .unwrap()
            .principal_id,
        owner
    );
    assert!(crate::resolve_auth_identity_by_key(fixture.state(), &key)
        .unwrap()
        .is_none());
    let db = fixture.state().core.db.get().unwrap();
    rebuild(&db);
    assert_eq!(
        rows(&db, &owner),
        before,
        "rebuild uses stable ownership,not the expired original credential"
    );
    assert_no_orphan_fts(&db);
    for (table, id) in [
        ("memory_facts", "rotation-rebuild-fact"),
        ("user_preferences", "rotation-rebuild-pref"),
        ("memories", "rotation-rebuild-recent"),
    ] {
        assert_eq!(
            db.query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE memory_id=?1 AND principal_id=?2"),
                params![id, owner],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1,
            "canonical source history must remain intact"
        );
    }
}
