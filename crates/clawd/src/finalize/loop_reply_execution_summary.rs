use std::path::Path;

use crate::agent_engine::{AgentRunContext, LoopState};

use super::valid_publishable_synthesis_output;
#[cfg(test)]
use super::{
    delivery_message_is_json_container, last_respond_matches_single_line_observation,
    looks_like_structured_machine_output, message_is_non_answer_separator,
    output_contract_requests_exact_delivery, route_has_evidence_policy_final_shape,
    route_requires_evidence_policy_deterministic_final_answer, single_publishable_delivery_message,
    step_output_is_read_range,
};

#[cfg(test)]
pub(super) fn route_requires_content_excerpt_evidence(route: &crate::IntentOutputContract) -> bool {
    crate::evidence_policy::required_evidence_fields_for_output_contract(route)
        .iter()
        .any(|field| field == "content_excerpt")
}

pub(super) fn latest_publishable_synthesis_step_matches(loop_state: &LoopState) -> bool {
    let Some(synthesis) = loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return false;
    };
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .find(|step| step.skill == "synthesize_answer" && step.is_ok())
        .and_then(|step| step.output.as_deref())
        .map(str::trim)
        .is_some_and(|output| output == synthesis)
}

pub(super) fn truncate_with_ellipsis(text: &str, max_chars: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    if max_chars <= 3 {
        return "...".to_string();
    }
    let mut truncated = text
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    truncated.push_str("...");
    truncated
}

