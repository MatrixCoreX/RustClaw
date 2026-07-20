use std::path::Path;

use super::{AgentRunContext, LoopState};
use crate::{llm_gateway, AppState, ClaimedTask};

#[path = "observed_output_text.rs"]
mod output_text;
#[cfg(test)]
use output_text::non_code_markdown_text;
use output_text::{
    extract_answer_from_finalizer_envelope_text, freeform_observed_answer_fallback,
    strip_bare_json_language_prefix, ObservedAnswerFallbackOut,
};
#[path = "observed_output_transform.rs"]
mod output_transform;
pub(crate) use output_transform::{
    direct_answer_from_referenced_observation_i18n, transform_skill_formatted_output_candidate,
};
#[path = "observed_output_success.rs"]
mod output_success;
pub(crate) use output_success::{
    extract_latest_generic_successful_output, normalized_success_body_for_observed_output,
    GenericObservedOutput,
};
use output_success::{
    extract_latest_generic_successful_output_with_state, has_successful_step_for_skill,
    latest_successful_step_index,
};

#[path = "observed_output_listing.rs"]
mod output_listing;
#[cfg(test)]
use output_listing::route_prefers_direct_observed_answer_for_scalar;
pub(crate) use output_listing::scalar_route_prefers_structured_observed_answer;
use output_listing::{
    canonical_existing_path, count_answer_from_latest_fs_search, count_answer_from_latest_listing,
    current_turn_request_text, exact_scalar_path_selector,
    latest_successful_list_dir_answer_candidate, looks_like_shell_long_listing_line,
    normalized_listing_text, recent_file_path_candidate_for_scalar_path,
    resolve_listing_entry_full_path, route_allows_path_batch_scalar_path_observed_answer,
    route_allows_raw_listing_direct_answer, route_allows_scalar_read_range_direct_answer,
    route_prefers_plain_fs_search_paths, route_requests_exact_scalar_path,
    route_requests_scalar_count, route_requests_scalar_existence,
    route_scalar_has_plain_path_terminal_respond,
};

#[path = "observed_output_system_inventory.rs"]
mod output_system_inventory;
use output_system_inventory::{
    count_inventory_direct_answer_candidate, count_inventory_planned_file_dir_breakdown_answer,
    dir_compare_direct_answer_candidate, inventory_dir_direct_answer_candidate,
    inventory_dir_names, inventory_dir_observed_candidate, inventory_dir_scalar_path_candidate,
    system_basic_existence_with_path_value, system_basic_info_scalar_path_candidate,
    system_basic_info_value, system_basic_inventory_dir_value,
    system_basic_structured_doc_observed_body, system_basic_structured_doc_value,
    system_basic_value_looks_like_info, tree_summary_direct_answer_candidate,
};

#[path = "observed_output_fs_search.rs"]
mod output_fs_search;
use output_fs_search::{
    absolutize_fs_search_answer_paths, fs_search_contract_listing_candidate,
    fs_search_direct_answer_candidate, fs_search_find_name_observed_candidate,
    fs_search_find_name_results, fs_search_grep_text_observed_candidate,
    fs_search_route_filtered_listing_candidate, fs_search_scalar_candidate,
    normalized_find_name_pattern, preferred_fs_search_exact_match,
};

#[path = "observed_output_path_facts.rs"]
mod output_path_facts;
use output_path_facts::*;

#[path = "observed_output_entries.rs"]
mod output_entries;
pub(crate) use output_entries::has_observed_answer_candidates;
#[cfg(test)]
use output_entries::recent_generated_output_from_user_request;
use output_entries::{
    compound_listing_content_delivery_guard_entry, cross_turn_observed_output_entries,
    execution_failed_step_guard_entry, observed_output_entries,
};

#[path = "observed_output_direct_scalar.rs"]
mod output_direct_scalar;
use output_direct_scalar::{
    selected_capability_result_exact_candidate, selected_capability_result_scalar_candidate,
    structured_scalar_candidate,
};

#[path = "observed_output_direct_answer.rs"]
mod output_direct_answer;
use output_direct_answer::{
    allows_normalized_scalar_direct_fallback, fs_search_output_direct_answer_candidate,
};
#[cfg(test)]
pub(crate) use output_direct_answer::{
    answer_is_direct_observation_passthrough, extract_direct_answer_from_generic_output,
    extract_direct_answer_from_generic_output_i18n,
};
pub(crate) use output_direct_answer::{
    answer_matches_observed_output_passthrough, extract_answer_from_observed_output,
    extract_answer_from_observed_output_i18n,
};

