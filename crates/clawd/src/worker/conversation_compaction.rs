use serde_json::{json, Value};

use crate::{AppState, ClaimedTask};

const ENTRYPOINT: &str = "compact_conversation";
const MAX_REF_BYTES: usize = 128;
const MAX_COMPACTION_FOCUS_CHARS: usize = 4_000;

pub(crate) fn is_conversation_compaction_payload(payload: &Value) -> bool {
    payload.get("entrypoint").and_then(Value::as_str) == Some(ENTRYPOINT)
}

pub(crate) fn validate_conversation_compaction_payload(
    payload: &Value,
) -> Result<(), &'static str> {
    let object = payload
        .as_object()
        .ok_or("conversation_compaction_payload_object_required")?;
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "entrypoint"
                | "source"
                | "conversation_id"
                | "thread_id"
                | "session_id"
                | "resume_task_id"
                | "compaction_focus"
        )
    }) {
        return Err("conversation_compaction_additional_field_denied");
    }
    let conversation_id = machine_ref(
        object.get("conversation_id"),
        "conversation_compaction_conversation_id_invalid",
    )?;
    let thread_id = machine_ref(
        object.get("thread_id"),
        "conversation_compaction_thread_id_invalid",
    )?;
    if conversation_id != thread_id {
        return Err("conversation_compaction_thread_mismatch");
    }
    machine_ref(
        object.get("session_id"),
        "conversation_compaction_session_id_invalid",
    )?;
    if object.contains_key("resume_task_id") {
        machine_ref(
            object.get("resume_task_id"),
            "conversation_compaction_resume_task_id_invalid",
        )?;
    }
    let source = object
        .get("source")
        .and_then(Value::as_str)
        .ok_or("conversation_compaction_source_invalid")?;
    if !matches!(source, "clawcli_machine" | "ui_machine") {
        return Err("conversation_compaction_source_invalid");
    }
    validate_compaction_focus(object.get("compaction_focus"))?;
    Ok(())
}

pub(crate) async fn process_conversation_compaction_task(
    state: &AppState,
    task: &ClaimedTask,
    payload: &Value,
) -> anyhow::Result<()> {
    validate_runtime_payload(payload)?;
    crate::log_ask_transition(
        state,
        &task.task_id,
        None,
        crate::AskState::Received,
        "conversation_compaction_received",
        None,
    );
    let request = "";
    let memory_budget = crate::dynamic_chat_memory_budget_chars(state, task, request);
    let context_state = state.clone();
    let context_task = task.clone();
    let mut bundle = tokio::task::spawn_blocking(move || {
        crate::task_context_builder::build_agent_loop_task_context_bundle(
            &context_state,
            &context_task,
            request,
            memory_budget,
        )
    })
    .await
    .map_err(|error| anyhow::anyhow!("task_context_build_join_failed:{error}"))?;
    let policy = crate::task_context_builder::ContextWindowPolicy::for_task(state, task);
    let focus = payload.get("compaction_focus").and_then(Value::as_str);
    let mut plan =
        crate::task_context_builder::force_agent_loop_context_compaction_plan_with_policy(
            &bundle,
            policy.as_ref(),
            focus,
        )
        .ok_or_else(|| anyhow::anyhow!("conversation_compaction_context_unavailable"))?;
    crate::task_context_builder::hydrate_agent_loop_context_compaction_plan(state, task, &mut plan);
    let lease =
        crate::task_context_builder::context_compaction_lifecycle::begin_context_compaction(
            state, task, &mut plan,
        )?;
    let pre_compact = crate::agent_hooks::lifecycle_stage_outcome_for_state(
        state,
        &task.task_id,
        crate::agent_hooks::HookStage::PreCompact,
        "conversation.compact",
        plan.hook_metadata(),
    )
    .await;
    let (model_summary, model_status_code) =
        crate::agent_engine::run_model_assisted_context_compaction(state, task, &bundle, &plan)
            .await;
    let record = crate::task_context_builder::apply_agent_loop_context_compaction(
        state,
        task,
        request,
        memory_budget,
        &mut bundle,
        &plan,
        model_summary,
        model_status_code,
    );
    let commit =
        match crate::task_context_builder::context_compaction_lifecycle::complete_context_compaction(
            state, task, &lease, &record,
        ) {
            Ok(commit) => commit,
            Err(error) => {
                let _ = crate::task_context_builder::context_compaction_lifecycle::abandon_context_compaction(
                state,
                &lease,
            );
                return Err(error);
            }
        };
    let record = commit.record;
    if let Some(last) = bundle.compaction_records.last_mut() {
        *last = record.clone();
    }
    let post_compact = crate::agent_hooks::lifecycle_stage_outcome_for_state(
        state,
        &task.task_id,
        crate::agent_hooks::HookStage::PostCompact,
        "conversation.compact",
        json!({
            "compaction_id": record.get("compaction_id"),
            "generation": record.get("generation"),
            "before_char_count": record.get("before_char_count"),
            "after_char_count": record.get("after_char_count"),
        }),
    )
    .await;
    let observation = json!({
        "schema_version": 1,
        "observation_kind": "conversation_compaction",
        "compaction_record": record.clone(),
        "pre_compact_hooks": pre_compact.machine_observations("conversation.compact"),
        "post_compact_hooks": post_compact.machine_observations("conversation.compact"),
    });
    let result = conversation_compaction_result(payload, &record);
    finalize_machine_operation_success(state, task, "conversation_compaction", result, observation)
        .await
}

