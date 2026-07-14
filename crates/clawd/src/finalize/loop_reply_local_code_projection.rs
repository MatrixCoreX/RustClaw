use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::ClaimedTask;

use super::final_answer_text_from_delivery;

pub(super) fn attach_local_code_strict_json_projection(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(answer) = crate::agent_engine::local_code_strict_json_projection_from_machine_evidence(
        user_text,
        loop_state,
        agent_run_context,
    ) else {
        return false;
    };
    if current_delivery_should_prevent_local_code_projection(
        user_text,
        loop_state,
        agent_run_context,
        &answer,
    ) {
        if let Some(candidate) = current_delivery_candidate(loop_state)
            .filter(|candidate| json_values_equivalent(candidate, &answer))
            .map(str::to_string)
        {
            record_local_code_strict_json_projection(loop_state, candidate);
            record_local_code_projection_summary(loop_state, finalizer_summary);
        }
        return false;
    }

    loop_state.delivery_messages.clear();
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        answer.clone(),
    );
    loop_state.last_user_visible_respond = Some(answer.clone());
    loop_state.last_publishable_synthesis_output = Some(answer.clone());
    record_local_code_strict_json_projection(loop_state, answer);
    record_local_code_projection_summary(loop_state, finalizer_summary);
    true
}

fn record_local_code_strict_json_projection(loop_state: &mut LoopState, answer: String) {
    loop_state.output_vars.insert(
        "agent_loop.strict_json_projection_publishable".to_string(),
        "true".to_string(),
    );
    loop_state.output_vars.insert(
        "agent_loop.strict_json_projection_output".to_string(),
        answer,
    );
}

fn record_local_code_projection_summary(
    loop_state: &LoopState,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) {
    *finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        parsed: true,
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: loop_state.executed_step_results.len(),
        ..Default::default()
    });
}

pub(super) fn sync_final_delivery_with_local_code_projection(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
    delivery_deduped: &mut Vec<String>,
) -> bool {
    if !attach_local_code_strict_json_projection(
        task,
        user_text,
        loop_state,
        agent_run_context,
        finalizer_summary,
    ) {
        return false;
    }
    *delivery_deduped = loop_state.delivery_messages.clone();
    true
}

pub(super) fn sync_recorded_local_code_projection_if_needed(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
    delivery_deduped: &mut Vec<String>,
) -> bool {
    if loop_state
        .output_vars
        .get("agent_loop.strict_json_projection_publishable")
        .map(String::as_str)
        != Some("true")
    {
        return false;
    }
    let Some(answer) = loop_state
        .output_vars
        .get("agent_loop.strict_json_projection_output")
        .map(String::as_str)
        .map(str::trim)
        .filter(|answer| !answer.is_empty())
        .map(str::to_string)
    else {
        return false;
    };
    if !projection_has_only_local_code_fields(&answer) {
        return false;
    }
    if !crate::agent_engine::local_code_strict_json_answer_satisfies_request(
        user_text,
        &answer,
        agent_run_context,
    ) {
        return false;
    }
    sync_local_code_projection_answer(
        task,
        user_text,
        loop_state,
        agent_run_context,
        finalizer_summary,
        delivery_deduped,
        answer,
    )
}

pub(super) fn sync_latest_synthesis_local_code_projection_if_needed(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
    delivery_deduped: &mut Vec<String>,
) -> bool {
    let Some(answer) =
        latest_synthesis_local_code_projection(user_text, loop_state, agent_run_context)
    else {
        return false;
    };
    sync_local_code_projection_answer(
        task,
        user_text,
        loop_state,
        agent_run_context,
        finalizer_summary,
        delivery_deduped,
        answer,
    )
}

fn latest_synthesis_local_code_projection(
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && step.skill == "synthesize_answer")
        .filter_map(|step| step.output.as_deref())
        .map(str::trim)
        .filter(|answer| !answer.is_empty())
        .find(|answer| {
            projection_has_only_local_code_fields(answer)
                && crate::agent_engine::local_code_strict_json_answer_satisfies_request(
                    user_text,
                    answer,
                    agent_run_context,
                )
        })
        .map(str::to_string)
}

