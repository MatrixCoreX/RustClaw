use std::path::Path;

use serde_json::{json, Value};
use tracing::warn;

use crate::AppState;

use super::task_content_evidence_delivery::route_has_file_delivery_contract;

const ANSWER_VERIFIER_RETRY_TRACE_MAX_CHARS: usize = 64_000;

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
        "operation",
        &["read_dir", "list_dir", "directory_lookup"],
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
        && (route_result.semantic_kind_is_unclassified()
            || route_result.semantic_kind_is_any(&[
                crate::OutputSemanticKind::ExistenceWithPath,
                crate::OutputSemanticKind::FilePaths,
            ]))
        && resume_context_failed_step_skill(resume_ctx).as_deref() == Some("fs_search")
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
    if matches!(step.skill.as_str(), "run_cmd" | "process_basic") {
        return true;
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
        "structured_field_value_projection": answer_verifier_retry_structured_projection(journal),
        "step_results": trace.get("step_results").cloned().unwrap_or_else(|| json!([])),
        "task_observations": trace.get("task_observations").cloned().unwrap_or_else(|| json!([])),
        "evidence_coverage": trace.get("evidence_coverage").cloned().unwrap_or(Value::Null),
        "finalizer_summary": trace.get("finalizer_summary").cloned().unwrap_or(Value::Null),
        "answer_verifier_summary": trace.get("answer_verifier_summary").cloned().unwrap_or(Value::Null),
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

fn answer_verifier_retry_structured_projection(
    journal: &crate::task_journal::TaskJournal,
) -> Value {
    let mut rows = Vec::new();
    for step in journal.step_results.iter().filter(|step| {
        step.status == crate::executor::StepExecutionStatus::Ok
            && !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think" | "answer_verifier"
            )
    }) {
        let Some(evidence) = crate::task_journal::observed_evidence_for_step_trace(step) else {
            continue;
        };
        collect_answer_verifier_retry_projection_rows(step, &evidence, &mut rows);
    }
    Value::Array(rows)
}

fn collect_answer_verifier_retry_projection_rows(
    step: &crate::task_journal::TaskJournalStepTrace,
    evidence: &Value,
    rows: &mut Vec<Value>,
) {
    let Some(items) = evidence.get("items").and_then(Value::as_array) else {
        return;
    };
    for item in items {
        let Some(field) = item.get("field").and_then(Value::as_str) else {
            continue;
        };
        if !answer_verifier_retry_projection_field_allowed(field) {
            continue;
        }
        let Some(value) = answer_verifier_retry_projection_item_value(item) else {
            continue;
        };
        rows.push(json!({
            "step_id": &step.step_id,
            "skill": &step.skill,
            "field": answer_verifier_retry_projection_label(&step.skill, field),
            "value": value,
        }));
    }
}

fn answer_verifier_retry_projection_field_allowed(field: &str) -> bool {
    field.starts_with("extra.field_value.")
        || matches!(
            field,
            "extra.archive"
                | "extra.db_path"
                | "extra.path"
                | "extra.member"
                | "extra.member_path"
                | "extra.content_excerpt"
                | "extra.schema_version"
                | "extra.manager_type"
                | "extra.post_state"
                | "extra.pre_state"
                | "extra.service_name"
                | "extra.status"
                | "extra.summary"
                | "extra.target"
                | "extra.verified"
                | "path"
        )
}

fn answer_verifier_retry_projection_label(skill: &str, field: &str) -> String {
    let domain = skill.strip_suffix("_basic").unwrap_or(skill);
    let field = field
        .strip_prefix("extra.field_value.")
        .or_else(|| field.strip_prefix("extra."))
        .unwrap_or(field);
    format!("{domain}.{field}")
}

fn answer_verifier_retry_projection_item_value(item: &Value) -> Option<String> {
    if let Some(values) = item.get("sample_values").and_then(Value::as_array) {
        let rendered = values
            .iter()
            .filter_map(answer_verifier_retry_projection_scalar)
            .collect::<Vec<_>>();
        if !rendered.is_empty() {
            return Some(rendered.join(", "));
        }
    }
    item.get("excerpt")
        .and_then(answer_verifier_retry_projection_scalar)
        .map(|value| value.replace(['\n', '\r'], " ").trim().to_string())
        .filter(|value| !value.is_empty())
}

