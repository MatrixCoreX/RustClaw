use std::collections::HashSet;

pub(crate) mod api;
pub(crate) mod apply;
pub(crate) mod embedding;
pub(crate) mod facts;
pub(crate) mod indexing;
pub(crate) mod intent;
#[path = "memory_recent.rs"]
mod memory_recent;
pub(crate) mod retrieval;
pub(crate) mod service;
pub(crate) mod use_policy;

pub(super) use memory_recent::is_transient_assistant_context_text_basic;
pub(crate) use memory_recent::{
    build_last_turn_full_context, build_recent_assistant_replies_context,
    build_recent_turns_full_context_with_sources,
};
pub(crate) use service::dynamic_chat_memory_budget_chars;

use memory_recent::is_transient_assistant_context_text;

#[cfg(test)]
use memory_recent::{
    clarify_assistant_placeholder, classify_assistant_context_reply_kind,
    extract_result_text_for_recent_turns, ordered_entries_from_assistant_reply,
    provider_unavailable_assistant_placeholder, AssistantContextReplyKind,
};

use anyhow::anyhow;
use claw_core::config::MemoryConfig;
use rusqlite::{params, Connection, OptionalExtension};
use tracing::warn;

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
pub(crate) const RETRIEVAL_SOURCE_MEMORY_FACT: &str = "memory_fact";

pub(crate) const MEMORY_FACT_STATUS_ACTIVE: &str = "active";
pub(crate) const MEMORY_FACT_STATUS_SUPERSEDED: &str = "superseded";
pub(crate) const MEMORY_FACT_STATUS_EXPIRED: &str = "expired";
pub(crate) const MEMORY_FACT_STATUS_DELETED: &str = "deleted";

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
#[cfg(test)]
pub(crate) const RETRIEVAL_PRODUCER_KB: &str = "kb";
pub(crate) const RETRIEVAL_PRODUCER_MEMORY_PIPELINE: &str = RETRIEVAL_SOURCE_MEMORY;
const AGENT_DISPLAY_NAME_RESERVED_TOKENS: &[&str] = &[
    MEMORY_ROLE_SYSTEM,
    MEMORY_ROLE_ASSISTANT,
    MEMORY_ROLE_USER,
    "agent",
    "executor",
];

pub(crate) fn retrieval_source_is_knowledge(source_kind: &str) -> bool {
    matches!(
        source_kind,
        RETRIEVAL_SOURCE_KB_DOC | RETRIEVAL_SOURCE_KNOWLEDGE_FACT | RETRIEVAL_SOURCE_MEMORY_FACT
    )
}

pub(crate) fn retrieval_kind_is_fact_bucket(memory_kind: &str) -> bool {
    memory_kind == RETRIEVAL_KIND_SEMANTIC_FACT
}

pub(crate) fn retrieval_kind_is_knowledge_doc_bucket(memory_kind: &str) -> bool {
    memory_kind == RETRIEVAL_KIND_KNOWLEDGE_DOC
}

pub(crate) fn fact_uses_cross_turn_deictic_locator(text: &str) -> bool {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(text);
    (surface.has_deictic_reference() && surface.has_concrete_locator_hint())
        || looks_like_structured_transient_mapping(text)
        || looks_like_short_plain_locator_fact(text)
}

fn looks_like_structured_transient_mapping(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let has_mapping_delimiter = trimmed
        .chars()
        .any(|ch| matches!(ch, ':' | '：' | '=' | '→' | '⇒'));
    if !has_mapping_delimiter || quoted_span_count(trimmed) < 2 {
        return false;
    }
    trimmed.chars().count() <= 240
}

fn looks_like_short_plain_locator_fact(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('{')
        || trimmed.lines().count() > 3
        || trimmed.chars().count() > 320
    {
        return false;
    }
    crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(trimmed)
        .into_iter()
        .any(|locator| {
            matches!(locator.locator_kind, crate::OutputLocatorKind::Path)
                && locator_hint_looks_like_local_filesystem_path(&locator.locator_hint)
        })
}

