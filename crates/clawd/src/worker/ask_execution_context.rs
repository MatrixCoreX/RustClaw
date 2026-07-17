use serde_json::{json, Value};
use tracing::info;

use crate::{AppState, ClaimedTask};

pub(super) struct PreparedAskExecutionContext {
    pub(super) context_bundle: crate::task_context_builder::TaskContextBundle,
    pub(super) resolved_prompt_for_execution: String,
    pub(super) prompt_with_memory_for_execution: String,
    pub(super) recent_execution_context: String,
    pub(super) initial_task_observations: Vec<Value>,
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
    if let Some(image_context) =
        crate::analyze_attached_images_for_ask(state, task, payload, planner_user_request).await?
    {
        crate::task_context_builder::set_execution_image_context(
            &mut context_bundle,
            Some(image_context),
        );
    }
    let mut initial_task_observations = Vec::new();
    if let Some(mut compaction_plan) =
        crate::task_context_builder::plan_agent_loop_context_compaction(&context_bundle)
    {
        crate::task_context_builder::hydrate_agent_loop_context_compaction_plan(
            state,
            task,
            &mut compaction_plan,
        );
        let pre_compact = crate::agent_hooks::lifecycle_stage_outcome_for_state(
            state,
            &task.task_id,
            crate::agent_hooks::HookStage::PreCompact,
            "agent_loop.context_compaction",
            compaction_plan.hook_metadata(),
        )
        .await;
        initial_task_observations
            .extend(pre_compact.machine_observations("agent_loop.context_compaction"));
        let (model_summary, model_status_code) =
            crate::agent_engine::run_model_assisted_context_compaction(
                state,
                task,
                &context_bundle,
                &compaction_plan,
            )
            .await;
        let compaction_record = crate::task_context_builder::apply_agent_loop_context_compaction(
            state,
            task,
            planner_user_request,
            chat_memory_budget_chars,
            &mut context_bundle,
            &compaction_plan,
            model_summary,
            model_status_code,
        );
        initial_task_observations.push(crate::task_journal::context_compaction_record_observation(
            compaction_record.clone(),
        ));
        let post_compact = crate::agent_hooks::lifecycle_stage_outcome_for_state(
            state,
            &task.task_id,
            crate::agent_hooks::HookStage::PostCompact,
            "agent_loop.context_compaction",
            json!({
                "compaction_kind": "deterministic_context_budget",
                "generation": compaction_record.get("generation"),
                "compaction_id": compaction_record.get("compaction_id"),
                "before_char_count": compaction_record.get("before_char_count"),
                "after_char_count": compaction_record.get("after_char_count"),
                "model_status_code": compaction_record.get("model_status_code"),
                "model_summary_attached": compaction_record.get("model_summary_attached"),
                "source_ref_count": compaction_record
                    .get("source_refs")
                    .and_then(Value::as_array)
                    .map(Vec::len),
                "retained_ref_count": compaction_record
                    .get("retained_refs")
                    .and_then(Value::as_array)
                    .map(Vec::len),
            }),
        )
        .await;
        initial_task_observations
            .extend(post_compact.machine_observations("agent_loop.context_compaction"));
    }
    let execution_view = context_bundle
        .execution_view
        .as_ref()
        .expect("execution_view_missing");
    let recalled_count = execution_view.memory_ctx.recalled.len();
    let mut chat_prompt_context = execution_view.memory_ctx.chat_prompt_context.clone();
    let mut resolved_prompt_for_execution = planner_user_request.to_string();
    let mut prompt_with_memory_for_execution = execution_view.memory_ctx.prompt_with_memory.clone();
    let recent_execution_context = execution_view.recent_execution_context.clone();
    let context_prompt_attribution =
        crate::task_context_builder::apply_execution_context_to_prompts(
            state,
            &context_bundle,
            &mut chat_prompt_context,
            &mut resolved_prompt_for_execution,
            &mut prompt_with_memory_for_execution,
        )?;
    if !context_prompt_attribution.is_empty() {
        initial_task_observations.push(json!({
            "schema_version": 1,
            "observation_kind": "context_prompt_attribution",
            "prompt_count": context_prompt_attribution.len(),
            "template_char_count": context_prompt_attribution
                .iter()
                .filter_map(|item| item.get("template_char_count").and_then(Value::as_u64))
                .sum::<u64>(),
            "rendered_char_count": context_prompt_attribution
                .iter()
                .filter_map(|item| item.get("rendered_char_count").and_then(Value::as_u64))
                .sum::<u64>(),
            "prompts": context_prompt_attribution,
        }));
    }
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
        initial_task_observations,
    })
}

#[cfg(test)]
#[path = "ask_execution_context_tests.rs"]
mod tests;
