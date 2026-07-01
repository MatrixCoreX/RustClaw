use super::PreparedAskFlow;

pub(in crate::worker) fn build_agent_run_context_from_prepared_flow(
    prompt: &str,
    prepared_flow: &PreparedAskFlow,
) -> crate::agent_engine::AgentRunContext {
    let cross_turn_recent_execution_context = {
        let trimmed = prepared_flow.recent_execution_context.trim();
        if trimmed.is_empty() || trimmed == "<none>" {
            None
        } else {
            Some(trimmed.to_string())
        }
    };

    crate::agent_engine::AgentRunContext {
        route_result: Some(prepared_flow.route_result.clone()),
        execution_recipe_hint: prepared_flow.execution_recipe_hint,
        execution_recipe_plan_hint: prepared_flow.execution_recipe_plan_hint.clone(),
        turn_analysis: prepared_flow.turn_analysis.clone(),
        context_bundle_summary: Some(prepared_flow.context_bundle_summary.clone()),
        session_alias_bindings: prepared_flow.session_alias_bindings.clone(),
        auto_locator_path: prepared_flow.auto_locator_path.clone(),
        original_user_request: Some(prompt.to_string()),
        user_request: Some(prepared_flow.resolved_prompt_for_execution.clone()),
        cross_turn_recent_execution_context,
    }
}
