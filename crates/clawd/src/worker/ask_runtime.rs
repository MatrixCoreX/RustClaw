use anyhow::Result;
use serde_json::Value;

use super::{prepare_ask_execution_context, prepare_planner_owned_ask_routing};
use crate::{AppState, ClaimedTask};

pub(super) struct PreparedAskFlow {
    pub(super) context_bundle_summary: String,
    pub(super) memory_trace: Option<Value>,
    pub(super) route_result: crate::RouteResult,
    pub(super) turn_boundary_envelope: crate::turn_boundary_envelope::TurnBoundaryEnvelope,
    pub(super) auto_locator_path: Option<String>,
    pub(super) planner_user_request: String,
    pub(super) resolved_prompt_for_execution: String,
    pub(super) prompt_with_memory_for_execution: String,
    pub(super) recent_execution_context: String,
    pub(super) session_alias_bindings: Vec<crate::conversation_state::SessionAliasBinding>,
}

pub(super) async fn prepare_ask_flow(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    prompt: &str,
    source: &str,
) -> Result<PreparedAskFlow> {
    let prepared_routing =
        prepare_planner_owned_ask_routing(state, task, payload, prompt, source).await?;
    let prepared_execution =
        prepare_ask_execution_context(state, task, payload, &prepared_routing.planner_user_request)
            .await?;
    let session_alias_bindings =
        crate::conversation_state::load_active_session_snapshot(state, task)
            .conversation_state
            .map(|conversation_state| conversation_state.alias_bindings)
            .unwrap_or_default();

    Ok(PreparedAskFlow {
        context_bundle_summary: prepared_execution.context_bundle.summary(),
        memory_trace: prepared_execution.context_bundle.memory_trace(),
        route_result: prepared_routing.route_result,
        turn_boundary_envelope: prepared_routing.turn_boundary_envelope,
        auto_locator_path: None,
        planner_user_request: prepared_routing.planner_user_request,
        resolved_prompt_for_execution: prepared_execution.resolved_prompt_for_execution,
        prompt_with_memory_for_execution: prepared_execution.prompt_with_memory_for_execution,
        recent_execution_context: prepared_execution.recent_execution_context,
        session_alias_bindings,
    })
}

pub(super) fn build_agent_run_context_from_prepared_flow(
    prepared_flow: &PreparedAskFlow,
) -> crate::agent_engine::AgentRunContext {
    let cross_turn_recent_execution_context = match prepared_flow.recent_execution_context.trim() {
        "" | "<none>" => None,
        value => Some(value.to_string()),
    };
    crate::agent_engine::AgentRunContext {
        route_result: Some(prepared_flow.route_result.clone()),
        execution_recipe_hint: None,
        execution_recipe_plan_hint: None,
        turn_analysis: None,
        boundary_envelope: Some(prepared_flow.turn_boundary_envelope.clone()),
        context_bundle_summary: Some(prepared_flow.context_bundle_summary.clone()),
        session_alias_bindings: prepared_flow.session_alias_bindings.clone(),
        auto_locator_path: prepared_flow.auto_locator_path.clone(),
        original_user_request: Some(prepared_flow.planner_user_request.clone()),
        user_request: Some(prepared_flow.planner_user_request.clone()),
        cross_turn_recent_execution_context,
    }
}

pub(super) async fn execute_ask_dispatch(
    state: &AppState,
    task: &ClaimedTask,
    prepared_flow: &PreparedAskFlow,
) -> Result<Result<crate::AskReply, String>> {
    crate::log_ask_transition(
        state,
        &task.task_id,
        Some(crate::AskState::Routing),
        crate::AskState::Executing,
        "agent_loop_default_entry",
        None,
    );
    let agent_run_context = Some(build_agent_run_context_from_prepared_flow(prepared_flow));
    Ok(crate::agent_engine::run_agent_with_tools(
        state,
        task,
        &prepared_flow.prompt_with_memory_for_execution,
        &prepared_flow.planner_user_request,
        agent_run_context,
    )
    .await)
}

#[cfg(test)]
#[path = "ask_runtime_tests.rs"]
mod tests;
