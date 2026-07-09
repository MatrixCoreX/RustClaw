use crate::agent_engine::{AgentRunContext, LoopState};
use crate::ClaimedTask;

pub(super) fn run_task_lifecycle_renderer_registry(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    delivery_messages: &mut Vec<String>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let mut rendered = false;
    for renderer in super::renderer_registry::renderers_for_shape_class(
        super::renderer_registry::FinalizerRendererShapeClass::TaskLifecycle,
    ) {
        let rendered_by_renderer = match renderer.key {
            "route_clarify_machine_envelope" => {
                super::clarify_envelope::attach_route_clarify_machine_envelope(
                    task,
                    loop_state,
                    delivery_messages,
                    finalizer_summary,
                    agent_run_context,
                )
            }
            "control_machine_envelope" => {
                super::control_envelope::attach_requested_control_machine_envelope(
                    task,
                    loop_state,
                    delivery_messages,
                    finalizer_summary,
                    agent_run_context,
                )
            }
            _ => false,
        };
        super::renderer_registry::record_renderer_trace(
            loop_state,
            renderer,
            rendered_by_renderer,
            task_lifecycle_renderer_evidence_refs(task, loop_state),
            (!rendered_by_renderer).then_some("not_applicable"),
        );
        rendered |= rendered_by_renderer;
    }
    rendered
}

fn task_lifecycle_renderer_evidence_refs(
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
