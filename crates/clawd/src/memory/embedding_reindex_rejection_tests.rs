use rusqlite::{types::Value, Connection};

#[test]
fn denied_reindex_does_not_install_schema_receipts_or_profile_state() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let key = crate::repo::auth::create_auth_key(&state, "user").unwrap();
    let identity = crate::resolve_auth_identity_by_key(&state, &key)
        .unwrap()
        .unwrap();
    let db = state.core.db.get().unwrap();
    crate::memory::indexing::ensure_retrieval_schema(&db).unwrap();
    let mut config = state.policy.memory.clone();
    config.embedding_provider_kind = "remote_http".into();
    config.embedding_endpoint_ref = "MEMORY_EMBEDDING_TEST_ENDPOINT".into();
    config.embedding_credential_ref = "MEMORY_EMBEDDING_TEST_TOKEN".into();
    config.embedding_model = "fixture-embedding".into();
    config.embedding_dims = 3;
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name='memory_embedding_jobs'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        0
    );
    let before = persisted_state(&db);
    let error = super::enqueue_reindex(
        &db,
        &config,
        &identity.principal_id,
        "sha256:no-consent",
        false,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "memory_embedding_remote_consent_required"
    );
    assert!(db.is_autocommit());
    assert_eq!(persisted_state(&db), before);
}

fn persisted_state(db: &Connection) -> Vec<Vec<Vec<Value>>> {
    [
        "SELECT type,name,tbl_name,rootpage,sql FROM sqlite_schema ORDER BY type,name",
        "SELECT * FROM runtime_schema_migrations ORDER BY migration_id",
        "SELECT * FROM sqlite_sequence ORDER BY name",
    ]
    .into_iter()
    .map(|sql| {
        let mut statement = db.prepare(sql).unwrap();
        let columns = statement.column_count();
        let rows = statement
            .query_map([], |row| {
                (0..columns)
                    .map(|column| row.get(column))
                    .collect::<rusqlite::Result<Vec<Value>>>()
            })
            .unwrap();
        rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
    })
    .collect()
}