fn answer_verifier_retry_projection_scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.trim().to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
    .filter(|value| !value.is_empty())
}

pub(super) async fn retry_answer_after_verifier(
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
    let observed_trace = answer_verifier_retry_observed_trace(journal);
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_request);
    let prompt = format!(
        "You are rewriting a direct chat answer after answer verification failed.\n\nRequest language hint: {request_language_hint}\nConfigured fallback language: {}\n\nCurrent user request:\n{}\n\nCurrent task context:\n{}\n\nObserved task trace JSON:\n{}\n\nRejected answer:\n{}\n\nVerifier reason:\n{}\n\nVerifier retry instruction:\n{}\n\nReturn only the corrected final answer. Use the request language. Use only observed evidence from Current task context and Observed task trace JSON. Do not re-run tools. Preserve the most recent generated output's factual scope and evidence boundary. If Observed task trace JSON contains structured_field_value_projection rows, treat them as the preferred compact field/value evidence for requested tables, summaries, and missing field_value gaps. Render those observed rows in the user's requested visible shape; do not return raw JSON unless the user explicitly requested JSON. If the rejected answer is a JSON object, message_key payload, or other machine-format evidence, treat its fields as evidence to render; do not explain the machine format and do not ask the user to clarify the format. Treat machine status fields such as status, effective_status, result_kind, effective_success, and idempotent_success as authoritative over incidental counters; zero update counters are not a failure when the machine status says ok or effective/idempotent success. If the user asks whether an operation succeeded, result_kind=already_indexed with effective_success=true or idempotent_success=true is a successful completion, not a blocker and not a request for confirmation. If verifier missing_evidence_fields names output_format, keep the observed facts and correct only the visible answer shape. Treat the Verifier retry instruction as a mandatory shape contract: if it requires an exact number of sentences, lines, bullets, items, or words, silently count the corrected answer before returning it and rewrite until the count is exact. Do not return a shorter condensed answer when an exact count is requested. Do not add new setup categories, project-doc references, support/contact recommendations, usage claims, paths, commands, config keys, credentials, callbacks, or verification steps unless they are present in the observed evidence.",
        state.policy.command_intent.default_locale,
        user_request.trim(),
        context,
        observed_trace,
        rejected_answer.trim(),
        verifier.answer_incomplete_reason.trim(),
        verifier.retry_instruction.trim(),
    );
    const PROMPT_SOURCE: &str = "inline:answer_verifier_retry";
    crate::log_prompt_render(
        state,
        &task.task_id,
        "answer_verifier_retry",
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

fn compact_resume_error_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn structured_error_detail(error: &crate::skills::StructuredSkillError) -> Option<String> {
    resume_context_extra_string(error, "stderr")
        .or_else(|| resume_context_extra_string(error, "stdout"))
        .or_else(|| resume_context_extra_string(error, "message_key"))
        .or_else(|| resume_context_extra_string(error, "error_code"))
        .or_else(|| resume_context_extra_string(error, "status_code"))
        .map(compact_resume_error_text)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            (!error.error_kind.trim().is_empty())
                .then(|| format!("error_kind={}", error.error_kind))
        })
}

pub(super) fn resume_failure_execution_failed_step_answer(
    route_result: &crate::IntentOutputContract,
    resume_ctx: &Value,
    _prefer_english: bool,
) -> Option<String> {
    if !route_result.semantic_kind_is(crate::OutputSemanticKind::ExecutionFailedStep) {
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
    let structured = resume_context_failed_structured_skill_error(resume_ctx);
    let command = structured
        .as_ref()
        .and_then(|error| resume_context_extra_string(error, "command"));
    let exit_code = structured
        .as_ref()
        .and_then(|error| resume_context_extra_i64(error, "exit_code"));
    let detail = structured.as_ref().and_then(structured_error_detail)?;

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
