use anyhow::Result;
use rusqlite::OptionalExtension;
use serde_json::json;
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
    if let Some(resume_input) =
        load_resume_steering_input(state, &claimed.task_id, &claimed.checkpoint_id)?
    {
        apply_resume_steering_prompt(&mut payload, &resume_input);
    }
    let prepared_input = super::prepare_ask_input(state, &claimed.task, &mut payload).await;
    let prompt = prepared_input.prompt;
    let source = prepared_input.source;
    let prepared_flow =
        super::ask_runtime::prepare_ask_flow(state, &claimed.task, &payload, &prompt, &source)
            .await?;
    let agent_run_context =
        Some(super::ask_runtime::build_agent_run_context_from_prepared_flow(&prepared_flow));
    let stored_checkpoint_action = load_required_checkpoint_action(state, claimed)?;

    info!(
        "resume replay seeded agent loop starting: task_id={} checkpoint_id={} resume_trigger={} completed_side_effect_count={} stored_action={}",
        claimed.task_id,
        claimed.checkpoint_id,
        claimed.resume_trigger,
        claimed.task_checkpoint.completed_side_effect_refs.len(),
        stored_checkpoint_action.is_some()
    );
    let result = if let Some(stored_action) = stored_checkpoint_action {
        let output_contract = stored_action
            .output_contract
            .map(serde_json::from_value::<crate::IntentOutputContract>)
            .transpose()
            .map_err(|_| anyhow::anyhow!("checkpoint_action_output_contract_invalid"))?;
        let request_envelope = json!({
            "protocol": "rustclaw.checkpoint_action.v1",
            "task_id": claimed.task_id,
            "checkpoint_id": claimed.checkpoint_id,
            "action_ref": stored_action.action_ref,
            "tool_or_skill": stored_action.tool_or_skill,
            "args_keys": stored_action
                .args
                .as_object()
                .map(|args| args.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default(),
        })
        .to_string();
        let plan = crate::agent_engine::checkpoint_action_plan(
            &stored_action.tool_or_skill,
            &stored_action.action_ref,
            stored_action.args,
            output_contract,
        );
        crate::agent_engine::run_agent_with_tools_seeded_direct_plan(
            state,
            &claimed.task,
            &request_envelope,
            agent_run_context,
            &claimed.task_checkpoint,
            &prepared_flow.initial_task_observations,
            &plan,
        )
        .await
    } else {
        crate::agent_engine::run_agent_with_tools_seeded(
            state,
            &claimed.task,
            &prepared_flow.prompt_with_memory_for_execution,
            &prepared_flow.planner_user_request,
            agent_run_context,
            &claimed.task_checkpoint,
            &prepared_flow.initial_task_observations,
        )
        .await
    };

    Ok(super::runtime_support::seeded_agent_loop_terminal_dispatch_result_payload(claimed, result))
}

fn load_required_checkpoint_action(
    state: &AppState,
    claimed: &repo::ClaimedDispatchedPausedCheckpointResumeExecution,
) -> Result<Option<repo::TaskCheckpointAction>> {
    if !checkpoint_requires_stored_action(&claimed.task_checkpoint) {
        return Ok(None);
    }
    repo::load_task_checkpoint_action(&state.core.db, &claimed.task_id, &claimed.checkpoint_id)?
        .ok_or_else(|| anyhow::anyhow!("checkpoint_action_missing"))
        .map(Some)
}

fn checkpoint_requires_stored_action(checkpoint: &crate::task_lifecycle::TaskCheckpoint) -> bool {
    checkpoint
        .pending_action
        .as_ref()
        .and_then(|action| action.get("kind"))
        .and_then(Value::as_str)
        == Some("agent_hook_pre_tool_use")
}

fn load_resume_steering_input(
    state: &AppState,
    task_id: &str,
    checkpoint_id: &str,
) -> Result<Option<Value>> {
    let db = state.core.db.get()?;
    let raw_result = db
        .query_row(
            "SELECT result_json
             FROM tasks
             WHERE task_id = ?1
               AND status = 'running'
             LIMIT 1",
            rusqlite::params![task_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    let Some(result) = raw_result
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
    else {
        return Ok(None);
    };
    let input = result.pointer("/task_lifecycle/resume_input");
    if input
        .and_then(|value| value.get("checkpoint_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        != Some(checkpoint_id)
    {
        return Ok(None);
    }
    Ok(input
        .filter(|value| value.is_object())
        .filter(|value| {
            value
                .get("user_message")
                .and_then(Value::as_str)
                .is_some_and(|text| !text.trim().is_empty())
                || value.get("new_constraints").is_some()
        })
        .cloned())
}

fn apply_resume_steering_prompt(payload: &mut Value, resume_input: &Value) {
    let original_request = super::ask_input::opaque_user_prompt(payload);
    let mut envelope = json!({
        "protocol": "rustclaw.resume_input.v1",
        "original_request": original_request,
    });
    if let Some(object) = envelope.as_object_mut() {
        if let Some(user_message) = resume_input
            .get("user_message")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            object.insert("user_message".to_string(), json!(user_message));
        }
        if let Some(constraints) = resume_input.get("new_constraints") {
            object.insert("new_constraints".to_string(), constraints.clone());
        }
    }
    if let Some(payload) = payload.as_object_mut() {
        payload.insert("text".to_string(), Value::String(envelope.to_string()));
    }
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

#[cfg(test)]
#[path = "resume_replay_executor_tests.rs"]
mod tests;
