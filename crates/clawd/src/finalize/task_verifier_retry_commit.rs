use crate::{AppState, ClaimedTask, RouteResult};

pub(super) async fn try_commit_answer_verifier_retry_answer(
    state: &AppState,
    task: &ClaimedTask,
    prompt: &str,
    route_result: &RouteResult,
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

    let answer_contract =
        crate::answer_verifier::AnswerContract::new(prompt, route_result.output_contract.clone());
    let retry_verifier = crate::answer_verifier::verify_answer_observe_only(
        state,
        task,
        prompt,
        &answer_contract,
        journal,
        &retried_answer,
    )
    .await;
    let retry_verifier_accepts = retry_verifier
        .as_ref()
        .is_none_or(|verifier| verifier.pass && !verifier.high_confidence_gap());
    if !retry_verifier_accepts {
        if let Some(retry_verifier) = retry_verifier {
            journal.record_answer_verifier_summary(retry_verifier);
        }
        return false;
    }

    *answer_text = retried_answer;
    answer_messages.retain(|message| crate::finalize::is_execution_summary_message(message));
    answer_messages.push(answer_text.clone());
    journal.record_final_answer(answer_text.as_str());
    journal.answer_verifier_summary = None;
    true
}
