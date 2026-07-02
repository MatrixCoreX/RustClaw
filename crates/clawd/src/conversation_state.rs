use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{AppState, ClaimedTask};

const MAX_SESSION_ALIAS_BINDINGS: usize = 12;

#[path = "conversation_alias.rs"]
mod conversation_alias;

#[cfg(test)]
pub(crate) use conversation_alias::session_alias_bindings_from_state_patch;
pub(crate) use conversation_alias::{
    alias_bindings_mentioned_in_prompt, alias_surface_matches_prompt,
    single_alias_binding_mentioned_in_prompt, state_patch_is_alias_bindings_only,
    structural_quoted_alias_binding_from_single_locator_prompt,
    structural_quoted_alias_bindings_from_prompt,
};

use conversation_alias::{merge_alias_bindings_for_turn, turn_analysis_has_alias_only_state_patch};

#[cfg(test)]
use conversation_alias::{
    merge_alias_bindings, structural_alias_binding_from_prompt,
    structural_alias_rebinds_from_prompt,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct SessionAliasBinding {
    pub(crate) alias: String,
    pub(crate) target: String,
    pub(crate) updated_at_ts: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct ConversationState {
    pub(crate) active_followup_task_id: Option<String>,
    pub(crate) active_clarify_task_id: Option<String>,
    pub(crate) active_observed_facts_task_id: Option<String>,
    #[serde(default)]
    pub(crate) alias_bindings: Vec<SessionAliasBinding>,
    #[serde(default)]
    pub(crate) last_primary_task_prompt: Option<String>,
    #[serde(default)]
    pub(crate) last_primary_task_output: Option<String>,
    pub(crate) locale_hint: Option<String>,
    pub(crate) last_task_id: String,
    pub(crate) updated_at_ts: u64,
}

pub(crate) struct ActiveSessionSnapshot {
    pub(crate) conversation_state: Option<ConversationState>,
    pub(crate) active_followup_frame: Option<crate::followup_frame::FollowupFrame>,
    pub(crate) active_clarify_state: Option<crate::clarify_state::ClarifyState>,
    pub(crate) active_observed_facts: Option<crate::observed_facts::ObservedFacts>,
}

#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub(crate) struct ActiveSessionPointers {
    pub(crate) active_followup_task_id: Option<String>,
    pub(crate) active_clarify_task_id: Option<String>,
    pub(crate) active_observed_facts_task_id: Option<String>,
}

fn effective_user_key(task: &ClaimedTask) -> String {
    task.user_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("anon:{}:{}", task.user_id, task.chat_id))
}

