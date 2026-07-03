use tracing::info;

use super::{
    append_route_reason, bare_path_only_input_can_fill_active_observable_task,
    execution_finalize_style_for_contract, is_bare_path_only_input_for_clarify,
    push_unique_repair_code, route_trace_record, structured_execution_signal_for_effective_route,
    ContractRepairReport, IntentNormalizerOutput, TargetTaskPolicy, TurnAnalysis, TurnType,
};
use crate::{
    ActFinalizeStyle, FirstLayerDecision, IntentOutputContract, ResumeBehavior, ScheduleKind,
};

#[allow(clippy::too_many_arguments)]
pub(super) fn build_normalizer_output_with_final_gate(
    task: &crate::ClaimedTask,
    req: &str,
    req_surface: &crate::intent::surface_signals::PromptSurfaceSignals,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    resolved_user_intent: String,
    resume_behavior: ResumeBehavior,
    schedule_kind: ScheduleKind,
    schedule_intent: Option<crate::ScheduleIntentOutput>,
    wants_file_delivery: bool,
    should_refresh_long_term_memory: bool,
    agent_display_name_hint: String,
    needs_clarify: bool,
    clarify_question: String,
    mut reason: String,
    confidence: f64,
    output_contract: IntentOutputContract,
    execution_recipe_hint: Option<crate::execution_recipe::ExecutionRecipeSpec>,
    execution_recipe_plan_hint: Option<crate::intent_router::ExecutionRecipePlanHint>,
    execution_finalize_style: ActFinalizeStyle,
    turn_analysis: Option<TurnAnalysis>,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    contract_repair_report: &ContractRepairReport,
    repair_reasons: &[Option<&'static str>],
) -> IntentNormalizerOutput {
    let bare_path_only = is_bare_path_only_input_for_clarify(req, req_surface);
    let bare_path_fills_active_observable_task = bare_path_only
        && bare_path_only_input_can_fill_active_observable_task(
            session_snapshot,
            turn_type,
            target_task_policy,
            &output_contract,
        );
    let (needs_clarify_eff, clarify_question_eff) = if bare_path_only
        && bare_path_fills_active_observable_task
    {
        append_route_reason(&mut reason, "bare_path_fills_active_observable_task");
        info!(
            "{} intent_normalizer task_id={} bare_path_active_observable_fill needs_clarify=false path_token={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(req.trim())
        );
        (false, String::new())
    } else if !needs_clarify && bare_path_only {
        append_route_reason(&mut reason, "bare_path_no_verb");
        info!(
            "{} intent_normalizer task_id={} bare_path_no_verb_boundary_hint needs_clarify=false path_token={}",
            crate::highlight_tag("routing"),
            task.task_id,
            crate::truncate_for_log(req.trim())
        );
        (false, String::new())
    } else {
        (needs_clarify, clarify_question)
    };
    let bare_path_active_observable_boundary =
        !needs_clarify_eff && bare_path_only && bare_path_fills_active_observable_task;
    let structured_execution_signal = structured_execution_signal_for_effective_route(
        &output_contract,
        wants_file_delivery,
        schedule_kind,
        execution_recipe_hint,
    );
    let legacy_normalizer_decision_eff = if needs_clarify_eff {
        FirstLayerDecision::Clarify
    } else if bare_path_active_observable_boundary || structured_execution_signal {
        FirstLayerDecision::PlannerExecute
    } else {
        FirstLayerDecision::DirectAnswer
    };
    let execution_finalize_style_eff = if matches!(
        legacy_normalizer_decision_eff,
        FirstLayerDecision::PlannerExecute
    ) {
        if bare_path_active_observable_boundary {
            crate::post_route_policy::content_evidence_execution_finalize_style(
                &output_contract,
                false,
            )
            .unwrap_or_else(|| execution_finalize_style_for_contract(&output_contract))
        } else if structured_execution_signal {
            execution_finalize_style_for_contract(&output_contract)
        } else {
            execution_finalize_style
        }
    } else {
        ActFinalizeStyle::Plain
    };
    let mut route_trace_repair_codes = contract_repair_report
        .details
        .iter()
        .copied()
        .map(str::to_string)
        .collect::<Vec<_>>();
    for code in repair_reasons.iter().copied().flatten() {
        push_unique_repair_code(&mut route_trace_repair_codes, code);
    }
    let route_trace_record = route_trace_record(
        legacy_normalizer_decision_eff,
        needs_clarify_eff,
        &output_contract,
        route_trace_repair_codes,
    );
    let attachment_processing_required = turn_analysis
        .as_ref()
        .is_some_and(|analysis| analysis.attachment_processing_required);
    let boundary_envelope = crate::intent_router::BoundaryEnvelope::from_request(
        req.trim(),
        schedule_intent.clone(),
        attachment_processing_required,
        &output_contract,
        turn_analysis.as_ref(),
        resume_behavior,
    );
    info!(
        "{} intent_normalizer_route_trace task_id={} owner_layer={} reason_code={} outcome={} route_trace_decision={} needs_clarify={} output_contract_ref={} repair_codes={} repair_classes={}",
        crate::highlight_tag("routing"),
        task.task_id,
        route_trace_record.owner_layer,
        route_trace_record.reason_code,
        route_trace_record.outcome,
        route_trace_record.route_trace_decision.as_str(),
        route_trace_record.needs_clarify,
        route_trace_record.output_contract_ref,
        route_trace_record.repair_codes.join(","),
        route_trace_record.repair_classes.join(","),
    );
    let output = IntentNormalizerOutput {
        boundary_envelope,
        resolved_user_intent,
        resume_behavior,
        schedule_kind,
        schedule_intent,
        wants_file_delivery,
        should_refresh_long_term_memory,
        agent_display_name_hint,
        needs_clarify: needs_clarify_eff,
        clarify_question: clarify_question_eff,
        reason,
        confidence,
        output_contract,
        execution_recipe_hint,
        execution_recipe_plan_hint,
        execution_finalize_style: execution_finalize_style_eff,
        turn_analysis,
        fallback_source: None,
        route_trace_record,
    };
    log_boundary_envelope(task, &output);
    output
}

fn log_boundary_envelope(task: &crate::ClaimedTask, output: &IntentNormalizerOutput) {
    let envelope = output.boundary_envelope();
    info!(
        "{} intent_normalizer_boundary_envelope task_id={} raw_chars={} schedule_intent={} attachment_refs={} explicit_locators={} active_task_reference={} session_binding={} language_hint={} safety_budget_hint={}",
        crate::highlight_tag("routing"),
        task.task_id,
        envelope.raw_char_count(),
        envelope
            .schedule_intent
            .as_ref()
            .map(|intent| intent.kind.as_str())
            .unwrap_or("none"),
        envelope.attachment_refs.len(),
        envelope.explicit_locators.len(),
        envelope.active_task_reference.as_deref().unwrap_or("none"),
        envelope.session_binding.as_deref().unwrap_or("none"),
        envelope.language_hint.as_deref().unwrap_or("none"),
        envelope.safety_budget_hint.as_deref().unwrap_or("none"),
    );
}
