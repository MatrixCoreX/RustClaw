use serde_json::json;

use super::{
    apply_state_patch_structured_field_selector, execution_finalize_style_for_contract,
    route_has_structured_execution_signal, route_trace_record,
    state_patch_targets_task_lifecycle_fields, IntentNormalizerOutput, RouteDecision, TurnAnalysis,
};
use crate::{
    ActFinalizeStyle, AppState, AskMode, ClaimedTask, FirstLayerDecision, OutputSemanticKind,
    ResumeBehavior, RiskCeiling, RouteResult,
};

fn ask_mode_from_machine_route_state(
    needs_clarify: bool,
    output_contract: &crate::IntentOutputContract,
    wants_file_delivery: bool,
    schedule_kind: crate::ScheduleKind,
    execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
    finalize_style: ActFinalizeStyle,
) -> AskMode {
    if needs_clarify {
        return AskMode::clarify();
    }
    if route_has_structured_execution_signal(
        output_contract,
        wants_file_delivery,
        schedule_kind,
        execution_recipe_hint,
    ) {
        return AskMode::Act {
            finalize: finalize_style,
        };
    }
    AskMode::direct_answer()
}

pub(super) fn render_auth_policy_context(state: &AppState, task: &ClaimedTask) -> String {
    let auth_role = task
        .user_key
        .as_deref()
        .and_then(|user_key| {
            crate::resolve_auth_identity_by_key(state, user_key)
                .ok()
                .flatten()
        })
        .map(|identity| identity.role)
        .unwrap_or_else(|| "unknown".to_string());
    let current_process_cwd = std::env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string());
    format!(
        "current_auth_role: {auth_role}\nallow_path_outside_workspace_for_task: {}\nallow_sudo_for_task: {}\nworkspace_root: {}\ncurrent_process_cwd: {}",
        crate::skills::task_allows_path_outside_workspace(state, Some(task)),
        crate::skills::task_allows_sudo(state, Some(task)),
        state.skill_rt.workspace_root.display(),
        current_process_cwd
    )
}

pub(crate) fn route_result_from_normalizer(
    state: &AppState,
    task: &ClaimedTask,
    normalizer_out: &IntentNormalizerOutput,
) -> RouteResult {
    let _turn_analysis_present = normalizer_out.turn_analysis.is_some();
    let mut output_contract = normalizer_out.output_contract.clone();
    let (agent_display_name_hint, sanitized_agent_display_name_hint) =
        sanitize_normalizer_agent_display_name_hint(state, task, normalizer_out);
    let mut route_reason = normalizer_out.reason.clone();
    if sanitized_agent_display_name_hint {
        super::append_route_reason(
            &mut route_reason,
            "agent_display_name_hint_backend_metadata_removed",
        );
    }
    apply_state_patch_structured_field_selector(
        &mut output_contract,
        normalizer_out
            .turn_analysis
            .as_ref()
            .and_then(|analysis| analysis.state_patch.as_ref()),
    );
    let mut needs_clarify = normalizer_out.needs_clarify;
    let mut clarify_question = normalizer_out.clarify_question.clone();
    let execution_finalize_style = normalizer_out.execution_finalize_style;
    if route_targets_task_lifecycle_query(normalizer_out, &route_reason) {
        output_contract.response_shape = crate::OutputResponseShape::Free;
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = false;
        output_contract.locator_kind = crate::OutputLocatorKind::None;
        output_contract.delivery_intent = crate::OutputDeliveryIntent::None;
        output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
        output_contract.locator_hint.clear();
        if output_contract
            .self_extension
            .structured_field_selector
            .as_deref()
            .is_none_or(|selector| selector.trim().is_empty())
        {
            output_contract.self_extension.structured_field_selector =
                Some("task_lifecycle.*".to_string());
        }
        needs_clarify = false;
        clarify_question.clear();
        super::append_route_reason(&mut route_reason, "capability_ref=task_control.list");
        super::append_route_reason(
            &mut route_reason,
            "task_lifecycle_machine_fields_bound_to_task_control",
        );
    }
    let ask_mode = ask_mode_from_machine_route_state(
        needs_clarify,
        &output_contract,
        normalizer_out.wants_file_delivery,
        normalizer_out.schedule_kind,
        normalizer_out.execution_recipe_hint,
        execution_finalize_style,
    );
    demote_output_contract_semantic_to_route_marker(&mut output_contract, &mut route_reason);
    RouteResult {
        ask_mode,
        resolved_intent: normalizer_out.resolved_user_intent.clone(),
        needs_clarify,
        clarify_question,
        route_reason,
        route_confidence: Some(normalizer_out.confidence),
        #[cfg(test)]
        visible_skill_candidates: state.planner_available_skills_for_task(task),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: normalizer_out.resume_behavior,
        schedule_kind: normalizer_out.schedule_kind,
        schedule_intent: normalizer_out.schedule_intent.clone(),
        wants_file_delivery: normalizer_out.wants_file_delivery,
        should_refresh_long_term_memory: normalizer_out.should_refresh_long_term_memory,
        agent_display_name_hint,
        output_contract,
    }
}

