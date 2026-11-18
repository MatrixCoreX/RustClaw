use rusqlite::Connection;

use super::fixtures::*;

fn failure_db() -> Connection {
    let db = initialized_db(false);
    // A table-shaped FTS stand-in permits failure injection; real FTS5 success
    // and isolation are exercised independently in the behavior cohort.
    db.execute_batch("CREATE TABLE memory_retrieval_index_fts(rowid INTEGER PRIMARY KEY,search_text TEXT,topic_tags TEXT); CREATE TABLE caller_work(value INTEGER)").unwrap();
    for id in 1..=2 {
        projection(&db, id, A, "memory_fact", Some("shared"));
        job(
            &db,
            &format!("job-{id}"),
            id,
            A,
            ("queued", 0),
            ("profile", 1, "digest"),
        );
        vector(&db, id, A, "profile", 1, "active");
    }
    db
}

#[test]
fn failures_at_every_retirement_stage_restore_all_rows_and_caller_can_commit() {
    for mode in ["ABORT", "FAIL"] {
        for (event, table, last_row) in [
            ("UPDATE", "memory_embedding_jobs", "OLD.job_id='job-2'"),
            ("UPDATE", "memory_vector_rows", "OLD.retrieval_id=2"),
            ("DELETE", "memory_retrieval_index_fts", "OLD.rowid=2"),
            ("DELETE", "memory_retrieval_index", "OLD.id=2"),
        ] {
            let db = failure_db();
            db.execute_batch(&format!("CREATE TRIGGER injected_failure BEFORE {event} ON {table} WHEN {last_row} BEGIN SELECT RAISE({mode},'fixture_retirement_failure'); END")).unwrap();
            let before = snapshot(&db);
            let tx = db.unchecked_transaction().unwrap();
            tx.execute("INSERT INTO caller_work VALUES (7)", [])
                .unwrap();
            let error = retire(&tx, A, "shared").unwrap_err();
            assert!(
                error.to_string().contains("fixture_retirement_failure"),
                "{event} {table}: {error}"
            );
            assert_eq!(snapshot(&tx), before, "{mode} {event} {table}");
            assert!(!tx.is_autocommit());
            tx.commit().unwrap();
            assert_eq!(snapshot(&db), before);
            assert_eq!(
                db.query_row("SELECT value FROM caller_work", [], |row| row
                    .get::<_, i64>(0))
                    .unwrap(),
                7
            );
        }
    }
}

#[test]
fn malformed_present_tables_fail_even_for_empty_targets_without_repair() {
    for table in [
        "memory_embedding_jobs",
        "memory_vector_rows",
        "memory_retrieval_index_fts",
    ] {
        for reference in ["shared", "missing"] {
            let db = failure_db();
            db.execute_batch(&format!("DROP TABLE {table}; CREATE TABLE {table}(broken TEXT PRIMARY KEY) WITHOUT ROWID; INSERT INTO {table} VALUES ('unchanged')")).unwrap();
            let before = snapshot(&db);
            let schema: i64 = db
                .query_row("PRAGMA schema_version", [], |row| row.get(0))
                .unwrap();
            let tx = db.unchecked_transaction().unwrap();
            assert!(retire(&tx, A, reference).is_err(), "{table} {reference}");
            assert_eq!(snapshot(&tx), before);
            tx.commit().unwrap();
            assert_eq!(snapshot(&db), before);
            assert_eq!(
                db.query_row("PRAGMA schema_version", [], |row| row.get::<_, i64>(0))
                    .unwrap(),
                schema
            );
        }
    }
}

#[test]
fn readonly_failure_does_not_cancel_or_tombstone_work() {
    let db = failure_db();
    let before = snapshot(&db);
    db.execute_batch("PRAGMA query_only=ON").unwrap();
    let tx = db.unchecked_transaction().unwrap();
    assert!(retire(&tx, A, "shared").is_err());
    assert_eq!(snapshot(&tx), before);
    tx.rollback().unwrap();
    assert_eq!(snapshot(&db), before);
}
