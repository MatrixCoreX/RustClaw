use claw_core::config::MemoryConfig;
use claw_core::retrieval_embedding::{LOCAL_HASH_DIMS, LOCAL_HASH_MODEL_ID, LOCAL_HASH_VERSION};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;

use super::retrieval::build_topic_tags;
use super::{
    retrieval_source_ref_for_memory, retrieval_source_ref_for_memory_fact,
    retrieval_source_ref_for_preference, LLM_SHORT_TERM_MEMORY_PREFIX, MEMORY_FACT_STATUS_ACTIVE,
    MEMORY_ROLE_ASSISTANT, MEMORY_ROLE_SYSTEM, MEMORY_ROLE_USER, MEMORY_SCOPE_CHAT,
    MEMORY_TYPE_SAFETY_SIGNAL, MEMORY_TYPE_UNFINISHED_GOAL, RETRIEVAL_KIND_ASSISTANT_RESULT,
    RETRIEVAL_KIND_EPISODIC_EVENT, RETRIEVAL_KIND_SEMANTIC_FACT, RETRIEVAL_KIND_TRIGGER_ANCHOR,
    RETRIEVAL_KIND_UNFINISHED_GOAL, RETRIEVAL_PRODUCER_MEMORY_PIPELINE, RETRIEVAL_SOURCE_MEMORY,
    RETRIEVAL_SOURCE_MEMORY_FACT, RETRIEVAL_SOURCE_PREFERENCE, RETRIEVAL_SUCCESS_STATE_NEUTRAL,
    RETRIEVAL_SUCCESS_STATE_SUCCEEDED,
};
#[cfg(test)]
use super::{MEMORY_SCOPE_PRINCIPAL, RETRIEVAL_SOURCE_KNOWLEDGE_FACT};

#[path = "indexing_canonical.rs"]
mod canonical;
pub(crate) use canonical::{
    index_memory_row, index_preference_entries, upsert_memory_fact_retrieval_row,
};

#[path = "indexing_insert.rs"]
mod writer;
pub(super) use writer::insert_index_row;

pub(crate) fn ensure_retrieval_schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS memory_retrieval_index (
            id                INTEGER PRIMARY KEY AUTOINCREMENT,
            source_kind       TEXT NOT NULL,
            source_memory_id  INTEGER,
            source_pref_key   TEXT,
            source_ref        TEXT,
            user_id           INTEGER NOT NULL,
            chat_id           INTEGER NOT NULL,
            user_key          TEXT,
            memory_kind       TEXT NOT NULL,
            role              TEXT,
            search_text       TEXT NOT NULL,
            trigger_text      TEXT,
            topic_tags        TEXT NOT NULL DEFAULT '',
            vector_json       TEXT NOT NULL DEFAULT '[]',
            embedding_model   TEXT NOT NULL DEFAULT '{LOCAL_HASH_MODEL_ID}',
            embedding_dims    INTEGER NOT NULL DEFAULT {LOCAL_HASH_DIMS},
            embedding_version TEXT NOT NULL DEFAULT '{LOCAL_HASH_VERSION}',
            metadata_json     TEXT NOT NULL DEFAULT '{{}}',
            salience          REAL NOT NULL DEFAULT 0.5,
            success_state     TEXT NOT NULL DEFAULT 'neutral',
            tool_or_skill_name TEXT,
            created_at_ts     INTEGER NOT NULL DEFAULT 0,
            updated_at_ts     INTEGER NOT NULL DEFAULT 0
        );"
    ))?;
    crate::ensure_column_exists(
        db,
        "memory_retrieval_index",
        "source_ref",
        "ALTER TABLE memory_retrieval_index ADD COLUMN source_ref TEXT",
    )?;
    crate::ensure_column_exists(
        db,
        "memory_retrieval_index",
        "metadata_json",
        "ALTER TABLE memory_retrieval_index ADD COLUMN metadata_json TEXT NOT NULL DEFAULT '{}'",
    )?;
    crate::ensure_column_exists(
        db,
        "memory_retrieval_index",
        "embedding_model",
        &format!(
            "ALTER TABLE memory_retrieval_index ADD COLUMN embedding_model TEXT NOT NULL DEFAULT '{LOCAL_HASH_MODEL_ID}'"
        ),
    )?;
    crate::ensure_column_exists(
        db,
        "memory_retrieval_index",
        "embedding_dims",
        &format!(
            "ALTER TABLE memory_retrieval_index ADD COLUMN embedding_dims INTEGER NOT NULL DEFAULT {LOCAL_HASH_DIMS}"
        ),
    )?;
    crate::ensure_column_exists(
        db,
        "memory_retrieval_index",
        "embedding_version",
        &format!(
            "ALTER TABLE memory_retrieval_index ADD COLUMN embedding_version TEXT NOT NULL DEFAULT '{LOCAL_HASH_VERSION}'"
        ),
    )?;
    db.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_memory_retrieval_scope_updated
         ON memory_retrieval_index(user_key, chat_id, updated_at_ts DESC);
         CREATE INDEX IF NOT EXISTS idx_memory_retrieval_scope_kind_updated
         ON memory_retrieval_index(user_key, chat_id, memory_kind, updated_at_ts DESC);
         CREATE INDEX IF NOT EXISTS idx_memory_retrieval_source_memory
         ON memory_retrieval_index(source_memory_id);
         CREATE INDEX IF NOT EXISTS idx_memory_retrieval_source_pref
         ON memory_retrieval_index(source_pref_key);
         CREATE INDEX IF NOT EXISTS idx_memory_retrieval_source_kind
         ON memory_retrieval_index(source_kind, updated_at_ts DESC);
         CREATE INDEX IF NOT EXISTS idx_memory_retrieval_source_ref
         ON memory_retrieval_index(source_ref);
         CREATE INDEX IF NOT EXISTS idx_memory_retrieval_embedding_version
         ON memory_retrieval_index(embedding_model, embedding_version);",
    )?;
    let _ = db.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS memory_retrieval_index_fts
         USING fts5(search_text, topic_tags);",
    );
    Ok(())
}