#[path = "observed_output_read_range.rs"]
mod output_read_range;
use output_read_range::{
    compose_content_excerpt_with_summary_answer, content_excerpt_summary_direct_answer_candidate,
    normalize_read_range_excerpt_for_direct_answer, read_range_observed_candidate,
    read_range_preserve_blank_lines,
};
pub(crate) use output_read_range::{
    normalize_read_range_excerpt, tail_read_range_direct_answer_candidate,
};

#[path = "observed_output_structured_scalar.rs"]
mod output_structured_scalar;
#[cfg(test)]
pub(crate) use output_structured_scalar::latest_structured_scalar_observation_text;
pub(crate) use output_structured_scalar::recent_structured_scalar_observation_count;
#[cfg(test)]
use output_structured_scalar::structured_scalar_observation_from_extract_item;
use output_structured_scalar::{
    multiple_structured_scalar_observations_need_synthesis,
    structured_scalar_observation_from_value,
};

#[path = "observed_output_structured_fields.rs"]
mod output_structured_fields;
use output_structured_fields::*;

#[path = "observed_output_route_policy.rs"]
mod output_route_policy;
use output_route_policy::{
    observed_language_supports_bilingual_template, observed_request_language_hint,
    observed_request_prefers_english_template, observed_response_style_hint,
    route_should_synthesize_non_bilingual_existence_with_path,
};
pub(crate) use output_route_policy::{
    route_disallows_direct_observation_passthrough, route_requires_synthesized_delivery,
};

#[path = "observed_output_process_service.rs"]
mod output_process_service;
use output_process_service::process_basic_observed_candidate;

#[path = "observed_output_scalar_text.rs"]
mod output_scalar_text;
use output_scalar_text::{
    normalized_scalar_candidate, scalar_count_diagnostic_line_for_answer, trim_for_observed_prompt,
    value_scalar_text, value_structured_text,
};

#[path = "observed_output_status_json.rs"]
mod output_status_json;
use output_status_json::multi_status_json_summary_candidate;

#[path = "observed_output_machine_candidates.rs"]
mod output_machine_candidates;
pub(crate) use output_machine_candidates::multi_field_machine_record_is_language_neutral;
use output_machine_candidates::*;

#[cfg(test)]
const OBSERVED_ANSWER_FALLBACK_PROMPT_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/observed_answer_fallback_prompt.md");
const OBSERVED_ANSWER_FALLBACK_PROMPT_LOGICAL_PATH: &str =
    "prompts/observed_answer_fallback_prompt.md";
const OBSERVED_ANSWER_FALLBACK_COMPACT_PROMPT_LOGICAL_PATH: &str =
    "prompts/observed_answer_fallback_compact_prompt.md";
fn extract_direct_scalar_from_generic_output_with_locator_hint_impl(
    state: Option<&AppState>,
    route: Option<&crate::IntentOutputContract>,
    loop_state: &LoopState,
    locator_hint: Option<&str>,
    auto_locator_path: Option<&str>,
    prefer_full_path: bool,
    allow_localized_direct_template: bool,
    prefer_english: bool,
) -> Option<String> {
    if let Some(answer) =
        selected_capability_result_scalar_candidate(route, &loop_state.capability_results)
    {
        return evidence_policy_checked_direct_candidate(
            route,
            loop_state,
            auto_locator_path,
            answer,
        );
    }
    if let Some(path) = recent_file_path_candidate_for_scalar_path(loop_state, route) {
        return evidence_policy_checked_direct_candidate(
            route,
            loop_state,
            auto_locator_path,
            path,
        );
    }
    if let Some(answer) = latest_successful_list_dir_answer_candidate(
        loop_state,
        Some(crate::OutputResponseShape::Scalar),
        auto_locator_path,
        prefer_full_path,
    ) {
        if !crate::finalize::looks_like_planner_artifact(&answer)
            && !crate::finalize::looks_like_internal_trace_artifact(&answer)
        {
            return evidence_policy_checked_direct_candidate(
                route,
                loop_state,
                auto_locator_path,
                answer,
            );
        }
    }
    let observed_output = extract_latest_generic_successful_output_with_state(state, loop_state)?;
    if route_should_synthesize_non_bilingual_existence_with_path(
        route,
        allow_localized_direct_template,
    ) {
        return None;
    }
    if multiple_structured_scalar_observations_need_synthesis(route, loop_state) {
        return None;
    }
    let answer = structured_scalar_candidate(
        state,
        route,
        &observed_output.skill,
        &observed_output.body,
        locator_hint.filter(|hint| !hint.trim().is_empty()),
        auto_locator_path,
        prefer_full_path,
        prefer_english,
    )
    .or_else(|| {
        allows_normalized_scalar_direct_fallback(
            &observed_output.skill,
            route,
            route.map(|route| route.response_shape),
        )
        .then(|| normalized_scalar_candidate(&observed_output.body))
        .flatten()
    })?;
    if crate::finalize::looks_like_planner_artifact(&answer)
        || crate::finalize::looks_like_internal_trace_artifact(&answer)
    {
        return None;
    }
    evidence_policy_checked_direct_candidate(route, loop_state, auto_locator_path, answer)
}

