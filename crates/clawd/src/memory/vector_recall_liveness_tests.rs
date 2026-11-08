use std::{cell::Cell, path::PathBuf, time::Duration};

use rusqlite::{params, Connection};

use super::{ExactSqliteVectorIndex, MemoryEmbeddingProfile, MemoryVectorIndex, VectorWrite};

struct Fixture {
    state: crate::AppState,
    root: PathBuf,
    principal: String,
    other_principal: String,
    profile: MemoryEmbeddingProfile,
    query: Vec<f32>,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "memory-recall-liveness-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let mut directory = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            directory.mode(0o700);
        }
        directory.create(&root).unwrap();
        println!(
            "memory_recall_liveness_fixture_workspace={}",
            root.display()
        );
        let initial = Connection::open(root.join("fixture.sqlite")).unwrap();
        initial.pragma_update(None, "journal_mode", "WAL").unwrap();
        drop(initial);
        let manager = r2d2_sqlite::SqliteConnectionManager::file(root.join("fixture.sqlite"))
            .with_init(|db: &mut Connection| {
                db.busy_timeout(Duration::ZERO)?;
                db.pragma_update(None, "foreign_keys", "ON")?;
                Ok(())
            });
        let mut state = crate::AppState::test_default_with_fixture_provider();
        state.core.db = r2d2::Pool::builder().max_size(2).build(manager).unwrap();
        state.skill_rt.workspace_root = root.clone();
        state.skill_rt.default_locator_search_dir = root.clone();
        let state = state.with_seeded_db_schema();
        state.seed_test_auth_identity("recall-liveness-owner", "user");
        state.seed_test_auth_identity("recall-liveness-other", "user");
        let db = state.core.db.get().unwrap();
        crate::memory::indexing::ensure_retrieval_schema(&db).unwrap();
        crate::memory::scope::ensure_memory_scope_schema(&db).unwrap();
        let principal = crate::repo::auth::principal_id_for_user_key(&db, "recall-liveness-owner")
            .unwrap()
            .unwrap();
        let other_principal =
            crate::repo::auth::principal_id_for_user_key(&db, "recall-liveness-other")
                .unwrap()
                .unwrap();
        let profile = super::register_configured_profile(&db, &state.policy.memory).unwrap();
        crate::memory::project_identity::resolve_project_identity(&db, &root).unwrap();
        drop(db);
        let mut query = vec![0.0; profile.dimensions];
        query[0] = 1.0;
        Self {
            state,
            root,
            principal,
            other_principal,
            profile,
            query,
        }
    }

    fn vector(&self, id: i64, owner: &str, vector: &[f32]) {
        let db = self.state.core.db.get().unwrap();
        ExactSqliteVectorIndex
            .upsert(
                &db,
                &self.profile,
                &VectorWrite {
                    retrieval_id: id,
                    principal_id: owner,
                    scope_kind: "principal",
                    scope_ref: owner,
                    projection_digest: "sha256:recall-liveness",
                    vector,
                },
            )
            .unwrap();
    }

    fn projection(&self, id: i64, owner: &str, kind: &str, scope: &str) {
        self.state
            .core
            .db
            .get()
            .unwrap()
            .execute(
                "INSERT INTO memory_retrieval_index(
                id,source_kind,user_id,chat_id,user_key,principal_id,scope_kind,
                scope_ref,memory_kind,search_text,topic_tags,vector_json,
                embedding_model,embedding_dims,embedding_version,metadata_json,
                created_at_ts,updated_at_ts
             ) VALUES (?1,'memory_fact',1,0,'recall-liveness-owner',?2,?3,?4,
                       'semantic_fact','eligible projection','','[]',
                       'pending',0,'pending','{}',1,1)",
                params![id, owner, kind, scope],
            )
            .unwrap();
    }

    fn valid_candidate(&self) {
        self.projection(1, &self.principal, "principal", &self.principal);
        let mut vector = vec![0.0; self.profile.dimensions];
        vector[1] = 1.0;
        self.vector(1, &self.principal, &vector);
    }

    fn assert_valid_candidate_wins(&self, db: &Connection) {
        let access =
            crate::memory::scope::resolve_memory_access(db, &self.principal, None, None).unwrap();
        let neighbors = ExactSqliteVectorIndex
            .nearest(db, &access, &self.profile, &self.query, 1)
            .unwrap();
        assert_eq!(neighbors.len(), 1);
        assert_eq!(
            neighbors[0].retrieval_id, 1,
            "ineligible vectors must be excluded before the top-k budget is consumed"
        );
    }
}

