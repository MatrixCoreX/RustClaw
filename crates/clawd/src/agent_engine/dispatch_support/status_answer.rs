use super::{AgentRunContext, LoopState};

pub(super) fn agent_loop_rich_content_should_defer_status(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &LoopState,
) -> bool {
    let Some(route) = agent_run_context.and_then(|context| context.route_result.as_ref()) else {
        return false;
    };
    route.output_contract.response_shape == crate::OutputResponseShape::Free
        && !route.output_contract.delivery_required
        && successful_content_observation_count(loop_state) >= 2
}

fn successful_content_observation_count(loop_state: &LoopState) -> usize {
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| {
            step.is_ok()
                && !matches!(
                    step.skill.as_str(),
                    "respond" | "synthesize_answer" | "think"
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
