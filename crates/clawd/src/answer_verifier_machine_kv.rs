pub(super) fn requested_machine_kv_projection_can_skip_answer_verifier(
    route_result: &super::AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    let contract = route_result.effective_output_contract();
    if contract.delivery_required {
        return false;
    }
    let candidate = candidate_answer.trim();
    if candidate.is_empty() {
        return false;
    }
    let observed_texts = nonterminal_observed_machine_text_fragments(journal);
    if observed_texts.is_empty() {
        return false;
    }
    crate::machine_kv_projection::requested_machine_kv_summary_from_observation_inputs(
        [
            journal.input_text.as_str(),
            route_result.request_text.as_str(),
        ],
        &observed_texts,
    )
    .is_some_and(|answer| answer.trim() == candidate)
}

fn nonterminal_observed_machine_text_fragments(
    journal: &crate::task_journal::TaskJournal,
) -> Vec<String> {
    let mut values = Vec::new();
    for step in &journal.step_results {
        if step.status != crate::executor::StepExecutionStatus::Ok
            || answer_verifier_terminal_step_kind(step.skill.as_str())
        {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        crate::machine_kv_projection::collect_machine_text_fragments_from_output(
            output,
            &mut values,
        );
    }
    values.sort();
    values.dedup();
    values
}

fn answer_verifier_terminal_step_kind(skill: &str) -> bool {
    matches!(
        skill,
        "respond" | "synthesize_answer" | "think" | "answer_verifier"
    )
}
