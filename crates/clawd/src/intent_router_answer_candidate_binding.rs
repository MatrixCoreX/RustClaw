use rusqlite::params;
use serde_json::Value;

use super::{
    active_context_has_structured_observation_anchor, active_primary_text_context,
    answer_candidate_can_conflict_with_active_text_followup, append_route_reason,
    parse_output_locator_kind, parse_output_semantic_kind, parse_target_task_policy,
    parse_turn_type, scalar_json_value_text, semantic_kind_can_use_existing_observed_context,
    AppState, ClaimedTask, IntentNormalizerOut, OutputLocatorKind, OutputSemanticKind,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct AnswerCandidateBindingReport {
    pub(super) candidate: String,
    pub(super) in_current_request: bool,
    pub(super) in_recent_assistant_replies: bool,
    pub(super) in_recent_turns_full: bool,
    pub(super) in_last_turn_full: bool,
    pub(super) in_recent_execution_context: bool,
    pub(super) in_memory_context: bool,
}

impl AnswerCandidateBindingReport {
    pub(super) fn has_current_or_recent_binding(&self) -> bool {
        self.in_current_request
            || self.in_recent_assistant_replies
            || self.in_recent_turns_full
            || self.in_last_turn_full
            || self.in_recent_execution_context
    }

    pub(super) fn is_memory_only_binding(&self) -> bool {
        self.in_memory_context && !self.has_current_or_recent_binding()
    }

    pub(super) fn is_distinctive(&self) -> bool {
        answer_candidate_is_distinctive_for_binding(&self.candidate)
    }
}

pub(super) fn analyze_answer_candidate_binding(
    request: &str,
    answer_candidate: &str,
    route_view: &crate::task_context_builder::RouteContextView,
) -> Option<AnswerCandidateBindingReport> {
    let answer = answer_candidate.trim();
    if answer.is_empty() {
        return None;
    }
    Some(AnswerCandidateBindingReport {
        candidate: answer.to_string(),
        in_current_request: request.contains(answer),
        in_recent_assistant_replies: route_view.recent_assistant_replies.contains(answer),
        in_recent_turns_full: route_view.recent_turns_full.contains(answer),
        in_last_turn_full: route_view.last_turn_full.contains(answer),
        in_recent_execution_context: route_view.recent_execution_context.contains(answer),
        in_memory_context: route_view.memory_context.contains(answer),
    })
}

fn answer_candidate_is_distinctive_for_binding(candidate: &str) -> bool {
    let trimmed = candidate.trim();
    let signal_chars = trimmed
        .chars()
        .filter(|ch| ch.is_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(ch))
        .count();
    let has_identifier_separator = trimmed.contains(['-', '_', '/', ':', '.']);
    signal_chars >= 8 || (signal_chars >= 4 && has_identifier_separator)
}

pub(super) fn answer_candidate_binding_repair_context(
    report: &AnswerCandidateBindingReport,
    should_refresh_long_term_memory: bool,
) -> String {
    format!(
        "answer_candidate_binding:\n\
         candidate: {}\n\
         should_refresh_long_term_memory: {}\n\
         in_current_request: {}\n\
         in_recent_assistant_replies: {}\n\
         in_recent_turns_full: {}\n\
         in_last_turn_full: {}\n\
         in_recent_execution_context: {}\n\
         in_memory_context: {}\n\
         memory_only_binding: {}\n\
         distinctive_candidate: {}",
        crate::truncate_for_log(&report.candidate),
        should_refresh_long_term_memory,
        report.in_current_request,
        report.in_recent_assistant_replies,
        report.in_recent_turns_full,
        report.in_last_turn_full,
        report.in_recent_execution_context,
        report.in_memory_context,
        report.is_memory_only_binding(),
        report.is_distinctive()
    )
}

pub(super) fn append_contract_repair_context(context: &mut String, block: String) {
    if block.trim().is_empty() {
        return;
    }
    if context.trim().is_empty() || context == "none" {
        *context = block;
    } else {
        context.push_str("\n\n");
        context.push_str(&block);
    }
}

pub(super) fn active_text_answer_candidate_conflict_context(
    binding: Option<&AnswerCandidateBindingReport>,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    should_refresh_long_term_memory: bool,
) -> Option<String> {
    if should_refresh_long_term_memory
        || req_surface.has_explicit_path_or_url()
        || req_surface.locator_target_pair.is_some()
        || req_surface.has_structured_target_refinement()
        || req_surface.has_delivery_token_reference()
        || req_surface.inline_json_shape.is_some()
    {
        return None;
    }
    let binding = binding?;
    if !answer_candidate_can_conflict_with_active_text_followup(Some(binding)) {
        return None;
    }
    let (prior_prompt, prior_output) = active_primary_text_context(session_snapshot)?;
    if prior_output.is_none() {
        return None;
    }
    Some(format!(
        "active_task_answer_candidate_conflict:\n\
         candidate: {}\n\
         in_recent_assistant_replies: {}\n\
         in_recent_turns_full: {}\n\
         in_last_turn_full: {}\n\
         in_memory_context: {}\n\
         active_task_prompt: {}\n\
         active_task_has_output: {}",
        crate::truncate_for_log(&binding.candidate),
        binding.in_recent_assistant_replies,
        binding.in_recent_turns_full,
        binding.in_last_turn_full,
        binding.in_memory_context,
        crate::truncate_for_log(prior_prompt),
        prior_output.is_some()
    ))
}

pub(super) fn active_task_invalid_turn_binding_context(
    raw_normalizer_output: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    should_refresh_long_term_memory: bool,
) -> Option<String> {
    if should_refresh_long_term_memory
        || req_surface.has_explicit_path_or_url()
        || req_surface.locator_target_pair.is_some()
        || req_surface.has_structured_target_refinement()
        || req_surface.has_delivery_token_reference()
        || req_surface.inline_json_shape.is_some()
    {
        return None;
    }
    let (prior_prompt, prior_output) = active_primary_text_context(session_snapshot)?;
    prior_output?;
    let raw_value =
        crate::prompt_utils::parse_llm_json_raw_or_any_with_repair::<Value>(raw_normalizer_output)?;
    let obj = raw_value.as_object()?;
    if raw_normalizer_output_uses_existing_observed_context_contract(obj)
        && active_context_has_structured_observation_anchor(session_snapshot)
    {
        return None;
    }
    let raw_turn_type = obj
        .get("turn_type")
        .and_then(scalar_json_value_text)
        .unwrap_or_default();
    let raw_target_task_policy = obj
        .get("target_task_policy")
        .and_then(scalar_json_value_text)
        .unwrap_or_default();
    let turn_type_invalid =
        !raw_turn_type.trim().is_empty() && parse_turn_type(&raw_turn_type).is_none();
    let target_policy_invalid = !raw_target_task_policy.trim().is_empty()
        && parse_target_task_policy(&raw_target_task_policy).is_none();
    if !(turn_type_invalid || target_policy_invalid) {
        return None;
    }
    Some(format!(
        "active_task_invalid_turn_binding:\n\
         raw_turn_type: {}\n\
         raw_target_task_policy: {}\n\
         turn_type_invalid: {}\n\
         target_task_policy_invalid: {}\n\
         active_task_prompt: {}\n\
         active_task_has_output: true",
        crate::truncate_for_log(raw_turn_type.trim()),
        crate::truncate_for_log(raw_target_task_policy.trim()),
        turn_type_invalid,
        target_policy_invalid,
        crate::truncate_for_log(prior_prompt)
    ))
}

fn raw_normalizer_output_uses_existing_observed_context_contract(
    obj: &serde_json::Map<String, Value>,
) -> bool {
    let Some(contract) = obj.get("output_contract").and_then(Value::as_object) else {
        return false;
    };
    let semantic_kind = contract
        .get("semantic_kind")
        .and_then(scalar_json_value_text)
        .map(|token| parse_output_semantic_kind(&token))
        .unwrap_or(OutputSemanticKind::None);
    if !semantic_kind_can_use_existing_observed_context(semantic_kind) {
        return false;
    }
    let requires_content_evidence = contract
        .get("requires_content_evidence")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| {
            contract
                .get("requires_content_evidence")
                .and_then(scalar_json_value_text)
                .is_some_and(|value| value.eq_ignore_ascii_case("true"))
        });
    let delivery_required = contract
        .get("delivery_required")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| {
            contract
                .get("delivery_required")
                .and_then(scalar_json_value_text)
                .is_some_and(|value| value.eq_ignore_ascii_case("true"))
        });
    let locator_kind = contract
        .get("locator_kind")
        .and_then(scalar_json_value_text)
        .map(|token| parse_output_locator_kind(&token))
        .unwrap_or(OutputLocatorKind::None);

    requires_content_evidence
        && !delivery_required
        && matches!(
            locator_kind,
            OutputLocatorKind::None
                | OutputLocatorKind::Path
                | OutputLocatorKind::Filename
                | OutputLocatorKind::CurrentWorkspace
        )
}

