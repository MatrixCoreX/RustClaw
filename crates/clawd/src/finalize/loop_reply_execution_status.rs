use std::path::Path;

use crate::agent_engine::{append_delivery_message, AgentRunContext};
use crate::{AppState, ClaimedTask};

use super::{
    log_deterministic_delivery_record, missing_file_path_from_loop,
    output_excerpt_has_missing_file_evidence, output_text_from_execution_result,
    plan_step_for_execution, planned_delivery_is_publishable_model_language_answer,
    raw_command_arg_from_plan_step, route_prefers_language_rendered_execution_failed_step,
    step_error_has_missing_file_evidence, structured_extra_string, truncate_with_ellipsis,
};

fn observed_execution_status_steps<'a>(
    loop_state: &'a crate::agent_engine::LoopState,
) -> Vec<&'a crate::executor::StepExecutionResult> {
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| {
            !matches!(
                step.skill.as_str(),
                "respond" | "think" | "synthesize_answer"
            ) && output_text_from_execution_result(step).is_some()
                && !step_error_is_contract_policy_gap(step)
        })
        .collect::<Vec<_>>()
}

fn step_error_is_contract_policy_gap(step: &crate::executor::StepExecutionResult) -> bool {
    let Some(error) = step
        .error
        .as_deref()
        .map(str::trim)
        .filter(|error| !error.is_empty())
    else {
        return false;
    };
    crate::skills::parse_structured_skill_error(error).is_some_and(|structured| {
        matches!(
            structured.error_kind.as_str(),
            "contract_action_rejected" | "contract_arg_rejected" | "contract_policy_violation"
        )
    })
}

pub(super) fn successful_content_observation_should_precede_status_summary(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &crate::agent_engine::LoopState,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if !route.output_contract.requires_content_evidence {
        return false;
    }
    if route.output_contract_marker_is_any(&[
        crate::OutputSemanticKind::ExecutionFailedStep,
        crate::OutputSemanticKind::RawCommandOutput,
    ]) || crate::finalize::route_matches_service_status_output_contract(route)
    {
        return false;
    }
    loop_state.executed_step_results.iter().any(|step| {
        step.is_ok()
            && !matches!(
                step.skill.as_str(),
                "respond" | "think" | "synthesize_answer"
            )
            && step
                .output
                .as_deref()
                .map(str::trim)
                .is_some_and(|text| !text.is_empty())
    })
}

pub(super) fn delivery_is_content_answer_candidate(
    agent_run_context: Option<&AgentRunContext>,
    loop_state: &crate::agent_engine::LoopState,
    delivery_messages: &[String],
) -> bool {
    if !successful_content_observation_should_precede_status_summary(agent_run_context, loop_state)
    {
        return false;
    }
    let Some(delivery) = delivery_messages.last().map(String::as_str).map(str::trim) else {
        return false;
    };
    if delivery.is_empty()
        || crate::finalize::is_execution_summary_message(delivery)
        || crate::finalize::looks_like_planner_artifact(delivery)
        || crate::finalize::looks_like_internal_trace_artifact(delivery)
        || crate::agent_engine::observed_output::answer_matches_observed_output_passthrough(
            delivery, loop_state,
        )
    {
        return false;
    }
    planned_delivery_is_publishable_model_language_answer(delivery)
}

pub(super) fn deterministic_observed_execution_status_answer(
    _state: &AppState,
    _user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
) -> Option<String> {
    let steps = observed_execution_status_steps(loop_state);
    if steps.len() < 2 || !steps.iter().any(|step| !step.is_ok()) {
        return None;
    }
    if steps.last().is_some_and(|step| step.is_ok()) {
        return None;
    }

    let mut lines = vec![
        "schema_version=1".to_string(),
        "reason_code=observed_execution_status".to_string(),
    ];
    for (idx, step) in steps.iter().take(6).enumerate() {
        let step_no = idx + 1;
        lines.push(format!("step.{step_no}.skill={}", step.skill.trim()));
        if step.is_ok() {
            lines.push(format!("step.{step_no}.status=ok"));
            continue;
        }
        lines.push(format!("step.{step_no}.status=error"));
        if let Some(error) = output_text_from_execution_result(step) {
            let error = truncate_with_ellipsis(&error.replace('\n', " "), 220);
            lines.push(format!("step.{step_no}.error_summary={error}"));
        }
        push_structured_step_error_machine_facts(&mut lines, step_no, step);
    }
    Some(lines.join("\n"))
}

