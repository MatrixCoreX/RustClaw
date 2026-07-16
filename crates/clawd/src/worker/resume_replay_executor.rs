use anyhow::Result;
use serde_json::Value;
use tracing::info;

use crate::{repo, AppState};

pub(super) async fn execute_seeded_agent_loop_dispatch_result(
    state: &AppState,
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
) -> Result<Option<Value>> {
    if !claimed_seeded_agent_loop_dispatch_ready(claimed) {
        return Ok(None);
    }

    let mut payload: Value = serde_json::from_str(&claimed.task.payload_json)?;
    let prepared_input = super::prepare_ask_input(state, &claimed.task, &mut payload).await;
    let prompt = prepared_input.prompt;
    let source = prepared_input.source;
    let prepared_flow =
        super::ask_runtime::prepare_ask_flow(state, &claimed.task, &payload, &prompt, &source)
            .await?;
    let agent_run_context =
        Some(super::ask_runtime::build_agent_run_context_from_prepared_flow(&prepared_flow));

    info!(
        "resume replay seeded agent loop starting: task_id={} checkpoint_id={} resume_trigger={} completed_side_effect_count={}",
        claimed.task_id,
        claimed.checkpoint_id,
        claimed.resume_trigger,
        claimed.task_checkpoint.completed_side_effect_refs.len()
    );
    let result = crate::agent_engine::run_agent_with_tools_seeded(
        state,
        &claimed.task,
        &prepared_flow.prompt_with_memory_for_execution,
        &prepared_flow.planner_user_request,
        agent_run_context,
        &claimed.task_checkpoint,
    )
    .await;

    Ok(super::runtime_support::seeded_agent_loop_terminal_dispatch_result_payload(claimed, result))
}

fn claimed_seeded_agent_loop_dispatch_ready(
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
) -> bool {
    claimed.task.kind == "ask"
        && claimed.task_checkpoint.checkpoint_id == claimed.checkpoint_id
        && claimed.executor_action == "run_seeded_agent_loop"
        && claimed.executor_status == "seeded_loop_requires_provider_window"
        && claimed.dispatch_state == "ready_to_run_seeded_agent_loop"
        && claimed.dispatch_execution_state == "claimed_to_run_seeded_agent_loop"
        && claimed.resume_directive == "run_next_planner_round"
        && matches!(
            claimed.task_checkpoint.resume_entrypoint,
            crate::task_lifecycle::ResumeEntrypoint::NextPlannerRound
        )
        && claimed.execution_plan.get("text").is_none()
        && claimed.execution_plan.get("error_text").is_none()
        && claimed.dispatch_payload.get("text").is_none()
        && claimed.dispatch_payload.get("error_text").is_none()
        && claimed.dispatch_claim.get("text").is_none()
        && claimed.dispatch_claim.get("error_text").is_none()
}
