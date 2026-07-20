use serde_json::Value;

use crate::agent_engine::{AgentRunContext, LoopState};
use crate::{AppState, OutputResponseShape};

#[path = "dispatch_synthesis_local_code_fields.rs"]
mod dispatch_synthesis_local_code_fields;
#[path = "dispatch_synthesis_local_code_projection.rs"]
mod dispatch_synthesis_local_code_projection;
#[path = "dispatch_synthesis_local_code_readbacks.rs"]
mod dispatch_synthesis_local_code_readbacks;
#[path = "dispatch_synthesis_local_code_writes.rs"]
mod dispatch_synthesis_local_code_writes;
use crate::read_range_utils::strip_read_range_line_prefix;
use dispatch_synthesis_local_code_projection::{
    common_parent_path, filesystem_projection_skill, machine_code_token, normalize_projection_path,
    path_looks_like_code_or_test_file, path_looks_like_test_file, projection_paths_match,
    successful_code_readbacks, successful_fs_readbacks_after_latest_writes, FsReadback,
};
pub(super) use dispatch_synthesis_local_code_projection::{
    local_code_task_strict_json_projection, requested_local_code_json_fields,
    strict_json_projection_answer_satisfies_request,
};

pub(super) fn synthesize_answer_allows_direct_fallback(evidence_refs: &[String]) -> bool {
    evidence_refs.is_empty()
        || evidence_refs
            .iter()
            .all(|reference| reference.trim().eq_ignore_ascii_case("last_output"))
}

pub(super) fn synthesize_route_allows_direct_fallback(
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|context| context.output_contract()) else {
        return true;
    };
    if crate::agent_engine::observed_output::route_disallows_direct_observation_passthrough(route) {
        return false;
    }
    if route.requires_content_evidence
        && !route.delivery_required
        && route.semantic_kind_is_unclassified()
    {
        return true;
    }
    if route.requests_exact_path_list() {
        return true;
    }
    if route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
        && route.response_shape == crate::OutputResponseShape::Strict
    {
        return false;
    }
    matches!(
        route.response_shape,
        crate::OutputResponseShape::Scalar
            | crate::OutputResponseShape::Strict
            | crate::OutputResponseShape::FileToken
    ) || route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
}

pub(super) fn synthesize_route_prefers_model_language_failure_answer(
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    agent_run_context
        .and_then(|context| context.output_contract())
        .is_some_and(|route| {
            route.semantic_kind_is(crate::OutputSemanticKind::ExecutionFailedStep)
                && route.requires_content_evidence
                && !route.delivery_required
        })
}

fn output_has_count_inventory_total(output: &str) -> bool {
    let output = crate::agent_engine::observed_output::normalized_success_body_for_observed_output(
        output.trim(),
    );
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
        return false;
    };
    value.get("action").and_then(|value| value.as_str()) == Some("count_inventory")
        && value
            .get("counts")
            .and_then(|counts| counts.get("total"))
            .and_then(|value| value.as_u64())
            .is_some()
}

fn multiple_count_inventory_observations_need_synthesis(loop_state: &LoopState) -> bool {
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok() && matches!(step.skill.as_str(), "system_basic" | "fs_basic"))
        .filter_map(|step| step.output.as_deref())
        .filter(|output| output_has_count_inventory_total(output))
        .take(2)
        .count()
        >= 2
}

pub(super) fn synthesize_direct_observed_fallback_answer(
    state: &AppState,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    if let Some(answer) = reusable_terminal_json_after_later_observation(loop_state) {
        return Some(answer);
    }
    if let Some(answer) =
        synthesize_strict_raw_tail_read_direct_answer(loop_state, agent_run_context)
    {
        return Some(answer);
    }
    if agent_run_context
        .and_then(|context| context.output_contract())
        .is_some_and(
            crate::agent_engine::observed_output::route_disallows_direct_observation_passthrough,
        )
    {
        return None;
    }
    if route_needs_synthesis_for_multi_observation_grounded_summary(loop_state, agent_run_context) {
        return None;
    }
    if multiple_count_inventory_observations_need_synthesis(loop_state) {
        return None;
    }
    crate::agent_engine::observed_output::extract_answer_from_observed_output_i18n(
        loop_state,
        state,
        agent_run_context,
    )
    .or_else(|| {
        crate::agent_engine::observed_output::extract_direct_scalar_from_generic_output_i18n(
            loop_state,
            state,
            agent_run_context,
        )
    })
    .map(|answer| answer.trim().to_string())
    .filter(|answer| !answer.is_empty())
}

