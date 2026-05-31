use std::collections::HashSet;

pub(crate) mod api;
pub(crate) mod apply;
pub(crate) mod embedding;
pub(crate) mod facts;
pub(crate) mod indexing;
pub(crate) mod intent;
pub(crate) mod retrieval;
pub(crate) mod service;
pub(crate) mod use_policy;

pub(crate) use service::dynamic_chat_memory_budget_chars;

use anyhow::anyhow;
use claw_core::config::MemoryConfig;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
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
#[allow(dead_code)]
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
pub(crate) const RETRIEVAL_PRODUCER_KB: &str = "kb";
pub(crate) const RETRIEVAL_PRODUCER_MEMORY_PIPELINE: &str = RETRIEVAL_SOURCE_MEMORY;
const AGENT_DISPLAY_NAME_INVALID_VALUES: &[&str] = &[
    "executor",
    "assistant",
    "agent",
    "系统",
    "身份",
    "formal identity",
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
    surface.has_deictic_reference() && surface.has_concrete_locator_hint()
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

fn strip_llm_reply_memory_prefix(text: &str) -> &str {
    text.trim()
        .strip_prefix(LLM_SHORT_TERM_MEMORY_PREFIX)
        .unwrap_or_else(|| text.trim())
        .trim()
}

pub(super) fn is_transient_assistant_context_text_basic(text: &str) -> bool {
    let trimmed = strip_llm_reply_memory_prefix(text);
    trimmed.is_empty()
        || trimmed == provider_unavailable_assistant_placeholder()
        || trimmed == clarify_assistant_placeholder()
        || crate::finalize::is_execution_summary_message(trimmed)
}

fn is_transient_assistant_context_text(state: &AppState, text: &str) -> bool {
    let trimmed = strip_llm_reply_memory_prefix(text);
    is_transient_assistant_context_text_basic(trimmed)
        || crate::fallback::is_known_clarify_fallback_text(state, trimmed)
}

fn assistant_context_text_for_recall<'a>(state: &AppState, text: &'a str) -> Option<&'a str> {
    if is_transient_assistant_context_text(state, text) {
        None
    } else {
        Some(strip_llm_reply_memory_prefix(text))
    }
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

    let safety_flag = classify_memory_safety_flag(&trimmed, &state.policy.memory);
    let is_instructional = detect_instructional_text(&trimmed, &state.policy.memory);
    let memory_type = infer_memory_type(role, is_instructional, &safety_flag, write_kind);
    let salience = estimate_memory_salience(
        &trimmed,
        is_instructional,
        &safety_flag,
        &state.policy.memory,
        write_kind,
    );

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
    if stats.upserted_preferences > 0 || stats.deleted_preferences > 0 || stats.ignored_actions > 0
    {
        tracing::info!(
            "memory intent applied task_id={} upserted_preferences={} deleted_preferences={} ignored_actions={}",
            task.task_id,
            stats.upserted_preferences,
            stats.deleted_preferences,
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

fn extract_last_turn_user_text_from_payload(payload_json: &str) -> Option<String> {
    let payload = serde_json::from_str::<Value>(payload_json).ok()?;
    let text = payload.get("text").and_then(Value::as_str)?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssistantContextReplyKind {
    Normal,
    ClarifyPlaceholder,
    ProviderUnavailablePlaceholder,
}

fn provider_unavailable_assistant_placeholder() -> &'static str {
    "[provider_unavailable_reply_omitted]"
}

fn clarify_assistant_placeholder() -> &'static str {
    "[clarification_requested]"
}

fn looks_like_structured_machine_output(text: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .map(|value| value.is_object() || value.is_array())
        .unwrap_or(false)
}

fn looks_like_linewise_json_machine_output(text: &str) -> bool {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    !lines.is_empty()
        && lines
            .iter()
            .all(|line| looks_like_structured_machine_output(line))
}

fn normalize_read_range_excerpt_for_recent_turns(excerpt: &str) -> Option<String> {
    let lines = excerpt
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.split_once('|')
                .filter(|(prefix, _)| {
                    !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit())
                })
                .map(|(_, rest)| rest.trim_start().to_string())
                .unwrap_or_else(|| line.trim().to_string())
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn normalize_observed_listing_for_recent_turns(text: &str) -> Option<String> {
    if looks_like_structured_machine_output(text) || looks_like_linewise_json_machine_output(text) {
        return None;
    }
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.starts_with("bash: warning: setlocale:")
                && !line.starts_with("warning: setlocale:")
        })
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if lines.len() < 2 {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn strip_ordered_list_prefix(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let digit_count = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if digit_count == 0 {
        return None;
    }
    let rest = &trimmed[digit_count..];
    let stripped = if let Some(rest) = rest.strip_prefix(". ") {
        rest
    } else if let Some(rest) = rest.strip_prefix(") ") {
        rest
    } else if let Some(rest) = rest.strip_prefix("、") {
        rest
    } else {
        return None;
    };
    let stripped = stripped.trim();
    (!stripped.is_empty()).then(|| stripped.to_string())
}

fn is_ordered_list_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    let digit_count = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if digit_count == 0 {
        return false;
    }
    let rest = &trimmed[digit_count..];
    rest.starts_with(". ") || rest.starts_with(") ") || rest.starts_with("、")
}

