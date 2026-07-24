use crate::agent_engine::AgentRunContext;
use crate::{AppState, ClaimedTask};

use super::{looks_like_structured_machine_output, message_is_non_answer_separator};

pub(super) fn final_reply_language_hint(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> String {
    let mut candidates = Vec::new();
    if let Some(ctx) = agent_run_context {
        if let Some(original) = ctx.original_user_request.as_deref() {
            candidates.push(original);
        }
        if let Some(request) = ctx.user_request.as_deref() {
            candidates.push(request);
        }
    }
    candidates.push(user_text);
    if let Some(hint) = crate::language_policy::first_clear_request_language_hint(candidates) {
        return hint;
    }
    crate::language_policy::task_response_language_hint(state, task, user_text)
}

pub(super) fn route_resolved_intent(agent_run_context: Option<&AgentRunContext>) -> String {
    agent_run_context
        .and_then(|ctx| {
            ctx.original_user_request
                .as_deref()
                .or(ctx.user_request.as_deref())
        })
        .map(str::trim)
        .filter(|intent| !intent.is_empty())
        .unwrap_or_default()
        .to_string()
}

pub(super) async fn execution_recipe_budget_exhausted_message(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> String {
    let repair_count = loop_state.execution_recipe.repair_count.to_string();
    let max_repairs = loop_state.execution_recipe.max_repairs.to_string();
    let language_hint = final_reply_language_hint(state, task, user_text, agent_run_context);
    let contract = crate::fallback::UserResponseContract::tool_failure(
        "execution_recipe_repair_budget_exhausted",
        user_text,
        &route_resolved_intent(agent_run_context),
        vec![
            "closed_loop_stage: inspect/apply/validate".to_string(),
            format!("repair_count: {repair_count}"),
            format!("max_repairs: {max_repairs}"),
            "result_validated: false".to_string(),
        ],
        vec![
            "success_allowed=false".to_string(),
            "validation_status=failed".to_string(),
            "continue_with_different_approach_available=true".to_string(),
            "additional_context_may_unblock=true".to_string(),
        ],
        "brief_failure_with_next_step",
        &language_hint,
    );
    crate::fallback::compose_user_response_from_contract(
        state,
        task,
        &contract,
        crate::fallback::ClarifyFallbackSource::ExecutionFailedPartial,
    )
    .await
}

pub(super) fn route_allows_model_language_final_answer(
    route: &crate::IntentOutputContract,
) -> bool {
    crate::evidence_policy::final_answer_shape_for_output_contract(route)
        .is_some_and(|shape| shape.allows_model_language())
}

pub(crate) fn planned_delivery_is_publishable_model_language_answer(delivery: &str) -> bool {
    let delivery = delivery.trim();
    !delivery.is_empty()
        && crate::finalize::parse_delivery_token(delivery).is_none()
        && !crate::finalize::looks_like_planner_artifact(delivery)
        && !crate::finalize::looks_like_internal_trace_artifact(delivery)
        && !looks_like_structured_machine_output(delivery)
        && !message_is_non_answer_separator(delivery)
}
