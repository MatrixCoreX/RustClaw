use serde_json::{json, Value};

use crate::task_lifecycle::{ResumeEntrypoint, TaskCheckpoint, TaskLifecycleState};

const COMPLETION_SOURCE: &str = "async_job_completion_checkpoint";

pub(crate) fn completed_async_job_continuation_result(
    task_kind: &str,
    checkpoint: &TaskCheckpoint,
    final_result_json: &Value,
    now_ts: i64,
) -> Option<Value> {
    if task_kind != "ask"
        || !checkpoint.checkpoint_id.starts_with("agent-loop:")
        || !matches!(checkpoint.resume_entrypoint, ResumeEntrypoint::PollAsyncJob)
        || !final_result_json.is_object()
    {
        return None;
    }
    if checkpoint
        .boundary_context
        .pointer("/async_completion_policy/mode")
        .and_then(Value::as_str)
        == Some("direct_terminal")
    {
        return None;
    }

    let job_id = checkpoint
        .pending_async_job
        .as_ref()
        .map(|job| job.job_id.trim())
        .filter(|job_id| !job_id.is_empty())?;
    let checkpoint_id = format!("{}:completion", checkpoint.checkpoint_id);
    let serialized_result = final_result_json.to_string();
    let terminal_observation = json!({
        "schema_version": 1,
        "source": COMPLETION_SOURCE,
        "job_id": job_id,
        "status": "succeeded",
        "final_result_json": final_result_json,
        "observed_at": now_ts,
    });

    let mut successor = checkpoint.clone();
    successor.checkpoint_id = checkpoint_id.clone();
    successor.pending_async_job = None;
    successor.resume_entrypoint = ResumeEntrypoint::NextPlannerRound;
    successor.observations.push(terminal_observation.clone());
    successor.boundary_context["source"] = json!(COMPLETION_SOURCE);
    successor.boundary_context["previous_checkpoint_id"] = json!(checkpoint.checkpoint_id);
    successor.boundary_context["completed_async_job_id"] = json!(job_id);
    successor.boundary_context["async_job_terminal_observation"] = terminal_observation;
    if let Some(resume_state) = successor
        .boundary_context
        .get_mut("agent_loop_resume_state")
        .and_then(Value::as_object_mut)
    {
        resume_state.insert("stage".to_string(), json!("planning"));
        resume_state.insert("last_output".to_string(), json!(serialized_result));
        resume_state.insert(
            "async_completion_continuation".to_string(),
            json!({
                "schema_version": 1,
                "status": "ready_for_planner",
                "job_id": job_id,
                "continue_original_request": true,
                "repeat_completed_side_effect": false,
            }),
        );
        if let Some(history) = resume_state
            .get_mut("history_compact")
            .and_then(Value::as_array_mut)
        {
            history.push(json!(format!(
                "async_job_completed job_id={job_id} continue_original_request=true repeat_completed_side_effect=false"
            )));
        }
        if let Some(observations) = resume_state
            .get_mut("task_observations")
            .and_then(Value::as_array_mut)
        {
            observations.push(json!({
                "schema_version": 1,
                "source": COMPLETION_SOURCE,
                "job_id": job_id,
                "status": "succeeded",
                "final_result_json": final_result_json,
            }));
        }
        if let Some(last_step) = resume_state
            .get_mut("executed_step_results")
            .and_then(Value::as_array_mut)
            .and_then(|steps| steps.last_mut())
            .and_then(Value::as_object_mut)
        {
            last_step.insert("status".to_string(), json!("ok"));
            last_step.insert("output".to_string(), json!(serialized_result));
            last_step.insert("error".to_string(), Value::Null);
            last_step.insert("finished_at".to_string(), json!(now_ts.max(0) as u64));
        }
    }

    Some(json!({
        "schema_version": 1,
        "task_lifecycle": {
            "schema_version": 1,
            "state": TaskLifecycleState::Background,
            "source": COMPLETION_SOURCE,
            "resume_reason": "async_job_completed_continue_planning",
            "next_check_after": now_ts,
            "checkpoint_id": checkpoint_id,
            "previous_checkpoint_id": checkpoint.checkpoint_id,
            "completed_async_job_id": job_id,
            "can_poll": true,
            "can_cancel": true,
            "last_heartbeat_ts": now_ts,
        },
        "task_checkpoint": successor.to_machine_json(),
    }))
}

#[cfg(test)]
#[path = "async_completion_checkpoint_tests.rs"]
mod tests;
