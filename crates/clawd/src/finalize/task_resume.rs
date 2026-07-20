use std::path::Path;

use serde_json::{json, Value};
use tracing::warn;

use crate::AppState;

use super::task_content_evidence_delivery::route_has_file_delivery_contract;

const ANSWER_VERIFIER_RETRY_TRACE_MAX_CHARS: usize = 64_000;
const ANSWER_VERIFIER_RETRY_PROMPT_LOGICAL_PATH: &str = "prompts/answer_verifier_retry_prompt.md";

fn resume_context_body(value: &Value) -> &Value {
    value.get("resume_context").unwrap_or(value)
}

pub(super) fn text_looks_like_missing_file_target(text: &str) -> bool {
    let trimmed = text.trim();
    crate::skills::read_file_not_found_path(trimmed).is_some()
        || crate::skills::parse_structured_skill_error(trimmed)
            .is_some_and(|structured| structured.error_kind == "not_found")
}

fn machine_token(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || !trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        return None;
    }
    Some(trimmed)
}

fn resume_context_has_remaining_actions(resume_ctx: &Value) -> bool {
    resume_context_body(resume_ctx)
        .get("remaining_actions")
        .and_then(|value| value.as_array())
        .is_some_and(|actions| !actions.is_empty())
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

fn structured_error_extra_string<'a>(
    error: &'a crate::skills::StructuredSkillError,
    key: &str,
) -> Option<&'a str> {
    error
        .extra
        .as_ref()
        .and_then(|extra| extra.get(key))
        .and_then(|value| value.as_str())
        .and_then(machine_token)
}

fn structured_error_has_machine_token(
    error: &crate::skills::StructuredSkillError,
    key: &str,
    allowed: &[&str],
) -> bool {
    structured_error_extra_string(error, key).is_some_and(|value| {
        allowed
            .iter()
            .any(|allowed_value| value.eq_ignore_ascii_case(allowed_value))
    })
}

fn structured_error_is_directory_lookup_failure(
    error: &crate::skills::StructuredSkillError,
) -> bool {
    matches!(
        error.error_kind.as_str(),
        "read_dir_failed" | "directory_not_found" | "directory_lookup_failed"
    ) || structured_error_has_machine_token(
        error,
        "reason_code",
        &[
            "read_dir_failed",
            "directory_not_found",
            "directory_lookup_failed",
        ],
    ) || structured_error_has_machine_token(
        error,
        "error_code",
        &[
            "read_dir_failed",
            "directory_not_found",
            "directory_lookup_failed",
        ],
    )
}

fn structured_error_is_missing_target(error: &crate::skills::StructuredSkillError) -> bool {
    error.error_kind == "not_found"
        || structured_error_has_machine_token(error, "reason_code", &["not_found", "missing"])
        || structured_error_has_machine_token(error, "error_code", &["not_found", "missing"])
}

pub(super) fn resume_context_has_directory_lookup_failure(resume_ctx: &Value) -> bool {
    resume_context_failed_structured_skill_error(resume_ctx)
        .as_ref()
        .is_some_and(structured_error_is_directory_lookup_failure)
}