fn route_targets_task_lifecycle_query(
    normalizer_out: &IntentNormalizerOutput,
    route_reason: &str,
) -> bool {
    if normalizer_out
        .turn_analysis
        .as_ref()
        .and_then(|analysis| analysis.state_patch.as_ref())
        .is_some_and(|patch| state_patch_targets_task_lifecycle_fields(Some(patch)))
    {
        return true;
    }
    let fields = [
        task_lifecycle_machine_field_count(&normalizer_out.resolved_user_intent),
        task_lifecycle_machine_field_count(route_reason),
    ]
    .into_iter()
    .sum::<usize>();
    fields >= 2
}

fn task_lifecycle_machine_field_count(text: &str) -> usize {
    let mut fields = std::collections::BTreeSet::new();
    for token in text
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.')))
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        match token {
            "task_lifecycle.can_poll" | "can_poll" => {
                fields.insert("can_poll");
            }
            "task_lifecycle.can_cancel" | "can_cancel" => {
                fields.insert("can_cancel");
            }
            "task_lifecycle.checkpoint_id" | "checkpoint_id" => {
                fields.insert("checkpoint_id");
            }
            "task_lifecycle.next_check_after" | "next_check_after" => {
                fields.insert("next_check_after");
            }
            _ => {}
        }
    }
    fields.len()
}

fn demote_output_contract_semantic_to_route_marker(
    output_contract: &mut crate::IntentOutputContract,
    route_reason: &mut String,
) {
    let semantic_kind = output_contract.semantic_kind;
    if semantic_kind == OutputSemanticKind::None {
        return;
    }
    super::append_route_reason(
        route_reason,
        &format!("contract:{}", semantic_kind.as_str()),
    );
    super::append_route_reason(
        route_reason,
        "normalizer_semantic_contract_demoted_to_route_marker",
    );
    output_contract.semantic_kind = OutputSemanticKind::None;
}

fn sanitize_normalizer_agent_display_name_hint(
    state: &AppState,
    task: &ClaimedTask,
    normalizer_out: &IntentNormalizerOutput,
) -> (String, bool) {
    let hint = normalizer_out.agent_display_name_hint.trim();
    if hint.is_empty() || normalizer_out.should_refresh_long_term_memory {
        return (hint.to_string(), false);
    }
    if normalizer_text_matches_backend_metadata(state, task, hint) {
        (String::new(), true)
    } else {
        (hint.to_string(), false)
    }
}

