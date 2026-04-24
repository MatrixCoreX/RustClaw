use anyhow::Result;
use serde_json::{json, Value};
use tracing::{error, info};

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

/// §7.2 后保留：仍被该模块某些调用方 / 单测引用以拿到老 super-fallback 的字面字符串。
/// 真正的"是不是 fallback 占位符"判定走 [`crate::fallback::is_known_clarify_fallback_text`]。
#[allow(dead_code)]
fn provider_unavailable_answer_text(state: &AppState) -> String {
    crate::i18n_t_with_default(
        state,
        crate::fallback::LEGACY_SUPER_FALLBACK_KEY,
        crate::fallback::LEGACY_SUPER_FALLBACK_DEFAULT_EN,
    )
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
        .any(|message| crate::fallback::is_known_clarify_fallback_text(state, message))
}

fn journal_has_missing_file_search_evidence(
    journal: Option<&crate::task_journal::TaskJournal>,
) -> bool {
    journal
        .into_iter()
        .flat_map(|journal| journal.step_results.iter().rev())
        .any(|step| {
            if step.skill != "fs_search" {
                return false;
            }
            let Some(output) = step.output_excerpt.as_deref() else {
                return false;
            };
            let Ok(value) = serde_json::from_str::<Value>(output) else {
                return false;
            };
            value.get("action").and_then(|v| v.as_str()) == Some("find_name")
                && value.get("count").and_then(|v| v.as_i64()) == Some(0)
                && value
                    .get("results")
                    .and_then(|v| v.as_array())
                    .is_some_and(|results| results.is_empty())
        })
}

fn has_any_delivery_file_token(text: &str, messages: &[String]) -> bool {
    !crate::extract_delivery_file_tokens(text).is_empty()
        || messages
            .iter()
            .any(|message| !crate::extract_delivery_file_tokens(message).is_empty())
}

