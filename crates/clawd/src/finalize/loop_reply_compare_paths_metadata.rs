use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::{AppState, ClaimedTask};

use super::quantity::{
    direct_compare_paths_required_metadata_from_observed_output,
    direct_quantity_compare_paths_required_metadata_from_compare_paths,
};
use super::{final_answer_text_from_delivery, log_deterministic_delivery_record};

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
