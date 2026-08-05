use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::{
    MEMORY_FACT_STATUS_DELETED, MEMORY_FACT_STATUS_EXPIRED, RETRIEVAL_SOURCE_MEMORY,
    RETRIEVAL_SOURCE_PREFERENCE,
};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct MemoryOverview {
    pub(crate) long_term_enabled: bool,
    pub(crate) hybrid_recall_enabled: bool,
    pub(crate) counts: MemoryCounts,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct MemoryCounts {
    pub(crate) recent: i64,
    pub(crate) preferences: i64,
    pub(crate) facts_active: i64,
    pub(crate) facts_total: i64,
    pub(crate) long_term_summaries: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct MemoryPreferenceItem {
    pub(crate) id: String,
    #[serde(skip)]
    pub(crate) raw_id: i64,
    pub(crate) key: String,
    pub(crate) value: String,
    pub(crate) confidence: f32,
    pub(crate) source: String,
    pub(crate) updated_at_ts: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct MemoryFactItem {
    pub(crate) id: String,
    #[serde(skip)]
    pub(crate) raw_id: i64,
    pub(crate) namespace: String,
    pub(crate) fact_key: String,
    pub(crate) fact_value: String,
    pub(crate) fact_text: String,
    pub(crate) confidence: f32,
    pub(crate) source_kind: String,
    pub(crate) source_ref: String,
    pub(crate) reason: String,
    pub(crate) updated_at_ts: i64,
    pub(crate) expires_at_ts: Option<i64>,
    pub(crate) conflict_group: Option<String>,
    pub(crate) status: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct MemoryRecentItem {
    pub(crate) id: String,
    #[serde(skip)]
    pub(crate) raw_id: i64,
    pub(crate) role: String,
    pub(crate) memory_type: String,
    pub(crate) content: String,
    pub(crate) created_at_ts: i64,
    pub(crate) safety_flag: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct MemoryDeleteResult {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) deleted: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct MemoryExpireResult {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) expired: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) struct MemoryClearRequest {
    #[serde(default)]
    pub(crate) scope: MemoryClearScope,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemoryClearScope {
    Recent,
    Preferences,
    Facts,
    All,
}

impl Default for MemoryClearScope {
    fn default() -> Self {
        Self::Recent
    }
}

impl MemoryClearScope {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Recent => "recent",
            Self::Preferences => "preferences",
            Self::Facts => "facts",
            Self::All => "all",
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct MemoryClearResult {
    pub(crate) scope: String,
    pub(crate) recent_deleted: usize,
    pub(crate) preferences_deleted: usize,
    pub(crate) facts_deleted: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemoryObjectKind {
    Fact,
    Preference,
    Recent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MemoryObjectRef {
    kind: Option<MemoryObjectKind>,
    raw_id: Option<i64>,
    opaque_id: Option<String>,
}

pub(crate) fn memory_overview(
    db: &Connection,
    chat_id: i64,
    principal_id: &str,
    long_term_enabled: bool,
    hybrid_recall_enabled: bool,
) -> anyhow::Result<MemoryOverview> {
    let counts = MemoryCounts {
        recent: count_recent(db, chat_id, principal_id)?,
        preferences: count_preferences(db, chat_id, principal_id)?,
        facts_active: count_facts(db, principal_id, Some(super::MEMORY_FACT_STATUS_ACTIVE))?,
        facts_total: count_facts(db, principal_id, None)?,
        long_term_summaries: count_long_term_summaries(db, chat_id, principal_id)?,
    };
    Ok(MemoryOverview {
        long_term_enabled,
        hybrid_recall_enabled,
        counts,
    })
}

pub(crate) fn list_preferences(
    db: &Connection,
    chat_id: i64,
    principal_id: &str,
) -> anyhow::Result<Vec<MemoryPreferenceItem>> {
    let mut stmt = db.prepare(
        "SELECT id, memory_id, pref_key, pref_value, confidence, source, updated_at_ts
         FROM user_preferences
         WHERE principal_id = ?1 AND scope_kind = 'principal' AND scope_ref = ?1
           AND chat_id = ?2
         ORDER BY updated_at_ts DESC, id DESC
         LIMIT 100",
    )?;
    let rows = stmt.query_map(params![principal_id, chat_id], |row| {
        let raw_id = row.get::<_, i64>(0)?;
        let memory_id = row.get::<_, String>(1)?;
        Ok(MemoryPreferenceItem {
            id: memory_id,
            raw_id,
            key: row.get(2)?,
            value: row.get(3)?,
            confidence: row.get::<_, f32>(4).unwrap_or(0.8),
            source: row.get(5)?,
            updated_at_ts: row.get::<_, i64>(6).unwrap_or(0),
        })
    })?;
    collect_rows(rows)
}

pub(crate) fn list_facts(
    db: &Connection,
    principal_id: &str,
) -> anyhow::Result<Vec<MemoryFactItem>> {
    let mut stmt = db.prepare(
        "SELECT id, memory_id, namespace, fact_key, fact_value, fact_text, confidence, source_kind, source_ref,
                reason, updated_at_ts, expires_at_ts, conflict_group, status
         FROM memory_facts
         WHERE principal_id = ?1 AND scope_kind = 'principal' AND scope_ref = ?1
         ORDER BY
           CASE status WHEN 'active' THEN 0 WHEN 'superseded' THEN 1 WHEN 'expired' THEN 2 ELSE 3 END,
           updated_at_ts DESC,
           id DESC
         LIMIT 100",
    )?;
    let rows = stmt.query_map([principal_id], |row| {
        let raw_id = row.get::<_, i64>(0)?;
        let memory_id = row.get::<_, String>(1)?;
        Ok(MemoryFactItem {
            id: memory_id,
            raw_id,
            namespace: row.get(2)?,
            fact_key: row.get(3)?,
            fact_value: row.get(4)?,
            fact_text: row.get(5)?,
            confidence: row.get::<_, f32>(6).unwrap_or(0.8),
            source_kind: row.get(7)?,
            source_ref: row.get(8)?,
            reason: row.get(9)?,
            updated_at_ts: row.get::<_, i64>(10).unwrap_or(0),
            expires_at_ts: row.get::<_, Option<i64>>(11)?,
            conflict_group: row.get::<_, Option<String>>(12)?,
            status: row.get(13)?,
        })
    })?;
    collect_rows(rows)
}

pub(crate) fn list_recent(
    db: &Connection,
    chat_id: i64,
    principal_id: &str,
    limit: usize,
) -> anyhow::Result<Vec<MemoryRecentItem>> {
    let mut stmt = db.prepare(
        "SELECT id, memory_id, role, memory_type, content, created_at_ts, safety_flag
         FROM memories
         WHERE principal_id = ?1 AND scope_kind = 'principal' AND scope_ref = ?1
           AND chat_id = ?2
         ORDER BY id DESC
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![principal_id, chat_id, limit as i64], |row| {
        let raw_id = row.get::<_, i64>(0)?;
        let memory_id = row.get::<_, String>(1)?;
        Ok(MemoryRecentItem {
            id: memory_id,
            raw_id,
            role: row.get(2)?,
            memory_type: row.get(3)?,
            content: row.get(4)?,
            created_at_ts: row.get::<_, i64>(5).unwrap_or(0),
            safety_flag: row.get(6)?,
        })
    })?;
    collect_rows(rows)
}

pub(crate) fn delete_memory_object(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    principal_id: &str,
    object_id: &str,
    now_ts: i64,
) -> anyhow::Result<Option<MemoryDeleteResult>> {
    let object_ref =
        resolve_memory_object_ref(db, principal_id, parse_memory_object_ref(object_id)?)?;
    let Some(raw_id) = object_ref.raw_id else {
        return Ok(None);
    };
    match object_ref.kind {
        Some(MemoryObjectKind::Fact) => {
            delete_fact(db, user_id, user_key, raw_id, object_id, now_ts)
        }
        Some(MemoryObjectKind::Preference) => {
            delete_preference(db, user_id, chat_id, user_key, raw_id, object_id)
        }
        Some(MemoryObjectKind::Recent) => {
            delete_recent_memory(db, user_id, chat_id, user_key, raw_id, object_id)
        }
        None => {
            if let Some(result) = delete_fact(db, user_id, user_key, raw_id, object_id, now_ts)? {
                return Ok(Some(result));
            }
            if let Some(result) =
                delete_preference(db, user_id, chat_id, user_key, raw_id, object_id)?
            {
                return Ok(Some(result));
            }
            delete_recent_memory(db, user_id, chat_id, user_key, raw_id, object_id)
        }
    }
}

pub(crate) fn expire_memory_object(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    principal_id: &str,
    object_id: &str,
    now_ts: i64,
) -> anyhow::Result<Option<MemoryExpireResult>> {
    let object_ref =
        resolve_memory_object_ref(db, principal_id, parse_memory_object_ref(object_id)?)?;
    let Some(raw_id) = object_ref.raw_id else {
        return Ok(None);
    };
    match object_ref.kind {
        Some(MemoryObjectKind::Fact) => {
            expire_fact(db, user_id, user_key, raw_id, object_id, now_ts)
        }
        Some(MemoryObjectKind::Preference) | Some(MemoryObjectKind::Recent) | None => {
            let deleted = delete_memory_object(
                db,
                user_id,
                chat_id,
                user_key,
                principal_id,
                object_id,
                now_ts,
            )?;
            Ok(deleted.map(|result| MemoryExpireResult {
                id: result.id,
                kind: result.kind,
                expired: result.deleted,
            }))
        }
    }
}

pub(crate) fn clear_memory_scope(
    db: &Connection,
    chat_id: i64,
    principal_id: &str,
    scope: MemoryClearScope,
    now_ts: i64,
) -> anyhow::Result<MemoryClearResult> {
    let mut result = MemoryClearResult {
        scope: scope.as_str().to_string(),
        recent_deleted: 0,
        preferences_deleted: 0,
        facts_deleted: 0,
    };
    if matches!(scope, MemoryClearScope::Recent | MemoryClearScope::All) {
        result.recent_deleted = clear_recent_memories(db, chat_id, principal_id)?;
    }
    if matches!(scope, MemoryClearScope::Preferences | MemoryClearScope::All) {
        result.preferences_deleted = clear_preferences(db, chat_id, principal_id)?;
    }
    if matches!(scope, MemoryClearScope::Facts | MemoryClearScope::All) {
        result.facts_deleted = clear_facts(db, principal_id, now_ts)?;
    }
    cleanup_fts(db)?;
    Ok(result)
}

fn delete_fact(
    db: &Connection,
    user_id: i64,
    user_key: &str,
    raw_id: i64,
    display_id: &str,
    now_ts: i64,
) -> anyhow::Result<Option<MemoryDeleteResult>> {
    let exists = db
        .query_row(
            "SELECT id FROM memory_facts WHERE id = ?1 AND user_id = ?2 AND user_key = ?3",
            params![raw_id, user_id, user_key],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    let Some(fact_id) = exists else {
        return Ok(None);
    };
    db.execute(
        "UPDATE memory_facts
         SET status = ?1, updated_at_ts = ?2
         WHERE id = ?3 AND user_id = ?4 AND user_key = ?5",
        params![
            MEMORY_FACT_STATUS_DELETED,
            now_ts,
            fact_id,
            user_id,
            user_key
        ],
    )?;
    crate::memory::indexing::delete_memory_fact_retrieval_rows(db, &[fact_id])?;
    Ok(Some(MemoryDeleteResult {
        id: display_id.to_string(),
        kind: "fact".to_string(),
        deleted: true,
    }))
}

fn expire_fact(
    db: &Connection,
    user_id: i64,
    user_key: &str,
    raw_id: i64,
    display_id: &str,
    now_ts: i64,
) -> anyhow::Result<Option<MemoryExpireResult>> {
    let exists = db
        .query_row(
            "SELECT id FROM memory_facts WHERE id = ?1 AND user_id = ?2 AND user_key = ?3",
            params![raw_id, user_id, user_key],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    let Some(fact_id) = exists else {
        return Ok(None);
    };
    db.execute(
        "UPDATE memory_facts
         SET status = ?1, expires_at_ts = COALESCE(expires_at_ts, ?2), updated_at_ts = ?2
         WHERE id = ?3 AND user_id = ?4 AND user_key = ?5",
        params![
            MEMORY_FACT_STATUS_EXPIRED,
            now_ts,
            fact_id,
            user_id,
            user_key
        ],
    )?;
    crate::memory::indexing::delete_memory_fact_retrieval_rows(db, &[fact_id])?;
    Ok(Some(MemoryExpireResult {
        id: display_id.to_string(),
        kind: "fact".to_string(),
        expired: true,
    }))
}

fn delete_preference(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    raw_id: i64,
    display_id: &str,
) -> anyhow::Result<Option<MemoryDeleteResult>> {
    let pref_key = db
        .query_row(
            "SELECT pref_key
             FROM user_preferences
             WHERE id = ?1 AND user_id = ?2 AND chat_id = ?3 AND COALESCE(user_key, '') = ?4",
            params![raw_id, user_id, chat_id, user_key],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(pref_key) = pref_key else {
        return Ok(None);
    };
    db.execute(
        "DELETE FROM user_preferences
         WHERE id = ?1 AND user_id = ?2 AND chat_id = ?3 AND COALESCE(user_key, '') = ?4",
        params![raw_id, user_id, chat_id, user_key],
    )?;
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
    cleanup_fts(db)?;
    Ok(Some(MemoryDeleteResult {
        id: display_id.to_string(),
        kind: "preference".to_string(),
        deleted: true,
    }))
}

fn delete_recent_memory(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    raw_id: i64,
    display_id: &str,
) -> anyhow::Result<Option<MemoryDeleteResult>> {
    let changed = db.execute(
        "DELETE FROM memories
         WHERE id = ?1 AND user_id = ?2 AND chat_id = ?3 AND COALESCE(user_key, '') = ?4",
        params![raw_id, user_id, chat_id, user_key],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    db.execute(
        "DELETE FROM memory_retrieval_index
         WHERE source_kind = ?1 AND source_memory_id = ?2",
        params![RETRIEVAL_SOURCE_MEMORY, raw_id],
    )?;
    cleanup_fts(db)?;
    Ok(Some(MemoryDeleteResult {
        id: display_id.to_string(),
        kind: "memory".to_string(),
        deleted: true,
    }))
}

fn clear_recent_memories(
    db: &Connection,
    chat_id: i64,
    principal_id: &str,
) -> anyhow::Result<usize> {
    let ids = collect_ids(
        db,
        "SELECT id FROM memories
         WHERE principal_id = ?1 AND scope_kind = 'principal' AND scope_ref = ?1
           AND chat_id = ?2",
        params![principal_id, chat_id],
    )?;
    if ids.is_empty() {
        return Ok(0);
    }
    db.execute(
        "DELETE FROM memories
         WHERE principal_id = ?1 AND scope_kind = 'principal' AND scope_ref = ?1
           AND chat_id = ?2",
        params![principal_id, chat_id],
    )?;
    for id in &ids {
        db.execute(
            "DELETE FROM memory_retrieval_index
             WHERE source_kind = ?1 AND source_memory_id = ?2",
            params![RETRIEVAL_SOURCE_MEMORY, id],
        )?;
    }
    Ok(ids.len())
}

fn clear_preferences(db: &Connection, chat_id: i64, principal_id: &str) -> anyhow::Result<usize> {
    let count = db.execute(
        "DELETE FROM user_preferences
         WHERE principal_id = ?1 AND scope_kind = 'principal' AND scope_ref = ?1
           AND chat_id = ?2",
        params![principal_id, chat_id],
    )?;
    db.execute(
        "DELETE FROM memory_retrieval_index
         WHERE source_kind = ?1 AND principal_id = ?2
           AND scope_kind = 'principal' AND scope_ref = ?2 AND chat_id = ?3",
        params![RETRIEVAL_SOURCE_PREFERENCE, principal_id, chat_id],
    )?;
    Ok(count)
}

fn clear_facts(db: &Connection, principal_id: &str, now_ts: i64) -> anyhow::Result<usize> {
    let ids = collect_ids(
        db,
        "SELECT id FROM memory_facts
         WHERE principal_id = ?1 AND scope_kind = 'principal' AND scope_ref = ?1
           AND status != 'deleted'",
        [principal_id],
    )?;
    if ids.is_empty() {
        return Ok(0);
    }
    db.execute(
        "UPDATE memory_facts
         SET status = ?1, updated_at_ts = ?2
         WHERE principal_id = ?3 AND scope_kind = 'principal' AND scope_ref = ?3
           AND status != ?1",
        params![MEMORY_FACT_STATUS_DELETED, now_ts, principal_id],
    )?;
    crate::memory::indexing::delete_memory_fact_retrieval_rows(db, &ids)?;
    Ok(ids.len())
}

fn collect_ids(
    db: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> anyhow::Result<Vec<i64>> {
    let mut stmt = db.prepare(sql)?;
    let rows = stmt.query_map(params, |row| row.get::<_, i64>(0))?;
    collect_rows(rows)
}

fn parse_memory_object_ref(raw: &str) -> anyhow::Result<MemoryObjectRef> {
    let raw = raw.trim();
    if raw.starts_with("memory_")
        && raw.len() <= 96
        && raw
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Ok(MemoryObjectRef {
            kind: None,
            raw_id: None,
            opaque_id: Some(raw.to_string()),
        });
    }
    let (kind, id_text) = match raw.split_once(':') {
        Some(("fact", id)) => (Some(MemoryObjectKind::Fact), id),
        Some(("preference", id)) => (Some(MemoryObjectKind::Preference), id),
        Some(("memory", id)) | Some(("recent", id)) => (Some(MemoryObjectKind::Recent), id),
        Some((_, _)) => anyhow::bail!("memory_id_prefix_unsupported"),
        None => (None, raw),
    };
    let raw_id = id_text
        .parse::<i64>()
        .map_err(|_| anyhow::anyhow!("memory_id_invalid"))?;
    if raw_id <= 0 {
        anyhow::bail!("memory_id_invalid");
    }
    Ok(MemoryObjectRef {
        kind,
        raw_id: Some(raw_id),
        opaque_id: None,
    })
}

fn resolve_memory_object_ref(
    db: &Connection,
    principal_id: &str,
    object_ref: MemoryObjectRef,
) -> anyhow::Result<MemoryObjectRef> {
    let Some(opaque_id) = object_ref.opaque_id.as_deref() else {
        return Ok(object_ref);
    };
    let resolved = db
        .query_row(
            "SELECT raw_id, kind FROM (
                SELECT id AS raw_id, 'fact' AS kind FROM memory_facts
                 WHERE memory_id = ?1 AND principal_id = ?2
                UNION ALL
                SELECT id AS raw_id, 'preference' AS kind FROM user_preferences
                 WHERE memory_id = ?1 AND principal_id = ?2
                UNION ALL
                SELECT id AS raw_id, 'recent' AS kind FROM memories
                 WHERE memory_id = ?1 AND principal_id = ?2
             ) LIMIT 1",
            params![opaque_id, principal_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let Some((raw_id, kind)) = resolved else {
        return Ok(object_ref);
    };
    let kind = match kind.as_str() {
        "fact" => MemoryObjectKind::Fact,
        "preference" => MemoryObjectKind::Preference,
        "recent" => MemoryObjectKind::Recent,
        _ => anyhow::bail!("memory_object_kind_invalid"),
    };
    Ok(MemoryObjectRef {
        kind: Some(kind),
        raw_id: Some(raw_id),
        opaque_id: object_ref.opaque_id,
    })
}

fn count_recent(db: &Connection, chat_id: i64, principal_id: &str) -> anyhow::Result<i64> {
    db.query_row(
        "SELECT COUNT(*) FROM memories
         WHERE principal_id = ?1 AND scope_kind = 'principal' AND scope_ref = ?1
           AND chat_id = ?2",
        params![principal_id, chat_id],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

fn count_preferences(db: &Connection, chat_id: i64, principal_id: &str) -> anyhow::Result<i64> {
    db.query_row(
        "SELECT COUNT(*) FROM user_preferences
         WHERE principal_id = ?1 AND scope_kind = 'principal' AND scope_ref = ?1
           AND chat_id = ?2",
        params![principal_id, chat_id],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

fn count_facts(db: &Connection, principal_id: &str, status: Option<&str>) -> anyhow::Result<i64> {
    match status {
        Some(status) => db
            .query_row(
                "SELECT COUNT(*) FROM memory_facts
                 WHERE principal_id = ?1 AND scope_kind = 'principal' AND scope_ref = ?1
                   AND status = ?2",
                params![principal_id, status],
                |row| row.get(0),
            )
            .map_err(Into::into),
        None => db
            .query_row(
                "SELECT COUNT(*) FROM memory_facts
                 WHERE principal_id = ?1 AND scope_kind = 'principal' AND scope_ref = ?1",
                [principal_id],
                |row| row.get(0),
            )
            .map_err(Into::into),
    }
}

fn count_long_term_summaries(
    db: &Connection,
    chat_id: i64,
    principal_id: &str,
) -> anyhow::Result<i64> {
    db.query_row(
        "SELECT COUNT(*) FROM long_term_memories
         WHERE principal_id = ?1 AND scope_kind = 'principal' AND scope_ref = ?1
           AND chat_id = ?2",
        params![principal_id, chat_id],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

fn cleanup_fts(db: &Connection) -> anyhow::Result<()> {
    let _ = db.execute(
        "DELETE FROM memory_retrieval_index_fts
         WHERE rowid NOT IN (SELECT id FROM memory_retrieval_index)",
        [],
    );
    Ok(())
}

fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> anyhow::Result<Vec<T>> {
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

#[cfg(test)]
#[path = "api_tests.rs"]
mod tests;