fn evidence_policy_checked_direct_candidate(
    route: Option<&crate::IntentOutputContract>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    answer: String,
) -> Option<String> {
    let Some(route) = route else {
        return Some(answer);
    };
    if latest_observation_lacks_required_content_evidence(route, loop_state) {
        return None;
    }
    if system_basic_scalar_path_candidate_satisfies_contract(route, loop_state, &answer) {
        return Some(answer);
    }
    let requires_evidence_policy_grounding =
        route_requires_evidence_policy_grounding_for_direct_candidate(route);
    if requires_evidence_policy_grounding
        && evidence_policy_direct_candidate_satisfies_contract(
            route,
            loop_state,
            auto_locator_path,
            &answer,
        )
    {
        return Some(answer);
    }
    if requires_evidence_policy_grounding {
        return None;
    }
    Some(answer)
}

fn system_basic_scalar_path_candidate_satisfies_contract(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
    answer: &str,
) -> bool {
    if !route_requests_exact_scalar_path(route) {
        return false;
    }
    let answer = answer.trim();
    if answer.is_empty()
        || crate::finalize::looks_like_planner_artifact(answer)
        || crate::finalize::looks_like_internal_trace_artifact(answer)
    {
        return false;
    }
    loop_state.executed_step_results.iter().rev().any(|step| {
        step.is_ok()
            && step.skill == "system_basic"
            && step
                .output
                .as_deref()
                .and_then(|body| system_basic_info_value("system_basic", body))
                .and_then(|value| system_basic_info_scalar_path_candidate(&value))
                .is_some_and(|candidate| candidate.trim() == answer)
    })
}

fn route_requires_evidence_policy_grounded_direct_candidate(
    route: &crate::IntentOutputContract,
) -> bool {
    crate::evidence_policy::final_answer_shape_for_output_contract(route)
        .is_some_and(|shape| !shape.allows_model_language())
}

fn route_requires_evidence_policy_grounding_for_direct_candidate(
    route: &crate::IntentOutputContract,
) -> bool {
    route_requires_evidence_policy_grounded_direct_candidate(route)
}

fn evidence_policy_direct_candidate_satisfies_contract(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    candidate: &str,
) -> bool {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("observed-output-direct-candidate", "ask", "");
    journal.record_output_contract(&route.clone());
    for step in &loop_state.executed_step_results {
        journal.push_step_result(step);
    }
    if let Some(path) = auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "auto_locator_path".to_string(),
            skill: "auto_locator".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(
                serde_json::json!({
                    "action": "auto_locator",
                    "path": path,
                    "resolved_path": path,
                })
                .to_string(),
            ),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
    }
    let answer_contract = crate::answer_verifier::AnswerContract::new("", route.clone());
    crate::answer_verifier::structurally_satisfies_answer_contract(
        &answer_contract,
        &journal,
        candidate,
    )
}

fn latest_observation_lacks_required_content_evidence(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
) -> bool {
    if !route_uses_enforced_generic_path_content_profile(route) {
        return false;
    }
    let Some(step) = loop_state.executed_step_results.iter().rev().find(|step| {
        step.is_ok()
            && !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think" | "answer_verifier"
            )
    }) else {
        return false;
    };
    if step.skill == "run_cmd" && !step_provides_path_content_evidence(step) {
        return true;
    }
    let args = step
        .output
        .as_deref()
        .and_then(|body| serde_json::from_str::<serde_json::Value>(body.trim()).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    crate::evidence_policy::action_policy_for_output_contract(Some(route), &step.skill, &args)
        .is_some_and(|policy| {
            policy.decision == crate::evidence_policy::ActionPolicyDecision::RejectedForbidden
        })
}

