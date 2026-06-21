use serde_json::json;

use super::{
    apply_state_patch_structured_field_selector, execution_finalize_style_for_contract,
    first_layer_decision_gate_record, route_has_structured_execution_signal,
    IntentNormalizerOutput, RouteDecision, TurnAnalysis,
};
use crate::{
    ActFinalizeStyle, AppState, AskMode, ClaimedTask, FirstLayerDecision, ResumeBehavior,
    RiskCeiling, RouteResult,
};

fn ask_mode_from_normalizer_hint(
    normalizer_hint: FirstLayerDecision,
    finalize_style: ActFinalizeStyle,
) -> AskMode {
    match normalizer_hint {
        FirstLayerDecision::Clarify => AskMode::clarify(),
        FirstLayerDecision::DirectAnswer => AskMode::direct_answer(),
        FirstLayerDecision::PlannerExecute => AskMode::Act {
            finalize: finalize_style,
        },
    }
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
    let sanitized_answer_candidate =
        normalizer_answer_candidate_matches_backend_metadata(state, task, normalizer_out);
    let mut route_reason = normalizer_out.reason.clone();
    if sanitized_agent_display_name_hint {
        super::append_route_reason(
            &mut route_reason,
            "agent_display_name_hint_backend_metadata_removed",
        );
    }
    if sanitized_answer_candidate {
        super::append_route_reason(
            &mut route_reason,
            "normalizer_answer_candidate_backend_metadata_removed",
        );
    }
    apply_state_patch_structured_field_selector(
        &mut output_contract,
        normalizer_out
            .turn_analysis
            .as_ref()
            .and_then(|analysis| analysis.state_patch.as_ref()),
    );
    RouteResult {
        ask_mode: ask_mode_from_normalizer_hint(
            normalizer_out.legacy_first_layer_decision,
            normalizer_out.execution_finalize_style,
        ),
        resolved_intent: normalizer_out.resolved_user_intent.clone(),
        needs_clarify: normalizer_out.needs_clarify,
        clarify_question: normalizer_out.clarify_question.clone(),
        route_reason,
        route_confidence: Some(normalizer_out.confidence),
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

fn normalizer_answer_candidate_matches_backend_metadata(
    state: &AppState,
    task: &ClaimedTask,
    normalizer_out: &IntentNormalizerOutput,
) -> bool {
    normalizer_answer_candidate_from_resolved_intent(&normalizer_out.resolved_user_intent)
        .is_some_and(|candidate| normalizer_text_matches_backend_metadata(state, task, candidate))
}

fn normalizer_answer_candidate_from_resolved_intent(resolved_intent: &str) -> Option<&str> {
    let (_intent, candidate) = resolved_intent.rsplit_once("\nanswer_candidate:")?;
    let candidate = candidate.trim();
    (!candidate.is_empty()).then_some(candidate)
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
    let mut legacy_normalizer_decision = legacy_normalizer_decision;
    let mut execution_finalize_style =
        execution_finalize_style_for_contract(&decision.output_contract);
    if let Some(finalize_style) =
        crate::post_route_policy::content_evidence_execution_finalize_style(
            &decision.output_contract,
            decision.needs_clarify,
        )
    {
        legacy_normalizer_decision = FirstLayerDecision::PlannerExecute;
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
    let first_layer_gate_record = first_layer_decision_gate_record(
        None,
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
        legacy_first_layer_decision: legacy_normalizer_decision,
        execution_finalize_style,
        turn_analysis,
        fallback_source,
        first_layer_gate_record,
    }
}
