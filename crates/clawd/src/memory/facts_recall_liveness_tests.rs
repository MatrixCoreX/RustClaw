use std::{cell::Cell, path::PathBuf, time::Duration};

use rusqlite::{params, types::Value, Connection};

const OWNER_KEY: &str = "expiry-recall-owner";
const OTHER_KEY: &str = "expiry-recall-other";

struct Fixture {
    state: crate::AppState,
    root: PathBuf,
    principal: String,
    other_principal: String,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "memory-expiry-recall-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let mut directory = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            directory.mode(0o700);
        }
        directory.create(&root).unwrap();
        println!("memory_expiry_recall_fixture_workspace={}", root.display());
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
        state.seed_test_auth_identity(OWNER_KEY, "user");
        state.seed_test_auth_identity(OTHER_KEY, "user");
        let db = state.core.db.get().unwrap();
        crate::memory::indexing::ensure_retrieval_schema(&db).unwrap();
        crate::memory::scope::ensure_memory_scope_schema(&db).unwrap();
        let principal = crate::repo::auth::principal_id_for_user_key(&db, OWNER_KEY)
            .unwrap()
            .unwrap();
        let other_principal = crate::repo::auth::principal_id_for_user_key(&db, OTHER_KEY)
            .unwrap()
            .unwrap();
        crate::memory::vector_store::register_configured_profile(&db, &state.policy.memory)
            .unwrap();
        crate::memory::project_identity::resolve_project_identity(&db, &root).unwrap();
        drop(db);
        Self {
            state,
            root,
            principal,
            other_principal,
        }
    }

    fn fact(&self, key: &str, fact_key: &str, text: &str) -> i64 {
        let fact = super::MemoryFactUpsert::from_long_term_summary(
            "recall_expiry",
            fact_key,
            text,
            text,
            1.0,
            "fixture-source",
            &[],
            "fixture evidence",
            None,
        );
        let db = self.state.core.db.get().unwrap();
        super::upsert_memory_fact_card(&db, 1, 0, key, &fact, crate::now_ts_u64() as i64)
            .unwrap()
            .unwrap()
    }

    fn expire_later(&self, id: i64) {
        self.state
            .core
            .db
            .get()
            .unwrap()
            .execute(
                "UPDATE memory_facts SET expires_at_ts = ?1 WHERE id = ?2",
                params![(crate::now_ts_u64() as i64).saturating_sub(1), id],
            )
            .unwrap();
    }

    fn recall(&self, query: &str) -> anyhow::Result<crate::memory::retrieval::IndexedRecall> {
        crate::memory::retrieval::retrieve_indexed_memories_for_scope(
            &self.state,
            Some(OWNER_KEY),
            1,
            0,
            None,
            query,
        )
    }

    fn fact_snapshot(&self, id: i64) -> Vec<Value> {
        self.state
            .core
            .db
            .get()
            .unwrap()
            .query_row("SELECT * FROM memory_facts WHERE id = ?1", [id], |row| {
                (0..row.as_ref().column_count())
                    .map(|index| row.get(index))
                    .collect()
            })
            .unwrap()
    }

    fn projection_count(&self, id: i64, owner: &str) -> i64 {
        self.state
            .core
            .db
            .get()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM memory_retrieval_index
             WHERE principal_id = ?1 AND source_kind = 'memory_fact' AND source_ref = ?2",
                params![
                    owner,
                    crate::memory::retrieval_source_ref_for_memory_fact(id)
                ],
                |row| row.get(0),
            )
            .unwrap()
    }
}

// Preserve private fixture databases on success and failure; no cleanup Drop.

#[test]
fn ordinary_scoped_recall_does_not_expire_another_principals_fact() {
    let fixture = Fixture::new();
    fixture.fact(OWNER_KEY, "caller", "retained canonical caller sentinel");
    let foreign = fixture.fact(OTHER_KEY, "foreign", "retained canonical foreign sentinel");
    fixture
        .recall("retained canonical caller sentinel")
        .unwrap();
    fixture.expire_later(foreign);
    let before = fixture.fact_snapshot(foreign);
    assert_eq!(
        fixture.projection_count(foreign, &fixture.other_principal),
        1
    );

    fixture
        .recall("retained canonical caller sentinel")
        .unwrap();

    assert_eq!(
        fixture.fact_snapshot(foreign),
        before,
        "ordinary scoped recall must not perform global foreign-fact expiry writes"
    );
    assert_eq!(
        fixture.projection_count(foreign, &fixture.other_principal),
        1
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
fn warmed_scoped_recall_with_foreign_expiry_does_not_attempt_a_wal_writer() {
    let fixture = Fixture::new();
    fixture.fact(OWNER_KEY, "caller", "retained canonical caller sentinel");
    let foreign = fixture.fact(OTHER_KEY, "foreign", "retained canonical foreign sentinel");
    fixture
        .recall("retained canonical caller sentinel")
        .unwrap();
    fixture.expire_later(foreign);
    let before = fixture.fact_snapshot(foreign);
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
            .execute(
                "UPDATE memory_facts SET updated_at_ts = updated_at_ts WHERE 0",
                [],
            )
            .unwrap_err();
        assert_eq!(
            error.sqlite_error_code(),
            Some(rusqlite::ErrorCode::DatabaseBusy)
        );
    }
    assert!(
        BUSY_CALLBACKS.with(Cell::get) > 0,
        "real WAL writer negative control"
    );
    BUSY_CALLBACKS.with(|count| count.set(0));

    let recall = fixture.recall("retained canonical caller sentinel");
    let busy = BUSY_CALLBACKS.with(Cell::get);
    assert!(!writer.is_autocommit());
    writer.execute_batch("ROLLBACK").unwrap();

    assert!(
        recall.is_ok(),
        "foreign expired facts must not make an ordinary warmed recall require a writer: {recall:?}"
    );
    assert_eq!(
        busy, 0,
        "ordinary recall must not attempt expiry maintenance writes"
    );
    assert_eq!(fixture.fact_snapshot(foreign), before);
}

#[test]
fn own_expired_fact_is_hidden_without_read_time_canonical_mutation() {
    let fixture = Fixture::new();
    let marker = "retained canonical expiring sentinel";
    let expiring = fixture.fact(OWNER_KEY, "expiring", marker);
    let warm = fixture.recall(marker).unwrap();
    assert!(
        warm.relevant_facts
            .iter()
            .any(|item| item.text.contains(marker)),
        "the real projection must be recallable before expiry"
    );
    fixture.expire_later(expiring);
    let before = fixture.fact_snapshot(expiring);

    let recall = fixture.recall(marker).unwrap();

    assert!(
        recall
            .relevant_facts
            .iter()
            .all(|item| !item.text.contains(marker)),
        "expired canonical facts must not remain in prompt recall"
    );
    assert_eq!(
        fixture.fact_snapshot(expiring),
        before,
        "expiry exclusion must be read-only until explicit maintenance"
    );
    assert_eq!(fixture.projection_count(expiring, &fixture.principal), 1);
}