pub(crate) fn retrieval_index_is_empty(db: &Connection) -> anyhow::Result<bool> {
    let count: i64 = db.query_row("SELECT COUNT(*) FROM memory_retrieval_index", [], |row| {
        row.get(0)
    })?;
    Ok(count <= 0)
}

#[cfg(test)]
pub(crate) fn cleanup_retrieval_index(
    db: &Connection,
    cutoff_ts: i64,
    max_rows: usize,
) -> anyhow::Result<()> {
    db.execute(
        "DELETE FROM memory_retrieval_index
         WHERE COALESCE(updated_at_ts, created_at_ts, 0) < ?1",
        params![cutoff_ts],
    )?;
    db.execute(
        "DELETE FROM memory_retrieval_index WHERE id IN (
            SELECT id FROM memory_retrieval_index
            ORDER BY id DESC
            LIMIT -1 OFFSET ?1
         )",
        params![max_rows as i64],
    )?;
    let _ = db.execute(
        "DELETE FROM memory_retrieval_index_fts
         WHERE rowid NOT IN (SELECT id FROM memory_retrieval_index)",
        [],
    );
    Ok(())
}

pub(crate) fn rebuild_retrieval_index(db: &Connection, _cfg: &MemoryConfig) -> anyhow::Result<()> {
    super::recall_policy::require_schema(db)?;
    ensure_retrieval_schema(db)?;
    super::vector_store::ensure_vector_pipeline_schema(db)?;
    let tx = db.unchecked_transaction()?;
    tx.execute(
        "UPDATE memory_embedding_jobs SET cancel_requested = 1, updated_at_ts = ?1
         WHERE status IN ('queued', 'retry_wait', 'running')",
        [crate::now_ts_u64() as i64],
    )?;
    tx.execute(
        "UPDATE memory_vector_rows SET status = 'tombstone', updated_at_ts = ?1
         WHERE status = 'active'",
        [crate::now_ts_u64() as i64],
    )?;
    tx.execute("DELETE FROM memory_retrieval_index", [])?;
    let _ = tx.execute("DELETE FROM memory_retrieval_index_fts", []);

    let mut mem_stmt = tx.prepare(
        "SELECT id, user_id, chat_id, COALESCE(user_key, ''), role, content, memory_type, salience,
                is_instructional, created_at_ts,principal_id
         FROM memories
         ORDER BY id ASC",
    )?;
    let mem_rows = mem_stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, f32>(7).unwrap_or(0.5),
            row.get::<_, i64>(8).unwrap_or(0) != 0,
            row.get::<_, i64>(9).unwrap_or(0),
            row.get::<_, Option<String>>(10)?,
        ))
    })?;
    for row in mem_rows {
        let (
            memory_id,
            user_id,
            chat_id,
            _source_key,
            role,
            content,
            memory_type,
            salience,
            is_instructional,
            ts,
            principal,
        ) = row?;
        let Some(principal) = principal.filter(|value| !value.trim().is_empty()) else {
            continue;
        };
        let Some(user_key) = crate::repo::auth::active_principal_credential(&tx, &principal)?
        else {
            continue;
        };
        index_memory_row(
            &tx,
            user_id,
            chat_id,
            &user_key,
            memory_id,
            &role,
            &content,
            &memory_type,
            salience,
            is_instructional,
            ts,
        )?;
    }

    let mut pref_stmt = tx.prepare(
        "SELECT memory_id,principal_id,updated_at_ts FROM user_preferences ORDER BY id ASC",
    )?;
    let pref_rows = pref_stmt.query_map([], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in pref_rows {
        let (Some(memory_id), Some(principal), ts) = row? else {
            continue;
        };
        if crate::repo::auth::active_principal_credential(&tx, &principal)?.is_some() {
            super::ux::refresh_owned_canonical(&tx, &principal, &memory_id, ts)?;
        }
    }
    rebuild_memory_fact_rows(&tx)?;
    drop(pref_stmt);
    drop(mem_stmt);
    tx.commit()?;
    Ok(())
}