fn locator_hint_looks_like_local_filesystem_path(locator_hint: &str) -> bool {
    let hint = locator_hint.trim();
    if hint.starts_with('/')
        || hint.starts_with("./")
        || hint.starts_with("../")
        || hint.starts_with("~/")
        || hint.contains('\\')
    {
        return true;
    }
    hint.rsplit_once('/').is_some_and(|(_, tail)| {
        tail.contains('.') && tail.chars().any(|ch| ch.is_ascii_alphabetic())
    })
}

fn quoted_span_count(text: &str) -> usize {
    let mut count = 0usize;
    let mut active: Option<char> = None;
    for ch in text.chars() {
        match active {
            Some(end) if ch == end => {
                count += 1;
                active = None;
            }
            Some(_) => {}
            None => {
                active = match ch {
                    '\'' => Some('\''),
                    '"' => Some('"'),
                    '`' => Some('`'),
                    '“' => Some('”'),
                    '‘' => Some('’'),
                    '「' => Some('」'),
                    '『' => Some('』'),
                    _ => None,
                };
            }
        }
    }
    count
}

pub(crate) fn retrieval_source_ref_for_memory(memory_id: i64) -> String {
    memory_id.to_string()
}

pub(crate) fn retrieval_source_ref_for_preference(pref_key: &str) -> String {
    pref_key.trim().to_string()
}

#[cfg(test)]
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