fn normalized_locale_hint(payload: Option<&Value>) -> Option<String> {
    payload
        .and_then(|value| {
            value
                .get("response_language")
                .or_else(|| value.get("language"))
                .or_else(|| value.get("locale"))
        })
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn next_last_primary_task_prompt(
    prior_state: Option<&ConversationState>,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    journal: &crate::task_journal::TaskJournal,
    prompt: &str,
    resolved_prompt_for_execution: &str,
) -> Option<String> {
    if standalone_preference_or_memory_turn_clears_primary_task(route_result, turn_analysis) {
        return None;
    }
    if should_preserve_active_session_pointers(turn_analysis) {
        return prior_state.and_then(|state| state.last_primary_task_prompt.clone());
    }
    let prior_prompt = prior_state.and_then(|state| state.last_primary_task_prompt.clone());
    if standalone_scalar_result_should_not_promote(
        prior_prompt.as_deref(),
        route_result,
        turn_analysis,
    ) {
        return prior_prompt;
    }
    if standalone_task_request_preserves_prior_primary(
        prior_prompt.as_deref(),
        route_result,
        turn_analysis,
    ) {
        return prior_prompt;
    }
    let user_prompt = prompt.trim();
    let resolved_prompt = resolved_prompt_for_execution.trim();
    let current_prompt = if user_prompt.is_empty() {
        resolved_prompt
    } else {
        user_prompt
    };
    if current_prompt.is_empty() {
        return prior_prompt;
    }
    if unannotated_structured_listing_starts_primary_task(route_result, turn_analysis, journal) {
        return Some(current_prompt.to_string());
    }
    let Some(turn_type) = turn_analysis.and_then(|analysis| analysis.turn_type) else {
        if unannotated_evidence_backed_deliverable_starts_primary_task(route_result, turn_analysis)
        {
            return Some(current_prompt.to_string());
        }
        if standalone_contextual_chat_result_starts_primary_task(route_result, turn_analysis) {
            return Some(current_prompt.to_string());
        }
        if route_result.needs_clarify && prior_prompt.is_none() {
            return Some(current_prompt.to_string());
        }
        return prior_prompt;
    };
    if !is_primary_task_turn_type(turn_type) {
        return prior_prompt;
    }
    match turn_type {
        crate::intent_router::TurnType::TaskRequest => Some(current_prompt.to_string()),
        crate::intent_router::TurnType::TaskReplace => Some(current_prompt.to_string()),
        crate::intent_router::TurnType::TaskAppend
        | crate::intent_router::TurnType::TaskCorrect
        | crate::intent_router::TurnType::TaskScopeUpdate => Some(merge_primary_task_prompt(
            prior_prompt.as_deref(),
            current_prompt,
            turn_type,
            turn_analysis.and_then(|analysis| analysis.state_patch.as_ref()),
        )),
        _ => prior_prompt,
    }
}

fn merge_primary_task_prompt(
    prior_prompt: Option<&str>,
    current_prompt: &str,
    turn_type: crate::intent_router::TurnType,
    state_patch: Option<&Value>,
) -> String {
    let prior = prior_prompt
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(prior) = prior else {
        return current_prompt.to_string();
    };
    if prior == current_prompt {
        return prior.to_string();
    }
    let label = match turn_type {
        crate::intent_router::TurnType::TaskCorrect => "Correction",
        crate::intent_router::TurnType::TaskScopeUpdate => "Scope update",
        _ => "Additional instruction",
    };
    let patch = state_patch
        .and_then(render_primary_task_state_patch)
        .map(|patch| format!("\nStructured update: {patch}"))
        .unwrap_or_default();
    if prior.starts_with("Task so far:\n") {
        format!("{prior}\n\n{label}: {current_prompt}{patch}")
    } else {
        format!("Task so far:\n{prior}\n\n{label}: {current_prompt}{patch}")
    }
}

fn render_primary_task_state_patch(state_patch: &Value) -> Option<String> {
    match state_patch {
        Value::Null => None,
        Value::Object(map) if map.is_empty() => None,
        Value::Array(items) if items.is_empty() => None,
        Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        other => serde_json::to_string(other).ok(),
    }
}

fn is_primary_task_turn_type(turn_type: crate::intent_router::TurnType) -> bool {
    matches!(
        turn_type,
        crate::intent_router::TurnType::TaskRequest
            | crate::intent_router::TurnType::TaskAppend
            | crate::intent_router::TurnType::TaskReplace
            | crate::intent_router::TurnType::TaskCorrect
            | crate::intent_router::TurnType::TaskScopeUpdate
    )
}

fn should_track_primary_task_output(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    let Some(turn_type) = turn_analysis.and_then(|analysis| analysis.turn_type) else {
        return false;
    };
    is_primary_task_turn_type(turn_type)
}

fn active_primary_followup_turn(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    matches!(
        (
            turn_analysis.and_then(|analysis| analysis.turn_type),
            turn_analysis.and_then(|analysis| analysis.target_task_policy),
        ),
        (
            Some(
                crate::intent_router::TurnType::TaskAppend
                    | crate::intent_router::TurnType::TaskCorrect
                    | crate::intent_router::TurnType::TaskReplace
                    | crate::intent_router::TurnType::TaskScopeUpdate
            ),
            Some(
                crate::intent_router::TargetTaskPolicy::ReuseActive
                    | crate::intent_router::TargetTaskPolicy::ReplaceActive
            )
        )
    )
}

fn active_primary_non_success_preserves_prior(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    active_primary_followup_turn(turn_analysis)
        && matches!(
            journal.final_status,
            Some(
                crate::task_journal::TaskJournalFinalStatus::Clarify
                    | crate::task_journal::TaskJournalFinalStatus::Failure
                    | crate::task_journal::TaskJournalFinalStatus::ResumeFailure
            )
        )
}

fn model_fallback_preserves_primary_state(
    fallback_source: Option<crate::fallback::ClarifyFallbackSource>,
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    matches!(
        fallback_source,
        Some(
            crate::fallback::ClarifyFallbackSource::LlmUnavailable
                | crate::fallback::ClarifyFallbackSource::EmptyResponse
        )
    ) && matches!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Clarify)
    )
}

