use anyhow::Result;
use serde_json::{json, Value};
use tracing::{error, info, warn};

use crate::{repo, AppState};

fn ask_result_payload(
    answer_text: &str,
    answer_messages: &[String],
    journal: Option<&crate::task_journal::TaskJournal>,
) -> Value {
    let base_result = if answer_messages.is_empty() {
        json!({ "text": answer_text })
    } else {
        json!({ "text": answer_text, "messages": answer_messages })
    };
    match journal {
        Some(journal) => journal.attach_to_result(base_result),
        None => base_result,
    }
}

fn should_skip_ask_memory_pair(
    state: &AppState,
    answer_text: &str,
    answer_messages: &[String],
) -> bool {
    // §7.2: 集合化比对 —— 旧 super-fallback / 新 7 个 source 文案任一命中都算
    // "fallback 占位符"，跳过不写入 ask 记忆对。
    if crate::fallback::is_known_clarify_fallback_text(state, answer_text) {
        return true;
    }
    answer_messages
        .iter()
        .filter(|message| !crate::finalize::is_execution_summary_message(message))
        .any(|message| crate::fallback::is_known_clarify_fallback_text(state, message))
}

fn non_failure_final_status(semantic_clarify: bool) -> crate::task_journal::TaskJournalFinalStatus {
    if semantic_clarify {
        crate::task_journal::TaskJournalFinalStatus::Clarify
    } else {
        crate::task_journal::TaskJournalFinalStatus::Success
    }
}

fn assistant_memory_source_text(answer_text: &str, answer_messages: &[String]) -> String {
    let publishable_messages = answer_messages
        .iter()
        .map(|message| message.trim())
        .filter(|message| !message.is_empty())
        .filter(|message| !crate::finalize::is_execution_summary_message(message))
        .collect::<Vec<_>>();
    if publishable_messages.is_empty() {
        let answer = answer_text.trim();
        if crate::finalize::is_execution_summary_message(answer) {
            String::new()
        } else {
            answer.to_string()
        }
    } else {
        publishable_messages.join("\n")
    }
}

fn should_reinsert_execution_summaries_for_delivery(
    route_result: &crate::RouteResult,
    answer_text: &str,
) -> bool {
    let output_contract = &route_result.output_contract;
    if (output_contract.response_shape == crate::OutputResponseShape::Scalar
        || output_contract.semantic_kind == crate::OutputSemanticKind::ConfigValidation)
        && !answer_text.trim().is_empty()
        && !crate::finalize::is_execution_summary_message(answer_text)
    {
        return false;
    }
    true
}

fn drop_execution_summaries_when_delivery_is_scalar(
    route_result: &crate::RouteResult,
    answer_text: &str,
    answer_messages: &mut Vec<String>,
) {
    if should_reinsert_execution_summaries_for_delivery(route_result, answer_text) {
        return;
    }
    answer_messages.retain(|message| !crate::finalize::is_execution_summary_message(message));
}

fn journal_has_missing_file_search_evidence(
    journal: Option<&crate::task_journal::TaskJournal>,
) -> bool {
    journal
        .into_iter()
        .flat_map(|journal| journal.step_results.iter().rev())
        .any(|step| {
            step.output_excerpt
                .as_deref()
                .is_some_and(crate::finalize::loop_reply::output_excerpt_has_missing_file_evidence)
                || step
                    .error_excerpt
                    .as_deref()
                    .is_some_and(text_looks_like_missing_file_target)
        })
}

fn has_any_delivery_file_token(text: &str, messages: &[String]) -> bool {
    !crate::extract_delivery_file_tokens(text).is_empty()
        || messages
            .iter()
            .any(|message| !crate::extract_delivery_file_tokens(message).is_empty())
}

fn route_has_file_delivery_contract(route_result: &crate::RouteResult) -> bool {
    route_result.wants_file_delivery
        || route_result.output_contract.delivery_required
        || matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
}

fn should_use_missing_file_delivery_reply(
    route_result: &crate::RouteResult,
    answer: &crate::AskReply,
) -> bool {
    route_has_file_delivery_contract(route_result)
        && !answer.should_fail_task
        && !has_any_delivery_file_token(&answer.text, &answer.messages)
        && journal_has_missing_file_search_evidence(answer.task_journal.as_ref())
}

fn resume_context_body(value: &Value) -> &Value {
    value.get("resume_context").unwrap_or(value)
}

fn text_looks_like_missing_file_target(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.starts_with("__RC_READ_FILE_NOT_FOUND__:")
        || crate::skills::parse_structured_skill_error(trimmed)
            .is_some_and(|structured| structured.error_kind == "not_found")
}

fn resume_context_has_remaining_actions(resume_ctx: &Value) -> bool {
    resume_context_body(resume_ctx)
        .get("remaining_actions")
        .and_then(|value| value.as_array())
        .is_some_and(|actions| !actions.is_empty())
}

fn resume_context_failed_step_texts<'a>(resume_ctx: &'a Value) -> Vec<&'a str> {
    let body = resume_context_body(resume_ctx);
    let mut texts = Vec::new();
    if let Some(error) = body
        .get("failed_step")
        .and_then(|step| step.get("error"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        texts.push(error);
    }
    if let Some(messages) = body
        .get("completed_messages")
        .and_then(|value| value.as_array())
    {
        texts.extend(
            messages
                .iter()
                .filter_map(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty()),
        );
    }
    texts
}

fn resume_failure_is_missing_file_delivery_result(
    route_result: &crate::RouteResult,
    user_error: &str,
    resume_ctx: &Value,
) -> bool {
    route_has_file_delivery_contract(route_result)
        && !resume_context_has_remaining_actions(resume_ctx)
        && (text_looks_like_missing_file_target(user_error)
            || resume_context_failed_step_texts(resume_ctx)
                .iter()
                .any(|text| text_looks_like_missing_file_target(text)))
}

fn resume_context_failed_structured_skill_error(
    resume_ctx: &Value,
) -> Option<crate::skills::StructuredSkillError> {
    resume_context_body(resume_ctx)
        .get("failed_step")
        .and_then(|step| {
            step.get("structured_error")
                .and_then(resume_context_structured_skill_error_from_value)
                .or_else(|| {
                    step.get("error")
                        .and_then(|value| value.as_str())
                        .and_then(crate::skills::parse_structured_skill_error)
                })
        })
}