pub(super) fn resume_failure_is_unbound_path_lookup_clarify_result(
    route_result: &crate::IntentOutputContract,
    resume_ctx: &Value,
) -> bool {
    let contract = route_result.clone();
    contract.requires_content_evidence
        && !route_has_file_delivery_contract(route_result)
        && !resume_context_has_remaining_actions(resume_ctx)
        && !matches!(
            contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
        && (route_result.does_not_request_exact_command_output()
            || route_result.requests_exact_path_list())
        && (resume_context_path_batch_facts_are_missing_only(resume_ctx)
            || resume_context_has_directory_lookup_failure(resume_ctx))
}

pub(super) fn resume_failure_is_missing_file_delivery_result(
    route_result: &crate::IntentOutputContract,
    resume_ctx: &Value,
) -> bool {
    route_has_file_delivery_contract(route_result)
        && !resume_context_has_remaining_actions(resume_ctx)
        && (resume_context_path_batch_facts_are_missing_only(resume_ctx)
            || resume_context_failed_structured_skill_error(resume_ctx)
                .as_ref()
                .is_some_and(structured_error_is_missing_target))
}

fn resume_context_failed_structured_skill_error(
    resume_ctx: &Value,
) -> Option<crate::skills::StructuredSkillError> {
    resume_context_body(resume_ctx)
        .get("failed_step")
        .and_then(|step| step.get("structured_error"))
        .and_then(resume_context_structured_skill_error_from_value)
}

pub(super) fn answer_verifier_retry_applicable(
    route_result: &crate::IntentOutputContract,
    journal: &crate::task_journal::TaskJournal,
    verifier: &crate::answer_verifier::AnswerVerifierOut,
) -> bool {
    if !verifier.high_confidence_gap() || !verifier.should_retry {
        return false;
    }
    let direct_answer_without_tool_observation = !route_result.requires_content_evidence
        && !route_result.delivery_required
        && journal.step_results.is_empty();
    let observed_tool_evidence = !route_result.delivery_required
        && journal_has_successful_non_terminal_step(journal)
        && !journal_has_failed_non_terminal_step(journal);
    direct_answer_without_tool_observation || observed_tool_evidence
}

pub(crate) fn answer_verifier_retry_answer_has_required_machine_evidence(
    journal: Option<&crate::task_journal::TaskJournal>,
    answer: &str,
) -> bool {
    let Some(requirement) = local_code_answer_machine_evidence_requirement(answer) else {
        return true;
    };
    let Some(journal) = journal else {
        return false;
    };
    if !journal_has_code_or_test_artifact_step(journal) {
        return false;
    }
    !requirement.requires_validation_signal
        || journal.step_results.iter().any(step_has_validation_signal)
}

#[derive(Debug, Clone, Copy)]
struct LocalCodeAnswerEvidenceRequirement {
    requires_validation_signal: bool,
}

fn local_code_answer_machine_evidence_requirement(
    answer: &str,
) -> Option<LocalCodeAnswerEvidenceRequirement> {
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(answer.trim()) else {
        return None;
    };
    if object.is_empty() || !object.keys().all(|key| local_code_json_key(key)) {
        return None;
    }
    let has_local_code_result_field = object.keys().any(|key| {
        matches!(
            key.as_str(),
            "created_files"
                | "changed_files"
                | "test_command"
                | "test_status"
                | "functions"
                | "error_codes"
                | "evidence_files"
                | "project_dir"
        )
    });
    if !has_local_code_result_field {
        return None;
    }
    Some(LocalCodeAnswerEvidenceRequirement {
        requires_validation_signal: object.contains_key("test_status")
            || object.contains_key("test_command"),
    })
}

fn journal_has_code_or_test_artifact_step(journal: &crate::task_journal::TaskJournal) -> bool {
    journal.step_results.iter().any(|step| {
        if step.status != crate::executor::StepExecutionStatus::Ok {
            return false;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            return false;
        };
        let Ok(value) = serde_json::from_str::<Value>(output.trim()) else {
            return false;
        };
        let extra = value
            .get("extra")
            .filter(|extra| extra.is_object())
            .unwrap_or(&value);
        let action = extra
            .get("action")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default();
        if !matches!(
            action,
            "write_text" | "append_text" | "read_range" | "read_text_range" | "grep_text"
        ) {
            return false;
        }
        ["resolved_path", "effective_path", "path"]
            .iter()
            .find_map(|field| extra.get(*field).and_then(Value::as_str))
            .is_some_and(path_looks_like_code_or_test)
    })
}

fn step_has_validation_signal(step: &crate::task_journal::TaskJournalStepTrace) -> bool {
    if step.status != crate::executor::StepExecutionStatus::Ok {
        return false;
    }
    step.output_excerpt
        .as_deref()
        .and_then(|text| serde_json::from_str::<Value>(text.trim()).ok())
        .is_some_and(output_has_validation_payload)
}

fn output_has_validation_payload(value: Value) -> bool {
    value
        .get("validation_result")
        .or_else(|| value.get("validation"))
        .is_some_and(|validation| {
            validation
                .get("status")
                .or_else(|| validation.get("status_code"))
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|status| !status.is_empty())
        })
}

