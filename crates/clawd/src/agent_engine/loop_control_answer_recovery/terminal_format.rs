use super::*;

pub(super) fn terminal_model_output_format_gap_satisfies_contract(
    reply: &AskReply,
    route: &crate::answer_verifier::AnswerContract,
) -> bool {
    if !route_allows_terminal_model_answer(route) {
        return false;
    }
    let Some(journal) = reply.task_journal.as_ref() else {
        return false;
    };
    let Some(summary) = journal.answer_verifier_summary.as_ref() else {
        return false;
    };
    if !summary.high_confidence_retry_gap()
        || summary.missing_evidence_fields.is_empty()
        || !summary
            .missing_evidence_fields
            .iter()
            .all(|field| field == "output_format")
    {
        return false;
    }
    if !crate::task_journal::evidence_coverage_for_output_contract(
        &route.effective_output_contract(),
        journal,
    )
    .is_complete()
    {
        return false;
    }
    if !finalizer_accepts_terminal_model_answer(route, journal) {
        return false;
    }
    let Some(answer) = final_user_answer_candidate(reply).map(str::trim) else {
        return false;
    };
    !answer_is_machine_or_internal(answer) && terminal_step_matches_answer(journal, answer)
}

pub(in crate::agent_engine::loop_control) fn prefer_terminal_model_answer_for_verifier_candidate(
    reply: &mut AskReply,
    route: Option<&crate::answer_verifier::AnswerContract>,
) -> bool {
    let Some(route) = route else {
        return false;
    };
    if !route_allows_terminal_model_answer(route) {
        return false;
    }
    let Some(journal) = reply.task_journal.as_ref() else {
        return false;
    };
    if !crate::task_journal::evidence_coverage_for_output_contract(
        &route.effective_output_contract(),
        journal,
    )
    .is_complete()
    {
        return false;
    }
    if !finalizer_accepts_terminal_model_answer(route, journal) {
        return false;
    }
    let Some(answer) = latest_terminal_model_answer(journal) else {
        return false;
    };
    if answer_is_machine_or_internal(answer.as_str()) || answer.trim() == reply.text.trim() {
        return false;
    }
    let output_contract = route.effective_output_contract();
    if crate::finalize::exact_observation_machine_field_delivery_satisfies_request(
        &output_contract,
        reply.text.as_str(),
    ) && !crate::finalize::exact_observation_machine_field_delivery_satisfies_request(
        &output_contract,
        answer.as_str(),
    ) {
        return false;
    }
    if terminal_model_answer_is_lossy_observed_scalar(journal, answer.as_str(), reply.text.as_str())
    {
        return false;
    }
    let mut messages = reply
        .messages
        .iter()
        .filter(|message| crate::finalize::is_execution_summary_message(message))
        .cloned()
        .collect::<Vec<_>>();
    messages.push(answer.clone());
    if let Some(journal) = reply.task_journal.as_mut() {
        journal.record_final_answer(answer.as_str());
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    }
    reply.text = answer;
    reply.messages = messages;
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = true;
    tracing::info!("answer_verifier_candidate_preferred_terminal_model_answer");
    true
}

fn finalizer_accepts_terminal_model_answer(
    route: &crate::answer_verifier::AnswerContract,
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    let Some(finalizer) = journal.finalizer_summary.as_ref() else {
        return false;
    };
    finalizer.disposition == Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        && finalizer.contract_ok
        && finalizer.completion_ok != Some(false)
        && finalizer.grounded_ok != Some(false)
        && finalizer.format_ok != Some(false)
        && finalizer.needs_clarify != Some(true)
        && (!route.output_contract.requires_content_evidence
            || finalizer.used_evidence_ids_count > 0)
}

fn route_allows_terminal_model_answer(route: &crate::answer_verifier::AnswerContract) -> bool {
    if route.output_contract.delivery_required {
        return false;
    }
    crate::evidence_policy::final_answer_shape_for_output_contract(&route.output_contract)
        .map(crate::evidence_policy::FinalAnswerShape::allows_model_language)
        .unwrap_or_else(|| {
            matches!(
                route.output_contract.response_shape,
                crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
            ) || route.output_contract.exact_sentence_count.is_some()
        })
}

fn answer_is_machine_or_internal(answer: &str) -> bool {
    answer.is_empty()
        || serde_json::from_str::<serde_json::Value>(answer).is_ok()
        || crate::finalize::looks_like_planner_artifact(answer)
        || crate::finalize::looks_like_internal_trace_artifact(answer)
        || crate::finalize::is_execution_summary_message(answer)
}

fn terminal_step_matches_answer(journal: &crate::task_journal::TaskJournal, answer: &str) -> bool {
    journal.step_results.iter().rev().any(|step| {
        matches!(step.skill.as_str(), "respond" | "synthesize_answer")
            && step.status == crate::executor::StepExecutionStatus::Ok
            && step
                .output_excerpt
                .as_deref()
                .map(str::trim)
                .is_some_and(|output| output == answer)
    })
}

fn latest_terminal_model_answer(journal: &crate::task_journal::TaskJournal) -> Option<String> {
    journal.step_results.iter().rev().find_map(|step| {
        if !matches!(step.skill.as_str(), "respond" | "synthesize_answer")
            || step.status != crate::executor::StepExecutionStatus::Ok
        {
            return None;
        }
        step.output_excerpt
            .as_deref()
            .map(str::trim)
            .filter(|answer| !answer.is_empty())
            .map(str::to_string)
    })
}

fn terminal_model_answer_is_lossy_observed_scalar(
    journal: &crate::task_journal::TaskJournal,
    terminal_answer: &str,
    current_answer: &str,
) -> bool {
    let terminal_answer = terminal_answer.trim();
    if terminal_answer.is_empty() || !visible_answer_is_machine_field_projection(current_answer) {
        return false;
    }
    journal
        .step_results
        .iter()
        .filter(|step| step.status == crate::executor::StepExecutionStatus::Ok)
        .filter(|step| {
            !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think"
            )
        })
        .filter_map(|step| step.output_excerpt.as_deref())
        .filter_map(|output| serde_json::from_str::<serde_json::Value>(output.trim()).ok())
        .any(|value| {
            let mut scalars = Vec::new();
            collect_observed_scalar_values(&value, &mut scalars);
            scalars.iter().any(|value| value == terminal_answer)
        })
}

fn visible_answer_is_machine_field_projection(answer: &str) -> bool {
    let answer = answer.trim();
    if answer.is_empty() {
        return false;
    }
    let mut field_count = 0usize;
    let mut token_count = 0usize;
    for token in answer.split_whitespace() {
        token_count += 1;
        let Some((key, value)) = token.split_once('=') else {
            continue;
        };
        if machine_projection_field_key(key.trim()) && !value.trim().is_empty() {
            field_count += 1;
            if field_count >= 2 {
                return true;
            }
        }
    }
    field_count == 1 && token_count == 1
}

fn machine_projection_field_key(key: &str) -> bool {
    !key.is_empty()
        && key.chars().all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '_' | '.' | '-')
        })
        && key.chars().any(|ch| ch.is_ascii_lowercase())
}

fn collect_observed_scalar_values(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, child) in object {
                if matches!(key.as_str(), "text" | "error_text") {
                    continue;
                }
                if let Some(scalar) = json_scalar_to_string(child) {
                    out.push(scalar);
                }
                collect_observed_scalar_values(child, out);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_observed_scalar_values(item, out);
            }
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {}
    }
}

fn json_scalar_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.trim().to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Null | serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            None
        }
    }
    .filter(|value| !value.is_empty())
}
