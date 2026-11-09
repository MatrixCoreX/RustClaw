use crate::task_journal::TaskJournal;

pub(super) fn prepare_verification_checkpoint_for_model_wait(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
    journal: &mut TaskJournal,
) -> bool {
    let lifecycle = journal.task_lifecycle.as_ref();
    if lifecycle
        .and_then(|value| value.get("state"))
        .and_then(serde_json::Value::as_str)
        != Some("waiting")
    {
        return false;
    }
    let reason = lifecycle
        .and_then(|value| value.get("resume_reason"))
        .and_then(serde_json::Value::as_str);
    if reason != Some(claw_core::provider_failure_policy::PROVIDER_WAIT_RESUME_REASON)
        && reason != Some("resource_admission_wait")
    {
        return false;
    }
    crate::answer_verifier::checkpoint_unavailable_verification(state, task, journal)
}

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

#[cfg(test)]
#[path = "task_answer_verification_tests.rs"]
mod tests;
