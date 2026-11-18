use super::*;
use rusqlite::types::Value as SqlValue;

pub(super) fn state_without_feedback(db: &Connection) -> Vec<(&'static str, Vec<Vec<SqlValue>>)> {
    [
        "memories",
        "memory_facts",
        "user_preferences",
        "memory_revisions",
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
        let values = statement
            .query_map([], |r| {
                (0..columns)
                    .map(|c| r.get::<_, SqlValue>(c))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        (table, values)
    })
    .collect()
}

pub(super) fn scope_rows(
    db: &Connection,
    owner: &str,
    scope: &str,
) -> Vec<(i64, String, String, Option<String>)> {
    let mut statement = db
        .prepare(
            "SELECT i.id,i.search_text,i.metadata_json,f.search_text
        FROM memory_retrieval_index i LEFT JOIN memory_retrieval_index_fts f ON f.rowid=i.id
        WHERE i.principal_id=?1 AND i.scope_ref=?2 ORDER BY i.id",
        )
        .unwrap();
    let values = statement
        .query_map(params![owner, scope], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    values
}