pub(super) fn clear_memory_update_answer_candidate_if_memory_only(
    out: &mut IntentNormalizerOut,
    binding: Option<&AnswerCandidateBindingReport>,
) -> Option<&'static str> {
    if !out.should_refresh_long_term_memory {
        return None;
    }
    let Some(binding) = binding else {
        return None;
    };
    if !binding.is_memory_only_binding() || !binding.is_distinctive() {
        return None;
    }
    out.answer_candidate.clear();
    append_route_reason(
        &mut out.reason,
        "memory_update_unbound_answer_candidate_cleared",
    );
    Some("memory_update_unbound_answer_candidate_cleared")
}

fn answer_candidate_contains_internal_context_marker(answer_candidate: &str) -> bool {
    const INTERNAL_CONTEXT_MARKERS: &[&str] = &[
        "### SESSION_ALIAS_BINDINGS",
        "### ACTIVE_EXECUTION_ANCHOR",
        "SESSION_ALIAS_BINDINGS",
        "ACTIVE_EXECUTION_ANCHOR",
    ];
    INTERNAL_CONTEXT_MARKERS
        .iter()
        .any(|marker| answer_candidate.contains(marker))
}

pub(super) fn clear_internal_context_answer_candidate(
    out: &mut IntentNormalizerOut,
) -> Option<&'static str> {
    if !answer_candidate_contains_internal_context_marker(&out.answer_candidate) {
        return None;
    }
    out.answer_candidate.clear();
    append_route_reason(&mut out.reason, "internal_context_answer_candidate_cleared");
    Some("internal_context_answer_candidate_cleared")
}

