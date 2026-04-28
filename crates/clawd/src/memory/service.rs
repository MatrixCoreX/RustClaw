use crate::memory::retrieval::{MemoryContextMode, RetrievedMemoryItem, StructuredMemoryContext};
use crate::memory::MEMORY_SAFETY_FLAG_INJECTION_LIKE;
use serde::Deserialize;
use tracing::info;

use crate::{AppState, ClaimedTask, LlmProviderRuntime};

const KNOWLEDGE_PERSIST_CONFIDENCE_THRESHOLD: f32 = 0.85;
const KNOWLEDGE_KIND_USER_PREFERENCE: &str = "user_preference";
const KNOWLEDGE_KIND_USER_PROFILE_FACT: &str = "user_profile_fact";
const KNOWLEDGE_KIND_PROJECT_FACT: &str = "project_fact";
const KNOWLEDGE_KIND_RULE: &str = "rule";
const KNOWLEDGE_KIND_TRANSIENT: &str = "transient";
const KNOWLEDGE_NAMESPACE_USER_PROFILE: &str = "user_profile";
const KNOWLEDGE_NAMESPACE_PROJECT_FACTS: &str = "project_facts";
const KNOWLEDGE_NAMESPACE_NONE: &str = "none";

#[derive(Debug, Clone, Deserialize, Default)]
struct LongTermRefreshLlmOut {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    knowledge_candidates: Vec<KnowledgeCandidateLlmOut>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct KnowledgeCandidateLlmOut {
    #[serde(default)]
    should_persist: bool,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    namespace: String,
    #[serde(default)]
    fact: String,
    #[serde(default)]
    confidence: f32,
    #[serde(default)]
    reason: String,
}

#[derive(Debug, Clone)]
struct ValidKnowledgeCandidate {
    namespace: &'static str,
    retrieval_kind: &'static str,
    fact: String,
    source_ref: String,
}

pub(crate) struct PromptMemoryContext {
    pub(crate) prompt_with_memory: String,
    pub(crate) chat_prompt_context: String,
    pub(crate) long_term_summary: Option<String>,
    pub(crate) preferences: Vec<(String, String)>,
    pub(crate) recalled: Vec<(String, String)>,
    pub(crate) similar_triggers: Vec<RetrievedMemoryItem>,
    pub(crate) relevant_facts: Vec<RetrievedMemoryItem>,
    pub(crate) recent_related_events: Vec<RetrievedMemoryItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptMemoryBudgetMode {
    Full,
    Light,
}

#[allow(dead_code)]
pub(crate) fn prepare_prompt_with_memory(
    state: &AppState,
    task: &ClaimedTask,
    prompt: &str,
    chat_memory_budget_chars: usize,
) -> PromptMemoryContext {
    prepare_prompt_with_memory_for_mode(
        state,
        task,
        prompt,
        chat_memory_budget_chars,
        PromptMemoryBudgetMode::Full,
    )
}

pub(crate) fn prepare_prompt_with_memory_for_mode(
    state: &AppState,
    task: &ClaimedTask,
    prompt: &str,
    chat_memory_budget_chars: usize,
    mode: PromptMemoryBudgetMode,
) -> PromptMemoryContext {
    let recent_limit = match mode {
        PromptMemoryBudgetMode::Full => state.policy.memory.prompt_recall_limit.max(1),
        PromptMemoryBudgetMode::Light => 1,
    };
    let include_long_term = true;
    let include_preferences = matches!(mode, PromptMemoryBudgetMode::Full);
    let include_indexed = matches!(mode, PromptMemoryBudgetMode::Full);
    let structured = recall_structured_memory_context_with_options(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        prompt,
        recent_limit,
        include_long_term,
        include_preferences,
        include_indexed,
    );
    let planner_budget = match mode {
        PromptMemoryBudgetMode::Full => state
            .policy
            .memory
            .agent_memory_budget_chars
            .max(512)
            .min(state.policy.memory.prompt_max_chars.max(512)),
        PromptMemoryBudgetMode::Light => state
            .policy
            .memory
            .agent_memory_budget_chars
            .max(512)
            .min(768)
            .min(state.policy.memory.prompt_max_chars.max(512)),
    };
    let chat_budget = match mode {
        PromptMemoryBudgetMode::Full => chat_memory_budget_chars
            .max(384)
            .min(state.policy.memory.prompt_max_chars.max(384)),
        PromptMemoryBudgetMode::Light => chat_memory_budget_chars
            .max(384)
            .min(640)
            .min(state.policy.memory.prompt_max_chars.max(384)),
    };
    let prompt_with_memory =
        structured_memory_context_block(&structured, MemoryContextMode::Planner, planner_budget);
    let chat_prompt_context =
        structured_memory_context_block(&structured, MemoryContextMode::Chat, chat_budget);
    PromptMemoryContext {
        chat_prompt_context,
        long_term_summary: structured.long_term_summary.clone(),
        preferences: structured.preferences.clone(),
        recalled: crate::memory::retrieval::legacy_pairs_from_structured(&structured),
        similar_triggers: structured.similar_triggers.clone(),
        relevant_facts: structured.relevant_facts.clone(),
        recent_related_events: structured.recent_related_events.clone(),
        prompt_with_memory,
    }
}

pub(crate) fn recall_structured_memory_context(
    state: &AppState,
    user_key: Option<&str>,
    user_id: i64,
    chat_id: i64,
    anchor_prompt: &str,
    recent_limit: usize,
    include_long_term: bool,
    include_preferences: bool,
) -> StructuredMemoryContext {
    recall_structured_memory_context_with_options(
        state,
        user_key,
        user_id,
        chat_id,
        anchor_prompt,
        recent_limit,
        include_long_term,
        include_preferences,
        true,
    )
}

fn recall_structured_memory_context_with_options(
    state: &AppState,
    user_key: Option<&str>,
    user_id: i64,
    chat_id: i64,
    anchor_prompt: &str,
    recent_limit: usize,
    include_long_term: bool,
    include_preferences: bool,
    include_indexed: bool,
) -> StructuredMemoryContext {
    let long_term_summary = if include_long_term && state.policy.memory.long_term_enabled {
        crate::memory::recall_long_term_summary(state, user_key, user_id, chat_id)
            .unwrap_or(None)
            .map(|s| {
                crate::truncate_text(&s, state.policy.memory.long_term_recall_max_chars.max(256))
            })
    } else {
        None
    };
    let preferences = if include_preferences {
        crate::memory::recall_user_preferences(
            state,
            user_key,
            user_id,
            chat_id,
            state.policy.memory.preference_recall_limit.max(1),
        )
        .unwrap_or_default()
    } else {
        Vec::new()
    };
    let recalled_recent =
        crate::memory::recall_recent_memories(state, user_key, user_id, chat_id, recent_limit)
            .unwrap_or_default();
    let recalled_recent = crate::memory::filter_memories_for_prompt_recall(
        recalled_recent,
        state.policy.memory.prefer_llm_assistant_memory,
    );
    let recalled_recent = if state.policy.memory.recent_relevance_enabled {
        crate::memory::select_relevant_memories_for_prompt(
            recalled_recent,
            anchor_prompt,
            state
                .policy
                .memory
                .recent_relevance_min_score
                .clamp(0.0, 1.0),
        )
    } else {
        recalled_recent
    };

    let indexed = if include_indexed && state.policy.memory.hybrid_recall_enabled {
        crate::memory::retrieval::retrieve_indexed_memories(
            state,
            user_key,
            user_id,
            chat_id,
            anchor_prompt,
        )
        .unwrap_or_default()
    } else {
        crate::memory::retrieval::IndexedRecall::default()
    };

    StructuredMemoryContext {
        long_term_summary,
        preferences,
        similar_triggers: indexed.similar_triggers,
        relevant_facts: indexed.relevant_facts,
        knowledge_docs: indexed.knowledge_docs,
        recent_related_events: indexed.recent_related_events,
        assistant_results: indexed.assistant_results,
        unfinished_goals: indexed.unfinished_goals,
        recalled_recent,
    }
}

pub(crate) fn structured_memory_context_block(
    ctx: &StructuredMemoryContext,
    mode: MemoryContextMode,
    max_chars: usize,
) -> String {
    crate::memory::retrieval::build_structured_memory_context_block(ctx, mode, max_chars)
}

pub(crate) fn dynamic_chat_memory_budget_chars(
    state: &AppState,
    task: &ClaimedTask,
    request_text: &str,
) -> usize {
    let configured_budget = state.policy.memory.chat_memory_budget_chars.max(384);
    let prompt_budget_cap = state.policy.memory.prompt_max_chars.max(384);
    let providers = state.task_llm_providers(task);
    if providers.is_empty() {
        return configured_budget.min(prompt_budget_cap);
    }
    let min_context_tokens = providers
        .iter()
        .map(|provider| estimate_context_window_tokens(provider.as_ref()))
        .min()
        .unwrap_or(32_000)
        .max(512);
    // Reserve output and control prompt overhead to keep headroom for provider formatting.
    let output_reserve_tokens = 4_096usize.min(min_context_tokens / 3).max(768);
    let fixed_overhead_tokens = 1_200usize;
    let request_tokens = estimate_text_tokens(request_text);
    let available_tokens = min_context_tokens
        .saturating_sub(output_reserve_tokens)
        .saturating_sub(fixed_overhead_tokens)
        .saturating_sub(request_tokens);
    // Keep memory context as a bounded fraction of remaining context.
    let dynamic_tokens = (available_tokens / 4).clamp(192, 8_000);
    let dynamic_chars = dynamic_tokens.saturating_mul(2);
    let dynamic_budget = dynamic_chars.clamp(384, prompt_budget_cap);
    info!(
        "{} dynamic_chat_memory_budget task_id={} configured={} computed={} cap={} min_ctx_tokens={} request_tokens={}",
        crate::highlight_tag("memory"),
        task.task_id,
        configured_budget,
        dynamic_budget,
        prompt_budget_cap,
        min_context_tokens,
        request_tokens
    );
    dynamic_budget
}

pub(crate) fn estimate_context_window_tokens(provider: &LlmProviderRuntime) -> usize {
    let model = provider.config.model.trim().to_ascii_lowercase();
    if let Some(explicit) = extract_model_k_or_m_capacity_tokens(&model) {
        return explicit.max(512);
    }
    match provider.config.provider_type.as_str() {
        "anthropic_claude" => 200_000,
        "google_gemini" => 256_000,
        "openai_compat" => {
            if model.contains("minimax") {
                2_013
            } else if model.contains("gpt-4.1")
                || model.contains("gpt-4o")
                || model.contains("o3")
                || model.contains("o4")
            {
                128_000
            } else if model.contains("gpt-3.5") {
                16_000
            } else if model.contains("deepseek") {
                64_000
            } else if model.contains("qwen") {
                32_000
            } else {
                64_000
            }
        }
        _ => 64_000,
    }
}

fn extract_model_k_or_m_capacity_tokens(model_lower: &str) -> Option<usize> {
    let bytes = model_lower.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        if !bytes[idx].is_ascii_digit() {
            idx += 1;
            continue;
        }
        let start = idx;
        while idx < bytes.len() && bytes[idx].is_ascii_digit() {
            idx += 1;
        }
        if idx >= bytes.len() {
            break;
        }
        let number = model_lower[start..idx].parse::<usize>().ok()?;
        let unit = bytes[idx];
        if unit == b'k' {
            return Some(number.saturating_mul(1_000));
        }
        if unit == b'm' {
            return Some(number.saturating_mul(1_000_000));
        }
        idx += 1;
    }
    None
}

fn estimate_text_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    let mut cjk_count = 0usize;
    for ch in text.chars() {
        if ('\u{4e00}'..='\u{9fff}').contains(&ch) {
            cjk_count += 1;
        }
    }
    if cjk_count * 2 >= chars.max(1) {
        chars.max(1)
    } else {
        chars.div_ceil(3).max(1)
    }
}