fn resume_context_string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn resume_context_structured_skill_error_from_value(
    value: &Value,
) -> Option<crate::skills::StructuredSkillError> {
    Some(crate::skills::StructuredSkillError {
        skill: resume_context_string_field(value, "skill")?,
        error_kind: resume_context_string_field(value, "error_kind")?,
        error_text: resume_context_string_field(value, "error_text")?,
        platform: resume_context_string_field(value, "platform"),
        manager_type: resume_context_string_field(value, "manager_type"),
        service_name: resume_context_string_field(value, "service_name"),
        extra: value.get("extra").cloned().filter(|value| !value.is_null()),
    })
}

fn structured_service_status_error_is_answerable(
    error: &crate::skills::StructuredSkillError,
) -> bool {
    error.skill == "service_control"
        && matches!(
            error.error_kind.as_str(),
            "not_found" | "service_inactive" | "service_failed" | "service_control_failed"
        )
}

fn resume_failure_is_structured_service_status_result(
    route_result: &crate::RouteResult,
    resume_ctx: &Value,
) -> bool {
    route_result.output_contract.semantic_kind == crate::OutputSemanticKind::ServiceStatus
        && !resume_context_has_remaining_actions(resume_ctx)
        && resume_context_failed_structured_skill_error(resume_ctx)
            .as_ref()
            .is_some_and(structured_service_status_error_is_answerable)
}

fn resume_context_extra_string<'a>(
    error: &'a crate::skills::StructuredSkillError,
    key: &str,
) -> Option<&'a str> {
    error
        .extra
        .as_ref()
        .and_then(|extra| extra.get(key))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn resume_context_extra_i64(error: &crate::skills::StructuredSkillError, key: &str) -> Option<i64> {
    error
        .extra
        .as_ref()
        .and_then(|extra| extra.get(key))
        .and_then(|value| value.as_i64())
}

fn compact_resume_error_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn resume_failure_execution_failed_step_answer(
    route_result: &crate::RouteResult,
    resume_ctx: &Value,
    prefer_english: bool,
) -> Option<String> {
    if route_result.output_contract.semantic_kind != crate::OutputSemanticKind::ExecutionFailedStep
    {
        return None;
    }
    let body = resume_context_body(resume_ctx);
    let failed_step = body.get("failed_step")?;
    let action = failed_step
        .get("action")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("step");
    let raw_error = failed_step
        .get("error")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let structured = resume_context_failed_structured_skill_error(resume_ctx);
    let command = structured
        .as_ref()
        .and_then(|error| resume_context_extra_string(error, "command"));
    let exit_code = structured
        .as_ref()
        .and_then(|error| resume_context_extra_i64(error, "exit_code"));
    let detail = structured
        .as_ref()
        .and_then(|error| resume_context_extra_string(error, "stderr"))
        .or_else(|| structured.as_ref().map(|error| error.error_text.trim()))
        .or(raw_error)
        .map(compact_resume_error_text)
        .filter(|value| !value.is_empty())?;

    if prefer_english {
        let subject = command
            .map(|command| format!("Command `{command}`"))
            .unwrap_or_else(|| format!("Step `{action}`"));
        if let Some(exit_code) = exit_code {
            Some(format!(
                "{subject} failed with exit code {exit_code}: {detail}"
            ))
        } else {
            Some(format!("{subject} failed: {detail}"))
        }
    } else {
        let subject = command
            .map(|command| format!("命令 `{command}`"))
            .unwrap_or_else(|| format!("步骤 `{action}`"));
        if let Some(exit_code) = exit_code {
            Some(format!("{subject}执行失败，退出码为 {exit_code}：{detail}"))
        } else {
            Some(format!("{subject}执行失败：{detail}"))
        }
    }
}

fn resume_context_user_visible_step_error(error: &str) -> String {
    crate::skills::parse_structured_skill_error(error)
        .map(|structured| crate::skills::normalize_skill_error_for_user(&structured.skill, error))
        .unwrap_or_else(|| error.to_string())
}

fn resume_context_execution_summary_messages(
    resume_ctx: &Value,
    prefer_english: bool,
) -> Vec<String> {
    let body = resume_context_body(resume_ctx);
    let Some(failed_step) = body.get("failed_step") else {
        return Vec::new();
    };
    let action = failed_step
        .get("action")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("step");
    let error = failed_step
        .get("error")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Execution failed.");
    let error = resume_context_user_visible_step_error(error);
    let prefix = if prefer_english {
        crate::finalize::EXECUTION_SUMMARY_MESSAGE_PREFIX_EN
    } else {
        crate::finalize::EXECUTION_SUMMARY_MESSAGE_PREFIX
    };
    let label = if prefer_english { "Error" } else { "错误" };
    let line = if prefer_english {
        format!("1. Called `{action}`")
    } else {
        format!("1. 调用 `{action}`")
    };
    vec![format!(
        "{prefix}\n{line}\n   {label}：\n```text\n{}\n```",
        crate::truncate_for_agent_trace(&error).replace("```", "'''")
    )]
}

async fn missing_file_delivery_reply_text(
    state: &AppState,
    task: &crate::ClaimedTask,
    prompt: &str,
    route_result: &crate::RouteResult,
    answer: &crate::AskReply,
) -> Option<String> {
    if !should_use_missing_file_delivery_reply(route_result, answer) {
        return None;
    }
    let language_hint = crate::language_policy::task_response_language_hint(state, task, prompt);
    Some(
        crate::fallback::compose_missing_file_delivery_response(
            state,
            task,
            prompt,
            &route_result.resolved_intent,
            Some(route_result.output_contract.locator_hint.as_str()),
            &language_hint,
        )
        .await,
    )
}

fn spawn_memory_intent_llm_extraction(state: &AppState, task: &crate::ClaimedTask, prompt: &str) {
    let state = state.clone();
    let mut task = task.clone();
    let parent_task_id = task.task_id.clone();
    task.task_id = format!("{parent_task_id}:memory_intent");
    let metrics_task_id = task.task_id.clone();
    let prompt = prompt.to_string();
    tokio::spawn(async move {
        if let Err(err) =
            crate::memory::maybe_extract_memory_intent_with_llm(&state, &task, &prompt).await
        {
            warn!(
                "memory intent llm extraction failed task_id={} parent_task_id={} err={}",
                task.task_id, parent_task_id, err
            );
        }
        state.clear_task_llm_call_count(&metrics_task_id);
    });
}

