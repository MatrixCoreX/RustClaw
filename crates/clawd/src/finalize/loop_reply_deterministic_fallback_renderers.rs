use crate::agent_engine::{AgentRunContext, LoopState};
use crate::{AppState, ClaimedTask};

pub(super) fn run_deterministic_fallback_renderer_registry(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let mut rendered = false;
    for renderer in super::renderer_registry::renderers_for_shape_class(
        super::renderer_registry::FinalizerRendererShapeClass::DeterministicFallback,
    ) {
        let rendered_by_renderer = match renderer.key {
            "scalar_placeholder_terminal_direct_answer" => {
                super::scalar_placeholder::replace_scalar_placeholder_delivery_with_direct_scalar_answer(
                    state,
                    task,
                    loop_state,
                    agent_run_context,
                    finalizer_summary,
                )
            }
            _ => false,
        };
        super::renderer_registry::record_renderer_trace(
            loop_state,
            renderer,
            rendered_by_renderer,
            deterministic_fallback_renderer_evidence_refs(task, loop_state),
            (!rendered_by_renderer).then_some("not_applicable"),
        );
        rendered |= rendered_by_renderer;
    }
    rendered
}

fn deterministic_fallback_renderer_evidence_refs(
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
