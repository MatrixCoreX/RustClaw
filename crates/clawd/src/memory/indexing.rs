use claw_core::config::MemoryConfig;
use rusqlite::{params, Connection};
use serde_json::json;

use super::embedding::{embed_text_locally, local_hash_embedding_spec};
use super::retrieval::{build_topic_tags, vector_to_json};
#[cfg(test)]
use super::RETRIEVAL_SOURCE_KNOWLEDGE_FACT;
use super::{
    retrieval_source_ref_for_memory, retrieval_source_ref_for_memory_fact,
    retrieval_source_ref_for_preference, LLM_SHORT_TERM_MEMORY_PREFIX, MEMORY_FACT_STATUS_ACTIVE,
    MEMORY_ROLE_ASSISTANT, MEMORY_ROLE_SYSTEM, MEMORY_ROLE_USER, MEMORY_SCOPE_CHAT,
    MEMORY_SCOPE_USER, MEMORY_TYPE_SAFETY_SIGNAL, MEMORY_TYPE_UNFINISHED_GOAL,
    RETRIEVAL_KIND_ASSISTANT_RESULT, RETRIEVAL_KIND_EPISODIC_EVENT, RETRIEVAL_KIND_SEMANTIC_FACT,
    RETRIEVAL_KIND_TRIGGER_ANCHOR, RETRIEVAL_KIND_UNFINISHED_GOAL,
    RETRIEVAL_PRODUCER_MEMORY_PIPELINE, RETRIEVAL_SOURCE_MEMORY, RETRIEVAL_SOURCE_MEMORY_FACT,
    RETRIEVAL_SOURCE_PREFERENCE, RETRIEVAL_SUCCESS_STATE_NEUTRAL,
    RETRIEVAL_SUCCESS_STATE_SUCCEEDED,
};

pub(crate) fn ensure_retrieval_schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch(
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
            embedding_model   TEXT NOT NULL DEFAULT 'local-hash-v1',
            embedding_dims    INTEGER NOT NULL DEFAULT 24,
            embedding_version TEXT NOT NULL DEFAULT 'local-hash-v1',
            metadata_json     TEXT NOT NULL DEFAULT '{}',
            salience          REAL NOT NULL DEFAULT 0.5,
            success_state     TEXT NOT NULL DEFAULT 'neutral',
            tool_or_skill_name TEXT,
            created_at_ts     INTEGER NOT NULL DEFAULT 0,
            updated_at_ts     INTEGER NOT NULL DEFAULT 0
        );",
    )?;
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
        "ALTER TABLE memory_retrieval_index ADD COLUMN embedding_model TEXT NOT NULL DEFAULT 'local-hash-v1'",
    )?;
    crate::ensure_column_exists(
        db,
        "memory_retrieval_index",
        "embedding_dims",
        "ALTER TABLE memory_retrieval_index ADD COLUMN embedding_dims INTEGER NOT NULL DEFAULT 24",
    )?;
    crate::ensure_column_exists(
        db,
        "memory_retrieval_index",
        "embedding_version",
        "ALTER TABLE memory_retrieval_index ADD COLUMN embedding_version TEXT NOT NULL DEFAULT 'local-hash-v1'",
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
    ensure_retrieval_schema(db)?;
    db.execute("DELETE FROM memory_retrieval_index", [])?;
    let _ = db.execute("DELETE FROM memory_retrieval_index_fts", []);

    let mut mem_stmt = db.prepare(
        "SELECT id, user_id, chat_id, COALESCE(user_key, ''), role, content, memory_type, salience,
                is_instructional, created_at_ts
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
        ))
    })?;
    for row in mem_rows {
        let (
            memory_id,
            user_id,
            chat_id,
            user_key,
            role,
            content,
            memory_type,
            salience,
            is_instructional,
            ts,
        ) = row?;
        index_memory_row(
            db,
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

    let mut pref_stmt = db.prepare(
        "SELECT user_id, chat_id, COALESCE(user_key, ''), pref_key, pref_value, confidence, source, updated_at_ts
         FROM user_preferences
         ORDER BY id ASC",
    )?;
    let pref_rows = pref_stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, f32>(5).unwrap_or(0.8),
            row.get::<_, String>(6)?,
            row.get::<_, i64>(7).unwrap_or(0),
        ))
    })?;
    for row in pref_rows {
        let (user_id, chat_id, user_key, pref_key, pref_value, confidence, source, ts) = row?;
        let pref = vec![(pref_key, pref_value, confidence, source)];
        index_preference_entries(db, user_id, chat_id, &user_key, &pref, ts)?;
    }
    rebuild_memory_fact_rows(db)?;
    Ok(())
}