pub(crate) async fn maybe_refresh_long_term_summary(
    state: &AppState,
    task: &ClaimedTask,
    force_refresh: bool,
) -> Result<(), String> {
    if !state.policy.memory.long_term_enabled {
        return Ok(());
    }
    if task
        .user_key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .is_none()
    {
        return Ok(());
    }
    let rounds = crate::memory::count_chat_memory_rounds(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
    )
    .map_err(|err| format!("count memory rounds failed: {err}"))?;
    if rounds == 0 {
        return Ok(());
    }
    if !force_refresh && rounds % state.policy.memory.long_term_every_rounds.max(1) != 0 {
        return Ok(());
    }
    let source_id = crate::memory::read_long_term_source_memory_id(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
    )
    .map_err(|err| format!("read long-term source id failed: {err}"))?;
    let fetch_limit = state.policy.memory.long_term_source_rounds.max(1) * 2;
    let entries = crate::memory::recall_memories_since_id(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        source_id,
        fetch_limit,
    )
    .map_err(|err| format!("read memories for summary failed: {err}"))?;
    let min_entries = if force_refresh {
        2
    } else {
        state.policy.memory.long_term_every_rounds.max(1) * 2
    };
    if entries.len() < min_entries {
        return Ok(());
    }
    let new_chars = entries
        .iter()
        .map(|(_, _, content, _)| content.trim().chars().count())
        .sum::<usize>();
    let min_new_chars = if force_refresh {
        (state.policy.memory.long_term_refresh_min_new_chars / 3).max(24)
    } else {
        state.policy.memory.long_term_refresh_min_new_chars.max(1)
    };
    if new_chars < min_new_chars {
        return Ok(());
    }
    if crate::memory::repeated_entries_ratio(&entries)
        > state.policy.memory.long_term_refresh_max_repeat_ratio
    {
        return Ok(());
    }

    let latest_id = entries.last().map(|e| e.0).unwrap_or(source_id);
    if latest_id <= source_id {
        return Ok(());
    }

    let previous_summary = crate::memory::recall_long_term_summary(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
    )
    .map_err(|err| format!("read previous long-term summary failed: {err}"))?
    .unwrap_or_default();

    let mut convo_lines = Vec::new();
    for (_, role, content, safety_flag) in &entries {
        if state.policy.memory.safety_filter_enabled
            && safety_flag == MEMORY_SAFETY_FLAG_INJECTION_LIKE
        {
            convo_lines.push(format!("{role}: [safety_signal content omitted]"));
            continue;
        }
        convo_lines.push(format!("{role}: {content}"));
    }
    if convo_lines.is_empty() {
        return Ok(());
    }
    let (summary_template, summary_prompt_source) =
        match crate::bootstrap::load_required_prompt_template_for_state(
            state,
            "prompts/long_term_summary_prompt.md",
        ) {
            Ok(resolved) => resolved,
            Err(err) => {
                tracing::warn!(
                    "long_term_summary prompt_missing task_id={} err={}",
                    task.task_id,
                    err
                );
                return Ok(());
            }
        };
    let summary_prompt = crate::render_prompt_template(
        &summary_template,
        &[
            ("__PREVIOUS_SUMMARY__", &previous_summary),
            ("__NEW_CONVERSATION_CHUNK__", &convo_lines.join("\n")),
        ],
    );
    crate::log_prompt_render(
        state,
        &task.task_id,
        "long_term_summary_prompt",
        &summary_prompt_source,
        None,
    );

    let summary = crate::llm_gateway::run_with_fallback_with_prompt_source(
        state,
        task,
        &summary_prompt,
        &summary_prompt_source,
    )
    .await?;
    let parsed = match try_parse_long_term_refresh_llm_out_with_schema(&summary) {
        Ok(parsed) => parsed,
        Err(err) => {
            info!(
                "long_term_summary schema_validation_failed task_id={} err={}",
                task.task_id, err
            );
            parse_long_term_refresh_llm_out_legacy(&summary)
        }
    };
    let trimmed = crate::truncate_text(
        &parsed.summary,
        state.policy.memory.long_term_summary_max_chars.max(512),
    );
    crate::memory::upsert_long_term_summary(
        state,
        task.user_id,
        task.chat_id,
        task.user_key.as_deref(),
        &trimmed,
        latest_id,
    )
    .map_err(|err| format!("write long-term summary failed: {err}"))?;
    persist_valid_knowledge_candidates(state, task, latest_id, &parsed.knowledge_candidates)
        .map_err(|err| format!("write knowledge candidates failed: {err}"))?;
    Ok(())
}

