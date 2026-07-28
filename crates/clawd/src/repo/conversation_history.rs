use std::collections::HashSet;

use claw_core::types::AuthIdentity;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{task_artifacts::TaskArtifactManifest, AppState};

const DEFAULT_PAGE_LIMIT: usize = 120;
const MAX_PAGE_LIMIT: usize = 200;
const MAX_USER_TEXT_BYTES: usize = 16 * 1024;
const MAX_ASSISTANT_TEXT_BYTES: usize = 64 * 1024;
const MAX_ERROR_TEXT_BYTES: usize = 8 * 1024;
const MAX_CONVERSATION_TITLE_CHARS: usize = 120;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ConversationHistoryTurn {
    pub(crate) schema_version: u32,
    pub(crate) conversation_id: String,
    pub(crate) external_chat_id: Option<String>,
    pub(crate) conversation_title: Option<String>,
    pub(crate) task_id: String,
    pub(crate) status: String,
    pub(crate) user_text: Option<String>,
    pub(crate) assistant_text: Option<String>,
    pub(crate) error_text: Option<String>,
    pub(crate) attachment_count: usize,
    pub(crate) attachment_kinds: Vec<String>,
    pub(crate) artifacts: Vec<TaskArtifactManifest>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ConversationHistoryPage {
    pub(crate) schema_version: u32,
    pub(crate) status: String,
    pub(crate) turns: Vec<ConversationHistoryTurn>,
    pub(crate) next_cursor: Option<String>,
    pub(crate) truncated: bool,
    pub(crate) content_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ConversationTitleUpdate {
    pub(crate) schema_version: u32,
    pub(crate) status: String,
    pub(crate) conversation_id: String,
    pub(crate) title: String,
    pub(crate) updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ConversationArchiveUpdate {
    pub(crate) schema_version: u32,
    pub(crate) status: String,
    pub(crate) conversation_id: String,
    pub(crate) archived_at: i64,
}

pub(crate) fn list_conversation_history(
    state: &AppState,
    identity: &AuthIdentity,
    limit: Option<usize>,
    cursor: Option<&str>,
) -> anyhow::Result<ConversationHistoryPage> {
    let limit = limit.unwrap_or(DEFAULT_PAGE_LIMIT).clamp(1, MAX_PAGE_LIMIT);
    let cursor = cursor.map(parse_cursor).transpose()?;
    let db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow::anyhow!("conversation_history_db_pool_failed:{error}"))?;
    let mut stmt = db.prepare(
        "SELECT tasks.task_id, tasks.external_chat_id, tasks.payload_json, tasks.status,
                tasks.result_json, tasks.error_text,
                CAST(COALESCE(NULLIF(tasks.created_at, ''), '0') AS INTEGER),
                CAST(COALESCE(NULLIF(tasks.updated_at, ''), tasks.created_at, '0') AS INTEGER),
                (
                    SELECT title
                    FROM conversation_metadata
                    WHERE conversation_id =
                            json_extract(tasks.payload_json, '$.conversation_id')
                      AND owner_user_key = ?1
                    ORDER BY CAST(COALESCE(NULLIF(updated_at, ''), '0') AS INTEGER) DESC
                    LIMIT 1
                )
         FROM tasks
         WHERE tasks.kind = 'ask'
           AND json_valid(tasks.payload_json)
           AND json_type(tasks.payload_json, '$.conversation_id') = 'text'
           AND (tasks.user_key = ?1 OR (tasks.user_key IS NULL AND tasks.user_id = ?2))
           AND NOT EXISTS (
                SELECT 1
                FROM conversation_archives
                WHERE owner_user_key = ?1
                  AND conversation_id = json_extract(tasks.payload_json, '$.conversation_id')
           )
           AND (
                ?3 IS NULL
                OR CAST(COALESCE(NULLIF(tasks.updated_at, ''), tasks.created_at, '0') AS INTEGER) < ?3
                OR (
                    CAST(COALESCE(NULLIF(tasks.updated_at, ''), tasks.created_at, '0') AS INTEGER) = ?3
                    AND tasks.task_id < ?4
                )
           )
         ORDER BY CAST(COALESCE(NULLIF(tasks.updated_at, ''), tasks.created_at, '0') AS INTEGER) DESC,
                  tasks.task_id DESC
         LIMIT ?5",
    )?;
    let cursor_ts = cursor.as_ref().map(|cursor| cursor.updated_at);
    let cursor_task_id = cursor.as_ref().map(|cursor| cursor.task_id.as_str());
    let rows = stmt.query_map(
        params![
            identity.user_key,
            identity.user_id,
            cursor_ts,
            cursor_task_id,
            (limit + 1) as i64,
        ],
        |row| {
            Ok(HistoryRow {
                task_id: row.get(0)?,
                external_chat_id: row.get(1)?,
                payload_json: row.get(2)?,
                status: row.get(3)?,
                result_json: row.get(4)?,
                error_text: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
                conversation_title: row.get(8)?,
            })
        },
    )?;
    let mut collected = rows.collect::<Result<Vec<_>, _>>()?;
    let truncated = collected.len() > limit;
    collected.truncate(limit);
    let next_cursor = if truncated {
        collected
            .last()
            .map(|row| format!("{}:{}", row.updated_at, row.task_id))
    } else {
        None
    };
    let turns = collected
        .into_iter()
        .filter_map(project_turn)
        .collect::<Vec<_>>();
    let content_sha256 = format!("{:x}", Sha256::digest(serde_json::to_vec(&turns)?));
    Ok(ConversationHistoryPage {
        schema_version: 1,
        status: "ok".to_string(),
        turns,
        next_cursor,
        truncated,
        content_sha256,
    })
}

pub(crate) fn archive_conversation(
    state: &AppState,
    identity: &AuthIdentity,
    conversation_id: &str,
) -> anyhow::Result<ConversationArchiveUpdate> {
    let conversation_id = bounded_machine_ref(conversation_id)
        .ok_or_else(|| anyhow::anyhow!("conversation_id_invalid"))?;
    let db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow::anyhow!("conversation_archive_db_pool_failed:{error}"))?;
    let visible = db.query_row(
        "SELECT EXISTS(
            SELECT 1
            FROM tasks
            WHERE kind = 'ask'
              AND json_valid(payload_json)
              AND json_extract(payload_json, '$.conversation_id') = ?1
              AND (user_key = ?2 OR (user_key IS NULL AND user_id = ?3))
         )",
        params![conversation_id, identity.user_key, identity.user_id,],
        |row| row.get::<_, i64>(0),
    )? != 0;
    if !visible {
        anyhow::bail!("conversation_not_found");
    }
    let archived_at = crate::app_helpers::now_ts_u64() as i64;
    db.execute(
        "INSERT INTO conversation_archives (
            owner_user_key, owner_user_id, conversation_id, archived_at
         ) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(owner_user_key, conversation_id) DO UPDATE SET
            owner_user_id = excluded.owner_user_id,
            archived_at = excluded.archived_at",
        params![
            identity.user_key,
            identity.user_id,
            conversation_id,
            archived_at.to_string(),
        ],
    )?;
    Ok(ConversationArchiveUpdate {
        schema_version: 1,
        status: "ok".to_string(),
        conversation_id,
        archived_at,
    })
}