fn normalizer_text_matches_backend_metadata(
    state: &AppState,
    task: &ClaimedTask,
    text: &str,
) -> bool {
    let normalized_text = normalize_backend_identity_token(text);
    if normalized_text.is_empty() {
        return false;
    }
    state.task_llm_providers(task).iter().any(|provider| {
        provider
            .config
            .name
            .trim()
            .strip_prefix("vendor-")
            .into_iter()
            .chain([
                provider.config.name.trim(),
                provider.config.model.trim(),
                provider.config.provider_type.trim(),
            ])
            .map(normalize_backend_identity_token)
            .filter(|token| token.len() >= 4)
            .any(|token| {
                normalized_text == token
                    || normalized_text.contains(&token)
                    || token.contains(&normalized_text)
            })
    })
}

fn normalize_backend_identity_token(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

pub(super) fn render_self_extension_runtime(state: &AppState) -> String {
    serde_json::to_string_pretty(&json!({
        "enabled": state.policy.self_extension.enabled,
        "auto_on_capability_gap": state.policy.self_extension.auto_on_capability_gap,
        "allow_execute": state.policy.self_extension.allow_execute,
        "allow_package_install": state.policy.self_extension.allow_package_install,
        "allow_permanent_extension": state.policy.self_extension.allow_permanent_extension,
        "allow_runtime_enable": state.policy.self_extension.allow_runtime_enable,
        "supported_modes": ["temporary_fix", "permanent_extension"],
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

pub(super) fn normalizer_output_from_fallback(
    user_request: &str,
    fallback_reason_prefix: &str,
    decision: RouteDecision,
    fallback_source: Option<crate::fallback::ClarifyFallbackSource>,
) -> IntentNormalizerOutput {
    normalizer_output_from_fallback_with_turn_analysis(
        user_request,
        fallback_reason_prefix,
        decision,
        fallback_source,
        None,
    )
}

pub(super) fn normalizer_output_from_fallback_with_turn_analysis(
    user_request: &str,
    fallback_reason_prefix: &str,
    decision: RouteDecision,
    fallback_source: Option<crate::fallback::ClarifyFallbackSource>,
    turn_analysis: Option<TurnAnalysis>,
) -> IntentNormalizerOutput {
    let legacy_normalizer_decision = if decision.needs_clarify {
        FirstLayerDecision::Clarify
    } else if route_has_structured_execution_signal(
        &decision.output_contract,
        decision.wants_file_delivery,
        decision.schedule_kind,
        None,
    ) {
        FirstLayerDecision::PlannerExecute
    } else {
        FirstLayerDecision::DirectAnswer
    };
    let mut execution_finalize_style =
        execution_finalize_style_for_contract(&decision.output_contract);
    if let Some(finalize_style) =
        crate::post_route_policy::content_evidence_execution_finalize_style(
            &decision.output_contract,
            decision.needs_clarify,
        )
    {
        execution_finalize_style = finalize_style;
    }
    let reason = if decision.reason.trim().is_empty() {
        fallback_reason_prefix.to_string()
    } else {
        format!("{fallback_reason_prefix}; {}", decision.reason.trim())
    };
    let resolved_user_intent = if decision.resolved_user_intent.trim().is_empty() {
        user_request.trim().to_string()
    } else {
        decision.resolved_user_intent
    };
    let trace_record = route_trace_record(
        legacy_normalizer_decision,
        decision.needs_clarify,
        &decision.output_contract,
        vec![fallback_reason_prefix.to_string()],
    );
    IntentNormalizerOutput {
        resolved_user_intent,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: decision.schedule_kind,
        schedule_intent: decision.schedule_intent,
        wants_file_delivery: decision.wants_file_delivery,
        should_refresh_long_term_memory: decision.should_refresh_long_term_memory,
        agent_display_name_hint: decision.agent_display_name_hint,
        needs_clarify: decision.needs_clarify,
        clarify_question: decision.clarify_question,
        reason,
        confidence: decision.confidence.unwrap_or(0.0),
        output_contract: decision.output_contract,
        execution_recipe_hint: None,
        execution_recipe_plan_hint: None,
        route_trace_decision: legacy_normalizer_decision,
        execution_finalize_style,
        turn_analysis,
        fallback_source,
        route_trace_record: trace_record,
    }
}
