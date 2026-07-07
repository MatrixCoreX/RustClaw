use std::path::Path;

use crate::agent_engine::{AgentRunContext, LoopState};

#[cfg(test)]
use super::{
    delivery_matches_config_guard_answer, delivery_message_is_json_container,
    first_markdown_heading_from_read_output, last_respond_matches_single_line_observation,
    looks_like_raw_command_snapshot, markdown_heading_from_line, message_is_non_answer_separator,
    output_contract_requests_exact_delivery,
    route_allows_observed_markdown_heading_scalar_delivery, route_has_evidence_policy_final_shape,
    route_requires_evidence_policy_deterministic_final_answer, single_publishable_delivery_message,
};
use super::{
    looks_like_structured_machine_output, step_output_is_read_range,
    valid_publishable_synthesis_output,
};

#[cfg(test)]
pub(super) fn should_attach_execution_summary(
    _loop_state: &LoopState,
    _agent_run_context: Option<&AgentRunContext>,
    _user_text: Option<&str>,
) -> bool {
    false
}

#[cfg(test)]
pub(super) fn route_requires_content_excerpt_evidence(route: &crate::RouteResult) -> bool {
    crate::evidence_policy::required_evidence_fields_for_route(route)
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

fn loop_has_structured_listing_observation(loop_state: &LoopState) -> bool {
    loop_state.executed_step_results.iter().any(|step| {
        if !step.is_ok() || !matches!(step.skill.as_str(), "system_basic" | "fs_basic") {
            return false;
        }
        let Some(output) = step.output.as_deref() else {
            return false;
        };
        serde_json::from_str::<serde_json::Value>(output.trim())
            .ok()
            .is_some_and(|value| value_has_structured_listing_observation(&value))
    })
}

fn value_has_structured_listing_observation(value: &serde_json::Value) -> bool {
    if value.get("names_by_kind").is_some()
        || value
            .get("names")
            .and_then(|value| value.as_array())
            .is_some_and(|items| !items.is_empty())
        || matches!(
            value.get("action").and_then(|value| value.as_str()),
            Some("inventory_dir" | "list_dir" | "tree_summary")
        )
    {
        return true;
    }
    if value
        .get("extra")
        .filter(|extra| extra.is_object())
        .is_some_and(value_has_structured_listing_observation)
    {
        return true;
    }
    false
}

#[cfg(test)]
pub(super) fn structured_listing_observation_for_test(value: &serde_json::Value) -> bool {
    value_has_structured_listing_observation(value)
}

pub(super) fn directory_entry_groups_prefers_observed_groups(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> bool {
    crate::finalize::route_prefers_grouped_name_list_output(route)
        && loop_has_structured_listing_observation(loop_state)
        && !loop_state
            .executed_step_results
            .iter()
            .any(step_output_is_read_range)
}

pub(super) fn latest_grounded_synthesis_for_mixed_listing_contract(
    route: &crate::RouteResult,
    loop_state: &LoopState,
) -> Option<(String, crate::task_journal::TaskJournalFinalizerSummary)> {
    if !crate::finalize::route_prefers_grouped_name_list_output(route)
        || !latest_publishable_synthesis_step_matches(loop_state)
        || !loop_has_structured_listing_observation(loop_state)
        || !loop_state
            .executed_step_results
            .iter()
            .any(step_output_is_read_range)
    {
        return None;
    }
    let synthesis = loop_state
        .last_publishable_synthesis_output
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())?;
    if crate::finalize::looks_like_planner_artifact(synthesis)
        || crate::finalize::looks_like_internal_trace_artifact(synthesis)
        || crate::finalize::parse_delivery_token(synthesis).is_some()
        || looks_like_structured_machine_output(synthesis)
    {
        return None;
    }

    Some((
        synthesis.to_string(),
        crate::task_journal::TaskJournalFinalizerSummary {
            stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
            disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
            parsed: true,
            contract_ok: true,
            completion_ok: Some(true),
            grounded_ok: Some(true),
            format_ok: Some(true),
            needs_clarify: Some(false),
            used_evidence_ids_count: 2,
            ..Default::default()
        },
    ))
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

pub(super) fn execution_summary_value_to_string(value: &serde_json::Value) -> String {
    let raw = match value {
        serde_json::Value::String(value) => value.trim().to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Null => String::new(),
        _ => value.to_string(),
    };
    crate::visible_text::sanitize_user_visible_text(&raw)
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
    if !(plan_skill.eq_ignore_ascii_case(step.skill.trim())
        || (step.skill == "run_cmd" && plan_skill.eq_ignore_ascii_case("run_cmd")))
    {
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

pub(super) fn raw_command_arg_from_plan_step(plan_step: Option<&crate::PlanStep>) -> Option<&str> {
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
        return Some("file not found".to_string());
    }
    if let Some(path) = trimmed.strip_prefix("__RC_READ_FILE_NOT_FOUND__:") {
        return Some(crate::visible_text::sanitize_user_visible_text(&format!(
            "file not found: {}",
            path.trim()
        )));
    }
    if crate::skills::parse_structured_skill_error(trimmed).is_some() {
        return Some(crate::visible_text::sanitize_user_visible_text(
            &crate::skills::normalize_skill_error_for_user(&step.skill, trimmed),
        ));
    }
    if !step.is_ok() && crate::skills::is_recoverable_skill_error(&step.skill, trimmed) {
        return Some(crate::visible_text::sanitize_user_visible_text(
            &crate::skills::normalize_skill_error_for_user(&step.skill, trimmed),
        ));
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(text) = value
            .get("text")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(crate::visible_text::sanitize_user_visible_text(text));
        }
        if let Some(text) = value
            .get("stdout")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(crate::visible_text::sanitize_user_visible_text(text));
        }
        if let Some(text) = value
            .get("error_text")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(crate::visible_text::sanitize_user_visible_text(text));
        }
    }
    Some(crate::visible_text::sanitize_user_visible_text(trimmed))
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
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
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
        && route.output_contract.requires_content_evidence
        && !route.output_contract_is_unclassified()
    {
        return true;
    }
    if route_has_evidence_policy_final_shape(route) {
        return true;
    }
    if route_requires_content_excerpt_evidence(route) && has_publishable_answer {
        return true;
    }
    if route.output_contract.response_shape == crate::OutputResponseShape::Strict
        && delivery_messages
            .iter()
            .any(|message| delivery_message_is_json_container(message))
    {
        return true;
    }
    if delivery_matches_latest_structured_scalar_observation(loop_state, route, delivery_messages) {
        return true;
    }
    if delivery_matches_config_guard_answer(loop_state, delivery_messages) {
        return true;
    }
    if delivery_matches_latest_transform_observation(loop_state, delivery_messages) {
        return true;
    }
    if delivery_matches_observed_markdown_heading_delivery(
        loop_state,
        agent_run_context,
        delivery_messages,
    ) {
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
    let contract = route.effective_output_contract();
    if contract.response_shape != crate::OutputResponseShape::Scalar {
        return false;
    }
    if !route.output_contract_is_unclassified() {
        return false;
    }
    delivery_messages.iter().any(|message| {
        let message = message.trim();
        !message.is_empty() && !crate::finalize::is_execution_summary_message(message)
    })
}

#[cfg(test)]
fn delivery_token_contract_suppresses_execution_summary(
    route: &crate::RouteResult,
    delivery_messages: &[String],
) -> bool {
    let delivery_contract = route.wants_file_delivery
        || route.output_contract.delivery_required
        || !matches!(
            route.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::None
        )
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        );
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
    route: &crate::RouteResult,
    loop_state: &LoopState,
    delivery_messages: &[String],
) -> bool {
    if output_contract_requests_exact_delivery(route) || route.output_contract.delivery_required {
        return false;
    }
    if route.output_contract.response_shape == crate::OutputResponseShape::Strict {
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
fn delivery_matches_observed_markdown_heading_delivery(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    delivery_messages: &[String],
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.route_result.as_ref()) else {
        return false;
    };
    if !route_allows_observed_markdown_heading_scalar_delivery(route)
        || route.output_contract.delivery_required
        || matches!(
            route.effective_output_contract_semantic_kind(),
            crate::OutputSemanticKind::FileNames
                | crate::OutputSemanticKind::DirectoryNames
                | crate::OutputSemanticKind::FilePaths
                | crate::OutputSemanticKind::DirectoryEntryGroups
                | crate::OutputSemanticKind::ScalarCount
                | crate::OutputSemanticKind::RawCommandOutput
                | crate::OutputSemanticKind::ScalarPathOnly
                | crate::OutputSemanticKind::ExistenceWithPath
                | crate::OutputSemanticKind::ExistenceWithPathSummary
        )
    {
        return false;
    }
    let Some(delivery_text) = single_publishable_delivery_message(delivery_messages) else {
        return false;
    };
    let Some(delivery_heading) = markdown_heading_from_line(delivery_text) else {
        return false;
    };
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok())
        .filter_map(|step| step.output.as_deref())
        .find(|output| output.contains("\"read_range\"") || output.contains("\"read_text_range\""))
        .and_then(first_markdown_heading_from_read_output)
        .is_some_and(|observed_heading| observed_heading.trim() == delivery_heading.trim())
}