pub(crate) fn update_conversation_title(
    state: &AppState,
    identity: &AuthIdentity,
    conversation_id: &str,
    title: &str,
) -> anyhow::Result<ConversationTitleUpdate> {
    let conversation_id = bounded_machine_ref(conversation_id)
        .ok_or_else(|| anyhow::anyhow!("conversation_id_invalid"))?;
    let title = normalized_conversation_title(title)
        .ok_or_else(|| anyhow::anyhow!("conversation_title_invalid"))?;
    let updated_at = crate::app_helpers::now_ts_u64() as i64;
    let now = updated_at.to_string();
    let db = state
        .core
        .db
        .get()
        .map_err(|error| anyhow::anyhow!("conversation_title_db_pool_failed:{error}"))?;
    if !conversation_exists_for_owner(&db, &identity.user_key, identity.user_id, &conversation_id)?
    {
        anyhow::bail!("conversation_not_found");
    }
    db.execute(
        "INSERT INTO conversation_metadata (
            owner_user_key, owner_user_id, conversation_id, title, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?5)
         ON CONFLICT(owner_user_key, conversation_id) DO UPDATE SET
            owner_user_id = excluded.owner_user_id,
            title = excluded.title,
            updated_at = excluded.updated_at",
        params![
            identity.user_key,
            identity.user_id,
            conversation_id,
            title,
            now,
        ],
    )?;
    Ok(ConversationTitleUpdate {
        schema_version: 1,
        status: "ok".to_string(),
        conversation_id,
        title,
        updated_at,
    })
}