fn reusable_terminal_json_after_later_observation(loop_state: &LoopState) -> Option<String> {
    let (terminal_idx, answer) = loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, step)| {
            step.is_ok() && matches!(step.skill.as_str(), "synthesize_answer" | "respond")
        })
        .filter_map(|(idx, step)| {
            let answer = step.output.as_deref()?.trim();
            terminal_json_is_reusable(answer).then(|| (idx, answer.to_string()))
        })
        .next()?;
    let has_later_observation =
        loop_state
            .executed_step_results
            .iter()
            .enumerate()
            .any(|(idx, step)| {
                idx > terminal_idx
                    && step.is_ok()
                    && !matches!(
                        step.skill.as_str(),
                        "synthesize_answer" | "respond" | "think"
                    )
                    && step
                        .output
                        .as_deref()
                        .map(str::trim)
                        .is_some_and(|output| !output.is_empty())
            });
    has_later_observation.then_some(answer)
}

fn terminal_json_is_reusable(answer: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(answer.trim()) else {
        return false;
    };
    let Some(obj) = value.as_object() else {
        return false;
    };
    if obj.is_empty() {
        return false;
    }
    if obj.len() == 1 && obj.contains_key("steps") {
        return false;
    }
    !json_contains_unresolved_terminal_value(&value)
}

fn json_contains_unresolved_terminal_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(_) | Value::Number(_) => false,
        Value::String(text) => {
            let trimmed = text.trim();
            trimmed.is_empty()
                || trimmed.contains("{{")
                || trimmed == "<missing>"
                || trimmed == "not_observed"
                || trimmed == "null"
        }
        Value::Array(items) => items.iter().any(json_contains_unresolved_terminal_value),
        Value::Object(map) => map.values().any(json_contains_unresolved_terminal_value),
    }
}

#[cfg(test)]
#[path = "dispatch_synthesis_tests.rs"]
mod tests;

fn skill_output_payload(output: &str) -> Option<Value> {
    let value = serde_json::from_str::<Value>(output.trim()).ok()?;
    if let Some(extra) = value.get("extra").filter(|extra| extra.is_object()) {
        return Some(extra.clone());
    }
    Some(value)
}

fn push_unique_string(values: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    if !values
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(value))
    {
        values.push(value.to_string());
    }
}

fn synthesize_strict_raw_tail_read_direct_answer(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|context| context.output_contract())?;
    if !route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
        || route.response_shape != crate::OutputResponseShape::Strict
        || !route.requires_content_evidence
        || route.delivery_required
    {
        return None;
    }
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok() && matches!(step.skill.as_str(), "fs_basic" | "system_basic"))
        .filter_map(|step| step.output.as_deref())
        .find_map(strict_raw_tail_read_answer_from_output)
        .map(|answer| answer.trim().to_string())
        .filter(|answer| !answer.is_empty())
}

fn strict_raw_tail_read_answer_from_output(output: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(output.trim()).ok()?;
    strict_raw_tail_read_answer_from_value(&value)
}

fn strict_raw_tail_read_answer_from_value(value: &Value) -> Option<String> {
    if let Some(answer) = strict_raw_tail_read_answer_from_flat_value(value) {
        return Some(answer);
    }
    value
        .get("extra")
        .and_then(strict_raw_tail_read_answer_from_value)
}

fn strict_raw_tail_read_answer_from_flat_value(value: &Value) -> Option<String> {
    if !matches!(
        value.get("action").and_then(Value::as_str),
        Some("read_range" | "read_text_range")
    ) || value.get("mode").and_then(Value::as_str) != Some("tail")
    {
        return None;
    }
    let requested_n = value
        .get("requested_n")
        .or_else(|| value.get("n"))
        .or_else(|| value.get("count"))
        .and_then(Value::as_u64)?;
    if requested_n == 0 || requested_n > 50 {
        return None;
    }
    value
        .get("excerpt")
        .and_then(Value::as_str)
        .filter(|excerpt| !excerpt.trim().is_empty())?;
    let mut candidate = value.clone();
    let obj = candidate.as_object_mut()?;
    obj.insert(
        "action".to_string(),
        Value::String("read_range".to_string()),
    );
    if !obj.contains_key("requested_n") {
        obj.insert("requested_n".to_string(), Value::Number(requested_n.into()));
    }
    crate::agent_engine::observed_output::tail_read_range_direct_answer_candidate(
        &candidate.to_string(),
        false,
    )
}

pub(super) fn synthesize_evidence_policy_direct_observed_fallback_answer(
    state: &AppState,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|context| context.output_contract())?;
    crate::evidence_policy::final_answer_shape_for_output_contract(route)?;
    if crate::agent_engine::observed_output::route_disallows_direct_observation_passthrough(route) {
        return None;
    }
    if multiple_count_inventory_observations_need_synthesis(loop_state) {
        return None;
    }
    if synthesize_direct_fallback_would_passthrough_multiline_read_range(
        loop_state,
        agent_run_context,
    ) {
        return None;
    }
    synthesize_direct_observed_fallback_answer(state, loop_state, agent_run_context)
}

