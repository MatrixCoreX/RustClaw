use serde_json::{json, Map, Value};
use tracing::warn;

use super::{AppState, ClaimedTask, LoopState, SkillActionOutcome};

const MUTATION_OUTCOME_MAX_STRING_BYTES: usize = 2_048;

pub(super) enum MutationExecutionGuard {
    NotRequired,
    Acquired(crate::repo::TaskMutationLease),
    Completed(crate::repo::TaskMutationRecord),
    Uncertain(crate::repo::TaskMutationRecord),
}

pub(super) fn prepare_mutation_execution(
    state: &AppState,
    task: &ClaimedTask,
    normalized_skill: &str,
    args: &Value,
    action_fingerprint: &str,
    effect: crate::execution_recipe::ActionEffect,
) -> Result<MutationExecutionGuard, String> {
    if !effect.mutates || registry_action_is_idempotent(state, normalized_skill, args) {
        return Ok(MutationExecutionGuard::NotRequired);
    }
    let action_ref = mutation_action_ref(normalized_skill, args);
    match crate::repo::begin_task_mutation(
        &state.core.db,
        &task.task_id,
        action_fingerprint,
        &action_ref,
    )
    .map_err(|error| {
        json!({
            "error_kind": "mutation_ledger_unavailable",
            "reason_code": "mutation_ledger_unavailable",
            "message_key": "clawd.task.mutation_ledger_unavailable",
            "owner_layer": "task_mutation_ledger",
            "action_ref": action_ref,
            "detail_code": crate::truncate_for_agent_trace(&error.to_string()),
        })
        .to_string()
    })? {
        crate::repo::BeginTaskMutationOutcome::Acquired(lease) => {
            Ok(MutationExecutionGuard::Acquired(lease))
        }
        crate::repo::BeginTaskMutationOutcome::Completed(record) => {
            Ok(MutationExecutionGuard::Completed(record))
        }
        crate::repo::BeginTaskMutationOutcome::Uncertain(record) => {
            Ok(MutationExecutionGuard::Uncertain(record))
        }
    }
}

pub(super) fn record_completed_without_replay(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    record: &crate::repo::TaskMutationRecord,
    action_fingerprint: &str,
    normalized_skill: &str,
    global_step: usize,
    step_in_round: usize,
) -> Result<SkillActionOutcome, String> {
    let output = json!({
        "schema_version": 1,
        "source": "task_mutation_ledger",
        "status": "completed",
        "execution": "suppressed",
        "reason_code": "mutation_already_completed",
        "action_ref": record.action_ref,
        "fingerprint_hash": record.fingerprint_hash,
    })
    .to_string();
    loop_state
        .successful_action_fingerprints
        .entry(action_fingerprint.to_string())
        .or_insert(1);
    loop_state.has_tool_or_skill_output = true;
    loop_state
        .executed_step_results
        .push(crate::executor::StepExecutionResult {
            step_id: format!("step_{global_step}"),
            skill: normalized_skill.to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(output.clone()),
            error: None,
            started_at: crate::now_ts_u64(),
            finished_at: crate::now_ts_u64(),
        });
    crate::append_subtask_result(
        &mut loop_state.subtask_results,
        global_step,
        &format!("skill({normalized_skill})"),
        true,
        &output,
    );
    loop_state.output_vars.insert(
        "agent_loop.mutation_replay_suppressed".to_string(),
        "true".to_string(),
    );
    loop_state.history_compact.push(format!(
        "step={global_step} source=task_mutation_ledger status=completed execution=suppressed action_ref={}",
        record.action_ref
    ));
    let structured_extra = record
        .outcome
        .as_ref()
        .and_then(|outcome| outcome.get("structured_extra"));
    let stop_signal = super::async_start_checkpoint::publish_pending_async_job_start_checkpoint(
        state,
        task,
        loop_state,
        normalized_skill,
        global_step,
        step_in_round,
        structured_extra,
    )?;
    Ok(SkillActionOutcome {
        ended_with_user_visible_output: false,
        continue_in_round: stop_signal.is_none(),
        stop_signal,
    })
}

pub(super) fn publish_uncertain_mutation_checkpoint(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    record: &crate::repo::TaskMutationRecord,
) -> SkillActionOutcome {
    super::support::publish_agent_loop_mutation_reconciliation_checkpoint(
        state,
        task,
        loop_state,
        &record.action_ref,
        &record.fingerprint_hash,
        &record.status,
    );
    SkillActionOutcome {
        ended_with_user_visible_output: false,
        stop_signal: Some("mutation_reconciliation_required".to_string()),
        continue_in_round: false,
    }
}