fn looks_like_wrapped_ordered_listing_answer(text: &str) -> bool {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    lines.len() >= 3
        && lines
            .iter()
            .skip(1)
            .filter(|line| is_ordered_list_line(line))
            .count()
            >= 2
}

fn is_delivery_token_line(line: &str) -> bool {
    let trimmed = line.trim();
    matches!(
        trimmed.split_once(':').map(|(prefix, _)| prefix),
        Some("FILE")
            | Some("IMAGE_FILE")
            | Some("IMAGE_URL")
            | Some("VIDEO_URL")
            | Some("FILE_URL")
            | Some("MEDIA_URL")
    )
}

fn extract_delivery_token_target(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let (_, rest) = trimmed.split_once(':')?;
    let rest = rest.trim();
    (!rest.is_empty()).then(|| rest.to_string())
}

fn normalize_recent_assistant_ordered_entry(entry: &str) -> Option<String> {
    let trimmed = entry
        .trim()
        .trim_matches(|c| matches!(c, '"' | '\'' | '`' | '“' | '”' | '‘' | '’'))
        .trim_end_matches(|c| matches!(c, ';' | '；' | ',' | '，' | '。'))
        .trim();
    if trimmed.is_empty() {
        return None;
    }
    if is_delivery_token_line(trimmed) {
        return extract_delivery_token_target(trimmed);
    }
    Some(trimmed.to_string())
}

fn looks_like_locatorish_recent_assistant_entry(entry: &str) -> bool {
    let trimmed = entry.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains('.')
        || (!trimmed.contains(char::is_whitespace) && trimmed.len() <= 128)
}

fn ordered_entries_from_assistant_reply(text: &str, max_entries: usize) -> Vec<String> {
    let max_entries = max_entries.max(2);

    let numbered = text
        .lines()
        .filter_map(strip_ordered_list_prefix)
        .filter_map(|entry| normalize_recent_assistant_ordered_entry(&entry))
        .take(max_entries)
        .collect::<Vec<_>>();
    if numbered.len() >= 2 {
        return numbered;
    }

    let token_lines = text
        .lines()
        .filter_map(extract_delivery_token_target)
        .take(max_entries)
        .collect::<Vec<_>>();
    if token_lines.len() >= 2 {
        return token_lines;
    }

    let semicolon_source = text
        .rsplit_once('：')
        .map(|(_, tail)| tail)
        .or_else(|| text.rsplit_once(':').map(|(_, tail)| tail))
        .unwrap_or(text);
    let semicolon_entries = semicolon_source
        .split([';', '；'])
        .filter_map(normalize_recent_assistant_ordered_entry)
        .collect::<Vec<_>>();
    if semicolon_entries.len() >= 2
        && semicolon_entries
            .iter()
            .filter(|entry| looks_like_locatorish_recent_assistant_entry(entry))
            .count()
            >= 2
    {
        return semicolon_entries.into_iter().take(max_entries).collect();
    }

    Vec::new()
}

fn format_recent_assistant_ordered_entries(text: &str) -> Option<String> {
    let entries = ordered_entries_from_assistant_reply(text, 10);
    if entries.len() < 2 {
        return None;
    }
    Some(
        entries
            .iter()
            .enumerate()
            .map(|(idx, entry)| format!("{}:{}", idx + 1, entry))
            .collect::<Vec<_>>()
            .join(" | "),
    )
}

