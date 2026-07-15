use crate::agent_engine::{AgentRunContext, LoopState};
use crate::finalize::build_from_loop_state as build_loop_journal;
use crate::{AppState, AskReply, ClaimedTask};

use super::{
    direct_scalar_observed_answer, execution_summary_arg_is_sensitive,
    latest_tail_read_range_observed_answer, plan_step_for_execution,
    prefer_english_for_agent_contextual_user_text, route_prefers_observed_answer,
    route_requires_content_evidence, route_resolved_intent, truncate_with_ellipsis,
};

#[cfg(test)]
#[path = "loop_reply_content_evidence_failure_tests.rs"]
mod tests;

fn error_looks_like_os_permission_denied(error: &str) -> bool {
    crate::skills::error_looks_like_os_permission_denied(error)
}

fn error_looks_like_missing_file_or_directory(error: &str) -> bool {
    if let Some(structured) = crate::skills::parse_structured_skill_error(error) {
        return structured.error_kind == "not_found";
    }
    error.trim().starts_with("__RC_READ_FILE_NOT_FOUND__:")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceStatusFailureObservation {
    UnitNotFound,
    Inactive,
    Failed,
}

impl ServiceStatusFailureObservation {
    fn status_code(self) -> &'static str {
        match self {
            Self::UnitNotFound => "service_unit_not_found",
            Self::Inactive => "service_inactive",
            Self::Failed => "service_failed",
        }
    }
}

fn route_is_service_status(agent_run_context: Option<&AgentRunContext>) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    crate::finalize::route_matches_service_control_machine_summary(route)
}

fn service_status_observation_from_error(error: &str) -> Option<ServiceStatusFailureObservation> {
    if let Some(structured) = crate::skills::parse_structured_skill_error(error) {
        return match structured.error_kind.as_str() {
            "not_found" => Some(ServiceStatusFailureObservation::UnitNotFound),
            "service_inactive" => Some(ServiceStatusFailureObservation::Inactive),
            "service_failed" | "service_control_failed" => {
                Some(ServiceStatusFailureObservation::Failed)
            }
            _ => None,
        };
    }
    None
}

fn extract_systemd_unit_from_error(error: &str) -> Option<String> {
    let _ = error;
    None
}

fn service_status_target_label(error: &str, agent_run_context: Option<&AgentRunContext>) -> String {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.output_contract.locator_hint.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            crate::skills::parse_structured_skill_error(error)
                .and_then(|structured| structured.service_name)
        })
        .or_else(|| extract_systemd_unit_from_error(error))
        .unwrap_or_else(|| "requested service".to_string())
}

fn service_status_failure_answer(
    _state: &AppState,
    _user_text: &str,
    error: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    if !route_is_service_status(agent_run_context) {
        return None;
    }
    let observation = service_status_observation_from_error(error)?;
    let target = service_status_target_label(error, agent_run_context);
    Some(service_status_failure_envelope(error, &target, observation))
}

fn service_status_failure_envelope(
    error: &str,
    target: &str,
    observation: ServiceStatusFailureObservation,
) -> String {
    let structured = crate::skills::parse_structured_skill_error(error);
    let mut envelope = serde_json::json!({
        "message_key": "service.status.failure",
        "status_code": observation.status_code(),
        "error_kind": structured
            .as_ref()
            .map(|value| value.error_kind.as_str())
            .unwrap_or(observation.status_code()),
        "target": target,
        "source": "service_control"
    });
    if let Some(manager_type) = structured
        .as_ref()
        .and_then(|value| value.manager_type.as_deref())
    {
        envelope["manager_type"] = serde_json::Value::String(manager_type.to_string());
    }
    if let Some(service_name) = structured
        .as_ref()
        .and_then(|value| value.service_name.as_deref())
    {
        envelope["service_name"] = serde_json::Value::String(service_name.to_string());
    }
    serde_json::to_string(&envelope).unwrap_or_else(|_| {
        "message_key=service.status.failure status_code=service_status_failure".to_string()
    })
}

fn crypto_account_access_failure_answer(
    state: &AppState,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
    failed_step: &crate::executor::StepExecutionResult,
    raw_error: &str,
) -> Option<String> {
    if !crate::skills::is_crypto_account_access_error(&failed_step.skill, raw_error) {
        return None;
    }
    let prefer_english =
        prefer_english_for_agent_contextual_user_text(state, user_text, agent_run_context);
    Some(crate::bilingual_t_with_default_vars(
        state,
        "crypto.err.account_access_failed",
        "crypto.err.account_access_failed",
        "crypto.err.account_access_failed",
        prefer_english,
        &[],
    ))
}

