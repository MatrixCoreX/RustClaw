use crate::agent_engine::{AgentRunContext, LoopState};
use crate::ClaimedTask;

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