fn extract_observed_step_text_for_recent_turns(value: &Value) -> Option<String> {
    let step_results = value
        .get("task_journal")
        .and_then(|v| v.get("trace"))
        .and_then(|v| v.get("step_results"))
        .and_then(Value::as_array)?;
    for step in step_results.iter().rev() {
        let skill = step
            .get("skill")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let output = step
            .get("output_excerpt")
            .or_else(|| step.get("output"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty());
        if skill == "system_basic" {
            let output = output?;
            let parsed = serde_json::from_str::<Value>(output).ok()?;
            let action = parsed
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if action == "read_range" {
                if let Some(excerpt) = parsed
                    .get("excerpt")
                    .and_then(Value::as_str)
                    .and_then(normalize_read_range_excerpt_for_recent_turns)
                {
                    let path = parsed
                        .get("resolved_path")
                        .or_else(|| parsed.get("path"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|v| !v.is_empty());
                    return Some(match path {
                        Some(path) => format!("read_range path={path}\n{excerpt}"),
                        None => excerpt,
                    });
                }
            }
        }
    }
    None
}

fn extract_observed_listing_text_for_recent_turns(value: &Value) -> Option<String> {
    let step_results = value
        .get("task_journal")
        .and_then(|v| v.get("trace"))
        .and_then(|v| v.get("step_results"))
        .and_then(Value::as_array)?;
    for step in step_results.iter().rev() {
        let skill = step
            .get("skill")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let output = step
            .get("output_excerpt")
            .or_else(|| step.get("output"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())?;
        if matches!(skill, "run_cmd" | "list_dir") {
            if let Some(listing) = normalize_observed_listing_for_recent_turns(output) {
                return Some(listing);
            }
        }
        if skill == "system_basic" {
            let parsed = serde_json::from_str::<Value>(output).ok()?;
            let action = parsed
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if action == "inventory_dir" {
                if let Some(listing) = parsed
                    .get("entries")
                    .and_then(Value::as_array)
                    .map(|entries| {
                        entries
                            .iter()
                            .filter_map(|entry| entry.get("name").and_then(Value::as_str))
                            .map(str::trim)
                            .filter(|name| !name.is_empty())
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                    })
                    .filter(|entries| entries.len() >= 2)
                    .map(|entries| entries.join("\n"))
                {
                    return Some(listing);
                }
            }
        }
    }
    None
}

fn extract_result_text_for_recent_turns(value: &Value) -> Option<String> {
    if let Some(observed_text) = extract_observed_step_text_for_recent_turns(value) {
        return Some(observed_text);
    }
    let direct_text = value
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string);
    let first_message = value
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string);
    let final_answer = value
        .get("task_journal")
        .and_then(|v| v.get("summary"))
        .and_then(|v| v.get("final_answer"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string);
    let should_prefer_observed = direct_text
        .iter()
        .chain(first_message.iter())
        .chain(final_answer.iter())
        .any(|text| {
            looks_like_structured_machine_output(text)
                || looks_like_linewise_json_machine_output(text)
        });
    if should_prefer_observed {
        if let Some(observed_text) = extract_observed_step_text_for_recent_turns(value) {
            return Some(observed_text);
        }
    }
    let should_prefer_observed_listing = direct_text
        .iter()
        .chain(first_message.iter())
        .chain(final_answer.iter())
        .any(|text| looks_like_wrapped_ordered_listing_answer(text));
    if should_prefer_observed_listing {
        if let Some(observed_listing) = extract_observed_listing_text_for_recent_turns(value) {
            return Some(observed_listing);
        }
    }
    direct_text.or(first_message).or(final_answer).or_else(|| {
        value
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(ToString::to_string)
    })
}

fn classify_assistant_context_reply_kind(
    parsed_result: Option<&Value>,
    assistant_text: &str,
    // The caller owns localized fallback detection. This function only reads
    // structured task metadata to avoid phrase-based language branching here.
    is_fallback: impl Fn(&str) -> bool,
) -> AssistantContextReplyKind {
    if is_fallback(assistant_text) {
        return AssistantContextReplyKind::ProviderUnavailablePlaceholder;
    }
    let summary = parsed_result
        .and_then(|value| value.get("task_journal"))
        .and_then(|value| value.get("summary"));
    let final_status = summary
        .and_then(|value| value.get("final_status"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if final_status.eq_ignore_ascii_case("clarify") {
        return AssistantContextReplyKind::ClarifyPlaceholder;
    }
    let route_gate_kind = summary
        .and_then(|value| value.get("route_result"))
        .and_then(|value| value.get("route_gate_kind"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let needs_clarify = summary
        .and_then(|value| value.get("route_result"))
        .and_then(|value| value.get("needs_clarify"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if route_gate_kind.eq_ignore_ascii_case("clarify") || needs_clarify {
        return AssistantContextReplyKind::ClarifyPlaceholder;
    }
    AssistantContextReplyKind::Normal
}

fn extract_last_turn_assistant_text_from_task(
    state: &AppState,
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
    let assistant_text = if let Some(val) = parsed.as_ref() {
        extract_result_text_for_recent_turns(val)
    } else {
        None
    };
    let assistant_text = assistant_text.unwrap_or_else(|| result_json.to_string());
    match classify_assistant_context_reply_kind(parsed.as_ref(), &assistant_text, |t| {
        crate::fallback::is_known_clarify_fallback_text(state, t)
    }) {
        AssistantContextReplyKind::Normal => Some(assistant_text),
        AssistantContextReplyKind::ClarifyPlaceholder => {
            Some(clarify_assistant_placeholder().to_string())
        }
        AssistantContextReplyKind::ProviderUnavailablePlaceholder => {
            Some(provider_unavailable_assistant_placeholder().to_string())
        }
    }
}

fn query_recent_terminal_ask_turn_for_chat(
    state: &AppState,
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
            state,
            &status,
            result_json.as_deref(),
            error_text.as_deref(),
        ) else {
            continue;
        };
        if assistant_text == provider_unavailable_assistant_placeholder() {
            continue;
        }
        if assistant_text.trim().is_empty() {
            continue;
        }
        return Ok(Some((user_text, assistant_text)));
    }
    Ok(None)
}

fn query_recent_terminal_ask_turns_for_chat(
    state: &AppState,
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
            state,
            &status,
            result_json.as_deref(),
            error_text.as_deref(),
        ) else {
            continue;
        };
        if assistant_text == provider_unavailable_assistant_placeholder() {
            continue;
        }
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
    let user_safety = classify_memory_safety_flag(user_content, &state.policy.memory);
    let assistant_safety = classify_memory_safety_flag(assistant_content, &state.policy.memory);
    let user_text = if state.policy.memory.safety_filter_enabled
        && user_safety == MEMORY_SAFETY_FLAG_INJECTION_LIKE
    {
        "[safety_signal content omitted]".to_string()
    } else {
        utf8_safe_prefix(user_content.trim(), max_segment_chars).to_string()
    };
    let assistant_text = if state.policy.memory.safety_filter_enabled
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
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(_) => return "<none>".to_string(),
    };
    let max_turns = max_turns.max(1).min(10);
    let max_segment_chars = max_segment_chars.max(128);
    let max_total_chars = max_total_chars.max(512);
    let turns = query_recent_terminal_ask_turns_for_chat(
        state, &db, user_id, chat_id, &user_key, max_turns,
    )
    .unwrap_or_default();
    if turns.is_empty() {
        return "<none>".to_string();
    }
    let mut out = String::from("### RECENT_TURNS_FULL\n");
    for (idx, (user_text, assistant_text)) in turns.iter().enumerate() {
        let relative = -((idx as i64) + 1);
        let user_safety = classify_memory_safety_flag(user_text, &state.policy.memory);
        let assistant_safety = classify_memory_safety_flag(assistant_text, &state.policy.memory);
        let user_view = if state.policy.memory.safety_filter_enabled
            && user_safety == MEMORY_SAFETY_FLAG_INJECTION_LIKE
        {
            "[safety_signal content omitted]".to_string()
        } else {
            utf8_safe_prefix(user_text.trim(), max_segment_chars).to_string()
        };
        let assistant_view = if state.policy.memory.safety_filter_enabled
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
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(_) => return "<none>".to_string(),
    };
    if let Ok(Some((user_text, assistant_text))) =
        query_recent_terminal_ask_turn_for_chat(state, &db, user_id, chat_id, &user_key)
    {
        return format_last_turn_full_context(
            state,
            &user_text,
            &assistant_text,
            max_segment_chars,
            max_total_chars,
        );
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
            if let Some(assistant_text) = assistant_context_text_for_recall(state, content) {
                found_assistant = Some((assistant_text.to_string(), safety_flag.clone()));
            }
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
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(_) => return "<none>".to_string(),
    };

    let rows = query_recent_memories_for_chat(&db, user_id, chat_id, &user_key, max_replies * 6)
        .unwrap_or_default();
    if rows.is_empty() {
        return "<none>".to_string();
    }

    let mut lines: Vec<String> = Vec::new();
    for (role, content, safety_flag) in rows {
        if role != MEMORY_ROLE_ASSISTANT {
            continue;
        }
        if state.policy.memory.safety_filter_enabled
            && safety_flag == MEMORY_SAFETY_FLAG_INJECTION_LIKE
        {
            continue;
        }
        let Some(trimmed_content) = assistant_context_text_for_recall(state, &content) else {
            continue;
        };
        let reply_index = lines.len() + 1;
        let relative_index = -(reply_index as i64);
        let preview = utf8_safe_prefix(trimmed_content, preview_chars)
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
        let mut line = format!(
            "- turn_id=assistant[{}] relative_index={} short_preview={} has_code_block={}",
            relative_index, relative_index, preview, has_code_block
        );
        if let Some(ordered_entries) = format_recent_assistant_ordered_entries(trimmed_content) {
            line.push_str(" ordered_entries=");
            line.push_str(&ordered_entries);
        }
        lines.push(line);
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
    contains_any_marker(&norm, &cfg.rules.salience_boost_markers)
        && contains_any_marker(&norm, &cfg.rules.instruction_markers)
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
    if AGENT_DISPLAY_NAME_INVALID_VALUES
        .iter()
        .any(|value| candidate.eq_ignore_ascii_case(value) || candidate.contains(value))
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