pub(super) fn recent_distinctive_scalar_conflict_tokens(
    binding: &AnswerCandidateBindingReport,
    route_view: &crate::task_context_builder::RouteContextView,
) -> Vec<String> {
    if !binding.is_memory_only_binding() || !binding.is_distinctive() {
        return Vec::new();
    }
    let mut tokens = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for source in [
        route_view.recent_assistant_replies.as_str(),
        route_view.recent_turns_full.as_str(),
        route_view.last_turn_full.as_str(),
        route_view.recent_execution_context.as_str(),
    ] {
        for token in source.split(|ch: char| {
            !(ch.is_ascii_alphanumeric()
                || ('\u{4e00}'..='\u{9fff}').contains(&ch)
                || matches!(ch, '_' | '-' | '/' | '.' | ':'))
        }) {
            let token = token.trim_matches(|ch: char| matches!(ch, '_' | '-' | '/' | '.' | ':'));
            if !recent_distinctive_scalar_conflict_token(&binding.candidate, token) {
                continue;
            }
            let normalized = token.to_ascii_lowercase();
            if seen.insert(normalized) {
                tokens.push(token.to_string());
            }
        }
    }
    tokens
}

fn recent_distinctive_scalar_conflict_token(candidate: &str, token: &str) -> bool {
    let token = token.trim();
    if token.is_empty()
        || token.eq_ignore_ascii_case(candidate.trim())
        || token.contains(['/', '\\'])
    {
        return false;
    }
    let signal_chars = token
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(ch))
        .count();
    let has_digit = token.chars().any(|ch| ch.is_ascii_digit());
    let has_identifier_separator = token.contains(['_', '-', '.', ':']);
    has_digit && ((signal_chars >= 4 && has_identifier_separator) || signal_chars >= 8)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DistinctiveMemoryScalarClass {
    LocatorLike,
    DottedVersion,
    StructuredId,
}

