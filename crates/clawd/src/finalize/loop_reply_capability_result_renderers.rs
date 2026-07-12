use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::{AppState, ClaimedTask};

pub(super) fn attach_config_edit_observed_answer_from_registry(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let rendered = if let Some((answer, summary)) =
        super::config_edit::direct_config_edit_observed_answer(state, user_text, loop_state)
    {
        if loop_state.delivery_messages.is_empty() {
            *finalizer_summary = Some(summary);
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            super::log_deterministic_delivery_record(
                &task.task_id,
                "fallback_from_config_edit_observed",
                "attached",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
            true
        } else if delivery_is_replaceable_machine_marker(&loop_state.delivery_messages) {
            *finalizer_summary = Some(summary);
            loop_state.delivery_messages.clear();
            loop_state.last_user_visible_respond = Some(answer.clone());
            append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
            super::log_deterministic_delivery_record(
                &task.task_id,
                "fallback_from_config_edit_observed_marker_replace",
                "attached",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
            true
        } else {
            false
        }
    } else {
        false
    };
    record_capability_result_renderer_trace(
        task,
        loop_state,
        "config_edit_observed_answer",
        rendered,
    );
    rendered
}

fn delivery_is_replaceable_machine_marker(delivery_messages: &[String]) -> bool {
    let [message] = delivery_messages else {
        return false;
    };
    let message = message.trim();
    !message.is_empty()
        && !message.contains('=')
        && message
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
            == 1
        && message
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

pub(super) fn run_service_status_observed_fields_renderer(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let rendered = super::replace_delivery_with_service_status_observed_answer(
        task,
        loop_state,
        agent_run_context,
        finalizer_summary,
    );
    record_capability_result_renderer_trace(
        task,
        loop_state,
        "service_status_observed_fields",
        rendered,
    );
    rendered
}

fn record_capability_result_renderer_trace(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    renderer_key: &'static str,
    rendered: bool,
) {
    let Some(renderer) = super::renderer_registry::renderers_for_shape_class(
        super::renderer_registry::FinalizerRendererShapeClass::CapabilityResult,
    )
    .find(|renderer| renderer.key == renderer_key) else {
        return;
    };
    super::renderer_registry::record_renderer_trace(
        loop_state,
        renderer,
        rendered,
        capability_result_renderer_evidence_refs(task, loop_state),
        (!rendered).then_some("not_applicable"),
    );
}

fn capability_result_renderer_evidence_refs(
    task: &ClaimedTask,
    loop_state: &LoopState,
) -> Vec<String> {
    let mut refs = loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let mut reference = String::from("step_result:");
            reference.push_str(&index.to_string());
            reference
        })
        .collect::<Vec<_>>();
    if refs.is_empty() {
        refs.push(format!("task:{}", task.task_id));
    }
    refs
}