fn insert_ask_memory_pair(
    state: &AppState,
    task: &crate::ClaimedTask,
    prompt: &str,
    answer_text: &str,
    answer_messages: &[String],
    is_llm_reply: bool,
    agent_display_name_hint: &str,
) {
    let _ = crate::memory::upsert_user_preferences_from_route_hint(
        state,
        task.user_id,
        task.chat_id,
        task.user_key.as_deref(),
        agent_display_name_hint,
    );
    spawn_memory_intent_llm_extraction(state, task, prompt);
    if should_skip_ask_memory_pair(state, answer_text, answer_messages) {
        return;
    }
    let _ = crate::memory::service::insert_memory(
        state,
        task.user_id,
        task.chat_id,
        task.user_key.as_deref(),
        &task.channel,
        task.external_chat_id.as_deref(),
        crate::memory::MEMORY_ROLE_USER,
        prompt,
        state.policy.memory.item_max_chars.max(256),
    );
    let assistant_source_text = assistant_memory_source_text(answer_text, answer_messages);
    if assistant_source_text.trim().is_empty() {
        return;
    }
    let assistant_memory_text = if is_llm_reply && state.policy.memory.mark_llm_reply_in_short_term
    {
        format!(
            "{}{}",
            crate::memory::LLM_SHORT_TERM_MEMORY_PREFIX,
            assistant_source_text
        )
    } else {
        assistant_source_text
    };
    let _ = crate::memory::service::insert_memory_with_kind(
        state,
        task.user_id,
        task.chat_id,
        task.user_key.as_deref(),
        &task.channel,
        task.external_chat_id.as_deref(),
        crate::memory::MEMORY_ROLE_ASSISTANT,
        &assistant_memory_text,
        state.policy.memory.item_max_chars.max(256),
        crate::memory::MemoryWriteKind::AssistantOutcome,
    );
}

fn build_unfinished_goal_memory_text(prompt: &str, blocker: &str) -> String {
    format!(
        "Unfinished goal\nUser request: {}\nCurrent blocker: {}",
        prompt.trim(),
        blocker.trim()
    )
}

fn insert_unfinished_goal_memory(
    state: &AppState,
    task: &crate::ClaimedTask,
    prompt: &str,
    blocker: &str,
) {
    let text = build_unfinished_goal_memory_text(prompt, blocker);
    let _ = crate::memory::service::insert_memory_with_kind(
        state,
        task.user_id,
        task.chat_id,
        task.user_key.as_deref(),
        &task.channel,
        task.external_chat_id.as_deref(),
        crate::memory::MEMORY_ROLE_SYSTEM,
        &text,
        state.policy.memory.item_max_chars.max(256),
        crate::memory::MemoryWriteKind::UnfinishedGoal,
    );
}

async fn finalize_ask_success(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    prompt: &str,
    answer_text: &str,
    answer_messages: &[String],
    is_llm_reply: bool,
    should_refresh_long_term_memory: bool,
    agent_display_name_hint: &str,
    journal: &mut crate::task_journal::TaskJournal,
) -> Result<()> {
    let notify_outcome =
        crate::worker::maybe_notify_schedule_result(state, task, payload, true, answer_text).await;
    crate::worker::record_schedule_notify_outcome(journal, notify_outcome);
    let result = ask_result_payload(answer_text, answer_messages, Some(journal));
    repo::update_task_success(state, &task.task_id, &result.to_string())?;
    insert_ask_memory_pair(
        state,
        task,
        prompt,
        answer_text,
        answer_messages,
        is_llm_reply,
        agent_display_name_hint,
    );
    crate::worker::spawn_long_term_summary_refresh(
        state.clone(),
        task.clone(),
        should_refresh_long_term_memory,
    );
    info!("{}", crate::LOG_CALL_WRAP);
    info!(
        "task_call_end task_id={} kind=ask status=success path=normal result={}",
        task.task_id,
        crate::truncate_for_log(answer_text)
    );
    info!("{}", crate::LOG_CALL_WRAP);
    Ok(())
}

async fn finalize_ask_resume_failure(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    user_error: &str,
    resume_payload: Value,
    answer_messages: &[String],
    journal: &mut crate::task_journal::TaskJournal,
) -> Result<()> {
    journal.record_final_failure_attribution_from_error(user_error);
    let notify_outcome =
        crate::worker::maybe_notify_schedule_result(state, task, payload, false, user_error).await;
    crate::worker::record_schedule_notify_outcome(journal, notify_outcome);
    let mut result = ask_result_payload(user_error, answer_messages, None);
    if let Some(obj) = result.as_object_mut() {
        obj.insert("resume_context".to_string(), resume_payload);
    }
    let result = journal.attach_to_result(result);
    repo::update_task_failure_with_result(state, &task.task_id, &result.to_string(), user_error)?;
    info!("{}", crate::LOG_CALL_WRAP);
    info!(
        "task_call_end task_id={} kind=ask status=failed path=normal error={} resume_context=true",
        task.task_id,
        crate::truncate_for_log(user_error)
    );
    info!("{}", crate::LOG_CALL_WRAP);
    Ok(())
}

async fn finalize_ask_failure(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    answer_text: &str,
    answer_messages: &[String],
    err_text: &str,
    journal: &mut crate::task_journal::TaskJournal,
) -> Result<()> {
    error!(
        "worker_once: ask task_id={} failed: {}",
        task.task_id, err_text
    );
    journal.record_final_failure_attribution_from_error(err_text);
    let notify_outcome =
        crate::worker::maybe_notify_schedule_result(state, task, payload, false, answer_text).await;
    crate::worker::record_schedule_notify_outcome(journal, notify_outcome);
    let result = journal.attach_to_result(ask_result_payload(answer_text, answer_messages, None));
    repo::update_task_failure_with_result(state, &task.task_id, &result.to_string(), err_text)?;
    info!("{}", crate::LOG_CALL_WRAP);
    info!(
        "task_call_end task_id={} kind=ask status=failed path=normal error={}",
        task.task_id,
        crate::truncate_for_log(err_text)
    );
    info!("{}", crate::LOG_CALL_WRAP);
    Ok(())
}

