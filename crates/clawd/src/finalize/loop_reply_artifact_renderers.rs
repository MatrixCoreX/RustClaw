use crate::agent_engine::{AgentRunContext, LoopState};
use crate::AppState;

pub(super) fn normalize_file_token_delivery_from_observed_paths(
    state: &AppState,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let before_last = loop_state.last_user_visible_respond.clone();
    let before_messages = loop_state.delivery_messages.clone();
    super::file_delivery::normalize_file_token_delivery_from_observed_paths(
        state,
        loop_state,
        agent_run_context,
    );
    let rendered = before_last != loop_state.last_user_visible_respond
        || before_messages != loop_state.delivery_messages;
    record_artifact_delivery_renderer_trace(loop_state, "file_token_delivery", rendered);
    rendered
}

fn record_artifact_delivery_renderer_trace(
    loop_state: &mut LoopState,
    renderer_key: &'static str,
    rendered: bool,
) {
    let Some(renderer) = super::renderer_registry::renderers_for_shape_class(
        super::renderer_registry::FinalizerRendererShapeClass::ArtifactDelivery,
    )
    .find(|renderer| renderer.key == renderer_key) else {
        return;
    };
    super::renderer_registry::record_renderer_trace(
        loop_state,
        renderer,
        rendered,
        artifact_delivery_renderer_evidence_refs(loop_state),
        (!rendered).then_some("not_applicable"),
    );
}

fn artifact_delivery_renderer_evidence_refs(loop_state: &LoopState) -> Vec<String> {
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
        refs.push("delivery_messages".to_string());
    }
    refs
}
