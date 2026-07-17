use crate::agent_engine::AgentRunContext;

pub(super) fn requested_machine_kv_request_surfaces(
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> Vec<String> {
    let mut surfaces = vec![user_text.to_string()];
    let Some(ctx) = agent_run_context else {
        return surfaces;
    };
    for value in [ctx.original_user_request.as_deref()].into_iter().flatten() {
        crate::machine_kv_projection::push_unique_machine_kv_surface(&mut surfaces, value);
    }
    if let Some(selector) = ctx
        .output_contract()
        .and_then(|contract| contract.selection.structured_field_selector.as_deref())
    {
        crate::machine_kv_projection::push_unique_machine_kv_surface(&mut surfaces, selector);
    }
    if let Some(state_patch) = ctx
        .turn_analysis
        .as_ref()
        .and_then(|analysis| analysis.state_patch.as_ref())
    {
        crate::machine_kv_projection::collect_requested_machine_kv_surfaces_from_state_patch(
            state_patch,
            &mut surfaces,
        );
    }
    surfaces
}