pub(crate) fn index_preference_entries(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    entries: &[(String, String, f32, String)],
    now_ts_i64: i64,
) -> anyhow::Result<()> {
    for (pref_key, pref_value, confidence, source) in entries {
        let source_ref = retrieval_source_ref_for_preference(pref_key);
        db.execute(
            "DELETE FROM memory_retrieval_index
             WHERE source_kind = ?1 AND user_id = ?2 AND chat_id = ?3
               AND COALESCE(user_key, '') = ?4 AND source_pref_key = ?5",
            params![
                RETRIEVAL_SOURCE_PREFERENCE,
                user_id,
                chat_id,
                user_key,
                pref_key
            ],
        )?;
        let text = format!("Preference {pref_key}: {pref_value}");
        insert_index_row(
            db,
            RETRIEVAL_SOURCE_PREFERENCE,
            None,
            Some(pref_key),
            Some(&source_ref),
            user_id,
            chat_id,
            user_key,
            RETRIEVAL_KIND_SEMANTIC_FACT,
            None,
            &text,
            Some(pref_key),
            Some(&build_preference_metadata_json(pref_key)),
            *confidence,
            RETRIEVAL_SUCCESS_STATE_SUCCEEDED,
            Some(source),
            now_ts_i64,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn index_memory_row(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    source_memory_id: i64,
    role: &str,
    content: &str,
    memory_type: &str,
    salience: f32,
    is_instructional: bool,
    created_at_ts: i64,
) -> anyhow::Result<()> {
    db.execute(
        "DELETE FROM memory_retrieval_index
         WHERE source_kind = ?1 AND source_memory_id = ?2",
        params![RETRIEVAL_SOURCE_MEMORY, source_memory_id],
    )?;

    let cleaned = content.trim();
    if cleaned.is_empty() {
        return Ok(());
    }
    let search_text = cleaned
        .strip_prefix(LLM_SHORT_TERM_MEMORY_PREFIX)
        .unwrap_or(cleaned)
        .trim();
    if search_text.is_empty() {
        return Ok(());
    }
    if memory_type == MEMORY_TYPE_SAFETY_SIGNAL {
        return Ok(());
    }
    if role == MEMORY_ROLE_ASSISTANT
        && super::is_transient_assistant_context_text_basic(search_text)
    {
        return Ok(());
    }
    let source_ref = retrieval_source_ref_for_memory(source_memory_id);

    if memory_type == MEMORY_TYPE_UNFINISHED_GOAL {
        insert_index_row(
            db,
            RETRIEVAL_SOURCE_MEMORY,
            Some(source_memory_id),
            None,
            Some(&source_ref),
            user_id,
            chat_id,
            user_key,
            RETRIEVAL_KIND_UNFINISHED_GOAL,
            Some(role),
            search_text,
            None,
            Some(&build_chat_scope_metadata_json()),
            (salience + 0.18).clamp(0.0, 1.0),
            RETRIEVAL_SUCCESS_STATE_NEUTRAL,
            None,
            created_at_ts,
        )?;
        return Ok(());
    }

    if role == MEMORY_ROLE_ASSISTANT {
        insert_index_row(
            db,
            RETRIEVAL_SOURCE_MEMORY,
            Some(source_memory_id),
            None,
            Some(&source_ref),
            user_id,
            chat_id,
            user_key,
            RETRIEVAL_KIND_ASSISTANT_RESULT,
            Some(role),
            search_text,
            None,
            Some(&build_chat_scope_metadata_json()),
            (salience + 0.08).clamp(0.0, 1.0),
            RETRIEVAL_SUCCESS_STATE_SUCCEEDED,
            None,
            created_at_ts,
        )?;
        return Ok(());
    }

    if role != MEMORY_ROLE_ASSISTANT {
        insert_index_row(
            db,
            RETRIEVAL_SOURCE_MEMORY,
            Some(source_memory_id),
            None,
            Some(&source_ref),
            user_id,
            chat_id,
            user_key,
            RETRIEVAL_KIND_EPISODIC_EVENT,
            Some(role),
            search_text,
            None,
            Some(&build_chat_scope_metadata_json()),
            salience,
            RETRIEVAL_SUCCESS_STATE_NEUTRAL,
            None,
            created_at_ts,
        )?;
    }

    if role == MEMORY_ROLE_USER && (is_instructional || search_text.chars().count() <= 240) {
        insert_index_row(
            db,
            RETRIEVAL_SOURCE_MEMORY,
            Some(source_memory_id),
            None,
            Some(&source_ref),
            user_id,
            chat_id,
            user_key,
            RETRIEVAL_KIND_TRIGGER_ANCHOR,
            Some(role),
            search_text,
            Some(search_text),
            Some(&build_chat_scope_metadata_json()),
            (salience + 0.08).clamp(0.0, 1.0),
            RETRIEVAL_SUCCESS_STATE_NEUTRAL,
            None,
            created_at_ts,
        )?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn insert_index_row(
    db: &Connection,
    source_kind: &str,
    source_memory_id: Option<i64>,
    source_pref_key: Option<&str>,
    source_ref: Option<&str>,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    memory_kind: &str,
    role: Option<&str>,
    search_text: &str,
    trigger_text: Option<&str>,
    metadata_json: Option<&str>,
    salience: f32,
    success_state: &str,
    tool_or_skill_name: Option<&str>,
    ts: i64,
) -> anyhow::Result<()> {
    let topic_tags = build_topic_tags(search_text);
    let vector_json = vector_to_json(&embed_text_locally(search_text));
    let embedding_spec = local_hash_embedding_spec();
    db.execute(
        "INSERT INTO memory_retrieval_index (
            source_kind, source_memory_id, source_pref_key, source_ref, user_id, chat_id, user_key,
            memory_kind, role, search_text, trigger_text, topic_tags, vector_json,
            embedding_model, embedding_dims, embedding_version, metadata_json,
            salience, success_state, tool_or_skill_name, created_at_ts, updated_at_ts
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?21)",
        params![
            source_kind,
            source_memory_id,
            source_pref_key,
            source_ref,
            user_id,
            chat_id,
            user_key,
            memory_kind,
            role,
            search_text,
            trigger_text,
            topic_tags,
            vector_json,
            embedding_spec.model_id,
            embedding_spec.dims as i64,
            embedding_spec.version,
            metadata_json.unwrap_or("{}"),
            salience,
            success_state,
            tool_or_skill_name,
            ts,
        ],
    )?;
    let row_id = db.last_insert_rowid();
    let _ = db.execute(
        "INSERT INTO memory_retrieval_index_fts(rowid, search_text, topic_tags)
         VALUES (?1, ?2, ?3)",
        params![row_id, search_text, topic_tags],
    );
    Ok(())
}

fn build_chat_scope_metadata_json() -> String {
    json!({
        "scope_kind": MEMORY_SCOPE_CHAT,
    })
    .to_string()
}

fn build_preference_metadata_json(pref_key: &str) -> String {
    json!({
        "scope_kind": MEMORY_SCOPE_CHAT,
        "namespace": "preferences",
        "path": pref_key,
        "preference_key": pref_key,
    })
    .to_string()
}

fn rebuild_memory_fact_rows(db: &Connection) -> anyhow::Result<()> {
    super::facts::ensure_memory_fact_schema(db)?;
    let now_ts = crate::now_ts_u64() as i64;
    let mut stmt = db.prepare(
        "SELECT id, user_id, COALESCE(user_key, ''), namespace, fact_text, confidence, updated_at_ts
         FROM memory_facts
         WHERE status = ?1 AND (expires_at_ts IS NULL OR expires_at_ts > ?2)
         ORDER BY updated_at_ts ASC, id ASC",
    )?;
    let rows = stmt.query_map(params![MEMORY_FACT_STATUS_ACTIVE, now_ts], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, f32>(5).unwrap_or(0.8),
            row.get::<_, i64>(6).unwrap_or(0),
        ))
    })?;
    for row in rows {
        let (fact_id, user_id, user_key, namespace, fact_text, confidence, ts) = row?;
        upsert_memory_fact_retrieval_row(
            db, user_id, &user_key, &namespace, fact_id, &fact_text, confidence, ts,
        )?;
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

pub(crate) fn upsert_memory_fact_retrieval_row(
    db: &Connection,
    user_id: i64,
    user_key: &str,
    namespace: &str,
    fact_id: i64,
    text: &str,
    confidence: f32,
    ts: i64,
) -> anyhow::Result<()> {
    let cleaned = text.trim();
    if cleaned.is_empty() {
        return Ok(());
    }
    let source_ref = retrieval_source_ref_for_memory_fact(fact_id);
    db.execute(
        "DELETE FROM memory_retrieval_index
         WHERE source_kind = ?1 AND source_ref = ?2",
        params![RETRIEVAL_SOURCE_MEMORY_FACT, source_ref],
    )?;
    let metadata = build_memory_fact_metadata_json(namespace, fact_id);
    insert_index_row(
        db,
        RETRIEVAL_SOURCE_MEMORY_FACT,
        None,
        None,
        Some(&source_ref),
        user_id,
        0,
        user_key,
        RETRIEVAL_KIND_SEMANTIC_FACT,
        Some(MEMORY_ROLE_SYSTEM),
        cleaned,
        None,
        Some(&metadata),
        confidence.clamp(0.0, 1.0),
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
        "scope_kind": MEMORY_SCOPE_USER,
        "namespace": namespace,
        "path": "conversation",
    })
    .to_string()
}

fn build_memory_fact_metadata_json(namespace: &str, fact_id: i64) -> String {
    json!({
        "scope_kind": MEMORY_SCOPE_USER,
        "namespace": namespace,
        "path": "memory_facts",
        "fact_id": fact_id,
    })
    .to_string()
}

#[cfg(test)]
#[path = "indexing_tests.rs"]
mod tests;
