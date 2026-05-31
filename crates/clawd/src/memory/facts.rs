use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;

use super::{
    MEMORY_FACT_STATUS_ACTIVE, MEMORY_FACT_STATUS_EXPIRED, MEMORY_FACT_STATUS_SUPERSEDED,
    MEMORY_SAFETY_FLAG_NORMAL, MEMORY_SCOPE_USER,
};

const SOURCE_KIND_LONG_TERM_SUMMARY: &str = "long_term_summary";

#[derive(Debug, Clone)]
pub(crate) struct MemoryFactUpsert<'a> {
    pub(crate) scope_kind: &'a str,
    pub(crate) namespace: &'a str,
    pub(crate) fact_key: &'a str,
    pub(crate) fact_value: &'a str,
    pub(crate) fact_text: &'a str,
    pub(crate) confidence: f32,
    pub(crate) source_kind: &'a str,
    pub(crate) source_ref: &'a str,
    pub(crate) source_memory_ids: &'a [i64],
    pub(crate) reason: &'a str,
    pub(crate) conflict_group: Option<&'a str>,
    pub(crate) expires_at_ts: Option<i64>,
    pub(crate) safety_flag: &'a str,
}

impl<'a> MemoryFactUpsert<'a> {
    pub(crate) fn from_long_term_summary(
        namespace: &'a str,
        fact_key: &'a str,
        fact_value: &'a str,
        fact_text: &'a str,
        confidence: f32,
        source_ref: &'a str,
        source_memory_ids: &'a [i64],
        reason: &'a str,
        conflict_group: Option<&'a str>,
    ) -> Self {
        Self {
            scope_kind: MEMORY_SCOPE_USER,
            namespace,
            fact_key,
            fact_value,
            fact_text,
            confidence,
            source_kind: SOURCE_KIND_LONG_TERM_SUMMARY,
            source_ref,
            source_memory_ids,
            reason,
            conflict_group,
            expires_at_ts: None,
            safety_flag: MEMORY_SAFETY_FLAG_NORMAL,
        }
    }
}

pub(crate) fn ensure_memory_fact_schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory_facts (
            id                     INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id                INTEGER NOT NULL,
            chat_id                INTEGER NOT NULL,
            user_key               TEXT NOT NULL,
            scope_kind             TEXT NOT NULL DEFAULT 'user',
            namespace              TEXT NOT NULL DEFAULT 'user_profile',
            fact_key               TEXT NOT NULL DEFAULT '',
            fact_value             TEXT NOT NULL DEFAULT '',
            fact_text              TEXT NOT NULL,
            confidence             REAL NOT NULL DEFAULT 0.8,
            source_kind            TEXT NOT NULL DEFAULT 'long_term_summary',
            source_ref             TEXT NOT NULL DEFAULT '',
            source_memory_ids_json TEXT NOT NULL DEFAULT '[]',
            reason                 TEXT NOT NULL DEFAULT '',
            created_at_ts          INTEGER NOT NULL DEFAULT 0,
            updated_at_ts          INTEGER NOT NULL DEFAULT 0,
            expires_at_ts          INTEGER,
            supersedes_fact_id     INTEGER,
            conflict_group         TEXT,
            safety_flag            TEXT NOT NULL DEFAULT 'normal',
            status                 TEXT NOT NULL DEFAULT 'active'
        );
        CREATE INDEX IF NOT EXISTS idx_memory_facts_scope_status_updated
        ON memory_facts(user_key, scope_kind, namespace, status, updated_at_ts DESC);
        CREATE INDEX IF NOT EXISTS idx_memory_facts_conflict_status
        ON memory_facts(user_key, namespace, conflict_group, status, updated_at_ts DESC);
        CREATE INDEX IF NOT EXISTS idx_memory_facts_source_ref
        ON memory_facts(source_kind, source_ref);
        CREATE INDEX IF NOT EXISTS idx_memory_facts_expiry
        ON memory_facts(status, expires_at_ts);",
    )?;
    Ok(())
}