fn normalize_long_term_refresh_llm_out(
    mut parsed: LongTermRefreshLlmOut,
) -> Option<LongTermRefreshLlmOut> {
    parsed.summary = parsed.summary.trim().to_string();
    if parsed.summary.is_empty() {
        None
    } else {
        Some(parsed)
    }
}

fn try_parse_long_term_refresh_llm_out_with_schema(
    raw: &str,
) -> Result<LongTermRefreshLlmOut, String> {
    crate::prompt_utils::validate_against_schema::<LongTermRefreshLlmOut>(
        raw,
        crate::prompt_utils::PromptSchemaId::LongTermSummary,
    )
    .map(|validated| validated.value)
    .map_err(|err| err.to_string())
    .and_then(|parsed| {
        normalize_long_term_refresh_llm_out(parsed)
            .ok_or_else(|| "long_term_summary empty summary after normalize".to_string())
    })
}

fn parse_long_term_refresh_llm_out_legacy(raw: &str) -> LongTermRefreshLlmOut {
    crate::parse_llm_json_extract_or_any::<LongTermRefreshLlmOut>(raw)
        .or_else(|| crate::parse_llm_json_raw_or_any::<LongTermRefreshLlmOut>(raw))
        .and_then(normalize_long_term_refresh_llm_out)
        .unwrap_or_else(|| LongTermRefreshLlmOut {
            summary: raw.trim().to_string(),
            knowledge_candidates: Vec::new(),
        })
}

