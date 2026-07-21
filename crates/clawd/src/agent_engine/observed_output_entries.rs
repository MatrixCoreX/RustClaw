use super::*;

pub(super) fn observed_error_step_body(
    step: &crate::executor::StepExecutionResult,
    body: &str,
) -> Option<String> {
    if !crate::skills::is_observable_run_cmd_error(&step.skill, body)
        && !crate::skills::is_recoverable_skill_error(&step.skill, body)
    {
        return None;
    }
    let observation = crate::skills::skill_error_machine_observation(&step.skill, body)
        .unwrap_or_else(|| body.trim().to_string());
    let sanitized = crate::visible_text::sanitize_user_visible_text(&observation);
    (!sanitized.trim().is_empty()).then(|| {
        [
            "execution_status:error".to_string(),
            format!("error_observation:{}", sanitized.trim()),
        ]
        .join("\n")
    })
}

pub(super) fn observed_step_body(step: &crate::executor::StepExecutionResult) -> Option<String> {
    let body = if step.is_ok() {
        step.output.as_deref()
    } else {
        step.error.as_deref().or(step.output.as_deref())
    }
    .map(str::trim)
    .filter(|text| !text.is_empty())?;
    if !step.is_ok() {
        return observed_error_step_body(step, body);
    }
    if let Some(normalized) = structured_observed_body(&step.skill, body) {
        let sanitized = crate::visible_text::sanitize_user_visible_text(&normalized);
        return (!sanitized.trim().is_empty()).then_some(sanitized);
    }
    if let Some(normalized) = system_basic_structured_doc_observed_body(&step.skill, body) {
        let sanitized = crate::visible_text::sanitize_user_visible_text(&normalized);
        return (!sanitized.trim().is_empty()).then_some(sanitized);
    }
    if step.skill == "run_cmd" && run_cmd_body_allows_command_output_observation(body) {
        let sanitized = crate::visible_text::sanitize_user_visible_text(body);
        return (!sanitized.trim().is_empty())
            .then(|| format!("command_output:\n{}", sanitized.trim()));
    }
    if crate::finalize::classify_observed_content_status(body)
        != crate::finalize::ObservedContentStatus::ContentAvailable
    {
        return None;
    }
    let sanitized = crate::visible_text::sanitize_user_visible_text(body);
    (!sanitized.trim().is_empty()).then_some(sanitized)
}

pub(super) fn observed_step_entry(step: &crate::executor::StepExecutionResult) -> Option<String> {
    let output = observed_step_body(step)?;
    let allows_artifact_filter_bypass =
        normalized_structured_observed_fact_allows_artifact_filter_bypass(&step.skill, &output);
    if !allows_artifact_filter_bypass
        && (crate::finalize::looks_like_planner_artifact(&output)
            || crate::finalize::looks_like_internal_trace_artifact(&output))
    {
        return None;
    }
    Some(format!(
        "### {} skill({})\n{}",
        step.step_id,
        step.skill,
        trim_for_observed_prompt(&output, 1800)
    ))
}

pub(in crate::agent_engine) fn latest_structured_capability_observation(
    loop_state: &LoopState,
) -> Option<String> {
    loop_state
        .capability_results
        .iter()
        .rev()
        .find_map(|result| {
            if result.status != claw_core::capability_result::CapabilityResultStatus::Ok {
                return None;
            }
            let normalized =
                structured_observed_body(&result.capability, &result.data.to_string())?;
            let sanitized = crate::visible_text::sanitize_user_visible_text(&normalized);
            (!sanitized.trim().is_empty()).then_some(sanitized)
        })
}

pub(super) fn normalized_structured_observed_fact_allows_artifact_filter_bypass(
    skill: &str,
    output: &str,
) -> bool {
    (matches!(skill, "fs_basic" | "system_basic") && output.trim_start().starts_with("read_range "))
        || (skill == "run_cmd" && output.trim_start().starts_with("command_output:\n"))
}