async fn finalize_machine_operation_success(
    state: &AppState,
    task: &ClaimedTask,
    operation: &str,
    machine_result: Value,
    observation: Value,
) -> anyhow::Result<()> {
    let status_token = format!("{operation}_completed");
    let mut journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", operation);
    journal.record_context_bundle_summary(format!("path={operation}"));
    journal.push_task_observation(observation);
    journal.record_runtime_llm_metrics(state, &task.task_id);
    journal.record_used_evidence_ids_count(1);
    journal.record_final_answer(&status_token);
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    let mut result = machine_result;
    if let Some(object) = result.as_object_mut() {
        object.insert("text".to_string(), json!(status_token));
    }
    let result = journal.attach_to_result(result);
    crate::repo::update_task_success(
        state,
        &task.task_id,
        task.claim_attempt,
        &result.to_string(),
    )?;
    crate::assistant_presentation::publish_terminal_answer(state, task, &status_token);
    tracing::info!(
        "task_call_end task_id={} kind=ask status=success path={}",
        task.task_id,
        operation
    );
    Ok(())
}

fn conversation_compaction_result(payload: &Value, record: &Value) -> Value {
    json!({
        "schema_version": 1,
        "status": "ok",
        "operation": "conversation.compact",
        "provenance": "task_context_builder",
        "conversation_id": payload.get("conversation_id"),
        "session_id": payload.get("session_id"),
        "compaction": record,
    })
}

fn validate_runtime_payload(payload: &Value) -> anyhow::Result<()> {
    if !is_conversation_compaction_payload(payload) {
        anyhow::bail!("conversation_compaction_entrypoint_invalid");
    }
    // Server-owned fields may be added after the public request contract is
    // validated, so runtime validates required identities independently.
    let object = payload
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("conversation_compaction_payload_object_required"))?;
    let conversation_id = machine_ref(
        object.get("conversation_id"),
        "conversation_compaction_conversation_id_invalid",
    )
    .map_err(anyhow::Error::msg)?;
    let thread_id = machine_ref(
        object.get("thread_id"),
        "conversation_compaction_thread_id_invalid",
    )
    .map_err(anyhow::Error::msg)?;
    if conversation_id != thread_id {
        anyhow::bail!("conversation_compaction_thread_mismatch");
    }
    machine_ref(
        object.get("session_id"),
        "conversation_compaction_session_id_invalid",
    )
    .map_err(anyhow::Error::msg)?;
    validate_compaction_focus(object.get("compaction_focus")).map_err(anyhow::Error::msg)?;
    Ok(())
}

fn validate_compaction_focus(value: Option<&Value>) -> Result<(), &'static str> {
    let Some(value) = value else {
        return Ok(());
    };
    let focus = value
        .as_str()
        .map(str::trim)
        .filter(|focus| !focus.is_empty())
        .ok_or("conversation_compaction_focus_invalid")?;
    if focus.chars().count() > MAX_COMPACTION_FOCUS_CHARS
        || focus
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err("conversation_compaction_focus_invalid");
    }
    Ok(())
}

fn machine_ref<'a>(
    value: Option<&'a Value>,
    error_code: &'static str,
) -> Result<&'a str, &'static str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| {
            !value.is_empty()
                && value.len() <= MAX_REF_BYTES
                && value
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':'))
        })
        .ok_or(error_code)
}

#[cfg(test)]
#[path = "conversation_compaction_tests.rs"]
mod tests;
