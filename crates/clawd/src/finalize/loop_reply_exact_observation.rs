use crate::agent_engine::{AgentRunContext, LoopState};
use crate::{AppState, ClaimedTask};

pub(super) fn looks_like_structured_machine_output(answer: &str) -> bool {
    let trimmed = answer.trim();
    serde_json::from_str::<serde_json::Value>(trimmed)
        .map(|value| value.is_object() || value.is_array())
        .unwrap_or(false)
        || looks_like_contract_evidence_projection(trimmed)
        || looks_like_structured_key_path_projection(trimmed)
        || looks_like_multiline_machine_field_projection(trimmed)
}

fn looks_like_contract_evidence_projection(answer: &str) -> bool {
    let mut has_path = false;
    let mut has_evidence_field = false;
    for line in answer.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        if line.starts_with("path=") || line.starts_with("resolved_path=") {
            has_path = true;
            continue;
        }
        if matches!(
            line,
            "content_excerpt:" | "field_value:" | "command_output:" | "candidates:" | "results:"
        ) {
            has_evidence_field = true;
        }
    }
    has_path && has_evidence_field
}

fn looks_like_structured_key_path_projection(answer: &str) -> bool {
    let lines = answer
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return false;
    }
    let mut key_path_assignments = 0usize;
    for line in &lines {
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        let key = key.trim();
        if value.trim().is_empty() || !valid_machine_projection_key(key) {
            return false;
        }
        if key.contains('.') || key.contains('[') || key.contains(']') {
            key_path_assignments += 1;
        }
    }
    key_path_assignments > 0
}

fn looks_like_multiline_machine_field_projection(answer: &str) -> bool {
    let lines = answer
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() < 2 {
        return false;
    }
    lines.iter().all(|line| {
        line.split_once('=').is_some_and(|(key, value)| {
            valid_machine_projection_key(key.trim()) && !value.trim().is_empty()
        })
    })
}

fn valid_machine_projection_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '[' | ']'))
}

pub(super) fn direct_exact_observation_output_projection(
    _state: &AppState,
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route.requests_exact_command_output()
        || route.delivery_required
        || matches!(route.response_shape, crate::OutputResponseShape::FileToken)
    {
        return None;
    }
    let answer = exact_selector_value(route, &loop_state.capability_results)
        .or_else(|| latest_successful_observation(loop_state).map(str::to_string))?;
    let answer = answer.trim_end().to_string();
    if answer.trim().is_empty() {
        return None;
    }
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            parsed: true,
            contract_ok: true,
            completion_ok: Some(true),
            grounded_ok: Some(true),
            format_ok: Some(true),
            needs_clarify: Some(false),
            used_evidence_ids_count: 1,
            ..Default::default()
        },
    ))
}

fn exact_selector_value(
    route: &crate::IntentOutputContract,
    results: &[claw_core::capability_result::CapabilityResultEnvelope],
) -> Option<String> {
    let fields = route
        .selection
        .structured_field_selector
        .as_deref()
        .and_then(crate::machine_kv_projection::exact_machine_field_selector)?;
    let [field] = fields.as_slice() else {
        return None;
    };
    crate::capability_result::selected_exact_machine_result(results, field)
}

fn latest_successful_observation(loop_state: &LoopState) -> Option<&str> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok())
        .filter(|step| {
            !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think"
            )
        })
        .find_map(|step| step.output.as_deref())
        .map(str::trim)
        .filter(|output| !output.is_empty())
}

pub(crate) fn exact_observation_machine_field_delivery_satisfies_request(
    route: &crate::IntentOutputContract,
    delivery: &str,
) -> bool {
    route.requests_exact_command_output() && !delivery.trim().is_empty()
}

pub(crate) fn exact_observation_machine_field_projection_from_journal(
    route: &crate::IntentOutputContract,
    journal: &crate::task_journal::TaskJournal,
) -> Option<String> {
    if !route.requests_exact_command_output() || route.delivery_required {
        return None;
    }
    journal
        .step_results
        .iter()
        .rev()
        .filter(|step| step.status == crate::executor::StepExecutionStatus::Ok)
        .find_map(|step| step.output_excerpt.as_deref())
        .map(str::trim_end)
        .filter(|output| !output.trim().is_empty())
        .map(str::to_string)
}

pub(super) fn replace_final_delivery_with_exact_observation_machine_field_projection(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
    delivery_messages: &mut Vec<String>,
) -> bool {
    let Some(route) = agent_run_context.and_then(AgentRunContext::output_contract) else {
        return false;
    };
    let Some((answer, summary)) =
        direct_exact_observation_output_projection(state, route, loop_state)
    else {
        return false;
    };
    if delivery_messages
        .last()
        .is_some_and(|message| message.trim() == answer.trim())
    {
        loop_state.last_user_visible_respond = Some(answer);
        *finalizer_summary = Some(summary);
        return false;
    }
    delivery_messages.clear();
    delivery_messages.push(answer.clone());
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(summary);
    super::log_deterministic_delivery_record(
        &task.task_id,
        "final_exact_machine_projection",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

pub(super) fn exact_observation_output_needs_structural_projection(
    _route: &crate::IntentOutputContract,
    _loop_state: &LoopState,
) -> bool {
    false
}

pub(super) fn output_contract_requests_exact_delivery(route: &crate::IntentOutputContract) -> bool {
    route.requests_exact_list()
        || matches!(
            route.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
        || route.requests_exact_command_output()
}