pub(super) fn execution_summary_arg_is_sensitive(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "secret", "token", "key", "password", "passwd", "cookie", "auth",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

fn plan_step_matches_execution(
    plan_step: &crate::PlanStep,
    step: &crate::executor::StepExecutionResult,
) -> bool {
    let plan_skill = plan_step.skill.trim();
    if !plan_skill.eq_ignore_ascii_case(step.skill.trim()) {
        return false;
    }
    plan_step_action_matches_execution(plan_step, step)
}

fn execution_output_json(step: &crate::executor::StepExecutionResult) -> Option<serde_json::Value> {
    let raw = step.output.as_deref()?.trim();
    if raw.is_empty() {
        return None;
    }
    serde_json::from_str::<serde_json::Value>(raw).ok()
}

fn execution_output_action(step: &crate::executor::StepExecutionResult) -> Option<String> {
    execution_output_json(step)?
        .get("action")?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn plan_step_action_arg(plan_step: &crate::PlanStep) -> Option<&str> {
    plan_step
        .args
        .get("action")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn plan_step_action_matches_execution(
    plan_step: &crate::PlanStep,
    step: &crate::executor::StepExecutionResult,
) -> bool {
    let Some(plan_action) = plan_step_action_arg(plan_step) else {
        return true;
    };
    let Some(output_action) = execution_output_action(step) else {
        return true;
    };
    plan_action.eq_ignore_ascii_case(output_action.trim())
}

pub(super) fn plan_step_for_execution<'a>(
    loop_state: &'a LoopState,
    step: &crate::executor::StepExecutionResult,
) -> Option<&'a crate::PlanStep> {
    let exact = loop_state
        .round_traces
        .iter()
        .filter_map(|trace| trace.plan_result.as_ref())
        .flat_map(|plan| plan.steps.iter())
        .find(|plan_step| {
            plan_step.step_id == step.step_id && plan_step_matches_execution(plan_step, step)
        });
    if exact.is_some() {
        return exact;
    }

    let output_action = execution_output_action(step)?;
    loop_state
        .round_traces
        .iter()
        .rev()
        .filter_map(|trace| trace.plan_result.as_ref())
        .flat_map(|plan| plan.steps.iter())
        .find(|plan_step| {
            plan_step_matches_execution(plan_step, step)
                && plan_step_action_arg(plan_step)
                    .is_some_and(|action| action.eq_ignore_ascii_case(output_action.trim()))
        })
}

pub(super) fn exact_observation_arg_from_plan_step(
    plan_step: Option<&crate::PlanStep>,
) -> Option<&str> {
    let args = &plan_step?.args;
    args.get("command")
        .or_else(|| args.get("cmd"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(super) fn output_text_from_execution_result(
    step: &crate::executor::StepExecutionResult,
) -> Option<String> {
    let raw = if step.is_ok() {
        step.output.as_deref()
    } else {
        step.error.as_deref().or(step.output.as_deref())
    }?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.eq_ignore_ascii_case("NOT_FOUND") {
        return Some(execution_summary_machine_json(serde_json::json!({
            "message_key": "clawd.msg.execution.step_observation",
            "reason_code": "not_found",
            "step_id": &step.step_id,
            "skill": &step.skill,
            "status": step.status.as_str(),
            "error_kind": "not_found",
        })));
    }
    if let Some(path) = crate::skills::read_file_not_found_path(trimmed) {
        return Some(execution_summary_machine_json(serde_json::json!({
            "message_key": "clawd.msg.execution.step_observation",
            "reason_code": "read_file_not_found",
            "step_id": &step.step_id,
            "skill": &step.skill,
            "status": step.status.as_str(),
            "error_kind": "not_found",
            "path": path,
        })));
    }
    if let Some(structured) = crate::skills::parse_structured_skill_error(trimmed) {
        return Some(structured_execution_error_summary(step, &structured));
    }
    if !step.is_ok() && crate::skills::is_recoverable_skill_error(&step.skill, trimmed) {
        return Some(execution_summary_machine_json(serde_json::json!({
            "message_key": "clawd.msg.execution.step_observation",
            "reason_code": "recoverable_skill_error",
            "step_id": &step.step_id,
            "skill": &step.skill,
            "status": step.status.as_str(),
            "error_kind": "recoverable_error",
        })));
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return Some(execution_json_summary_without_user_text_fields(step, value));
    }
    Some(crate::visible_text::sanitize_user_visible_text(trimmed))
}

fn execution_summary_machine_json(mut value: serde_json::Value) -> String {
    strip_user_visible_json_fields(&mut value);
    crate::visible_text::sanitize_user_visible_text(&value.to_string())
}

fn structured_execution_error_summary(
    step: &crate::executor::StepExecutionResult,
    structured: &crate::skills::StructuredSkillError,
) -> String {
    let effective_skill = if structured.skill.trim().is_empty() {
        step.skill.trim()
    } else {
        structured.skill.trim()
    };
    execution_summary_machine_json(serde_json::json!({
        "message_key": "clawd.msg.execution.step_observation",
        "reason_code": "structured_skill_error",
        "step_id": &step.step_id,
        "skill": effective_skill,
        "status": step.status.as_str(),
        "error_kind": &structured.error_kind,
        "platform": &structured.platform,
        "manager_type": &structured.manager_type,
        "service_name": &structured.service_name,
        "extra": &structured.extra,
    }))
}

fn execution_json_summary_without_user_text_fields(
    step: &crate::executor::StepExecutionResult,
    mut value: serde_json::Value,
) -> String {
    strip_user_visible_json_fields(&mut value);
    if let Some(object) = value.as_object_mut() {
        object
            .entry("message_key".to_string())
            .or_insert_with(|| serde_json::json!("clawd.msg.execution.step_observation"));
        object
            .entry("reason_code".to_string())
            .or_insert_with(|| serde_json::json!("json_observation"));
        object
            .entry("step_id".to_string())
            .or_insert_with(|| serde_json::json!(&step.step_id));
        object
            .entry("skill".to_string())
            .or_insert_with(|| serde_json::json!(&step.skill));
        object
            .entry("status".to_string())
            .or_insert_with(|| serde_json::json!(step.status.as_str()));
        return crate::visible_text::sanitize_user_visible_text(&value.to_string());
    }
    execution_summary_machine_json(serde_json::json!({
        "message_key": "clawd.msg.execution.step_observation",
        "reason_code": "json_observation",
        "step_id": &step.step_id,
        "skill": &step.skill,
        "status": step.status.as_str(),
        "value": value,
    }))
}

fn strip_user_visible_json_fields(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            object.remove("text");
            object.remove("error_text");
            for child in object.values_mut() {
                strip_user_visible_json_fields(child);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                strip_user_visible_json_fields(child);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
pub(super) fn build_execution_summary_messages(
    _loop_state: &LoopState,
    _agent_run_context: Option<&AgentRunContext>,
    _user_text: Option<&str>,
) -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
pub(super) fn build_execution_summary_message(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    user_text: Option<&str>,
) -> Option<String> {
    let messages = build_execution_summary_messages(loop_state, agent_run_context, user_text);
    if messages.is_empty() {
        None
    } else {
        Some(messages.join("\n\n"))
    }
}

pub(super) fn attach_execution_summary_to_delivery(
    _loop_state: &LoopState,
    _agent_run_context: Option<&AgentRunContext>,
    _user_text: Option<&str>,
    delivery_messages: &mut Vec<String>,
) {
    delivery_messages.retain(|message| !crate::finalize::is_execution_summary_message(message));
}

#[cfg(test)]
pub(super) fn delivery_contract_suppresses_execution_summary(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &[String],
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) else {
        return false;
    };
    let has_publishable_answer = delivery_messages.iter().any(|message| {
        let trimmed = message.trim();
        !trimmed.is_empty() && !crate::finalize::is_execution_summary_message(trimmed)
    });
    if delivery_keeps_execution_summary_for_context(route, loop_state, delivery_messages) {
        return false;
    }
    if delivery_token_contract_suppresses_execution_summary(route, delivery_messages) {
        return true;
    }
    if has_publishable_answer
        && route.requires_content_evidence
        && !route.does_not_request_exact_command_output()
    {
        return true;
    }
    if route_has_evidence_policy_final_shape(route) {
        return true;
    }
    if route_requires_content_excerpt_evidence(route) && has_publishable_answer {
        return true;
    }
    if route.response_shape == crate::OutputResponseShape::Strict
        && delivery_messages
            .iter()
            .any(|message| delivery_message_is_json_container(message))
    {
        return true;
    }
    if delivery_matches_latest_transform_observation(loop_state, delivery_messages) {
        return true;
    }
    if delivery_matches_latest_read_range_synthesis(loop_state, route, delivery_messages) {
        return true;
    }
    let has_existing_execution_summary =
        delivery_messages_have_execution_summary(delivery_messages);
    if has_existing_execution_summary
        && delivery_has_synthesized_answer_result(loop_state, route, delivery_messages)
    {
        return true;
    }
    if has_existing_execution_summary
        && delivery_matches_synthesized_content_answer(loop_state, route, delivery_messages)
    {
        return true;
    }
    if delivery_matches_grounded_content_answer(loop_state, route, delivery_messages) {
        return true;
    }
    let contract = route.clone();
    if contract.response_shape != crate::OutputResponseShape::Scalar {
        return false;
    }
    if !route.does_not_request_exact_command_output() {
        return false;
    }
    delivery_messages.iter().any(|message| {
        let message = message.trim();
        !message.is_empty() && !crate::finalize::is_execution_summary_message(message)
    })
}

#[cfg(test)]
fn delivery_token_contract_suppresses_execution_summary(
    route: &crate::IntentOutputContract,
    delivery_messages: &[String],
) -> bool {
    let delivery_contract = route.delivery_required
        || route.delivery_required
        || !matches!(route.delivery_intent, crate::OutputDeliveryIntent::None)
        || matches!(route.response_shape, crate::OutputResponseShape::FileToken);
    delivery_contract && delivery_messages_include_delivery_token(delivery_messages)
}

pub(super) fn delivery_messages_include_delivery_token(delivery_messages: &[String]) -> bool {
    delivery_messages.iter().any(|message| {
        message
            .lines()
            .map(str::trim)
            .any(|line| crate::finalize::parse_delivery_token(line).is_some())
    })
}

#[cfg(test)]
fn delivery_messages_have_execution_summary(delivery_messages: &[String]) -> bool {
    delivery_messages
        .iter()
        .any(|message| crate::finalize::is_execution_summary_message(message))
}

#[cfg(test)]
fn delivery_keeps_execution_summary_for_context(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
    delivery_messages: &[String],
) -> bool {
    if output_contract_requests_exact_delivery(route) || route.delivery_required {
        return false;
    }
    if route.response_shape == crate::OutputResponseShape::Strict {
        return false;
    }
    let Some(delivery_text) = single_publishable_delivery_message(delivery_messages) else {
        return false;
    };
    if delivery_matches_latest_read_range_synthesis(loop_state, route, delivery_messages) {
        return false;
    }
    if last_respond_matches_single_line_observation(loop_state, delivery_text) {
        return true;
    }
    valid_publishable_synthesis_output(loop_state)
        .is_some_and(|synthesis| synthesis == delivery_text.trim())
}

pub(super) fn delivery_matches_latest_publishable_synthesis(
    loop_state: &LoopState,
    delivery_messages: &[String],
) -> bool {
    let Some(synthesis) = loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return false;
    };
    latest_publishable_synthesis_step_matches(loop_state)
        && delivery_messages
            .last()
            .map(|message| message.trim())
            .is_some_and(|message| message == synthesis)
}

fn same_observed_or_display_path(left: &str, right: &str) -> bool {
    let left = left.trim();
    let right = right.trim();
    if left.is_empty() || right.is_empty() {
        return false;
    }
    if left == right {
        return true;
    }
    let left_path = Path::new(left);
    let right_path = Path::new(right);
    match (left_path.canonicalize(), right_path.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left_path == right_path,
    }
}

pub(super) fn latest_publishable_synthesis_matches_written_file_path(
    loop_state: &LoopState,
) -> bool {
    let Some(synthesis) = valid_publishable_synthesis_output(loop_state) else {
        return false;
    };
    loop_state
        .output_vars
        .get("last_written_file_path")
        .or(loop_state.last_written_file_path.as_ref())
        .map(String::as_str)
        .is_some_and(|path| same_observed_or_display_path(synthesis, path))
}

#[cfg(test)]
fn delivery_matches_latest_read_range_synthesis(
    loop_state: &LoopState,
    route: &crate::IntentOutputContract,
    delivery_messages: &[String],
) -> bool {
    if !route.requires_content_evidence
        || route.delivery_required
        || !latest_publishable_synthesis_step_matches(loop_state)
    {
        return false;
    }
    let Some(delivery_text) = single_publishable_delivery_message(delivery_messages) else {
        return false;
    };
    if !loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .is_some_and(|synthesis| synthesis == delivery_text.trim())
    {
        return false;
    }
    loop_state
        .executed_step_results
        .iter()
        .any(step_output_is_read_range)
}

#[cfg(test)]
fn delivery_matches_synthesized_content_answer(
    loop_state: &LoopState,
    route: &crate::IntentOutputContract,
    delivery_messages: &[String],
) -> bool {
    if !route.requires_content_evidence || route.delivery_required {
        return false;
    }
    if !matches!(
        route.response_shape,
        crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
    ) {
        return false;
    }
    let Some(delivery_text) = single_publishable_delivery_message(delivery_messages) else {
        return false;
    };
    if crate::agent_engine::observed_output::answer_matches_observed_output_passthrough(
        delivery_text,
        loop_state,
    ) {
        return false;
    }
    loop_state.executed_step_results.iter().any(|step| {
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
    })
}

#[cfg(test)]
fn delivery_matches_grounded_content_answer(
    loop_state: &LoopState,
    route: &crate::IntentOutputContract,
    delivery_messages: &[String],
) -> bool {
    if !route.requires_content_evidence || route.delivery_required {
        return false;
    }
    if !matches!(
        route.response_shape,
        crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
    ) {
        return false;
    }
    if matches!(route.response_shape, crate::OutputResponseShape::FileToken)
        || route.requests_exact_command_output()
    {
        return false;
    }
    if route_requires_evidence_policy_deterministic_final_answer(route) {
        return false;
    }
    if latest_publishable_synthesis_step_matches(loop_state) {
        return false;
    }
    let Some(delivery_text) = single_publishable_delivery_message(delivery_messages) else {
        return false;
    };
    let delivery_text = delivery_text.trim();
    if delivery_text.is_empty()
        || crate::agent_engine::observed_output::answer_matches_observed_output_passthrough(
            delivery_text,
            loop_state,
        )
        || crate::finalize::looks_like_planner_artifact(delivery_text)
        || crate::finalize::looks_like_internal_trace_artifact(delivery_text)
        || looks_like_structured_machine_output(delivery_text)
        || message_is_non_answer_separator(delivery_text)
    {
        return false;
    }
    loop_state.executed_step_results.iter().any(|step| {
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
    })
}

#[cfg(test)]
fn delivery_has_synthesized_answer_result(
    loop_state: &LoopState,
    route: &crate::IntentOutputContract,
    delivery_messages: &[String],
) -> bool {
    if !route.requires_content_evidence || route.delivery_required {
        return false;
    }
    let Some(delivery_text) = single_publishable_delivery_message(delivery_messages) else {
        return false;
    };
    if crate::agent_engine::observed_output::answer_matches_observed_output_passthrough(
        delivery_text,
        loop_state,
    ) {
        return false;
    }
    loop_state.executed_step_results.iter().any(|step| {
        step.is_ok()
            && step.skill == "synthesize_answer"
            && step
                .output
                .as_deref()
                .map(str::trim)
                .is_some_and(|output| !output.is_empty())
    })
}

#[cfg(test)]
fn delivery_matches_latest_transform_observation(
    loop_state: &LoopState,
    delivery_messages: &[String],
) -> bool {
    let Some(delivery_text) = single_publishable_delivery_message(delivery_messages) else {
        return false;
    };
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok())
        .filter_map(|step| step.output.as_deref())
        .any(|output| {
            crate::agent_engine::observed_output::transform_skill_formatted_output_candidate(output)
                .is_some_and(|answer| answer.trim() == delivery_text)
        })
}
