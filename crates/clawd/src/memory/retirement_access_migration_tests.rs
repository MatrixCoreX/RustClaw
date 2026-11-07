use super::{fixtures::*, *};

fn schema(db: &Connection) -> Vec<(String, String, Option<String>)> {
    db.prepare("SELECT type,name,sql FROM main.sqlite_schema ORDER BY type,name")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn receipt_count(db: &Connection) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM main.runtime_schema_migrations WHERE migration_id=?1",
        [MIGRATION_ID],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn upgrade_preserves_duplicates_data_and_prior_receipts() {
    let db = legacy_db();
    projection(&db, 1, A, "memory_fact", Some("shared"));
    projection(&db, 2, A, "memory_fact", Some("shared"));
    db.execute(
        "INSERT INTO runtime_schema_migrations VALUES ('prior-fixture','unchanged','0')",
        [],
    )
    .unwrap();
    let before = snapshot(&db);
    let tables = db
        .prepare("SELECT name,sql FROM sqlite_schema WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    ensure_retirement_access_schema(&db).unwrap();
    assert_eq!(snapshot(&db), before);
    assert_eq!(receipt_count(&db), 1);
    assert_eq!(db.query_row("SELECT schema_digest FROM runtime_schema_migrations WHERE migration_id='prior-fixture'", [], |row| row.get::<_,String>(0)).unwrap(), "unchanged");
    assert_eq!(
        db.prepare("SELECT name,sql FROM sqlite_schema WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?
            )))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap(),
        tables
    );
    validate_indexes(&db).unwrap();
}

#[test]
fn repeated_initialization_is_read_only_and_query_only_safe() {
    let db = initialized_db(false);
    let before = (schema(&db), snapshot(&db), db.total_changes());
    db.execute_batch("PRAGMA query_only=ON").unwrap();
    for _ in 0..3 {
        ensure_retirement_access_schema(&db).unwrap();
    }
    assert_eq!((schema(&db), snapshot(&db), db.total_changes()), before);
}

#[test]
fn digest_mismatch_precedes_any_schema_write() {
    let db = initialized_db(false);
    db.execute(
        "UPDATE runtime_schema_migrations SET schema_digest='wrong' WHERE migration_id=?1",
        [MIGRATION_ID],
    )
    .unwrap();
    let before = (schema(&db), snapshot(&db), db.total_changes());
    let error = ensure_retirement_access_schema(&db).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("runtime_schema_migration_digest_mismatch:{MIGRATION_ID}")
    );
    assert_eq!((schema(&db), snapshot(&db), db.total_changes()), before);
}

#[test]
fn unreceipted_wrong_projection_index_shapes_reject_atomically() {
    for definition in [
        "CREATE UNIQUE INDEX idx_memory_retrieval_owned_source ON memory_retrieval_index(principal_id,scope_kind,scope_ref,source_kind,source_ref)",
        "CREATE INDEX idx_memory_retrieval_owned_source ON memory_retrieval_index(principal_id,scope_kind,scope_ref,source_kind,source_ref) WHERE principal_id IS NOT NULL",
        "CREATE INDEX idx_memory_retrieval_owned_source ON memory_retrieval_index(principal_id,scope_kind,scope_ref,source_ref,source_kind)",
        "CREATE INDEX idx_memory_retrieval_owned_source ON memory_retrieval_index(principal_id,scope_kind,scope_ref,source_kind,source_ref DESC)",
        "CREATE INDEX idx_memory_retrieval_owned_source ON memory_retrieval_index(principal_id,scope_kind,scope_ref,source_kind,source_ref COLLATE NOCASE)",
        "CREATE INDEX idx_memory_retrieval_owned_source ON memory_retrieval_index(principal_id,scope_kind,scope_ref,source_kind,lower(source_ref))",
        "CREATE INDEX idx_memory_retrieval_owned_source ON memory_embedding_jobs(retrieval_id)",
    ] {
        let db = legacy_db();
        db.execute_batch(definition).unwrap();
        let before = (schema(&db), snapshot(&db));
        assert_eq!(ensure_retirement_access_schema(&db).unwrap_err().to_string(),
            "memory_retirement_access_index_invalid:idx_memory_retrieval_owned_source", "{definition}");
        assert_eq!((schema(&db), snapshot(&db)), before);
        assert_eq!(receipt_count(&db), 0);
        assert!(db.is_autocommit());
    }
}

