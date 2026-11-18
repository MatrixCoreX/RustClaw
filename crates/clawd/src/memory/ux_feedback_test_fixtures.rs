use rusqlite::types::Value as SqlValue;

use super::*;
use crate::memory::capacity_test_fixtures::Fixture;

pub(super) const MEMORY_ID: &str = "feedback-memory";

pub(super) fn fixture() -> Fixture {
    let fixture = setup::fixture();
    {
        let db = fixture.state().core.db.get().unwrap();
        setup::fact(
            &db,
            &fixture.key,
            &fixture.principal,
            MEMORY_ID,
            "The original private fact",
            100,
        );
    }
    fixture
}

pub(super) fn request(
    feedback_kind: MemoryFeedbackKind,
    corrected_content: Option<&str>,
) -> MemoryFeedbackRequest {
    MemoryFeedbackRequest {
        expected_revision: 1,
        feedback_kind,
        retrieval_event_ref: Some("opaque-retrieval-event".into()),
        corrected_content: corrected_content.map(str::to_owned),
    }
}

pub(super) fn private_state(db: &Connection) -> Vec<(&'static str, Vec<Vec<SqlValue>>)> {
    [
        "memories",
        "long_term_memories",
        "memory_facts",
        "user_preferences",
        "memory_revisions",
        "memory_retrieval_feedback",
        "memory_import_sessions",
        "memory_privacy_purge_queue",
        "memory_retrieval_index",
        "memory_principal_quotas",
        "memory_jobs",
    ]
    .into_iter()
    .map(|table| {
        let mut statement = db
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let columns = statement.column_count();
        let rows = statement
            .query_map([], |row| {
                (0..columns)
                    .map(|column| row.get::<_, SqlValue>(column))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        (table, rows)
    })
    .collect()
}

pub(super) fn reject_without_change(
    db: &Connection,
    principal: &str,
    memory_id: &str,
    request: &MemoryFeedbackRequest,
    expected_error: &str,
) {
    assert!(db.is_autocommit());
    let before = private_state(db);
    let error = record_feedback(db, principal, memory_id, request, 200).unwrap_err();
    assert!(
        error.to_string().contains(expected_error),
        "expected {expected_error}, got {error}"
    );
    assert_eq!(private_state(db), before);
    assert!(db.is_autocommit());
}

pub(super) fn assert_feedback(db: &Connection, principal: &str, memory_id: &str, kind: &str) {
    let row = db
        .query_row(
            "SELECT principal_id,memory_id,feedback_kind,retrieval_event_ref,
                    expected_revision,created_at_ts FROM memory_retrieval_feedback",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        row,
        (
            principal.into(),
            memory_id.into(),
            kind.into(),
            "opaque-retrieval-event".into(),
            1,
            200,
        )
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memory_retrieval_feedback",
            [],
            |row| { row.get::<_, i64>(0) }
        )
        .unwrap(),
        1
    );
    assert!(db.is_autocommit());
}