fn retrieval_scope_contract_available(db: &Connection) -> anyhow::Result<bool> {
    let mut stmt = db.prepare("PRAGMA table_info(memory_retrieval_index)")?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let mut has_principal = false;
    let mut has_scope_ref = false;
    for column in columns {
        match column?.as_str() {
            "principal_id" => has_principal = true,
            "scope_ref" => has_scope_ref = true,
            _ => {}
        }
    }
    Ok(has_principal && has_scope_ref)
}

fn build_chat_scope_metadata_json() -> String {
    json!({
        "scope_kind": MEMORY_SCOPE_CHAT,
    })
    .to_string()
}

pub(super) fn build_preference_metadata_json(
    pref_key: &str,
    scope_kind: &str,
    scope_ref: Option<&str>,
    canonical_memory_id: Option<&str>,
) -> String {
    json!({
        "scope_kind": scope_kind,
        "scope_ref": scope_ref,
        "namespace": "preferences",
        "path": pref_key,
        "preference_key": pref_key,
        "canonical_memory_id": canonical_memory_id,
    })
    .to_string()
}

fn rebuild_memory_fact_rows(db: &Connection) -> anyhow::Result<()> {
    super::facts::ensure_memory_fact_schema(db)?;
    let now_ts = crate::now_ts_u64() as i64;
    let mut statement = db.prepare(
        "SELECT memory_id,principal_id,updated_at_ts FROM memory_facts
         WHERE status=?1 AND (expires_at_ts IS NULL OR expires_at_ts>?2)
         ORDER BY updated_at_ts ASC,id ASC",
    )?;
    let rows = statement.query_map(params![MEMORY_FACT_STATUS_ACTIVE, now_ts], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (Some(memory_id), Some(principal), ts) = row? else {
            continue;
        };
        if crate::repo::auth::active_principal_credential(db, &principal)?.is_some() {
            super::ux::refresh_owned_canonical(db, &principal, &memory_id, ts)?;
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn upsert_knowledge_fact(
    db: &Connection,
    user_id: i64,
    user_key: &str,
    namespace: &str,
    retrieval_kind: &str,
    source_ref: &str,
    text: &str,
    ts: i64,
) -> anyhow::Result<()> {
    let cleaned = text.trim();
    if cleaned.is_empty() {
        return Ok(());
    }
    db.execute(
        "DELETE FROM memory_retrieval_index
         WHERE source_kind = ?1 AND source_ref = ?2",
        params![RETRIEVAL_SOURCE_KNOWLEDGE_FACT, source_ref],
    )?;
    let metadata = build_knowledge_fact_metadata_json(namespace);
    insert_index_row(
        db,
        RETRIEVAL_SOURCE_KNOWLEDGE_FACT,
        None,
        None,
        Some(source_ref),
        user_id,
        0,
        user_key,
        retrieval_kind,
        Some(MEMORY_ROLE_SYSTEM),
        cleaned,
        None,
        Some(&metadata),
        0.86,
        RETRIEVAL_SUCCESS_STATE_SUCCEEDED,
        Some(RETRIEVAL_PRODUCER_MEMORY_PIPELINE),
        ts,
    )?;
    let _ = db.execute(
        "DELETE FROM memory_retrieval_index_fts
         WHERE rowid NOT IN (SELECT id FROM memory_retrieval_index)",
        [],
    );
    Ok(())
}

pub(crate) fn delete_memory_fact_retrieval_rows(
    db: &Connection,
    fact_ids: &[i64],
) -> anyhow::Result<()> {
    for fact_id in fact_ids {
        let source_ref = retrieval_source_ref_for_memory_fact(*fact_id);
        db.execute(
            "DELETE FROM memory_retrieval_index
             WHERE source_kind = ?1 AND source_ref = ?2",
            params![RETRIEVAL_SOURCE_MEMORY_FACT, source_ref],
        )?;
    }
    let _ = db.execute(
        "DELETE FROM memory_retrieval_index_fts
         WHERE rowid NOT IN (SELECT id FROM memory_retrieval_index)",
        [],
    );
    Ok(())
}

#[cfg(test)]
fn build_knowledge_fact_metadata_json(namespace: &str) -> String {
    json!({
        "scope_kind": MEMORY_SCOPE_PRINCIPAL,
        "namespace": namespace,
        "path": "conversation",
    })
    .to_string()
}

pub(super) fn build_memory_fact_metadata_json(
    namespace: &str,
    fact_id: i64,
    scope_kind: &str,
    scope_ref: Option<&str>,
) -> String {
    json!({
        "scope_kind": scope_kind,
        "scope_ref": scope_ref,
        "namespace": namespace,
        "path": "memory_facts",
        "fact_id": fact_id,
    })
    .to_string()
}

#[cfg(test)]
#[path = "indexing_tests.rs"]
mod tests;