fn crypto_recoverable_i18n_failure_answer(
    state: &AppState,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
    failed_step: &crate::executor::StepExecutionResult,
    raw_error: &str,
) -> Option<String> {
    let key = crate::skills::crypto_recoverable_i18n_error_key(&failed_step.skill, raw_error)?;
    let prefer_english =
        prefer_english_for_agent_contextual_user_text(state, user_text, agent_run_context);
    Some(crate::bilingual_t_with_default_vars(
        state,
        &key,
        &key,
        &key,
        prefer_english,
        &[],
    ))
}

pub(super) fn structured_extra_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| crate::truncate_for_agent_trace(&compact_observed_stream(value)))
}

fn content_evidence_failed_step_locator(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    failed_step: &crate::executor::StepExecutionResult,
) -> Option<String> {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.output_contract.locator_hint.trim())
        .filter(|locator| !locator.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            plan_step_for_execution(loop_state, failed_step)
                .and_then(|plan_step| structured_target_label_from_args(&plan_step.args))
        })
        .or_else(|| structured_target_label_from_step_error(failed_step))
}

fn structured_target_label_from_step_error(
    failed_step: &crate::executor::StepExecutionResult,
) -> Option<String> {
    let error = failed_step.error.as_deref()?.trim();
    let structured = crate::skills::parse_structured_skill_error(error)?;
    structured
        .extra
        .as_ref()
        .and_then(structured_target_label_from_args)
        .or(structured.service_name)
}

fn structured_target_label_from_args(args: &serde_json::Value) -> Option<String> {
    let object = args.as_object()?;
    for key in [
        "path",
        "resolved_path",
        "file_path",
        "target_path",
        "dir",
        "directory",
        "root",
        "service_name",
        "unit",
        "target",
        "name",
    ] {
        if execution_summary_arg_is_sensitive(key) {
            continue;
        }
        if let Some(label) = object
            .get(key)
            .and_then(structured_target_label_from_value)
            .map(|value| truncate_with_ellipsis(&value, 180))
        {
            return Some(label);
        }
    }
    None
}

fn structured_target_label_from_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        }
        serde_json::Value::Array(items) => {
            let labels = items
                .iter()
                .filter_map(structured_target_label_from_value)
                .take(3)
                .collect::<Vec<_>>();
            (!labels.is_empty()).then(|| labels.join(", "))
        }
        serde_json::Value::Object(_) => structured_target_label_from_args(value),
        _ => None,
    }
}

fn structured_failure_is_publishable_user_result(
    failed_step: &crate::executor::StepExecutionResult,
    raw_error: &str,
) -> bool {
    let Some(structured) = crate::skills::parse_structured_skill_error(raw_error) else {
        return false;
    };
    let effective_skill = if structured.skill.trim().is_empty() {
        failed_step.skill.as_str()
    } else {
        structured.skill.as_str()
    };
    if effective_skill.eq_ignore_ascii_case("db_basic") {
        return matches!(
            structured.error_kind.as_str(),
            "sqlite_open_failed"
                | "sqlite_query_failed"
                | "sqlite_execute_failed"
                | "unsafe_sql"
                | "confirmation_required"
                | "invalid_input"
                | "unsupported_action"
        );
    }
    matches!(structured.error_kind.as_str(), "contract_action_rejected")
}

fn push_structured_error_facts(observed_facts: &mut Vec<String>, raw_error: &str) {
    let Some(structured) = crate::skills::parse_structured_skill_error(raw_error) else {
        return;
    };
    if !structured.error_kind.trim().is_empty() {
        observed_facts.push(format!("error_kind: {}", structured.error_kind.trim()));
    }
    if !structured.skill.trim().is_empty() {
        observed_facts.push(format!("structured_skill: {}", structured.skill.trim()));
    }
    if let Some(extra) = structured.extra.as_ref() {
        for key in ["exit_code", "stderr", "stdout", "output_truncated"] {
            if let Some(value) = extra.get(key) {
                let value_text = value
                    .as_str()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| value.to_string());
                observed_facts.push(format!(
                    "{key}: {}",
                    crate::truncate_for_agent_trace(&compact_observed_stream(&value_text))
                ));
            }
        }
    }
}

fn machine_fallback_from_observed_facts(reason_code: &str, observed_facts: &[String]) -> String {
    let mut lines = vec![format!("reason_code={}", reason_code.trim())];
    for fact in observed_facts {
        let fact = fact.trim();
        if fact.is_empty() {
            continue;
        }
        let Some((key, value)) = fact.split_once(':') else {
            lines.push(fact.replace(' ', "_"));
            continue;
        };
        let key = key.trim().replace(' ', "_");
        let value = value.trim();
        if key == "locator" {
            lines.push(format!("{key}=`{value}`"));
        } else {
            lines.push(format!("{key}={value}"));
        }
    }
    lines.join("\n")
}

