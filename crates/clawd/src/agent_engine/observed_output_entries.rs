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
    let normalized = crate::skills::normalize_skill_error_for_user(&step.skill, body);
    let sanitized = crate::visible_text::sanitize_user_visible_text(&normalized);
    (!sanitized.trim().is_empty()).then(|| {
        format!(
            "execution_status: error\nerror_summary: {}",
            sanitized.trim()
        )
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

pub(super) fn normalized_structured_observed_fact_allows_artifact_filter_bypass(
    skill: &str,
    output: &str,
) -> bool {
    (skill == "archive_basic" && output.trim_start().starts_with("archive_basic action="))
        || (matches!(skill, "fs_basic" | "system_basic")
            && output.trim_start().starts_with("read_range "))
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

pub(super) fn git_repository_state_facts_entry(
    loop_state: &LoopState,
    route: Option<&crate::RouteResult>,
) -> Option<String> {
    let route = route?;
    if !super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::GitRepositoryState,
    ) {
        return None;
    }
    let idx = latest_successful_step_index(loop_state, |step| step.skill == "git_basic")?;
    let body = loop_state.executed_step_results[idx]
        .output
        .as_deref()
        .map(str::trim)
        .filter(|body| !body.is_empty())?;
    let branch = latest_git_current_branch(loop_state);
    let observation = git_basic_observation_text_candidates(body)
        .into_iter()
        .find_map(|candidate| {
            git_repository_state_observation_from_status_output(&candidate, branch.as_deref())
        })?;
    let mut fields = Vec::new();
    if let Some(branch) = observation
        .branch
        .as_deref()
        .map(str::trim)
        .filter(|branch| !branch.is_empty())
    {
        fields.push(format!("git.branch={branch}"));
    }
    fields.push(format!(
        "git.worktree={}",
        if observation.dirty { "dirty" } else { "clean" }
    ));
    fields.push(format!(
        "git.changed.count={}",
        observation.changed_entries.len()
    ));
    Some(format!(
        "### git_repository_state_facts\n{}",
        fields.join("\n")
    ))
}

pub(super) fn compound_listing_content_delivery_guard_entry(
    loop_state: &LoopState,
    route: Option<&crate::RouteResult>,
) -> Option<String> {
    let route = route?;
    if !route.output_contract.requires_content_evidence || route.output_contract.delivery_required {
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

pub(super) fn execution_failed_step_guard_entry(
    loop_state: &LoopState,
    route: Option<&crate::RouteResult>,
) -> Option<String> {
    let route = route?;
    if !super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::ExecutionFailedStep,
    ) {
        return None;
    }
    let mut lines = vec![
        format!(
            "final_answer_shape={}",
            crate::evidence_policy::final_answer_shape_for_route(route)
                .map(crate::evidence_policy::FinalAnswerShape::as_str)
                .unwrap_or("failed_step_with_evidence")
        ),
        "answer_scope=failed_steps_only".to_string(),
        "successful_step_outputs_are_not_final_answer=true".to_string(),
    ];
    let mut failed_count = 0usize;
    let mut success_count = 0usize;
    let mut seen_failed_actions = std::collections::BTreeSet::new();
    for step in loop_state
        .executed_step_results
        .iter()
        .filter(|step| is_observation_step_for_answer_synthesis(step))
    {
        if step.is_ok() {
            let Some(output) = step
                .output
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
            else {
                continue;
            };
            success_count += 1;
            lines.push(format!(
                "success_step.{success_count}.step_id={}",
                step.step_id
            ));
            lines.push(format!("success_step.{success_count}.skill={}", step.skill));
            lines.push(format!(
                "success_step.{success_count}.output_is_not_answer={}",
                trim_for_observed_prompt(
                    &crate::visible_text::sanitize_user_visible_text(
                        &normalized_success_body_for_observed_output(output)
                    )
                    .replace('\n', " "),
                    240,
                )
            ));
            continue;
        }
        let Some(error) = step
            .error
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            continue;
        };
        let structured = crate::skills::parse_structured_skill_error(error);
        if structured
            .as_ref()
            .is_some_and(structured_error_is_contract_policy_gap)
        {
            continue;
        }
        let dedupe_key = execution_failed_step_dedupe_key(step, error, structured.as_ref());
        if !seen_failed_actions.insert(dedupe_key) {
            continue;
        }
        failed_count += 1;
        lines.push(format!(
            "failed_step.{failed_count}.step_id={}",
            step.step_id
        ));
        lines.push(format!("failed_step.{failed_count}.skill={}", step.skill));
        if let Some(structured) = structured {
            lines.push(format!(
                "failed_step.{failed_count}.error_kind={}",
                structured.error_kind
            ));
            if let Some(extra) = structured.extra.as_ref() {
                push_failed_step_json_string_field(
                    &mut lines,
                    failed_count,
                    "command",
                    extra.get("command"),
                );
                push_failed_step_json_string_field(
                    &mut lines,
                    failed_count,
                    "exit_category",
                    extra.get("exit_category"),
                );
                push_failed_step_json_string_field(
                    &mut lines,
                    failed_count,
                    "exit_classification_source",
                    extra.get("exit_classification_source"),
                );
                if let Some(exit_code) = extra.get("exit_code").and_then(|value| value.as_i64()) {
                    lines.push(format!("failed_step.{failed_count}.exit_code={exit_code}"));
                }
                push_failed_step_json_string_field(
                    &mut lines,
                    failed_count,
                    "stderr",
                    extra.get("stderr"),
                );
            }
            let summary = crate::visible_text::sanitize_user_visible_text(
                &crate::skills::normalize_skill_error_for_user(&step.skill, error),
            )
            .replace('\n', " ");
            if !summary.trim().is_empty() {
                lines.push(format!(
                    "failed_step.{failed_count}.error_summary={}",
                    trim_for_observed_prompt(&summary, 360)
                ));
            }
        } else {
            let summary = crate::visible_text::sanitize_user_visible_text(error).replace('\n', " ");
            if !summary.trim().is_empty() {
                lines.push(format!(
                    "failed_step.{failed_count}.error_summary={}",
                    trim_for_observed_prompt(&summary, 360)
                ));
            }
        }
    }
    (failed_count > 0).then(|| format!("### execution_failed_step_guard\n{}", lines.join("\n")))
}

fn structured_error_is_contract_policy_gap(
    structured: &crate::skills::StructuredSkillError,
) -> bool {
    matches!(
        structured.error_kind.as_str(),
        "contract_action_rejected" | "contract_arg_rejected" | "contract_policy_violation"
    )
}

pub(super) fn execution_failed_step_dedupe_key(
    step: &crate::executor::StepExecutionResult,
    error: &str,
    structured: Option<&crate::skills::StructuredSkillError>,
) -> String {
    if let Some(structured) = structured {
        let command = structured
            .extra
            .as_ref()
            .and_then(|extra| extra.get("command"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let exit_code = structured
            .extra
            .as_ref()
            .and_then(|extra| extra.get("exit_code"))
            .and_then(serde_json::Value::as_i64);
        if let Some(command) = command {
            return format!(
                "{}:command:{command}:kind:{}:exit:{exit_code:?}",
                step.skill, structured.error_kind
            );
        }
        return format!(
            "{}:kind:{}:summary:{}",
            step.skill,
            structured.error_kind,
            structured.error_text.trim()
        );
    }
    format!("{}:error:{}", step.skill, error.trim())
}

pub(super) fn push_failed_step_json_string_field(
    lines: &mut Vec<String>,
    failed_count: usize,
    field: &str,
    value: Option<&serde_json::Value>,
) {
    let Some(value) = value
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    let sanitized = crate::visible_text::sanitize_user_visible_text(value).replace('\n', " ");
    if sanitized.trim().is_empty() {
        return;
    }
    lines.push(format!(
        "failed_step.{failed_count}.{field}={}",
        trim_for_observed_prompt(&sanitized, 360)
    ));
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

pub(super) fn route_observation_facts_entry(
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let ctx = agent_run_context?;
    let route = ctx.route_result.as_ref()?;
    if !super::output_route_policy::route_contract_marker_is(
        route,
        crate::OutputSemanticKind::ExistenceWithPathSummary,
    ) {
        return None;
    }
    let resolved_path = ctx
        .auto_locator_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .or_else(|| {
            let hint = route.output_contract.locator_hint.trim();
            (!hint.is_empty()).then_some(hint)
        })?;
    Some(format!(
        "### route_contract_facts\nresolved_target_path: {resolved_path}\npath_rule: use resolved_target_path as the target file path; do not infer the target path from file content fields such as WorkingDirectory."
    ))
}

pub(super) fn cross_turn_synthesis_allowed(agent_run_context: Option<&AgentRunContext>) -> bool {
    matches!(
        agent_run_context
            .and_then(|ctx| ctx.turn_analysis.as_ref())
            .and_then(|analysis| analysis.target_task_policy),
        Some(crate::intent_router::TargetTaskPolicy::ReuseActive)
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
