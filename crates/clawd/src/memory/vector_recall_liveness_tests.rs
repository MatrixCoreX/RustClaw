use std::{path::PathBuf, time::Duration};

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
