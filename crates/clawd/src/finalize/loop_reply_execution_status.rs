use crate::agent_engine::AgentRunContext;
use crate::AppState;

use super::{
    missing_file_path_from_loop, output_excerpt_has_missing_file_evidence,
    step_error_has_missing_file_evidence,
};

pub(super) fn successful_content_observation_should_precede_status_summary(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &crate::agent_engine::LoopState,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) else {
        return false;
    };
    let agent_loop_rich_content = route.response_shape == crate::OutputResponseShape::Free
        && !route.delivery_required
        && successful_content_observation_count(loop_state) >= 2;
    if !route.requires_content_evidence && !agent_loop_rich_content {
        return false;
    }
    if route.requests_exact_command_output() {
        return false;
    }
    successful_content_observation_count(loop_state) > 0
}

fn successful_content_observation_count(loop_state: &crate::agent_engine::LoopState) -> usize {
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| {
            step.is_ok()
                && !matches!(
                    step.skill.as_str(),
                    "respond" | "think" | "synthesize_answer"
                )
                && step
                    .output
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(successful_content_observation_text)
        })
        .count()
}

fn successful_content_observation_text(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty()
        && !machine_separator_only_output(text)
        && !crate::finalize::is_execution_summary_message(text)
        && !crate::finalize::looks_like_planner_artifact(text)
        && !crate::finalize::looks_like_internal_trace_artifact(text)
}

fn machine_separator_only_output(text: &str) -> bool {
    let mut saw_line = false;
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        saw_line = true;
        if !(line.len() >= 6 && line.starts_with("---") && line.ends_with("---")) {
            return false;
        }
    }
    saw_line
}

pub(super) fn deterministic_missing_observed_target_answer(
    _state: &AppState,
    _user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let latest_missing_idx = loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .rev()
        .find_map(|(idx, step)| {
            (step
                .output
                .as_deref()
                .is_some_and(output_excerpt_has_missing_file_evidence)
                || step_error_has_missing_file_evidence(step))
            .then_some(idx)
        })?;
    let has_later_successful_observation = loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .skip(latest_missing_idx + 1)
        .any(|(_, step)| {
            step.is_ok()
                && !matches!(
                    step.skill.as_str(),
                    "respond" | "think" | "synthesize_answer"
                )
                && step.output.as_deref().map(str::trim).is_some_and(|output| {
                    !output.is_empty() && !output_excerpt_has_missing_file_evidence(output)
                })
        });
    if has_later_successful_observation {
        return None;
    }
    let path = missing_file_path_from_loop(loop_state, agent_run_context)?;
    let final_answer_shape = agent_run_context
        .and_then(|ctx| ctx.output_contract())
        .and_then(crate::evidence_policy::final_answer_shape_for_output_contract);
    let exact_count = agent_run_context
        .and_then(|ctx| ctx.output_contract())
        .is_some_and(crate::IntentOutputContract::requests_exact_count);
    let mut lines = vec![
        "schema_version=1".to_string(),
        "reason_code=missing_observed_target".to_string(),
        "exists=false".to_string(),
        format!("path=`{path}`"),
        "kind=missing".to_string(),
    ];
    if let Some(final_answer_shape) = final_answer_shape {
        lines.push(format!(
            "final_answer_shape={}",
            final_answer_shape.as_str()
        ));
    }
    if exact_count {
        lines.push("count_available=false".to_string());
    }
    Some(lines.join("\n"))
}

pub(super) fn deterministic_observed_execution_status_summary(
    loop_state: &crate::agent_engine::LoopState,
) -> crate::task_journal::TaskJournalFinalizerSummary {
    crate::task_journal::TaskJournalFinalizerSummary {
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
    }
}
