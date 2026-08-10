use super::*;

#[test]
fn vector_status_remains_read_only_while_another_connection_holds_the_writer_lock() {
    let db_path = std::env::temp_dir().join(format!(
        "agent-runtime-vector-status-{}.sqlite",
        uuid::Uuid::new_v4().simple()
    ));
    let manager = r2d2_sqlite::SqliteConnectionManager::file(&db_path).with_init(
        |connection: &mut rusqlite::Connection| {
            connection.busy_timeout(std::time::Duration::from_millis(50))?;
            connection.pragma_update(None, "journal_mode", "WAL")?;
            connection.pragma_update(None, "synchronous", "NORMAL")?;
            connection.pragma_update(None, "foreign_keys", "ON")?;
            Ok(())
        },
    );
    let pool = r2d2::Pool::builder()
        .max_size(2)
        .build(manager)
        .expect("build file-backed vector status pool");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.core.db = pool;
    let state = state.with_seeded_db_schema();
    let user_key =
        crate::repo::auth::create_auth_key(&state, "admin").expect("create vector status identity");
    let principal_id = crate::resolve_auth_identity_by_key(&state, &user_key)
        .expect("resolve vector status identity")
        .expect("vector status identity")
        .principal_id;
    {
        let db = state.core.db.get().expect("vector status setup connection");
        crate::memory::embedding_jobs::initialize_embedding_runtime(&db, &state.policy.memory)
            .expect("initialize embedding runtime");
    }
    {
        let first = state.core.db.get().expect("prime first pool connection");
        let second = state.core.db.get().expect("prime second pool connection");
        drop((first, second));
    }

    let writer = rusqlite::Connection::open(&db_path).expect("open competing writer");
    writer
        .busy_timeout(std::time::Duration::from_millis(50))
        .unwrap();
    writer
        .execute_batch("BEGIN IMMEDIATE")
        .expect("hold sqlite writer lock");
    let status = load_vector_status(&state, &principal_id)
        .expect("vector status must not need the writer lock");
    assert_eq!(status.provider_location, "local");
    writer.execute_batch("ROLLBACK").unwrap();
    drop(writer);
    drop(state);

    for path in [
        db_path.clone(),
        db_path.with_extension("sqlite-wal"),
        db_path.with_extension("sqlite-shm"),
    ] {
        let _ = std::fs::remove_file(path);
    }
}