fn step_provides_path_content_evidence(step: &crate::executor::StepExecutionResult) -> bool {
    let Some(body) = step
        .output
        .as_deref()
        .map(str::trim)
        .filter(|body| !body.is_empty())
    else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return false;
    };
    let value = structured_observed_body_value(&value);
    match step.skill.as_str() {
        "system_basic" | "fs_basic" => {
            matches!(
                value.get("action").and_then(serde_json::Value::as_str),
                Some("read_range")
            ) && value
                .get("excerpt")
                .or_else(|| value.get("content"))
                .or_else(|| value.get("text"))
                .and_then(serde_json::Value::as_str)
                .is_some_and(|text| !text.trim().is_empty())
        }
        "doc_parse" => value
            .get("content")
            .or_else(|| value.get("text"))
            .or_else(|| value.get("excerpt"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|text| !text.trim().is_empty()),
        "log_analyze" => value
            .get("recent_matches")
            .or_else(|| value.get("recent_notable_lines"))
            .and_then(serde_json::Value::as_array)
            .is_some_and(|items| !items.is_empty()),
        _ => false,
    }
}

fn route_uses_enforced_generic_path_content_profile(route: &crate::IntentOutputContract) -> bool {
    output_route_policy::route_is_unclassified_contract(route)
        && route.requires_content_evidence
        && !route.delivery_required
        && route.response_shape == crate::OutputResponseShape::Free
        && matches!(
            route.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::CurrentWorkspace
        )
}

#[cfg(test)]
pub(crate) fn extract_direct_scalar_from_generic_output_with_locator_hint(
    loop_state: &LoopState,
    locator_hint: Option<&str>,
    auto_locator_path: Option<&str>,
    prefer_full_path: bool,
) -> Option<String> {
    extract_direct_scalar_from_generic_output_with_locator_hint_impl(
        None,
        None,
        loop_state,
        locator_hint,
        auto_locator_path,
        prefer_full_path,
        true,
        false,
    )
}

pub(crate) fn extract_direct_scalar_from_generic_output(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.output_contract());
    let auto_locator_path = agent_run_context
        .and_then(|ctx| ctx.auto_locator_path.as_deref())
        .filter(|path| !path.trim().is_empty());
    let prefer_full_path = route.is_some_and(route_requests_exact_scalar_path);
    let request_language_hint = current_turn_request_text(route, agent_run_context)
        .map(observed_request_language_hint)
        .unwrap_or("config_default");
    let allow_localized_direct_template =
        observed_language_supports_bilingual_template(request_language_hint);
    if route_should_synthesize_non_bilingual_existence_with_path(
        route,
        allow_localized_direct_template,
    ) {
        return None;
    }
    if let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) {
        if multiple_structured_scalar_observations_need_synthesis(Some(route), loop_state) {
            return None;
        }
        if let Some(answer) =
            count_inventory_planned_file_dir_breakdown_answer(None, loop_state, false)
        {
            return evidence_policy_checked_direct_candidate(
                Some(route),
                loop_state,
                auto_locator_path,
                answer,
            );
        }
        if let Some(answer) = count_answer_from_latest_listing(route, loop_state) {
            return evidence_policy_checked_direct_candidate(
                Some(route),
                loop_state,
                auto_locator_path,
                answer,
            );
        }
        if let Some(answer) = count_answer_from_latest_fs_search(route, loop_state) {
            return evidence_policy_checked_direct_candidate(
                Some(route),
                loop_state,
                auto_locator_path,
                answer,
            );
        }
    }
    let locator_hint = route.map(|route| route.locator_hint.as_str());
    extract_direct_scalar_from_generic_output_with_locator_hint_impl(
        None,
        route,
        loop_state,
        locator_hint,
        auto_locator_path,
        prefer_full_path,
        allow_localized_direct_template,
        false,
    )
}

