use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::{AppState, ClaimedTask};

use super::quantity::{
    direct_compare_paths_required_metadata_from_observed_output,
    direct_quantity_compare_paths_required_metadata_from_compare_paths,
};
use super::{
    direct_scalar_observed_answer, final_answer_text_from_delivery,
    log_deterministic_delivery_record,
};

pub(super) fn replace_final_delivery_with_quantity_compare_paths_required_metadata(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &mut Vec<String>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some((answer, summary)) =
        direct_compare_paths_required_metadata_from_observed_output(loop_state, agent_run_context)
            .or_else(|| {
                direct_quantity_compare_paths_required_metadata_from_compare_paths(
                    state,
                    user_text,
                    loop_state,
                    agent_run_context,
                )
            })
    else {
        return false;
    };
    if final_answer_text_from_delivery(delivery_messages).trim() == answer.trim() {
        loop_state.last_user_visible_respond = Some(answer);
        *finalizer_summary = Some(summary);
        return true;
    }
    delivery_messages.clear();
    append_delivery_message(task.task_id.as_str(), delivery_messages, answer.clone());
    loop_state.delivery_messages = delivery_messages.clone();
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(summary);
    log_deterministic_delivery_record(
        &task.task_id,
        "quantity_compare_paths_required_metadata",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

pub(super) fn replace_final_delivery_with_recent_scalar_compare_paths_required_metadata(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &mut Vec<String>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    if !agent_run_context
        .and_then(|ctx| ctx.output_contract())
        .is_some_and(|route| {
            route.semantic_kind == crate::OutputSemanticKind::RecentScalarEqualityCheck
                && !route.delivery_required
        })
    {
        return false;
    }
    let Some((answer, summary)) =
        direct_scalar_observed_answer(Some(state), loop_state, agent_run_context)
    else {
        return false;
    };
    if !answer_has_compare_paths_existence_fields(&answer) {
        return false;
    }
    if final_answer_text_from_delivery(delivery_messages).trim() == answer.trim() {
        loop_state.last_user_visible_respond = Some(answer);
        *finalizer_summary = Some(summary);
        return true;
    }
    delivery_messages.clear();
    append_delivery_message(task.task_id.as_str(), delivery_messages, answer.clone());
    loop_state.delivery_messages = delivery_messages.clone();
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(summary);
    log_deterministic_delivery_record(
        &task.task_id,
        "recent_scalar_compare_paths_required_metadata",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

fn answer_has_compare_paths_existence_fields(answer: &str) -> bool {
    let mut has_same_path = false;
    let mut has_left_exists = false;
    let mut has_right_exists = false;
    for line in answer.lines().map(str::trim) {
        if line.starts_with("same_path=") {
            has_same_path = true;
        } else if line.starts_with("left_exists=") {
            has_left_exists = true;
        } else if line.starts_with("right_exists=") {
            has_right_exists = true;
        }
    }
    has_same_path && has_left_exists && has_right_exists
}