#[cfg(test)]
fn parse_long_term_refresh_llm_out(raw: &str) -> LongTermRefreshLlmOut {
    try_parse_long_term_refresh_llm_out_with_schema(raw)
        .unwrap_or_else(|_| parse_long_term_refresh_llm_out_legacy(raw))
}

fn persist_valid_knowledge_candidates(
    state: &AppState,
    task: &ClaimedTask,
    latest_id: i64,
    candidates: &[KnowledgeCandidateLlmOut],
) -> anyhow::Result<()> {
    let Some(user_key) = task
        .user_key
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return Ok(());
    };
    if candidates.is_empty() {
        return Ok(());
    }
    let db = state
        .core
        .db
        .get()
        .map_err(|e| anyhow::anyhow!("db pool: {e}"))?;
    for candidate in candidates {
        let Some(valid) = validate_knowledge_candidate(user_key, latest_id, candidate) else {
            continue;
        };
        crate::memory::indexing::upsert_knowledge_fact(
            &db,
            task.user_id,
            user_key,
            valid.namespace,
            valid.retrieval_kind,
            &valid.source_ref,
            &valid.fact,
            crate::now_ts_u64() as i64,
        )?;
    }
    Ok(())
}

fn validate_knowledge_candidate(
    user_key: &str,
    _latest_id: i64,
    candidate: &KnowledgeCandidateLlmOut,
) -> Option<ValidKnowledgeCandidate> {
    if !candidate.should_persist || candidate.confidence < KNOWLEDGE_PERSIST_CONFIDENCE_THRESHOLD {
        return None;
    }
    let kind = candidate.kind.trim();
    let namespace = candidate.namespace.trim();
    let fact = candidate.fact.trim();
    let reason = candidate.reason.trim();
    if fact.is_empty()
        || kind.is_empty()
        || namespace.is_empty()
        || namespace == KNOWLEDGE_NAMESPACE_NONE
        || kind == KNOWLEDGE_KIND_TRANSIENT
    {
        return None;
    }
    let normalized_namespace = match kind {
        KNOWLEDGE_KIND_USER_PREFERENCE | KNOWLEDGE_KIND_USER_PROFILE_FACT | KNOWLEDGE_KIND_RULE
            if namespace == KNOWLEDGE_NAMESPACE_USER_PROFILE =>
        {
            KNOWLEDGE_NAMESPACE_USER_PROFILE
        }
        KNOWLEDGE_KIND_PROJECT_FACT if namespace == KNOWLEDGE_NAMESPACE_PROJECT_FACTS => {
            KNOWLEDGE_NAMESPACE_PROJECT_FACTS
        }
        _ => return None,
    };
    let retrieval_kind = crate::memory::RETRIEVAL_KIND_SEMANTIC_FACT;
    let persisted_fact = if reason.is_empty() {
        fact.to_string()
    } else {
        format!("{fact}\nReason: {reason}")
    };
    let source_ref = knowledge_source_ref(user_key, kind, normalized_namespace, fact);
    Some(ValidKnowledgeCandidate {
        namespace: normalized_namespace,
        retrieval_kind,
        fact: persisted_fact,
        source_ref,
    })
}

