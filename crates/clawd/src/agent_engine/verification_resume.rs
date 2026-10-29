//! Resume an unchanged verification candidate without another planning/tool turn.
use super::*;
use crate::answer_verifier::{AnswerVerificationOutcome, AnswerVerificationRecord};
use crate::task_journal::{TaskJournal, TaskJournalFinalStatus};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn hash(value: &impl serde::Serialize) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).expect("serializable verification data"))
    )
}

fn evidence(checkpoint: &crate::task_lifecycle::TaskCheckpoint) -> String {
    hash(
        &json!({"capabilities":checkpoint.capability_results,"observations":checkpoint.observations,
        "resume_state":checkpoint.boundary_context.get("agent_loop_resume_state"),
        "completed":checkpoint.completed_side_effect_refs}),
    )
}

fn revision(state: &AppState, task: &ClaimedTask) -> Result<Value, String> {
    crate::repo::conversation_inputs::conversation_execution_snapshot_for_task(&state.core.db,&task.task_id)
        .map(|value|value.map(|v|json!({"instruction_revision":v.instruction_revision,"execution_epoch":v.execution_epoch})).unwrap_or(Value::Null))
        .map_err(|_|"verification_conversation_snapshot_unavailable".into())
}

pub(crate) fn bind_checkpoint(
    state: &AppState,
    task: &ClaimedTask,
    journal: &TaskJournal,
    checkpoint: &mut Value,
) {
    let Some(candidate) = journal.answer_verification_candidate.as_deref() else {
        return;
    };
    let Some(record) = journal
        .answer_verification
        .as_ref()
        .filter(|r| r.matches(candidate))
    else {
        return;
    };
    let Ok(parsed) =
        serde_json::from_value::<crate::task_lifecycle::TaskCheckpoint>(checkpoint.clone())
    else {
        return;
    };
    let Ok(revision) = revision(state, task) else {
        return;
    };
    checkpoint["boundary_context"]["verification_resume"] = json!({"schema_version":1,
        "candidate":candidate,"verification":record,"request_digest":hash(&task.payload_json),
        "input_digest":hash(&journal.input_text),"evidence_digest":evidence(&parsed),"revision":revision,
        "complete_step_evidence":journal.step_results.len()<=128});
}

fn binding_matches(
    checkpoint: &crate::task_lifecycle::TaskCheckpoint,
    task: &ClaimedTask,
    user_text: &str,
    current_revision: &Value,
) -> Option<String> {
    let binding = checkpoint.boundary_context.get("verification_resume")?;
    if binding.get("schema_version")?.as_u64() != Some(1)
        || binding.get("complete_step_evidence")?.as_bool() != Some(true)
        || binding.get("request_digest")?.as_str() != Some(hash(&task.payload_json).as_str())
        || binding.get("input_digest")?.as_str() != Some(hash(&user_text).as_str())
        || binding.get("evidence_digest")?.as_str() != Some(evidence(checkpoint).as_str())
        || binding.get("revision")? != current_revision
    {
        return None;
    }
    let candidate = binding.get("candidate")?.as_str()?;
    let record: AnswerVerificationRecord =
        serde_json::from_value(binding.get("verification")?.clone()).ok()?;
    if record.schema_version != 1
        || record.status != crate::answer_verifier::AnswerVerificationStatus::Unavailable
    {
        return None;
    }
    record.matches(candidate).then(|| candidate.to_string())
}

fn authorized(state: &AppState, task: &ClaimedTask) -> bool {
    let Some(key) = task.user_key.as_deref() else {
        return false;
    };
    let Ok(Some(identity)) = crate::resolve_auth_identity_by_key(state, key) else {
        return false;
    };
    crate::repo::get_task_delivery_record(state, &task.task_id)
        .ok()
        .flatten()
        .is_some_and(|record| {
            record.owner_principal_id.as_deref() == Some(identity.principal_id.as_str())
                && record.task.user_key.as_deref() == Some(key)
        })
}

