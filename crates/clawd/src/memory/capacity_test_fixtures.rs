use std::path::PathBuf;
use std::time::Duration;

use rusqlite::{params, Connection};

pub(crate) struct Fixture {
    state: Option<crate::AppState>,
    pub(crate) principal: String,
    pub(crate) key: String,
    root: Option<PathBuf>,
}

impl Fixture {
    pub(super) fn memory() -> Self {
        Self::finish(crate::AppState::test_default_with_fixture_provider(), None)
    }

    pub(crate) fn file() -> Self {
        let root = std::env::temp_dir().join(format!(
            "agent-runtime-memory-capacity-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&root).expect("isolated capacity directory");
        let manager = r2d2_sqlite::SqliteConnectionManager::file(root.join("fixture.sqlite"))
            .with_init(|db: &mut Connection| {
                db.busy_timeout(Duration::from_millis(50))?;
                db.pragma_update(None, "journal_mode", "WAL")?;
                db.pragma_update(None, "foreign_keys", "ON")?;
                Ok(())
            });
        let mut state = crate::AppState::test_default_with_fixture_provider();
        state.core.db = r2d2::Pool::builder()
            .max_size(2)
            .build(manager)
            .expect("isolated capacity pool");
        Self::finish(state, Some(root))
    }

    fn finish(mut state: crate::AppState, root: Option<PathBuf>) -> Self {
        state.policy.memory.embedding_reindex_batch_delay_ms = 0;
        let state = state.with_seeded_db_schema();
        let key = crate::repo::auth::create_auth_key(&state, "user").expect("capacity identity");
        let principal = crate::resolve_auth_identity_by_key(&state, &key)
            .expect("capacity identity lookup")
            .expect("capacity identity binding")
            .principal_id;
        {
            let db = state.core.db.get().expect("capacity fixture connection");
            super::indexing::ensure_retrieval_schema(&db).expect("retrieval startup");
            super::jobs::ensure_memory_job_schema(&db).expect("capacity job schema");
            super::embedding_jobs::initialize_embedding_runtime(&db, &state.policy.memory)
                .expect("capacity embedding schema");
            super::retention::ensure_principal_quota(
                &db,
                &state.policy.memory,
                &principal,
                crate::now_ts_u64() as i64,
            )
            .expect("valid quota prerequisite");
        }
        Self {
            state: Some(state),
            principal,
            key,
            root,
        }
    }

    pub(crate) fn state(&self) -> &crate::AppState {
        self.state.as_ref().unwrap()
    }

    pub(crate) fn state_mut(&mut self) -> &mut crate::AppState {
        self.state.as_mut().unwrap()
    }

    pub(super) fn competing_writer(&self) -> Connection {
        let db = Connection::open(self.root.as_ref().unwrap().join("fixture.sqlite"))
            .expect("independent writer");
        db.busy_timeout(Duration::from_millis(50)).unwrap();
        db.execute_batch("BEGIN IMMEDIATE")
            .expect("hold writer lock");
        db
    }

    pub(super) fn seed_embedding(&self) {
        let db = self.state().core.db.get().unwrap();
        persist_preference_sources(
            &db,
            1,
            0,
            &self.key,
            &[(
                "format".into(),
                "capacity fixture".into(),
                1.0,
                "user".into(),
            )],
            crate::now_ts_u64() as i64,
        );
        super::indexing::index_preference_entries(
            &db,
            1,
            0,
            &self.key,
            &[(
                "format".to_string(),
                "capacity fixture".to_string(),
                1.0,
                "user".to_string(),
            )],
            crate::now_ts_u64() as i64,
        )
        .expect("isolated preference outbox");
        assert_eq!(scalar(&db, "SELECT COUNT(*) FROM memory_embedding_jobs"), 1);
    }

    pub(super) fn seed_job(&self, id: &str, status: &str) {
        let db = self.state().core.db.get().unwrap();
        let now = crate::now_ts_u64() as i64;
        db.execute(
            "INSERT INTO memory_jobs(
                job_id, job_kind, principal_id, scope_kind, scope_ref, source_digest,
                eligibility_json, settings_revision, policy_digest, provider_name,
                provider_type, model_name, model_capability_digest, status,
                lease_owner, lease_expires_at_ts, attempt, not_before_ts,
                created_at_ts, updated_at_ts
             ) VALUES (?1, 'extract', ?2, 'principal', ?2, 'sha256:capacity', '{}',
                       1, 'sha256:capacity-policy', 'fixture', 'fixture', 'fixture',
                       'fixture', ?3, CASE WHEN ?3 = 'running' THEN 'capacity-worker' END,
                       CASE WHEN ?3 = 'running' THEN ?4 + 300 END,
                       CASE WHEN ?3 = 'running' THEN 1 ELSE 0 END, ?4 - 1, ?4 - 10, ?4)",
            params![id, self.principal, status, now],
        )
        .expect("isolated scheduler job");
    }
}

/// Explicitly persist canonical test sources before invoking production indexing.
/// This helper performs no migration or repair and is never called by runtime reads.
pub(super) fn persist_preference_sources(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    key: &str,
    entries: &[(String, String, f32, String)],
    ts: i64,
) {
    let owner = crate::repo::auth::principal_id_for_user_key(db, key)
        .expect("fixture source principal lookup")
        .expect("fixture sources require a real admitted owner");
    for (pref_key, content, confidence, source) in entries {
        db.execute(
            "INSERT INTO user_preferences(
                memory_id,user_id,chat_id,user_key,principal_id,scope_kind,scope_ref,
                pref_key,pref_value,confidence,source,updated_at,updated_at_ts,
                origin,row_revision,legacy_scope_inferred
             ) VALUES (?1,?2,?3,?4,?5,'principal',?5,?6,?7,?8,?9,?10,?11,
                       'user_confirmed',1,0)",
            params![
                format!("memory_{}", uuid::Uuid::new_v4().simple()),
                user_id,
                chat_id,
                key,
                owner,
                pref_key,
                content,
                confidence,
                source,
                ts.to_string(),
                ts
            ],
        )
        .expect("persist owned canonical preference fixture");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        drop(self.state.take());
        if let Some(root) = self.root.take() {
            std::fs::remove_dir_all(root).expect("remove owned isolated capacity directory");
        }
    }
}

pub(super) fn scalar(db: &Connection, sql: &str) -> i64 {
    db.query_row(sql, [], |row| row.get(0))
        .expect("fixture scalar")
}

pub(super) fn pressure(db: &Connection, state: &str) {
    assert_eq!(
        db.execute(
            "UPDATE memory_storage_pressure SET state = ?1, reason_code = 'capacity_fixture',
            revision = revision + 1, updated_at_ts = ?2 WHERE singleton_id = 1",
            params![state, crate::now_ts_u64() as i64],
        )
        .expect("fixture pressure"),
        1
    );
}

pub(super) fn job_status(db: &Connection, id: &str) -> (String, i64) {
    db.query_row(
        "SELECT status, attempt FROM memory_jobs WHERE job_id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .expect("fixture job status")
}
