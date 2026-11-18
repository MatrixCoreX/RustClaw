use std::collections::BTreeMap;

use rusqlite::{params, types::Value, Connection};

pub(super) const A: (&str, &str, &str) = ("owner-a", "principal", "owner-a");
pub(super) const B: (&str, &str, &str) = ("owner-b", "principal", "owner-b");
pub(super) const ROOM: (&str, &str, &str) = ("owner-a", "conversation", "room-a");
pub(super) const PROJECT: (&str, &str, &str) = ("owner-a", "project", "project-a");
pub(super) const TARGETS: &str = "SELECT id FROM memory_retrieval_index
    WHERE principal_id=?1 AND source_kind=?2 AND source_ref=?3 AND scope_kind=?4 AND scope_ref=?5";

pub(super) fn create_legacy_schema(db: &Connection, pipeline: bool) {
    db.execute_batch(
        "CREATE TABLE runtime_schema_migrations(
            migration_id TEXT PRIMARY KEY,schema_digest TEXT NOT NULL,applied_at TEXT NOT NULL);
         CREATE TABLE memory_retrieval_index(
            id INTEGER PRIMARY KEY,principal_id TEXT,scope_kind TEXT,scope_ref TEXT,
            source_kind TEXT NOT NULL,source_ref TEXT,source_memory_id INTEGER,source_pref_key TEXT,
            search_text TEXT NOT NULL,updated_at_ts INTEGER NOT NULL DEFAULT 0);
         CREATE INDEX idx_memory_retrieval_principal_scope
            ON memory_retrieval_index(principal_id,scope_kind,scope_ref,updated_at_ts DESC);
         CREATE INDEX idx_memory_retrieval_source_ref ON memory_retrieval_index(source_ref);",
    )
    .unwrap();
    if pipeline {
        db.execute_batch(include_str!(
            "../../../../migrations/014_memory_vector_pipeline.sql"
        ))
        .unwrap();
    }
}

pub(super) fn legacy_db() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    create_legacy_schema(&db, true);
    db
}

pub(super) fn initialized_db(fts: bool) -> Connection {
    let db = legacy_db();
    super::ensure_retirement_access_schema(&db).unwrap();
    if fts {
        add_fts(&db);
    }
    db
}

pub(super) fn add_fts(db: &Connection) {
    db.execute_batch(
        "CREATE VIRTUAL TABLE memory_retrieval_index_fts USING fts5(search_text,topic_tags);
         INSERT INTO memory_retrieval_index_fts(rowid,search_text,topic_tags)
            SELECT id,search_text,'' FROM memory_retrieval_index;",
    )
    .unwrap();
}

pub(super) fn projection(
    db: &Connection,
    id: i64,
    owner: (&str, &str, &str),
    kind: &str,
    reference: Option<&str>,
) {
    let text = format!("row-{id}");
    db.execute(
        "INSERT INTO memory_retrieval_index(id,principal_id,scope_kind,scope_ref,source_kind,source_ref,search_text)
         VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![id, owner.0, owner.1, owner.2, kind, reference, text],
    )
    .unwrap();
    let fts: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='memory_retrieval_index_fts')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    if fts {
        db.execute("INSERT INTO memory_retrieval_index_fts(rowid,search_text,topic_tags) VALUES (?1,?2,'')", params![id,text]).unwrap();
    }
}

pub(super) fn job(
    db: &Connection,
    key: &str,
    retrieval: i64,
    owner: (&str, &str, &str),
    state: (&str, i64),
    version: (&str, i64, &str),
) {
    db.execute(
        "INSERT INTO memory_embedding_jobs(job_id,retrieval_id,principal_id,scope_kind,scope_ref,
            profile_id,profile_generation,request_item_id,projection_version,projection_digest,
            consent_policy_digest,status,not_before_ts,cancel_requested,created_at_ts,updated_at_ts)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?1,'fixture-v1',?8,'fixture-policy',?9,0,?10,0,0)",
        params![
            key, retrieval, owner.0, owner.1, owner.2, version.0, version.1, version.2, state.0,
            state.1
        ],
    )
    .unwrap();
}

pub(super) fn vector(
    db: &Connection,
    retrieval: i64,
    owner: (&str, &str, &str),
    profile: &str,
    generation: i64,
    status: &str,
) {
    db.execute(
        "INSERT INTO memory_vector_rows(retrieval_id,principal_id,scope_kind,scope_ref,profile_id,
            generation,projection_version,projection_digest,vector_format,dimensions,normalization,
            vector_blob,vector_checksum,status,created_at_ts,updated_at_ts)
         VALUES (?1,?2,?3,?4,?5,?6,'fixture-v1','fixture-digest','f32le_v1',1,'none',
            X'00000000','fixture-checksum',?7,0,0)",
        params![retrieval, owner.0, owner.1, owner.2, profile, generation, status],
    )
    .unwrap();
}

pub(super) fn retire(
    db: &Connection,
    owner: (&str, &str, &str),
    reference: &str,
) -> anyhow::Result<usize> {
    super::super::projection_retirement::remove(
        db,
        TARGETS,
        params![owner.0, "memory_fact", reference, owner.1, owner.2],
    )
}

pub(super) fn ids(db: &Connection, table: &str) -> Vec<i64> {
    db.prepare(&format!("SELECT rowid FROM {table} ORDER BY rowid"))
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

pub(super) fn snapshot(db: &Connection) -> BTreeMap<String, Vec<Vec<Value>>> {
    let mut result = BTreeMap::new();
    for table in [
        "memory_retrieval_index",
        "memory_embedding_jobs",
        "memory_vector_rows",
        "memory_retrieval_index_fts",
    ] {
        let exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        if !exists {
            continue;
        }
        // A deliberately malformed WITHOUT ROWID table must still be observable
        // when checking that an unsuccessful retirement made no repair writes.
        let mut statement = db
            .prepare(&format!("SELECT rowid,* FROM {table} ORDER BY rowid"))
            .or_else(|_| db.prepare(&format!("SELECT * FROM {table} ORDER BY 1")))
            .unwrap();
        let width = statement.column_count();
        let rows = statement
            .query_map([], |row| {
                (0..width)
                    .map(|index| row.get::<_, Value>(index))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        result.insert(table.to_string(), rows);
    }
    result
}

pub(super) fn seed_populated(db: &Connection, count: i64) {
    let tx = db.unchecked_transaction().unwrap();
    tx.execute(
        "WITH RECURSIVE ids(id) AS (VALUES(1) UNION ALL SELECT id+1 FROM ids WHERE id<?1)
         INSERT INTO memory_retrieval_index(id,principal_id,scope_kind,scope_ref,source_kind,source_ref,search_text)
         SELECT id,'owner-a','principal','owner-a','memory_fact','ref:'||id,'fixture' FROM ids",
        [count],
    ).unwrap();
    tx.execute_batch(
        "INSERT INTO memory_embedding_jobs(job_id,retrieval_id,principal_id,scope_kind,scope_ref,
            profile_id,profile_generation,request_item_id,projection_version,projection_digest,
            consent_policy_digest,status,not_before_ts,cancel_requested,created_at_ts,updated_at_ts)
         SELECT 'job:'||id,id,principal_id,scope_kind,scope_ref,'profile-a',1,'request:'||id,
            'fixture-v1','fixture-digest','fixture-policy','queued',0,0,0,0 FROM memory_retrieval_index;",
    ).unwrap();
    tx.commit().unwrap();
}
