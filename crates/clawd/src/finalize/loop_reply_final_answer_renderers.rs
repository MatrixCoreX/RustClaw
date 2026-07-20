use crate::agent_engine::{AgentRunContext, LoopState};
use crate::{AppState, ClaimedTask};

pub(super) fn replace_delivery_with_matrix_observed_shape_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &mut Vec<String>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let rendered = super::matrix_shape::replace_delivery_with_matrix_observed_shape_answer(
        state,
        task,
        user_text,
        loop_state,
        agent_run_context,
        delivery_messages,
        finalizer_summary,
    );
    record_final_answer_renderer_trace(task, loop_state, "matrix_observed_shape", rendered);
    rendered
}

pub(super) fn replace_delivery_with_requested_machine_kv_summary(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
    delivery_messages: &mut Vec<String>,
) -> bool {
    let rendered = super::machine_kv::replace_delivery_with_requested_machine_kv_summary(
        task,
        user_text,
        loop_state,
        agent_run_context,
        finalizer_summary,
        delivery_messages,
    );
    record_final_answer_renderer_trace(task, loop_state, "machine_kv_summary", rendered);
    rendered
}

pub(super) fn replace_final_delivery_with_exact_observation_machine_field_projection(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
    delivery_messages: &mut Vec<String>,
) -> bool {
    let rendered =
        super::exact_observation::replace_final_delivery_with_exact_observation_machine_field_projection(
            state,
            task,
            loop_state,
            agent_run_context,
            finalizer_summary,
            delivery_messages,
        );
    record_final_answer_renderer_trace(
        task,
        loop_state,
        "exact_observation_machine_field_projection",
        rendered,
    );
    rendered
}

fn record_final_answer_renderer_trace(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    renderer_key: &'static str,
    rendered: bool,
) {
    let Some(renderer) = super::renderer_registry::renderers_for_shape_class(
        super::renderer_registry::FinalizerRendererShapeClass::FinalAnswerShape,
    )
    .find(|renderer| renderer.key == renderer_key) else {
        return;
    };
    super::renderer_registry::record_renderer_trace(
        loop_state,
        renderer,
        rendered,
        final_answer_renderer_evidence_refs(task, loop_state),
        (!rendered).then_some("not_applicable"),
    );
}

fn final_answer_renderer_evidence_refs(task: &ClaimedTask, loop_state: &LoopState) -> Vec<String> {
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
