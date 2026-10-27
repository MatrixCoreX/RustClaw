use crate::{AppState, ClaimedTask, IntentOutputContract};

pub(super) async fn try_commit_answer_verifier_retry_answer(
    state: &AppState,
    task: &ClaimedTask,
    prompt: &str,
    route_result: &IntentOutputContract,
    journal: &mut crate::task_journal::TaskJournal,
    answer_text: &mut String,
    answer_messages: &mut Vec<String>,
    retried_answer: String,
) -> bool {
    if !super::answer_verifier_retry_answer_has_required_machine_evidence(
        Some(journal),
        &retried_answer,
    ) {
        tracing::info!(
            "finalize_answer_verifier_retry_rejected_missing_machine_validation_evidence task_id={}",
            task.task_id
        );
        return false;
    }

    let answer_contract = crate::answer_verifier::AnswerContract::new(prompt, route_result.clone());
    let retry_verifier = crate::answer_verifier::verify_answer_observe_only(
        state,
        task,
        prompt,
        &answer_contract,
        journal,
        &retried_answer,
    )
    .await;
    let retry_verifier_accepts = retry_verifier.accepts_candidate();
    if !retry_verifier_accepts {
        journal.record_answer_verification(retry_verifier, &retried_answer);
        return false;
    }

    let retried_answer =
        super::preserve_verified_delivery_tokens_after_retry(journal, answer_text, retried_answer);
    *answer_text = retried_answer;
    answer_messages.retain(|message| crate::finalize::is_execution_summary_message(message));
    answer_messages.push(answer_text.clone());
    journal.record_final_answer(answer_text.as_str());
    journal.answer_verifier_summary = None;
    journal.record_answer_verification(retry_verifier, answer_text);
    true
}
