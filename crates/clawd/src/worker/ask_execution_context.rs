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
    let context_state = state.clone();
    let context_task = task.clone();
    let context_request = planner_user_request.to_string();
    let mut context_bundle = tokio::task::spawn_blocking(move || {
        crate::task_context_builder::build_agent_loop_task_context_bundle(
            &context_state,
            &context_task,
            &context_request,
            chat_memory_budget_chars,
        )
    })
    .await
    .map_err(|error| anyhow::anyhow!("task_context_build_join_failed:{error}"))?;
    if let Some(image_context) =
        crate::analyze_attached_images_for_ask(state, task, payload, planner_user_request).await?
    {
        crate::task_context_builder::set_execution_image_context(
            &mut context_bundle,
            Some(image_context),
        );
    }
    let mut initial_task_observations = Vec::new();
    if let Some(rewind) = session_rewind_observation(state, payload)? {
        initial_task_observations.push(rewind);
    }
    if let Some(task_plan_snapshot) = context_bundle.task_plan_snapshot.as_ref() {
        initial_task_observations.push(json!({
            "schema_version": 1,
            "source": crate::repo::task_plan::TASK_PLAN_SOURCE,
            "observation_kind": "task_plan_snapshot",
            "data_only": true,
            "instruction_authority": "none",
            "snapshot": task_plan_snapshot,
        }));
    }
    let provider_context_window_tokens = state
        .task_llm_providers(task)
        .iter()
        .filter_map(|provider| provider.config.context_window_tokens)
        .min();
    if let Some(mut compaction_plan) =
        crate::task_context_builder::plan_agent_loop_context_compaction_with_provider_window(
            &context_bundle,
            provider_context_window_tokens,
        )
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
    if let Some(workspace_context) =
        super::workspace_instructions::prepare_workspace_instructions(state, payload)?
    {
        if let Some(rendered) = workspace_context.rendered_context {
            resolved_prompt_for_execution.push_str("\n\n");
            resolved_prompt_for_execution.push_str(&rendered);
            prompt_with_memory_for_execution.push_str("\n\n");
            prompt_with_memory_for_execution.push_str(&rendered);
        }
        initial_task_observations.push(workspace_context.attribution);
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

fn session_rewind_observation(state: &AppState, payload: &Value) -> anyhow::Result<Option<Value>> {
    let Some(rewind) = payload.get("session_rewind") else {
        return Ok(None);
    };
    let object = rewind
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("session_rewind_schema_invalid"))?;
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "schema_version" | "anchor" | "completed_side_effect_refs"
        )
    }) || object.get("schema_version").and_then(Value::as_u64) != Some(1)
    {
        anyhow::bail!("session_rewind_schema_invalid");
    }
    let anchor = object
        .get("anchor")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("session_rewind_anchor_invalid"))?;
    if anchor.keys().any(|key| {
        !matches!(
            key.as_str(),
            "schema_version"
                | "source_session_id"
                | "source_task_id"
                | "event_seq"
                | "checkpoint_id"
        )
    }) || anchor.get("schema_version").and_then(Value::as_u64) != Some(1)
    {
        anyhow::bail!("session_rewind_anchor_invalid");
    }
    let source_task_id = bounded_machine_ref(anchor.get("source_task_id"))
        .ok_or_else(|| anyhow::anyhow!("session_rewind_source_task_invalid"))?;
    let source_session_id = bounded_machine_ref(anchor.get("source_session_id"))
        .ok_or_else(|| anyhow::anyhow!("session_rewind_source_session_invalid"))?;
    let event_seq = anchor
        .get("event_seq")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| anyhow::anyhow!("session_rewind_event_seq_invalid"))?;
    let db = state.core.db.get()?;
    let raw_result: String = db
        .query_row(
            "SELECT COALESCE(result_json, '{}') FROM tasks WHERE task_id = ?1 LIMIT 1",
            rusqlite::params![source_task_id],
            |row| row.get(0),
        )
        .map_err(|_| anyhow::anyhow!("session_rewind_source_task_missing"))?;
    let result = serde_json::from_str::<Value>(&raw_result)
        .map_err(|_| anyhow::anyhow!("session_rewind_source_result_invalid"))?;
    let events = result
        .pointer("/task_journal/trace/event_stream")
        .or_else(|| result.pointer("/result/task_journal/trace/event_stream"))
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("session_rewind_event_stream_missing"))?;
    if !events
        .iter()
        .any(|event| event.get("seq").and_then(Value::as_u64) == Some(event_seq))
    {
        anyhow::bail!("session_rewind_event_not_found");
    }
    let bounded_events = events
        .iter()
        .filter(|event| {
            event
                .get("seq")
                .and_then(Value::as_u64)
                .is_some_and(|seq| seq <= event_seq)
        })
        .rev()
        .take(64)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();
    let checkpoint_id = anchor
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let completed_side_effect_refs = authoritative_completed_side_effect_refs(&result);
    Ok(Some(json!({
        "schema_version": 1,
        "observation_kind": "session_rewind_boundary",
        "source": "clawcli_session_rewind",
        "data_only": true,
        "instruction_authority": "none",
        "source_session_id": source_session_id,
        "source_task_id": source_task_id,
        "event_seq": event_seq,
        "checkpoint_id": checkpoint_id,
        "original_history_preserved": true,
        "completed_side_effect_refs": completed_side_effect_refs,
        "side_effect_replay_policy": "already_occurred_do_not_replay",
        "event_count": bounded_events.len(),
        "events": bounded_events,
    })))
}

fn authoritative_completed_side_effect_refs(result: &Value) -> Vec<String> {
    [
        "/task_journal/summary/coding_workflow/completed_side_effect_refs",
        "/task_checkpoint/completed_side_effect_refs",
        "/result/task_journal/summary/coding_workflow/completed_side_effect_refs",
    ]
    .iter()
    .find_map(|pointer| result.pointer(pointer).and_then(Value::as_array))
    .into_iter()
    .flatten()
    .filter_map(Value::as_str)
    .filter_map(|value| bounded_machine_ref(Some(&Value::String(value.to_string()))))
    .take(256)
    .collect()
}

fn bounded_machine_ref(value: Option<&Value>) -> Option<String> {
    let value = value?.as_str()?.trim();
    (!value.is_empty()
        && value.len() <= 512
        && !value.chars().any(char::is_control)
        && !value.contains("../"))
    .then(|| value.to_string())
}

#[cfg(test)]
#[path = "ask_execution_context_tests.rs"]
mod tests;