fn knowledge_source_ref(user_key: &str, kind: &str, namespace: &str, fact: &str) -> String {
    let basis = format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}",
        user_key,
        kind,
        namespace,
        fact.trim()
    );
    format!("knowledge:{}:{:x}", user_key, stable_hash64(&basis))
}

fn stable_hash64(input: &str) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    for byte in input.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
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
) -> anyhow::Result<()> {
    crate::memory::insert_memory(
        state,
        user_id,
        chat_id,
        user_key,
        channel,
        external_chat_id,
        role,
        content,
        max_chars,
        crate::memory::MemoryWriteKind::Default,
    )
}

pub(crate) fn insert_memory_with_kind(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    user_key: Option<&str>,
    channel: &str,
    external_chat_id: Option<&str>,
    role: &str,
    content: &str,
    max_chars: usize,
    write_kind: crate::memory::MemoryWriteKind,
) -> anyhow::Result<()> {
    crate::memory::insert_memory(
        state,
        user_id,
        chat_id,
        user_key,
        channel,
        external_chat_id,
        role,
        content,
        max_chars,
        write_kind,
    )
}

#[cfg(test)]
mod tests {
    use super::{
        knowledge_source_ref, parse_long_term_refresh_llm_out, validate_knowledge_candidate,
        KnowledgeCandidateLlmOut, KNOWLEDGE_KIND_PROJECT_FACT, KNOWLEDGE_KIND_RULE,
        KNOWLEDGE_KIND_TRANSIENT, KNOWLEDGE_KIND_USER_PREFERENCE, KNOWLEDGE_KIND_USER_PROFILE_FACT,
        KNOWLEDGE_NAMESPACE_NONE, KNOWLEDGE_NAMESPACE_PROJECT_FACTS,
        KNOWLEDGE_NAMESPACE_USER_PROFILE,
    };
    use serde_json::Value;
    use std::collections::HashSet;