pub(crate) fn upsert_memory_fact_card(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    fact: &MemoryFactUpsert<'_>,
    ts: i64,
) -> anyhow::Result<Option<i64>> {
    ensure_memory_fact_schema(db)?;
    let user_key = user_key.trim();
    let namespace = fact.namespace.trim();
    let fact_text = fact.fact_text.trim();
    if user_key.is_empty()
        || namespace.is_empty()
        || fact_text.is_empty()
        || super::fact_uses_cross_turn_deictic_locator(fact_text)
    {
        return Ok(None);
    }

    let scope_kind = normalized_or_default(fact.scope_kind, MEMORY_SCOPE_USER);
    let fact_key = fact.fact_key.trim();
    let fact_value = fact.fact_value.trim();
    let source_kind = normalized_or_default(fact.source_kind, SOURCE_KIND_LONG_TERM_SUMMARY);
    let source_ref = fact.source_ref.trim();
    let reason = fact.reason.trim();
    let safety_flag = normalized_or_default(fact.safety_flag, MEMORY_SAFETY_FLAG_NORMAL);
    let conflict_group = normalized_conflict_group(namespace, fact_key, fact.conflict_group);
    let source_memory_ids_json =
        serde_json::to_string(fact.source_memory_ids).unwrap_or_else(|_| json!([]).to_string());

    if let Some(conflict) = conflict_group.as_deref() {
        if let Some(existing_id) = find_active_same_fact(
            db, user_id, user_key, namespace, conflict, fact_text, fact_value,
        )? {
            db.execute(
                "UPDATE memory_facts
                 SET confidence = ?1,
                     source_kind = ?2,
                     source_ref = ?3,
                     source_memory_ids_json = ?4,
                     reason = ?5,
                     updated_at_ts = ?6,
                     expires_at_ts = ?7,
                     safety_flag = ?8
                 WHERE id = ?9",
                params![
                    fact.confidence,
                    source_kind,
                    source_ref,
                    source_memory_ids_json,
                    reason,
                    ts,
                    fact.expires_at_ts,
                    safety_flag,
                    existing_id
                ],
            )?;
            crate::memory::indexing::upsert_memory_fact_retrieval_row(
                db,
                user_id,
                user_key,
                namespace,
                existing_id,
                fact_text,
                fact.confidence,
                ts,
            )?;
            return Ok(Some(existing_id));
        }
    }

    let superseded_ids = match conflict_group.as_deref() {
        Some(conflict) => active_fact_ids_for_conflict(db, user_id, user_key, namespace, conflict)?,
        None => Vec::new(),
    };
    let supersedes_fact_id = superseded_ids.first().copied();
    if !superseded_ids.is_empty() {
        mark_facts_status(db, &superseded_ids, MEMORY_FACT_STATUS_SUPERSEDED, ts)?;
        crate::memory::indexing::delete_memory_fact_retrieval_rows(db, &superseded_ids)?;
    }

    db.execute(
        "INSERT INTO memory_facts (
            user_id, chat_id, user_key, scope_kind, namespace, fact_key, fact_value, fact_text,
            confidence, source_kind, source_ref, source_memory_ids_json, reason, created_at_ts,
            updated_at_ts, expires_at_ts, supersedes_fact_id, conflict_group, safety_flag, status
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14, ?15, ?16, ?17, ?18, ?19)",
        params![
            user_id,
            chat_id,
            user_key,
            scope_kind,
            namespace,
            fact_key,
            fact_value,
            fact_text,
            fact.confidence,
            source_kind,
            source_ref,
            source_memory_ids_json,
            reason,
            ts,
            fact.expires_at_ts,
            supersedes_fact_id,
            conflict_group,
            safety_flag,
            MEMORY_FACT_STATUS_ACTIVE,
        ],
    )?;
    let fact_id = db.last_insert_rowid();
    crate::memory::indexing::upsert_memory_fact_retrieval_row(
        db,
        user_id,
        user_key,
        namespace,
        fact_id,
        fact_text,
        fact.confidence,
        ts,
    )?;
    Ok(Some(fact_id))
}

