use super::task_machine_kv_summary::recover_requested_machine_kv_summary_final_answer;
use super::task_structured_evidence_table::deterministic_structured_evidence_table_recovery;
use super::task_tree_summary_recovery::deterministic_tree_summary_rows_failure_recovery;

pub(super) fn mark_answer_verifier_recovered_by_deterministic_observed_evidence(
    journal: &mut crate::task_journal::TaskJournal,
) {
    journal.record_answer_verifier_summary(crate::answer_verifier::AnswerVerifierOut {
        pass: true,
        missing_evidence_fields: Vec::new(),
        answer_incomplete_reason: String::new(),
        should_retry: false,
        retry_instruction: String::new(),
        confidence: 1.0,
    });
}

pub(super) fn recover_answer_verifier_gap_with_deterministic_machine_evidence(
    prompt: &str,
    route_result: &crate::RouteResult,
    journal: &mut crate::task_journal::TaskJournal,
    answer_text: &mut String,
    answer_messages: &mut Vec<String>,
) -> bool {
    if let Some(recovered_answer) =
        deterministic_structured_evidence_table_recovery(route_result, journal, false)
    {
        *answer_text = recovered_answer;
        answer_messages.clear();
        answer_messages.push(answer_text.clone());
        journal.record_final_answer(answer_text.as_str());
        mark_answer_verifier_recovered_by_deterministic_observed_evidence(journal);
        return true;
    }
    if let Some(recovered_answer) = deterministic_tree_summary_rows_failure_recovery(journal) {
        *answer_text = recovered_answer;
        answer_messages.clear();
        answer_messages.push(answer_text.clone());
        journal.record_final_answer(answer_text.as_str());
        mark_answer_verifier_recovered_by_deterministic_observed_evidence(journal);
        return true;
    }
    recover_requested_machine_kv_summary_final_answer(
        prompt,
        route_result,
        journal,
        answer_text,
        answer_messages,
        true,
    )
}

pub(super) fn recover_raw_command_machine_field_final_answer(
    route_result: &crate::RouteResult,
    journal: &mut crate::task_journal::TaskJournal,
    answer_text: &mut String,
    answer_messages: &mut Vec<String>,
) -> bool {
    let Some(answer) =
        crate::finalize::raw_command_machine_field_projection_from_journal(route_result, journal)
    else {
        return false;
    };
    if answer.trim() == answer_text.trim() {
        return false;
    }
    *answer_text = answer;
    answer_messages.clear();
    answer_messages.push(answer_text.clone());
    journal.record_final_answer(answer_text.as_str());
    mark_answer_verifier_recovered_by_deterministic_observed_evidence(journal);
    true
}
