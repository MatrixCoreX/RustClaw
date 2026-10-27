use serde_json::{json, Value};

use crate::answer_verifier::AnswerVerificationStatus;
use crate::{task_journal::TaskJournal, AppState, ClaimedTask};

pub(super) fn apply_exact_observation_machine_field_delivery(
    route_result: &crate::IntentOutputContract,
    journal: &mut TaskJournal,
    answer_text: &mut String,
    answer_messages: &mut Vec<String>,
) -> bool {
    let Some(answer) = crate::finalize::exact_observation_machine_field_projection_from_journal(
        route_result,
        journal,
    ) else {
        return false;
    };
    if answer.trim() == answer_text.trim() {
        return false;
    }
    *answer_text = answer;
    answer_messages.clear();
    answer_messages.push(answer_text.clone());
    journal.record_final_answer(answer_text.as_str());
    journal.record_answer_verification(
        crate::answer_verifier::AnswerVerificationOutcome::from_verdict(
            crate::answer_verifier::AnswerVerifierOut {
                pass: true,
                confidence: 1.0,
                ..Default::default()
            },
        ),
        answer_text,
    );
    true
}

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
    let now = crate::now_ts_u64().min(i64::MAX as u64) as i64;
    let mut checkpoint_payload = journal
        .answer_verification_checkpoint
        .take()
        .map(std::sync::Arc::unwrap_or_clone)
        .unwrap_or_else(|| fallback_checkpoint(task, journal, now));
    let provider = state.task_provider_blocker(&task.task_id);
    let retry_after = provider
        .as_ref()
        .map(|blocker| blocker.retry_after_seconds)
        .unwrap_or(60)
        .max(1)
        .min(i64::MAX as u64) as i64;
    let needs_user = !record.retryable;
    let checkpoint = &mut checkpoint_payload["task_checkpoint"];
    checkpoint["boundary_context"]["source"] = json!("answer_verifier");
    checkpoint["boundary_context"]["answer_verification"] = json!(record);
    checkpoint["boundary_context"]["agent_loop_resume_state"]["stage"] = json!("verification");
    checkpoint["boundary_context"]["task_llm_metrics"] =
        state.task_llm_metrics_checkpoint_json(&task.task_id);
    checkpoint["pending_action"] = Value::Null;
    checkpoint["resume_entrypoint"] = json!("next_planner_round");
    checkpoint["repair_signal"] = json!({
        "schema_version": 1,
        "source": "answer_verifier",
        "status_code": "answer_verification_unavailable",
        "issue_codes": [record.reason_code],
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
        "reason_code": record.reason_code,
        "message_key": "agent.answer_verification.unavailable",
        "answer_verification": record,
        "checkpoint_id": checkpoint_id,
        "next_check_after": if needs_user { None } else { Some(now.saturating_add(retry_after)) },
        "can_poll": true,
        "can_cancel": true,
        "last_heartbeat_ts": now,
        "provider_blocker_active": provider.is_some(),
        "provider_status": provider.map(|blocker| blocker.to_machine_json()),
    }));
    crate::assistant_presentation::abort_for_verifier_retry(state, task);
    journal.record_final_stop_signal("answer_verification_unavailable");
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
#[path = "task_answer_verification_tests.rs"]
mod tests;
