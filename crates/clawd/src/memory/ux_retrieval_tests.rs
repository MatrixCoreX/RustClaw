use super::*;

fn fixture(with_fts: bool) -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE memory_facts(id INTEGER PRIMARY KEY,principal_id TEXT);
         CREATE TABLE memories(id INTEGER PRIMARY KEY,principal_id TEXT);
         CREATE TABLE memory_retrieval_index(
            id INTEGER PRIMARY KEY,principal_id TEXT,source_kind TEXT,source_ref TEXT,
            source_memory_id INTEGER,source_pref_key TEXT,search_text TEXT);
         INSERT INTO memory_retrieval_index VALUES
            (1,'owner-a','memory_fact','memory_fact:1',NULL,NULL,'owned'),
            (2,'owner-b','memory_fact','memory_fact:1',NULL,NULL,'other owner'),
            (3,'owner-a','preference','memory_fact:1',NULL,'memory_fact:1','other kind'),
            (4,'owner-a','memory_fact','memory_fact:2',NULL,NULL,'other reference');",
    )
    .unwrap();
    if with_fts {
        db.execute_batch(
            "CREATE VIRTUAL TABLE memory_retrieval_index_fts USING fts5(search_text,topic_tags);
             INSERT INTO memory_retrieval_index_fts(rowid,search_text,topic_tags)
                SELECT id,search_text,'' FROM memory_retrieval_index;",
        )
        .unwrap();
    }
    db
}

fn identity() -> RetrievalIdentity {
    RetrievalIdentity {
        source_kind: RETRIEVAL_SOURCE_MEMORY_FACT,
        source_ref: retrieval_source_ref_for_memory_fact(1),
        source_memory_id: None,
        source_pref_key: None,
    }
}

fn ids(db: &Connection, table: &str) -> Vec<i64> {
    let mut statement = db
        .prepare(&format!("SELECT rowid FROM {table} ORDER BY rowid"))
        .unwrap();
    let rows = statement.query_map([], |row| row.get::<_, i64>(0)).unwrap();
    rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
}

#[test]
fn without_fts_mutation_removes_only_exact_owned_identity() {
    let db = fixture(false);
    let tx = db.unchecked_transaction().unwrap();
    remove_identity(&tx, "owner-a", &identity()).unwrap();
    tx.commit().unwrap();
    assert_eq!(ids(&db, "memory_retrieval_index"), [2, 3, 4]);
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='memory_retrieval_index_fts'",
            [],
            |row| { row.get::<_, i64>(0) }
        )
        .unwrap(),
        0,
        "mutation does not initialize an absent optional FTS table"
    );
}

#[test]
fn fts_targets_preserve_other_owners_kinds_and_references() {
    let db = fixture(true);
    let tx = db.unchecked_transaction().unwrap();
    remove_identity(&tx, "owner-a", &identity()).unwrap();
    tx.commit().unwrap();
    assert_eq!(ids(&db, "memory_retrieval_index"), [2, 3, 4]);
    assert_eq!(ids(&db, "memory_retrieval_index_fts"), [2, 3, 4]);
}

#[test]
fn failed_index_delete_rolls_back_fts_and_core_rows() {
    let db = fixture(true);
    db.execute_batch(
        "CREATE TRIGGER fail_owned_index_delete BEFORE DELETE ON memory_retrieval_index
         BEGIN SELECT RAISE(ABORT,'index_fixture_delete_failed'); END;",
    )
    .unwrap();
    {
        let tx = db.unchecked_transaction().unwrap();
        let error = remove_identity(&tx, "owner-a", &identity()).unwrap_err();
        assert!(error.to_string().contains("index_fixture_delete_failed"));
    }
    assert!(db.is_autocommit());
    assert_eq!(ids(&db, "memory_retrieval_index"), [1, 2, 3, 4]);
    assert_eq!(ids(&db, "memory_retrieval_index_fts"), [1, 2, 3, 4]);
}

#[test]
fn autocommit_mutation_is_rejected_before_any_write_or_initialization() {
    let db = fixture(false);
    let before = db.total_changes();
    let error = remove_identity(&db, "owner-a", &identity()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "memory_retrieval_mutation_requires_transaction"
    );
    assert_eq!(db.total_changes(), before);
    assert_eq!(ids(&db, "memory_retrieval_index"), [1, 2, 3, 4]);
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='memory_retrieval_index_fts'",
            [],
            |row| { row.get::<_, i64>(0) }
        )
        .unwrap(),
        0
    );
}

#[test]
fn orphan_fact_checks_use_primary_key_lookup_without_scanning_all_facts() {
    let db = fixture(false);
    let mut statement = db
        .prepare(&format!("EXPLAIN QUERY PLAN {ORPHAN_TARGETS}"))
        .unwrap();
    let rows = statement
        .query_map(params!["owner-a", MEMORY_FACT_SOURCE_REF_PREFIX], |row| {
            row.get::<_, String>(3)
        })
        .unwrap();
    let plan = rows.collect::<rusqlite::Result<Vec<_>>>().unwrap();
    assert!(
        plan.iter()
            .any(|detail| detail.contains("SEARCH f USING INTEGER PRIMARY KEY")),
        "{plan:?}"
    );
    assert!(
        !plan.iter().any(|detail| detail.contains("SCAN f")),
        "{plan:?}"
    );
}
