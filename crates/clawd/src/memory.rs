use std::collections::HashSet;

pub(crate) mod indexing;
pub(crate) mod retrieval;
pub(crate) mod service;

pub(crate) use service::dynamic_chat_memory_budget_chars;

use anyhow::anyhow;
use claw_core::config::MemoryConfig;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use super::{extract_delivery_file_tokens, now_ts, now_ts_u64, utf8_safe_prefix, AppState};

pub(crate) const LLM_SHORT_TERM_MEMORY_PREFIX: &str = "[LLM_REPLY] ";
pub(crate) const MEMORY_ROLE_USER: &str = "user";
pub(crate) const MEMORY_ROLE_ASSISTANT: &str = "assistant";
pub(crate) const MEMORY_ROLE_SYSTEM: &str = "system";

pub(crate) const MEMORY_SAFETY_FLAG_NORMAL: &str = "normal";
pub(crate) const MEMORY_SAFETY_FLAG_INJECTION_LIKE: &str = "injection_like";

pub(crate) const MEMORY_TYPE_GENERIC: &str = "generic";
pub(crate) const MEMORY_TYPE_SAFETY_SIGNAL: &str = "safety_signal";
pub(crate) const MEMORY_TYPE_UNFINISHED_GOAL: &str = "unfinished_goal";
pub(crate) const MEMORY_TYPE_ASSISTANT_OUTCOME: &str = "assistant_outcome";
pub(crate) const MEMORY_TYPE_ASSISTANT_REPLY: &str = "assistant_reply";
pub(crate) const MEMORY_TYPE_USER_INSTRUCTION: &str = "user_instruction";

// `source_kind` answers "where did this retrieval row come from?"
// It tracks the producer/origin table or pipeline.
pub(crate) const RETRIEVAL_SOURCE_MEMORY: &str = "memory";
pub(crate) const RETRIEVAL_SOURCE_PREFERENCE: &str = "preference";
pub(crate) const RETRIEVAL_SOURCE_KB_DOC: &str = "kb_doc";
pub(crate) const RETRIEVAL_SOURCE_KNOWLEDGE_FACT: &str = "knowledge_fact";

// `memory_kind` answers "how should this row be recalled/presented?"
// Multiple sources may map into the same recall bucket.
pub(crate) const RETRIEVAL_KIND_TRIGGER_ANCHOR: &str = "trigger_anchor";
pub(crate) const RETRIEVAL_KIND_SEMANTIC_FACT: &str = "semantic_fact";
pub(crate) const RETRIEVAL_KIND_KNOWLEDGE_DOC: &str = "knowledge_doc";
pub(crate) const RETRIEVAL_KIND_EPISODIC_EVENT: &str = "episodic_event";
pub(crate) const RETRIEVAL_KIND_ASSISTANT_RESULT: &str = "assistant_result";
pub(crate) const RETRIEVAL_KIND_UNFINISHED_GOAL: &str = "unfinished_goal";

pub(crate) const RETRIEVAL_SUCCESS_STATE_SUCCEEDED: &str = "succeeded";
pub(crate) const RETRIEVAL_SUCCESS_STATE_FAILED: &str = "failed";
pub(crate) const RETRIEVAL_SUCCESS_STATE_NEUTRAL: &str = "neutral";

pub(crate) const MEMORY_SCOPE_CHAT: &str = "chat";
pub(crate) const MEMORY_SCOPE_USER: &str = "user";

// `source_ref` is a stable, source-local identity key for update/dedup flows.
// It should be machine-oriented and not treated as user-facing display text.
// `tool_or_skill_name` is a best-effort producer label for debugging/analytics.
pub(crate) const RETRIEVAL_PRODUCER_KB: &str = "kb";
pub(crate) const RETRIEVAL_PRODUCER_MEMORY_PIPELINE: &str = RETRIEVAL_SOURCE_MEMORY;

pub(crate) fn retrieval_source_is_knowledge(source_kind: &str) -> bool {
    matches!(
        source_kind,
        RETRIEVAL_SOURCE_KB_DOC | RETRIEVAL_SOURCE_KNOWLEDGE_FACT
    )
}

pub(crate) fn retrieval_kind_is_fact_bucket(memory_kind: &str) -> bool {
    memory_kind == RETRIEVAL_KIND_SEMANTIC_FACT
}

pub(crate) fn retrieval_kind_is_knowledge_doc_bucket(memory_kind: &str) -> bool {
    memory_kind == RETRIEVAL_KIND_KNOWLEDGE_DOC
}

pub(crate) fn retrieval_source_ref_for_memory(memory_id: i64) -> String {
    memory_id.to_string()
}

pub(crate) fn retrieval_source_ref_for_preference(pref_key: &str) -> String {
    pref_key.trim().to_string()
}

