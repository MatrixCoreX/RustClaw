use super::answer_candidate_binding::{
    active_task_invalid_turn_binding_context, append_contract_repair_context,
};
use super::contract_repair_judge::{apply_contract_repair_judge_output, run_contract_repair_judge};
use super::{ContractRepairReport, IntentNormalizerOut};
use crate::intent::surface_signals::PromptSurfaceSignals;
use crate::{AppState, ClaimedTask};

pub(super) async fn apply_answer_candidate_and_contract_judge_repair(
    state: &AppState,
    task: &ClaimedTask,
    req: &str,
    req_surface: &PromptSurfaceSignals,
    _route_view: &crate::task_context_builder::RouteContextView,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    llm_out: &str,
    llm_out_for_parse: &str,
    mut contract_repair_report: ContractRepairReport,
    mut out: IntentNormalizerOut,
) -> (IntentNormalizerOut, ContractRepairReport) {
    let mut contract_repair_context = String::from("none");
    if contract_repair_judge_runtime_enabled() {
        if let Some(invalid_binding_context) = active_task_invalid_turn_binding_context(
            llm_out,
            session_snapshot,
            req_surface,
            out.should_refresh_long_term_memory,
        ) {
            append_contract_repair_context(&mut contract_repair_context, invalid_binding_context);
            contract_repair_report.add("semantic_suspect", "active_task_invalid_turn_binding");
        }
    }
    if contract_repair_judge_runtime_enabled()
        && contract_repair_report.needs_llm_contract_integrity_repair()
    {
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
                contract_repair_report.merge(&repair_applied);
            }
        }
    }
    (out, contract_repair_report)
}

fn contract_repair_judge_runtime_enabled() -> bool {
    cfg!(test)
}