    #[test]
    fn parse_long_term_refresh_output_falls_back_to_plain_summary() {
        let parsed = parse_long_term_refresh_llm_out("plain summary");
        assert_eq!(parsed.summary, "plain summary");
        assert!(parsed.knowledge_candidates.is_empty());
    }

    #[test]
    fn parse_long_term_refresh_output_falls_back_to_legacy_parse_on_schema_mismatch() {
        let raw = serde_json::json!({
            "summary": "durable summary",
            "knowledge_candidates": [
                {
                    "should_persist": true,
                    "kind": "oops_kind",
                    "namespace": "user_profile",
                    "fact": "some fact",
                    "confidence": 0.9,
                    "reason": "bad enum"
                }
            ]
        })
        .to_string();
        let parsed = parse_long_term_refresh_llm_out(&raw);
        assert_eq!(parsed.summary, "durable summary");
        assert_eq!(parsed.knowledge_candidates.len(), 1);
        assert_eq!(parsed.knowledge_candidates[0].kind, "oops_kind");
    }

    #[test]
    fn long_term_summary_schema_drift() {
        const SCHEMA_RAW: &str =
            include_str!("../../../../prompts/schemas/long_term_summary.schema.json");
        let schema: Value = serde_json::from_str(SCHEMA_RAW)
            .expect("long_term_summary.schema.json must be valid JSON");
        let properties = schema
            .get("properties")
            .and_then(|v| v.as_object())
            .expect("schema.properties must be an object");
        for field in ["summary", "knowledge_candidates"] {
            assert!(
                properties.contains_key(field),
                "schema missing parser field `{field}` under properties — sync prompts/schemas/long_term_summary.schema.json with LongTermRefreshLlmOut",
            );
        }

        let candidate_props = properties
            .get("knowledge_candidates")
            .and_then(|v| v.get("items"))
            .and_then(|v| v.get("properties"))
            .and_then(|v| v.as_object())
            .expect("knowledge_candidates.items.properties must be an object");
        for field in [
            "should_persist",
            "kind",
            "namespace",
            "fact",
            "confidence",
            "reason",
        ] {
            assert!(
                candidate_props.contains_key(field),
                "schema missing parser field `{field}` under candidate properties",
            );
        }

        let kind_enum = candidate_props
            .get("kind")
            .and_then(|v| v.get("enum"))
            .and_then(|v| v.as_array())
            .expect("kind enum must exist");
        let kind_tokens: HashSet<String> = kind_enum
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::to_string)
            .collect();
        let expected_kinds = HashSet::from([
            KNOWLEDGE_KIND_USER_PREFERENCE.to_string(),
            KNOWLEDGE_KIND_USER_PROFILE_FACT.to_string(),
            KNOWLEDGE_KIND_PROJECT_FACT.to_string(),
            KNOWLEDGE_KIND_RULE.to_string(),
            KNOWLEDGE_KIND_TRANSIENT.to_string(),
        ]);
        assert_eq!(kind_tokens, expected_kinds, "kind enum drifted");