fn prior_last_primary_task_output(prior_state: Option<&ConversationState>) -> Option<String> {
    prior_state.and_then(|state| state.last_primary_task_output.clone())
}

fn standalone_contextual_chat_result_starts_primary_task(
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    let allowed_turn = turn_analysis.is_none()
        || matches!(
            (
                turn_analysis.and_then(|analysis| analysis.turn_type),
                turn_analysis.and_then(|analysis| analysis.target_task_policy),
            ),
            (
                Some(crate::intent_router::TurnType::TaskRequest),
                Some(crate::intent_router::TargetTaskPolicy::Standalone)
            )
        );
    if !allowed_turn
        || !route_result.is_resume_discussion_mode()
        || route_result.needs_clarify
        || route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
        || !matches!(
            route_result.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::None
        )
    {
        return false;
    }
    matches!(
        route_result.effective_output_contract_semantic_kind(),
        crate::OutputSemanticKind::QuantityComparison
            | crate::OutputSemanticKind::RecentScalarEqualityCheck
    )
}

fn has_prior_primary_task(prior_state: Option<&ConversationState>) -> bool {
    prior_state.is_some_and(|state| {
        state
            .last_primary_task_prompt
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
            || state
                .last_primary_task_output
                .as_deref()
                .map(str::trim)
                .is_some_and(|value| !value.is_empty())
    })
}

fn unannotated_evidence_backed_deliverable_starts_primary_task(
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    if turn_analysis.is_some()
        || route_result.needs_clarify
        || !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
    {
        return false;
    }

    true
}

fn unannotated_structured_listing_starts_primary_task(
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    turn_analysis.is_none()
        && !route_result.needs_clarify
        && !crate::followup_frame::derive_ordered_entries_from_journal(journal).is_empty()
}

fn standalone_preference_or_memory_turn_clears_primary_task(
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    matches!(
        turn_analysis.and_then(|analysis| analysis.turn_type),
        Some(crate::intent_router::TurnType::PreferenceOrMemory)
    ) && !matches!(
        turn_analysis.and_then(|analysis| analysis.target_task_policy),
        Some(
            crate::intent_router::TargetTaskPolicy::ReuseActive
                | crate::intent_router::TargetTaskPolicy::ReplaceActive
        )
    ) && route_result.is_resume_discussion_mode()
        && !route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
}

fn state_patch_requests_primary_task_replacement(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    let Some(patch) = turn_analysis.and_then(|analysis| analysis.state_patch.as_ref()) else {
        return false;
    };
    let primary_update = patch
        .get("primary_task_update")
        .and_then(|value| value.as_str())
        .map(str::trim);
    let active_boundary = patch
        .get("active_task_boundary")
        .and_then(|value| value.as_str())
        .map(str::trim);
    matches!(primary_update, Some("replace")) || matches!(active_boundary, Some("new_deliverable"))
}

fn standalone_task_request_preserves_prior_primary(
    prior_primary_task_prompt: Option<&str>,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    if standalone_contextual_chat_result_starts_primary_task(route_result, turn_analysis) {
        return false;
    }
    if state_patch_requests_primary_task_replacement(turn_analysis) {
        return false;
    }
    prior_primary_task_prompt
        .map(str::trim)
        .is_some_and(|prompt| !prompt.is_empty())
        && matches!(
            turn_analysis.and_then(|analysis| analysis.turn_type),
            Some(crate::intent_router::TurnType::TaskRequest)
        )
        && matches!(
            turn_analysis.and_then(|analysis| analysis.target_task_policy),
            Some(crate::intent_router::TargetTaskPolicy::Standalone)
        )
        && route_allows_standalone_scalar_non_promotion(route_result)
        && !route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
}

fn standalone_scalar_result_should_not_promote(
    _prior_primary_task_prompt: Option<&str>,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    let is_standalone_task_request =
        matches!(
            turn_analysis.and_then(|analysis| analysis.turn_type),
            Some(crate::intent_router::TurnType::TaskRequest)
        ) && matches!(
            turn_analysis.and_then(|analysis| analysis.target_task_policy),
            Some(crate::intent_router::TargetTaskPolicy::Standalone)
        ) && route_allows_standalone_scalar_non_promotion(route_result);
    if !is_standalone_task_request
        || route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
    {
        return false;
    }
    matches!(
        route_result.output_contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
    )
}

