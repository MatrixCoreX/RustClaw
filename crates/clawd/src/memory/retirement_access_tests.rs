use rusqlite::{params, Connection};

use super::fixtures::*;

fn changed_jobs(db: &Connection) -> Vec<String> {
    db.prepare("SELECT job_id FROM memory_embedding_jobs WHERE updated_at_ts<>0 ORDER BY job_id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn changed_vectors(db: &Connection) -> Vec<(i64, String, i64)> {
    db.prepare("SELECT retrieval_id,profile_id,generation FROM memory_vector_rows WHERE updated_at_ts<>0 ORDER BY retrieval_id,profile_id,generation")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

#[test]
fn exact_identity_retirement_preserves_foreign_derived_rows_and_fts() {
    let db = initialized_db(true);
    for (id, owner, kind, reference) in [
        (1, A, "memory_fact", Some("shared")),
        (2, A, "memory_fact", Some("shared")),
        (3, A, "memory_fact", None),
        (4, A, "user_preference", Some("shared")),
        (5, A, "memory_fact", Some("other")),
        (6, B, "memory_fact", Some("shared")),
        (7, ROOM, "memory_fact", Some("shared")),
        (8, PROJECT, "memory_fact", Some("shared")),
    ] {
        projection(&db, id, owner, kind, reference);
        job(
            &db,
            &format!("job-{id}"),
            id,
            owner,
            ("queued", 0),
            ("profile", 1, "digest"),
        );
        vector(&db, id, owner, "profile", 1, "active");
    }
    for (profile, owner) in [("foreign-owner", B), ("foreign-scope", ROOM)] {
        job(
            &db,
            profile,
            1,
            owner,
            ("running", 0),
            (profile, 1, "digest"),
        );
        vector(&db, 1, owner, profile, 1, "active");
    }
    let tx = db.unchecked_transaction().unwrap();
    assert_eq!(retire(&tx, A, "shared").unwrap(), 2);
    tx.commit().unwrap();
    assert_eq!(ids(&db, "memory_retrieval_index"), vec![3, 4, 5, 6, 7, 8]);
    assert_eq!(
        ids(&db, "memory_retrieval_index_fts"),
        vec![3, 4, 5, 6, 7, 8]
    );
    assert_eq!(changed_jobs(&db), vec!["job-1", "job-2"]);
    assert_eq!(
        changed_vectors(&db),
        vec![(1, "profile".into(), 1), (2, "profile".into(), 1)]
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memory_embedding_jobs WHERE cancel_requested=1",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memory_vector_rows WHERE status='tombstone'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}

#[test]
fn all_scopes_and_principals_retire_only_their_selected_identity() {
    for selected in [A, B, ROOM, PROJECT] {
        let db = initialized_db(true);
        for (index, owner) in [A, B, ROOM, PROJECT].into_iter().enumerate() {
            let id = index as i64 + 1;
            projection(&db, id, owner, "memory_fact", Some("shared"));
            job(
                &db,
                &format!("job-{id}"),
                id,
                owner,
                ("queued", 0),
                ("profile", 1, "digest"),
            );
            vector(&db, id, owner, "profile", 1, "active");
        }
        let tx = db.unchecked_transaction().unwrap();
        assert_eq!(retire(&tx, selected, "shared").unwrap(), 1);
        tx.commit().unwrap();
        let removed = [A, B, ROOM, PROJECT]
            .iter()
            .position(|owner| *owner == selected)
            .unwrap() as i64
            + 1;
        let remaining = (1..=4).filter(|id| *id != removed).collect::<Vec<_>>();
        assert_eq!(ids(&db, "memory_retrieval_index"), remaining);
        assert_eq!(ids(&db, "memory_retrieval_index_fts"), remaining);
        assert_eq!(changed_jobs(&db), vec![format!("job-{removed}")]);
        assert_eq!(changed_vectors(&db), vec![(removed, "profile".into(), 1)]);
    }
}

#[test]
fn eligibility_and_rotated_profile_generations_preserve_terminal_work() {
    let db = initialized_db(false);
    projection(&db, 1, A, "memory_fact", Some("shared"));
    for (index, status) in [
        "queued",
        "running",
        "retry_wait",
        "completed",
        "cancelled",
        "failed",
    ]
    .into_iter()
    .enumerate()
    {
        for cancelled in [0, 1] {
            let generation = index as i64 * 2 + cancelled + 1;
            job(
                &db,
                &format!("{status}-{cancelled}"),
                1,
                A,
                (status, cancelled),
                ("profile", generation, "digest"),
            );
        }
    }
    job(
        &db,
        "rotated",
        1,
        A,
        ("retry_wait", 0),
        ("rotated-profile", 9, "rotated-digest"),
    );
    vector(&db, 1, A, "profile", 1, "active");
    vector(&db, 1, A, "profile", 2, "active");
    vector(&db, 1, A, "rotated-profile", 9, "active");
    vector(&db, 1, A, "old-tombstone", 1, "tombstone");
    let tx = db.unchecked_transaction().unwrap();
    assert_eq!(retire(&tx, A, "shared").unwrap(), 1);
    tx.commit().unwrap();
    for status in [
        "queued",
        "running",
        "retry_wait",
        "completed",
        "cancelled",
        "failed",
    ] {
        for cancelled in [0, 1] {
            let eligible = cancelled == 0 && ["queued", "running", "retry_wait"].contains(&status);
            let actual: (String, i64, i64) = db.query_row("SELECT status,cancel_requested,updated_at_ts FROM memory_embedding_jobs WHERE job_id=?1", [format!("{status}-{cancelled}")], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
            assert_eq!(actual.0, status);
            assert_eq!(actual.1, if eligible { 1 } else { cancelled });
            assert_eq!(actual.2 > 0, eligible);
        }
    }
    assert_eq!(
        changed_jobs(&db),
        vec!["queued-0", "retry_wait-0", "rotated", "running-0"]
    );
    assert_eq!(
        changed_vectors(&db),
        vec![
            (1, "profile".into(), 1),
            (1, "profile".into(), 2),
            (1, "rotated-profile".into(), 9)
        ]
    );
    assert_eq!(
        db.query_row(
            "SELECT status FROM memory_vector_rows WHERE profile_id='old-tombstone'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "tombstone"
    );
}

#[test]
fn legacy_and_parameterless_selectors_preserve_duplicate_target_semantics() {
    for parameterless in [false, true] {
        let db = initialized_db(true);
        for id in 1..=3 {
            projection(&db, id, if id == 3 { B } else { A }, "memory_fact", None);
            db.execute(
                "UPDATE memory_retrieval_index SET source_memory_id=77 WHERE id=?1",
                [id],
            )
            .unwrap();
            job(
                &db,
                &format!("job-{id}"),
                id,
                if id == 3 { B } else { A },
                ("queued", 0),
                ("profile", 1, "digest"),
            );
        }
        let tx = db.unchecked_transaction().unwrap();
        let removed = if parameterless {
            super::super::projection_retirement::remove(&tx, "SELECT id FROM memory_retrieval_index WHERE id IN (1,1,2)", &[])
        } else {
            super::super::projection_retirement::remove(&tx, "SELECT id FROM memory_retrieval_index WHERE source_memory_id=?1 AND principal_id=?2", params![77, A.0])
        }.unwrap();
        assert_eq!(removed, 2);
        tx.commit().unwrap();
        assert_eq!(ids(&db, "memory_retrieval_index"), vec![3]);
        assert_eq!(ids(&db, "memory_retrieval_index_fts"), vec![3]);
        assert_eq!(changed_jobs(&db), vec!["job-1", "job-2"]);
    }
}

#[test]
fn missing_targets_and_replays_have_no_unrelated_side_effects() {
    let db = initialized_db(true);
    projection(&db, 1, A, "memory_fact", Some("shared"));
    job(&db, "job", 1, A, ("queued", 0), ("profile", 1, "digest"));
    vector(&db, 1, A, "profile", 1, "active");
    let before = snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert_eq!(retire(&tx, A, "missing").unwrap(), 0);
    assert_eq!(snapshot(&tx), before);
    assert_eq!(retire(&tx, A, "shared").unwrap(), 1);
    let retired = snapshot(&tx);
    assert_eq!(retire(&tx, A, "shared").unwrap(), 0);
    assert_eq!(snapshot(&tx), retired);
    tx.commit().unwrap();
    assert_eq!(snapshot(&db), retired);
}

#[test]
fn absent_optional_tables_are_not_created_by_text_only_retirement() {
    let db = Connection::open_in_memory().unwrap();
    create_legacy_schema(&db, false);
    projection(&db, 1, A, "memory_fact", Some("shared"));
    let before = db
        .query_row("SELECT COUNT(*) FROM sqlite_schema", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
    let tx = db.unchecked_transaction().unwrap();
    assert_eq!(retire(&tx, A, "shared").unwrap(), 1);
    tx.commit().unwrap();
    assert!(ids(&db, "memory_retrieval_index").is_empty());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM sqlite_schema", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        before
    );
    assert_eq!(snapshot(&db).len(), 1);
}

#[test]
fn autocommit_retirement_is_rejected_without_changes() {
    let db = initialized_db(true);
    projection(&db, 1, A, "memory_fact", Some("shared"));
    job(&db, "job", 1, A, ("queued", 0), ("profile", 1, "digest"));
    let before = snapshot(&db);
    assert_eq!(
        retire(&db, A, "shared").unwrap_err().to_string(),
        "memory_retrieval_mutation_requires_transaction"
    );
    assert_eq!(snapshot(&db), before);
    assert!(db.is_autocommit());
}

#[test]
fn successful_nested_retirement_remains_owned_by_caller_transaction() {
    let db = initialized_db(true);
    projection(&db, 1, A, "memory_fact", Some("shared"));
    job(&db, "job", 1, A, ("queued", 0), ("profile", 1, "digest"));
    vector(&db, 1, A, "profile", 1, "active");
    let before = snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert_eq!(retire(&tx, A, "shared").unwrap(), 1);
    assert!(!tx.is_autocommit());
    tx.rollback().unwrap();
    assert_eq!(snapshot(&db), before);
}