fn run_cmd_body_allows_command_output_observation(body: &str) -> bool {
    let trimmed = body.trim_start();
    if trimmed.is_empty()
        || trimmed.starts_with("[TOOL_CALL]")
        || trimmed.contains("<minimax:tool_call")
        || trimmed.contains("<invoke ")
        || crate::finalize::looks_like_internal_trace_artifact(trimmed)
    {
        return false;
    }
    if matches!(trimmed.as_bytes().first(), Some(b'{') | Some(b'['))
        && crate::finalize::looks_like_planner_artifact(trimmed)
    {
        return false;
    }
    true
}

pub(super) fn observed_output_entries(loop_state: &LoopState) -> Vec<String> {
    let latest_listing_idx = loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .rfind(|(_, step)| {
            is_observation_step_for_answer_synthesis(step)
                && step.skill == "list_dir"
                && observed_step_entry(step).is_some()
        })
        .map(|(idx, _)| idx);
    let mut selected_indices = latest_listing_idx.into_iter().collect::<Vec<_>>();
    let mut recent_non_listing = loop_state
        .executed_step_results
        .iter()
        .enumerate()
        .filter(|(_, step)| {
            is_observation_step_for_answer_synthesis(step)
                && step.skill != "list_dir"
                && observed_step_entry(step).is_some()
        })
        .map(|(idx, _)| idx)
        .collect::<Vec<_>>();
    recent_non_listing = retain_latest_observation_indices_by_supersede_key(
        &loop_state.executed_step_results,
        recent_non_listing,
    );
    if recent_non_listing.len() > 4 {
        recent_non_listing = recent_non_listing.split_off(recent_non_listing.len() - 4);
    }
    selected_indices.extend(recent_non_listing);
    selected_indices.sort_unstable();
    selected_indices.dedup();
    selected_indices
        .into_iter()
        .filter_map(|idx| observed_step_entry(&loop_state.executed_step_results[idx]))
        .collect()
}

pub(super) fn compound_listing_content_delivery_guard_entry(
    loop_state: &LoopState,
    route: Option<&crate::IntentOutputContract>,
) -> Option<String> {
    let route = route?;
    if !route.requires_content_evidence || route.delivery_required {
        return None;
    }
    let names = latest_inventory_dir_names(loop_state)?;
    if names.is_empty() || !has_current_task_content_excerpt_observation(loop_state) {
        return None;
    }
    Some(format!(
        "### delivery_completeness_guard\ncurrent_task_observed_listing_names: {}\ncurrent_task_observed_content_excerpt: present\nIf the original request requires both listing/candidate delivery and synthesis, judgment, summary, or comparison, the final answer must include both components in the requested response shape.",
        names.join(", ")
    ))
}

pub(super) fn latest_inventory_dir_names(loop_state: &LoopState) -> Option<Vec<String>> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.status == crate::executor::StepExecutionStatus::Ok)
        .filter_map(|step| step.output.as_deref())
        .filter_map(|output| serde_json::from_str::<serde_json::Value>(output.trim()).ok())
        .find_map(|value| inventory_dir_names(&value))
}