// Retain the private workspace on both success and failure as test evidence.
// No Drop implementation deletes databases, directories, or other test fixtures.

#[test]
fn nearest_excludes_missing_projection_before_top_k_without_cleanup() {
    let fixture = Fixture::new();
    fixture.valid_candidate();
    fixture.vector(2, &fixture.principal, &fixture.query);
    let db = fixture.state.core.db.get().unwrap();
    let changes = db.total_changes();
    fixture.assert_valid_candidate_wins(&db);
    assert_eq!(db.total_changes(), changes);
    assert_eq!(
        db.query_row(
            "SELECT status FROM memory_vector_rows WHERE retrieval_id=2",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        "active",
        "query filtering must not perform lazy write repair"
    );
}

#[test]
fn nearest_excludes_mismatched_owner_and_scope_before_top_k() {
    let fixture = Fixture::new();
    fixture.valid_candidate();
    for (id, owner, kind, scope) in [
        (
            2,
            fixture.other_principal.as_str(),
            "principal",
            fixture.other_principal.as_str(),
        ),
        (3, fixture.principal.as_str(), "project", "other-project"),
        (
            4,
            fixture.principal.as_str(),
            "principal",
            "other-principal-scope",
        ),
    ] {
        fixture.projection(id, owner, kind, scope);
        fixture.vector(id, &fixture.principal, &fixture.query);
    }
    let db = fixture.state.core.db.get().unwrap();
    let changes = db.total_changes();
    fixture.assert_valid_candidate_wins(&db);
    assert_eq!(db.total_changes(), changes);
}

#[test]
fn nearest_filters_orphans_while_an_independent_wal_writer_is_held() {
    let fixture = Fixture::new();
    fixture.valid_candidate();
    fixture.vector(2, &fixture.principal, &fixture.query);
    let writer = Connection::open(fixture.root.join("fixture.sqlite")).unwrap();
    writer.busy_timeout(Duration::ZERO).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let db = fixture.state.core.db.get().unwrap();
    let error = db
        .execute("DELETE FROM memory_vector_rows WHERE 0", [])
        .unwrap_err();
    assert_eq!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy)
    );
    fixture.assert_valid_candidate_wins(&db);
    assert!(!writer.is_autocommit());
    writer.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn ordinary_recall_does_not_mutate_another_principals_orphan_vectors() {
    let fixture = Fixture::new();
    fixture.vector(9, &fixture.other_principal, &fixture.query);
    let snapshot = || {
        fixture
            .state
            .core
            .db
            .get()
            .unwrap()
            .query_row(
                "SELECT status,updated_at_ts,vector_blob,vector_checksum,projection_digest
             FROM memory_vector_rows WHERE retrieval_id=9 AND principal_id=?1",
                [&fixture.other_principal],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .unwrap()
    };
    let before = snapshot();
    assert_eq!(before.0, "active");
    crate::memory::retrieval::retrieve_indexed_memories_for_scope(
        &fixture.state,
        Some("recall-liveness-owner"),
        1,
        0,
        None,
        "ordinary recall",
    )
    .unwrap();
    assert_eq!(
        snapshot(),
        before,
        "one user's recall must not perform global mutation of another user's vectors"
    );
}

thread_local! {
    static BUSY_CALLBACKS: Cell<usize> = const { Cell::new(0) };
}

fn count_busy_attempt(_attempt: i32) -> bool {
    BUSY_CALLBACKS.with(|count| count.set(count.get() + 1));
    false
}

#[test]
fn warmed_ordinary_recall_does_not_attempt_an_independent_wal_writer() {
    let fixture = Fixture::new();
    {
        let first = fixture.state.core.db.get().unwrap();
        let second = fixture.state.core.db.get().unwrap();
        first.busy_handler(Some(count_busy_attempt)).unwrap();
        second.busy_handler(Some(count_busy_attempt)).unwrap();
    }
    let writer = Connection::open(fixture.root.join("fixture.sqlite")).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    BUSY_CALLBACKS.with(|count| count.set(0));
    {
        let db = fixture.state.core.db.get().unwrap();
        let error = db
            .execute("DELETE FROM memory_vector_rows WHERE 0", [])
            .unwrap_err();
        assert_eq!(
            error.sqlite_error_code(),
            Some(rusqlite::ErrorCode::DatabaseBusy)
        );
    }
    assert!(
        BUSY_CALLBACKS.with(Cell::get) > 0,
        "the actual SQLite busy handler must be observable"
    );
    BUSY_CALLBACKS.with(|count| count.set(0));
    crate::memory::retrieval::retrieve_indexed_memories_for_scope(
        &fixture.state,
        Some("recall-liveness-owner"),
        1,
        0,
        None,
        "ordinary recall",
    )
    .unwrap();
    assert_eq!(
        BUSY_CALLBACKS.with(Cell::get),
        0,
        "an ordinary warmed recall must not attempt a maintenance write, even when no rows change"
    );
    assert!(!writer.is_autocommit());
    writer.execute_batch("ROLLBACK").unwrap();
}

fn canonical_fact(fixture: &Fixture, owner_key: &str, owner: &str) -> i64 {
    let now = crate::now_ts_u64() as i64;
    let fact = crate::memory::facts::MemoryFactUpsert::from_long_term_summary(
        "liveness",
        "retained",
        "stable-value",
        "retained canonical fact",
        1.0,
        "fixture-source",
        &[],
        "fixture evidence",
        None,
    );
    let db = fixture.state.core.db.get().unwrap();
    let fact_id = crate::memory::facts::upsert_memory_fact_card(&db, 1, 0, owner_key, &fact, now)
        .unwrap()
        .unwrap();
    let id = db.query_row(
        "SELECT id FROM memory_retrieval_index WHERE principal_id=?1 AND source_kind='memory_fact' AND source_ref=?2",
        params![owner, crate::memory::retrieval_source_ref_for_memory_fact(fact_id)],
        |row| row.get::<_, i64>(0),
    ).unwrap();
    drop(db);
    fixture.vector(id, owner, &fixture.query);
    id
}

fn vector_snapshot(db: &Connection, id: i64) -> (String, Vec<u8>, String, i64) {
    db.query_row(
        "SELECT status,vector_blob,vector_checksum,updated_at_ts FROM memory_vector_rows WHERE retrieval_id=?1",
        [id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).unwrap()
}

#[test]
fn background_maintenance_preserves_canonical_fact_and_retires_only_orphans() {
    let fixture = Fixture::new();
    let live = canonical_fact(&fixture, "recall-liveness-owner", &fixture.principal);
    fixture.vector(99, &fixture.other_principal, &fixture.query);
    fixture.projection(
        98,
        &fixture.other_principal,
        "principal",
        &fixture.other_principal,
    );
    fixture.vector(98, &fixture.principal, &fixture.query);
    let db = fixture.state.core.db.get().unwrap();
    let before = vector_snapshot(&db, live);
    let now = crate::now_ts_u64() as i64;
    crate::memory::retention::cleanup_memory_data(&db, &fixture.state.policy.memory, now).unwrap();
    assert_eq!(
        vector_snapshot(&db, live),
        before,
        "a healthy canonical fact with its actual versioned source ref must remain searchable"
    );
    for id in [98, 99] {
        assert_eq!(vector_snapshot(&db, id).0, "tombstone");
    }
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index WHERE id=?1",
            [live],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        1
    );
    let before = vector_snapshot(&db, 99);
    crate::memory::retention::cleanup_memory_data(&db, &fixture.state.policy.memory, now + 1)
        .unwrap();
    assert_eq!(
        vector_snapshot(&db, 99),
        before,
        "already retired vectors are not rewritten"
    );
}

#[test]
fn background_maintenance_retires_fact_projection_owned_by_the_wrong_principal() {
    let fixture = Fixture::new();
    let live = canonical_fact(&fixture, "recall-liveness-other", &fixture.other_principal);
    let db = fixture.state.core.db.get().unwrap();
    let source_ref: String = db
        .query_row(
            "SELECT source_ref FROM memory_retrieval_index WHERE id=?1",
            [live],
            |row| row.get(0),
        )
        .unwrap();
    drop(db);
    fixture.projection(99, &fixture.principal, "principal", &fixture.principal);
    fixture.vector(99, &fixture.principal, &fixture.query);
    let db = fixture.state.core.db.get().unwrap();
    db.execute(
        "UPDATE memory_retrieval_index SET source_ref=?1 WHERE id=99",
        [source_ref],
    )
    .unwrap();
    let before = vector_snapshot(&db, live);
    crate::memory::retention::cleanup_memory_data(
        &db,
        &fixture.state.policy.memory,
        crate::now_ts_u64() as i64,
    )
    .unwrap();
    assert_eq!(vector_snapshot(&db, live), before);
    assert_eq!(vector_snapshot(&db, 99).0, "tombstone");
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index WHERE id=99",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        0
    );
}

