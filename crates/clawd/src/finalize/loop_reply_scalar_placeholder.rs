use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::{AppState, ClaimedTask};

use super::{
    current_user_visible_delivery_text, log_deterministic_delivery_record,
    route_allows_direct_scalar_observed_answer,
};

pub(super) fn replace_scalar_placeholder_delivery_with_direct_scalar_answer(
    _state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if !route_allows_direct_scalar_observed_answer(route) {
        return false;
    }
    let Some(current) = current_user_visible_delivery_text(loop_state).map(str::trim) else {
        return false;
    };
    if !matches!(
        current,
        "field_value"
            | "value"
            | "value_text"
            | "path"
            | "resolved_path"
            | "command_output"
            | "count"
            | "total"
    ) {
        return false;
    }
    let Some(answer) = latest_terminal_scalar_respond_for_placeholder(route, loop_state) else {
        return false;
    };
    if answer.trim().is_empty() || answer.trim() == current {
        return false;
    }
    loop_state.delivery_messages.clear();
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        answer.clone(),
    );
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        parsed: true,
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: loop_state.executed_step_results.len(),
        ..Default::default()
    });
    log_deterministic_delivery_record(
        &task.task_id,
        "scalar_placeholder_terminal_direct_answer",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

fn latest_terminal_scalar_respond_for_placeholder(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<String> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && step.skill == "respond")
        .filter_map(|step| step.output.as_deref())
        .map(str::trim)
        .find(|candidate| terminal_scalar_placeholder_replacement_matches_route(route, candidate))
        .map(ToOwned::to_owned)
}

fn terminal_scalar_placeholder_replacement_matches_route(
    route: &crate::RouteResult,
    candidate: &str,
) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty()
        || candidate
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
            != 1
        || candidate.starts_with('{')
        || candidate.starts_with('[')
        || candidate.contains('=')
        || crate::finalize::parse_delivery_token(candidate).is_some()
        || crate::finalize::looks_like_planner_artifact(candidate)
        || crate::finalize::looks_like_internal_trace_artifact(candidate)
        || crate::finalize::is_execution_summary_message(candidate)
    {
        return false;
    }
    if crate::finalize::route_matches_single_path_output_contract(route) {
        return candidate.starts_with('/')
            || candidate.starts_with("./")
            || candidate.starts_with("../")
            || candidate.contains('/');
    }
    true
}
