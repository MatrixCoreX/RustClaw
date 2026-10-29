use serde_json::{json, Value};

use crate::task_lifecycle::{CheckpointBudgetCounters, ResumeEntrypoint, TaskCheckpoint};

pub(crate) async fn persist_prepared_direct_start(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
    extra: Option<Value>,
) -> Result<(), String> {
    // Ask tasks retain their planner continuation through the agent-loop snapshot.
    if task.kind != "run_skill" {
        return Ok(());
    }
    let state = state.clone();
    let task = task.clone();
    tokio::task::spawn_blocking(move || persist(&state, &task, extra.as_ref()))
        .await
        .map_err(|_| "async_start_checkpoint_unavailable".to_string())?
        .map_err(|_| "async_start_checkpoint_unavailable".to_string())
}

fn persist(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
    extra: Option<&Value>,
) -> anyhow::Result<()> {
    let job =
        super::parse_pending_async_job_ref_from_extra(extra, "async_start_checkpoint_invalid")
            .map_err(anyhow::Error::msg)?
            .ok_or_else(|| anyhow::anyhow!("async_start_checkpoint_missing"))?;
    let now = crate::now_ts_u64() as i64;
    let checkpoint_id = format!("run-skill:{}:async-job:{}", task.task_id, job.job_id);
    let checkpoint = TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: checkpoint_id.clone(),
        boundary_context: json!({"schema_version":1,"source":"prepared_async_start",
            "task_id":task.task_id,"async_start_phase":"prepared",
            "instruction_authority":"none"}),
        last_successful_round: None,
        last_successful_step: None,
        pending_action: None,
        observations: vec![
            json!({"schema_version":1,"observation_kind":"async_start_intent",
            "async_job_id":job.job_id,"status":"prepared","execution_observed":false}),
        ],
        capability_results: Vec::new(),
        evidence_refs: Vec::new(),
        artifact_refs: Vec::new(),
        completed_side_effect_refs: Vec::new(),
        budget: CheckpointBudgetCounters {
            round: 0,
            step: 0,
            llm_calls: 0,
            tool_calls: 1,
            elapsed_ms: 0,
            llm_elapsed_ms: 0,
            tool_elapsed_ms: 0,
        },
        attempt_ledger: None,
        pending_async_job: Some(job.clone()),
        repair_signal: None,
        resume_entrypoint: ResumeEntrypoint::PollAsyncJob,
    };
    let record = crate::repo::get_task_delivery_record(state, &task.task_id)?
        .ok_or_else(|| anyhow::anyhow!("async_start_task_unavailable"))?;
    let mut progress = record.result_json.unwrap_or_else(|| json!({}));
    let object = progress
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("async_start_task_result_invalid"))?;
    object.insert("task_checkpoint".into(), checkpoint.to_machine_json());
    object.insert("task_lifecycle".into(), json!({
        "schema_version":1,"state":"waiting","source":"prepared_async_start",
        "resume_reason":"pending_async_job","async_start_phase":"prepared",
        "checkpoint_id":checkpoint_id,"poll_ref":job.job_id,"cancel_ref":job.cancel_ref,
        "poll_after_seconds":job.poll_after_seconds,"async_job_expires_at":job.expires_at,
        "next_check_after":now.saturating_add(job.poll_after_seconds.min(i64::MAX as u64) as i64),
        "can_poll":true,"can_cancel":true,
        "async_timeout_policy":super::pending_async_job_timeout_policy(None, &job, now),
    }));
    // This CAS retains the current owner's lease; recovery still waits for fencing.
    crate::repo::update_task_progress_result(
        state,
        &task.task_id,
        task.claim_attempt,
        &progress.to_string(),
    )?;
    crate::task_result_retention::record_durable_job(state, &task.task_id, &job.job_id)?;
    Ok(())
}

#[cfg(test)]
#[path = "prepared_start_tests.rs"]
mod tests;