pub(super) fn complete_mutation_execution(
    state: &AppState,
    lease: &crate::repo::TaskMutationLease,
    outcome: &str,
    structured_extra: Option<&Value>,
) -> bool {
    let projection = safe_mutation_outcome_projection(structured_extra);
    match crate::repo::complete_task_mutation(&state.core.db, lease, outcome, projection.as_ref()) {
        Ok(()) => true,
        Err(error) => {
            warn!(
                "task mutation completion persistence failed task_id={} action_ref={} error={}",
                lease.record.task_id,
                lease.record.action_ref,
                crate::truncate_for_log(&error.to_string())
            );
            false
        }
    }
}

fn safe_mutation_outcome_projection(structured_extra: Option<&Value>) -> Option<Value> {
    let extra = structured_extra?;
    let mut projected_extra = Map::new();
    if let Ok(Some(job)) = crate::async_job_contract::parse_pending_async_job_ref_from_extra(
        Some(extra),
        "mutation_ledger_async_job",
    ) {
        if let Ok(value) = serde_json::to_value(job) {
            projected_extra.insert("pending_async_job".to_string(), value);
        }
    }
    if let Ok(Some(adapter)) =
        crate::async_job_contract::parse_pending_async_job_poll_adapter_from_extra(
            Some(extra),
            "mutation_ledger_async_job",
        )
    {
        if let Some(adapter) = safe_poll_adapter_projection(&adapter) {
            projected_extra.insert("poll_adapter".to_string(), adapter);
        }
    }
    for key in [
        "schema_version",
        "source",
        "action",
        "status",
        "status_code",
        "message_key",
        "checkpoint_id",
        "job_id",
        "result_ref",
        "patch_id",
    ] {
        if let Some(value) = extra.get(key).and_then(safe_machine_scalar) {
            projected_extra.entry(key.to_string()).or_insert(value);
        }
    }
    (!projected_extra.is_empty()).then(|| {
        json!({
            "schema_version": 1,
            "structured_extra": Value::Object(projected_extra),
        })
    })
}

fn safe_poll_adapter_projection(adapter: &Value) -> Option<Value> {
    let adapter = adapter.as_object()?;
    let mut projected = Map::new();
    for key in ["kind", "adapter_kind", "skill_name"] {
        if let Some(value) = adapter.get(key).and_then(safe_machine_scalar) {
            projected.insert(key.to_string(), value);
        }
    }
    if let Some(args) = adapter.get("args").and_then(Value::as_object) {
        let mut projected_args = Map::new();
        for key in [
            "action",
            "task_id",
            "job_id",
            "request_id",
            "vendor",
            "provider",
            "model",
            "output_path",
            "result_ref",
            "cancel_ref",
            "cancel_token",
            "status",
            "message_key",
            "poll_after_seconds",
            "poll_after_ms",
            "expires_at",
            "retryable",
            "dry_run",
        ] {
            if let Some(value) = args.get(key).and_then(safe_machine_scalar) {
                projected_args.insert(key.to_string(), value);
            }
        }
        if !projected_args.is_empty() {
            projected.insert("args".to_string(), Value::Object(projected_args));
        }
    }
    (!projected.is_empty()).then_some(Value::Object(projected))
}

fn safe_machine_scalar(value: &Value) -> Option<Value> {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => Some(value.clone()),
        Value::String(value) if value.len() <= MUTATION_OUTCOME_MAX_STRING_BYTES => {
            Some(Value::String(value.clone()))
        }
        _ => None,
    }
}

pub(super) fn mark_mutation_execution_uncertain(
    state: &AppState,
    lease: &crate::repo::TaskMutationLease,
) {
    if let Err(error) = crate::repo::mark_task_mutation_uncertain(&state.core.db, lease) {
        warn!(
            "task mutation uncertainty persistence failed task_id={} action_ref={} error={}",
            lease.record.task_id,
            lease.record.action_ref,
            crate::truncate_for_log(&error.to_string())
        );
    }
}

fn registry_action_is_idempotent(state: &AppState, normalized_skill: &str, args: &Value) -> bool {
    let action = normalized_action_token(args);
    state
        .get_skills_registry()
        .is_some_and(|registry| registry.resolved_idempotent(normalized_skill, action.as_deref()))
}

fn mutation_action_ref(normalized_skill: &str, args: &Value) -> String {
    format!(
        "skill:{}:action:{}",
        normalized_skill,
        normalized_action_token(args).unwrap_or_else(|| "_default".to_string())
    )
}

fn normalized_action_token(args: &Value) -> Option<String> {
    args.get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .to_ascii_lowercase()
                .chars()
                .map(|ch| {
                    if matches!(ch, '-' | ' ' | '.') {
                        '_'
                    } else {
                        ch
                    }
                })
                .collect()
        })
}

#[cfg(test)]
#[path = "mutation_ledger_tests.rs"]
mod tests;