pub(crate) fn extract_direct_scalar_from_generic_output_i18n(
    loop_state: &LoopState,
    state: &AppState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|ctx| ctx.output_contract());
    let auto_locator_path = agent_run_context
        .and_then(|ctx| ctx.auto_locator_path.as_deref())
        .filter(|path| !path.trim().is_empty());
    let prefer_full_path = route.is_some_and(route_requests_exact_scalar_path);
    let request_language_hint = current_turn_request_text(route, agent_run_context)
        .map(crate::language_policy::request_language_hint)
        .unwrap_or("config_default");
    let prefer_english =
        observed_request_prefers_english_template(Some(state), request_language_hint);
    let allow_localized_direct_template =
        observed_language_supports_bilingual_template(request_language_hint);
    if let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) {
        if multiple_structured_scalar_observations_need_synthesis(Some(route), loop_state) {
            return None;
        }
        if let Some(answer) = count_inventory_planned_file_dir_breakdown_answer(
            Some(state),
            loop_state,
            prefer_english,
        ) {
            return evidence_policy_checked_direct_candidate(
                Some(route),
                loop_state,
                auto_locator_path,
                answer,
            );
        }
        if let Some(answer) = count_answer_from_latest_listing(route, loop_state) {
            return evidence_policy_checked_direct_candidate(
                Some(route),
                loop_state,
                auto_locator_path,
                answer,
            );
        }
        if let Some(answer) = count_answer_from_latest_fs_search(route, loop_state) {
            return evidence_policy_checked_direct_candidate(
                Some(route),
                loop_state,
                auto_locator_path,
                answer,
            );
        }
    }
    let locator_hint = route.map(|route| route.locator_hint.as_str());
    extract_direct_scalar_from_generic_output_with_locator_hint_impl(
        Some(state),
        route,
        loop_state,
        locator_hint,
        auto_locator_path,
        prefer_full_path,
        allow_localized_direct_template,
        prefer_english,
    )
}

fn replace_internal_missing_sentinel_with_structured_observation(
    answer: &str,
    state: &AppState,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    if !is_internal_missing_scalar_sentinel(answer) {
        return None;
    }
    extract_answer_from_observed_output_i18n(loop_state, state, agent_run_context)
        .or_else(|| {
            extract_direct_scalar_from_generic_output_i18n(loop_state, state, agent_run_context)
        })
        .map(|replacement| replacement.trim().to_string())
        .filter(|replacement| !replacement.is_empty())
        .filter(|replacement| !is_internal_missing_scalar_sentinel(replacement))
}

fn observed_contract_json(agent_run_context: Option<&AgentRunContext>) -> String {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) else {
        return "{}".to_string();
    };
    let direct_observation_passthrough_allowed =
        !route_disallows_direct_observation_passthrough(route);
    let final_answer_shape = crate::evidence_policy::final_answer_shape_for_output_contract(route);
    serde_json::json!({
        "response_shape": route.response_shape.as_str(),
        "final_answer_shape": final_answer_shape.map(crate::evidence_policy::FinalAnswerShape::as_str),
        "final_answer_shape_class": final_answer_shape.map(|shape| shape.class().as_str()),
        "exact_sentence_count": route.exact_sentence_count,
        "requires_content_evidence": route.requires_content_evidence,
        "delivery_required": route.delivery_required,
        "direct_observation_passthrough_allowed": direct_observation_passthrough_allowed,
        "locator_kind": route.locator_kind.as_str(),
        "delivery_intent": route.delivery_intent.as_str(),
        "locator_hint": route.locator_hint,
        "needs_clarify": false,
    })
    .to_string()
}

fn observed_answer_fallback_prompt_logical_path(
    agent_run_context: Option<&AgentRunContext>,
    observed_block: &str,
) -> &'static str {
    if observed_answer_fallback_can_use_compact_prompt(agent_run_context, observed_block) {
        OBSERVED_ANSWER_FALLBACK_COMPACT_PROMPT_LOGICAL_PATH
    } else {
        OBSERVED_ANSWER_FALLBACK_PROMPT_LOGICAL_PATH
    }
}

fn observed_answer_fallback_can_use_compact_prompt(
    agent_run_context: Option<&AgentRunContext>,
    observed_block: &str,
) -> bool {
    const MAX_COMPACT_OBSERVED_BLOCK_BYTES: usize = 12_000;
    if observed_block.len() > MAX_COMPACT_OBSERVED_BLOCK_BYTES {
        return false;
    }
    let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) else {
        return false;
    };
    if route.delivery_required
        || route.delivery_intent != crate::OutputDeliveryIntent::None
        || route.response_shape == crate::OutputResponseShape::FileToken
    {
        return false;
    }
    crate::evidence_policy::final_answer_shape_for_output_contract(route)
        .is_some_and(observed_answer_fallback_shape_can_use_compact_prompt)
}

