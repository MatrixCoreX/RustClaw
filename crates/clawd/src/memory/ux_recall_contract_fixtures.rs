use rusqlite::types::Value as SqlValue;

use super::*;
use crate::memory::indexing;

pub(super) fn seed(db: &Connection, key: &str, owner: &str, kind: &str, id: &str) -> String {
    match kind {
        "fact" => {
            setup::fact(db, key, owner, id, "Original private fact", 100);
            let row: i64 = db
                .query_row(
                    "SELECT id FROM memory_facts WHERE memory_id=?1",
                    [id],
                    |r| r.get(0),
                )
                .unwrap();
            indexing::upsert_memory_fact_retrieval_row(
                db,
                1,
                key,
                "fixture_profile",
                row,
                "Original private fact",
                1.0,
                100,
            )
            .unwrap();
            crate::memory::retrieval_source_ref_for_memory_fact(row)
        }
        "preference" => {
            setup::preference(db, key, owner, id);
            indexing::index_preference_entries(
                db,
                1,
                7,
                key,
                &[(
                    "response_format".into(),
                    "markdown".into(),
                    1.0,
                    "user_message".into(),
                )],
                100,
            )
            .unwrap();
            crate::memory::retrieval_source_ref_for_preference("response_format")
        }
        "recent" => {
            setup::recent(db, key, owner, id, "Original private transcript");
            let row: i64 = db
                .query_row("SELECT id FROM memories WHERE memory_id=?1", [id], |r| {
                    r.get(0)
                })
                .unwrap();
            indexing::index_memory_row(
                db,
                1,
                7,
                key,
                row,
                "user",
                "Original private transcript",
                "episodic_event",
                0.5,
                false,
                100,
            )
            .unwrap();
            crate::memory::retrieval_source_ref_for_memory(row)
        }
        _ => panic!("unsupported fixture kind"),
    }
}

pub(super) fn rows(db: &Connection, owner: &str) -> Vec<(String, String, Option<String>)> {
    let mut statement = db
        .prepare(
            "SELECT i.source_ref,i.search_text,f.search_text FROM memory_retrieval_index i
         LEFT JOIN memory_retrieval_index_fts f ON f.rowid=i.id
         WHERE i.principal_id=?1 ORDER BY i.source_ref,i.id",
        )
        .unwrap();
    let rows = statement
        .query_map([owner], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    rows
}

pub(super) fn assert_no_orphan_fts(db: &Connection) {
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM memory_retrieval_index_fts
        WHERE rowid NOT IN (SELECT id FROM memory_retrieval_index)",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0,
        "removed retrieval rows must not leave searchable FTS content"
    );
}

pub(super) fn state(db: &Connection) -> Vec<(&'static str, Vec<Vec<SqlValue>>)> {
    [
        "memories",
        "memory_facts",
        "user_preferences",
        "memory_revisions",
        "memory_retrieval_feedback",
        "memory_retrieval_index",
        "memory_retrieval_index_fts",
        "memory_embedding_jobs",
        "memory_vector_rows",
    ]
    .into_iter()
    .map(|table| {
        let mut statement = db
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let columns = statement.column_count();
        let rows = statement
            .query_map([], |r| {
                (0..columns)
                    .map(|c| r.get::<_, SqlValue>(c))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        (table, rows)
    })
    .collect()
}

pub(super) fn correction() -> MemoryCorrectionRequest {
    MemoryCorrectionRequest {
        expected_revision: 1,
        content: "Corrected private content".into(),
    }
}

pub(super) fn feedback(kind: MemoryFeedbackKind) -> MemoryFeedbackRequest {
    MemoryFeedbackRequest {
        expected_revision: 1,
        feedback_kind: kind,
        retrieval_event_ref: None,
        corrected_content: Some("Corrected private content".into()),
    }
}

pub(super) fn assert_replacement(db: &Connection, owner: &str, kind: &str, replacement: &str) {
    let (reference, text) = if kind == "fact" {
        let row: i64 = db
            .query_row(
                "SELECT id FROM memory_facts WHERE memory_id=?1 AND principal_id=?2",
                params![replacement, owner],
                |r| r.get(0),
            )
            .unwrap();
        (
            crate::memory::retrieval_source_ref_for_memory_fact(row),
            "Corrected private content".to_owned(),
        )
    } else {
        (
            crate::memory::retrieval_source_ref_for_preference("response_format"),
            "Preference response_format: Corrected private content".to_owned(),
        )
    };
    assert_eq!(
        rows(db, owner),
        vec![(reference, text.clone(), Some(text))],
        "retrieval and FTS must expose only the corrected canonical content"
    );
    assert_no_orphan_fts(db);
}

pub(super) fn rebuild(db: &Connection) {
    indexing::rebuild_retrieval_index(db, &claw_core::config::MemoryConfig::default()).unwrap();
}