fn compact_observed_stream(text: &str) -> String {
    let compact = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" | ");
    if compact.is_empty() {
        text.trim().to_string()
    } else {
        compact
    }
}

fn missing_content_target_label(
    agent_run_context: Option<&AgentRunContext>,
    error: &str,
) -> String {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.output_contract.locator_hint.trim())
        .filter(|locator| !locator.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            error
                .trim()
                .strip_prefix("__RC_READ_FILE_NOT_FOUND__:")
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| "requested target".to_string())
}

pub(super) fn content_evidence_missing_target_answer(
    state: &AppState,
    _task: &ClaimedTask,
    _user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
    error: &str,
) -> String {
    let target = missing_content_target_label(agent_run_context, error);
    crate::i18n_t_with_default_vars(
        state,
        "clawd.msg.content_missing_target",
        "message_key=clawd.msg.content_missing_target target={target} content_read=false",
        &[("target", &target)],
    )
}

pub(super) async fn content_evidence_step_failure_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !route_requires_content_evidence(agent_run_context) {
        return None;
    }
    if loop_state.executed_step_results.iter().any(|step| {
        step.is_ok()
            && !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think"
            )
            && step
                .output
                .as_deref()
                .map(str::trim)
                .is_some_and(|output| !output.is_empty())
    }) {
        return None;
    }

    let failed_step = loop_state.executed_step_results.iter().rev().find(|step| {
        !step.is_ok()
            && !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think"
            )
    })?;
    let raw_error = failed_step.error.as_deref().map(str::trim)?;
    if raw_error.is_empty() {
        return None;
    }
    let recoverable_skill_error =
        crate::skills::is_recoverable_skill_error(&failed_step.skill, raw_error);
    let observable_run_cmd_error =
        crate::skills::is_observable_run_cmd_error(&failed_step.skill, raw_error);
    let error_observation = if crate::skills::parse_structured_skill_error(raw_error).is_some()
        || recoverable_skill_error
        || observable_run_cmd_error
    {
        crate::skills::skill_error_machine_observation(&failed_step.skill, raw_error)
            .unwrap_or_else(|| raw_error.to_string())
    } else {
        raw_error.to_string()
    };
    let error = error_observation.as_str();

    if let Some(answer) =
        service_status_failure_answer(state, user_text, raw_error, agent_run_context)
    {
        return Some((
            answer,
            crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                contract_ok: true,
                completion_ok: Some(true),
                grounded_ok: Some(true),
                format_ok: Some(true),
                needs_clarify: Some(false),
                used_evidence_ids_count: 1,
                ..Default::default()
            },
        ));
    }

    if let Some(answer) = crypto_account_access_failure_answer(
        state,
        user_text,
        agent_run_context,
        failed_step,
        raw_error,
    ) {
        return Some((
            answer,
            crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                contract_ok: true,
                completion_ok: Some(true),
                grounded_ok: Some(true),
                format_ok: Some(true),
                needs_clarify: Some(false),
                used_evidence_ids_count: 1,
                ..Default::default()
            },
        ));
    }

    if let Some(answer) = crypto_recoverable_i18n_failure_answer(
        state,
        user_text,
        agent_run_context,
        failed_step,
        raw_error,
    ) {
        return Some((
            answer,
            crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                contract_ok: true,
                completion_ok: Some(true),
                grounded_ok: Some(true),
                format_ok: Some(true),
                needs_clarify: Some(false),
                used_evidence_ids_count: 1,
                ..Default::default()
            },
        ));
    }

    let missing_target = error_looks_like_missing_file_or_directory(raw_error);
    if missing_target {
        let answer = content_evidence_missing_target_answer(
            state,
            task,
            user_text,
            agent_run_context,
            raw_error,
        );
        return Some((
            answer,
            crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                contract_ok: true,
                completion_ok: Some(true),
                grounded_ok: Some(true),
                format_ok: Some(true),
                needs_clarify: Some(false),
                used_evidence_ids_count: 1,
                ..Default::default()
            },
        ));
    }

    let permission_denied = error_looks_like_os_permission_denied(raw_error);
    let publishable_observed_failure = permission_denied
        || recoverable_skill_error
        || observable_run_cmd_error
        || structured_failure_is_publishable_user_result(failed_step, raw_error);
    let locator = content_evidence_failed_step_locator(loop_state, agent_run_context, failed_step);
    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let mut observed_facts = vec![
        format!("failed_skill: {}", failed_step.skill.trim()),
        format!(
            "error_observation: {}",
            crate::truncate_for_agent_trace(error)
        ),
        "content_evidence_observed: false".to_string(),
    ];
    if let Some(locator) = locator.as_deref() {
        observed_facts.push(format!("locator: {locator}"));
    }
    push_structured_error_facts(&mut observed_facts, raw_error);
    if permission_denied {
        observed_facts.push("os_permission_denied: true".to_string());
        observed_facts.push("clawd_process_lacks_sudo_or_root_permission: true".to_string());
    }
    if recoverable_skill_error {
        observed_facts.push("recoverable_skill_error: true".to_string());
    }
    if observable_run_cmd_error {
        observed_facts.push("observable_run_cmd_error: true".to_string());
    }
    let mut policy_boundary = vec![
        "content_read_claim_allowed=false".to_string(),
        "content_summary_claim_allowed=false".to_string(),
        "expose_internal_details=false".to_string(),
        "response_scope=observed_execution_failure_and_recovery_path".to_string(),
    ];
    if permission_denied {
        policy_boundary.push("process_privilege_status=lacks_sudo_or_root".to_string());
    }
    let reason_code = if permission_denied {
        "content_evidence_step_permission_denied"
    } else {
        "content_evidence_step_failed"
    };
    let contract = crate::fallback::UserResponseContract::tool_failure(
        reason_code,
        user_text,
        &route_resolved_intent(agent_run_context),
        observed_facts.clone(),
        policy_boundary,
        "brief_failure_with_next_step",
        &language_hint,
    );
    let default_answer = machine_fallback_from_observed_facts(reason_code, &observed_facts);
    let answer = crate::fallback::compose_user_response_from_contract_with_default(
        state,
        task,
        &contract,
        crate::fallback::ClarifyFallbackSource::ExecutionFailedPartial,
        &default_answer,
    )
    .await;
    Some((
        answer,
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(if publishable_observed_failure {
                crate::finalize::FinalizerDisposition::QualifiedCompletion
            } else {
                crate::finalize::FinalizerDisposition::AllowFallback
            }),
            contract_ok: true,
            completion_ok: Some(publishable_observed_failure),
            grounded_ok: Some(true),
            format_ok: Some(true),
            needs_clarify: Some(false),
            used_evidence_ids_count: 1,
            ..Default::default()
        },
    ))
}