fn observed_answer_fallback_shape_can_use_compact_prompt(
    shape: crate::evidence_policy::FinalAnswerShape,
) -> bool {
    use crate::evidence_policy::{FinalAnswerShape, FinalAnswerShapeClass};

    if matches!(
        shape.class(),
        FinalAnswerShapeClass::StrictList
            | FinalAnswerShapeClass::ScalarValue
            | FinalAnswerShapeClass::SinglePath
    ) {
        return true;
    }
    matches!(
        shape,
        FinalAnswerShape::ExistenceVerdictWithPath | FinalAnswerShape::RawOutputOrShortSummary
    )
}

fn resolved_user_intent(agent_run_context: Option<&AgentRunContext>, user_text: &str) -> String {
    current_turn_request_text(
        agent_run_context.and_then(AgentRunContext::output_contract),
        agent_run_context,
    )
    .unwrap_or_else(|| user_text.trim())
    .to_string()
}

pub(crate) async fn try_synthesize_answer_from_observed_output(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Result<Option<(String, crate::task_journal::TaskJournalFinalizerSummary)>, String> {
    // §3.3 Stage 3.2 invariant：observed-tier LLM 兜底属于 finalize 子层，
    // 进入时 ask_state 必须是 Executing 或 Finalizing。Executing 是因为
    // 该兜底由 finalize_loop_reply 里调用、ask_state 还没 transition 到
    // Finalizing；Finalizing 兼容 §3.1 后续把 transition 提前的可能。
    debug_assert!(
        matches!(
            state.current_ask_state(&task.task_id),
            None | Some(crate::AskState::Executing) | Some(crate::AskState::Finalizing)
        ),
        "synthesize_answer_from_observed_output invariant: ask_state must be Executing|Finalizing, got {:?} (task_id={})",
        state.current_ask_state(&task.task_id),
        task.task_id,
    );

    if let Some(answer) = strict_raw_tail_read_observed_answer(loop_state, agent_run_context) {
        return Ok(Some((
            answer,
            crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                parsed: true,
                contract_ok: true,
                completion_ok: Some(true),
                grounded_ok: Some(true),
                format_ok: Some(true),
                needs_clarify: Some(false),
                used_evidence_ids_count: 1,
                evidence_quotes_count: 0,
                ..Default::default()
            },
        )));
    }

    let mut observed_entries = observed_output_entries(loop_state);
    if let Some(guard) = execution_failed_step_guard_entry(
        loop_state,
        agent_run_context.and_then(|ctx| ctx.output_contract()),
    ) {
        observed_entries = vec![guard];
    } else {
        if let Some(guard) = multi_count_observation_guard_entry(loop_state) {
            observed_entries.insert(0, guard);
        }
        if let Some(guard) = compound_listing_content_delivery_guard_entry(
            loop_state,
            agent_run_context.and_then(|ctx| ctx.output_contract()),
        ) {
            observed_entries.insert(0, guard);
        }
    }
    if observed_entries.is_empty() {
        observed_entries = cross_turn_observed_output_entries(loop_state, agent_run_context);
    }
    if observed_entries.is_empty() {
        return Ok(None);
    }
    let observed_block = observed_entries.join("\n\n");
    let resolved_intent = resolved_user_intent(agent_run_context, user_text);
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_text);
    let user_request_for_prompt = crate::language_policy::task_original_user_text(task)
        .unwrap_or_else(|| user_text.trim().to_string());
    let prompt_logical_path =
        observed_answer_fallback_prompt_logical_path(agent_run_context, &observed_block);
    let (prompt_template, prompt_source) =
        match crate::bootstrap::load_required_prompt_template_for_state(state, prompt_logical_path)
        {
            Ok(resolved) => resolved,
            Err(err) => {
                tracing::warn!(
                    "observed_answer_fallback prompt_missing task_id={} err={}",
                    task.task_id,
                    err
                );
                return Err(format!("observed answer fallback prompt missing: {err}"));
            }
        };
    let response_style_hint = observed_response_style_hint(agent_run_context);
    let prompt = crate::render_prompt_template(
        &prompt_template,
        &[
            ("__USER_REQUEST__", &user_request_for_prompt),
            ("__RESOLVED_USER_INTENT__", &resolved_intent),
            (
                "__OUTPUT_CONTRACT__",
                &observed_contract_json(agent_run_context),
            ),
            ("__OBSERVED_OUTPUTS__", &observed_block),
            (
                "__CONFIG_RESPONSE_LANGUAGE__",
                &state.policy.command_intent.default_locale,
            ),
            ("__REQUEST_LANGUAGE_HINT__", &request_language_hint),
            ("__RESPONSE_STYLE_HINT__", &response_style_hint),
        ],
    );
    crate::log_prompt_render(
        state,
        &task.task_id,
        "observed_answer_fallback_prompt",
        &prompt_source,
        None,
    );
    let llm_out =
        llm_gateway::run_with_fallback_with_prompt_source(state, task, &prompt, &prompt_source)
            .await
            .map_err(|err| format!("observed answer fallback LLM failed: {err}"))?;
    let llm_out_for_parse = strip_bare_json_language_prefix(&llm_out);
    let (parsed, parsed_from_schema) = match crate::prompt_utils::validate_against_schema::<
        ObservedAnswerFallbackOut,
    >(
        llm_out_for_parse,
        crate::prompt_utils::PromptSchemaId::FinalizerOut,
    ) {
        Ok(validated) => {
            if !validated.raw_parse_ok {
                tracing::info!(
                    "observed_answer_fallback schema_parse_recovery task_id={} schema_normalized={}",
                    task.task_id,
                    validated.schema_normalized
                );
            }
            (Some(validated.value), true)
        }
        Err(err) => {
            tracing::info!(
                "observed_answer_fallback schema_validation_failed task_id={} err={}",
                task.task_id,
                err
            );
            (freeform_observed_answer_fallback(llm_out_for_parse), false)
        }
    };
    let Some(parsed) = parsed else {
        return Ok(None);
    };
    let mut answer = parsed.answer.trim().to_string();
    if let Some(unwrapped) = extract_answer_from_finalizer_envelope_text(&answer) {
        answer = unwrapped;
    }
    if let Some(replacement) = replace_internal_missing_sentinel_with_structured_observation(
        &answer,
        state,
        loop_state,
        agent_run_context,
    ) {
        tracing::info!(
            "observed_answer_fallback_replace_internal_missing_sentinel task_id={} replacement={}",
            task.task_id,
            crate::truncate_for_log(&replacement)
        );
        answer = replacement;
    }
    if let Some(diagnostic) = scalar_count_diagnostic_line_for_answer(
        &answer,
        agent_run_context.and_then(|ctx| ctx.output_contract()),
        loop_state,
    ) {
        tracing::info!(
            "observed_answer_fallback_replace_scalar_count_with_diagnostic task_id={} diagnostic={}",
            task.task_id,
            crate::truncate_for_log(&diagnostic)
        );
        answer = scalar_count_diagnostic_machine_answer(&diagnostic);
    }
    let direct_passthrough_disallowed = agent_run_context
        .and_then(|ctx| ctx.output_contract())
        .is_some_and(route_disallows_direct_observation_passthrough)
        && answer_matches_observed_output_passthrough(&answer, loop_state);
    if direct_passthrough_disallowed {
        tracing::info!(
            "observed_answer_fallback_reject_direct_passthrough task_id={} answer={}",
            task.task_id,
            crate::truncate_for_log(&answer)
        );
        answer.clear();
    }
    let route_result = agent_run_context.and_then(|ctx| ctx.output_contract());
    let language_compatible = observed_answer_language_compatible_for_route(
        route_result,
        loop_state,
        agent_run_context.and_then(|ctx| ctx.auto_locator_path.as_deref()),
        &answer,
        &request_language_hint,
    );
    if !answer.is_empty() && !language_compatible {
        tracing::info!(
            "observed_answer_fallback_reject_language_mismatch task_id={} language_hint={} answer={}",
            task.task_id,
            request_language_hint,
            crate::truncate_for_log(&answer)
        );
    }
    if !answer.is_empty() {
        let prefer_english = crate::fallback::fallback_prefers_english_for_language_hint(
            state,
            &request_language_hint,
        );
        answer = compose_content_excerpt_with_summary_answer(
            &answer,
            loop_state,
            prefer_english,
            agent_run_context,
        );
    }
    // §3.4 finalize-tier: 这里属于 observed_answer_fallback 兜底路径（finalize 层
    // 的 fallback 分支），是 semantic_judge LLM 入口的允许调用方之一。
    // Phase 0.2: 复用同一次 LLM 调用已经返回的 `publishable` + `is_meta_instruction`，
    // 高置信度时直接信任，避免再发一次 `semantic_judge::is_meta_respond_instruction`
    // 二次判定调用。低置信度（<0.55）时才回退到 semantic_judge 做安全兜底，
    // 保留"LLM 过保守错判为不可发"的救回链路。
    const OBSERVED_SELF_CLASSIFY_CONF_THRESHOLD: f64 = 0.55;
    let semantically_publishable = if !answer.is_empty() && !parsed.needs_clarify {
        if parsed.confidence >= OBSERVED_SELF_CLASSIFY_CONF_THRESHOLD {
            parsed.publishable && !parsed.is_meta_instruction
        } else if parsed.publishable {
            !parsed.is_meta_instruction
        } else {
            !crate::semantic_judge::is_meta_respond_instruction(state, task, &answer).await
        }
    } else {
        false
    };
    let qualified = !answer.is_empty()
        && !parsed.needs_clarify
        && !direct_passthrough_disallowed
        && language_compatible
        && (parsed.qualified || semantically_publishable);
    Ok(Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(if qualified {
                crate::finalize::FinalizerDisposition::QualifiedCompletion
            } else {
                crate::finalize::FinalizerDisposition::AllowFallback
            }),
            fallback: (!parsed_from_schema)
                .then_some(crate::task_journal::TaskJournalFinalizerFallback::FreeformText),
            parsed: parsed_from_schema,
            contract_ok: qualified,
            completion_ok: Some(qualified),
            grounded_ok: Some(qualified),
            format_ok: Some(qualified),
            needs_clarify: Some(parsed.needs_clarify),
            confidence: Some(parsed.confidence.clamp(0.0, 1.0)),
            used_evidence_ids_count: observed_entries.len(),
            evidence_quotes_count: 0,
            ..Default::default()
        },
    )))
}

