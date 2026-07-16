use serde_json::Value;
use tracing::info;

use crate::{AppState, ClaimedTask};

pub(super) struct PreparedAskExecutionContext {
    pub(super) context_bundle: crate::task_context_builder::TaskContextBundle,
    pub(super) resolved_prompt_for_execution: String,
    pub(super) prompt_with_memory_for_execution: String,
    pub(super) recent_execution_context: String,
}

pub(super) async fn prepare_ask_execution_context(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
    planner_user_request: &str,
) -> anyhow::Result<PreparedAskExecutionContext> {
    let chat_memory_budget_chars =
        crate::dynamic_chat_memory_budget_chars(state, task, planner_user_request);
    let mut context_bundle = crate::task_context_builder::build_agent_loop_task_context_bundle(
        state,
        task,
        planner_user_request,
        chat_memory_budget_chars,
    );
    let execution_view = context_bundle
        .execution_view
        .as_ref()
        .expect("execution_view_missing");
    let recalled_count = execution_view.memory_ctx.recalled.len();
    let mut chat_prompt_context = execution_view.memory_ctx.chat_prompt_context.clone();
    let mut resolved_prompt_for_execution = planner_user_request.to_string();
    let mut prompt_with_memory_for_execution = execution_view.memory_ctx.prompt_with_memory.clone();
    let recent_execution_context = execution_view.recent_execution_context.clone();

    if let Some(image_context) =
        crate::analyze_attached_images_for_ask(state, task, payload, planner_user_request).await?
    {
        crate::task_context_builder::set_execution_image_context(
            &mut context_bundle,
            Some(image_context),
        );
    }
    crate::task_context_builder::apply_execution_context_to_prompts(
        &context_bundle,
        &mut chat_prompt_context,
        &mut resolved_prompt_for_execution,
        &mut prompt_with_memory_for_execution,
    );
    info!(
        "ask_context_ready task_id={} recalled_recent_count={} context_summary_bytes={} recent_execution_bytes={}",
        task.task_id,
        recalled_count,
        context_bundle.summary().len(),
        recent_execution_context.len(),
    );

    Ok(PreparedAskExecutionContext {
        context_bundle,
        resolved_prompt_for_execution,
        prompt_with_memory_for_execution,
        recent_execution_context,
    })
}