pub(crate) fn retrieval_source_ref_for_memory_fact(fact_id: i64) -> String {
    format!("memory_fact:{fact_id}")
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
           AND memory_type != ?5
         ORDER BY id ASC
         LIMIT ?6",
    )?;
    let rows = stmt.query_map(
        params![
            user_id,
            chat_id,
            user_key,
            source_memory_id,
            MEMORY_TYPE_UNFINISHED_GOAL,
            limit as i64
        ],
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
    let should_skip = state.policy.memory.write_filter_enabled
        && should_skip_memory_write(
            &trimmed,
            role,
            state.policy.memory.write_min_chars.max(1),
            &state.policy.memory,
        );
    if should_skip {
        return Ok(());
    }

    let safety_flag = MEMORY_SAFETY_FLAG_NORMAL.to_string();
    let is_instructional = false;
    let memory_type = infer_memory_type(role, is_instructional, &safety_flag, write_kind);
    let salience = estimate_memory_salience(&trimmed, is_instructional, &safety_flag, write_kind);

    let now_text = now_ts();
    let now_ts_i64 = now_ts_u64() as i64;
    let db = state.core.db.get().map_err(|e| anyhow!("db pool: {e}"))?;
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
    if state.policy.memory.hybrid_recall_enabled {
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

const MEMORY_INTENT_EXTRACT_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/layers/overlays/memory_intent_extract_prompt.md");
const MEMORY_INTENT_SCHEMA_TEXT: &str =
    include_str!("../../../prompts/schemas/memory_intent.schema.json");

pub(crate) async fn maybe_extract_memory_intent_with_llm(
    state: &AppState,
    task: &crate::ClaimedTask,
    content: &str,
) -> anyhow::Result<()> {
    let cfg = &state.policy.memory;
    if !cfg.enable_preference_extraction || !cfg.llm_preference_fallback_enabled {
        return Ok(());
    }
    let trimmed = content.trim();
    if trimmed.chars().count() < cfg.write_min_chars.max(8) {
        return Ok(());
    }

    let prompt_text = utf8_safe_prefix(trimmed, cfg.llm_preference_max_chars.max(128));
    let source_ref = format!("task:{}:user", task.task_id);
    let prompt = build_memory_intent_llm_prompt(prompt_text, &source_ref);
    let raw = match crate::llm_gateway::run_with_fallback_with_prompt_source(
        state,
        task,
        &prompt,
        "memory_intent_extract",
    )
    .await
    {
        Ok(raw) => raw,
        Err(err) => {
            warn!(
                "memory intent llm extraction failed task_id={} err={}",
                task.task_id, err
            );
            return Ok(());
        }
    };

    let intent = match intent::parse_memory_intent_schema(&raw) {
        Ok(intent) => intent,
        Err(err) => {
            warn!(
                "memory intent schema validation failed task_id={} err={}",
                task.task_id, err
            );
            return Ok(());
        }
    };
    let validation =
        intent::validate_memory_intent_actions(intent, cfg.llm_preference_min_confidence);
    if !validation.rejected.is_empty() {
        warn!(
            "memory intent rejected actions task_id={} rejected_count={} first_reason={}",
            task.task_id,
            validation.rejected.len(),
            validation
                .rejected
                .first()
                .map(|item| item.reason.as_str())
                .unwrap_or("unknown")
        );
    }
    if validation.accepted.is_empty() {
        return Ok(());
    }

    let user_key = effective_user_key(task.user_key.as_deref(), task.user_id, task.chat_id);
    let now_text = now_ts();
    let now_ts_i64 = now_ts_u64() as i64;
    let db = state.core.db.get().map_err(|e| anyhow!("db pool: {e}"))?;
    let stats = apply::apply_memory_actions(
        state,
        &db,
        task.user_id,
        task.chat_id,
        &user_key,
        &validation.accepted,
        &now_text,
        now_ts_i64,
    )?;
    if stats.upserted_preferences > 0
        || stats.deleted_preferences > 0
        || stats.marked_safety_signals > 0
        || stats.ignored_actions > 0
    {
        tracing::info!(
            "memory intent applied task_id={} upserted_preferences={} deleted_preferences={} marked_safety_signals={} ignored_actions={}",
            task.task_id,
            stats.upserted_preferences,
            stats.deleted_preferences,
            stats.marked_safety_signals,
            stats.ignored_actions
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
    let db = state.core.db.get().map_err(|e| anyhow!("db pool: {e}"))?;
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
    let db = state.core.db.get().map_err(|e| anyhow!("db pool: {e}"))?;
    let rows = query_recent_memories_for_chat(&db, user_id, chat_id, &user_key, limit)?;
    let mut out = Vec::new();
    for (role, content, safety_flag) in rows {
        if state.policy.memory.safety_filter_enabled
            && safety_flag == MEMORY_SAFETY_FLAG_INJECTION_LIKE
        {
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
                if state.policy.memory.safety_filter_enabled
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
    // Memory recall uses lightweight lexical terms only to rank already-stored memories.
    // It must not become an intent router, planner shortcut, or response-shaping rule.
    let recall_terms = extract_recall_terms(prompt);
    let source = memories;
    let mut out = Vec::new();
    for (role, content) in &source {
        let score = score_memory_relevance(role, content, &recall_terms);
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
    let db = state.core.db.get().map_err(|e| anyhow!("db pool: {e}"))?;
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
    let db = state.core.db.get().map_err(|e| anyhow!("db pool: {e}"))?;
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
    let db = state.core.db.get().map_err(|e| anyhow!("db pool: {e}"))?;
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
    out.retain(|(_, role, content, _)| {
        role != MEMORY_ROLE_ASSISTANT || !is_transient_assistant_context_text(state, content)
    });
    Ok(out)
}

pub(crate) fn read_long_term_source_memory_id(
    state: &AppState,
    user_key: Option<&str>,
    user_id: i64,
    chat_id: i64,
) -> anyhow::Result<i64> {
    let user_key = effective_user_key(user_key, user_id, chat_id);
    let db = state.core.db.get().map_err(|e| anyhow!("db pool: {e}"))?;
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
    let db = state.core.db.get().map_err(|e| anyhow!("db pool: {e}"))?;
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
    text.trim().to_string()
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

fn should_skip_memory_write(
    content: &str,
    _role: &str,
    min_chars: usize,
    _cfg: &MemoryConfig,
) -> bool {
    let text = content.trim();
    if text.is_empty() {
        return true;
    }
    if text.chars().count() < min_chars && extract_delivery_file_tokens(text).is_empty() {
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
    _text: &str,
    is_instructional: bool,
    safety_flag: &str,
    write_kind: MemoryWriteKind,
) -> f32 {
    let mut score: f32 = if is_instructional { 0.72 } else { 0.48 };
    match write_kind {
        MemoryWriteKind::AssistantOutcome => score += 0.12,
        MemoryWriteKind::UnfinishedGoal => score += 0.2,
        MemoryWriteKind::Default => {}
    }
    if safety_flag == MEMORY_SAFETY_FLAG_INJECTION_LIKE {
        score = 0.12;
    }
    score.clamp(0.0, 1.0)
}

fn build_memory_intent_llm_prompt(content: &str, source_ref: &str) -> String {
    let content = content.replace("```", "'''");
    crate::prompt_utils::render_prompt_template(
        MEMORY_INTENT_EXTRACT_PROMPT_TEMPLATE,
        &[
            ("__MEMORY_INTENT_SCHEMA__", MEMORY_INTENT_SCHEMA_TEXT),
            ("__SOURCE_REF__", source_ref),
            ("__USER_TEXT__", &content),
        ],
    )
}

fn normalize_agent_display_name(raw: &str) -> Option<String> {
    let candidate = raw
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '“' || c == '”' || c == '‘' || c == '’')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if candidate.is_empty() {
        return None;
    }
    if AGENT_DISPLAY_NAME_RESERVED_TOKENS
        .iter()
        .any(|value| candidate.eq_ignore_ascii_case(value))
        || candidate.chars().any(char::is_control)
        || candidate.starts_with('{')
        || candidate.starts_with('[')
        || candidate.contains('/')
        || candidate.contains('\\')
    {
        return None;
    }
    let char_count = candidate.chars().count();
    ((1..=24).contains(&char_count)).then_some(candidate)
}

fn structured_user_preferences_from_route_hint(
    agent_display_name_hint: &str,
) -> Vec<(String, String, f32, String)> {
    let mut prefs = Vec::new();
    if let Some(name) = normalize_agent_display_name(agent_display_name_hint) {
        prefs.push((
            "agent_display_name".to_string(),
            name,
            0.92,
            "route_semantic_extract".to_string(),
        ));
    }
    prefs
}

fn upsert_user_preferences(
    state: &AppState,
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    extracted_prefs: &[(String, String, f32, String)],
    now_text: &str,
    now_ts_i64: i64,
) -> anyhow::Result<()> {
    for (pref_key, pref_value, confidence, source) in extracted_prefs {
        db.execute(
            "INSERT INTO user_preferences (user_id, chat_id, user_key, pref_key, pref_value, confidence, source, updated_at, updated_at_ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(user_id, chat_id, user_key, pref_key)
             DO UPDATE SET user_key=excluded.user_key, pref_value=excluded.pref_value, confidence=excluded.confidence, source=excluded.source, updated_at=excluded.updated_at, updated_at_ts=excluded.updated_at_ts",
            params![
                user_id,
                chat_id,
                user_key,
                pref_key,
                pref_value,
                *confidence,
                source,
                now_text,
                now_ts_i64
            ],
        )?;
    }
    if state.policy.memory.hybrid_recall_enabled && !extracted_prefs.is_empty() {
        let _ = indexing::index_preference_entries(
            db,
            user_id,
            chat_id,
            user_key,
            extracted_prefs,
            now_ts_i64,
        );
    }
    Ok(())
}

pub(crate) fn upsert_user_preferences_from_route_hint(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    user_key: Option<&str>,
    agent_display_name_hint: &str,
) -> anyhow::Result<()> {
    let extracted_prefs = structured_user_preferences_from_route_hint(agent_display_name_hint);
    if extracted_prefs.is_empty() {
        return Ok(());
    }
    let user_key = effective_user_key(user_key, user_id, chat_id);
    let now_text = now_ts();
    let now_ts_i64 = now_ts_u64() as i64;
    let db = state.core.db.get().map_err(|e| anyhow!("db pool: {e}"))?;
    upsert_user_preferences(
        state,
        &db,
        user_id,
        chat_id,
        &user_key,
        &extracted_prefs,
        &now_text,
        now_ts_i64,
    )
}

fn extract_recall_terms(prompt: &str) -> Vec<String> {
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

fn score_memory_relevance(role: &str, content: &str, recall_terms: &[String]) -> f32 {
    let mut score = if role == MEMORY_ROLE_USER { 0.1 } else { 0.05 };
    let text = content.to_ascii_lowercase();
    let mut hits = 0usize;
    for term in recall_terms {
        if term.len() <= 1 {
            continue;
        }
        if text.contains(term) || content.contains(term) {
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
#[path = "memory_tests.rs"]
mod tests;
