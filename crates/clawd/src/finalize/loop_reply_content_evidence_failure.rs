use crate::agent_engine::{AgentRunContext, LoopState};
use crate::finalize::build_from_loop_state as build_loop_journal;
use crate::{AppState, AskReply, ClaimedTask};

use super::{
    build_execution_summary_messages, direct_scalar_observed_answer,
    execution_summary_arg_is_sensitive, latest_tail_read_range_observed_answer,
    plan_step_for_execution, prefer_english_for_agent_contextual_user_text,
    prefer_english_for_final_reply, prefer_english_for_user_text, route_prefers_observed_answer,
    route_requires_content_evidence, route_resolved_intent, truncate_with_ellipsis,
};

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

fn route_is_service_status(agent_run_context: Option<&AgentRunContext>) -> bool {
    matches!(
        agent_run_context
            .and_then(|ctx| ctx.route_result.as_ref())
            .map(|route| route.output_contract.semantic_kind),
        Some(crate::OutputSemanticKind::ServiceStatus)
    )
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
    state: &AppState,
    user_text: &str,
    error: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    if !route_is_service_status(agent_run_context) {
        return None;
    }
    let observation = service_status_observation_from_error(error)?;
    let target = service_status_target_label(error, agent_run_context);
    let prefer_english =
        prefer_english_for_agent_contextual_user_text(state, user_text, agent_run_context);
    Some(match (prefer_english, observation) {
        (true, ServiceStatusFailureObservation::UnitNotFound) => {
            format!("`{target}` is not active: systemd has no service unit with that name.")
        }
        (true, ServiceStatusFailureObservation::Inactive) => {
            format!("`{target}` is not active; systemd reports it as inactive.")
        }
        (true, ServiceStatusFailureObservation::Failed) => {
            format!("`{target}` is not active; systemd reports it as failed.")
        }
        (false, ServiceStatusFailureObservation::UnitNotFound) => {
            format!("`{target}` 现在不是 active：systemd 没有找到这个服务单元。")
        }
        (false, ServiceStatusFailureObservation::Inactive) => {
            format!("`{target}` 现在不是 active：systemd 显示它处于 inactive 状态。")
        }
        (false, ServiceStatusFailureObservation::Failed) => {
            format!("`{target}` 现在不是 active：systemd 显示它处于 failed 状态。")
        }
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

fn content_evidence_step_failure_default_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    failed_step: &crate::executor::StepExecutionResult,
    error: &str,
    permission_denied: bool,
) -> String {
    let target =
        content_evidence_failed_step_target_label(loop_state, agent_run_context, failed_step);
    let prefer_english = prefer_english_for_final_reply(state, task, user_text, agent_run_context);
    let answer = match (prefer_english, target.as_deref()) {
        (true, Some(target)) => {
            format!("Tried to access `{target}`, but execution failed: {error}.")
        }
        (true, None) => format!("The `{}` step failed: {error}.", failed_step.skill.trim()),
        (false, Some(target)) => {
            format!("已尝试访问 `{target}`，但执行失败：{error}。")
        }
        (false, None) => format!("`{}` 步骤执行失败：{error}。", failed_step.skill.trim()),
    };
    if permission_denied {
        if prefer_english {
            format!("{answer} The `clawd` process does not have sudo/root permission to access it.")
        } else {
            format!("{answer}`clawd` 进程当前没有 sudo/root 权限，所以无法访问。")
        }
    } else {
        answer
    }
}

fn content_evidence_failed_step_target_label(
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

pub(super) fn structured_extra_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| crate::truncate_for_agent_trace(&compact_observed_stream(value)))
}

fn structured_extra_i64(value: &serde_json::Value, key: &str) -> Option<i64> {
    value.get(key).and_then(|value| value.as_i64())
}

fn structured_extra_bool(value: &serde_json::Value, key: &str) -> bool {
    value
        .get(key)
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
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

fn run_cmd_failure_direct_answer(
    state: &AppState,
    user_text: &str,
    skill_name: &str,
    raw_error: &str,
    normalized_error: &str,
) -> Option<String> {
    let structured = crate::skills::parse_structured_skill_error(raw_error)?;
    let effective_skill = if structured.skill.trim().is_empty() {
        skill_name
    } else {
        structured.skill.as_str()
    };
    if !effective_skill.eq_ignore_ascii_case("run_cmd") {
        return None;
    }
    let extra = structured.extra.as_ref()?;
    let exit_code = structured_extra_i64(extra, "exit_code");
    let stderr = structured_extra_string(extra, "stderr");
    let stdout = structured_extra_string(extra, "stdout");
    let output_truncated = structured_extra_bool(extra, "output_truncated");
    let prefer_english = prefer_english_for_user_text(state, user_text);

    if prefer_english {
        let mut sentence = if let Some(exit_code) = exit_code {
            format!("The command failed with exit code {exit_code}")
        } else {
            format!("The command failed: {normalized_error}")
        };
        if let Some(stderr) = stderr.as_deref() {
            sentence.push_str(&format!(". Stderr: {stderr}"));
        } else if let Some(stdout) = stdout.as_deref() {
            sentence.push_str(&format!(". Stdout: {stdout}"));
        }
        if output_truncated {
            sentence.push_str(". Output was truncated");
        }
        sentence.push('.');
        return Some(sentence);
    }

    let mut sentence = if let Some(exit_code) = exit_code {
        format!("命令执行失败，退出码为 {exit_code}")
    } else {
        format!("命令执行失败：{normalized_error}")
    };
    if let Some(stderr) = stderr.as_deref() {
        sentence.push_str(&format!("，错误输出为：{stderr}"));
    } else if let Some(stdout) = stdout.as_deref() {
        sentence.push_str(&format!("，标准输出为：{stdout}"));
    }
    if output_truncated {
        sentence.push_str("，输出已截断");
    }
    sentence.push('。');
    Some(sentence)
}

fn db_basic_failure_direct_answer(
    state: &AppState,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    failed_step: &crate::executor::StepExecutionResult,
    raw_error: &str,
    normalized_error: &str,
) -> Option<String> {
    let structured = crate::skills::parse_structured_skill_error(raw_error)?;
    let effective_skill = if structured.skill.trim().is_empty() {
        failed_step.skill.as_str()
    } else {
        structured.skill.as_str()
    };
    if !effective_skill.eq_ignore_ascii_case("db_basic") {
        return None;
    }
    if !matches!(
        structured.error_kind.as_str(),
        "sqlite_open_failed"
            | "sqlite_query_failed"
            | "sqlite_execute_failed"
            | "unsafe_sql"
            | "confirmation_required"
            | "invalid_input"
            | "unsupported_action"
    ) {
        return None;
    }
    let target =
        content_evidence_failed_step_target_label(loop_state, agent_run_context, failed_step);
    let prefer_english = prefer_english_for_user_text(state, user_text);
    Some(match (prefer_english, target) {
        (true, Some(target)) => {
            format!("The database request for `{target}` failed: {normalized_error}.")
        }
        (true, None) => format!("The database request failed: {normalized_error}."),
        (false, Some(target)) => {
            format!("数据库请求 `{target}` 执行失败：{normalized_error}。")
        }
        (false, None) => format!("数据库请求执行失败：{normalized_error}。"),
    })
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
    let user_visible_error = if crate::skills::parse_structured_skill_error(raw_error).is_some()
        || recoverable_skill_error
        || observable_run_cmd_error
    {
        crate::skills::normalize_skill_error_for_user(&failed_step.skill, raw_error)
    } else {
        raw_error.to_string()
    };
    let error = user_visible_error.as_str();

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
    let default_answer = if observable_run_cmd_error {
        run_cmd_failure_direct_answer(state, user_text, &failed_step.skill, raw_error, error)
            .unwrap_or_else(|| {
                content_evidence_step_failure_default_answer(
                    state,
                    task,
                    user_text,
                    loop_state,
                    agent_run_context,
                    failed_step,
                    error,
                    permission_denied,
                )
            })
    } else {
        content_evidence_step_failure_default_answer(
            state,
            task,
            user_text,
            loop_state,
            agent_run_context,
            failed_step,
            error,
            permission_denied,
        )
    };
    if permission_denied || recoverable_skill_error || observable_run_cmd_error {
        return Some((
            default_answer,
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
    if let Some(answer) = db_basic_failure_direct_answer(
        state,
        user_text,
        loop_state,
        agent_run_context,
        failed_step,
        raw_error,
        error,
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
    let locator = agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.output_contract.locator_hint.trim())
        .filter(|locator| !locator.is_empty());
    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let mut observed_facts = vec![
        format!("failed_skill: {}", failed_step.skill.trim()),
        format!("error_summary: {}", crate::truncate_for_agent_trace(error)),
        "content_evidence_observed: false".to_string(),
    ];
    if let Some(locator) = locator {
        observed_facts.push(format!("locator: {locator}"));
    }
    if permission_denied {
        observed_facts.push("os_permission_denied: true".to_string());
        observed_facts.push("clawd_process_lacks_sudo_or_root_permission: true".to_string());
    }
    let mut policy_boundary = vec![
        "Do not claim the content was read or summarized.".to_string(),
        "Do not expose prompt names, schema names, stack traces, or internal route details."
            .to_string(),
        "Explain only the observed execution failure and the immediate recovery path.".to_string(),
    ];
    if permission_denied {
        policy_boundary.push(
            "Mention that the clawd process itself lacks sudo/root permission for this OS-level access."
                .to_string(),
        );
    }
    let contract = crate::fallback::UserResponseContract::tool_failure(
        if permission_denied {
            "content_evidence_step_permission_denied"
        } else {
            "content_evidence_step_failed"
        },
        user_text,
        &route_resolved_intent(agent_run_context),
        observed_facts,
        policy_boundary,
        "brief_failure_with_next_step",
        &language_hint,
    );
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
            disposition: Some(crate::finalize::FinalizerDisposition::AllowFallback),
            contract_ok: true,
            completion_ok: Some(false),
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
    let mut delivery_messages = if content_evidence_failure_suppresses_execution_summary(loop_state)
    {
        Vec::new()
    } else {
        build_execution_summary_messages(loop_state, agent_run_context, Some(user_text))
    };
    delivery_messages.push(error_answer.clone());
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