fn sync_local_code_projection_answer(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
    delivery_deduped: &mut Vec<String>,
    answer: String,
) -> bool {
    let current = final_answer_text_from_delivery(delivery_deduped);
    if crate::agent_engine::local_code_strict_json_answer_satisfies_request(
        user_text,
        &current,
        agent_run_context,
    ) && !local_code_projection_is_richer(&answer, &current)
    {
        return false;
    }

    delivery_deduped.clear();
    delivery_deduped.push(answer.clone());
    loop_state.delivery_messages.clear();
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        answer.clone(),
    );
    loop_state.last_user_visible_respond = Some(answer.clone());
    loop_state.last_publishable_synthesis_output = Some(answer.clone());
    record_local_code_strict_json_projection(loop_state, answer);
    record_local_code_projection_summary(loop_state, finalizer_summary);
    true
}

fn projection_has_only_local_code_fields(answer: &str) -> bool {
    let Ok(serde_json::Value::Object(object)) =
        serde_json::from_str::<serde_json::Value>(answer.trim())
    else {
        return false;
    };
    !object.is_empty()
        && object
            .keys()
            .all(|key| local_code_projection_field_key(key.as_str()))
}

fn local_code_projection_field_key(key: &str) -> bool {
    matches!(
        key,
        "created_files"
            | "changed_files"
            | "failed_command"
            | "failure_observed"
            | "failure_evidence"
            | "fix_summary"
            | "test_command"
            | "verification_command"
            | "test_status"
            | "functions"
            | "error_codes"
            | "evidence_files"
            | "project_dir"
            | "diff_summary"
    )
}

fn current_delivery_should_prevent_local_code_projection(
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    projected_answer: &str,
) -> bool {
    let candidate = loop_state
        .delivery_messages
        .iter()
        .rev()
        .map(|message| message.trim())
        .find(|message| !message.is_empty());
    candidate.is_some_and(|candidate| {
        crate::agent_engine::local_code_strict_json_answer_satisfies_request(
            user_text,
            candidate,
            agent_run_context,
        ) && !local_code_projection_is_richer(projected_answer, candidate)
    })
}

fn current_delivery_candidate(loop_state: &LoopState) -> Option<&str> {
    loop_state
        .delivery_messages
        .iter()
        .rev()
        .map(|message| message.trim())
        .find(|message| !message.is_empty())
}

fn json_values_equivalent(left: &str, right: &str) -> bool {
    match (
        serde_json::from_str::<serde_json::Value>(left.trim()),
        serde_json::from_str::<serde_json::Value>(right.trim()),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => left.trim() == right.trim(),
    }
}

fn local_code_projection_is_richer(projected_answer: &str, existing_answer: &str) -> bool {
    let Ok(serde_json::Value::Object(projected)) =
        serde_json::from_str::<serde_json::Value>(projected_answer.trim())
    else {
        return false;
    };
    let Ok(serde_json::Value::Object(existing)) =
        serde_json::from_str::<serde_json::Value>(existing_answer.trim())
    else {
        return false;
    };

    [
        "functions",
        "error_codes",
        "changed_files",
        "created_files",
        "evidence_files",
    ]
    .iter()
    .any(|field| {
        let projected_values = string_array_values(projected.get(*field));
        let existing_values = string_array_values(existing.get(*field));
        !projected_values.is_empty()
            && projected_values.len() > existing_values.len()
            && existing_values
                .iter()
                .all(|value| projected_values.iter().any(|candidate| candidate == value))
    }) || test_command_projection_is_richer(
        projected.get("test_command"),
        existing.get("test_command"),
    )
}

fn string_array_values(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn test_command_projection_is_richer(
    projected: Option<&serde_json::Value>,
    existing: Option<&serde_json::Value>,
) -> bool {
    let projected_values = string_or_array_values(projected);
    let existing_values = string_or_array_values(existing);
    !projected_values.is_empty()
        && projected_values.len() > existing_values.len()
        && existing_values
            .iter()
            .all(|value| projected_values.iter().any(|candidate| candidate == value))
}

fn string_or_array_values(value: Option<&serde_json::Value>) -> Vec<String> {
    match value {
        Some(serde_json::Value::String(value)) => {
            let value = value.trim();
            if value.is_empty() {
                Vec::new()
            } else {
                vec![value.to_string()]
            }
        }
        Some(serde_json::Value::Array(_)) => string_array_values(value),
        _ => Vec::new(),
    }
}

#[cfg(test)]
#[path = "loop_reply_local_code_projection_tests.rs"]
mod tests;
