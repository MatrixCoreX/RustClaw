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
fn canonical_fact_expiry_filter_avoids_owner_population_cross_product() {
    use rusqlite::StatementStatus;

    let fixture = Fixture::new();
    let db = fixture.state.core.db.get().unwrap();
    let measure = |expected: i64| {
        let before = db.total_changes();
        let predicate = super::current_fact_projection_sql(&db).unwrap();
        let mut statement = db
            .prepare(&format!(
                "SELECT COUNT(*) FROM memory_retrieval_index i
                 WHERE i.principal_id = ?1 AND i.source_kind = 'memory_fact'
                   AND {predicate}"
            ))
            .unwrap();
        let selected: i64 = statement
            .query_row([&fixture.principal], |row| row.get(0))
            .unwrap();
        assert_eq!(
            selected, expected,
            "every current canonical fact is retained"
        );
        assert_eq!(db.total_changes(), before, "recall filtering is read-only");
        let steps = statement.get_status(StatementStatus::VmStep);
        assert!(steps > 0, "measure actual bundled SQLite work");
        steps as u64
    };
    for index in 0..128 {
        fixture.fact(OWNER_KEY, &format!("scaled-{index}"), "current scaled fact");
    }
    let small = measure(128);
    for index in 128..256 {
        fixture.fact(OWNER_KEY, &format!("scaled-{index}"), "current scaled fact");
    }
    let large = measure(256);
    println!(
        "FACT_RECALL_WORK_JSON {}",
        serde_json::json!({"small_rows":128,"large_rows":256,
            "small_vm_steps":small,"large_vm_steps":large})
    );
    assert!(
        large <= small.saturating_mul(3),
        "doubling current facts must not rescan the owner's entire fact population per projection: small={small} large={large}"
    );
}

