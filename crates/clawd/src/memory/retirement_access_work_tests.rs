use rusqlite::{
    params, types::Value, Connection, StatementStatus, Transaction, TransactionBehavior,
};

use super::{ensure_retirement_access_schema, fixtures::*};

fn update_sql(table: &str, legacy: bool) -> String {
    let owned = if legacy {
        // Frozen pre-change predicate. It remains an independent effects oracle,
        // not a requirement that future SQLite versions keep the old plan slow.
        format!(
            "EXISTS(SELECT 1 FROM memory_retrieval_index i
             WHERE i.id={table}.retrieval_id AND i.id IN ({TARGETS})
               AND i.principal_id={table}.principal_id
               AND i.scope_kind={table}.scope_kind AND i.scope_ref={table}.scope_ref)"
        )
    } else {
        super::super::projection_retirement::owned_targets(table, TARGETS)
    };
    match table {
        "memory_embedding_jobs" => format!(
            "UPDATE memory_embedding_jobs SET cancel_requested=1,updated_at_ts=?6
             WHERE cancel_requested=0 AND status IN ('queued','retry_wait','running') AND {owned}"
        ),
        "memory_vector_rows" => format!(
            "UPDATE memory_vector_rows SET status='tombstone',updated_at_ts=?6
             WHERE status='active' AND {owned}"
        ),
        _ => panic!("unexpected fixture table"),
    }
}

fn measure(
    db: &Connection,
    table: &str,
    legacy: bool,
    owner: (&str, &str, &str),
    reference: &str,
) -> (i32, usize, Vec<Vec<Value>>, Vec<String>) {
    let tx = db.unchecked_transaction().unwrap();
    let sql = update_sql(table, legacy);
    let values = params![owner.0, "memory_fact", reference, owner.1, owner.2, -1];
    let plan = tx
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap()
        .query_map(values, |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let (steps, affected) = {
        let mut statement = tx.prepare(&sql).unwrap();
        let affected = statement.execute(values).unwrap();
        (statement.get_status(StatementStatus::VmStep), affected)
    };
    let changed = {
        let mut statement = tx
            .prepare(&format!(
                "SELECT * FROM {table} WHERE updated_at_ts=-1 ORDER BY retrieval_id"
            ))
            .unwrap();
        let width = statement.column_count();
        let rows = statement
            .query_map([], |row| {
                (0..width)
                    .map(|column| row.get::<_, Value>(column))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        rows
    };
    tx.rollback().unwrap();
    (steps, affected, changed, plan)
}

fn populated_vectors(db: &Connection) {
    db.execute_batch(
        "INSERT INTO memory_vector_rows(retrieval_id,principal_id,scope_kind,scope_ref,
            profile_id,generation,projection_version,projection_digest,vector_format,dimensions,
            normalization,vector_blob,vector_checksum,status,created_at_ts,updated_at_ts)
         SELECT id,principal_id,scope_kind,scope_ref,'profile-a',1,'fixture-v1','fixture-digest',
            'f32le_v1',1,'none',X'00000000','fixture-checksum','active',0,0 FROM memory_retrieval_index;",
    ).unwrap();
}

fn target_bounded_work(table: &str) {
    let mut small_work = Vec::new();
    for count in [128, 4096, 20002] {
        let db = initialized_db(false);
        seed_populated(&db, count);
        if table == "memory_vector_rows" {
            populated_vectors(&db);
        }
        let last = format!("ref:{count}");
        for (index, (owner, reference, expected)) in [
            (A, "missing", 0),
            (A, "ref:1", 1),
            (A, last.as_str(), 1),
            (B, "ref:1", 0),
            (ROOM, "ref:1", 0),
        ]
        .into_iter()
        .enumerate()
        {
            let old = measure(&db, table, true, owner, reference);
            let new = measure(&db, table, false, owner, reference);
            assert_eq!(
                (&new.1, &new.2),
                (&old.1, &old.2),
                "{table} {count} {owner:?} {reference}"
            );
            assert_eq!(new.1, expected);
            assert!(new.0 > 0);
            if count == 128 {
                small_work.push(new.0);
            } else {
                assert!(new.0 <= small_work[index].max(64) * 4,
                    "retirement work grew with unrelated rows: {table} count={count} small={} large={} plan={:?}", small_work[index], new.0, new.3);
            }
            // Do not force a particular index name or an old-plan slowdown;
            // bounded real VM work and identical effects are the contract.
            assert!(
                new.3.iter().any(|detail| detail.contains("SEARCH")
                    && detail.contains(table)
                    && detail.contains("retrieval_id=?")),
                "{:?}",
                new.3
            );
            eprintln!("retirement_vm table={table} rows={count} case={index} legacy={} current={} affected={} plan={:?}", old.0, new.0, new.1, new.3);
        }
        assert_eq!(db.query_row("SELECT COUNT(*) FROM memory_embedding_jobs WHERE cancel_requested<>0 OR updated_at_ts<>0", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    }
}

#[test]
fn populated_job_retirement_is_target_bounded_and_matches_legacy_effects() {
    target_bounded_work("memory_embedding_jobs");
}

#[test]
fn populated_vector_retirement_is_target_bounded_and_matches_legacy_effects() {
    target_bounded_work("memory_vector_rows");
}

#[test]
fn populated_upgrade_reports_storage_and_writer_hold_without_rewriting_rows() {
    let db = legacy_db();
    seed_populated(&db, 20002);
    populated_vectors(&db);
    let before = snapshot(&db);
    let changes = db.total_changes();
    let allocated = |db: &Connection| {
        let pages: i64 = db
            .query_row("PRAGMA page_count", [], |row| row.get(0))
            .unwrap();
        let bytes: i64 = db
            .query_row("PRAGMA page_size", [], |row| row.get(0))
            .unwrap();
        pages * bytes
    };
    let before_bytes = allocated(&db);
    let tx = Transaction::new_unchecked(&db, TransactionBehavior::Immediate).unwrap();
    let writer_started = std::time::Instant::now();
    ensure_retirement_access_schema(&tx).unwrap();
    tx.commit().unwrap();
    let writer_hold = writer_started.elapsed();
    let after_bytes = allocated(&db);
    assert_eq!(snapshot(&db), before);
    assert_eq!(
        db.total_changes() - changes,
        1,
        "only the migration receipt is inserted"
    );
    assert!(after_bytes >= before_bytes);
    eprintln!("retirement_index_upgrade rows=20002 whole_db_allocated_before={before_bytes} whole_db_allocated_after={after_bytes} whole_db_allocation_delta_including_receipt={} immediate_writer_hold_us={}", after_bytes-before_bytes, writer_hold.as_micros());
    // A repeated startup may read schema/receipt metadata, but never needs a
    // writer or a fallback reindex merely because these access paths exist.
    db.execute_batch("PRAGMA query_only=ON").unwrap();
    ensure_retirement_access_schema(&db).unwrap();
    assert_eq!(db.total_changes() - changes, 1);
}