fn strict_raw_tail_read_observed_answer(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|context| context.output_contract())?;
    if !output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::RawCommandOutput,
    ) || route.response_shape != crate::OutputResponseShape::Strict
        || !route.requires_content_evidence
        || route.delivery_required
    {
        return None;
    }
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && matches!(step.skill.as_str(), "fs_basic" | "system_basic"))
        .filter_map(|step| step.output.as_deref())
        .find_map(strict_raw_tail_read_answer_from_output)
        .map(|answer| answer.trim().to_string())
        .filter(|answer| !answer.is_empty())
}

fn strict_raw_tail_read_answer_from_output(output: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(output.trim()).ok()?;
    strict_raw_tail_read_answer_from_value(&value)
}

fn strict_raw_tail_read_answer_from_value(value: &serde_json::Value) -> Option<String> {
    if let Some(answer) = strict_raw_tail_read_answer_from_flat_value(value) {
        return Some(answer);
    }
    value
        .get("extra")
        .and_then(strict_raw_tail_read_answer_from_value)
}

fn strict_raw_tail_read_answer_from_flat_value(value: &serde_json::Value) -> Option<String> {
    if !matches!(
        value.get("action").and_then(serde_json::Value::as_str),
        Some("read_range" | "read_text_range")
    ) || value.get("mode").and_then(serde_json::Value::as_str) != Some("tail")
    {
        return None;
    }
    let requested_n = value
        .get("requested_n")
        .or_else(|| value.get("n"))
        .or_else(|| value.get("count"))
        .and_then(serde_json::Value::as_u64)?;
    if requested_n == 0 || requested_n > 50 {
        return None;
    }
    value
        .get("excerpt")
        .and_then(serde_json::Value::as_str)
        .filter(|excerpt| !excerpt.trim().is_empty())?;
    let mut candidate = value.clone();
    let obj = candidate.as_object_mut()?;
    obj.insert(
        "action".to_string(),
        serde_json::Value::String("read_range".to_string()),
    );
    if !obj.contains_key("requested_n") {
        obj.insert(
            "requested_n".to_string(),
            serde_json::Value::Number(requested_n.into()),
        );
    }
    tail_read_range_direct_answer_candidate(&candidate.to_string(), false)
}

pub(crate) async fn synthesize_answer_from_observed_output(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    match try_synthesize_answer_from_observed_output(
        state,
        task,
        user_text,
        loop_state,
        agent_run_context,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(err) => {
            tracing::warn!(
                "observed_answer_fallback unavailable task_id={} err={}",
                task.task_id,
                err
            );
            None
        }
    }
}

pub(crate) fn normalized_observed_listing(observed: &str) -> Option<String> {
    normalized_listing_text(observed)
}

#[cfg(test)]
#[path = "observed_output_empty_values_tests.rs"]
mod empty_values_tests;
#[cfg(test)]
#[path = "observed_output_tests.rs"]
mod tests;