fn route_allows_standalone_scalar_non_promotion(route_result: &crate::RouteResult) -> bool {
    if route_result.is_resume_discussion_mode() {
        return true;
    }
    route_result.uses_pure_chat_agent_loop_submode()
        && route_result.schedule_kind == crate::ScheduleKind::None
        && !route_result.needs_clarify
        && !route_result.wants_file_delivery
        && !route_result.should_refresh_long_term_memory
        && !route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
        && matches!(
            route_result.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::None
        )
        && matches!(
            route_result.effective_output_contract_semantic_kind(),
            crate::OutputSemanticKind::None
        )
}

fn standalone_scalar_output_should_not_promote(
    prior_state: Option<&ConversationState>,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    _resolved_prompt_for_execution: &str,
) -> bool {
    standalone_scalar_result_should_not_promote(
        prior_state.and_then(|state| state.last_primary_task_prompt.as_deref()),
        route_result,
        turn_analysis,
    ) || standalone_scalar_result_should_not_promote(
        prior_state.and_then(|state| state.last_primary_task_output.as_deref()),
        route_result,
        turn_analysis,
    )
}

fn current_turn_answer_text(answer_text: &str, answer_messages: &[String]) -> Option<String> {
    answer_text
        .trim()
        .is_empty()
        .then(|| {
            answer_messages
                .iter()
                .map(String::as_str)
                .find(|text| !text.trim().is_empty())
                .map(str::to_string)
        })
        .flatten()
        .or_else(|| {
            let trimmed = answer_text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
}

fn substantial_text_deliverable(answer_text: &str) -> bool {
    let trimmed = answer_text.trim();
    if trimmed.is_empty() {
        return false;
    }

    let char_count = trimmed.chars().count();
    if char_count >= 180 {
        return true;
    }

    let non_empty_line_count = trimmed
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    if char_count >= 80 && non_empty_line_count >= 3 {
        return true;
    }

    let has_structural_markdown = trimmed.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with('#')
            || line.starts_with("- ")
            || line.starts_with("* ")
            || line.starts_with("> ")
            || line.starts_with("| ")
            || line.starts_with("```")
            || line.split_once('.').is_some_and(|(prefix, suffix)| {
                !suffix.trim_start().is_empty()
                    && !prefix.is_empty()
                    && prefix.chars().all(|ch| ch.is_ascii_digit())
            })
    });
    if has_structural_markdown && char_count >= 60 && non_empty_line_count >= 2 {
        return true;
    }

    compact_sentence_deliverable(trimmed, char_count, non_empty_line_count)
}

fn compact_sentence_deliverable(
    trimmed: &str,
    char_count: usize,
    non_empty_line_count: usize,
) -> bool {
    if !(24..=180).contains(&char_count) || non_empty_line_count != 1 {
        return false;
    }
    let starts_like_machine_payload = trimmed.starts_with('{')
        || trimmed.starts_with('[')
        || trimmed.starts_with("FILE:")
        || trimmed.starts_with('/')
        || trimmed.starts_with("./")
        || trimmed.starts_with("../");
    if starts_like_machine_payload {
        return false;
    }
    let Some(last) = trimmed.chars().last() else {
        return false;
    };
    matches!(last, '.' | '!' | '。' | '！')
}

fn unannotated_chat_output_starts_primary_task(
    prior_state: Option<&ConversationState>,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    answer_text: &str,
    answer_messages: &[String],
) -> bool {
    if turn_analysis.is_some()
        || has_prior_primary_task(prior_state)
        || !route_result.is_resume_discussion_mode()
        || route_result.needs_clarify
        || route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || !matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
        || !matches!(
            route_result.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::None
        )
        || !matches!(
            route_result.effective_output_contract_semantic_kind(),
            crate::OutputSemanticKind::None
        )
        || matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
    {
        return false;
    }

    current_turn_answer_text(answer_text, answer_messages)
        .as_deref()
        .is_some_and(substantial_text_deliverable)
}

