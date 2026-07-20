pub(super) fn apply_exact_observation_machine_field_delivery(
    route_result: &crate::IntentOutputContract,
    journal: &mut crate::task_journal::TaskJournal,
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
    journal.record_answer_verifier_summary(crate::answer_verifier::AnswerVerifierOut {
        pass: true,
        missing_evidence_fields: Vec::new(),
        answer_incomplete_reason: String::new(),
        should_retry: false,
        retry_instruction: String::new(),
        confidence: 1.0,
    });
    true
}
