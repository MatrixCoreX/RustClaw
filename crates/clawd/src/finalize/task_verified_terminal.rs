use serde_json::Value;

pub(super) fn verified_terminal_answer_after_verifier_pass(
    journal: &crate::task_journal::TaskJournal,
) -> Option<String> {
    let verifier = journal.answer_verifier_summary.as_ref()?;
    if !verifier.pass || verifier.high_confidence_retry_gap() {
        return None;
    }
    journal
        .step_results
        .iter()
        .rev()
        .find_map(verified_terminal_answer_from_step)
}

fn verified_terminal_answer_from_step(
    step: &crate::task_journal::TaskJournalStepTrace,
) -> Option<String> {
    if !matches!(step.skill.as_str(), "respond" | "synthesize_answer")
        || step.status != crate::executor::StepExecutionStatus::Ok
    {
        return None;
    }
    let answer = step.output_excerpt.as_deref()?.trim();
    if answer.is_empty()
        || terminal_answer_is_internal_machine_payload(answer)
        || crate::finalize::looks_like_planner_artifact(answer)
        || crate::finalize::looks_like_internal_trace_artifact(answer)
        || crate::finalize::is_execution_summary_message(answer)
    {
        return None;
    }
    Some(answer.to_string())
}

fn terminal_answer_is_internal_machine_payload(answer: &str) -> bool {
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(answer.trim()) else {
        return false;
    };
    if object.contains_key("owner_layer")
        || object
            .get("output_format")
            .and_then(Value::as_str)
            .is_some_and(|format| format == "machine_json")
    {
        return true;
    }
    [
        "message_key",
        "reason_code",
        "error_code",
        "missing_evidence_fields",
        "answer_incomplete_reason",
    ]
    .iter()
    .any(|key| object.contains_key(*key))
}