fn current_turn_primary_task_prompt(
    prompt: &str,
    resolved_prompt_for_execution: &str,
) -> Option<String> {
    let user_prompt = prompt.trim();
    let resolved_prompt = resolved_prompt_for_execution.trim();
    let current_prompt = if user_prompt.is_empty() {
        resolved_prompt
    } else {
        user_prompt
    };
    (!current_prompt.is_empty()).then(|| current_prompt.to_string())
}

fn unannotated_chat_primary_prompt_for_output(
    prior_state: Option<&ConversationState>,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    prompt: &str,
    resolved_prompt_for_execution: &str,
    answer_text: &str,
    answer_messages: &[String],
) -> Option<String> {
    unannotated_chat_output_starts_primary_task(
        prior_state,
        route_result,
        turn_analysis,
        answer_text,
        answer_messages,
    )
    .then(|| current_turn_primary_task_prompt(prompt, resolved_prompt_for_execution))
    .flatten()
}

fn standalone_chat_deliverable_starts_primary_task(
    prior_state: Option<&ConversationState>,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    if has_prior_primary_task(prior_state)
        && !state_patch_requests_primary_task_replacement(turn_analysis)
    {
        return false;
    }
    matches!(
        (
            turn_analysis.and_then(|analysis| analysis.turn_type),
            turn_analysis.and_then(|analysis| analysis.target_task_policy),
        ),
        (
            Some(crate::intent_router::TurnType::TaskRequest),
            Some(crate::intent_router::TargetTaskPolicy::Standalone)
        )
    ) && route_result.is_resume_discussion_mode()
        && !route_result.needs_clarify
        && !route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::None
        )
        && !matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
}

fn next_last_primary_task_output(
    prior_state: Option<&ConversationState>,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    journal: &crate::task_journal::TaskJournal,
    resolved_prompt_for_execution: &str,
    answer_text: &str,
    answer_messages: &[String],
) -> Option<String> {
    if standalone_preference_or_memory_turn_clears_primary_task(route_result, turn_analysis) {
        return None;
    }
    if active_primary_non_success_preserves_prior(turn_analysis, journal) {
        return prior_last_primary_task_output(prior_state);
    }
    if should_preserve_active_session_pointers(turn_analysis)
        || standalone_scalar_output_should_not_promote(
            prior_state,
            route_result,
            turn_analysis,
            resolved_prompt_for_execution,
        )
        || standalone_task_request_preserves_prior_primary(
            prior_state.and_then(|state| state.last_primary_task_prompt.as_deref()),
            route_result,
            turn_analysis,
        )
        || !should_track_primary_task_output(turn_analysis)
    {
        if unannotated_evidence_backed_deliverable_starts_primary_task(route_result, turn_analysis)
        {
            let latest_output = current_turn_answer_text(answer_text, answer_messages);
            return latest_output.or_else(|| prior_last_primary_task_output(prior_state));
        }
        if unannotated_structured_listing_starts_primary_task(route_result, turn_analysis, journal)
        {
            let latest_output = current_turn_answer_text(answer_text, answer_messages);
            return latest_output.or_else(|| prior_last_primary_task_output(prior_state));
        }
        if standalone_contextual_chat_result_starts_primary_task(route_result, turn_analysis) {
            let latest_output = current_turn_answer_text(answer_text, answer_messages);
            return latest_output.or_else(|| prior_last_primary_task_output(prior_state));
        }
        if unannotated_chat_output_starts_primary_task(
            prior_state,
            route_result,
            turn_analysis,
            answer_text,
            answer_messages,
        ) {
            let latest_output = current_turn_answer_text(answer_text, answer_messages);
            return latest_output.or_else(|| prior_last_primary_task_output(prior_state));
        }
        if standalone_chat_deliverable_starts_primary_task(prior_state, route_result, turn_analysis)
        {
            let latest_output = current_turn_answer_text(answer_text, answer_messages);
            return latest_output.or_else(|| prior_last_primary_task_output(prior_state));
        }
        return prior_last_primary_task_output(prior_state);
    }
    let latest_output = current_turn_answer_text(answer_text, answer_messages);
    latest_output.or_else(|| prior_last_primary_task_output(prior_state))
}