#[test]
fn indexed_fact_filter_preserves_exact_reference_owner_and_expiry_semantics() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE memory_facts(id INTEGER PRIMARY KEY, memory_id TEXT UNIQUE,
            principal_id TEXT, user_key TEXT, status TEXT, expires_at_ts INTEGER);
         CREATE TABLE memory_retrieval_index(id INTEGER PRIMARY KEY, source_kind TEXT,
            source_ref TEXT, principal_id TEXT, user_key TEXT);",
    )
    .unwrap();
    let facts = [
        (
            0_i64,
            Some("memory_fact:2"),
            Some("owner"),
            "owner",
            Some("expired"),
            None,
        ),
        (
            1,
            Some("opaque-1"),
            Some("owner"),
            "owner",
            Some("active"),
            Some(0),
        ),
        (
            2,
            Some("0"),
            Some("foreign"),
            "foreign",
            Some("expired"),
            None,
        ),
        (-1, Some("mem_abc"), None, "owner", Some("active"), Some(0)),
        (
            i64::MAX,
            None,
            Some("owner"),
            "owner",
            Some("active"),
            Some(i64::MAX),
        ),
        (
            i64::MIN,
            Some("memory_fact:-1"),
            Some("owner"),
            "owner",
            None,
            None,
        ),
    ];
    for (id, memory_id, owner, key, status, expiry) in facts {
        db.execute(
            "INSERT INTO memory_facts VALUES(?1,?2,?3,?4,?5,?6)",
            params![id, memory_id, owner, key, status, expiry],
        )
        .unwrap();
    }
    let references = [
        Some("0"),
        Some("-1"),
        Some("1"),
        Some("2"),
        Some("memory_fact:0"),
        Some("memory_fact:-1"),
        Some("memory_fact:1"),
        Some("01"),
        Some("memory_fact:01"),
        Some("memory_fact:memory_fact:1"),
        Some("opaque-1"),
        Some("mem_abc"),
        Some("9223372036854775807"),
        Some("9223372036854775808"),
        Some("-9223372036854775808"),
        Some("memory_fact:9223372036854775807"),
        None,
        Some(""),
        Some("+1"),
        Some("1.0"),
        Some(" memory_fact:1"),
        Some("memory_fact:1x"),
    ];
    for kind in ["memory_fact", "other"] {
        for reference in references {
            for owner in [Some("owner"), Some("foreign"), None] {
                for key in [Some("owner"), Some("foreign"), None] {
                    db.execute(
                        "INSERT INTO memory_retrieval_index(source_kind,source_ref,principal_id,user_key)
                         VALUES(?1,?2,?3,?4)",
                        params![kind, reference, owner, key],
                    )
                    .unwrap();
                }
            }
        }
    }
    // Frozen pre-optimization predicate is an independent effects oracle.
    let original = "NOT EXISTS(SELECT 1 FROM memory_facts f
        WHERE i.source_kind='memory_fact'
          AND (f.principal_id=i.principal_id OR
               (f.principal_id IS NULL AND i.principal_id IS NULL AND f.user_key=i.user_key))
          AND (i.source_ref=CAST(f.id AS TEXT) OR i.source_ref=f.memory_id
               OR i.source_ref='memory_fact:'||CAST(f.id AS TEXT))
          AND (f.status<>'active' OR (f.expires_at_ts IS NOT NULL
               AND f.expires_at_ts<=CAST(strftime('%s','now') AS INTEGER))))";
    let selected = |predicate: &str| {
        db.prepare(&format!(
            "SELECT i.id FROM memory_retrieval_index i WHERE {predicate} ORDER BY i.id"
        ))
        .unwrap()
        .query_map([], |row| row.get::<_, i64>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
    };
    let before = db.total_changes();
    assert_eq!(
        selected(original),
        selected(&super::current_fact_projection_sql(&db).unwrap())
    );
    assert_eq!(
        db.total_changes(),
        before,
        "no read-time repair or mutation"
    );
    let rows: i64 = db
        .query_row("SELECT COUNT(*) FROM memory_retrieval_index", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(rows, 396);
    let version: String = db
        .query_row("SELECT sqlite_version()", [], |row| row.get(0))
        .unwrap();
    println!(
        "FACT_REFERENCE_SQLITE_JSON {}",
        serde_json::json!({"version":version,"projection_rows":rows,"same_effects_as_original":true})
    );
}

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

fn seed_fact_vector(
    fixture: &Fixture,
    fact_id: i64,
    position: usize,
) -> (
    i64,
    crate::memory::vector_store::MemoryEmbeddingProfile,
    Vec<f32>,
) {
    use crate::memory::vector_store::{ExactSqliteVectorIndex, MemoryVectorIndex, VectorWrite};
    let db = fixture.state.core.db.get().unwrap();
    let profile =
        crate::memory::vector_store::register_configured_profile(&db, &fixture.state.policy.memory)
            .unwrap();
    let id = db
        .query_row(
            "SELECT id FROM memory_retrieval_index WHERE principal_id=?1
         AND source_kind='memory_fact' AND source_ref=?2",
            params![
                fixture.principal,
                crate::memory::retrieval_source_ref_for_memory_fact(fact_id)
            ],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    let mut vector = vec![0.0; profile.dimensions];
    vector[position] = 1.0;
    ExactSqliteVectorIndex
        .upsert(
            &db,
            &profile,
            &VectorWrite {
                retrieval_id: id,
                principal_id: &fixture.principal,
                scope_kind: "principal",
                scope_ref: &fixture.principal,
                projection_digest: "sha256:expiry-liveness",
                vector: &vector,
            },
        )
        .unwrap();
    (id, profile, vector)
}

#[test]
fn background_maintenance_expires_due_fact_and_retires_its_vector() {
    let fixture = Fixture::new();
    let fact = fixture.fact(
        OWNER_KEY,
        "expiring",
        "retained canonical expiring sentinel",
    );
    let (projection, _, _) = seed_fact_vector(&fixture, fact, 0);
    fixture.expire_later(fact);
    let db = fixture.state.core.db.get().unwrap();

    crate::memory::retention::cleanup_memory_data(
        &db,
        &fixture.state.policy.memory,
        crate::now_ts_u64() as i64,
    )
    .unwrap();

    assert_eq!(
        db.query_row(
            "SELECT status FROM memory_facts WHERE id=?1",
            [fact],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "expired"
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index WHERE id=?1",
            [projection],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT status FROM memory_vector_rows WHERE retrieval_id=?1",
            [projection],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "tombstone"
    );
}

#[test]
fn failed_expiry_maintenance_rolls_back_fact_projection_and_source_deletions() {
    let fixture = Fixture::new();
    let fact = fixture.fact(
        OWNER_KEY,
        "expiring",
        "retained canonical expiring sentinel",
    );
    fixture.expire_later(fact);
    let before = fixture.fact_snapshot(fact);
    {
        let db = fixture.state.core.db.get().unwrap();
        db.execute(
            "INSERT INTO memories(memory_id,user_id,chat_id,user_key,principal_id,scope_kind,
             scope_ref,channel,role,content,created_at,created_at_ts)
             VALUES ('expiry-rollback-source',1,0,?1,?2,'principal',?2,
                     'ui','user','retention rollback witness','0',0)",
            params![OWNER_KEY, fixture.principal],
        )
        .unwrap();
        db.execute_batch(
            "CREATE TRIGGER reject_expiry_projection_delete BEFORE DELETE ON memory_retrieval_index
             WHEN OLD.source_kind='memory_fact'
             BEGIN SELECT RAISE(ABORT,'fixture_expiry_projection_delete_rejected'); END;",
        )
        .unwrap();
        let error = crate::memory::retention::cleanup_memory_data(
            &db,
            &fixture.state.policy.memory,
            crate::now_ts_u64() as i64,
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("fixture_expiry_projection_delete_rejected"));
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM memories WHERE memory_id='expiry-rollback-source'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1,
            "earlier retention writes must roll back with failed expiry"
        );
    }
    assert_eq!(fixture.fact_snapshot(fact), before);
    assert_eq!(fixture.projection_count(fact, &fixture.principal), 1);
}

#[test]
fn nearest_excludes_expired_canonical_fact_before_top_k_without_read_repair() {
    use crate::memory::vector_store::{ExactSqliteVectorIndex, MemoryVectorIndex};
    let fixture = Fixture::new();
    let valid = fixture.fact(OWNER_KEY, "valid", "retained canonical valid sentinel");
    let expired = fixture.fact(
        OWNER_KEY,
        "expiring",
        "retained canonical expiring sentinel",
    );
    let (live_projection, _, _) = seed_fact_vector(&fixture, valid, 1);
    let (_, profile, query) = seed_fact_vector(&fixture, expired, 0);
    fixture.expire_later(expired);
    let before = fixture.fact_snapshot(expired);
    {
        let db = fixture.state.core.db.get().unwrap();
        let changes = db.total_changes();
        let access =
            crate::memory::scope::resolve_memory_access(&db, &fixture.principal, None, None)
                .unwrap();
        let neighbors = ExactSqliteVectorIndex
            .nearest(&db, &access, &profile, &query, 1)
            .unwrap();
        assert_eq!(neighbors.len(), 1);
        assert_eq!(
            neighbors[0].retrieval_id, live_projection,
            "an expired higher-scoring fact must not consume the top-k budget"
        );
        assert_eq!(db.total_changes(), changes);
    }
    assert_eq!(fixture.fact_snapshot(expired), before);
    assert_eq!(fixture.projection_count(expired, &fixture.principal), 1);
}

#[test]
fn legacy_fact_expiry_filter_preserves_the_unscoped_schema_without_read_repair() {
    let db = Connection::open_in_memory().unwrap();
    super::ensure_memory_fact_schema(&db).unwrap();
    db.execute_batch(
        "CREATE TABLE memory_retrieval_index(
            id INTEGER PRIMARY KEY, source_kind TEXT, source_ref TEXT, user_key TEXT
         );
         INSERT INTO memory_facts(id,user_id,chat_id,user_key,fact_text,status,expires_at_ts)
         VALUES (1,1,0,'legacy-owner','legacy fact','active',1);
         INSERT INTO memory_retrieval_index VALUES(1,'memory_fact','memory_fact:1','legacy-owner');"
    ).unwrap();
    let before = db.total_changes();
    let schema: Vec<(String, String)> = db
        .prepare("SELECT name,sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY name")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    let predicate = super::current_fact_projection_sql(&db).unwrap();
    assert_eq!(
        db.query_row(
            &format!("SELECT COUNT(*) FROM memory_retrieval_index i WHERE {predicate}"),
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(db.total_changes(), before);
    let after: Vec<(String, String)> = db
        .prepare("SELECT name,sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY name")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        after, schema,
        "legacy reads must not add ownership columns or migrate schema"
    );
}
