use serde_json::{json, Value};

use crate::answer_verifier::AnswerVerificationStatus;
use crate::{task_journal::TaskJournal, AppState, ClaimedTask};

#[path = "answer_verification_retry.rs"]
mod retry_policy;

pub(crate) fn checkpoint_unavailable_verification(
    state: &AppState,
    task: &ClaimedTask,
    journal: &mut TaskJournal,
) -> bool {
    let Some(record) = journal
        .answer_verification
        .clone()
        .filter(|record| record.status == AnswerVerificationStatus::Unavailable)
    else {
        return false;
    };
    if journal.answer_verification_checkpoint_prepared {
        return true;
    }
    let now = crate::now_ts_u64().min(i64::MAX as u64) as i64;
    let mut checkpoint_payload = journal
        .answer_verification_checkpoint
        .take()
        .map(std::sync::Arc::unwrap_or_clone)
        .or_else(|| {
            journal
                .task_checkpoint
                .as_ref()
                .map(|value| json!({"task_checkpoint": value}))
        })
        .unwrap_or_else(|| fallback_checkpoint(task, journal, now));
    let provider = state.task_provider_blocker(&task.task_id);
    let retry = retry_policy::plan(
        &record,
        journal
            .task_checkpoint
            .as_ref()
            .and_then(|value| value.pointer("/boundary_context/verification_retry")),
        provider.as_ref().map(|blocker| blocker.retry_after_seconds),
    );
    let retry_after = retry.retry_after_seconds.min(i64::MAX as u64) as i64;
    let needs_user = retry.needs_user;
    let checkpoint = &mut checkpoint_payload["task_checkpoint"];
    checkpoint["boundary_context"]["source"] = json!("answer_verifier");
    checkpoint["boundary_context"]["answer_verification"] = json!(record);
    checkpoint["boundary_context"]["verification_retry"] = json!(retry.state);
    checkpoint["boundary_context"]["agent_loop_resume_state"]["stage"] = json!("verification");
    checkpoint["boundary_context"]["task_llm_metrics"] =
        state.task_llm_metrics_checkpoint_json(&task.task_id);
    checkpoint["pending_action"] = Value::Null;
    checkpoint["resume_entrypoint"] = json!("next_planner_round");
    checkpoint["repair_signal"] = json!({
        "schema_version": 1,
        "source": "answer_verifier",
        "status_code": "answer_verification_unavailable",
        "issue_codes": [retry.reason_code],
        "next_recovery_kind": if needs_user { "needs_user" } else { "wait_background" },
    });
    let checkpoint_id = checkpoint["checkpoint_id"].clone();
    crate::agent_engine::bind_verification_checkpoint(state, task, journal, checkpoint);
    journal.record_task_checkpoint(checkpoint.clone());
    journal.record_task_lifecycle(json!({
        "schema_version": 1,
        "state": if needs_user { "needs_user" } else { "waiting" },
        "source": "answer_verifier",
        "resume_reason": "answer_verification_unavailable",
        "reason_code": retry.reason_code,
        "message_key": "agent.answer_verification.unavailable",
        "answer_verification": record,
        "verification_retry": retry.state,
        "checkpoint_id": checkpoint_id,
        "next_check_after": if needs_user { None } else { Some(now.saturating_add(retry_after)) },
        "can_poll": true,
        "can_cancel": true,
        "manual_resume_required": needs_user,
        "last_heartbeat_ts": now,
        "provider_blocker_active": provider.is_some(),
        "provider_status": provider.map(|blocker| blocker.to_machine_json()),
    }));
    crate::assistant_presentation::abort_for_verifier_retry(state, task);
    journal.record_final_stop_signal("answer_verification_unavailable");
    journal.answer_verification_checkpoint_prepared = true;
    true
}

fn fallback_checkpoint(task: &ClaimedTask, journal: &TaskJournal, now: i64) -> Value {
    // Non-loop callers have no replayable action; keep their observations, never
    // fabricate an execution fingerprint or permission grant.
    json!({"task_checkpoint": {
        "schema_version": 1,
        "checkpoint_id": format!("verification:{}:{now}", task.task_id),
        "boundary_context": {"schema_version": 1, "task_id": task.task_id,
            "agent_loop_resume_state": {"schema_version": 1, "output_contract": journal.output_contract}},
        "last_successful_round": null, "last_successful_step": null,
        "pending_action": null, "observations": journal.task_observations,
        "capability_results": journal.capability_results,
        "evidence_refs": [], "artifact_refs": [], "completed_side_effect_refs": [],
        "budget": {"round": 0, "step": journal.step_results.len(), "llm_calls": 0,
            "tool_calls": 0, "elapsed_ms": 0, "llm_elapsed_ms": 0, "tool_elapsed_ms": 0},
        "resume_entrypoint": "next_planner_round"
    }})
}

#[cfg(test)]
#[path = "answer_verification_recovery_tests.rs"]
mod tests;