fn effective_locale_hint(
    prior_state: Option<&ConversationState>,
    payload: Option<&Value>,
) -> Option<String> {
    normalized_locale_hint(payload).or_else(|| {
        prior_state
            .and_then(|state| state.locale_hint.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    })
}

#[cfg(test)]
fn persist_conversation_state(
    state: &AppState,
    task: &ClaimedTask,
    conversation_state: &ConversationState,
) -> Result<()> {
    let db = state
        .core
        .db
        .get()
        .map_err(|err| anyhow::anyhow!("acquire db for conversation state persist: {err}"))?;
    let user_key = effective_user_key(task);
    let state_json = serde_json::to_string(conversation_state)?;
    db.execute(
        "INSERT INTO conversation_states (
            user_id, chat_id, user_key, state_json, last_task_id, updated_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(user_id, chat_id, user_key) DO UPDATE SET
            state_json = excluded.state_json,
            last_task_id = excluded.last_task_id,
            updated_at_ts = excluded.updated_at_ts",
        params![
            task.user_id,
            task.chat_id,
            user_key,
            state_json,
            conversation_state.last_task_id,
            conversation_state.updated_at_ts as i64,
        ],
    )?;
    Ok(())
}

fn persist_conversation_state_tx(
    tx: &rusqlite::Transaction<'_>,
    task: &ClaimedTask,
    conversation_state: &ConversationState,
) -> Result<()> {
    let user_key = effective_user_key(task);
    let state_json = serde_json::to_string(conversation_state)?;
    tx.execute(
        "INSERT INTO conversation_states (
            user_id, chat_id, user_key, state_json, last_task_id, updated_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(user_id, chat_id, user_key) DO UPDATE SET
            state_json = excluded.state_json,
            last_task_id = excluded.last_task_id,
            updated_at_ts = excluded.updated_at_ts",
        params![
            task.user_id,
            task.chat_id,
            user_key,
            state_json,
            conversation_state.last_task_id,
            conversation_state.updated_at_ts as i64,
        ],
    )?;
    Ok(())
}

pub(crate) fn load_active_conversation_state(
    state: &AppState,
    task: &ClaimedTask,
) -> Option<ConversationState> {
    let db = state.core.db.get().ok()?;
    let user_key = effective_user_key(task);
    let mut stmt = db
        .prepare(
            "SELECT state_json
             FROM conversation_states
             WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3",
        )
        .ok()?;
    let state_json = stmt
        .query_row(params![task.user_id, task.chat_id, user_key], |row| {
            row.get::<_, String>(0)
        })
        .ok()?;
    serde_json::from_str::<ConversationState>(&state_json).ok()
}

#[cfg(test)]
pub(crate) fn replace_active_conversation_state_with_pointers(
    state: &AppState,
    task: &ClaimedTask,
    payload: Option<&Value>,
    pointers: ActiveSessionPointers,
) {
    let prior_state = load_active_conversation_state(state, task);
    let conversation_state = ConversationState {
        active_followup_task_id: pointers.active_followup_task_id,
        active_clarify_task_id: pointers.active_clarify_task_id,
        active_observed_facts_task_id: pointers.active_observed_facts_task_id,
        alias_bindings: prior_state
            .as_ref()
            .map(|state| state.alias_bindings.clone())
            .unwrap_or_default(),
        last_primary_task_prompt: prior_state
            .as_ref()
            .and_then(|state| state.last_primary_task_prompt.clone()),
        last_primary_task_output: prior_state
            .as_ref()
            .and_then(|state| state.last_primary_task_output.clone()),
        locale_hint: effective_locale_hint(prior_state.as_ref(), payload),
        last_task_id: task.task_id.clone(),
        updated_at_ts: crate::now_ts_u64(),
    };
    if let Err(err) = persist_conversation_state(state, task, &conversation_state) {
        tracing::warn!(
            "conversation_state persist failed task_id={} err={}",
            task.task_id,
            err
        );
    }
}

pub(crate) fn update_active_session_from_ask_outcome(
    state: &AppState,
    task: &ClaimedTask,
    payload: Option<&Value>,
    prompt: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    resolved_prompt_for_execution: &str,
    answer_text: &str,
    answer_messages: &[String],
    semantic_clarify: bool,
    fuzzy_locator_suggestions: &[String],
    journal: &crate::task_journal::TaskJournal,
    clarify_fallback_source: Option<crate::fallback::ClarifyFallbackSource>,
) {
    let prior_session_snapshot = load_active_session_snapshot(state, task);
    let prior_state = load_active_conversation_state(state, task);
    let preserve_primary_task_for_clarifying_output =
        active_primary_non_success_preserves_prior(turn_analysis, journal)
            || model_fallback_preserves_primary_state(clarify_fallback_source, journal);
    let mut db = match state.core.db.get() {
        Ok(db) => db,
        Err(err) => {
            tracing::warn!(
                "conversation_state tx acquire failed task_id={} err={}",
                task.task_id,
                err
            );
            return;
        }
    };
    let clear_active_session_pointers_for_alias_update =
        turn_analysis_has_alias_only_state_patch(turn_analysis);
    let current_outcome_refreshes_session_pointers =
        current_outcome_has_ordered_entries(journal, semantic_clarify);
    let preserve_active_session_pointers = !clear_active_session_pointers_for_alias_update
        && !current_outcome_refreshes_session_pointers
        && (should_preserve_active_session_pointers(turn_analysis)
            || preserve_primary_task_for_clarifying_output);
    if preserve_active_session_pointers {
        tracing::info!(
            "conversation_state preserve_active_session_pointers task_id={} turn_type={}",
            task.task_id,
            turn_analysis
                .and_then(|analysis| analysis.turn_type)
                .map(crate::intent_router::TurnType::as_str)
                .unwrap_or("unknown")
        );
    }
    let tx = match db.transaction() {
        Ok(tx) => tx,
        Err(err) => {
            tracing::warn!(
                "conversation_state begin tx failed task_id={} err={}",
                task.task_id,
                err
            );
            return;
        }
    };
    let result: Result<()> = (|| {
        let (active_followup_task_id, active_clarify_task_id, active_observed_facts_task_id) =
            if preserve_active_session_pointers {
                (
                    prior_state
                        .as_ref()
                        .and_then(|state| state.active_followup_task_id.clone()),
                    prior_state
                        .as_ref()
                        .and_then(|state| state.active_clarify_task_id.clone()),
                    prior_state
                        .as_ref()
                        .and_then(|state| state.active_observed_facts_task_id.clone()),
                )
            } else if clear_active_session_pointers_for_alias_update {
                (None, None, None)
            } else {
                let active_followup_task_id =
                    crate::followup_frame::sync_active_frame_from_ask_outcome_tx(
                        &tx,
                        task,
                        prompt,
                        route_result,
                        answer_text,
                        answer_messages,
                        semantic_clarify,
                        journal,
                        prior_session_snapshot.active_followup_frame.as_ref(),
                    )?;
                let active_clarify_task_id =
                    crate::clarify_state::sync_active_clarify_state_from_ask_outcome_tx(
                        &tx,
                        task,
                        prompt,
                        route_result,
                        answer_text,
                        answer_messages,
                        semantic_clarify,
                        fuzzy_locator_suggestions,
                        Some(&prior_session_snapshot),
                    )?;
                let active_observed_facts_task_id =
                    crate::observed_facts::sync_active_observed_facts_from_ask_outcome_tx(
                        &tx,
                        task,
                        prompt,
                        route_result,
                        answer_text,
                        answer_messages,
                        journal,
                    )?;
                (
                    active_followup_task_id,
                    active_clarify_task_id,
                    active_observed_facts_task_id,
                )
            };
        let mut last_primary_task_prompt = if preserve_primary_task_for_clarifying_output {
            prior_state
                .as_ref()
                .and_then(|state| state.last_primary_task_prompt.clone())
        } else {
            next_last_primary_task_prompt(
                prior_state.as_ref(),
                route_result,
                turn_analysis,
                journal,
                prompt,
                resolved_prompt_for_execution,
            )
        };
        let last_primary_task_output = if preserve_primary_task_for_clarifying_output {
            prior_state
                .as_ref()
                .and_then(|state| state.last_primary_task_output.clone())
        } else {
            next_last_primary_task_output(
                prior_state.as_ref(),
                route_result,
                turn_analysis,
                journal,
                resolved_prompt_for_execution,
                answer_text,
                answer_messages,
            )
        };
        if last_primary_task_prompt.is_none() && last_primary_task_output.is_some() {
            last_primary_task_prompt = unannotated_chat_primary_prompt_for_output(
                prior_state.as_ref(),
                route_result,
                turn_analysis,
                prompt,
                resolved_prompt_for_execution,
                answer_text,
                answer_messages,
            );
        }
        let conversation_state = ConversationState {
            active_followup_task_id,
            active_clarify_task_id,
            active_observed_facts_task_id,
            alias_bindings: merge_alias_bindings_for_turn(
                prior_state.as_ref(),
                turn_analysis,
                prompt,
                route_result,
                resolved_prompt_for_execution,
            ),
            last_primary_task_prompt,
            last_primary_task_output,
            locale_hint: effective_locale_hint(prior_state.as_ref(), payload),
            last_task_id: task.task_id.clone(),
            updated_at_ts: crate::now_ts_u64(),
        };
        persist_conversation_state_tx(&tx, task, &conversation_state)?;
        tx.commit()?;
        Ok(())
    })();
    if let Err(err) = result {
        tracing::warn!(
            "conversation_state transactional sync failed task_id={} err={}",
            task.task_id,
            err
        );
    }
}

fn should_preserve_active_session_pointers(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    let Some(turn_type) = turn_analysis.and_then(|analysis| analysis.turn_type) else {
        return false;
    };
    matches!(
        turn_type,
        crate::intent_router::TurnType::RunControl
            | crate::intent_router::TurnType::ApprovalDecision
            | crate::intent_router::TurnType::StatusQuery
            | crate::intent_router::TurnType::FeedbackOrError
            | crate::intent_router::TurnType::PreferenceOrMemory
    )
}

fn current_outcome_has_ordered_entries(
    journal: &crate::task_journal::TaskJournal,
    semantic_clarify: bool,
) -> bool {
    !semantic_clarify
        && !crate::followup_frame::derive_ordered_entries_from_journal(journal).is_empty()
}

fn load_authoritative_followup_frame(
    state: &AppState,
    task: &ClaimedTask,
    expected_task_id: Option<&str>,
) -> Option<crate::followup_frame::FollowupFrame> {
    let frame = crate::followup_frame::load_active_followup_frame(state, task)?;
    match expected_task_id {
        Some(expected) if frame.source_task_id == expected => Some(frame),
        Some(_) => None,
        None => Some(frame),
    }
}

fn load_authoritative_clarify_state(
    state: &AppState,
    task: &ClaimedTask,
    expected_task_id: Option<&str>,
) -> Option<crate::clarify_state::ClarifyState> {
    let clarify_state = crate::clarify_state::load_active_clarify_state(state, task)?;
    match expected_task_id {
        Some(expected) if clarify_state.source_task_id == expected => Some(clarify_state),
        Some(_) => None,
        None => Some(clarify_state),
    }
}

fn load_authoritative_observed_facts(
    state: &AppState,
    task: &ClaimedTask,
    expected_task_id: Option<&str>,
) -> Option<crate::observed_facts::ObservedFacts> {
    let (facts, source_task_id) =
        crate::observed_facts::load_active_observed_facts_snapshot(state, task)?;
    match expected_task_id {
        Some(expected) if source_task_id == expected => Some(facts),
        Some(_) => None,
        None => Some(facts),
    }
}

pub(crate) fn load_active_session_snapshot(
    state: &AppState,
    task: &ClaimedTask,
) -> ActiveSessionSnapshot {
    let conversation_state = load_active_conversation_state(state, task);
    let (active_followup_frame, active_clarify_state, active_observed_facts) =
        if let Some(conversation_state) = conversation_state.as_ref() {
            (
                load_authoritative_followup_frame(
                    state,
                    task,
                    conversation_state.active_followup_task_id.as_deref(),
                ),
                load_authoritative_clarify_state(
                    state,
                    task,
                    conversation_state.active_clarify_task_id.as_deref(),
                ),
                load_authoritative_observed_facts(
                    state,
                    task,
                    conversation_state.active_observed_facts_task_id.as_deref(),
                ),
            )
        } else {
            (
                load_authoritative_followup_frame(state, task, None),
                load_authoritative_clarify_state(state, task, None),
                load_authoritative_observed_facts(state, task, None),
            )
        };
    ActiveSessionSnapshot {
        conversation_state,
        active_followup_frame,
        active_clarify_state,
        active_observed_facts,
    }
}

#[cfg(test)]
#[path = "conversation_state_tests.rs"]
mod tests;