pub(super) fn has_current_task_content_excerpt_observation(loop_state: &LoopState) -> bool {
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.status == crate::executor::StepExecutionStatus::Ok)
        .filter_map(|step| step.output.as_deref())
        .filter_map(|output| serde_json::from_str::<serde_json::Value>(output.trim()).ok())
        .any(|value| {
            matches!(
                value.get("action").and_then(|value| value.as_str()),
                Some("read_range" | "read_text_range")
            ) && value
                .get("excerpt")
                .or_else(|| value.get("content"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .is_some_and(|text| !text.is_empty())
        })
}

pub(super) fn retain_latest_observation_indices_by_supersede_key(
    steps: &[crate::executor::StepExecutionResult],
    indices: Vec<usize>,
) -> Vec<usize> {
    let mut seen = std::collections::HashSet::new();
    let mut kept = Vec::with_capacity(indices.len());
    for idx in indices.into_iter().rev() {
        let Some(step) = steps.get(idx) else {
            continue;
        };
        let Some(key) = observation_supersede_key(step) else {
            kept.push(idx);
            continue;
        };
        if seen.insert(key) {
            kept.push(idx);
        }
    }
    kept.reverse();
    kept
}

pub(super) fn observation_supersede_key(
    step: &crate::executor::StepExecutionResult,
) -> Option<String> {
    if !step.is_ok() {
        return None;
    }
    let body = step.output.as_deref()?.trim();
    if body.is_empty() {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let action = value
        .get("action")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let is_range_read = matches!(
        action,
        "read_range" | "read_text_range" | "read_file" | "parse_doc"
    );
    if !is_range_read {
        return None;
    }
    let path = value
        .get("resolved_path")
        .or_else(|| value.get("path"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())?;
    Some(format!(
        "file_content:{}:{path}",
        action_family_for_supersede(action)
    ))
}

pub(super) fn action_family_for_supersede(action: &str) -> &'static str {
    match action {
        "read_range" | "read_text_range" | "read_file" | "parse_doc" => "read_content",
        _ => "other",
    }
}

pub(super) fn is_observation_step_for_answer_synthesis(
    step: &crate::executor::StepExecutionResult,
) -> bool {
    !matches!(
        step.skill.as_str(),
        "respond" | "synthesize_answer" | "think" | "answer_verifier"
    )
}

pub(super) fn cross_turn_synthesis_allowed(agent_run_context: Option<&AgentRunContext>) -> bool {
    matches!(
        agent_run_context
            .and_then(|ctx| ctx.turn_analysis.as_ref())
            .and_then(|analysis| analysis.target_task_policy),
        Some(crate::turn_context::TargetTaskPolicy::ReuseActive)
    )
}

pub(super) fn recent_generated_output_from_user_request(user_request: &str) -> Option<String> {
    const MARKER: &str = "Most recent generated output:\n";
    let (_, tail) = user_request.split_once(MARKER)?;
    let stop_idx = [
        "\n\nContinuity rules:",
        "\n\nStructured task updates:",
        "\n\nNew user instruction:",
        "\n\n### SESSION_ALIAS_BINDINGS",
    ]
    .iter()
    .filter_map(|marker| tail.find(marker))
    .min()
    .unwrap_or(tail.len());
    let output = tail[..stop_idx].trim();
    if output.is_empty()
        || output == "<none>"
        || crate::finalize::looks_like_planner_artifact(output)
        || crate::finalize::looks_like_internal_trace_artifact(output)
    {
        return None;
    }
    let sanitized = crate::visible_text::sanitize_user_visible_text(output);
    (!sanitized.trim().is_empty()).then_some(sanitized)
}

pub(super) fn cross_turn_observed_output_entries(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Vec<String> {
    if !cross_turn_synthesis_allowed(agent_run_context) {
        return Vec::new();
    }

    if let Some(recent_output) = agent_run_context
        .and_then(|ctx| ctx.user_request.as_deref())
        .and_then(recent_generated_output_from_user_request)
    {
        return vec![format!(
            "### prior_turn_observed_output\n{}",
            trim_for_observed_prompt(&recent_output, 1800)
        )];
    }

    if let Some(cross_turn_context) = loop_state
        .output_vars
        .get("cross_turn_recent_execution_context")
        .map(String::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty() && *text != "<none>")
    {
        return vec![format!(
            "### prior_turn_execution_context\n{}",
            trim_for_observed_prompt(
                &crate::visible_text::sanitize_user_visible_text(cross_turn_context),
                1800
            )
        )];
    }

    Vec::new()
}

pub(crate) fn has_observed_answer_candidates(loop_state: &LoopState) -> bool {
    !observed_output_entries(loop_state).is_empty()
}