fn local_code_json_key(key: &str) -> bool {
    matches!(
        key,
        "created_files"
            | "changed_files"
            | "test_command"
            | "test_status"
            | "functions"
            | "error_codes"
            | "evidence_files"
            | "project_dir"
    )
}

fn path_looks_like_code_or_test(path: &str) -> bool {
    let normalized = path.trim().replace('\\', "/");
    let extension = Path::new(&normalized)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase());
    let Some(extension) = extension else {
        return false;
    };
    matches!(
        extension.as_str(),
        "py" | "rs"
            | "js"
            | "jsx"
            | "ts"
            | "tsx"
            | "go"
            | "java"
            | "c"
            | "h"
            | "cc"
            | "cpp"
            | "hpp"
            | "cs"
            | "rb"
            | "php"
            | "swift"
            | "kt"
            | "scala"
            | "sh"
    )
}

fn journal_has_successful_non_terminal_step(journal: &crate::task_journal::TaskJournal) -> bool {
    journal.step_results.iter().any(|step| {
        step.status == crate::executor::StepExecutionStatus::Ok
            && !step_is_terminal_planner_action(step.skill.as_str())
    })
}

fn journal_has_failed_non_terminal_step(journal: &crate::task_journal::TaskJournal) -> bool {
    journal.step_results.iter().any(|step| {
        step.status != crate::executor::StepExecutionStatus::Ok
            && !step_is_terminal_planner_action(step.skill.as_str())
    })
}

fn step_is_terminal_planner_action(skill: &str) -> bool {
    matches!(
        skill,
        "respond" | "synthesize_answer" | "think" | "answer_verifier"
    )
}

pub(super) fn answer_verifier_retry_observed_trace(
    journal: &crate::task_journal::TaskJournal,
) -> String {
    let trace = journal.to_trace_json();
    let compact = json!({
        "step_results": trace.get("step_results").cloned().unwrap_or_else(|| json!([])),
        "task_observations": trace.get("task_observations").cloned().unwrap_or_else(|| json!([])),
        "evidence_coverage": trace.get("evidence_coverage").cloned().unwrap_or(Value::Null),
        "finalizer_summary": trace.get("finalizer_summary").cloned().unwrap_or(Value::Null),
    });
    let serialized = serde_json::to_string(&compact).unwrap_or_else(|_| "{}".to_string());
    if serialized.len() <= ANSWER_VERIFIER_RETRY_TRACE_MAX_CHARS {
        return serialized;
    }
    let mut out =
        crate::utf8_safe_prefix(&serialized, ANSWER_VERIFIER_RETRY_TRACE_MAX_CHARS).to_string();
    out.push_str("...(truncated)");
    out
}

pub(super) fn bounded_answer_retry_prompt(
    template: &str,
    fallback_locale: &str,
    request_language_hint: &str,
    user_request: &str,
    output_contract: &crate::IntentOutputContract,
    context: &str,
    observed_trace: &str,
    rejected_answer: &str,
    verifier: &crate::answer_verifier::AnswerVerifierOut,
) -> String {
    let issue = json!({
        "issue_kind": "answer_contract_gap",
        "missing_evidence_fields": verifier.missing_evidence_fields,
        "should_retry": verifier.should_retry,
        "confidence": verifier.confidence,
    });
    let output_contract =
        serde_json::to_string(output_contract).unwrap_or_else(|_| "{}".to_string());
    crate::render_prompt_template(
        template,
        &[
            ("__REQUEST_LANGUAGE_HINT__", request_language_hint),
            ("__FALLBACK_LOCALE__", fallback_locale),
            ("__USER_REQUEST__", user_request.trim()),
            ("__OUTPUT_CONTRACT__", &output_contract),
            ("__VERIFIER_ISSUE__", &issue.to_string()),
            ("__TASK_CONTEXT__", context),
            ("__OBSERVED_TRACE__", observed_trace),
            ("__REJECTED_ANSWER__", rejected_answer.trim()),
        ],
    )
}

