use super::active_observation::active_ordered_scalar_path_missing_state_patch_context;
use super::answer_candidate_binding::{
    active_task_invalid_turn_binding_context, active_text_answer_candidate_conflict_context,
    analyze_answer_candidate_binding, answer_candidate_binding_repair_context,
    append_contract_repair_context, clear_internal_context_answer_candidate,
    clear_memory_only_answer_candidate_if_recent_context_conflicts,
    clear_memory_update_answer_candidate_if_memory_only,
    rebind_memory_only_answer_candidate_to_recent_user_memory,
};
use super::contract_repair_judge::{apply_contract_repair_judge_output, run_contract_repair_judge};
use super::semantic_suspect::semantic_suspect_detail_for_normalizer_output_with_command_runtime;
use super::{
    workspace_identity_semantic_repair_context, ContractRepairReport, IntentNormalizerOut,
};
use crate::intent::surface_signals::PromptSurfaceSignals;
use crate::{AppState, ClaimedTask};

pub(super) async fn apply_answer_candidate_and_contract_judge_repair(
    state: &AppState,
    task: &ClaimedTask,
    req: &str,
    req_surface: &PromptSurfaceSignals,
    route_view: &crate::task_context_builder::RouteContextView,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    llm_out: &str,
    llm_out_for_parse: &str,
    mut contract_repair_report: ContractRepairReport,
    mut out: IntentNormalizerOut,
) -> (IntentNormalizerOut, ContractRepairReport) {
    let cleared_internal_context_candidate =
        clear_internal_context_answer_candidate(&mut out).is_some();
    let answer_candidate_binding =
        analyze_answer_candidate_binding(req, &out.answer_candidate, route_view);
    let mut contract_repair_context = String::from("none");
    let mut active_text_answer_candidate_conflict = false;
    if cleared_internal_context_candidate {
        contract_repair_report.add(
            "structural_cleanup",
            "internal_context_answer_candidate_cleared",
        );
    } else if clear_memory_update_answer_candidate_if_memory_only(
        &mut out,
        answer_candidate_binding.as_ref(),
    )
    .is_some()
    {
        contract_repair_report.add(
            "structural_cleanup",
            "memory_update_unbound_answer_candidate_cleared",
        );
    } else if rebind_memory_only_answer_candidate_to_recent_user_memory(
        state,
        task,
        &mut out,
        answer_candidate_binding.as_ref(),
    )
    .is_some()
    {
        contract_repair_report.add(
            "structural_cleanup",
            "memory_only_answer_candidate_rebound_to_recent_user_memory",
        );
    } else if clear_memory_only_answer_candidate_if_recent_context_conflicts(
        &mut out,
        answer_candidate_binding.as_ref(),
        route_view,
    )
    .is_some()
    {
        contract_repair_report.add(
            "structural_cleanup",
            "memory_only_answer_candidate_recent_scalar_conflict_cleared",
        );
    } else if let Some(binding) = answer_candidate_binding
        .as_ref()
        .filter(|binding| binding.is_memory_only_binding() && binding.is_distinctive())
    {
        contract_repair_context =
            answer_candidate_binding_repair_context(binding, out.should_refresh_long_term_memory);
        contract_repair_report.add("semantic_suspect", "answer_candidate_memory_only_binding");
    }
    if let Some(active_conflict_context) = active_text_answer_candidate_conflict_context(
        answer_candidate_binding.as_ref(),
        session_snapshot,
        req_surface,
        out.should_refresh_long_term_memory,
    ) {
        active_text_answer_candidate_conflict = true;
        append_contract_repair_context(&mut contract_repair_context, active_conflict_context);
        contract_repair_report.add("semantic_suspect", "active_task_answer_candidate_conflict");
    }
    if let Some(invalid_binding_context) = active_task_invalid_turn_binding_context(
        llm_out,
        session_snapshot,
        req_surface,
        out.should_refresh_long_term_memory,
    ) {
        append_contract_repair_context(&mut contract_repair_context, invalid_binding_context);
        contract_repair_report.add("semantic_suspect", "active_task_invalid_turn_binding");
    }
    if let Some(ordered_ref_context) =
        active_ordered_scalar_path_missing_state_patch_context(&out, session_snapshot)
    {
        append_contract_repair_context(&mut contract_repair_context, ordered_ref_context);
        contract_repair_report.add(
            "semantic_suspect",
            "active_ordered_scalar_path_missing_ordered_entry_ref",
        );
    }
    if let Some(detail) = semantic_suspect_detail_for_normalizer_output_with_command_runtime(
        &out,
        Some(req_surface),
        req,
        &state.skill_rt.workspace_root,
        Some(&state.policy.command_intent),
    ) {
        if detail == "workspace_identity_chat_route_needs_semantic_review" {
            if let Some(context) =
                workspace_identity_semantic_repair_context(req, &state.skill_rt.workspace_root)
            {
                append_contract_repair_context(&mut contract_repair_context, context);
            }
        } else if detail == "raw_command_output_locator_needs_semantic_review" {
            append_contract_repair_context(
                &mut contract_repair_context,
                "raw_command_output_locator_review: explicit_command_segment=false; command_payload=false"
                    .to_string(),
            );
        } else if detail == "command_output_summary_needs_failure_contract_review" {
            append_contract_repair_context(
                &mut contract_repair_context,
                "command_summary_review: decision=planner_execute; contract=command_output_summary; review=failure_contract"
                    .to_string(),
            );
        } else if detail == "locatorless_generic_evidence_contract_needs_semantic_shape_review" {
            append_contract_repair_context(
                &mut contract_repair_context,
                "locatorless_generic_evidence_review: decision=planner_execute; contract=semantic_none; locator_kind=none; review=semantic_shape"
                    .to_string(),
            );
        }
        contract_repair_report.add("semantic_suspect", detail);
    }
    if contract_repair_report.needs_llm_contract_integrity_repair() {
        if let Some(repair) = run_contract_repair_judge(
            state,
            task,
            req,
            llm_out,
            llm_out_for_parse,
            &contract_repair_report,
            &contract_repair_context,
        )
        .await
        {
            if apply_contract_repair_judge_output(&mut out, repair) {
                let mut repair_applied = ContractRepairReport::default();
                repair_applied.add("llm_semantic", "contract_repair_judge_applied");
                if active_text_answer_candidate_conflict {
                    repair_applied.add("llm_semantic", "active_task_answer_candidate_repaired");
                }
                contract_repair_report.merge(&repair_applied);
            }
        }
    }
    (out, contract_repair_report)
}