pub(crate) fn retrieval_source_ref_for_kb_chunk(
    user_key: &str,
    namespace: &str,
    chunk_id: &str,
) -> String {
    format!(
        "kb:{}:{}:{}",
        user_key.trim(),
        namespace.trim(),
        chunk_id.trim()
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemoryWriteKind {
    Default,
    AssistantOutcome,
    UnfinishedGoal,
}

fn normalized_user_key_opt(user_key: Option<&str>) -> Option<&str> {
    user_key.map(str::trim).filter(|v| !v.is_empty())
}

fn effective_user_key(user_key: Option<&str>, user_id: i64, chat_id: i64) -> String {
    normalized_user_key_opt(user_key)
        .map(str::to_string)
        .unwrap_or_else(|| format!("anon:{user_id}:{chat_id}"))
}

fn legacy_principal_chat_id(user_key: &str, chat_id: i64) -> Option<i64> {
    let legacy = super::stable_i64_from_key(user_key);
    if legacy == chat_id {
        None
    } else {
        Some(legacy)
    }
}

fn query_recent_memories_for_chat(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    limit: usize,
) -> anyhow::Result<Vec<(String, String, String)>> {
    let mut stmt = db.prepare(
        "SELECT role, content, safety_flag
         FROM memories
         WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3
         ORDER BY id DESC
         LIMIT ?4",
    )?;
    let rows = stmt.query_map(params![user_id, chat_id, user_key, limit as i64], |row| {
        let role: String = row.get(0)?;
        let content: String = row.get(1)?;
        let safety_flag: String = row.get(2)?;
        Ok((role, content, safety_flag))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn query_preferences_for_chat(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    limit: usize,
) -> anyhow::Result<Vec<(String, String)>> {
    let mut stmt = db.prepare(
        "SELECT pref_key, pref_value
         FROM user_preferences
         WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3
         ORDER BY COALESCE(updated_at_ts, CAST(updated_at AS INTEGER)) DESC
         LIMIT ?4",
    )?;
    let rows = stmt.query_map(params![user_id, chat_id, user_key, limit as i64], |row| {
        let key: String = row.get(0)?;
        let value: String = row.get(1)?;
        Ok((key, value))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn query_memories_since_id_for_chat(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    source_memory_id: i64,
    limit: usize,
) -> anyhow::Result<Vec<(i64, String, String, String)>> {
    let mut stmt = db.prepare(
        "SELECT id, role, content, safety_flag
         FROM memories
         WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3 AND id > ?4
         ORDER BY id ASC
         LIMIT ?5",
    )?;
    let rows = stmt.query_map(
        params![user_id, chat_id, user_key, source_memory_id, limit as i64],
        |row| {
            let id: i64 = row.get(0)?;
            let role: String = row.get(1)?;
            let content: String = row.get(2)?;
            let safety_flag: String = row.get(3)?;
            Ok((id, role, content, safety_flag))
        },
    )?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub(crate) fn insert_memory(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    user_key: Option<&str>,
    channel: &str,
    external_chat_id: Option<&str>,
    role: &str,
    content: &str,
    max_chars: usize,
    write_kind: MemoryWriteKind,
) -> anyhow::Result<()> {
    let user_key = effective_user_key(user_key, user_id, chat_id);
    if content.trim().is_empty() {
        return Ok(());
    }
    let keep = max_chars.max(128);
    let mut normalized = content.trim().to_string();
    let file_tokens = extract_delivery_file_tokens(content);
    if !file_tokens.is_empty() {
        let merged = file_tokens.join("\n");
        if !normalized.contains(&merged) {
            normalized = format!("{merged}\n{normalized}");
        }
    }
    let trimmed = utf8_safe_prefix(&normalized, keep).to_string();
    let extracted_prefs = if role == MEMORY_ROLE_USER && state.memory.enable_preference_extraction {
        extract_user_preferences(content, &state.memory, crate::main_flow_rules(state))
    } else {
        Vec::new()
    };
    let should_skip = state.memory.write_filter_enabled
        && should_skip_memory_write(
            &trimmed,
            role,
            state.memory.write_min_chars.max(1),
            &state.memory,
        );
    if should_skip && extracted_prefs.is_empty() {
        return Ok(());
    }

    let safety_flag = classify_memory_safety_flag(&trimmed, &state.memory);
    let is_instructional = detect_instructional_text(&trimmed, &state.memory);
    let memory_type = infer_memory_type(role, is_instructional, &safety_flag, write_kind);
    let salience = estimate_memory_salience(
        &trimmed,
        is_instructional,
        &safety_flag,
        &state.memory,
        write_kind,
    );

    let now_text = now_ts();
    let now_ts_i64 = now_ts_u64() as i64;
    let db = state.db.lock().map_err(|_| anyhow!("db lock poisoned"))?;
    for (pref_key, pref_value, confidence, source) in &extracted_prefs {
        db.execute(
            "INSERT INTO user_preferences (user_id, chat_id, user_key, pref_key, pref_value, confidence, source, updated_at, updated_at_ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(user_id, chat_id, user_key, pref_key)
             DO UPDATE SET user_key=excluded.user_key, pref_value=excluded.pref_value, confidence=excluded.confidence, source=excluded.source, updated_at=excluded.updated_at, updated_at_ts=excluded.updated_at_ts",
            params![
                user_id,
                chat_id,
                &user_key,
                pref_key,
                pref_value,
                *confidence,
                source,
                now_text,
                now_ts_i64
            ],
        )?;
    }
    if state.memory.hybrid_recall_enabled && !extracted_prefs.is_empty() {
        let _ = indexing::index_preference_entries(
            &db,
            user_id,
            chat_id,
            &user_key,
            &extracted_prefs,
            now_ts_i64,
        );
    }
    if should_skip {
        return Ok(());
    }
    if is_duplicate_recent_memory(&db, user_id, chat_id, &user_key, role, &trimmed)? {
        return Ok(());
    }

    db.execute(
        "INSERT INTO memories (user_id, chat_id, user_key, channel, external_chat_id, role, content, created_at, created_at_ts, memory_type, salience, is_instructional, safety_flag)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            user_id,
            chat_id,
            user_key,
            channel,
            external_chat_id.map(str::trim).filter(|v| !v.is_empty()),
            role,
            trimmed,
            now_text,
            now_ts_i64,
            memory_type,
            salience,
            if is_instructional { 1 } else { 0 },
            safety_flag
        ],
    )?;
    if state.memory.hybrid_recall_enabled {
        let memory_id = db.last_insert_rowid();
        let _ = indexing::index_memory_row(
            &db,
            user_id,
            chat_id,
            &user_key,
            memory_id,
            role,
            &trimmed,
            memory_type,
            salience,
            is_instructional,
            now_ts_i64,
        );
    }
    Ok(())
}

pub(crate) fn count_chat_memory_rounds(
    state: &AppState,
    user_key: Option<&str>,
    user_id: i64,
    chat_id: i64,
) -> anyhow::Result<usize> {
    let user_key = effective_user_key(user_key, user_id, chat_id);
    let db = state.db.lock().map_err(|_| anyhow!("db lock poisoned"))?;
    let current_cnt: i64 = db.query_row(
        "SELECT COUNT(*) FROM memories WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3 AND role = 'user'",
        params![user_id, chat_id, user_key],
        |row| row.get(0),
    )?;
    if current_cnt > 0 {
        return Ok(current_cnt.max(0) as usize);
    }
    let Some(legacy_chat_id) = legacy_principal_chat_id(&user_key, chat_id) else {
        return Ok(0);
    };
    let legacy_cnt: i64 = db.query_row(
        "SELECT COUNT(*) FROM memories WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3 AND role = 'user'",
        params![user_id, legacy_chat_id, user_key],
        |row| row.get(0),
    )?;
    Ok(legacy_cnt.max(0) as usize)
}

pub(crate) fn recall_recent_memories(
    state: &AppState,
    user_key: Option<&str>,
    user_id: i64,
    chat_id: i64,
    limit: usize,
) -> anyhow::Result<Vec<(String, String)>> {
    let user_key = effective_user_key(user_key, user_id, chat_id);
    let db = state.db.lock().map_err(|_| anyhow!("db lock poisoned"))?;
    let rows = query_recent_memories_for_chat(&db, user_id, chat_id, &user_key, limit)?;
    let mut out = Vec::new();
    for (role, content, safety_flag) in rows {
        if state.memory.safety_filter_enabled && safety_flag == MEMORY_SAFETY_FLAG_INJECTION_LIKE {
            out.push((role, "[safety_signal content omitted]".to_string()));
            continue;
        }
        out.push((role, content));
    }
    if out.is_empty() {
        if let Some(legacy_chat_id) = legacy_principal_chat_id(&user_key, chat_id) {
            let rows =
                query_recent_memories_for_chat(&db, user_id, legacy_chat_id, &user_key, limit)?;
            for (role, content, safety_flag) in rows {
                if state.memory.safety_filter_enabled
                    && safety_flag == MEMORY_SAFETY_FLAG_INJECTION_LIKE
                {
                    out.push((role, "[safety_signal content omitted]".to_string()));
                    continue;
                }
                out.push((role, content));
            }
        }
    }
    out.reverse();
    Ok(out)
}

pub(crate) fn filter_memories_for_prompt_recall(
    memories: Vec<(String, String)>,
    prefer_llm_assistant_memory: bool,
) -> Vec<(String, String)> {
    if !prefer_llm_assistant_memory {
        return memories;
    }
    memories
        .into_iter()
        .filter(|(role, content)| {
            if role != MEMORY_ROLE_ASSISTANT {
                return true;
            }
            content.starts_with(LLM_SHORT_TERM_MEMORY_PREFIX)
        })
        .collect()
}

pub(crate) fn select_relevant_memories_for_prompt(
    memories: Vec<(String, String)>,
    prompt: &str,
    min_score: f32,
) -> Vec<(String, String)> {
    if memories.is_empty() {
        return memories;
    }
    let keywords = extract_recall_keywords(prompt);
    let source = memories;
    let mut out = Vec::new();
    for (role, content) in &source {
        let score = score_memory_relevance(role, content, &keywords);
        if score >= min_score {
            out.push((role.clone(), content.clone()));
        }
    }
    if out.is_empty() {
        let mut user_pick: Option<(String, String)> = None;
        for (role, content) in source.iter().rev() {
            if user_pick.is_none() && role == MEMORY_ROLE_USER {
                user_pick = Some((role.clone(), content.clone()));
                continue;
            }
            if user_pick.is_some() {
                break;
            }
        }
        if let Some(v) = user_pick {
            out.push(v);
        }
    }
    out
}

pub(crate) fn recall_user_preferences(
    state: &AppState,
    user_key: Option<&str>,
    user_id: i64,
    chat_id: i64,
    limit: usize,
) -> anyhow::Result<Vec<(String, String)>> {
    let user_key = effective_user_key(user_key, user_id, chat_id);
    let db = state.db.lock().map_err(|_| anyhow!("db lock poisoned"))?;
    let rows = query_preferences_for_chat(&db, user_id, chat_id, &user_key, limit)?;
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (key, value) in rows {
        if seen.insert(key.clone()) {
            out.push((key, value));
        }
    }
    if out.is_empty() {
        if let Some(legacy_chat_id) = legacy_principal_chat_id(&user_key, chat_id) {
            let rows = query_preferences_for_chat(&db, user_id, legacy_chat_id, &user_key, limit)?;
            for (key, value) in rows {
                if seen.insert(key.clone()) {
                    out.push((key, value));
                }
            }
        }
    }
    out.reverse();
    Ok(out)
}

pub(crate) fn recall_long_term_summary(
    state: &AppState,
    user_key: Option<&str>,
    user_id: i64,
    chat_id: i64,
) -> anyhow::Result<Option<String>> {
    let user_key = effective_user_key(user_key, user_id, chat_id);
    let db = state.db.lock().map_err(|_| anyhow!("db lock poisoned"))?;
    let summary = db
        .query_row(
            "SELECT summary FROM long_term_memories WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3",
            params![user_id, chat_id, user_key],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if summary.is_some() {
        return Ok(summary);
    }
    let Some(legacy_chat_id) = legacy_principal_chat_id(&user_key, chat_id) else {
        return Ok(None);
    };
    let legacy_summary = db
        .query_row(
            "SELECT summary FROM long_term_memories WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3",
            params![user_id, legacy_chat_id, user_key],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    Ok(legacy_summary)
}

pub(crate) fn recall_memories_since_id(
    state: &AppState,
    user_key: Option<&str>,
    user_id: i64,
    chat_id: i64,
    source_memory_id: i64,
    limit: usize,
) -> anyhow::Result<Vec<(i64, String, String, String)>> {
    let user_key = effective_user_key(user_key, user_id, chat_id);
    let db = state.db.lock().map_err(|_| anyhow!("db lock poisoned"))?;
    let mut out = query_memories_since_id_for_chat(
        &db,
        user_id,
        chat_id,
        &user_key,
        source_memory_id,
        limit,
    )?;
    if out.is_empty() {
        if let Some(legacy_chat_id) = legacy_principal_chat_id(&user_key, chat_id) {
            out = query_memories_since_id_for_chat(
                &db,
                user_id,
                legacy_chat_id,
                &user_key,
                source_memory_id,
                limit,
            )?;
        }
    }
    Ok(out)
}

fn extract_last_turn_user_text_from_payload(payload_json: &str) -> Option<String> {
    let payload = serde_json::from_str::<Value>(payload_json).ok()?;
    let text = payload.get("text").and_then(Value::as_str)?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn extract_last_turn_assistant_text_from_task(
    status: &str,
    result_json: Option<&str>,
    error_text: Option<&str>,
) -> Option<String> {
    if status.eq_ignore_ascii_case(RETRIEVAL_SUCCESS_STATE_FAILED) {
        if let Some(err) = error_text.map(str::trim).filter(|v| !v.is_empty()) {
            return Some(err.to_string());
        }
    }
    let result_json = result_json.map(str::trim).filter(|v| !v.is_empty())?;
    let parsed = serde_json::from_str::<Value>(result_json).ok();
    if let Some(val) = parsed.as_ref() {
        if let Some(text) = val.get("text").and_then(Value::as_str).map(str::trim) {
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
        if let Some(text) = val.as_str().map(str::trim) {
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }
    Some(result_json.to_string())
}

fn query_recent_terminal_ask_turn_for_chat(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
) -> anyhow::Result<Option<(String, String)>> {
    let mut stmt = db.prepare(
        "SELECT payload_json, result_json, error_text, status
         FROM tasks
         WHERE user_id = ?1
           AND chat_id = ?2
           AND user_key = ?3
           AND kind = 'ask'
           AND status IN ('succeeded', 'failed')
         ORDER BY CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) DESC
         LIMIT 8",
    )?;
    let rows = stmt.query_map(params![user_id, chat_id, user_key], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    for row in rows {
        let (payload_json, result_json, error_text, status) = row?;
        let Some(user_text) = extract_last_turn_user_text_from_payload(&payload_json) else {
            continue;
        };
        let Some(assistant_text) = extract_last_turn_assistant_text_from_task(
            &status,
            result_json.as_deref(),
            error_text.as_deref(),
        ) else {
            continue;
        };
        if assistant_text.trim().is_empty() {
            continue;
        }
        return Ok(Some((user_text, assistant_text)));
    }
    Ok(None)
}

fn query_recent_terminal_ask_turns_for_chat(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    limit: usize,
) -> anyhow::Result<Vec<(String, String)>> {
    let limit = limit.max(1).min(12);
    let mut stmt = db.prepare(
        "SELECT payload_json, result_json, error_text, status
         FROM tasks
         WHERE user_id = ?1
           AND chat_id = ?2
           AND user_key = ?3
           AND kind = 'ask'
           AND status IN ('succeeded', 'failed')
         ORDER BY CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) DESC
         LIMIT ?4",
    )?;
    let rows = stmt.query_map(params![user_id, chat_id, user_key, limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    let mut out: Vec<(String, String)> = Vec::new();
    for row in rows {
        let (payload_json, result_json, error_text, status) = row?;
        let Some(user_text) = extract_last_turn_user_text_from_payload(&payload_json) else {
            continue;
        };
        let Some(assistant_text) = extract_last_turn_assistant_text_from_task(
            &status,
            result_json.as_deref(),
            error_text.as_deref(),
        ) else {
            continue;
        };
        if assistant_text.trim().is_empty() {
            continue;
        }
        out.push((user_text, assistant_text));
    }
    Ok(out)
}

fn format_last_turn_full_context(
    state: &AppState,
    user_content: &str,
    assistant_content: &str,
    max_segment_chars: usize,
    max_total_chars: usize,
) -> String {
    let user_safety = classify_memory_safety_flag(user_content, &state.memory);
    let assistant_safety = classify_memory_safety_flag(assistant_content, &state.memory);
    let user_text =
        if state.memory.safety_filter_enabled && user_safety == MEMORY_SAFETY_FLAG_INJECTION_LIKE {
            "[safety_signal content omitted]".to_string()
        } else {
            utf8_safe_prefix(user_content.trim(), max_segment_chars).to_string()
        };
    let assistant_text = if state.memory.safety_filter_enabled
        && assistant_safety == MEMORY_SAFETY_FLAG_INJECTION_LIKE
    {
        "[safety_signal content omitted]".to_string()
    } else {
        utf8_safe_prefix(assistant_content.trim(), max_segment_chars).to_string()
    };
    let formatted = format!(
        "[LAST_TURN_FULL]\nUser: {}\nAssistant: {}\n[/LAST_TURN_FULL]",
        user_text, assistant_text
    );
    if formatted.len() > max_total_chars {
        let truncated = utf8_safe_prefix(&formatted, max_total_chars).to_string();
        if !truncated.ends_with("[/LAST_TURN_FULL]") {
            let mut out = truncated;
            if out.len() + 18 <= max_total_chars {
                out.push_str("[/LAST_TURN_FULL]");
            }
            out
        } else {
            truncated
        }
    } else {
        formatted
    }
}

pub(crate) fn build_recent_turns_full_context(
    state: &AppState,
    user_key: Option<&str>,
    user_id: i64,
    chat_id: i64,
    max_turns: usize,
    max_segment_chars: usize,
    max_total_chars: usize,
) -> String {
    let user_key = effective_user_key(user_key, user_id, chat_id);
    let db = match state.db.lock() {
        Ok(db) => db,
        Err(_) => return "<none>".to_string(),
    };
    let max_turns = max_turns.max(1).min(10);
    let max_segment_chars = max_segment_chars.max(128);
    let max_total_chars = max_total_chars.max(512);
    let mut turns =
        query_recent_terminal_ask_turns_for_chat(&db, user_id, chat_id, &user_key, max_turns)
            .unwrap_or_default();
    if turns.is_empty() {
        if let Some(legacy_chat_id) = legacy_principal_chat_id(&user_key, chat_id) {
            turns = query_recent_terminal_ask_turns_for_chat(
                &db,
                user_id,
                legacy_chat_id,
                &user_key,
                max_turns,
            )
            .unwrap_or_default();
        }
    }
    if turns.is_empty() {
        return "<none>".to_string();
    }
    let mut out = String::from("### RECENT_TURNS_FULL\n");
    for (idx, (user_text, assistant_text)) in turns.iter().enumerate() {
        let relative = -((idx as i64) + 1);
        let user_safety = classify_memory_safety_flag(user_text, &state.memory);
        let assistant_safety = classify_memory_safety_flag(assistant_text, &state.memory);
        let user_view = if state.memory.safety_filter_enabled
            && user_safety == MEMORY_SAFETY_FLAG_INJECTION_LIKE
        {
            "[safety_signal content omitted]".to_string()
        } else {
            utf8_safe_prefix(user_text.trim(), max_segment_chars).to_string()
        };
        let assistant_view = if state.memory.safety_filter_enabled
            && assistant_safety == MEMORY_SAFETY_FLAG_INJECTION_LIKE
        {
            "[safety_signal content omitted]".to_string()
        } else {
            utf8_safe_prefix(assistant_text.trim(), max_segment_chars).to_string()
        };
        let turn_block = format!(
            "[TURN {}]\nUser: {}\nAssistant: {}\n[/TURN]\n",
            relative, user_view, assistant_view
        );
        if out.len() + turn_block.len() > max_total_chars {
            break;
        }
        out.push_str(&turn_block);
    }
    if out.trim() == "### RECENT_TURNS_FULL" {
        "<none>".to_string()
    } else {
        out
    }
}

/// Build last turn full context: query the most recent user+assistant pair (one complete Q&A turn).
/// Returns formatted string with [LAST_TURN_FULL] tags, or "<none>" if not available.
/// Truncates each segment to max_segment_chars (default 1200) and total to max_total_chars (default 2400).
pub(crate) fn build_last_turn_full_context(
    state: &AppState,
    user_key: Option<&str>,
    user_id: i64,
    chat_id: i64,
    max_segment_chars: usize,
    max_total_chars: usize,
) -> String {
    let user_key = effective_user_key(user_key, user_id, chat_id);
    let db = match state.db.lock() {
        Ok(db) => db,
        Err(_) => return "<none>".to_string(),
    };
    if let Ok(Some((user_text, assistant_text))) =
        query_recent_terminal_ask_turn_for_chat(&db, user_id, chat_id, &user_key)
    {
        return format_last_turn_full_context(
            state,
            &user_text,
            &assistant_text,
            max_segment_chars,
            max_total_chars,
        );
    }
    if let Some(legacy_chat_id) = legacy_principal_chat_id(&user_key, chat_id) {
        if let Ok(Some((user_text, assistant_text))) =
            query_recent_terminal_ask_turn_for_chat(&db, user_id, legacy_chat_id, &user_key)
        {
            return format_last_turn_full_context(
                state,
                &user_text,
                &assistant_text,
                max_segment_chars,
                max_total_chars,
            );
        }
    }
    // Query recent memories, ordered by id DESC (most recent first)
    let recent = match query_recent_memories_for_chat(&db, user_id, chat_id, &user_key, 10) {
        Ok(v) => v,
        Err(_) => return "<none>".to_string(),
    };
    // Find the most recent assistant reply, then find the user message that precedes it
    // In DESC order (newest first): assistant comes first, then the user it replies to comes later in list
    let mut found_assistant: Option<(String, String)> = None; // (content, safety_flag)
    let mut found_user: Option<(String, String)> = None;
    for (role, content, safety_flag) in &recent {
        // First, find the most recent assistant reply
        if found_assistant.is_none() && role == MEMORY_ROLE_ASSISTANT {
            found_assistant = Some((content.clone(), safety_flag.clone()));
            continue;
        }
        // After finding assistant, look for the user message that precedes it (comes later in DESC list = older)
        if found_assistant.is_some() && found_user.is_none() && role == MEMORY_ROLE_USER {
            found_user = Some((content.clone(), safety_flag.clone()));
            break;
        }
    }
    // Need both user and assistant to form a complete turn
    let (user_content, _) = match found_user {
        Some(v) => v,
        None => return "<none>".to_string(),
    };
    let (assistant_content, _) = match found_assistant {
        Some(v) => v,
        None => return "<none>".to_string(),
    };
    format_last_turn_full_context(
        state,
        &user_content,
        &assistant_content,
        max_segment_chars,
        max_total_chars,
    )
}

/// Build a compact recent assistant-replies block for ordinal follow-up anchoring.
/// Output format:
/// ### RECENT_ASSISTANT_REPLIES
/// - turn_id=assistant[-1] relative_index=-1 short_preview=... has_code_block=true|false
pub(crate) fn build_recent_assistant_replies_context(
    state: &AppState,
    user_key: Option<&str>,
    user_id: i64,
    chat_id: i64,
    max_replies: usize,
    preview_chars: usize,
) -> String {
    let max_replies = max_replies.max(1);
    let preview_chars = preview_chars.max(48);
    let user_key = effective_user_key(user_key, user_id, chat_id);
    let db = match state.db.lock() {
        Ok(db) => db,
        Err(_) => return "<none>".to_string(),
    };

    let mut rows =
        query_recent_memories_for_chat(&db, user_id, chat_id, &user_key, max_replies * 6)
            .unwrap_or_default();
    if rows.is_empty() {
        if let Some(legacy_chat_id) = legacy_principal_chat_id(&user_key, chat_id) {
            rows = query_recent_memories_for_chat(
                &db,
                user_id,
                legacy_chat_id,
                &user_key,
                max_replies * 6,
            )
            .unwrap_or_default();
        }
    }
    if rows.is_empty() {
        return "<none>".to_string();
    }

    let mut lines: Vec<String> = Vec::new();
    for (role, content, safety_flag) in rows {
        if role != MEMORY_ROLE_ASSISTANT {
            continue;
        }
        if state.memory.safety_filter_enabled && safety_flag == MEMORY_SAFETY_FLAG_INJECTION_LIKE {
            continue;
        }
        let reply_index = lines.len() + 1;
        let relative_index = -(reply_index as i64);
        let preview = utf8_safe_prefix(content.trim(), preview_chars)
            .replace('\n', " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if preview.is_empty() {
            continue;
        }
        let has_code_block = if content.contains("```") {
            "true"
        } else {
            "false"
        };
        lines.push(format!(
            "- turn_id=assistant[{}] relative_index={} short_preview={} has_code_block={}",
            relative_index, relative_index, preview, has_code_block
        ));
        if lines.len() >= max_replies {
            break;
        }
    }

    if lines.is_empty() {
        "<none>".to_string()
    } else {
        format!("### RECENT_ASSISTANT_REPLIES\n{}", lines.join("\n"))
    }
}

pub(crate) fn read_long_term_source_memory_id(
    state: &AppState,
    user_key: Option<&str>,
    user_id: i64,
    chat_id: i64,
) -> anyhow::Result<i64> {
    let user_key = effective_user_key(user_key, user_id, chat_id);
    let db = state.db.lock().map_err(|_| anyhow!("db lock poisoned"))?;
    let source = db
        .query_row(
            "SELECT source_memory_id FROM long_term_memories WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3",
            params![user_id, chat_id, user_key],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if let Some(source) = source {
        return Ok(source);
    }
    let Some(legacy_chat_id) = legacy_principal_chat_id(&user_key, chat_id) else {
        return Ok(0);
    };
    let legacy_source = db
        .query_row(
            "SELECT source_memory_id FROM long_term_memories WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3",
            params![user_id, legacy_chat_id, user_key],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    Ok(legacy_source.unwrap_or(0))
}

pub(crate) fn upsert_long_term_summary(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    user_key: Option<&str>,
    summary: &str,
    source_memory_id: i64,
) -> anyhow::Result<()> {
    let user_key = effective_user_key(user_key, user_id, chat_id);
    let db = state.db.lock().map_err(|_| anyhow!("db lock poisoned"))?;
    let now = now_ts();
    let now_ts_i64 = now_ts_u64() as i64;
    db.execute(
        "INSERT INTO long_term_memories (user_id, chat_id, user_key, summary, source_memory_id, created_at, updated_at, created_at_ts, updated_at_ts)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?7)
         ON CONFLICT(user_id, chat_id, user_key)
         DO UPDATE SET user_key = excluded.user_key, summary = excluded.summary, source_memory_id = excluded.source_memory_id, updated_at = excluded.updated_at, updated_at_ts = excluded.updated_at_ts",
        params![user_id, chat_id, user_key, summary, source_memory_id, now, now_ts_i64],
    )?;
    Ok(())
}

fn sanitize_memory_text_for_prompt(text: &str) -> String {
    let policy_markers = [
        "no code generation",
        "all code generation is forbidden",
        "all executable code is forbidden",
        "pure text only",
        "only pure-text",
        "不能提供可执行代码",
        "禁止生成任何可执行代码",
        "不提供java代码示例",
        "不提供可执行的java",
        "根据当前策略，不能提供可执行代码",
        "当前策略明确禁止",
        "including teaching examples",
    ];
    let kept = text
        .lines()
        .filter(|line| {
            let lower = line.trim().to_ascii_lowercase();
            !policy_markers.iter().any(|m| lower.contains(m))
        })
        .collect::<Vec<_>>();
    kept.join("\n").trim().to_string()
}

pub(crate) fn repeated_entries_ratio(entries: &[(i64, String, String, String)]) -> f32 {
    if entries.is_empty() {
        return 0.0;
    }
    let mut uniq = HashSet::new();
    for (_, role, content, _) in entries {
        let normalized = format!(
            "{}:{}",
            role,
            content
                .trim()
                .to_ascii_lowercase()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        );
        uniq.insert(normalized);
    }
    let unique = uniq.len() as f32;
    let total = entries.len() as f32;
    (1.0 - unique / total).clamp(0.0, 1.0)
}

fn contains_any_marker(norm_text: &str, markers: &[String]) -> bool {
    markers.iter().any(|m| {
        let token = m.trim();
        !token.is_empty() && norm_text.contains(&token.to_ascii_lowercase())
    })
}

fn should_skip_memory_write(
    content: &str,
    role: &str,
    min_chars: usize,
    cfg: &MemoryConfig,
) -> bool {
    let text = content.trim();
    if text.is_empty() {
        return true;
    }
    if text.chars().count() < min_chars && extract_delivery_file_tokens(text).is_empty() {
        return true;
    }
    let tiny = text.to_ascii_lowercase();
    if role == MEMORY_ROLE_ASSISTANT
        && cfg
            .rules
            .assistant_ack_skip
            .iter()
            .any(|m| tiny == m.trim().to_ascii_lowercase())
    {
        return true;
    }
    false
}

fn is_duplicate_recent_memory(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    role: &str,
    content: &str,
) -> anyhow::Result<bool> {
    let last: Option<String> = db
        .query_row(
            "SELECT content
             FROM memories
             WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3 AND role = ?4
             ORDER BY id DESC
             LIMIT 1",
            params![user_id, chat_id, user_key, role],
            |row| row.get(0),
        )
        .optional()?;
    Ok(last.is_some_and(|v| v.trim() == content.trim()))
}

fn detect_instructional_text(text: &str, cfg: &MemoryConfig) -> bool {
    let norm = text.to_ascii_lowercase();
    contains_any_marker(&norm, &cfg.rules.instruction_markers)
}

fn classify_memory_safety_flag(text: &str, cfg: &MemoryConfig) -> String {
    let norm = text.to_ascii_lowercase();
    if contains_any_marker(&norm, &cfg.rules.injection_markers) {
        return MEMORY_SAFETY_FLAG_INJECTION_LIKE.to_string();
    }
    MEMORY_SAFETY_FLAG_NORMAL.to_string()
}

fn infer_memory_type(
    role: &str,
    is_instructional: bool,
    safety_flag: &str,
    write_kind: MemoryWriteKind,
) -> &'static str {
    if safety_flag == MEMORY_SAFETY_FLAG_INJECTION_LIKE {
        return MEMORY_TYPE_SAFETY_SIGNAL;
    }
    if matches!(write_kind, MemoryWriteKind::UnfinishedGoal) {
        return MEMORY_TYPE_UNFINISHED_GOAL;
    }
    if matches!(write_kind, MemoryWriteKind::AssistantOutcome) {
        return MEMORY_TYPE_ASSISTANT_OUTCOME;
    }
    if role == MEMORY_ROLE_ASSISTANT {
        return MEMORY_TYPE_ASSISTANT_REPLY;
    }
    if is_instructional {
        return MEMORY_TYPE_USER_INSTRUCTION;
    }
    MEMORY_TYPE_GENERIC
}

fn estimate_memory_salience(
    text: &str,
    is_instructional: bool,
    safety_flag: &str,
    cfg: &MemoryConfig,
    write_kind: MemoryWriteKind,
) -> f32 {
    let mut score: f32 = if is_instructional { 0.72 } else { 0.48 };
    match write_kind {
        MemoryWriteKind::AssistantOutcome => score += 0.12,
        MemoryWriteKind::UnfinishedGoal => score += 0.2,
        MemoryWriteKind::Default => {}
    }
    if contains_any_marker(
        &text.to_ascii_lowercase(),
        &cfg.rules.salience_boost_markers,
    ) {
        score += 0.16;
    }
    if safety_flag == MEMORY_SAFETY_FLAG_INJECTION_LIKE {
        score = 0.12;
    }
    score.clamp(0.0, 1.0)
}

fn extract_user_preferences(
    content: &str,
    cfg: &MemoryConfig,
    rules: &claw_core::hard_rules::types::MainFlowRules,
) -> Vec<(String, String, f32, String)> {
    let mut out = Vec::new();
    let norm = content.to_ascii_lowercase();
    let pref = &cfg.rules.preferences;
    if contains_any_marker(&norm, &pref.language_zh) {
        out.push((
            "response_language".to_string(),
            "zh-CN".to_string(),
            0.96,
            "rule_extract".to_string(),
        ));
    }

    if contains_any_marker(&norm, &pref.language_en) {
        out.push((
            "response_language".to_string(),
            "en-US".to_string(),
            0.96,
            "rule_extract".to_string(),
        ));
    }

    if contains_any_marker(&norm, &pref.style_concise) {
        out.push((
            "response_style".to_string(),
            "concise".to_string(),
            0.8,
            "rule_extract".to_string(),
        ));
    }
    if contains_any_marker(&norm, &pref.style_detailed) {
        out.push((
            "response_style".to_string(),
            "detailed".to_string(),
            0.8,
            "rule_extract".to_string(),
        ));
    }

    if contains_any_marker(&norm, &pref.format_plain_text) {
        out.push((
            "response_format".to_string(),
            "plain_text".to_string(),
            0.84,
            "rule_extract".to_string(),
        ));
    }
    if let Some(name) = extract_agent_display_name(content, rules) {
        out.push((
            "agent_display_name".to_string(),
            name,
            0.78,
            "assistant_name_extract".to_string(),
        ));
    }
    out
}

fn extract_agent_display_name(
    content: &str,
    rules: &claw_core::hard_rules::types::MainFlowRules,
) -> Option<String> {
    let text = content.trim();
    if text.is_empty() {
        return None;
    }
    let lower = text.to_ascii_lowercase();
    for marker in &rules.assistant_name_extract_markers {
        let pos = if marker.is_ascii() {
            lower.find(&marker.to_ascii_lowercase())
        } else {
            text.find(marker)
        };
        let Some(pos) = pos else {
            continue;
        };
        let start = pos + marker.len();
        let tail = &text[start..];
        let candidate = tail
            .split([
                '\n', '\r', '，', ',', '。', '.', '！', '!', '？', '?', '；', ';', '（', '）', '(',
                ')',
            ])
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches(|c| c == '"' || c == '\'' || c == '“' || c == '”' || c == '‘' || c == '’')
            .trim();
        if candidate.is_empty() {
            continue;
        }
        let candidate = candidate.split_whitespace().collect::<Vec<_>>().join(" ");
        if rules
            .assistant_name_invalid_values
            .iter()
            .any(|v| candidate.eq_ignore_ascii_case(v) || candidate.contains(v))
        {
            continue;
        }
        let char_count = candidate.chars().count();
        if (1..=24).contains(&char_count) {
            return Some(candidate);
        }
    }
    None
}

fn extract_recall_keywords(prompt: &str) -> Vec<String> {
    let lower = prompt.to_ascii_lowercase();
    let mut out = lower
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_string())
        .collect::<Vec<_>>();
    let cjk = prompt
        .chars()
        .filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c))
        .collect::<String>();
    let chars = cjk.chars().collect::<Vec<_>>();
    for w in chars.windows(2).take(10) {
        out.push(w.iter().collect::<String>());
    }
    out.sort();
    out.dedup();
    out
}

fn score_memory_relevance(role: &str, content: &str, keywords: &[String]) -> f32 {
    let mut score = if role == MEMORY_ROLE_USER { 0.1 } else { 0.05 };
    let text = content.to_ascii_lowercase();
    let mut hits = 0usize;
    for kw in keywords {
        if kw.len() <= 1 {
            continue;
        }
        if text.contains(kw) || content.contains(kw) {
            hits += 1;
        }
    }
    score += (hits.min(5) as f32) * 0.12;
    if content.starts_with(LLM_SHORT_TERM_MEMORY_PREFIX) {
        score += 0.04;
    }
    score.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::{
        retrieval_source_ref_for_kb_chunk, retrieval_source_ref_for_memory,
        retrieval_source_ref_for_preference, RETRIEVAL_PRODUCER_KB,
        RETRIEVAL_PRODUCER_MEMORY_PIPELINE, RETRIEVAL_SOURCE_MEMORY,
    };

    #[test]
    fn retrieval_source_ref_for_memory_is_stable_id_string() {
        assert_eq!(retrieval_source_ref_for_memory(42), "42");
    }

    #[test]
    fn retrieval_source_ref_for_preference_uses_trimmed_pref_key() {
        assert_eq!(
            retrieval_source_ref_for_preference(" response_language "),
            "response_language"
        );
    }

    #[test]
    fn retrieval_source_ref_for_kb_chunk_is_chunk_scoped() {
        assert_eq!(
            retrieval_source_ref_for_kb_chunk("user:test", "docs", "chunk-001"),
            "kb:user:test:docs:chunk-001"
        );
    }

    #[test]
    fn retrieval_producer_constants_match_pipeline_intent() {
        assert_eq!(RETRIEVAL_PRODUCER_KB, "kb");
        assert_eq!(RETRIEVAL_PRODUCER_MEMORY_PIPELINE, RETRIEVAL_SOURCE_MEMORY);
    }
}