async fn compose_ask_failure_user_message(
    state: &AppState,
    task: &crate::ClaimedTask,
    user_request: &str,
    err_text: &str,
) -> String {
    let language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_request);
    let prefer_english = language_hint.to_ascii_lowercase().starts_with("en");
    let fallback_text = if prefer_english {
        "The task could not be completed, and no reliable user-facing result was produced. Please adjust the request or retry later."
    } else {
        "这次任务没有完成，也没有形成可靠的可交付结果。请调整请求或稍后重试。"
    };
    let mut observed_facts = Vec::new();
    let err = err_text.trim();
    if !err.is_empty() {
        observed_facts.push(format!(
            "error_summary: {}",
            crate::truncate_for_agent_trace(err)
        ));
    }
    let contract = crate::fallback::UserResponseContract::tool_failure(
        "ask_runtime_failure",
        user_request,
        user_request,
        observed_facts,
        vec![
            "Do not expose raw provider errors, prompt names, schema names, stack traces, or internal planner action names.".to_string(),
            "Do not claim the task succeeded or that any unobserved action was completed.".to_string(),
            "Give one concise recovery path the user can act on.".to_string(),
        ],
        "brief_failure_with_next_step",
        &language_hint,
    );
    crate::fallback::compose_user_response_from_contract_with_default(
        state,
        task,
        &contract,
        crate::fallback::ClarifyFallbackSource::ExecutionFailedPartial,
        fallback_text,
    )
    .await
}

pub(crate) async fn finalize_ask_direct_success(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    prompt: &str,
    answer_text: &str,
    path_label: &str,
    should_refresh_long_term_memory: bool,
    agent_display_name_hint: &str,
) -> Result<()> {
    let mut journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", prompt);
    journal.record_context_bundle_summary(format!("path={path_label}"));
    journal.record_llm_calls_per_task(state.task_llm_call_count(&task.task_id));
    journal.record_llm_elapsed_ms_per_task(state.task_llm_elapsed_ms(&task.task_id));
    journal.record_llm_by_prompt(state.task_llm_by_prompt(&task.task_id));
    journal.record_used_evidence_ids_count(0);
    journal.record_delivery_consistent(crate::task_journal::delivery_payload_consistent(
        answer_text,
        &[],
    ));
    journal.record_final_answer(answer_text);
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    let notify_outcome =
        crate::worker::maybe_notify_schedule_result(state, task, payload, true, answer_text).await;
    crate::worker::record_schedule_notify_outcome(&mut journal, notify_outcome);
    let result = journal.attach_to_result(json!({ "text": answer_text }));
    repo::update_task_success(state, &task.task_id, &result.to_string())?;
    insert_ask_memory_pair(
        state,
        task,
        prompt,
        answer_text,
        &[],
        false,
        agent_display_name_hint,
    );
    crate::worker::spawn_long_term_summary_refresh(
        state.clone(),
        task.clone(),
        should_refresh_long_term_memory,
    );
    info!("{}", crate::LOG_CALL_WRAP);
    info!(
        "task_call_end task_id={} kind=ask status=success path={} result={}",
        task.task_id,
        path_label,
        crate::truncate_for_log(answer_text)
    );
    info!("{}", crate::LOG_CALL_WRAP);
    state.clear_task_llm_call_count(&task.task_id);
    Ok(())
}

pub(crate) async fn run_direct_classifier_reply(
    state: &AppState,
    task: &crate::ClaimedTask,
    resolved_prompt_for_execution: &str,
) -> Result<crate::AskReply, String> {
    const DIRECT_CLASSIFIER_PROMPT_LABEL: &str = "inline:direct_classifier";
    let request_language_hint = crate::language_policy::task_response_language_hint(
        state,
        task,
        resolved_prompt_for_execution,
    );
    let prompt = format!(
        "You are producing the final user-facing reply directly.\n\nRequest language hint: {request_language_hint}\nConfigured fallback language: {}\n\nLanguage policy (strict): follow the Request language hint when it is clear. Clear hints include `zh-CN`, `en`, `mixed`, BCP-47 style language tags such as `ja`/`ko`/`fr-FR`, and script hints such as `und-Latn`/`und-Cyrl`/`und-Arab`. Use the configured fallback language only when the hint is `config_default` or otherwise unclear. If the hint is `en` but the current request is clearly another Latin-script human language, follow the current request language. Do not switch languages just because names, paths, commands, code, or other normalized values are in English.\n\nReturn only the user-facing reply.\n\n{}",
        state.policy.command_intent.default_locale,
        resolved_prompt_for_execution
    );
    crate::log_prompt_render(
        state,
        &task.task_id,
        "direct_classifier",
        DIRECT_CLASSIFIER_PROMPT_LABEL,
        None,
    );
    crate::llm_gateway::run_with_fallback_with_prompt_source(
        state,
        task,
        &prompt,
        DIRECT_CLASSIFIER_PROMPT_LABEL,
    )
    .await
    .map(|s| crate::AskReply::llm(s.trim().to_string()))
    .map_err(|e| e.to_string())
}

pub(crate) async fn try_finalize_schedule_direct_success(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    prompt: &str,
    resolved_prompt_for_execution: &str,
    route_result: &crate::RouteResult,
) -> Result<bool> {
    if let Ok(Some(schedule_reply)) = crate::intent_router::try_handle_schedule_request(
        state,
        task,
        resolved_prompt_for_execution,
        route_result.schedule_intent.as_ref(),
    )
    .await
    {
        let schedule_reply = crate::intercept_response_text_for_delivery(&schedule_reply);
        finalize_ask_direct_success(
            state,
            task,
            payload,
            prompt,
            &schedule_reply,
            "schedule_direct",
            route_result.should_refresh_long_term_memory,
            &route_result.agent_display_name_hint,
        )
        .await?;
        return Ok(true);
    }
    Ok(false)
}

fn should_use_answer_route_result(
    initial: &crate::RouteResult,
    answer_route: &crate::RouteResult,
    answer_journal: &crate::task_journal::TaskJournal,
) -> bool {
    let answer_is_clarify = answer_journal.final_status.is_some_and(|status| {
        matches!(status, crate::task_journal::TaskJournalFinalStatus::Clarify)
    });
    if answer_is_clarify && answer_route.is_clarify_gate() && !initial.is_clarify_gate() {
        return true;
    }
    let answer_has_execution_trace = !answer_journal.rounds.is_empty()
        || !answer_journal.step_results.is_empty()
        || answer_journal.plan_result.is_some()
        || answer_journal.verify_result.is_some();
    answer_has_execution_trace && answer_route.is_execute_gate() && !initial.is_execute_gate()
}