        let namespace_enum = candidate_props
            .get("namespace")
            .and_then(|v| v.get("enum"))
            .and_then(|v| v.as_array())
            .expect("namespace enum must exist");
        let namespace_tokens: HashSet<String> = namespace_enum
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::to_string)
            .collect();
        let expected_namespaces = HashSet::from([
            KNOWLEDGE_NAMESPACE_USER_PROFILE.to_string(),
            KNOWLEDGE_NAMESPACE_PROJECT_FACTS.to_string(),
            KNOWLEDGE_NAMESPACE_NONE.to_string(),
        ]);
        assert_eq!(
            namespace_tokens, expected_namespaces,
            "namespace enum drifted"
        );

        let probe = serde_json::json!({
            "summary": "durable summary",
            "knowledge_candidates": [
                {
                    "should_persist": true,
                    "kind": "user_profile_fact",
                    "namespace": "user_profile",
                    "fact": "用户长期偏好中文回复",
                    "confidence": 0.93,
                    "reason": "explicit long-term preference"
                }
            ]
        });
        let validated = crate::prompt_utils::validate_against_schema::<Value>(
            &probe.to_string(),
            crate::prompt_utils::PromptSchemaId::LongTermSummary,
        )
        .expect("long_term summary probe should validate");
        assert_eq!(
            validated
                .value
                .pointer("/knowledge_candidates/0/kind")
                .and_then(|v| v.as_str()),
            Some("user_profile_fact")
        );
    }

    #[test]
    fn validate_knowledge_candidate_accepts_high_confidence_profile_fact() {
        let candidate = KnowledgeCandidateLlmOut {
            should_persist: true,
            kind: "user_profile_fact".to_string(),
            namespace: KNOWLEDGE_NAMESPACE_USER_PROFILE.to_string(),
            fact: "用户长期偏好中文回复".to_string(),
            confidence: 0.93,
            reason: "explicit long-term preference".to_string(),
        };
        let valid =
            validate_knowledge_candidate("user-key", 42, &candidate).expect("candidate valid");
        assert_eq!(valid.namespace, KNOWLEDGE_NAMESPACE_USER_PROFILE);
        assert_eq!(
            valid.retrieval_kind,
            crate::memory::RETRIEVAL_KIND_SEMANTIC_FACT
        );
        assert!(valid.fact.contains("用户长期偏好中文回复"));
    }

    #[test]
    fn validate_knowledge_candidate_rejects_transient_or_mismatched_namespace() {
        let transient = KnowledgeCandidateLlmOut {
            should_persist: true,
            kind: "transient".to_string(),
            namespace: "none".to_string(),
            fact: "刚才命令失败了".to_string(),
            confidence: 0.99,
            reason: "temporary".to_string(),
        };
        assert!(validate_knowledge_candidate("user-key", 42, &transient).is_none());

        let mismatched = KnowledgeCandidateLlmOut {
            should_persist: true,
            kind: "project_fact".to_string(),
            namespace: KNOWLEDGE_NAMESPACE_USER_PROFILE.to_string(),
            fact: "这个项目固定用 cargo check".to_string(),
            confidence: 0.97,
            reason: "project-level rule".to_string(),
        };
        assert!(validate_knowledge_candidate("user-key", 42, &mismatched).is_none());

        let valid_project = KnowledgeCandidateLlmOut {
            should_persist: true,
            kind: "project_fact".to_string(),
            namespace: KNOWLEDGE_NAMESPACE_PROJECT_FACTS.to_string(),
            fact: "这个项目固定用 cargo check".to_string(),
            confidence: 0.97,
            reason: "project-level rule".to_string(),
        };
        assert!(validate_knowledge_candidate("user-key", 42, &valid_project).is_some());
    }

    #[test]
    fn knowledge_source_ref_is_stable_across_refresh_rounds() {
        let first = knowledge_source_ref(
            "user-key",
            "user_profile_fact",
            KNOWLEDGE_NAMESPACE_USER_PROFILE,
            "用户长期偏好中文回复",
        );
        let second = knowledge_source_ref(
            "user-key",
            "user_profile_fact",
            KNOWLEDGE_NAMESPACE_USER_PROFILE,
            "用户长期偏好中文回复",
        );
        assert_eq!(first, second);
    }
}