pub(super) async fn retry_answer_after_verifier(
    state: &AppState,
    task: &crate::ClaimedTask,
    user_request: &str,
    output_contract: &crate::IntentOutputContract,
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
    let observed_trace = answer_verifier_retry_observed_trace(journal);
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_request);
    let resolved = match crate::bootstrap::load_required_prompt_template_for_state_with_meta(
        state,
        ANSWER_VERIFIER_RETRY_PROMPT_LOGICAL_PATH,
    ) {
        Ok(resolved) => resolved,
        Err(err) => {
            warn!(
                "answer_verifier_retry_prompt_unavailable task_id={} err={}",
                task.task_id, err
            );
            return None;
        }
    };
    let prompt = bounded_answer_retry_prompt(
        &resolved.template,
        &state.policy.command_intent.default_locale,
        &request_language_hint,
        user_request,
        output_contract,
        context,
        &observed_trace,
        rejected_answer,
        verifier,
    );
    crate::log_prompt_render(
        state,
        &task.task_id,
        "answer_verifier_retry",
        &resolved.source,
        None,
    );
    match crate::llm_gateway::run_with_fallback_with_prompt_source(
        state,
        task,
        &prompt,
        &resolved.source,
    )
    .await
    {
        Ok(answer) => {
            let answer = answer.trim().to_string();
            (!answer.is_empty()).then_some(answer)
        }
        Err(err) => {
            warn!(
                "answer_verifier_retry_failed task_id={} err={}",
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
        error_text: String::new(),
        platform: resume_context_string_field(value, "platform"),
        manager_type: resume_context_string_field(value, "manager_type"),
        service_name: resume_context_string_field(value, "service_name"),
        extra: value.get("extra").cloned().filter(|value| !value.is_null()),
    })
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

fn resume_context_user_visible_step_error(error: &str) -> String {
    crate::skills::parse_structured_skill_error(error)
        .and_then(|structured| {
            crate::skills::skill_error_machine_observation(&structured.skill, error)
        })
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
    let mut payload = serde_json::json!({
        "message_key": "clawd.msg.execution.summary",
        "reason_code": "resume_failed_step_summary",
        "step_index": 1,
        "action": action,
    });
    if let Some(structured) = resume_context_failed_structured_skill_error(resume_ctx) {
        payload["skill"] = serde_json::json!(structured.skill);
        payload["error_kind"] = serde_json::json!(structured.error_kind);
        if let Some(command) = resume_context_extra_string(&structured, "command") {
            payload["command"] = serde_json::json!(command);
        }
        if let Some(exit_code) = resume_context_extra_i64(&structured, "exit_code") {
            payload["exit_code"] = serde_json::json!(exit_code);
        }
        if let Some(error_code) = resume_context_extra_string(&structured, "error_code") {
            payload["error_code"] = serde_json::json!(error_code);
        }
        if let Some(status_code) = resume_context_extra_string(&structured, "status_code") {
            payload["status_code"] = serde_json::json!(status_code);
        }
    } else if let Some(error) = failed_step
        .get("error")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let error = resume_context_user_visible_step_error(error);
        payload["error"] =
            serde_json::json!(crate::truncate_for_agent_trace(&error).replace("```", "'''"));
    } else {
        payload["error_kind"] = serde_json::json!("unstructured_failure");
    }
    vec![payload.to_string()]
}