pub(super) async fn try_resume(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    context: Option<&AgentRunContext>,
    checkpoint: &crate::task_lifecycle::TaskCheckpoint,
    loop_state: &mut LoopState,
) -> Result<Option<AskReply>, String> {
    if checkpoint
        .boundary_context
        .get("verification_resume")
        .is_none()
    {
        return Ok(None);
    }
    ensure_task_running(state, task)?;
    if !authorized(state, task) {
        return Err("verification_resume_authorization_revoked".into());
    }
    if crate::repo::conversation_inputs::task_has_pending_conversation_inputs(
        &state.core.db,
        &task.task_id,
    )
    .map_err(|_| "verification_input_lookup_failed")?
        || !crate::repo::pending_task_control_directives(state, &task.task_id, 1)
            .map_err(|_| "verification_control_lookup_failed")?
            .is_empty()
    {
        return Ok(None);
    }
    let current_revision = revision(state, task)?;
    let Some(candidate) = binding_matches(checkpoint, task, user_text, &current_revision) else {
        return Ok(None);
    };
    let mut journal = crate::finalize::build_terminal_from_loop_state(
        state,
        task,
        user_text,
        loop_state,
        context,
        None,
        true,
        &candidate,
        TaskJournalFinalStatus::Success,
    )
    .await;
    let contract = crate::answer_verifier::AnswerContract::new(
        user_text,
        loop_state.output_contract.clone().unwrap_or_default(),
    );
    let outcome = crate::answer_verifier::verify_answer_observe_only(
        state, task, user_text, &contract, &journal, &candidate,
    )
    .await;
    ensure_task_running(state, task)?;
    if !authorized(state, task) {
        return Err("verification_resume_authorization_revoked".into());
    }
    if current_revision != revision(state, task)?
        || crate::repo::conversation_inputs::task_has_pending_conversation_inputs(
            &state.core.db,
            &task.task_id,
        )
        .map_err(|_| "verification_input_lookup_failed")?
        || !crate::repo::pending_task_control_directives(state, &task.task_id, 1)
            .map_err(|_| "verification_control_lookup_failed")?
            .is_empty()
    {
        return Ok(None);
    }
    let accepted = matches!(outcome, AnswerVerificationOutcome::Passed(_));
    let unavailable = matches!(outcome, AnswerVerificationOutcome::Unavailable { .. });
    journal.record_answer_verification(outcome, &candidate);
    journal.record_runtime_llm_metrics(state, &task.task_id);
    if unavailable {
        crate::answer_verifier::checkpoint_unavailable_verification(state, task, &mut journal);
        journal.record_final_answer("");
        return Ok(Some(
            AskReply::non_llm(String::new()).with_task_journal(journal),
        ));
    }
    if !accepted {
        loop_state.task_observations.push(json!({"observation_kind":"answer_verification","status":"failed","reason_code":"verification_resume_rejected","answer_verification":journal.answer_verification}));
        return Ok(None);
    }
    let rev = current_revision
        .get("instruction_revision")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let epoch = current_revision
        .get("execution_epoch")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    match crate::repo::conversation_inputs::claim_conversation_terminal_boundary(
        &state.core.db,
        &task.task_id,
        rev,
        epoch,
    )
    .map_err(|_| "verification_terminal_boundary_failed")?
    {
        crate::repo::conversation_inputs::ConversationTerminalBoundaryOutcome::PendingOrStale => {
            return Ok(None)
        }
        _ => {}
    }
    journal.task_lifecycle = None;
    journal.task_checkpoint = None;
    journal.record_final_stop_signal("verification_resume_completed");
    Ok(Some(
        AskReply::non_llm(candidate.clone())
            .with_messages(vec![candidate])
            .with_task_journal(journal),
    ))
}

#[cfg(test)]
#[path = "verification_resume_tests.rs"]
mod tests;