#[test]
fn wrong_job_index_rolls_back_projection_index_and_receipt() {
    let db = legacy_db();
    db.execute_batch(
        "CREATE INDEX idx_memory_embedding_jobs_retirement ON memory_embedding_jobs(status)",
    )
    .unwrap();
    let before = (schema(&db), snapshot(&db));
    assert_eq!(
        ensure_retirement_access_schema(&db)
            .unwrap_err()
            .to_string(),
        "memory_retirement_access_index_invalid:idx_memory_embedding_jobs_retirement"
    );
    assert_eq!((schema(&db), snapshot(&db)), before);
    assert_eq!(receipt_count(&db), 0);
}

#[test]
fn missing_applied_index_is_not_repaired() {
    let db = initialized_db(false);
    db.execute_batch("DROP INDEX idx_memory_embedding_jobs_retirement; PRAGMA query_only=ON")
        .unwrap();
    let before = schema(&db);
    assert_eq!(
        ensure_retirement_access_schema(&db)
            .unwrap_err()
            .to_string(),
        "memory_retirement_access_index_invalid:idx_memory_embedding_jobs_retirement"
    );
    assert_eq!(schema(&db), before);
    assert_eq!(receipt_count(&db), 1);
}

#[test]
fn outer_transaction_can_roll_back_successful_migration() {
    let db = legacy_db();
    let before = schema(&db);
    let tx = db.unchecked_transaction().unwrap();
    ensure_retirement_access_schema(&tx).unwrap();
    assert!(!tx.is_autocommit());
    assert_eq!(receipt_count(&tx), 1);
    tx.rollback().unwrap();
    assert_eq!(schema(&db), before);
    assert_eq!(receipt_count(&db), 0);
}

#[test]
fn failed_nested_migration_preserves_caller_work() {
    let db = legacy_db();
    db.execute_batch("CREATE TABLE caller_work(value INTEGER); CREATE INDEX idx_memory_embedding_jobs_retirement ON memory_embedding_jobs(status)").unwrap();
    let tx = db.unchecked_transaction().unwrap();
    tx.execute("INSERT INTO caller_work VALUES (7)", [])
        .unwrap();
    assert!(ensure_retirement_access_schema(&tx).is_err());
    assert!(!tx.is_autocommit());
    tx.commit().unwrap();
    assert_eq!(
        db.query_row("SELECT value FROM caller_work", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        7
    );
    assert_eq!(receipt_count(&db), 0);
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name='idx_memory_retrieval_owned_source'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn missing_jobs_schema_is_not_initialized_by_index_migration() {
    let db = Connection::open_in_memory().unwrap();
    create_legacy_schema(&db, false);
    let before = schema(&db);
    assert!(ensure_retirement_access_schema(&db).is_err());
    assert_eq!(schema(&db), before);
    assert_eq!(receipt_count(&db), 0);
}

#[test]
fn readonly_first_initialization_has_no_partial_adoption() {
    let db = legacy_db();
    let before = schema(&db);
    db.execute_batch("PRAGMA query_only=ON").unwrap();
    assert!(ensure_retirement_access_schema(&db).is_err());
    assert!(db.is_autocommit());
    assert_eq!(schema(&db), before);
    assert_eq!(receipt_count(&db), 0);
}

#[test]
fn concurrent_initializers_publish_one_valid_receipt() {
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory = Directory(std::env::temp_dir().join(format!(
        "agent-retirement-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    )));
    std::fs::create_dir(&directory.0).unwrap();
    let path = directory.0.join("fixture.db");
    {
        let db = Connection::open(&path).unwrap();
        create_legacy_schema(&db, true);
    }
    let first = Connection::open(&path).unwrap();
    let second = Connection::open(&path).unwrap();
    first
        .busy_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    second
        .busy_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles = [first, second]
        .into_iter()
        .map(|db| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                ensure_retirement_access_schema(&db)
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().unwrap().unwrap();
    }
    let db = Connection::open(&path).unwrap();
    validate_indexes(&db).unwrap();
    assert_eq!(receipt_count(&db), 1);
}

#[test]
fn validation_and_receipts_are_bound_to_main_not_temp_shadows() {
    let db = legacy_db();
    db.execute_batch(
        "CREATE TEMP TABLE memory_retrieval_index(source_ref TEXT);
         CREATE INDEX temp.idx_memory_retrieval_owned_source ON memory_retrieval_index(source_ref);
         CREATE TEMP TABLE runtime_schema_migrations(migration_id TEXT,schema_digest TEXT,applied_at TEXT);
         INSERT INTO temp.runtime_schema_migrations VALUES ('033_memory_owned_retirement_access_v1','wrong','0');",
    ).unwrap();
    ensure_retirement_access_schema(&db).unwrap();
    validate_indexes(&db).unwrap();
    assert_eq!(receipt_count(&db), 1);
    assert_eq!(
        db.query_row(
            "SELECT schema_digest FROM temp.runtime_schema_migrations",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "wrong"
    );
}
