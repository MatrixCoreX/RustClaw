use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::{
    MEMORY_FACT_STATUS_DELETED, MEMORY_FACT_STATUS_EXPIRED, RETRIEVAL_SOURCE_MEMORY,
    RETRIEVAL_SOURCE_PREFERENCE,
};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct MemoryOverview {
    pub(crate) user_key: String,
    pub(crate) user_id: i64,
    pub(crate) chat_id: i64,
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

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub(crate) struct MemorySettingsRequest {
    pub(crate) long_term_enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct MemorySettingsResult {
    pub(crate) config_path: String,
    pub(crate) long_term_enabled: bool,
    pub(crate) restart_required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemoryObjectKind {
    Fact,
    Preference,
    Recent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MemoryObjectRef {
    kind: Option<MemoryObjectKind>,
    raw_id: i64,
}

pub(crate) fn memory_overview(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    long_term_enabled: bool,
    hybrid_recall_enabled: bool,
) -> anyhow::Result<MemoryOverview> {
    let counts = MemoryCounts {
        recent: count_recent(db, user_id, chat_id, user_key)?,
        preferences: count_preferences(db, user_id, chat_id, user_key)?,
        facts_active: count_facts(
            db,
            user_id,
            user_key,
            Some(super::MEMORY_FACT_STATUS_ACTIVE),
        )?,
        facts_total: count_facts(db, user_id, user_key, None)?,
        long_term_summaries: count_long_term_summaries(db, user_id, chat_id, user_key)?,
    };
    Ok(MemoryOverview {
        user_key: user_key.to_string(),
        user_id,
        chat_id,
        long_term_enabled,
        hybrid_recall_enabled,
        counts,
    })
}

pub(crate) fn list_preferences(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
) -> anyhow::Result<Vec<MemoryPreferenceItem>> {
    let mut stmt = db.prepare(
        "SELECT id, pref_key, pref_value, confidence, source, updated_at_ts
         FROM user_preferences
         WHERE user_id = ?1 AND chat_id = ?2 AND COALESCE(user_key, '') = ?3
         ORDER BY updated_at_ts DESC, id DESC",
    )?;
    let rows = stmt.query_map(params![user_id, chat_id, user_key], |row| {
        let raw_id = row.get::<_, i64>(0)?;
        Ok(MemoryPreferenceItem {
            id: format!("preference:{raw_id}"),
            raw_id,
            key: row.get(1)?,
            value: row.get(2)?,
            confidence: row.get::<_, f32>(3).unwrap_or(0.8),
            source: row.get(4)?,
            updated_at_ts: row.get::<_, i64>(5).unwrap_or(0),
        })
    })?;
    collect_rows(rows)
}

pub(crate) fn list_facts(
    db: &Connection,
    user_id: i64,
    user_key: &str,
) -> anyhow::Result<Vec<MemoryFactItem>> {
    let mut stmt = db.prepare(
        "SELECT id, namespace, fact_key, fact_value, fact_text, confidence, source_kind, source_ref,
                reason, updated_at_ts, expires_at_ts, conflict_group, status
         FROM memory_facts
         WHERE user_id = ?1 AND user_key = ?2
         ORDER BY
           CASE status WHEN 'active' THEN 0 WHEN 'superseded' THEN 1 WHEN 'expired' THEN 2 ELSE 3 END,
           updated_at_ts DESC,
           id DESC",
    )?;
    let rows = stmt.query_map(params![user_id, user_key], |row| {
        let raw_id = row.get::<_, i64>(0)?;
        Ok(MemoryFactItem {
            id: format!("fact:{raw_id}"),
            raw_id,
            namespace: row.get(1)?,
            fact_key: row.get(2)?,
            fact_value: row.get(3)?,
            fact_text: row.get(4)?,
            confidence: row.get::<_, f32>(5).unwrap_or(0.8),
            source_kind: row.get(6)?,
            source_ref: row.get(7)?,
            reason: row.get(8)?,
            updated_at_ts: row.get::<_, i64>(9).unwrap_or(0),
            expires_at_ts: row.get::<_, Option<i64>>(10)?,
            conflict_group: row.get::<_, Option<String>>(11)?,
            status: row.get(12)?,
        })
    })?;
    collect_rows(rows)
}

pub(crate) fn list_recent(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    limit: usize,
) -> anyhow::Result<Vec<MemoryRecentItem>> {
    let mut stmt = db.prepare(
        "SELECT id, role, memory_type, content, created_at_ts, safety_flag
         FROM memories
         WHERE user_id = ?1 AND chat_id = ?2 AND COALESCE(user_key, '') = ?3
         ORDER BY id DESC
         LIMIT ?4",
    )?;
    let rows = stmt.query_map(params![user_id, chat_id, user_key, limit as i64], |row| {
        let raw_id = row.get::<_, i64>(0)?;
        Ok(MemoryRecentItem {
            id: format!("memory:{raw_id}"),
            raw_id,
            role: row.get(1)?,
            memory_type: row.get(2)?,
            content: row.get(3)?,
            created_at_ts: row.get::<_, i64>(4).unwrap_or(0),
            safety_flag: row.get(5)?,
        })
    })?;
    collect_rows(rows)
}

pub(crate) fn delete_memory_object(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    object_id: &str,
    now_ts: i64,
) -> anyhow::Result<Option<MemoryDeleteResult>> {
    let object_ref = parse_memory_object_ref(object_id)?;
    match object_ref.kind {
        Some(MemoryObjectKind::Fact) => {
            delete_fact(db, user_id, user_key, object_ref.raw_id, object_id, now_ts)
        }
        Some(MemoryObjectKind::Preference) => {
            delete_preference(db, user_id, chat_id, user_key, object_ref.raw_id, object_id)
        }
        Some(MemoryObjectKind::Recent) => {
            delete_recent_memory(db, user_id, chat_id, user_key, object_ref.raw_id, object_id)
        }
        None => {
            if let Some(result) =
                delete_fact(db, user_id, user_key, object_ref.raw_id, object_id, now_ts)?
            {
                return Ok(Some(result));
            }
            if let Some(result) =
                delete_preference(db, user_id, chat_id, user_key, object_ref.raw_id, object_id)?
            {
                return Ok(Some(result));
            }
            delete_recent_memory(db, user_id, chat_id, user_key, object_ref.raw_id, object_id)
        }
    }
}

pub(crate) fn expire_memory_object(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    object_id: &str,
    now_ts: i64,
) -> anyhow::Result<Option<MemoryExpireResult>> {
    let object_ref = parse_memory_object_ref(object_id)?;
    match object_ref.kind {
        Some(MemoryObjectKind::Fact) => {
            expire_fact(db, user_id, user_key, object_ref.raw_id, object_id, now_ts)
        }
        Some(MemoryObjectKind::Preference) | Some(MemoryObjectKind::Recent) | None => {
            let deleted = delete_memory_object(db, user_id, chat_id, user_key, object_id, now_ts)?;
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
    user_id: i64,
    chat_id: i64,
    user_key: &str,
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
        result.recent_deleted = clear_recent_memories(db, user_id, chat_id, user_key)?;
    }
    if matches!(scope, MemoryClearScope::Preferences | MemoryClearScope::All) {
        result.preferences_deleted = clear_preferences(db, user_id, chat_id, user_key)?;
    }
    if matches!(scope, MemoryClearScope::Facts | MemoryClearScope::All) {
        result.facts_deleted = clear_facts(db, user_id, user_key, now_ts)?;
    }
    cleanup_fts(db)?;
    Ok(result)
}

pub(crate) fn update_memory_settings_file(
    workspace_root: &Path,
    req: &MemorySettingsRequest,
) -> anyhow::Result<MemorySettingsResult> {
    let config_path = workspace_root.join("configs/memory.toml");
    let mut raw = std::fs::read_to_string(&config_path)?;
    let mut long_term_enabled = read_bool_setting(&raw, "long_term_enabled").unwrap_or(true);
    let mut restart_required = false;
    if let Some(next) = req.long_term_enabled {
        if next != long_term_enabled {
            raw = upsert_bool_setting(&raw, "long_term_enabled", next);
            std::fs::write(&config_path, raw)?;
            restart_required = true;
        }
        long_term_enabled = next;
    }
    Ok(MemorySettingsResult {
        config_path: "configs/memory.toml".to_string(),
        long_term_enabled,
        restart_required,
    })
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
    user_id: i64,
    chat_id: i64,
    user_key: &str,
) -> anyhow::Result<usize> {
    let ids = collect_ids(
        db,
        "SELECT id FROM memories
         WHERE user_id = ?1 AND chat_id = ?2 AND COALESCE(user_key, '') = ?3",
        params![user_id, chat_id, user_key],
    )?;
    if ids.is_empty() {
        return Ok(0);
    }
    db.execute(
        "DELETE FROM memories
         WHERE user_id = ?1 AND chat_id = ?2 AND COALESCE(user_key, '') = ?3",
        params![user_id, chat_id, user_key],
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

fn clear_preferences(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
) -> anyhow::Result<usize> {
    let count = db.execute(
        "DELETE FROM user_preferences
         WHERE user_id = ?1 AND chat_id = ?2 AND COALESCE(user_key, '') = ?3",
        params![user_id, chat_id, user_key],
    )?;
    db.execute(
        "DELETE FROM memory_retrieval_index
         WHERE source_kind = ?1 AND user_id = ?2 AND chat_id = ?3 AND COALESCE(user_key, '') = ?4",
        params![RETRIEVAL_SOURCE_PREFERENCE, user_id, chat_id, user_key],
    )?;
    Ok(count)
}

fn clear_facts(
    db: &Connection,
    user_id: i64,
    user_key: &str,
    now_ts: i64,
) -> anyhow::Result<usize> {
    let ids = collect_ids(
        db,
        "SELECT id FROM memory_facts
         WHERE user_id = ?1 AND user_key = ?2 AND status != 'deleted'",
        params![user_id, user_key],
    )?;
    if ids.is_empty() {
        return Ok(0);
    }
    db.execute(
        "UPDATE memory_facts
         SET status = ?1, updated_at_ts = ?2
         WHERE user_id = ?3 AND user_key = ?4 AND status != ?1",
        params![MEMORY_FACT_STATUS_DELETED, now_ts, user_id, user_key],
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

fn read_bool_setting(raw: &str, key: &str) -> Option<bool> {
    raw.lines().find_map(|line| {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            return None;
        }
        let (left, right) = trimmed.split_once('=')?;
        if left.trim() != key {
            return None;
        }
        match right.trim().split('#').next()?.trim() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    })
}

fn upsert_bool_setting(raw: &str, key: &str, value: bool) -> String {
    let rendered = format!("{key} = {value}");
    let mut replaced = false;
    let mut lines = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with('#') {
            if let Some((left, _)) = trimmed.split_once('=') {
                if left.trim() == key {
                    let indent = &line[..line.len() - trimmed.len()];
                    lines.push(format!("{indent}{rendered}"));
                    replaced = true;
                    continue;
                }
            }
        }
        lines.push(line.to_string());
    }
    if !replaced {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(rendered);
    }
    let mut out = lines.join("\n");
    if raw.ends_with('\n') || !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn parse_memory_object_ref(raw: &str) -> anyhow::Result<MemoryObjectRef> {
    let raw = raw.trim();
    let (kind, id_text) = match raw.split_once(':') {
        Some(("fact", id)) => (Some(MemoryObjectKind::Fact), id),
        Some(("preference", id)) => (Some(MemoryObjectKind::Preference), id),
        Some(("memory", id)) | Some(("recent", id)) => (Some(MemoryObjectKind::Recent), id),
        Some((_, _)) => anyhow::bail!("unsupported memory id prefix"),
        None => (None, raw),
    };
    let raw_id = id_text
        .parse::<i64>()
        .map_err(|_| anyhow::anyhow!("invalid memory id"))?;
    if raw_id <= 0 {
        anyhow::bail!("invalid memory id");
    }
    Ok(MemoryObjectRef { kind, raw_id })
}

fn count_recent(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
) -> anyhow::Result<i64> {
    db.query_row(
        "SELECT COUNT(*) FROM memories
         WHERE user_id = ?1 AND chat_id = ?2 AND COALESCE(user_key, '') = ?3",
        params![user_id, chat_id, user_key],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

fn count_preferences(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
) -> anyhow::Result<i64> {
    db.query_row(
        "SELECT COUNT(*) FROM user_preferences
         WHERE user_id = ?1 AND chat_id = ?2 AND COALESCE(user_key, '') = ?3",
        params![user_id, chat_id, user_key],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

fn count_facts(
    db: &Connection,
    user_id: i64,
    user_key: &str,
    status: Option<&str>,
) -> anyhow::Result<i64> {
    match status {
        Some(status) => db
            .query_row(
                "SELECT COUNT(*) FROM memory_facts
                 WHERE user_id = ?1 AND user_key = ?2 AND status = ?3",
                params![user_id, user_key, status],
                |row| row.get(0),
            )
            .map_err(Into::into),
        None => db
            .query_row(
                "SELECT COUNT(*) FROM memory_facts
                 WHERE user_id = ?1 AND user_key = ?2",
                params![user_id, user_key],
                |row| row.get(0),
            )
            .map_err(Into::into),
    }
}

fn count_long_term_summaries(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
) -> anyhow::Result<i64> {
    db.query_row(
        "SELECT COUNT(*) FROM long_term_memories
         WHERE user_id = ?1 AND chat_id = ?2 AND COALESCE(user_key, '') = ?3",
        params![user_id, chat_id, user_key],
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