fn dotted_version_like_scalar(value: &str) -> bool {
    let trimmed = value.trim().trim_start_matches(['v', 'V']);
    let mut saw_dot = false;
    let mut saw_part = false;
    for part in trimmed.split('.') {
        if part.is_empty() || !part.chars().all(|ch| ch.is_ascii_digit()) {
            return false;
        }
        saw_part = true;
        saw_dot = true;
    }
    saw_part && saw_dot && trimmed.contains('.')
}

fn locator_like_scalar(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.contains(['/', '\\']) {
        return true;
    }
    if trimmed.chars().any(char::is_whitespace) {
        return false;
    }
    trimmed.contains('.') && !dotted_version_like_scalar(trimmed)
}

fn distinctive_memory_scalar_class(value: &str) -> Option<DistinctiveMemoryScalarClass> {
    let trimmed = value
        .trim()
        .trim_matches(|ch: char| matches!(ch, '_' | '-' | '/' | '\\' | '.' | ':'));
    if trimmed.is_empty() {
        return None;
    }
    if locator_like_scalar(trimmed) {
        return Some(DistinctiveMemoryScalarClass::LocatorLike);
    }
    if dotted_version_like_scalar(trimmed) {
        return Some(DistinctiveMemoryScalarClass::DottedVersion);
    }
    if recent_distinctive_scalar_conflict_token("", trimmed) {
        return Some(DistinctiveMemoryScalarClass::StructuredId);
    }
    None
}

fn pathish_basename(value: &str) -> &str {
    value
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(value)
        .trim_matches(|ch: char| matches!(ch, '_' | '-' | '/' | '\\' | '.' | ':'))
}

fn memory_recall_rebind_token(
    candidate: &str,
    token: &str,
    candidate_class: DistinctiveMemoryScalarClass,
) -> Option<String> {
    let token = token.trim();
    if token.is_empty() || token.eq_ignore_ascii_case(candidate.trim()) {
        return None;
    }
    let token_class = distinctive_memory_scalar_class(token)?;
    if token_class != candidate_class {
        return None;
    }
    match candidate_class {
        DistinctiveMemoryScalarClass::LocatorLike => {
            let candidate_base = pathish_basename(candidate);
            let token_base = pathish_basename(token);
            if token_base.is_empty() || token_base.eq_ignore_ascii_case(candidate_base) {
                return None;
            }
            if candidate.contains(['/', '\\']) {
                Some(token.to_string())
            } else {
                Some(token_base.to_string())
            }
        }
        DistinctiveMemoryScalarClass::DottedVersion
        | DistinctiveMemoryScalarClass::StructuredId => {
            if recent_distinctive_scalar_conflict_token(candidate, token) {
                Some(token.to_string())
            } else {
                None
            }
        }
    }
}