fn push_structured_step_error_machine_facts(
    lines: &mut Vec<String>,
    step_no: usize,
    step: &crate::executor::StepExecutionResult,
) {
    let Some(error) = step.error.as_deref().map(str::trim) else {
        return;
    };
    let Some(structured) = crate::skills::parse_structured_skill_error(error) else {
        return;
    };
    if !structured.error_kind.trim().is_empty() {
        lines.push(format!(
            "step.{step_no}.error_kind={}",
            structured.error_kind.trim()
        ));
    }
    if let Some(extra) = structured.extra.as_ref() {
        for key in ["exit_code", "stderr", "stdout", "output_truncated"] {
            if let Some(value) = extra.get(key) {
                let value_text = value
                    .as_str()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| value.to_string());
                lines.push(format!(
                    "step.{step_no}.{key}={}",
                    truncate_with_ellipsis(&value_text.replace('\n', " "), 220)
                ));
            }
        }
    }
}

pub(super) fn deterministic_missing_observed_target_answer(
    _state: &AppState,
    _user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let latest_missing_idx = loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .rev()
        .find_map(|(idx, step)| {
            (step
                .output
                .as_deref()
                .is_some_and(output_excerpt_has_missing_file_evidence)
                || step_error_has_missing_file_evidence(step))
            .then_some(idx)
        })?;
    let has_later_successful_observation = loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .skip(latest_missing_idx + 1)
        .any(|(_, step)| {
            step.is_ok()
                && !matches!(
                    step.skill.as_str(),
                    "respond" | "think" | "synthesize_answer"
                )
                && step.output.as_deref().map(str::trim).is_some_and(|output| {
                    !output.is_empty() && !output_excerpt_has_missing_file_evidence(output)
                })
        });
    if has_later_successful_observation {
        return None;
    }
    let path = missing_file_path_from_loop(loop_state, agent_run_context)?;
    let contract_marker = agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(crate::RouteResult::effective_output_contract_semantic_kind);
    let final_answer_shape = agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .and_then(crate::evidence_policy::final_answer_shape_for_route);
    let scalar_count = contract_marker == Some(crate::OutputSemanticKind::ScalarCount);
    let concise_existence = agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(|route| {
            let contract = route.effective_output_contract();
            route.output_contract_marker_is(crate::OutputSemanticKind::ExistenceWithPath)
                && !contract.delivery_required
                && matches!(
                    contract.response_shape,
                    crate::OutputResponseShape::Scalar | crate::OutputResponseShape::OneSentence
                )
        });
    let mut lines = vec![
        "schema_version=1".to_string(),
        "reason_code=missing_observed_target".to_string(),
        "exists=false".to_string(),
        format!("path=`{path}`"),
        "kind=missing".to_string(),
    ];
    if let Some(final_answer_shape) = final_answer_shape {
        lines.push(format!(
            "final_answer_shape={}",
            final_answer_shape.as_str()
        ));
    }
    if scalar_count {
        lines.push("count_available=false".to_string());
    }
    if concise_existence {
        lines.push("response_shape=existence_with_path".to_string());
    }
    Some(lines.join("\n"))
}

fn route_requests_execution_failed_step_answer(
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .is_some_and(|route| {
            route.output_contract_marker_is(crate::OutputSemanticKind::ExecutionFailedStep)
        })
}

fn command_label_for_execution_step(
    loop_state: &crate::agent_engine::LoopState,
    step: &crate::executor::StepExecutionResult,
) -> Option<String> {
    plan_step_for_execution(loop_state, step)
        .and_then(|plan_step| raw_command_arg_from_plan_step(Some(plan_step)))
        .map(ToOwned::to_owned)
        .or_else(|| raw_command_arg_from_step_error(step))
        .map(|value| truncate_with_ellipsis(&value.replace('`', "'"), 180))
}

fn raw_command_arg_from_step_error(step: &crate::executor::StepExecutionResult) -> Option<String> {
    if step.skill != "run_cmd" {
        return None;
    }
    let error = step.error.as_deref()?.trim();
    let structured = crate::skills::parse_structured_skill_error(error)?;
    let extra = structured.extra.as_ref()?;
    ["command", "cmd"]
        .iter()
        .find_map(|key| structured_extra_string(extra, key))
        .filter(|command| !command.is_empty())
}