pub(crate) fn expire_due_memory_facts(db: &Connection, now_ts: i64) -> anyhow::Result<usize> {
    ensure_memory_fact_schema(db)?;
    let ids = {
        let mut stmt = db.prepare(
            "SELECT id
             FROM memory_facts
             WHERE status = ?1 AND expires_at_ts IS NOT NULL AND expires_at_ts <= ?2",
        )?;
        let rows = stmt.query_map(params![MEMORY_FACT_STATUS_ACTIVE, now_ts], |row| {
            row.get::<_, i64>(0)
        })?;
        let mut ids = Vec::new();
        for row in rows {
            ids.push(row?);
        }
        ids
    };
    if ids.is_empty() {
        return Ok(0);
    }
    mark_facts_status(db, &ids, MEMORY_FACT_STATUS_EXPIRED, now_ts)?;
    crate::memory::indexing::delete_memory_fact_retrieval_rows(db, &ids)?;
    Ok(ids.len())
}

fn normalized_or_default<'a>(value: &'a str, default: &'a str) -> &'a str {
    let value = value.trim();
    if value.is_empty() {
        default
    } else {
        value
    }
}

fn normalized_conflict_group(
    namespace: &str,
    fact_key: &str,
    explicit: Option<&str>,
) -> Option<String> {
    explicit
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            let fact_key = fact_key.trim();
            (!fact_key.is_empty()).then(|| format!("{namespace}:{fact_key}"))
        })
}

fn find_active_same_fact(
    db: &Connection,
    user_id: i64,
    user_key: &str,
    namespace: &str,
    conflict_group: &str,
    fact_text: &str,
    fact_value: &str,
) -> anyhow::Result<Option<i64>> {
    db.query_row(
        "SELECT id
         FROM memory_facts
         WHERE user_id = ?1
           AND user_key = ?2
           AND namespace = ?3
           AND conflict_group = ?4
           AND fact_text = ?5
           AND fact_value = ?6
           AND status = ?7
         ORDER BY updated_at_ts DESC, id DESC
         LIMIT 1",
        params![
            user_id,
            user_key,
            namespace,
            conflict_group,
            fact_text,
            fact_value,
            MEMORY_FACT_STATUS_ACTIVE,
        ],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map_err(Into::into)
}

fn active_fact_ids_for_conflict(
    db: &Connection,
    user_id: i64,
    user_key: &str,
    namespace: &str,
    conflict_group: &str,
) -> anyhow::Result<Vec<i64>> {
    let mut stmt = db.prepare(
        "SELECT id
         FROM memory_facts
         WHERE user_id = ?1
           AND user_key = ?2
           AND namespace = ?3
           AND conflict_group = ?4
           AND status = ?5
         ORDER BY updated_at_ts DESC, id DESC",
    )?;
    let rows = stmt.query_map(
        params![
            user_id,
            user_key,
            namespace,
            conflict_group,
            MEMORY_FACT_STATUS_ACTIVE,
        ],
        |row| row.get::<_, i64>(0),
    )?;
    let mut ids = Vec::new();
    for row in rows {
        ids.push(row?);
    }
    Ok(ids)
}

fn mark_facts_status(
    db: &Connection,
    fact_ids: &[i64],
    status: &str,
    ts: i64,
) -> anyhow::Result<()> {
    for fact_id in fact_ids {
        db.execute(
            "UPDATE memory_facts SET status = ?1, updated_at_ts = ?2 WHERE id = ?3",
            params![status, ts, fact_id],
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "facts_tests.rs"]
mod tests;