pub(super) async fn content_evidence_step_failure_reply_from_loop(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<AskReply> {
    if latest_tail_read_range_observed_answer(state, task, user_text, loop_state, agent_run_context)
        .is_some()
    {
        return None;
    }
    if agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(route_prefers_observed_answer)
        && direct_scalar_observed_answer(Some(state), loop_state, agent_run_context).is_some()
    {
        return None;
    }
    let (error_answer, summary) =
        content_evidence_step_failure_answer(state, task, user_text, loop_state, agent_run_context)
            .await?;
    let delivery_messages = vec![error_answer.clone()];
    let delivery_consistent =
        crate::task_journal::delivery_payload_consistent(&error_answer, &delivery_messages);
    let should_fail = !matches!(
        summary.disposition,
        Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
    ) || summary.completion_ok == Some(false);
    let final_status = if should_fail {
        crate::task_journal::TaskJournalFinalStatus::Failure
    } else {
        crate::task_journal::TaskJournalFinalStatus::Success
    };
    let journal = build_loop_journal(
        task,
        user_text,
        loop_state,
        agent_run_context,
        Some(summary),
        delivery_consistent,
        &error_answer,
        final_status,
    );
    let reply = AskReply::non_llm(error_answer.clone())
        .with_messages(delivery_messages)
        .with_task_journal(journal);
    Some(if should_fail {
        reply.with_failure(error_answer)
    } else {
        reply
    })
}

#[cfg(test)]
pub(super) fn content_evidence_failure_suppresses_execution_summary(
    loop_state: &LoopState,
) -> bool {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .find(|step| {
            !step.is_ok()
                && !matches!(
                    step.skill.as_str(),
                    "respond" | "synthesize_answer" | "think"
                )
        })
        .and_then(|step| {
            step.error
                .as_deref()
                .map(str::trim)
                .filter(|error| !error.is_empty())
                .map(|error| {
                    error_looks_like_os_permission_denied(error)
                        || error_looks_like_missing_file_or_directory(error)
                        || crate::skills::is_observable_run_cmd_error(&step.skill, error)
                })
        })
        .unwrap_or(false)
}