pub(super) fn clear_memory_only_answer_candidate_if_recent_context_conflicts(
    out: &mut IntentNormalizerOut,
    binding: Option<&AnswerCandidateBindingReport>,
    route_view: &crate::task_context_builder::RouteContextView,
) -> Option<&'static str> {
    let binding = binding?;
    if recent_distinctive_scalar_conflict_tokens(binding, route_view).is_empty() {
        return None;
    }
    out.answer_candidate.clear();
    append_route_reason(
        &mut out.reason,
        "memory_only_answer_candidate_recent_scalar_conflict_cleared",
    );
    Some("memory_only_answer_candidate_recent_scalar_conflict_cleared")
}

pub(super) fn latest_user_memory_distinctive_scalar_candidate(
    state: &AppState,
    task: &ClaimedTask,
    binding: &AnswerCandidateBindingReport,
) -> Option<String> {
    if !binding.is_memory_only_binding() || !binding.is_distinctive() {
        return None;
    }
    let candidate_class = distinctive_memory_scalar_class(&binding.candidate)?;
    if candidate_class != DistinctiveMemoryScalarClass::LocatorLike {
        return None;
    }
    let user_key = task.user_key.as_deref().unwrap_or_default().trim();
    let db = state.core.db.get().ok()?;
    let mut stmt = db
        .prepare(
            "SELECT search_text
             FROM memory_retrieval_index
             WHERE source_kind = ?1
               AND user_id = ?2
               AND (?3 = '' OR COALESCE(user_key, '') = ?3)
               AND memory_kind IN (?4, ?5, ?6)
             ORDER BY COALESCE(updated_at_ts, created_at_ts, 0) DESC, id DESC
             LIMIT 64",
        )
        .ok()?;
    let rows = stmt
        .query_map(
            params![
                crate::memory::RETRIEVAL_SOURCE_MEMORY,
                task.user_id,
                user_key,
                crate::memory::RETRIEVAL_KIND_ASSISTANT_RESULT,
                crate::memory::RETRIEVAL_KIND_TRIGGER_ANCHOR,
                crate::memory::RETRIEVAL_KIND_EPISODIC_EVENT,
            ],
            |row| row.get::<_, String>(0),
        )
        .ok()?;
    for row in rows.flatten() {
        for token in
            distinctive_scalar_tokens_for_memory_recall(&binding.candidate, candidate_class, &row)
        {
            return Some(token);
        }
    }
    None
}

fn distinctive_scalar_tokens_for_memory_recall(
    candidate: &str,
    candidate_class: DistinctiveMemoryScalarClass,
    text: &str,
) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for token in text.split(|ch: char| {
        !(ch.is_ascii_alphanumeric()
            || ('\u{4e00}'..='\u{9fff}').contains(&ch)
            || matches!(ch, '_' | '-' | '/' | '.' | ':'))
    }) {
        let token = token.trim_matches(|ch: char| matches!(ch, '_' | '-' | '/' | '.' | ':'));
        let Some(rebind_token) = memory_recall_rebind_token(candidate, token, candidate_class)
        else {
            continue;
        };
        let normalized = rebind_token.to_ascii_lowercase();
        if seen.insert(normalized) {
            tokens.push(rebind_token);
        }
    }
    tokens
}

pub(super) fn rebind_memory_only_answer_candidate_to_recent_user_memory(
    state: &AppState,
    task: &ClaimedTask,
    out: &mut IntentNormalizerOut,
    binding: Option<&AnswerCandidateBindingReport>,
) -> Option<&'static str> {
    let binding = binding?;
    let candidate = latest_user_memory_distinctive_scalar_candidate(state, task, binding)?;
    out.answer_candidate = candidate;
    out.decision = "direct_answer".to_string();
    out.needs_clarify = false;
    out.clarify_question.clear();
    append_route_reason(
        &mut out.reason,
        "memory_only_answer_candidate_rebound_to_recent_user_memory",
    );
    Some("memory_only_answer_candidate_rebound_to_recent_user_memory")
}
