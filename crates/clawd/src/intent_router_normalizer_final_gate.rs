use tracing::info;

use super::{
    append_route_reason, bare_path_only_input_can_fill_active_observable_task,
    execution_finalize_style_for_contract, first_layer_decision_gate_record,
    is_bare_path_only_input_for_clarify, push_unique_repair_code,
    structured_execution_signal_for_effective_route, ContractRepairReport, IntentNormalizerOutput,
    TargetTaskPolicy, TurnAnalysis, TurnType,
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
    legacy_normalizer_decision: FirstLayerDecision,
    execution_finalize_style: ActFinalizeStyle,
    turn_analysis: Option<TurnAnalysis>,
    turn_type: Option<TurnType>,
    target_task_policy: Option<TargetTaskPolicy>,
    parsed_decision: Option<FirstLayerDecision>,
    active_file_basename_answer_candidate_repair: Option<&'static str>,
    contract_repair_report: &ContractRepairReport,
    repair_reasons: &[Option<&'static str>],
) -> IntentNormalizerOutput {
    let bare_path_only = is_bare_path_only_input_for_clarify(req, req_surface);
    let bare_path_fills_active_observable_task = bare_path_only
        && bare_path_only_input_can_fill_active_observable_task(
            session_snapshot,
            turn_type,
            target_task_policy,
            legacy_normalizer_decision,
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
                "{} intent_normalizer task_id={} bare_path_no_verb_override needs_clarify=true path_token={}",
                crate::highlight_tag("routing"),
                task.task_id,
                crate::truncate_for_log(req.trim())
            );
        (true, String::new())
    } else {
        (needs_clarify, clarify_question)
    };
    let bare_path_promotes_to_execute = !needs_clarify_eff
        && bare_path_only
        && bare_path_fills_active_observable_task
        && matches!(legacy_normalizer_decision, FirstLayerDecision::Clarify);
    let structured_execution_signal = structured_execution_signal_for_effective_route(
        &output_contract,
        wants_file_delivery,
        schedule_kind,
        execution_recipe_hint,
        active_file_basename_answer_candidate_repair,
    );
    let legacy_normalizer_decision_eff = if needs_clarify_eff {
        FirstLayerDecision::Clarify
    } else if bare_path_promotes_to_execute
        || structured_execution_signal
        || matches!(
            legacy_normalizer_decision,
            FirstLayerDecision::PlannerExecute
        )
    {
        FirstLayerDecision::PlannerExecute
    } else {
        FirstLayerDecision::DirectAnswer
    };
    let execution_finalize_style_eff = if matches!(
        legacy_normalizer_decision_eff,
        FirstLayerDecision::PlannerExecute
    ) {
        if bare_path_promotes_to_execute {
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
    let mut first_layer_repair_codes = contract_repair_report
        .details
        .iter()
        .copied()
        .map(str::to_string)
        .collect::<Vec<_>>();
    for code in repair_reasons.iter().copied().flatten() {
        push_unique_repair_code(&mut first_layer_repair_codes, code);
    }
    let first_layer_gate_record = first_layer_decision_gate_record(
        parsed_decision,
        legacy_normalizer_decision_eff,
        needs_clarify_eff,
        &output_contract,
        first_layer_repair_codes,
    );
    info!(
        "{} intent_normalizer_first_layer_gate task_id={} owner_layer={} reason_code={} outcome={} source_decision={} final_decision={} needs_clarify={} output_contract_ref={} repair_codes={} repair_classes={}",
        crate::highlight_tag("routing"),
        task.task_id,
        first_layer_gate_record.owner_layer,
        first_layer_gate_record.reason_code,
        first_layer_gate_record.outcome,
        first_layer_gate_record
            .source_decision
            .map(|decision| decision.as_str())
            .unwrap_or("none"),
        first_layer_gate_record.final_decision.as_str(),
        first_layer_gate_record.needs_clarify,
        first_layer_gate_record.output_contract_ref,
        first_layer_gate_record.repair_codes.join(","),
        first_layer_gate_record.repair_classes.join(","),
    );
    IntentNormalizerOutput {
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
        legacy_first_layer_decision: legacy_normalizer_decision_eff,
        execution_finalize_style: execution_finalize_style_eff,
        turn_analysis,
        fallback_source: None,
        first_layer_gate_record,
    }
}