pub(super) fn deterministic_execution_failed_step_answer(
    _state: &AppState,
    _user_text: &str,
    loop_state: &crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    if !route_requests_execution_failed_step_answer(agent_run_context) {
        return None;
    }
    let steps = observed_execution_status_steps(loop_state);
    if steps.len() < 2 {
        return None;
    }
    let failed_count = steps.iter().filter(|step| !step.is_ok()).count();
    let failed_steps = steps
        .iter()
        .enumerate()
        .filter(|(_, step)| !step.is_ok())
        .take(6)
        .map(|(idx, step)| {
            let command = command_label_for_execution_step(loop_state, step);
            let observed_error = output_text_from_execution_result(step).map(|value| {
                truncate_with_ellipsis(&value.replace('\n', " ").replace('`', "'"), 180)
            });
            serde_json::json!({
                "step_index": idx + 1,
                "skill": step.skill.trim(),
                "command": command,
                "observed_error": observed_error,
            })
        })
        .collect::<Vec<_>>();
    Some(
        serde_json::json!({
            "schema_version": 1,
            "message_key": "clawd.msg.execution.failed_step_status",
            "reason_code": "execution_failed_step_status",
            "failed_step_count": failed_count,
            "failed_steps": failed_steps,
            "remaining_unexecuted_command_steps": 0,
        })
        .to_string(),
    )
}

pub(super) fn deterministic_observed_execution_status_summary(
    loop_state: &crate::agent_engine::LoopState,
) -> crate::task_journal::TaskJournalFinalizerSummary {
    crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        parsed: true,
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: loop_state.executed_step_results.len(),
        ..Default::default()
    }
}

pub(super) fn path_display_label(value: &serde_json::Value, fallback: &str) -> String {
    let raw = value
        .get("path")
        .or_else(|| value.get("resolved_path"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback);
    Path::new(raw)
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(raw)
        .to_string()
}
fn has_publishable_synthesis_other_than_status(
    loop_state: &crate::agent_engine::LoopState,
    status_answer: &str,
) -> bool {
    loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .is_some_and(|text| {
            text != status_answer.trim() && !delivery_text_is_underinformative_success_status(text)
        })
}

fn latest_delivery_is_underinformative_success_status(
    loop_state: &crate::agent_engine::LoopState,
) -> bool {
    loop_state
        .delivery_messages
        .last()
        .or(loop_state.last_user_visible_respond.as_ref())
        .map(String::as_str)
        .is_some_and(delivery_text_is_underinformative_success_status)
}

fn delivery_text_is_underinformative_success_status(text: &str) -> bool {
    let text = text.trim();
    if text.eq_ignore_ascii_case("status=ok") || text.eq_ignore_ascii_case("status: ok") {
        return true;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    value.as_object().is_some_and(|object| {
        object.len() == 1
            && object
                .get("status")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|status| status.eq_ignore_ascii_case("ok"))
    })
}

fn deterministic_service_control_status_observed_answer(
    loop_state: &crate::agent_engine::LoopState,
) -> Option<String> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .find_map(service_control_status_observed_answer_from_step)
}

fn service_control_status_observed_answer_from_step(
    step: &crate::executor::StepExecutionResult,
) -> Option<String> {
    if step.skill != "service_control" || !step.is_ok() {
        return None;
    }
    let output = output_text_from_execution_result(step)?;
    let output = serde_json::from_str::<serde_json::Value>(&output).ok()?;
    let payload = output
        .get("extra")
        .filter(|value| value.is_object())
        .unwrap_or(&output);
    let action = first_trimmed_string(payload, &["requested_action", "action"])?;
    if !action.eq_ignore_ascii_case("status") {
        return None;
    }

    let mut lines = Vec::new();
    push_first_payload_field(&mut lines, payload, "target", &["target", "service_name"]);
    push_first_payload_field(&mut lines, payload, "service_name", &["service_name"]);
    push_first_payload_field(&mut lines, payload, "post_state", &["post_state"]);
    push_first_payload_field(&mut lines, payload, "pre_state", &["pre_state"]);
    push_first_payload_field(&mut lines, payload, "status", &["status"]);
    push_first_payload_field(&mut lines, payload, "verified", &["verified"]);
    push_first_payload_field(&mut lines, payload, "manager_type", &["manager_type"]);
    push_service_control_evidence_fields(&mut lines, payload);
    if lines.len() <= 1 {
        return None;
    }
    lines.push("source=service_control".to_string());
    Some(lines.join("\n"))
}

fn push_first_payload_field(
    lines: &mut Vec<String>,
    payload: &serde_json::Value,
    output_key: &str,
    input_keys: &[&str],
) {
    let Some(value) = input_keys
        .iter()
        .find_map(|key| payload.get(*key).and_then(payload_scalar_to_string))
    else {
        return;
    };
    lines.push(format!("{output_key}={value}"));
}