fn conversation_exists_for_owner(
    db: &rusqlite::Connection,
    owner_user_key: &str,
    owner_user_id: i64,
    conversation_id: &str,
) -> rusqlite::Result<bool> {
    db.query_row(
        "SELECT EXISTS(
            SELECT 1
            FROM tasks
            WHERE kind = 'ask'
              AND json_valid(payload_json)
              AND json_extract(payload_json, '$.conversation_id') = ?1
              AND (user_key = ?2 OR (user_key IS NULL AND user_id = ?3))
         )",
        params![conversation_id, owner_user_key, owner_user_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|value| value != 0)
}

#[derive(Debug)]
struct HistoryRow {
    task_id: String,
    external_chat_id: Option<String>,
    payload_json: String,
    status: String,
    result_json: Option<String>,
    error_text: Option<String>,
    created_at: i64,
    updated_at: i64,
    conversation_title: Option<String>,
}

fn project_turn(row: HistoryRow) -> Option<ConversationHistoryTurn> {
    let payload = serde_json::from_str::<Value>(&row.payload_json).ok()?;
    let conversation_id = bounded_machine_ref(payload.get("conversation_id")?.as_str()?)?;
    let user_text = payload
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|text| bounded_text(text, MAX_USER_TEXT_BYTES));
    let (attachment_count, attachment_kinds) = attachment_projection(&payload);
    let result = row
        .result_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
    let assistant_text = result
        .as_ref()
        .and_then(visible_result_text)
        .map(|text| bounded_text(text, MAX_ASSISTANT_TEXT_BYTES));
    let artifacts = crate::task_artifacts::manifests_from_result(result.as_ref());
    let error_text = row
        .error_text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|text| bounded_text(text, MAX_ERROR_TEXT_BYTES));
    Some(ConversationHistoryTurn {
        schema_version: 1,
        conversation_id,
        external_chat_id: row
            .external_chat_id
            .map(|value| bounded_text(value.trim(), 256))
            .filter(|value| !value.is_empty()),
        conversation_title: row
            .conversation_title
            .as_deref()
            .and_then(normalized_conversation_title),
        task_id: row.task_id,
        status: row.status,
        user_text,
        assistant_text,
        error_text,
        attachment_count,
        attachment_kinds,
        artifacts,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn normalized_conversation_title(value: &str) -> Option<String> {
    let title = value.trim();
    if title.is_empty() || title.chars().count() > MAX_CONVERSATION_TITLE_CHARS {
        return None;
    }
    Some(title.to_string())
}

fn visible_result_text(value: &Value) -> Option<&str> {
    value
        .get("text")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .pointer("/task_journal/summary/final_answer")
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn attachment_projection(payload: &Value) -> (usize, Vec<String>) {
    let Some(items) = payload.get("attachments").and_then(Value::as_array) else {
        return (0, Vec::new());
    };
    let mut kinds = HashSet::new();
    for item in items {
        if let Some(kind) = item
            .get("kind")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|kind| !kind.is_empty() && kind.len() <= 32)
        {
            kinds.insert(kind.to_string());
        }
    }
    let mut kinds = kinds.into_iter().collect::<Vec<_>>();
    kinds.sort_unstable();
    (items.len(), kinds)
}

fn bounded_machine_ref(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':'))
    {
        return None;
    }
    Some(value.to_string())
}

fn bounded_text(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

#[derive(Debug)]
struct HistoryCursor {
    updated_at: i64,
    task_id: String,
}

fn parse_cursor(raw: &str) -> anyhow::Result<HistoryCursor> {
    let (updated_at, task_id) = raw
        .trim()
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("conversation_history_cursor_invalid"))?;
    let updated_at = updated_at
        .parse::<i64>()
        .map_err(|_| anyhow::anyhow!("conversation_history_cursor_invalid"))?;
    if uuid::Uuid::parse_str(task_id).is_err() {
        anyhow::bail!("conversation_history_cursor_invalid");
    }
    Ok(HistoryCursor {
        updated_at,
        task_id: task_id.to_string(),
    })
}

#[cfg(test)]
#[path = "conversation_history_tests.rs"]
mod tests;
