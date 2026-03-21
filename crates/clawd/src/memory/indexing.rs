use claw_core::config::MemoryConfig;
use rusqlite::{params, Connection};

use super::retrieval::{build_topic_tags, embed_text_locally, vector_to_json};
use super::LLM_SHORT_TERM_MEMORY_PREFIX;

pub(crate) fn ensure_retrieval_schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory_retrieval_index (
            id                INTEGER PRIMARY KEY AUTOINCREMENT,
            source_kind       TEXT NOT NULL,
            source_memory_id  INTEGER,
            source_pref_key   TEXT,
            user_id           INTEGER NOT NULL,
            chat_id           INTEGER NOT NULL,
            user_key          TEXT,
            memory_kind       TEXT NOT NULL,
            role              TEXT,
            search_text       TEXT NOT NULL,
            trigger_text      TEXT,
            topic_tags        TEXT NOT NULL DEFAULT '',
            vector_json       TEXT NOT NULL DEFAULT '[]',
            salience          REAL NOT NULL DEFAULT 0.5,
            success_state     TEXT NOT NULL DEFAULT 'neutral',
            tool_or_skill_name TEXT,
            created_at_ts     INTEGER NOT NULL DEFAULT 0,
            updated_at_ts     INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_memory_retrieval_scope_updated
        ON memory_retrieval_index(user_key, chat_id, updated_at_ts DESC);
        CREATE INDEX IF NOT EXISTS idx_memory_retrieval_scope_kind_updated
        ON memory_retrieval_index(user_key, chat_id, memory_kind, updated_at_ts DESC);
        CREATE INDEX IF NOT EXISTS idx_memory_retrieval_source_memory
        ON memory_retrieval_index(source_memory_id);
        CREATE INDEX IF NOT EXISTS idx_memory_retrieval_source_pref
        ON memory_retrieval_index(source_pref_key);",
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
        db.execute(
            "DELETE FROM memory_retrieval_index
             WHERE source_kind = 'preference' AND user_id = ?1 AND chat_id = ?2
               AND COALESCE(user_key, '') = ?3 AND source_pref_key = ?4",
            params![user_id, chat_id, user_key, pref_key],
        )?;
        let text = format!("Preference {pref_key}: {pref_value}");
        insert_index_row(
            db,
            "preference",
            None,
            Some(pref_key),
            user_id,
            chat_id,
            user_key,
            "semantic_fact",
            None,
            &text,
            Some(pref_key),
            *confidence,
            "succeeded",
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
         WHERE source_kind = 'memory' AND source_memory_id = ?1",
        params![source_memory_id],
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

    insert_index_row(
        db,
        "memory",
        Some(source_memory_id),
        None,
        user_id,
        chat_id,
        user_key,
        "episodic_event",
        Some(role),
        search_text,
        None,
        salience,
        "neutral",
        None,
        created_at_ts,
    )?;

    if role == "user" && (is_instructional || search_text.chars().count() <= 240) {
        insert_index_row(
            db,
            "memory",
            Some(source_memory_id),
            None,
            user_id,
            chat_id,
            user_key,
            "trigger_anchor",
            Some(role),
            search_text,
            Some(search_text),
            (salience + 0.08).clamp(0.0, 1.0),
            "neutral",
            None,
            created_at_ts,
        )?;
    }

    if role == "user" && looks_like_durable_fact(search_text, memory_type) {
        insert_index_row(
            db,
            "memory",
            Some(source_memory_id),
            None,
            user_id,
            chat_id,
            user_key,
            "semantic_fact",
            Some(role),
            search_text,
            None,
            (salience + 0.12).clamp(0.0, 1.0),
            "neutral",
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
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    memory_kind: &str,
    role: Option<&str>,
    search_text: &str,
    trigger_text: Option<&str>,
    salience: f32,
    success_state: &str,
    tool_or_skill_name: Option<&str>,
    ts: i64,
) -> anyhow::Result<()> {
    let topic_tags = build_topic_tags(search_text);
    let vector_json = vector_to_json(&embed_text_locally(search_text));
    db.execute(
        "INSERT INTO memory_retrieval_index (
            source_kind, source_memory_id, source_pref_key, user_id, chat_id, user_key,
            memory_kind, role, search_text, trigger_text, topic_tags, vector_json,
            salience, success_state, tool_or_skill_name, created_at_ts, updated_at_ts
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?16)",
        params![
            source_kind,
            source_memory_id,
            source_pref_key,
            user_id,
            chat_id,
            user_key,
            memory_kind,
            role,
            search_text,
            trigger_text,
            topic_tags,
            vector_json,
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

fn looks_like_durable_fact(text: &str, memory_type: &str) -> bool {
    if memory_type == "user_instruction" {
        return true;
    }
    let norm = text.to_ascii_lowercase();
    [
        "以后",
        "默认",
        "记住",
        "always",
        "default",
        "prefer",
        "i am",
        "i'm",
        "我是",
        "我叫",
        "我的项目",
        "我的目标",
    ]
    .iter()
    .any(|marker| norm.contains(marker))
}
