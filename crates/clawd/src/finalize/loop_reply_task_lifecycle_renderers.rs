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
        rendered |= match renderer.key {
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
    }
    rendered
}