pub(crate) async fn finalize_ask_result(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    prompt: &str,
    context_bundle_summary: &str,
    memory_trace: Option<&Value>,
    resolved_prompt_for_execution: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    fuzzy_locator_suggestions: &[String],
    clarify_fallback_source: Option<crate::fallback::ClarifyFallbackSource>,
    result: Result<crate::AskReply, String>,
) -> Result<()> {
    // §3.1: ask 状态机 — 进入 finalize。
    // from = None 因为 dispatch 内部各分支态没向调用面回传"上一次状态"；
    // reason 携带 ask_mode 信息以便日志检索。
    let finalize_entry_transition = crate::log_ask_transition(
        state,
        &task.task_id,
        None,
        crate::AskState::Finalizing,
        &format!(
            "finalize_ask_result_entry mode={}",
            route_result.ask_mode.as_str()
        ),
        None,
    );
    let mut journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", prompt);
    journal.transitions.push(finalize_entry_transition);
    if let Some(turn_analysis) = turn_analysis {
        journal.record_turn_analysis(turn_analysis);
    }
    journal.record_route_result(route_result);
    journal.record_context_bundle_summary(format!(
        "{} needs_clarify={} resolved_prompt={}",
        context_bundle_summary,
        route_result.ask_mode.is_clarify_only(),
        crate::truncate_for_log(resolved_prompt_for_execution)
    ));
    if let Some(memory_trace) = memory_trace {
        journal.record_memory_trace(memory_trace.clone());
    }
    match result {
        Ok(answer) => {
            if !repo::is_task_still_running(state, &task.task_id)? {
                state.clear_task_llm_call_count(&task.task_id);
                info!(
                    "task_call_end task_id={} kind=ask status=canceled path=normal",
                    task.task_id
                );
                return Ok(());
            }
            let mut effective_route_result = route_result.clone();
            if let Some(answer_journal) = answer.task_journal.as_ref() {
                journal.merge_from(answer_journal);
                if let Some(answer_route_result) = answer_journal.route_result.as_ref() {
                    if should_use_answer_route_result(
                        route_result,
                        answer_route_result,
                        answer_journal,
                    ) {
                        effective_route_result = answer_route_result.clone();
                        journal.record_route_result(&effective_route_result);
                    }
                }
            }
            let route_result = &effective_route_result;
            let semantic_clarify = route_result.ask_mode.is_clarify_only()
                || answer
                    .task_journal
                    .as_ref()
                    .and_then(|journal| journal.final_status)
                    .is_some_and(|status| {
                        matches!(status, crate::task_journal::TaskJournalFinalStatus::Clarify)
                    });
            let failure_reply = answer.should_fail_task;
            let missing_file_delivery_reply =
                missing_file_delivery_reply_text(state, task, prompt, route_result, &answer).await;
            let (answer_text, answer_messages) = if failure_reply
                || route_result.ask_mode.is_clarify_only()
            {
                (
                    crate::intercept_response_text_for_delivery(&answer.text),
                    answer
                        .messages
                        .into_iter()
                        .map(|message| message.trim().to_string())
                        .filter(|message| !message.is_empty())
                        .collect(),
                )
            } else if let Some(reply_text) = missing_file_delivery_reply {
                let mut messages = answer
                    .messages
                    .into_iter()
                    .map(|message| message.trim().to_string())
                    .filter(|message| !message.is_empty())
                    .filter(|message| crate::finalize::is_execution_summary_message(message))
                    .collect::<Vec<_>>();
                messages.push(reply_text.clone());
                (reply_text, messages)
            } else {
                let original_messages = answer.messages;
                let execution_summaries = original_messages
                    .iter()
                    .map(|message| message.trim().to_string())
                    .filter(|message| !message.is_empty())
                    .filter(|message| crate::finalize::is_execution_summary_message(message))
                    .collect::<Vec<_>>();
                let (answer_text, mut answer_messages) =
                    crate::intercept_response_payload_for_delivery(
                        state,
                        // Delivery interception must stay grounded in the original user request.
                        // The execution prompt may contain injected runtime hints such as
                        // [AUTO_LOCATOR], which are useful for planning/execution but must not be
                        // reinterpreted as fresh user-provided locator input during final delivery
                        // normalization.
                        prompt,
                        route_result.wants_file_delivery,
                        &route_result.output_contract,
                        answer.text,
                        original_messages,
                    );
                if should_reinsert_execution_summaries_for_delivery(route_result, &answer_text) {
                    for summary in execution_summaries.into_iter().rev() {
                        if !answer_messages.iter().any(|message| message == &summary) {
                            answer_messages.insert(0, summary);
                        }
                    }
                } else {
                    drop_execution_summaries_when_delivery_is_scalar(
                        route_result,
                        &answer_text,
                        &mut answer_messages,
                    );
                }
                (answer_text, answer_messages)
            };
            journal.record_final_answer(&answer_text);
            if !failure_reply && !semantic_clarify && journal.answer_verifier_summary.is_none() {
                if let Some(answer_verifier) = crate::answer_verifier::verify_answer_observe_only(
                    state,
                    task,
                    prompt,
                    route_result,
                    &journal,
                    &answer_text,
                )
                .await
                {
                    journal.record_answer_verifier_summary(answer_verifier);
                }
            }
            journal.record_llm_calls_per_task(state.task_llm_call_count(&task.task_id));
            journal.record_llm_elapsed_ms_per_task(state.task_llm_elapsed_ms(&task.task_id));
            journal.record_llm_by_prompt(state.task_llm_by_prompt(&task.task_id));
            crate::finalize::ensure_task_metrics(&mut journal, &answer_text, &answer_messages);
            if failure_reply {
                let err_text = answer.error_text.unwrap_or_else(|| answer_text.clone());
                if let Some(resume_payload) = answer.resume_context {
                    journal.record_final_status(
                        crate::task_journal::TaskJournalFinalStatus::ResumeFailure,
                    );
                    finalize_ask_resume_failure(
                        state,
                        task,
                        payload,
                        &err_text,
                        resume_payload,
                        &answer_messages,
                        &mut journal,
                    )
                    .await?;
                    insert_unfinished_goal_memory(state, task, prompt, &err_text);
                } else {
                    journal
                        .record_final_status(crate::task_journal::TaskJournalFinalStatus::Failure);
                    finalize_ask_failure(
                        state,
                        task,
                        payload,
                        &answer_text,
                        &answer_messages,
                        &err_text,
                        &mut journal,
                    )
                    .await?;
                    insert_unfinished_goal_memory(state, task, prompt, &err_text);
                }
            } else {
                journal.record_final_status(non_failure_final_status(semantic_clarify));
                finalize_ask_success(
                    state,
                    task,
                    payload,
                    prompt,
                    &answer_text,
                    &answer_messages,
                    answer.is_llm_reply,
                    route_result.should_refresh_long_term_memory,
                    &route_result.agent_display_name_hint,
                    &mut journal,
                )
                .await?;
                crate::conversation_state::update_active_session_from_ask_outcome(
                    state,
                    task,
                    Some(payload),
                    prompt,
                    route_result,
                    turn_analysis,
                    resolved_prompt_for_execution,
                    &answer_text,
                    &answer_messages,
                    semantic_clarify,
                    fuzzy_locator_suggestions,
                    &journal,
                    clarify_fallback_source,
                );
                if semantic_clarify {
                    insert_unfinished_goal_memory(state, task, prompt, &answer_text);
                }
            }
            // §3.1: Finalizing → Completed（成功路径，含 success / failure / resume_failure / clarify
            // 子分类，在 final_status 字段已区分；这里 ask 状态机视为正常完成 = Completed）。
            // 真实失败的 Err(...) 入分支会在下方打 Failed。
            let completed_transition = crate::log_ask_transition(
                state,
                &task.task_id,
                Some(crate::AskState::Finalizing),
                crate::AskState::Completed,
                "finalize_ok",
                None,
            );
            journal.transitions.push(completed_transition);
            info!(
                "task_journal_summary task_id={} kind=ask phase=finalize {}",
                task.task_id,
                journal.to_log_json()
            );
            state.clear_task_llm_call_count(&task.task_id);
        }
        Err(err_text) => {
            if err_text == crate::agent_engine::TASK_CANCELED_ERR
                || !repo::is_task_still_running(state, &task.task_id)?
            {
                state.clear_task_llm_call_count(&task.task_id);
                info!(
                    "task_call_end task_id={} kind=ask status=canceled path=normal",
                    task.task_id
                );
                return Ok(());
            }
            if let Some((user_error, resume_ctx)) = crate::parse_resume_context_error(&err_text) {
                info!(
                    "task_journal_summary task_id={} kind=ask phase=resume_failure {}",
                    task.task_id,
                    journal.to_log_json()
                );
                let resume_payload = resume_ctx
                    .get("resume_context")
                    .cloned()
                    .unwrap_or(resume_ctx);
                let language_hint =
                    crate::language_policy::task_response_language_hint(state, task, prompt);
                let prefer_english = language_hint.to_ascii_lowercase().starts_with("en");
                let qualified_resume_completion = if resume_failure_is_missing_file_delivery_result(
                    route_result,
                    &user_error,
                    &resume_payload,
                ) {
                    Some(("missing_file_delivery", user_error.clone()))
                } else if resume_failure_is_structured_service_status_result(
                    route_result,
                    &resume_payload,
                ) {
                    Some(("structured_service_status", user_error.clone()))
                } else if let Some(answer) = resume_failure_execution_failed_step_answer(
                    route_result,
                    &resume_payload,
                    prefer_english,
                ) {
                    Some(("execution_failed_step", answer))
                } else {
                    None
                };
                if let Some((qualified_resume_reason, qualified_answer)) =
                    qualified_resume_completion
                {
                    let mut answer_messages =
                        resume_context_execution_summary_messages(&resume_payload, prefer_english);
                    answer_messages.push(qualified_answer.clone());
                    journal.record_llm_calls_per_task(state.task_llm_call_count(&task.task_id));
                    journal
                        .record_llm_elapsed_ms_per_task(state.task_llm_elapsed_ms(&task.task_id));
                    journal.record_llm_by_prompt(state.task_llm_by_prompt(&task.task_id));
                    journal.record_final_answer(&qualified_answer);
                    crate::finalize::ensure_task_metrics(
                        &mut journal,
                        &qualified_answer,
                        &answer_messages,
                    );
                    journal
                        .record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
                    finalize_ask_success(
                        state,
                        task,
                        payload,
                        prompt,
                        &qualified_answer,
                        &answer_messages,
                        false,
                        route_result.should_refresh_long_term_memory,
                        &route_result.agent_display_name_hint,
                        &mut journal,
                    )
                    .await?;
                    crate::conversation_state::update_active_session_from_ask_outcome(
                        state,
                        task,
                        Some(payload),
                        prompt,
                        route_result,
                        turn_analysis,
                        resolved_prompt_for_execution,
                        &qualified_answer,
                        &answer_messages,
                        false,
                        fuzzy_locator_suggestions,
                        &journal,
                        clarify_fallback_source,
                    );
                    let completed_transition = crate::log_ask_transition(
                        state,
                        &task.task_id,
                        Some(crate::AskState::Finalizing),
                        crate::AskState::Completed,
                        &format!("finalize_{qualified_resume_reason}_resume_success"),
                        None,
                    );
                    journal.transitions.push(completed_transition);
                    info!(
                        "task_journal_summary task_id={} kind=ask phase=qualified_resume_success reason={} {}",
                        task.task_id,
                        qualified_resume_reason,
                        journal.to_log_json()
                    );
                    state.clear_task_llm_call_count(&task.task_id);
                    return Ok(());
                }
                journal.record_llm_calls_per_task(state.task_llm_call_count(&task.task_id));
                journal.record_llm_elapsed_ms_per_task(state.task_llm_elapsed_ms(&task.task_id));
                journal.record_llm_by_prompt(state.task_llm_by_prompt(&task.task_id));
                journal.record_final_answer(&user_error);
                crate::finalize::ensure_task_metrics(&mut journal, &user_error, &[]);
                journal.record_final_status(
                    crate::task_journal::TaskJournalFinalStatus::ResumeFailure,
                );
                finalize_ask_resume_failure(
                    state,
                    task,
                    payload,
                    &user_error,
                    resume_payload,
                    &[],
                    &mut journal,
                )
                .await?;
                insert_unfinished_goal_memory(state, task, prompt, &user_error);
                // §3.1: Finalizing → Failed (resume_failure 子路径)。
                crate::log_ask_transition(
                    state,
                    &task.task_id,
                    Some(crate::AskState::Finalizing),
                    crate::AskState::Failed,
                    "finalize_resume_failure",
                    None,
                );
                state.clear_task_llm_call_count(&task.task_id);
                return Ok(());
            }
            let user_error = compose_ask_failure_user_message(state, task, prompt, &err_text).await;
            journal.record_llm_calls_per_task(state.task_llm_call_count(&task.task_id));
            journal.record_llm_elapsed_ms_per_task(state.task_llm_elapsed_ms(&task.task_id));
            journal.record_llm_by_prompt(state.task_llm_by_prompt(&task.task_id));
            journal.record_final_answer(&user_error);
            crate::finalize::ensure_task_metrics(&mut journal, &user_error, &[]);
            journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Failure);
            finalize_ask_failure(
                state,
                task,
                payload,
                &user_error,
                &[],
                &err_text,
                &mut journal,
            )
            .await?;
            insert_unfinished_goal_memory(state, task, prompt, &user_error);
            // §3.1: Finalizing → Failed（dispatch 抛 Err 进入此分支）。
            crate::log_ask_transition(
                state,
                &task.task_id,
                Some(crate::AskState::Finalizing),
                crate::AskState::Failed,
                "finalize_err",
                None,
            );
            info!(
                "task_journal_summary task_id={} kind=ask phase=failure error={} {}",
                task.task_id,
                crate::truncate_for_log(&err_text),
                journal.to_log_json()
            );
            state.clear_task_llm_call_count(&task.task_id);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        assistant_memory_source_text, drop_execution_summaries_when_delivery_is_scalar,
        journal_has_missing_file_search_evidence, non_failure_final_status,
        should_reinsert_execution_summaries_for_delivery, should_use_answer_route_result,
    };

    use serde_json::json;

    fn route_result(ask_mode: crate::AskMode) -> crate::RouteResult {
        crate::RouteResult {
            ask_mode,
            resolved_intent: "test".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        }
    }

    // ensure_journal_task_metrics_* tests 已搬移到 finalize/journal.rs（Stage 3.1）。

    #[test]
    fn non_failure_final_status_preserves_clarify_semantics() {
        assert_eq!(
            non_failure_final_status(false),
            crate::task_journal::TaskJournalFinalStatus::Success
        );
        assert_eq!(
            non_failure_final_status(true),
            crate::task_journal::TaskJournalFinalStatus::Clarify
        );
    }

    #[test]
    fn assistant_memory_source_text_filters_execution_summary() {
        let messages = vec![
            "**执行过程**\n1. 调用命令 `pwd`\n   输出：\n```text\n/tmp\n```".to_string(),
            "最终答案".to_string(),
        ];

        assert_eq!(
            assistant_memory_source_text("最终答案", &messages),
            "最终答案"
        );
    }

    #[test]
    fn assistant_memory_source_text_drops_execution_summary_only_answers() {
        let messages = vec![
            "**执行过程**\n1. 调用技能 `rss_fetch`\n   输出：ok".to_string(),
            "**执行过程**\n1. 调用技能 `rss_fetch`\n   输出：ok".to_string(),
        ];

        assert_eq!(
            assistant_memory_source_text(
                "**执行过程**\n1. 调用技能 `rss_fetch`\n   输出：ok",
                &messages
            ),
            ""
        );
    }

    #[test]
    fn scalar_delivery_does_not_reinsert_execution_summary() {
        let mut route = route_result(crate::AskMode::Act {
            finalize: crate::ActFinalizeStyle::Plain,
        });
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;

        assert!(!should_reinsert_execution_summaries_for_delivery(
            &route, "1.0.0"
        ));
    }

    #[test]
    fn scalar_delivery_drops_existing_execution_summary_messages() {
        let mut route = route_result(crate::AskMode::Act {
            finalize: crate::ActFinalizeStyle::Plain,
        });
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        let mut messages = vec![
            "**执行过程**\n1. 调用工具 `fs_basic`\n   输出：ok".to_string(),
            "{\"workspace\":true}".to_string(),
        ];

        drop_execution_summaries_when_delivery_is_scalar(
            &route,
            "{\"workspace\":true}",
            &mut messages,
        );

        assert_eq!(messages, vec!["{\"workspace\":true}".to_string()]);
    }

    #[test]
    fn config_validation_delivery_drops_existing_execution_summary_messages() {
        let mut route = route_result(crate::AskMode::Act {
            finalize: crate::ActFinalizeStyle::ChatWrapped,
        });
        route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ConfigValidation;
        let mut messages = vec![
            "**Execution**\n1. Called tool `config_basic`\n   Output: valid".to_string(),
            "pass".to_string(),
        ];

        drop_execution_summaries_when_delivery_is_scalar(&route, "pass", &mut messages);

        assert_eq!(messages, vec!["pass".to_string()]);
    }

    #[test]
    fn free_delivery_keeps_execution_summary_available() {
        let mut route = route_result(crate::AskMode::Act {
            finalize: crate::ActFinalizeStyle::Plain,
        });
        route.output_contract.response_shape = crate::OutputResponseShape::Free;

        assert!(should_reinsert_execution_summaries_for_delivery(
            &route,
            "配置检查通过。"
        ));
    }

    #[test]
    fn journal_missing_file_search_evidence_detects_zero_match_fs_search() {
        let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                skill: "fs_search".to_string(),
                output_excerpt: Some(
                    json!({
                        "action": "find_name",
                        "count": 0,
                        "results": [],
                        "root": ""
                    })
                    .to_string(),
                ),
                ..Default::default()
            });
        assert!(journal_has_missing_file_search_evidence(Some(&journal)));
    }

    #[test]
    fn journal_missing_file_search_evidence_detects_path_batch_facts() {
        let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                skill: "system_basic".to_string(),
                output_excerpt: Some(
                    json!({
                        "action": "path_batch_facts",
                        "count": 1,
                        "facts": [{
                            "exists": false,
                            "path": "/tmp/missing.txt",
                            "error": "not found"
                        }],
                        "include_missing": true
                    })
                    .to_string(),
                ),
                ..Default::default()
            });
        assert!(journal_has_missing_file_search_evidence(Some(&journal)));
    }

    #[test]
    fn journal_missing_file_search_evidence_detects_not_found_probe() {
        let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                skill: "run_cmd".to_string(),
                output_excerpt: Some("NOT_FOUND\n".to_string()),
                ..Default::default()
            });
        assert!(journal_has_missing_file_search_evidence(Some(&journal)));
    }

    #[test]
    fn answer_route_result_overrides_initial_chat_when_execution_trace_exists() {
        let initial = route_result(crate::AskMode::direct_answer());
        let answer_route = route_result(crate::AskMode::planner_execute_chat_wrapped());
        let mut answer_journal =
            crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
        answer_journal.record_plan_result(&crate::PlanResult {
            plan_kind: crate::PlanKind::Single,
            goal: "inspect project".to_string(),
            planner_notes: String::new(),
            raw_plan_text: String::new(),
            missing_slots: Vec::new(),
            needs_confirmation: false,
            steps: Vec::new(),
        });

        assert!(should_use_answer_route_result(
            &initial,
            &answer_route,
            &answer_journal
        ));
    }

    #[test]
    fn answer_route_result_does_not_override_chat_without_execution_trace() {
        let initial = route_result(crate::AskMode::direct_answer());
        let answer_route = route_result(crate::AskMode::planner_execute_chat_wrapped());
        let answer_journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");

        assert!(!should_use_answer_route_result(
            &initial,
            &answer_route,
            &answer_journal
        ));
    }

    #[test]
    fn answer_route_result_overrides_initial_chat_for_clarify_journal() {
        let initial = route_result(crate::AskMode::direct_answer());
        let mut answer_route = route_result(crate::AskMode::clarify());
        answer_route.needs_clarify = true;
        answer_route.clarify_question = "Which file should I send?".to_string();
        answer_route.wants_file_delivery = true;
        answer_route.output_contract.delivery_required = true;
        answer_route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
        let mut answer_journal =
            crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
        answer_journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Clarify);

        assert!(should_use_answer_route_result(
            &initial,
            &answer_route,
            &answer_journal
        ));
    }

    #[test]
    fn journal_missing_file_search_evidence_detects_read_file_error_marker() {
        let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                skill: "read_file".to_string(),
                error_excerpt: Some("__RC_READ_FILE_NOT_FOUND__:/tmp/missing.txt".to_string()),
                ..Default::default()
            });
        assert!(journal_has_missing_file_search_evidence(Some(&journal)));
    }

    #[test]
    fn missing_file_delivery_reply_uses_structured_search_evidence() {
        let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                skill: "fs_search".to_string(),
                output_excerpt: Some(
                    json!({
                        "action": "find_name",
                        "count": 0,
                        "results": [],
                        "root": ""
                    })
                    .to_string(),
                ),
                ..Default::default()
            });
        let answer = crate::AskReply::llm(
            "文件 `definitely_missing_named_file_rustclaw_001.txt` 未找到。".to_string(),
        )
        .with_task_journal(journal);
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "send definitely_missing_named_file_rustclaw_001.txt".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "explicit filename".to_string(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: true,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        assert!(route.wants_file_delivery);
        assert!(journal_has_missing_file_search_evidence(
            answer.task_journal.as_ref()
        ));
    }

    #[test]
    fn missing_file_delivery_reply_uses_output_contract_file_token_even_without_wants_flag() {
        let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "prompt");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                skill: "fs_search".to_string(),
                output_excerpt: Some(
                    json!({
                        "action": "find_name",
                        "count": 0,
                        "results": [],
                        "root": ""
                    })
                    .to_string(),
                ),
                ..Default::default()
            });
        let answer = crate::AskReply::llm(
            "找不到文件 `definitely_missing_named_file_rustclaw_001.txt`。".to_string(),
        )
        .with_task_journal(journal);
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
        route.output_contract.delivery_required = true;

        assert!(super::should_use_missing_file_delivery_reply(
            &route, &answer
        ));
    }

    #[test]
    fn resume_failure_missing_file_delivery_is_success_result() {
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
        route.output_contract.delivery_required = true;
        let resume_ctx = json!({
            "failed_step": {
                "action": "skill(run_cmd)",
                "error": "__RC_READ_FILE_NOT_FOUND__:/tmp/missing.txt"
            },
            "remaining_actions": []
        });

        assert!(super::resume_failure_is_missing_file_delivery_result(
            &route,
            "I couldn't send the requested file because it doesn't exist at the path `/tmp/missing.txt`.",
            &resume_ctx
        ));
    }

    #[test]
    fn resume_failure_structured_service_status_is_success_result() {
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ServiceStatus;
        let resume_ctx = json!({
            "failed_step": {
                "action": "skill(service_control)",
                "error": "no matching service found for the given target",
                "structured_error": {
                    "skill": "service_control",
                    "error_kind": "not_found",
                    "error_text": "no matching service found for the given target",
                    "service_name": "definitely_missing_rustclaw_demo",
                    "platform": "linux",
                    "manager_type": "unknown"
                }
            },
            "remaining_actions": []
        });

        assert!(super::resume_failure_is_structured_service_status_result(
            &route,
            &resume_ctx
        ));

        let messages = super::resume_context_execution_summary_messages(&resume_ctx, false);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains("no matching service found"));
        assert!(!messages[0].contains("__RC_SKILL_ERROR__"));
    }

    #[test]
    fn resume_failure_execution_failed_step_is_success_answer_with_remaining_actions() {
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ExecutionFailedStep;
        let resume_ctx = json!({
            "failed_step": {
                "action": "skill(run_cmd)",
                "error": "command failed with exit code 1; stderr: cat: /definitely_missing_rustclaw_contract_case: No such file or directory (os error 2)",
                "structured_error": {
                    "skill": "run_cmd",
                    "error_kind": "nonzero_exit",
                    "error_text": "Command failed with exit code 1\nstderr:\ncat: /definitely_missing_rustclaw_contract_case: No such file or directory (os error 2)",
                    "platform": "linux",
                    "extra": {
                        "command": "cat /definitely_missing_rustclaw_contract_case",
                        "exit_code": 1,
                        "stderr": "cat: /definitely_missing_rustclaw_contract_case: No such file or directory (os error 2)\n"
                    }
                }
            },
            "remaining_actions": [
                {"type": "call_skill", "skill": "log_analyze"},
                {"type": "synthesize_answer"}
            ]
        });

        let answer = super::resume_failure_execution_failed_step_answer(&route, &resume_ctx, false)
            .expect("execution-failed-step answer");

        assert!(answer.contains("cat /definitely_missing_rustclaw_contract_case"));
        assert!(answer.contains("退出码为 1"));
        assert!(answer.contains("No such file or directory"));
        assert!(!answer.contains("继续"));
        assert!(!answer.contains("暂停"));
    }

    #[test]
    fn resume_context_execution_summary_uses_failed_step() {
        let resume_ctx = json!({
            "failed_step": {
                "action": "skill(run_cmd)",
                "error": "ls: cannot access '/tmp/missing.txt': No such file or directory"
            },
            "remaining_actions": []
        });

        let messages = super::resume_context_execution_summary_messages(&resume_ctx, false);

        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains("**执行过程**"));
        assert!(messages[0].contains("skill(run_cmd)"));
        assert!(messages[0].contains("No such file or directory"));
    }
}