fn missing_file_delivery_reply_text(
    state: &AppState,
    route_result: &crate::RouteResult,
    answer: &crate::AskReply,
) -> Option<String> {
    let file_delivery_contract = route_result.wants_file_delivery
        || route_result.output_contract.delivery_required
        || matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        );
    (file_delivery_contract
        && !answer.should_fail_task
        && !has_any_delivery_file_token(&answer.text, &answer.messages)
        && journal_has_missing_file_search_evidence(answer.task_journal.as_ref()))
    .then(|| {
        crate::i18n_t_with_default(
            state,
            "clawd.msg.delivery.rule3_file_not_found",
            "File not found.",
        )
    })
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
    let assistant_source_text = if answer_messages.is_empty() {
        answer_text.to_string()
    } else {
        answer_messages.join("\n")
    };
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
    journal: &crate::task_journal::TaskJournal,
) -> Result<()> {
    let result = ask_result_payload(answer_text, answer_messages, Some(journal));
    repo::update_task_success(state, &task.task_id, &result.to_string())?;
    crate::worker::maybe_notify_schedule_result(state, task, payload, true, answer_text).await;
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
    journal: &crate::task_journal::TaskJournal,
) -> Result<()> {
    let mut result = ask_result_payload(user_error, answer_messages, None);
    if let Some(obj) = result.as_object_mut() {
        obj.insert("resume_context".to_string(), resume_payload);
    }
    let result = journal.attach_to_result(result);
    repo::update_task_failure_with_result(state, &task.task_id, &result.to_string(), user_error)?;
    crate::worker::maybe_notify_schedule_result(state, task, payload, false, user_error).await;
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
    journal: &crate::task_journal::TaskJournal,
) -> Result<()> {
    error!(
        "worker_once: ask task_id={} failed: {}",
        task.task_id, err_text
    );
    let result = journal.attach_to_result(ask_result_payload(answer_text, answer_messages, None));
    repo::update_task_failure_with_result(state, &task.task_id, &result.to_string(), err_text)?;
    crate::worker::maybe_notify_schedule_result(state, task, payload, false, answer_text).await;
    info!("{}", crate::LOG_CALL_WRAP);
    info!(
        "task_call_end task_id={} kind=ask status=failed path=normal error={}",
        task.task_id,
        crate::truncate_for_log(err_text)
    );
    info!("{}", crate::LOG_CALL_WRAP);
    Ok(())
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
    let result = journal.attach_to_result(json!({ "text": answer_text }));
    repo::update_task_success(state, &task.task_id, &result.to_string())?;
    crate::worker::maybe_notify_schedule_result(state, task, payload, true, answer_text).await;
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
    let prompt = format!(
        "You are producing the final user-facing reply directly.\n\nLanguage policy (strict): use {} as the highest-priority default for user-visible text. Override to English only when the current user request is fully English with no meaningful non-English content. Do not switch languages just because names, paths, commands, code, or other normalized values are in English.\n\nReturn only the user-facing reply.\n\n{}",
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

pub(crate) async fn finalize_ask_result(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    prompt: &str,
    context_bundle_summary: &str,
    resolved_prompt_for_execution: &str,
    route_result: &crate::RouteResult,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
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
            if let Some(answer_journal) = answer.task_journal.as_ref() {
                journal.merge_from(answer_journal);
            }
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
                missing_file_delivery_reply_text(state, route_result, &answer);
            let (answer_text, answer_messages) =
                if failure_reply || route_result.ask_mode.is_clarify_only() {
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
                    (reply_text.clone(), vec![reply_text])
                } else {
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
                        answer.messages,
                    )
                };
            journal.record_llm_calls_per_task(state.task_llm_call_count(&task.task_id));
            journal.record_llm_elapsed_ms_per_task(state.task_llm_elapsed_ms(&task.task_id));
            journal.record_llm_by_prompt(state.task_llm_by_prompt(&task.task_id));
            journal.record_final_answer(&answer_text);
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
                        &journal,
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
                        &journal,
                    )
                    .await?;
                    insert_unfinished_goal_memory(state, task, prompt, &err_text);
                }
            } else {
                journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
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
                    &journal,
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
                    &journal,
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
                    &journal,
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
            journal.record_llm_calls_per_task(state.task_llm_call_count(&task.task_id));
            journal.record_llm_elapsed_ms_per_task(state.task_llm_elapsed_ms(&task.task_id));
            journal.record_llm_by_prompt(state.task_llm_by_prompt(&task.task_id));
            journal.record_final_answer(&err_text);
            crate::finalize::ensure_task_metrics(&mut journal, &err_text, &[]);
            journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Failure);
            finalize_ask_failure(state, task, payload, &err_text, &[], &err_text, &journal).await?;
            insert_unfinished_goal_memory(state, task, prompt, &err_text);
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
    use super::journal_has_missing_file_search_evidence;
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, RwLock};

    use serde_json::json;

    use crate::{
        runtime::{AgentRuntimeConfig, SkillViewsSnapshot},
        AppState, CommandIntentRuntime, ScheduleRuntime, ToolsPolicy,
    };
    use claw_core::config::{AgentConfig, ToolsConfig};

    fn test_state() -> AppState {
        let agents_by_id = HashMap::from([(
            crate::DEFAULT_AGENT_ID.to_string(),
            AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
        )]);
        AppState {
            core: crate::CoreServices {
                agents_by_id: Arc::new(agents_by_id),
                skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                    registry: None,
                    skills_list: Arc::new(HashSet::new()),
                }))),
                ..crate::CoreServices::test_default()
            },
            skill_rt: crate::SkillRuntime {
                locator_scan_max_depth: 3,
                locator_scan_max_files: 200,
                tools_policy: Arc::new(
                    ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
                ),
                ..crate::SkillRuntime::test_default()
            },
            policy: crate::PolicyConfig {
                command_intent: CommandIntentRuntime {
                    all_result_suffixes: Vec::new(),
                    default_locale: "en".to_string(),
                    verify_enforce_enabled: false,
                },
                schedule: ScheduleRuntime {
                    timezone: "Asia/Shanghai".to_string(),
                    intent_prompt_template: Arc::new(RwLock::new(String::new())),
                    intent_prompt_source: String::new(),
                    intent_rules_template: Arc::new(RwLock::new(String::new())),
                    locale: "en".to_string(),
                    i18n_dict: HashMap::new(),
                },
                ..crate::PolicyConfig::test_default()
            },
            worker: crate::WorkerConfig::test_default(),
            metrics: crate::TaskMetricsRegistry::default(),
            channels: crate::ChannelConfig::default(),
            reload_ctx: crate::ReloadContext::default(),
            ask_states: crate::AskStateRegistry::default(),
        }
    }

    // ensure_journal_task_metrics_* tests 已搬移到 finalize/journal.rs（Stage 3.1）。

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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
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
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
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
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        };
        route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
        route.output_contract.delivery_required = true;

        let state = test_state();
        let reply = super::missing_file_delivery_reply_text(&state, &route, &answer);
        assert_eq!(reply.as_deref(), Some("File not found."));
    }
}