#[test]
fn failed_orphan_maintenance_rolls_back_other_retention_deletions() {
    let fixture = Fixture::new();
    fixture.vector(99, &fixture.other_principal, &fixture.query);
    let db = fixture.state.core.db.get().unwrap();
    db.execute(
        "INSERT INTO memories(memory_id,user_id,chat_id,user_key,principal_id,scope_kind,
         scope_ref,channel,role,content,created_at,created_at_ts)
         VALUES ('rollback-source',1,0,'recall-liveness-owner',?1,'principal',?1,
                 'ui','user','retention rollback witness','0',0)",
        [&fixture.principal],
    )
    .unwrap();
    db.execute_batch(
        "CREATE TRIGGER reject_orphan_retirement BEFORE UPDATE OF status ON memory_vector_rows
         WHEN OLD.retrieval_id=99 BEGIN SELECT RAISE(ABORT,'fixture_orphan_retirement_rejected'); END;",
    ).unwrap();
    let before = vector_snapshot(&db, 99);
    let error = crate::memory::retention::cleanup_memory_data(
        &db,
        &fixture.state.policy.memory,
        crate::now_ts_u64() as i64,
    )
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("fixture_orphan_retirement_rejected"));
    assert!(db.is_autocommit());
    assert_eq!(vector_snapshot(&db, 99), before);
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memories WHERE memory_id='rollback-source'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        1,
        "unrelated earlier retention deletes must roll back atomically"
    );
}

#[test]
fn orphan_maintenance_does_not_install_an_uninitialized_optional_pipeline() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE runtime_schema_migrations(migration_id TEXT PRIMARY KEY,schema_digest TEXT,applied_at TEXT);",
    ).unwrap();
    let before = db.total_changes();
    assert_eq!(super::maintain_orphaned_vectors(&db).unwrap(), 0);
    assert_eq!(db.total_changes(), before);
    assert_eq!(db.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'memory_vector_%' OR name LIKE 'memory_embedding_%'", [],
        |row| row.get::<_, i64>(0),
    ).unwrap(), 0);
}