#[cfg(test)]
fn delivery_matches_latest_read_range_synthesis(
    loop_state: &LoopState,
    route: &crate::RouteResult,
    delivery_messages: &[String],
) -> bool {
    if !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
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
fn delivery_matches_latest_structured_scalar_observation(
    loop_state: &LoopState,
    route: &crate::RouteResult,
    delivery_messages: &[String],
) -> bool {
    if !route.output_contract_marker_is(crate::OutputSemanticKind::StructuredKeys) {
        return false;
    }
    let Some(delivery_text) = single_publishable_delivery_message(delivery_messages) else {
        return false;
    };
    crate::agent_engine::observed_output::latest_structured_scalar_observation_text(loop_state)
        .is_some_and(|observed_text| delivery_text == observed_text.trim())
}

#[cfg(test)]
fn delivery_matches_synthesized_content_answer(
    loop_state: &LoopState,
    route: &crate::RouteResult,
    delivery_messages: &[String],
) -> bool {
    if !route.output_contract.requires_content_evidence || route.output_contract.delivery_required {
        return false;
    }
    if !matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
    ) {
        return false;
    }
    if !matches!(
        route.effective_output_contract_semantic_kind(),
        crate::OutputSemanticKind::None | crate::OutputSemanticKind::ContentExcerptSummary
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
    route: &crate::RouteResult,
    delivery_messages: &[String],
) -> bool {
    if !route.output_contract.requires_content_evidence || route.output_contract.delivery_required {
        return false;
    }
    if !matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
    ) {
        return false;
    }
    if matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    ) || matches!(
        route.effective_output_contract_semantic_kind(),
        crate::OutputSemanticKind::RawCommandOutput
    ) {
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
        || looks_like_raw_command_snapshot(delivery_text)
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
    route: &crate::RouteResult,
    delivery_messages: &[String],
) -> bool {
    if !route.output_contract.requires_content_evidence || route.output_contract.delivery_required {
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
        .filter(|step| step.is_ok() && step.skill == "transform")
        .filter_map(|step| step.output.as_deref())
        .any(|output| {
            crate::agent_engine::observed_output::transform_skill_formatted_output_candidate(output)
                .is_some_and(|answer| answer.trim() == delivery_text)
        })
}