fn first_trimmed_string(payload: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        payload
            .get(*key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn payload_scalar_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => {
            let value = value.trim();
            (!value.is_empty()).then(|| value.replace('\n', " "))
        }
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn push_service_control_evidence_fields(lines: &mut Vec<String>, payload: &serde_json::Value) {
    let Some(evidence) = payload.get("key_evidence") else {
        return;
    };
    match evidence {
        serde_json::Value::Array(values) => {
            for (idx, value) in values
                .iter()
                .filter_map(payload_scalar_to_string)
                .take(3)
                .enumerate()
            {
                lines.push(format!("evidence.{}={value}", idx + 1));
            }
        }
        value => {
            if let Some(value) = payload_scalar_to_string(value) {
                lines.push(format!("evidence.1={value}"));
            }
        }
    }
}

pub(super) fn attach_deterministic_observed_execution_status_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut crate::agent_engine::LoopState,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(answer) = deterministic_observed_execution_status_answer(state, user_text, loop_state)
    else {
        return false;
    };
    if has_publishable_synthesis_other_than_status(loop_state, &answer) {
        return false;
    }
    *finalizer_summary = Some(deterministic_observed_execution_status_summary(loop_state));
    loop_state.last_user_visible_respond = Some(answer.clone());
    append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
    log_deterministic_delivery_record(
        &task.task_id,
        "fallback_from_deterministic_observed_status",
        "attached",
        None,
        loop_state.executed_step_results.len(),
    );
    true
}

pub(super) fn attach_deterministic_execution_failed_step_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    if route_prefers_language_rendered_execution_failed_step(agent_run_context) {
        return false;
    }
    let Some(answer) =
        deterministic_execution_failed_step_answer(state, user_text, loop_state, agent_run_context)
    else {
        return false;
    };
    *finalizer_summary = Some(deterministic_observed_execution_status_summary(loop_state));
    loop_state.last_user_visible_respond = Some(answer.clone());
    append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
    log_deterministic_delivery_record(
        &task.task_id,
        "fallback_from_deterministic_execution_failed_step",
        "attached",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

pub(super) fn replace_delivery_with_deterministic_observed_execution_status_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut crate::agent_engine::LoopState,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let answer = if let Some(answer) =
        deterministic_observed_execution_status_answer(state, user_text, loop_state)
    {
        answer
    } else if latest_delivery_is_underinformative_success_status(loop_state) {
        let Some(answer) = deterministic_service_control_status_observed_answer(loop_state) else {
            return false;
        };
        answer
    } else {
        return false;
    };
    if has_publishable_synthesis_other_than_status(loop_state, &answer) {
        return false;
    }
    if loop_state.delivery_messages.last().is_some_and(|message| {
        planned_delivery_identifies_failed_observed_step(message, loop_state)
    }) {
        *finalizer_summary = Some(deterministic_observed_execution_status_summary(loop_state));
        return false;
    }
    let unchanged = loop_state
        .delivery_messages
        .last()
        .map(|message| message.trim() == answer.trim())
        .unwrap_or(false);
    *finalizer_summary = Some(deterministic_observed_execution_status_summary(loop_state));
    loop_state.last_user_visible_respond = Some(answer.clone());
    loop_state.delivery_messages.clear();
    if !unchanged {
        append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
        log_deterministic_delivery_record(
            &task.task_id,
            "replace_with_deterministic_observed_status",
            "replaced",
            None,
            loop_state.executed_step_results.len(),
        );
    }
    true
}

pub(super) fn replace_delivery_with_deterministic_execution_failed_step_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut crate::agent_engine::LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    if route_prefers_language_rendered_execution_failed_step(agent_run_context) {
        return false;
    }
    let Some(answer) =
        deterministic_execution_failed_step_answer(state, user_text, loop_state, agent_run_context)
    else {
        return false;
    };
    let unchanged = loop_state
        .delivery_messages
        .last()
        .map(|message| message.trim() == answer.trim())
        .unwrap_or(false);
    *finalizer_summary = Some(deterministic_observed_execution_status_summary(loop_state));
    loop_state.last_user_visible_respond = Some(answer.clone());
    loop_state.delivery_messages.clear();
    if !unchanged {
        append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, answer);
        log_deterministic_delivery_record(
            &task.task_id,
            "replace_with_deterministic_execution_failed_step",
            "replaced",
            agent_run_context,
            loop_state.executed_step_results.len(),
        );
    }
    true
}

pub(super) fn planned_delivery_identifies_failed_observed_step(
    delivery: &str,
    loop_state: &crate::agent_engine::LoopState,
) -> bool {
    let delivery = delivery.trim();
    if delivery.is_empty() {
        return false;
    }
    loop_state.executed_step_results.iter().any(|step| {
        !step.is_ok()
            && !matches!(
                step.skill.as_str(),
                "respond" | "think" | "synthesize_answer"
            )
            && plan_step_for_execution(loop_state, step)
                .and_then(|plan_step| raw_command_arg_from_plan_step(Some(plan_step)))
                .is_some_and(|command| delivery.contains(command))
    })
}