fn route_needs_synthesis_for_multi_observation_grounded_summary(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|context| context.output_contract()) else {
        return false;
    };
    if !route.requires_content_evidence || route.delivery_required {
        return false;
    }
    let Some(shape) = crate::evidence_policy::final_answer_shape_for_output_contract(route) else {
        return false;
    };
    if shape.class() != crate::evidence_policy::FinalAnswerShapeClass::GroundedSummary {
        return false;
    }
    successful_observation_step_count(loop_state) >= 2
}

fn successful_observation_step_count(loop_state: &LoopState) -> usize {
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| {
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
        .count()
}

pub(super) fn synthesize_direct_fallback_would_passthrough_multiline_read_range(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|context| context.output_contract()) else {
        return false;
    };
    if !route.requires_content_evidence
        || route.delivery_required
        || !matches!(
            route.response_shape,
            OutputResponseShape::Free
                | OutputResponseShape::Scalar
                | OutputResponseShape::Strict
                | OutputResponseShape::OneSentence
        )
    {
        return false;
    }
    let semantic_blocks_direct_passthrough = route.semantic_kind_is_unclassified()
        || route.semantic_kind.is_content_excerpt_summary()
        || (route.semantic_kind_is(crate::OutputSemanticKind::RawCommandOutput)
            && latest_round_plan_requests_synthesis(loop_state));
    if !semantic_blocks_direct_passthrough {
        return false;
    }
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok())
        .filter_map(|step| step.output.as_deref())
        .find_map(multiline_read_range_content_line_count)
        .is_some_and(|line_count| line_count > 1)
}

fn latest_round_plan_requests_synthesis(loop_state: &LoopState) -> bool {
    loop_state.round_traces.iter().rev().any(|round| {
        round.plan_result.as_ref().is_some_and(|plan| {
            plan.steps
                .iter()
                .any(|step| step.action_type == "synthesize_answer")
        })
    })
}

fn multiline_read_range_content_line_count(output: &str) -> Option<usize> {
    let value = serde_json::from_str::<Value>(output.trim()).ok()?;
    multiline_read_range_content_line_count_from_value(&value)
}

fn multiline_read_range_content_line_count_from_value(value: &Value) -> Option<usize> {
    if value
        .get("action")
        .and_then(Value::as_str)
        .is_some_and(|action| matches!(action, "read_range" | "read_text_range"))
    {
        if let Some(text) = value
            .get("content")
            .or_else(|| value.get("excerpt"))
            .and_then(Value::as_str)
        {
            return Some(
                text.lines()
                    .map(strip_read_range_line_prefix)
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .count(),
            );
        }
    }
    value
        .get("extra")
        .and_then(multiline_read_range_content_line_count_from_value)
}

pub(super) fn route_resolved_intent(agent_run_context: Option<&AgentRunContext>) -> String {
    agent_run_context
        .and_then(|ctx| {
            ctx.original_user_request
                .as_deref()
                .or(ctx.user_request.as_deref())
        })
        .map(str::trim)
        .filter(|intent| !intent.is_empty())
        .unwrap_or_default()
        .to_string()
}

pub(super) fn synthesize_failure_observed_facts(
    loop_state: &LoopState,
    refs_summary: &str,
) -> Vec<String> {
    let mut facts = vec![
        format!("synthesize_refs: {}", refs_summary.trim()),
        format!(
            "observed_steps_count: {}",
            loop_state.executed_step_results.len()
        ),
    ];
    let mut recent_steps = loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| {
            !matches!(
                step.skill.as_str(),
                "respond" | "think" | "synthesize_answer"
            )
        })
        .take(4)
        .collect::<Vec<_>>();
    recent_steps.reverse();
    for step in recent_steps {
        let mut parts = vec![
            format!("skill={}", step.skill.trim()),
            format!("status={}", step.status.as_str()),
        ];
        if let Some(output) = step
            .output
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            parts.push(format!(
                "output_excerpt={}",
                crate::truncate_for_agent_trace(output)
            ));
        }
        if let Some(error) = step
            .error
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            parts.push(format!(
                "error_summary={}",
                crate::truncate_for_agent_trace(error)
            ));
        }
        facts.push(format!("observed_step: {}", parts.join(", ")));
    }
    facts
}

pub(super) fn step_has_observable_synthesis_fact(
    step: &crate::executor::StepExecutionResult,
) -> bool {
    if step.is_ok() {
        return step
            .output
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());
    }
    step.error
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some_and(|err| {
            crate::skills::is_observable_run_cmd_error(&step.skill, err)
                || crate::skills::is_recoverable_skill_error(&step.skill, err)
        })
}

pub(super) fn synthesize_failure_should_replan(loop_state: &LoopState) -> bool {
    let previous_synthesis_failures = loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.skill == "synthesize_answer" && !step.is_ok())
        .count();
    if previous_synthesis_failures > 0 {
        return false;
    }
    loop_state.executed_step_results.iter().any(|step| {
        !matches!(
            step.skill.as_str(),
            "respond" | "think" | "synthesize_answer"
        ) && step_has_observable_synthesis_fact(step)
    })
}
