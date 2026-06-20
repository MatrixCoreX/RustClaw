use serde_json::Value;
use tracing::warn;

use crate::AppState;

fn resume_context_body(value: &Value) -> &Value {
    value.get("resume_context").unwrap_or(value)
}

pub(super) fn text_looks_like_missing_file_target(text: &str) -> bool {
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

fn resume_context_failed_step_texts(resume_ctx: &Value) -> Vec<&str> {
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

fn resume_context_failed_step_action(resume_ctx: &Value) -> Option<&str> {
    resume_context_body(resume_ctx)
        .get("failed_step")
        .and_then(|step| step.get("action"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn resume_context_failed_step_skill(resume_ctx: &Value) -> Option<String> {
    if let Some(error) = resume_context_failed_structured_skill_error(resume_ctx) {
        return Some(error.skill);
    }
    let action = resume_context_failed_step_action(resume_ctx)?;
    action
        .strip_prefix("skill(")
        .and_then(|value| value.strip_suffix(')'))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn resume_context_completed_message_json(message: &str) -> Option<Value> {
    let trimmed = message.trim();
    let json_start = trimmed.find('{')?;
    serde_json::from_str::<Value>(&trimmed[json_start..]).ok()
}

fn resume_context_completed_structured_values(resume_ctx: &Value) -> Vec<Value> {
    resume_context_body(resume_ctx)
        .get("completed_messages")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .filter_map(resume_context_completed_message_json)
        .collect()
}

pub(super) fn resume_context_path_batch_facts_are_missing_only(resume_ctx: &Value) -> bool {
    let mut saw_path_batch = false;
    let mut saw_missing = false;
    let mut saw_existing = false;
    for value in resume_context_completed_structured_values(resume_ctx) {
        if value.get("action").and_then(|value| value.as_str()) != Some("path_batch_facts") {
            continue;
        }
        saw_path_batch = true;
        if let Some(facts) = value.get("facts").and_then(|value| value.as_array()) {
            for fact in facts {
                match fact.get("exists").and_then(|value| value.as_bool()) {
                    Some(true) => saw_existing = true,
                    Some(false) => {
                        if fact
                            .get("path")
                            .and_then(|value| value.as_str())
                            .is_some_and(|path| !path.trim().is_empty())
                        {
                            saw_missing = true;
                        }
                    }
                    None => {}
                }
            }
        }
    }
    saw_path_batch && saw_missing && !saw_existing
}

fn text_is_directory_lookup_failure(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.starts_with("read_dir failed")
        || crate::skills::parse_structured_skill_error(trimmed)
            .is_some_and(|structured| structured.error_text.trim().starts_with("read_dir failed"))
}

pub(super) fn resume_context_has_directory_lookup_failure(resume_ctx: &Value) -> bool {
    let body = resume_context_body(resume_ctx);
    if body
        .get("failed_step")
        .and_then(|step| step.get("error"))
        .and_then(|value| value.as_str())
        .is_some_and(text_is_directory_lookup_failure)
    {
        return true;
    }
    body.get("failed_step")
        .and_then(|step| step.get("structured_error"))
        .and_then(|error| error.get("error_text"))
        .and_then(|value| value.as_str())
        .is_some_and(text_is_directory_lookup_failure)
}

pub(super) fn resume_failure_is_unbound_path_lookup_clarify_result(
    route_result: &crate::RouteResult,
    resume_ctx: &Value,
) -> bool {
    route_result.output_contract.requires_content_evidence
        && !super::route_has_file_delivery_contract(route_result)
        && !resume_context_has_remaining_actions(resume_ctx)
        && !matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
        && matches!(
            route_result.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
                | crate::OutputSemanticKind::ScalarPathOnly
                | crate::OutputSemanticKind::ExistenceWithPath
                | crate::OutputSemanticKind::ExistenceWithPathSummary
                | crate::OutputSemanticKind::FilePaths
        )
        && resume_context_failed_step_skill(resume_ctx).as_deref() == Some("fs_search")
        && (resume_context_path_batch_facts_are_missing_only(resume_ctx)
            || resume_context_has_directory_lookup_failure(resume_ctx))
}

pub(super) fn resume_failure_is_missing_file_delivery_result(
    route_result: &crate::RouteResult,
    user_error: &str,
    resume_ctx: &Value,
) -> bool {
    super::route_has_file_delivery_contract(route_result)
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

pub(super) fn direct_chat_answer_verifier_retry_applicable(
    route_result: &crate::RouteResult,
    journal: &crate::task_journal::TaskJournal,
    verifier: &crate::answer_verifier::AnswerVerifierOut,
) -> bool {
    if !verifier.high_confidence_gap() || !verifier.should_retry {
        return false;
    }
    let active_text_rewrite = route_result.is_chat_gate()
        && route_result
            .route_reason
            .contains("active_text_followup_route_repair")
        && journal
            .context_bundle_summary
            .as_deref()
            .is_some_and(|summary| summary.contains("Most recent generated output:"));
    let pure_chat_agent_loop = route_result
        .route_reason
        .contains("pure_chat_agent_loop_submode")
        && !route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && !route_result.wants_file_delivery
        && journal.step_results.is_empty();
    active_text_rewrite || pure_chat_agent_loop
}

pub(super) async fn retry_direct_chat_answer_after_verifier(
    state: &AppState,
    task: &crate::ClaimedTask,
    user_request: &str,
    journal: &crate::task_journal::TaskJournal,
    rejected_answer: &str,
    verifier: &crate::answer_verifier::AnswerVerifierOut,
) -> Option<String> {
    let context = journal
        .context_bundle_summary
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("<none>");
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_request);
    let prompt = format!(
        "You are rewriting a direct chat answer after answer verification failed.\n\nRequest language hint: {request_language_hint}\nConfigured fallback language: {}\n\nCurrent user request:\n{}\n\nCurrent task context:\n{}\n\nRejected answer:\n{}\n\nVerifier reason:\n{}\n\nVerifier retry instruction:\n{}\n\nReturn only the corrected final answer. Use the request language. Preserve the most recent generated output's factual scope and evidence boundary. Do not add new setup categories, project-doc references, support/contact recommendations, usage claims, paths, commands, config keys, credentials, callbacks, or verification steps unless they are present in Current task context.",
        state.policy.command_intent.default_locale,
        user_request.trim(),
        context,
        rejected_answer.trim(),
        verifier.answer_incomplete_reason.trim(),
        verifier.retry_instruction.trim(),
    );
    const PROMPT_SOURCE: &str = "inline:answer_verifier_direct_chat_retry";
    crate::log_prompt_render(
        state,
        &task.task_id,
        "answer_verifier_direct_chat_retry",
        PROMPT_SOURCE,
        None,
    );
    match crate::llm_gateway::run_with_fallback_with_prompt_source(
        state,
        task,
        &prompt,
        PROMPT_SOURCE,
    )
    .await
    {
        Ok(answer) => {
            let answer = answer.trim().to_string();
            (!answer.is_empty()).then_some(answer)
        }
        Err(err) => {
            warn!(
                "answer_verifier_direct_chat_retry_failed task_id={} err={}",
                task.task_id, err
            );
            None
        }
    }
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

pub(super) fn resume_failure_is_structured_service_status_result(
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

pub(super) fn resume_failure_execution_failed_step_answer(
    route_result: &crate::RouteResult,
    resume_ctx: &Value,
    _prefer_english: bool,
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

    let mut payload = serde_json::json!({
        "message_key": "clawd.msg.execution.failed_step",
        "reason_code": "execution_failed_step",
        "action": action,
        "detail": detail,
    });
    if let Some(command) = command {
        payload["command"] = serde_json::json!(command);
    }
    if let Some(exit_code) = exit_code {
        payload["exit_code"] = serde_json::json!(exit_code);
    }
    Some(payload.to_string())
}

fn resume_context_user_visible_step_error(error: &str) -> String {
    crate::skills::parse_structured_skill_error(error)
        .map(|structured| crate::skills::normalize_skill_error_for_user(&structured.skill, error))
        .unwrap_or_else(|| error.to_string())
}

pub(super) fn resume_context_execution_summary_messages(
    resume_ctx: &Value,
    _prefer_english: bool,
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
    vec![serde_json::json!({
        "message_key": "clawd.msg.execution.summary",
        "reason_code": "resume_failed_step_summary",
        "step_index": 1,
        "action": action,
        "error": crate::truncate_for_agent_trace(&error).replace("```", "'''"),
    })
    .to_string()]
}
