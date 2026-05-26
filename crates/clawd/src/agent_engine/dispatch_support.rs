use serde_json::Value;
use std::path::Path;
use tracing::{debug, info};

use super::{
    append_delivery_message, append_progress_hint, build_safe_skill_args_summary,
    encode_progress_i18n, execute_prepared_skill_action, normalize_skill_arg_aliases,
    register_step_output, resolve_arg_string, resolve_arg_value,
    rewrite_args_with_auto_locator_path, rewrite_run_cmd_with_written_aliases,
    rewrite_tool_path_with_written_aliases, ActionLoopDecision, AgentLoopGuardPolicy,
    AgentRunContext, AppState, ClaimedTask, LoopState, RespondActionOutcome, SkillActionOutcome,
    WriteFileEffectivePath, PROGRESS_ARGS_SUMMARY_MAX_LEN,
};
use crate::{AgentAction, OutputResponseShape};

pub(super) fn apply_skill_action_outcome(
    loop_state: &mut LoopState,
    executed_actions: &mut usize,
    ended_with_user_visible_output: &mut bool,
    outcome: SkillActionOutcome,
) -> ActionLoopDecision {
    *ended_with_user_visible_output |= outcome.ended_with_user_visible_output;
    *executed_actions += 1;
    loop_state.total_steps_executed += 1;
    if outcome.continue_in_round {
        return ActionLoopDecision::ContinueRound;
    }
    if let Some(reason) = outcome.stop_signal {
        return ActionLoopDecision::StopRound(reason);
    }
    ActionLoopDecision::NextAction
}

pub(super) fn apply_respond_action_outcome(
    loop_state: &mut LoopState,
    executed_actions: &mut usize,
    ended_with_user_visible_output: &mut bool,
    outcome: RespondActionOutcome,
) -> ActionLoopDecision {
    *ended_with_user_visible_output |= outcome.ended_with_user_visible_output;
    *executed_actions += 1;
    loop_state.total_steps_executed += 1;
    if outcome.should_stop {
        return ActionLoopDecision::StopRound(outcome.stop_signal.unwrap_or_default());
    }
    ActionLoopDecision::NextAction
}

fn synthesize_answer_allows_direct_fallback(evidence_refs: &[String]) -> bool {
    evidence_refs.is_empty()
        || evidence_refs
            .iter()
            .all(|reference| reference.trim().eq_ignore_ascii_case("last_output"))
}

fn synthesize_route_allows_direct_fallback(agent_run_context: Option<&AgentRunContext>) -> bool {
    let Some(route) = agent_run_context.and_then(|context| context.route_result.as_ref()) else {
        return true;
    };
    if crate::agent_engine::observed_output::route_requires_synthesized_delivery(route) {
        return false;
    }
    if route.ask_mode.is_plain_act()
        && route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && route.output_contract.semantic_kind == crate::OutputSemanticKind::None
    {
        return true;
    }
    if matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::FileNames
            | crate::OutputSemanticKind::DirectoryNames
            | crate::OutputSemanticKind::FilePaths
            | crate::OutputSemanticKind::ConfigValidation
    ) {
        return true;
    }
    if route.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
        && route.output_contract.response_shape == crate::OutputResponseShape::Strict
    {
        return false;
    }
    matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar
            | crate::OutputResponseShape::Strict
            | crate::OutputResponseShape::FileToken
    ) || route.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
}

fn synthesize_direct_observed_fallback_answer(
    state: &AppState,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    crate::agent_engine::observed_output::extract_direct_answer_from_generic_output_i18n(
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

fn synthesize_contract_matrix_direct_observed_fallback_answer(
    state: &AppState,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context.and_then(|context| context.route_result.as_ref())?;
    crate::contract_matrix::final_answer_shape_for_output_contract(&route.output_contract)?;
    synthesize_direct_observed_fallback_answer(state, loop_state, agent_run_context)
}

fn synthesize_direct_fallback_would_passthrough_multiline_read_range(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|context| context.route_result.as_ref()) else {
        return false;
    };
    if !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !matches!(
            route.output_contract.response_shape,
            OutputResponseShape::Scalar
                | OutputResponseShape::Strict
                | OutputResponseShape::OneSentence
        )
        || !matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None | crate::OutputSemanticKind::ContentExcerptSummary
        )
    {
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

fn multiline_read_range_content_line_count(output: &str) -> Option<usize> {
    let value = serde_json::from_str::<Value>(output.trim()).ok()?;
    let action = value.get("action").and_then(Value::as_str)?;
    if !matches!(action, "read_range" | "read_text_range") {
        return None;
    }
    let text = value
        .get("content")
        .or_else(|| value.get("excerpt"))
        .and_then(Value::as_str)?;
    Some(
        text.lines()
            .map(strip_markdown_read_line_prefix)
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .count(),
    )
}

fn deterministic_scalar_markdown_heading_answer(
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<String> {
    let route = agent_run_context?.route_result.as_ref()?;
    if route.output_contract.response_shape != OutputResponseShape::Scalar
        || route.output_contract.delivery_required
        || matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::FileNames
                | crate::OutputSemanticKind::DirectoryNames
                | crate::OutputSemanticKind::FilePaths
                | crate::OutputSemanticKind::DirectoryEntryGroups
                | crate::OutputSemanticKind::ScalarCount
                | crate::OutputSemanticKind::RawCommandOutput
        )
    {
        return None;
    }
    let output = loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| step.is_ok())
        .filter_map(|step| step.output.as_deref())
        .find(|output| {
            output.contains("\"read_range\"") || output.contains("\"read_text_range\"")
        })?;
    markdown_heading_from_read_output(output)
}

fn markdown_heading_from_read_output(output: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(output.trim()).ok()?;
    let text = value
        .get("content")
        .or_else(|| value.get("excerpt"))
        .and_then(Value::as_str)?;
    standalone_markdown_heading_from_text(text)
}

fn standalone_markdown_heading_from_text(text: &str) -> Option<String> {
    let mut heading: Option<String> = None;
    for line in text.lines() {
        let stripped = strip_markdown_read_line_prefix(line).trim();
        if stripped.is_empty() {
            continue;
        }
        if let Some(candidate) = markdown_heading_from_line(stripped) {
            if heading.is_some() {
                return None;
            }
            heading = Some(candidate);
            continue;
        }
        if markdown_line_is_non_answer_separator_heading(stripped) {
            continue;
        }
        return None;
    }
    heading
}

fn strip_markdown_read_line_prefix(line: &str) -> &str {
    let trimmed = line.trim();
    if let Some((prefix, rest)) = trimmed.split_once('|') {
        if !prefix.is_empty() && prefix.chars().all(|ch| ch.is_ascii_digit()) {
            return rest.trim();
        }
    }
    line
}

fn markdown_heading_from_line(line: &str) -> Option<String> {
    let trimmed = strip_markdown_read_line_prefix(line).trim();
    let hashes = trimmed.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = trimmed.get(hashes..)?.trim();
    (!rest.is_empty()).then(|| rest.to_string())
}

fn markdown_line_is_non_answer_separator_heading(line: &str) -> bool {
    let trimmed = strip_markdown_read_line_prefix(line).trim();
    let hashes = trimmed.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&hashes) {
        return false;
    }
    trimmed.get(hashes..).map(str::trim).is_some_and(|rest| {
        !rest.is_empty()
            && rest
                .chars()
                .all(|ch| matches!(ch, '=' | '-' | '_' | '*' | '#'))
    })
}

fn synthesize_user_language_source<'a>(
    user_text: &'a str,
    agent_run_context: Option<&'a AgentRunContext>,
) -> &'a str {
    agent_run_context
        .and_then(|context| {
            context
                .original_user_request
                .as_deref()
                .or(context.user_request.as_deref())
        })
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or(user_text)
}

fn deterministic_observed_execution_status_answer(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
) -> Option<String> {
    let observed_steps = loop_state
        .executed_step_results
        .iter()
        .filter(|step| {
            !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think"
            )
        })
        .collect::<Vec<_>>();
    if observed_steps.last().is_some_and(|step| step.is_ok()) {
        return None;
    }
    if observed_steps.len() < 2 || !observed_steps.iter().any(|step| !step.is_ok()) {
        return None;
    }

    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let prefer_english = language_hint == "en";
    let mut parts = Vec::new();
    for (idx, step) in observed_steps.iter().enumerate() {
        let step_no = idx + 1;
        let skill = step.skill.trim();
        if step.is_ok() {
            if prefer_english {
                parts.push(format!("Step {step_no} `{skill}` succeeded."));
            } else {
                parts.push(format!("第 {step_no} 步 `{skill}` 成功。"));
            }
            continue;
        }
        let error = step
            .error
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(|text| {
                if crate::skills::parse_structured_skill_error(text).is_some()
                    || crate::skills::is_recoverable_skill_error(skill, text)
                {
                    crate::skills::normalize_skill_error_for_user(skill, text)
                } else {
                    text.to_string()
                }
            })
            .unwrap_or_else(|| {
                if prefer_english {
                    "execution failed without a clear error".to_string()
                } else {
                    "执行失败，但没有返回明确错误".to_string()
                }
            });
        let error = crate::truncate_for_agent_trace(
            &crate::visible_text::sanitize_user_visible_text(&error).replace('\n', " "),
        );
        if prefer_english {
            parts.push(format!("Step {step_no} `{skill}` failed: {error}."));
        } else {
            parts.push(format!("第 {step_no} 步 `{skill}` 失败：{error}。"));
        }
    }
    Some(parts.join(" "))
}

fn rewrite_response_with_written_aliases(text: &str, loop_state: &LoopState) -> String {
    let mut out = text.to_string();
    for (alias, effective) in &loop_state.written_file_aliases {
        let file_alias = format!("FILE:{alias}");
        let file_effective = format!("FILE:{effective}");
        let image_alias = format!("IMAGE_FILE:{alias}");
        let image_effective = format!("IMAGE_FILE:{effective}");
        out = out.replace(&file_alias, &file_effective);
        out = out.replace(&image_alias, &image_effective);
        let trimmed = out.trim();
        if trimmed == alias {
            return effective.clone();
        }
        if trimmed == format!("`{alias}`") {
            return effective.clone();
        }
    }
    out
}

fn has_remaining_action_after(
    actions: &[AgentAction],
    current_idx: usize,
    max_steps: usize,
) -> bool {
    actions
        .iter()
        .take(max_steps.max(1))
        .skip(current_idx + 1)
        .any(|action| !matches!(action, AgentAction::Think { .. }))
}

fn has_remaining_action_after_full(actions: &[AgentAction], current_idx: usize) -> bool {
    actions
        .iter()
        .skip(current_idx + 1)
        .any(|action| !matches!(action, AgentAction::Think { .. }))
}

fn remaining_actions_are_discussion_only(
    actions: &[AgentAction],
    current_idx: usize,
    max_steps: usize,
) -> bool {
    let remaining = actions
        .iter()
        .take(max_steps.max(1))
        .skip(current_idx + 1)
        .filter(|action| !matches!(action, AgentAction::Think { .. }))
        .collect::<Vec<_>>();
    !remaining.is_empty()
        && remaining.iter().all(|action| match action {
            AgentAction::Respond { .. } | AgentAction::SynthesizeAnswer { .. } => true,
            _ => false,
        })
}

fn remaining_actions_after_round_cap_are_discussion_only(
    actions: &[AgentAction],
    current_idx: usize,
    max_steps: usize,
) -> bool {
    if current_idx + 1 < max_steps.max(1) {
        return false;
    }
    let remaining = actions
        .iter()
        .skip(current_idx + 1)
        .filter(|action| !matches!(action, AgentAction::Think { .. }))
        .collect::<Vec<_>>();
    !remaining.is_empty()
        && remaining.iter().all(|action| match action {
            AgentAction::Respond { .. } | AgentAction::SynthesizeAnswer { .. } => true,
            _ => false,
        })
}

fn run_cmd_should_continue_after_split_failure(args: Option<&Value>) -> bool {
    args.and_then(|value| value.get(super::CLAWD_CONTINUE_ON_ERROR_ARG))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn run_cmd_is_literal_user_command(args: Option<&Value>) -> bool {
    args.and_then(|value| value.get(super::CLAWD_LITERAL_COMMAND_ARG))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn run_cmd_literal_failure_is_repairable(args: Option<&Value>) -> bool {
    args.and_then(|value| value.get(super::CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn missing_target_failure_is_repairable(args: Option<&Value>) -> bool {
    args.and_then(|value| value.get(super::CLAWD_MISSING_TARGET_REPAIRABLE_ARG))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn structured_error_kind(err: &str) -> Option<String> {
    crate::skills::parse_structured_skill_error(err).map(|structured| structured.error_kind)
}

fn planner_can_repair_structured_skill_error(err: &str) -> bool {
    structured_error_kind(err).is_some_and(|kind| {
        matches!(
            kind.as_str(),
            "unsupported_action"
                | "invalid_input"
                | "invalid_args"
                | "schema_error"
                | "missing_required_field"
                | "timeout"
                | "idle_timeout"
                | "spawn_failed"
                | "wait_failed"
                | "output_read_failed"
                | "status_unavailable"
        )
    })
}

fn structured_read_permission_denial_is_terminal(normalized_skill: &str, err: &str) -> bool {
    let Some(structured) = crate::skills::parse_structured_skill_error(err) else {
        return false;
    };
    if structured.error_kind != "permission_denied" {
        return false;
    }
    let effective_skill = if structured.skill.trim().is_empty() {
        normalized_skill
    } else {
        structured.skill.as_str()
    };
    matches!(
        effective_skill.to_ascii_lowercase().as_str(),
        "fs_basic" | "system_basic" | "read_file" | "list_dir"
    )
}

fn run_cmd_error_is_observable(normalized_skill: &str, err: &str) -> bool {
    if crate::skills::is_observable_run_cmd_error(normalized_skill, err) {
        return true;
    }
    if !normalized_skill.eq_ignore_ascii_case("run_cmd") {
        return false;
    }
    let err = err.to_ascii_lowercase();
    err.contains("command failed")
        || err.contains("exit code")
        || err.contains("command not found")
        || err.contains("timed out")
        || err.contains("timeout")
}

fn strip_internal_execution_args(args: &mut Value) {
    if let Some(obj) = args.as_object_mut() {
        obj.remove(super::CLAWD_CONTINUE_ON_ERROR_ARG);
        obj.remove(super::CLAWD_LITERAL_COMMAND_ARG);
        obj.remove(super::CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG);
        obj.remove(super::CLAWD_MISSING_TARGET_REPAIRABLE_ARG);
        obj.remove(crate::execution_recipe::CLAWD_VALIDATION_ARG);
    }
}

pub(super) fn classify_skill_failure_recovery(
    state: &AppState,
    actions: &[AgentAction],
    current_idx: usize,
    max_steps: usize,
    normalized_skill: &str,
    call_args: Option<&Value>,
    err: &str,
) -> Option<&'static str> {
    if structured_read_permission_denial_is_terminal(normalized_skill, err) {
        return Some("recoverable_failure_finalize");
    }
    if crate::skills::is_recoverable_skill_error(normalized_skill, err) {
        if has_remaining_action_after(actions, current_idx, max_steps)
            && !remaining_actions_are_discussion_only(actions, current_idx, max_steps)
        {
            return Some("recoverable_failure_continue_in_round");
        }
        if crate::skills::is_missing_target_skill_error(normalized_skill, err) {
            if missing_target_failure_is_repairable(call_args) {
                return Some("recoverable_failure_continue_round");
            }
            return Some("recoverable_failure_finalize");
        }
        if remaining_actions_after_round_cap_are_discussion_only(actions, current_idx, max_steps) {
            return Some("recoverable_failure_finalize");
        }
        if remaining_actions_are_discussion_only(actions, current_idx, max_steps) {
            return Some("recoverable_failure_continue_round");
        }
        return Some("recoverable_failure_continue_round");
    }
    if normalized_skill.eq_ignore_ascii_case("run_cmd")
        && run_cmd_should_continue_after_split_failure(call_args)
        && has_remaining_action_after(actions, current_idx, max_steps)
    {
        return Some("recoverable_failure_continue_in_round");
    }
    if normalized_skill.eq_ignore_ascii_case("run_cmd")
        && run_cmd_error_is_observable(normalized_skill, err)
        && !has_remaining_action_after(actions, current_idx, max_steps)
    {
        if current_idx > 0 && !has_remaining_action_after_full(actions, current_idx) {
            return Some("recoverable_failure_finalize");
        }
        if remaining_actions_after_round_cap_are_discussion_only(actions, current_idx, max_steps) {
            return Some("recoverable_failure_finalize");
        }
        if run_cmd_is_literal_user_command(call_args)
            && run_cmd_literal_failure_is_repairable(call_args)
        {
            return Some("recoverable_failure_continue_round");
        }
        if !run_cmd_is_literal_user_command(call_args)
            && !run_cmd_should_continue_after_split_failure(call_args)
        {
            return Some("recoverable_failure_continue_round");
        }
        return Some("recoverable_failure_finalize");
    }
    if normalized_skill.eq_ignore_ascii_case("run_cmd")
        && run_cmd_error_is_observable(normalized_skill, err)
        && !run_cmd_is_literal_user_command(call_args)
        && !run_cmd_should_continue_after_split_failure(call_args)
        && remaining_actions_are_discussion_only(actions, current_idx, max_steps)
    {
        return Some("recoverable_failure_continue_round");
    }
    if normalized_skill.eq_ignore_ascii_case("run_cmd")
        && run_cmd_error_is_observable(normalized_skill, err)
        && run_cmd_is_literal_user_command(call_args)
        && run_cmd_literal_failure_is_repairable(call_args)
        && remaining_actions_are_discussion_only(actions, current_idx, max_steps)
    {
        return Some("recoverable_failure_continue_round");
    }
    if planner_can_repair_structured_skill_error(err) {
        if has_remaining_action_after(actions, current_idx, max_steps)
            && !remaining_actions_are_discussion_only(actions, current_idx, max_steps)
        {
            return Some("recoverable_failure_continue_in_round");
        }
        return Some("recoverable_failure_continue_round");
    }
    if state.skill_is_retryable(normalized_skill)
        && !state.skill_invocation_requires_confirmation_policy(normalized_skill, call_args)
    {
        if has_remaining_action_after(actions, current_idx, max_steps) {
            return Some("recoverable_failure_continue_in_round");
        }
        if remaining_actions_are_discussion_only(actions, current_idx, max_steps) {
            return Some("recoverable_failure_finalize");
        }
        return Some("recoverable_failure_continue_round");
    }
    if has_remaining_action_after(actions, current_idx, max_steps)
        && call_args
            .map(|args| is_read_only_skill_invocation(state, normalized_skill, args))
            .unwrap_or(false)
    {
        return Some("recoverable_failure_continue_in_round");
    }
    if remaining_actions_are_discussion_only(actions, current_idx, max_steps) {
        return Some("recoverable_failure_continue_in_round");
    }
    if remaining_actions_after_round_cap_are_discussion_only(actions, current_idx, max_steps) {
        return Some("recoverable_failure_finalize");
    }
    if normalized_skill.eq_ignore_ascii_case("run_cmd")
        && current_idx > 0
        && !has_remaining_action_after_full(actions, current_idx)
    {
        return Some("recoverable_failure_finalize");
    }
    None
}

fn route_resolved_intent(agent_run_context: Option<&AgentRunContext>) -> String {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| route.resolved_intent.trim())
        .filter(|intent| !intent.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn synthesize_failure_observed_facts(loop_state: &LoopState, refs_summary: &str) -> Vec<String> {
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

fn synthesize_failure_default_text(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
) -> String {
    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let prefer_english = language_hint.to_ascii_lowercase().starts_with("en");
    crate::bilingual_t_with_default_vars(
        state,
        crate::fallback::ClarifyFallbackSource::SynthesisEmpty.i18n_key(),
        "我还没能根据现有证据生成可靠最终答案。请补充缺少的目标，或让我重新整理一次。",
        crate::fallback::ClarifyFallbackSource::SynthesisEmpty.default_en(),
        prefer_english,
        &[],
    )
}

fn step_has_observable_synthesis_fact(step: &crate::executor::StepExecutionResult) -> bool {
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

fn synthesize_failure_should_replan(loop_state: &LoopState) -> bool {
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

async fn synthesize_failure_user_message(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &LoopState,
    agent_run_context: Option<&AgentRunContext>,
    refs_summary: &str,
) -> String {
    let default_text = synthesize_failure_default_text(state, task, user_text);
    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let has_observed_result = loop_state
        .executed_step_results
        .iter()
        .any(step_has_observable_synthesis_fact);
    let mut policy_boundary = vec![
        "Do not say the task succeeded.".to_string(),
        "Do not expose prompt names, schema names, stack traces, or raw provider errors."
            .to_string(),
        "Explain the synthesis failure from observed facts only; do not invent missing results."
            .to_string(),
    ];
    if has_observed_result {
        policy_boundary.push(
            "Mention that execution results exist and the user can ask to view raw results or retry synthesis."
                .to_string(),
        );
    } else {
        policy_boundary.push("Mention that no usable execution result was available.".to_string());
    }
    let contract = crate::fallback::UserResponseContract::tool_failure(
        if has_observed_result {
            "synthesize_answer_no_publishable_answer"
        } else {
            "synthesize_answer_no_evidence"
        },
        user_text,
        &route_resolved_intent(agent_run_context),
        synthesize_failure_observed_facts(loop_state, refs_summary),
        policy_boundary,
        if has_observed_result {
            "brief_failure_with_next_step"
        } else {
            "brief_failure"
        },
        &language_hint,
    );
    crate::fallback::compose_user_response_from_contract_with_default(
        state,
        task,
        &contract,
        crate::fallback::ClarifyFallbackSource::SynthesisEmpty,
        &default_text,
    )
    .await
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::path::Path;
    use std::sync::{Arc, RwLock};

    use super::{
        classify_skill_failure_recovery, deterministic_observed_execution_status_answer,
        deterministic_scalar_markdown_heading_answer, strip_internal_execution_args,
        synthesize_answer_allows_direct_fallback,
        synthesize_contract_matrix_direct_observed_fallback_answer,
        synthesize_direct_fallback_would_passthrough_multiline_read_range,
        synthesize_direct_observed_fallback_answer, synthesize_failure_observed_facts,
        synthesize_failure_should_replan, synthesize_route_allows_direct_fallback,
        unresolved_file_token_delivery_artifact,
    };
    use crate::agent_engine::{AgentRunContext, LoopState};
    use crate::executor::{StepExecutionResult, StepExecutionStatus};
    use crate::{
        AgentAction, AgentRuntimeConfig, AppState, SkillViewsSnapshot, ToolsPolicy,
        DEFAULT_AGENT_ID,
    };
    use claw_core::config::{AgentConfig, ToolsConfig};
    use claw_core::skill_registry::SkillsRegistry;

    fn test_state_with_registry() -> AppState {
        let agents_by_id = HashMap::from([(
            DEFAULT_AGENT_ID.to_string(),
            AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
        )]);
        let registry = SkillsRegistry::load_from_path(Path::new("configs/skills_registry.toml"))
            .expect("load registry");
        AppState {
            core: crate::CoreServices {
                agents_by_id: Arc::new(agents_by_id),
                skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                    registry: Some(Arc::new(registry)),
                    skills_list: Arc::new(HashSet::new()),
                }))),
                ..crate::CoreServices::test_default()
            },
            skill_rt: crate::SkillRuntime {
                locator_scan_max_files: 200,
                tools_policy: Arc::new(
                    ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
                ),
                ..crate::SkillRuntime::test_default()
            },
            policy: crate::PolicyConfig::test_default(),
            worker: crate::WorkerConfig::test_default(),
            metrics: crate::TaskMetricsRegistry::default(),
            channels: crate::ChannelConfig::default(),
            reload_ctx: crate::ReloadContext::default(),
            ask_states: crate::AskStateRegistry::default(),
        }
    }

    #[test]
    fn retryable_run_cmd_failure_does_not_auto_continue_when_confirmation_policy_applies() {
        let state = test_state_with_registry();
        let actions = vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: serde_json::json!({"command":"resume_fail_cmd_001_xyz"}),
            },
            AgentAction::CallSkill {
                skill: "stock".to_string(),
                args: serde_json::json!({"symbol":"ETH"}),
            },
        ];

        assert_eq!(
            classify_skill_failure_recovery(
                &state,
                &actions,
                0,
                8,
                "run_cmd",
                Some(&serde_json::json!({"command":"resume_fail_cmd_001_xyz"})),
                "command not found",
            ),
            None
        );
    }

    #[test]
    fn split_sequence_run_cmd_failure_continues_to_remaining_run_cmd() {
        let state = test_state_with_registry();
        let actions = vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: serde_json::json!({
                    "command": "echo before",
                    "_clawd_continue_on_error": true
                }),
            },
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: serde_json::json!({
                    "command": "missing_cmd_from_split",
                    "_clawd_continue_on_error": true
                }),
            },
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: serde_json::json!({
                    "command": "echo after",
                    "_clawd_continue_on_error": true
                }),
            },
        ];

        assert_eq!(
            classify_skill_failure_recovery(
                &state,
                &actions,
                1,
                8,
                "run_cmd",
                Some(&serde_json::json!({
                    "command": "missing_cmd_from_split",
                    "_clawd_continue_on_error": true
                })),
                "command not found",
            ),
            Some("recoverable_failure_continue_in_round")
        );
    }

    #[test]
    fn internal_execution_args_are_removed_before_skill_call() {
        let mut args = serde_json::json!({
            "command": "echo visible",
            "_clawd_continue_on_error": true
        });

        strip_internal_execution_args(&mut args);

        assert_eq!(args, serde_json::json!({"command": "echo visible"}));
    }

    #[test]
    fn invalid_file_delivery_token_detects_embedded_runtime_observation() {
        let candidate = r#"FILE:/tmp/docs/{"action":"inventory_dir","counts":{"files":2},"names":["a.txt","b.txt"]}"#;

        assert!(unresolved_file_token_delivery_artifact(candidate));
        assert!(!unresolved_file_token_delivery_artifact(
            "FILE:/tmp/docs/a.txt"
        ));
        assert!(!unresolved_file_token_delivery_artifact(
            "请查看 /tmp/docs/a.txt"
        ));
    }

    #[test]
    fn failure_at_round_cap_with_terminal_discussion_remaining_finalizes() {
        let state = test_state_with_registry();
        let actions = vec![
            AgentAction::CallSkill {
                skill: "list_dir".to_string(),
                args: serde_json::json!({"path":"logs"}),
            },
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: serde_json::json!({"command":"definitely_missing_command"}),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["s1".to_string(), "s2".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];

        assert_eq!(
            classify_skill_failure_recovery(
                &state,
                &actions,
                1,
                2,
                "run_cmd",
                Some(&serde_json::json!({"command":"definitely_missing_command"})),
                "command not found",
            ),
            Some("recoverable_failure_finalize")
        );
    }

    #[test]
    fn terminal_run_cmd_failure_after_prior_command_finalizes_for_summary() {
        let state = test_state_with_registry();
        let actions = vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: serde_json::json!({"command":"echo READY"}),
            },
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: serde_json::json!({"command":"definitely_missing_command"}),
            },
        ];

        assert_eq!(
            classify_skill_failure_recovery(
                &state,
                &actions,
                1,
                4,
                "run_cmd",
                Some(&serde_json::json!({"command":"definitely_missing_command"})),
                "command not found",
            ),
            Some("recoverable_failure_finalize")
        );
    }

    #[test]
    fn single_literal_structured_run_cmd_failure_finalizes_as_observed_result() {
        let state = test_state_with_registry();
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({"command":"printf problem >&2; exit 7"}),
        }];
        let err = format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "run_cmd",
                "error_kind": "nonzero_exit",
                "error_text": "Command failed with exit code 7\nstderr:\nproblem",
                "extra": {
                    "command": "printf problem >&2; exit 7",
                    "exit_code": 7,
                    "stderr": "problem",
                    "output_truncated": false
                }
            })
        );

        assert_eq!(
            classify_skill_failure_recovery(
                &state,
                &actions,
                0,
                4,
                "run_cmd",
                Some(&serde_json::json!({
                    "command":"printf problem >&2; exit 7",
                    "_clawd_literal_command": true
                })),
                &err,
            ),
            Some("recoverable_failure_finalize")
        );
    }

    #[test]
    fn permission_failure_without_remaining_action_finalizes_without_shell_fallback() {
        let state = test_state_with_registry();
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({"action":"read_range","path":"/root/secret.txt"}),
        }];
        let err = format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "system_basic",
                "error_kind": "permission_denied",
                "error_text": "permission denied: /root/secret.txt"
            })
        );

        assert_eq!(
            classify_skill_failure_recovery(
                &state,
                &actions,
                0,
                4,
                "system_basic",
                Some(&serde_json::json!({"action":"read_range","path":"/root/secret.txt"})),
                &err,
            ),
            Some("recoverable_failure_finalize")
        );
    }

    #[test]
    fn fs_basic_virtual_permission_failure_finalizes_without_shell_fallback() {
        let state = test_state_with_registry();
        let actions = vec![
            AgentAction::CallSkill {
                skill: "fs_basic".to_string(),
                args: serde_json::json!({
                    "action":"read_text_range",
                    "path":"/root/secret.txt"
                }),
            },
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: serde_json::json!({"command":"head -n 1 /root/secret.txt"}),
            },
        ];
        let err = format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "system_basic",
                "error_kind": "permission_denied",
                "error_text": "permission denied: /root/secret.txt"
            })
        );

        assert_eq!(
            classify_skill_failure_recovery(
                &state,
                &actions,
                0,
                4,
                "fs_basic",
                Some(&serde_json::json!({
                    "action":"read_text_range",
                    "path":"/root/secret.txt"
                })),
                &err,
            ),
            Some("recoverable_failure_finalize")
        );
    }

    #[test]
    fn explicit_missing_target_without_fallback_finalizes_not_found() {
        let state = test_state_with_registry();
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({"action":"read_range","path":"missing.md"}),
        }];
        let err = format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "system_basic",
                "error_kind": "not_found",
                "error_text": "path not found: missing.md"
            })
        );

        assert_eq!(
            classify_skill_failure_recovery(
                &state,
                &actions,
                0,
                4,
                "system_basic",
                Some(&serde_json::json!({"action":"read_range","path":"missing.md"})),
                &err,
            ),
            Some("recoverable_failure_finalize")
        );
    }

    #[test]
    fn repairable_missing_target_continues_next_round() {
        let state = test_state_with_registry();
        let actions = vec![AgentAction::CallSkill {
            skill: "read_file".to_string(),
            args: serde_json::json!({
                "path":"missing.md",
                "_clawd_missing_target_repairable": true
            }),
        }];
        let err = "__RC_READ_FILE_NOT_FOUND__:/tmp/missing.md";

        assert_eq!(
            classify_skill_failure_recovery(
                &state,
                &actions,
                0,
                4,
                "read_file",
                Some(&serde_json::json!({
                    "path":"missing.md",
                    "_clawd_missing_target_repairable": true
                })),
                err,
            ),
            Some("recoverable_failure_continue_round")
        );
    }

    #[test]
    fn planner_protocol_failure_replans_next_round() {
        let state = test_state_with_registry();
        let actions = vec![AgentAction::CallSkill {
            skill: "system_basic".to_string(),
            args: serde_json::json!({"action":"check_exists","path":"README.md"}),
        }];
        let err = format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "system_basic",
                "error_kind": "unsupported_action",
                "error_text": "unknown action: check_exists"
            })
        );

        assert_eq!(
            classify_skill_failure_recovery(
                &state,
                &actions,
                0,
                4,
                "system_basic",
                Some(&serde_json::json!({"action":"check_exists","path":"README.md"})),
                &err,
            ),
            Some("recoverable_failure_continue_round")
        );
    }

    #[test]
    fn planner_generated_terminal_command_failure_replans_but_literal_command_finalizes() {
        let state = test_state_with_registry();
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({"command":"missing_tool --version"}),
        }];
        let err = format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "run_cmd",
                "error_kind": "nonzero_exit",
                "error_text": "Command failed with exit code 127",
                "extra": {
                    "exit_code": 127,
                    "exit_category": "command_not_found",
                    "stderr": "missing_tool: command not found"
                }
            })
        );

        assert_eq!(
            classify_skill_failure_recovery(
                &state,
                &actions,
                0,
                4,
                "run_cmd",
                Some(&serde_json::json!({"command":"missing_tool --version"})),
                &err,
            ),
            Some("recoverable_failure_continue_round")
        );

        assert_eq!(
            classify_skill_failure_recovery(
                &state,
                &actions,
                0,
                4,
                "run_cmd",
                Some(&serde_json::json!({
                    "command":"missing_tool --version",
                    "_clawd_literal_command": true
                })),
                &err,
            ),
            Some("recoverable_failure_finalize")
        );
    }

    #[test]
    fn literal_command_failure_with_structured_repairable_marker_replans() {
        let state = test_state_with_registry();
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({
                "command":"missing_tool --version",
                "_clawd_literal_command": true,
                "_clawd_literal_failure_repairable": true
            }),
        }];
        let err = format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "run_cmd",
                "error_kind": "nonzero_exit",
                "error_text": "Command failed with exit code 127",
                "extra": {
                    "exit_code": 127,
                    "exit_category": "command_not_found",
                    "stderr": "missing_tool: command not found"
                }
            })
        );

        assert_eq!(
            classify_skill_failure_recovery(
                &state,
                &actions,
                0,
                4,
                "run_cmd",
                Some(&serde_json::json!({
                    "command":"missing_tool --version",
                    "_clawd_literal_command": true,
                    "_clawd_literal_failure_repairable": true
                })),
                &err,
            ),
            Some("recoverable_failure_continue_round")
        );
    }

    #[test]
    fn visible_run_cmd_error_without_structured_payload_replans() {
        let state = test_state_with_registry();
        let actions = vec![AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: serde_json::json!({"command":"missing_tool --version"}),
        }];
        let err = "command failed: command not found (exit code 127); stderr: missing_tool: command not found";

        assert_eq!(
            classify_skill_failure_recovery(
                &state,
                &actions,
                0,
                4,
                "run_cmd",
                Some(&serde_json::json!({"command":"missing_tool --version"})),
                err,
            ),
            Some("recoverable_failure_continue_round")
        );
    }

    #[test]
    fn planner_generated_command_failure_replans_before_discussion_only_tail() {
        let state = test_state_with_registry();
        let actions = vec![
            AgentAction::CallSkill {
                skill: "run_cmd".to_string(),
                args: serde_json::json!({"command":"missing_tool --version"}),
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];
        let err = format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "run_cmd",
                "error_kind": "nonzero_exit",
                "error_text": "Command failed with exit code 127",
                "extra": {
                    "exit_code": 127,
                    "exit_category": "command_not_found",
                    "stderr": "missing_tool: command not found"
                }
            })
        );

        assert_eq!(
            classify_skill_failure_recovery(
                &state,
                &actions,
                0,
                4,
                "run_cmd",
                Some(&serde_json::json!({"command":"missing_tool --version"})),
                &err,
            ),
            Some("recoverable_failure_continue_round")
        );
    }

    #[test]
    fn recoverable_nonterminal_failure_with_only_discussion_remaining_continues_next_round() {
        let state = test_state_with_registry();
        let actions = vec![
            AgentAction::CallSkill {
                skill: "list_dir".to_string(),
                args: serde_json::json!({"path":"missing_dir"}),
            },
            AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
            AgentAction::Respond {
                content: "{{last_output}}".to_string(),
            },
        ];
        let err = format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "list_dir",
                "error_kind": "ambiguous_target",
                "error_text": "directory locator matched multiple candidates",
                "extra": { "candidates": ["/tmp/a", "/tmp/b"] }
            })
        );

        assert_eq!(
            classify_skill_failure_recovery(
                &state,
                &actions,
                0,
                4,
                "list_dir",
                Some(&serde_json::json!({"path":"missing_dir"})),
                &err,
            ),
            Some("recoverable_failure_continue_round")
        );
    }

    fn ok_step(step_id: &str, skill: &str, output: &str) -> StepExecutionResult {
        StepExecutionResult {
            step_id: step_id.to_string(),
            skill: skill.to_string(),
            status: StepExecutionStatus::Ok,
            output: Some(output.to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        }
    }

    #[test]
    fn synthesize_failure_observed_facts_include_recent_execution_outputs() {
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "list_dir", "alpha.md\nbeta.md\n"));

        let facts = synthesize_failure_observed_facts(&loop_state, "last_output");
        let joined = facts.join("\n");

        assert!(joined.contains("synthesize_refs: last_output"));
        assert!(joined.contains("observed_steps_count: 1"));
        assert!(joined.contains("skill=list_dir"));
        assert!(joined.contains("alpha.md"));
    }

    #[test]
    fn synthesize_failure_after_observation_allows_one_replan() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "read_file",
            "large file excerpt...\n",
        ));

        assert!(synthesize_failure_should_replan(&loop_state));

        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_2".to_string(),
            skill: "synthesize_answer".to_string(),
            status: StepExecutionStatus::Error,
            output: None,
            error: Some("no publishable answer".to_string()),
            started_at: 0,
            finished_at: 0,
        });

        assert!(!synthesize_failure_should_replan(&loop_state));
    }

    #[test]
    fn deterministic_status_answer_uses_observed_step_status_before_llm() {
        let state = test_state_with_registry();
        let task = crate::ClaimedTask {
            task_id: "task-1".to_string(),
            user_id: 1,
            chat_id: 1,
            user_key: None,
            channel: "test".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: String::new(),
        };
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "list_dir", "alpha.log\n"));
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_2".to_string(),
            skill: "run_cmd".to_string(),
            status: StepExecutionStatus::Error,
            output: None,
            error: Some("Command failed with exit code 127".to_string()),
            started_at: 0,
            finished_at: 0,
        });

        let answer = deterministic_observed_execution_status_answer(
            &state,
            &task,
            "先列目录，再执行缺失命令，总结成功和失败。",
            &loop_state,
        )
        .expect("deterministic status answer");

        assert!(answer.contains("list_dir"));
        assert!(answer.contains("run_cmd"));
        assert!(answer.contains("exit code 127"));
    }

    #[test]
    fn deterministic_status_answer_defers_after_recovery_success() {
        let state = test_state_with_registry();
        let task = crate::ClaimedTask {
            task_id: "task-recovered".to_string(),
            user_id: 1,
            chat_id: 1,
            user_key: None,
            channel: "test".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: String::new(),
        };
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Error,
            output: None,
            error: Some("unknown action: grep_text".to_string()),
            started_at: 0,
            finished_at: 0,
        });
        loop_state.executed_step_results.push(ok_step(
            "step_2",
            "fs_search",
            r#"{"count":1,"match_count":2}"#,
        ));

        assert!(deterministic_observed_execution_status_answer(
            &state,
            &task,
            "检查文件里是否包含目标分支",
            &loop_state,
        )
        .is_none());
    }

    #[test]
    fn deterministic_status_answer_uses_structured_run_cmd_stderr() {
        let state = test_state_with_registry();
        let task = crate::ClaimedTask {
            task_id: "task-structured-run-cmd".to_string(),
            user_id: 1,
            chat_id: 1,
            user_key: None,
            channel: "test".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: String::new(),
        };
        let err = format!(
            "__RC_SKILL_ERROR__:{}",
            serde_json::json!({
                "skill": "run_cmd",
                "error_kind": "nonzero_exit",
                "error_text": "Command failed with exit code 7",
                "platform": "linux",
                "extra": {
                    "exit_code": 7,
                    "stderr": "problem",
                    "output_truncated": false
                }
            })
        );
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "run_cmd", "READY\n"));
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_2".to_string(),
            skill: "run_cmd".to_string(),
            status: StepExecutionStatus::Error,
            output: None,
            error: Some(err),
            started_at: 0,
            finished_at: 0,
        });

        let answer = deterministic_observed_execution_status_answer(
            &state,
            &task,
            "执行两个命令，总结成功和失败，并说明错误输出。",
            &loop_state,
        )
        .expect("deterministic status answer");

        assert!(answer.contains("exit code 7"), "answer: {answer}");
        assert!(answer.contains("stderr: problem"), "answer: {answer}");
    }

    #[test]
    fn synthesize_answer_direct_fallback_only_for_single_last_output() {
        assert!(synthesize_answer_allows_direct_fallback(&[]));
        assert!(synthesize_answer_allows_direct_fallback(&[
            "last_output".to_string()
        ]));
        assert!(!synthesize_answer_allows_direct_fallback(&[
            "s1".to_string(),
            "s2".to_string()
        ]));
        assert!(!synthesize_answer_allows_direct_fallback(&[
            "last_output".to_string(),
            "step_1".to_string()
        ]));
    }

    #[test]
    fn synthesize_direct_fallback_uses_scalar_path_observation() {
        let state = test_state_with_registry();
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"path_batch_facts","facts":[{"path":".","resolved_path":"/home/guagua/rustclaw","exists":true}]}"#,
        ));
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "current workspace path".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "scalar path".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::Scalar,
                requires_content_evidence: false,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let ctx = AgentRunContext {
            route_result: Some(route),
            ..AgentRunContext::default()
        };

        let answer = synthesize_direct_observed_fallback_answer(&state, &loop_state, Some(&ctx))
            .expect("scalar path fallback");

        assert_eq!(answer, "/home/guagua/rustclaw");
    }

    #[test]
    fn contract_matrix_synthesis_prefers_observed_answer_over_step_status() {
        let state = test_state_with_registry();
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "archive_basic",
            "dest_path=/tmp/rustclaw-workspace/tmp/contract_matrix_unpacked\nexit=0\nArchive: /tmp/test_bundle.zip\n inflating: /tmp/rustclaw-workspace/tmp/contract_matrix_unpacked/notes.txt\n",
        ));
        loop_state.executed_step_results.push(StepExecutionResult {
            step_id: "step_2".to_string(),
            skill: "system_basic".to_string(),
            status: StepExecutionStatus::Error,
            output: None,
            error: Some("__RC_SKILL_ERROR__:{\"error_kind\":\"contract_action_rejected\",\"error_text\":\"action `system_basic.inventory_dir` is rejected by contract `archive_unpack`\"}".to_string()),
            started_at: 0,
            finished_at: 0,
        });
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent:
                "把 test_bundle.zip 解压到 tmp/contract_matrix_unpacked，并简短说明结果".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "structured_contract_hint_fast_path; contract_hint_fast_path".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ArchiveUnpack,
                locator_hint: "/tmp/test_bundle.zip | tmp/contract_matrix_unpacked".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let ctx = AgentRunContext {
            route_result: Some(route),
            ..AgentRunContext::default()
        };

        let answer = synthesize_contract_matrix_direct_observed_fallback_answer(
            &state,
            &loop_state,
            Some(&ctx),
        )
        .expect("contract matrix observed fallback");

        assert!(answer.contains("/tmp/rustclaw-workspace/tmp/contract_matrix_unpacked"));
        assert!(answer.contains("notes.txt"), "answer: {answer}");
        assert!(!answer.contains("第 1 步"), "answer: {answer}");
        assert!(!answer.contains("system_basic"), "answer: {answer}");
    }

    #[test]
    fn synthesize_direct_fallback_blocks_multiline_read_range_for_scalar_extraction() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r##"{"action":"read_range","excerpt":"1|# Service Notes\n2|\n3|Operators should check the app log first when requests fail, then verify the config file and database tables.","path":"/tmp/service_notes.md"}"##,
        ));
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "extract one scalar from a markdown file".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "scalar locator requires evidence".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: "/tmp/service_notes.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let ctx = AgentRunContext {
            route_result: Some(route),
            ..AgentRunContext::default()
        };

        assert!(synthesize_route_allows_direct_fallback(Some(&ctx)));
        assert!(
            synthesize_direct_fallback_would_passthrough_multiline_read_range(
                &loop_state,
                Some(&ctx)
            )
        );
    }

    #[test]
    fn deterministic_scalar_markdown_heading_uses_structural_read_evidence() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r##"{"action":"read_range","excerpt":"1|# Release Checklist","path":"/tmp/release_checklist.md"}"##,
        ));
        let mut route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "extract one scalar from a markdown file".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "scalar markdown heading".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: "/tmp/release_checklist.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let ctx = AgentRunContext {
            route_result: Some(route.clone()),
            ..AgentRunContext::default()
        };

        assert_eq!(
            deterministic_scalar_markdown_heading_answer(&loop_state, Some(&ctx)).as_deref(),
            Some("Release Checklist")
        );

        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        let ctx = AgentRunContext {
            route_result: Some(route),
            ..AgentRunContext::default()
        };
        assert!(deterministic_scalar_markdown_heading_answer(&loop_state, Some(&ctx)).is_none());
    }

    #[test]
    fn deterministic_scalar_markdown_heading_defers_when_read_evidence_has_body() {
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "fs_basic",
            r##"{"action":"read_range","excerpt":"1|# Release Checklist\n2|\n3|1. Verify configuration loads correctly.","path":"/tmp/release_checklist.md"}"##,
        ));
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "extract one scalar from a markdown file".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "scalar markdown body".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: "/tmp/release_checklist.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let ctx = AgentRunContext {
            route_result: Some(route),
            ..AgentRunContext::default()
        };

        assert!(deterministic_scalar_markdown_heading_answer(&loop_state, Some(&ctx)).is_none());
    }

    #[test]
    fn synthesize_route_allows_direct_fallback_for_plain_act_observed_read() {
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "读 README.md 前 2 行".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "plain observed read".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Filename,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: "README.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let ctx = AgentRunContext {
            route_result: Some(route),
            ..AgentRunContext::default()
        };

        assert!(synthesize_route_allows_direct_fallback(Some(&ctx)));
    }

    #[test]
    fn synthesize_route_allows_direct_fallback_for_structured_listing_contract() {
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_plain(),
            resolved_intent: "List files from a known directory.".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "structured file listing".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::FileNames,
                locator_hint: "document".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let ctx = AgentRunContext {
            route_result: Some(route),
            ..AgentRunContext::default()
        };

        assert!(synthesize_route_allows_direct_fallback(Some(&ctx)));
    }

    #[test]
    fn synthesize_route_allows_direct_fallback_for_config_validation_contract() {
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "Validate config syntax from structured parser evidence.".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "structured config validation".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ConfigValidation,
                locator_hint: "configs/config.toml".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let ctx = AgentRunContext {
            route_result: Some(route),
            original_user_request: Some(
                "Validate only the TOML syntax of configs/config.toml and answer pass or fail."
                    .to_string(),
            ),
            ..AgentRunContext::default()
        };
        let mut loop_state = LoopState::new(2);
        loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"validate_structured","format":"toml","path":"configs/config.toml","resolved_path":"/tmp/config.toml","valid":true}"#,
        ));
        let state = test_state_with_registry();

        assert!(synthesize_route_allows_direct_fallback(Some(&ctx)));
        assert_eq!(
            synthesize_direct_observed_fallback_answer(&state, &loop_state, Some(&ctx)).as_deref(),
            Some("pass: toml parsed successfully")
        );
    }

    #[test]
    fn synthesize_route_uses_llm_for_chat_wrapped_unclassified_delivery() {
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "Run a command, then produce a short final reply based on the result."
                .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "execution plus synthesis".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let ctx = AgentRunContext {
            route_result: Some(route),
            ..AgentRunContext::default()
        };

        assert!(!synthesize_route_allows_direct_fallback(Some(&ctx)));
    }

    #[test]
    fn synthesize_route_allows_direct_fallback_for_strict_plain_observation() {
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "Return an already formatted observed result.".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "strict formatted output".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::Strict,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: "logs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let ctx = AgentRunContext {
            route_result: Some(route),
            ..AgentRunContext::default()
        };

        assert!(synthesize_route_allows_direct_fallback(Some(&ctx)));
    }

    #[test]
    fn synthesize_route_uses_llm_for_strict_raw_output_contract() {
        let route = crate::RouteResult {
            ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
            resolved_intent: "Run a command and return its raw output.".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "raw command output".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                exact_sentence_count: None,
                response_shape: crate::OutputResponseShape::Strict,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::RawCommandOutput,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let ctx = AgentRunContext {
            route_result: Some(route),
            ..AgentRunContext::default()
        };

        assert!(!synthesize_route_allows_direct_fallback(Some(&ctx)));
    }
}

fn is_read_only_skill_invocation(state: &AppState, normalized_skill: &str, args: &Value) -> bool {
    if state.skill_is_read_only(normalized_skill) {
        return true;
    }
    match normalized_skill {
        "read_file" | "list_dir" | "fs_search" | "system_basic" | "log_analyze" | "doc_parse"
        | "git_basic" | "http_basic" | "stock" | "weather" | "web_search_extract"
        | "health_check" | "task_control" => true,
        "db_basic" => args
            .get("action")
            .and_then(|v| v.as_str())
            .map(|a| {
                a.eq_ignore_ascii_case("sqlite_query")
                    || a.eq_ignore_ascii_case("schema_version")
                    || a.eq_ignore_ascii_case("sqlite_schema_version")
            })
            .unwrap_or(true),
        _ => false,
    }
}

fn should_publish_respond_message(loop_state: &LoopState, text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if !loop_state.has_tool_or_skill_output {
        return true;
    }
    if loop_state
        .delivery_messages
        .last()
        .is_some_and(|last| last.trim() == trimmed)
    {
        return false;
    }
    if loop_state
        .last_output
        .as_deref()
        .map(str::trim)
        .is_some_and(|last| last == trimmed)
    {
        return false;
    }
    true
}

fn route_requires_file_token_delivery(agent_run_context: Option<&AgentRunContext>) -> bool {
    agent_run_context
        .and_then(|ctx| ctx.route_result.as_ref())
        .map(|route| {
            route.output_contract.delivery_required
                || matches!(
                    route.output_contract.response_shape,
                    OutputResponseShape::FileToken
                )
        })
        .unwrap_or(false)
}

fn file_token_payload_contains_runtime_artifact(payload: &str) -> bool {
    let payload = payload.trim();
    if payload.is_empty() {
        return true;
    }
    if Path::new(payload).is_file() {
        return false;
    }
    payload.contains("{{")
        || payload.contains("}}")
        || payload.contains('\n')
        || payload.starts_with('{')
        || payload.starts_with('[')
        || payload.contains("\"action\"")
        || payload.contains("\"counts\"")
        || payload.contains("\"names\"")
        || payload.contains("\"results\"")
}

fn unresolved_file_token_delivery_artifact(text: &str) -> bool {
    let Some((_kind, payload)) = crate::finalize::parse_delivery_file_token(text.trim()) else {
        return false;
    };
    file_token_payload_contains_runtime_artifact(payload)
}

pub(super) fn handle_respond_action(
    state: &AppState,
    task: &ClaimedTask,
    actions: &[AgentAction],
    loop_state: &mut LoopState,
    policy: &AgentLoopGuardPolicy,
    idx: usize,
    global_step: usize,
    step_in_round: usize,
    fingerprint: &str,
    content: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> RespondActionOutcome {
    let text = rewrite_response_with_written_aliases(
        &resolve_arg_string(content, loop_state).trim().to_string(),
        loop_state,
    )
    .trim()
    .to_string();

    if route_requires_file_token_delivery(agent_run_context)
        && unresolved_file_token_delivery_artifact(&text)
    {
        let error = "invalid file delivery token: runtime observation was embedded into FILE path";
        loop_state.has_recoverable_failure_context = true;
        super::attempt_ledger::record_attempt_with_retry_instruction(
            loop_state,
            "respond",
            &format!("content={}", crate::truncate_for_agent_trace(&text)),
            crate::executor::StepExecutionStatus::Error,
            &text,
            Some("invalid_delivery_token"),
            error,
            Some(
                "Use the already observed structured output to select a concrete existing file path, or run one bounded command/tool that directly returns that selected path. Then respond with exactly FILE:<path>; do not put {{last_output}} or a structured object inside FILE:.",
            ),
        );
        crate::append_subtask_result(
            &mut loop_state.subtask_results,
            global_step,
            "respond",
            false,
            error,
        );
        append_progress_hint(
            state,
            task,
            &mut loop_state.progress_messages,
            encode_progress_i18n("telegram.progress.retry_replan", &[]),
        );
        loop_state
            .executed_step_results
            .push(crate::executor::StepExecutionResult {
                step_id: format!("step_{}", global_step),
                skill: "respond".to_string(),
                status: crate::executor::StepExecutionStatus::Error,
                output: None,
                error: Some(error.to_string()),
                started_at: 0,
                finished_at: 0,
            });
        loop_state.history_compact.push(format!(
            "round={} step={} respond invalid_delivery_token",
            loop_state.round_no, step_in_round
        ));
        info!(
            "respond_invalid_delivery_token_replan task_id={} round={} step={} text={}",
            task.task_id,
            loop_state.round_no,
            step_in_round,
            crate::truncate_for_log(&text)
        );
        return RespondActionOutcome {
            ended_with_user_visible_output: false,
            stop_signal: Some("recoverable_failure_continue_round".to_string()),
            should_stop: true,
        };
    }

    let has_remaining_actions = has_remaining_action_after(actions, idx, policy.max_steps);
    let publish_respond = should_publish_respond_message(loop_state, &text);
    if !text.is_empty() && (publish_respond || !has_remaining_actions) {
        loop_state.last_user_visible_respond = Some(text.clone());
    }
    if publish_respond {
        crate::append_subtask_result(
            &mut loop_state.subtask_results,
            global_step,
            "respond",
            true,
            &text,
        );
        append_delivery_message(
            &task.task_id,
            &mut loop_state.delivery_messages,
            text.clone(),
        );
        info!(
            "delivery appended from respond task_id={} len={} has_remaining={}",
            task.task_id,
            loop_state.delivery_messages.len(),
            has_remaining_actions
        );
        let hint = encode_progress_i18n("telegram.progress.reply_generated", &[]);
        append_progress_hint(state, task, &mut loop_state.progress_messages, hint);
    }
    if !publish_respond && !text.is_empty() {
        debug!(
            "executor_step_skip task_id={} round={} step={} type=respond reason=respond_not_publishable trace_only",
            task.task_id, loop_state.round_no, step_in_round
        );
    }
    register_step_output(loop_state, global_step, step_in_round, "respond", &text);
    *loop_state
        .successful_action_fingerprints
        .entry(fingerprint.to_string())
        .or_insert(0) += 1;
    info!(
        "executor_result_ok task_id={} round={} step={} type=respond output={}",
        task.task_id,
        loop_state.round_no,
        step_in_round,
        crate::truncate_for_log(&text)
    );
    loop_state.history_compact.push(format!(
        "round={} step={} respond{}",
        loop_state.round_no,
        step_in_round,
        if has_remaining_actions {
            "_intermediate"
        } else {
            ""
        }
    ));
    RespondActionOutcome {
        ended_with_user_visible_output: publish_respond
            && !has_remaining_actions
            && !text.is_empty(),
        stop_signal: if has_remaining_actions {
            None
        } else {
            Some("respond".to_string())
        },
        should_stop: !has_remaining_actions,
    }
}

fn read_file_requested_path(skill_name: &str, args: &Value) -> Option<String> {
    if skill_name != "read_file" {
        return None;
    }
    args.get("path")
        .and_then(|v| v.as_str())
        .map(|path| path.to_string())
}

fn write_file_effective_path(
    state: &AppState,
    normalized_skill: &str,
    args: &Value,
) -> Option<WriteFileEffectivePath> {
    if normalized_skill != "write_file" {
        return None;
    }
    args.get("path").and_then(|v| v.as_str()).map(|path| {
        let effective = crate::ensure_default_file_path(&state.skill_rt.workspace_root, path);
        let user_visible = if Path::new(&effective).is_absolute() {
            effective.clone()
        } else {
            state
                .skill_rt
                .workspace_root
                .join(&effective)
                .display()
                .to_string()
        };
        (path.to_string(), effective, user_visible)
    })
}

fn apply_recipe_run_cmd_overrides(
    state: &AppState,
    loop_state: &LoopState,
    policy: &AgentLoopGuardPolicy,
    normalized_skill: &str,
    args: &mut Value,
) {
    if normalized_skill != "run_cmd" || !loop_state.execution_recipe.is_active() {
        return;
    }
    let Some(obj) = args.as_object_mut() else {
        return;
    };
    if obj.get("timeout_seconds").is_some() {
        return;
    }
    let raw_effect = crate::execution_recipe::classify_skill_action_effect(
        state,
        normalized_skill,
        &Value::Object(obj.clone()),
    );
    let effect = crate::execution_recipe::effective_action_effect_for_recipe(
        loop_state.execution_recipe,
        raw_effect,
    );
    let Some(timeout_seconds) =
        policy.run_cmd_timeout_override(loop_state.execution_recipe, effect)
    else {
        return;
    };
    obj.insert(
        "timeout_seconds".to_string(),
        Value::Number(serde_json::Number::from(timeout_seconds)),
    );
}

pub(super) async fn handle_call_tool_action(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    actions: &[AgentAction],
    round_steps: &[String],
    loop_state: &mut LoopState,
    policy: &AgentLoopGuardPolicy,
    idx: usize,
    action: &AgentAction,
    fingerprint: &str,
    global_step: usize,
    step_in_round: usize,
    executed_actions: &mut usize,
    ended_with_user_visible_output: &mut bool,
    tool: &str,
    args: &Value,
) -> Result<ActionLoopDecision, String> {
    let mut resolved_args = resolve_arg_value(args, loop_state);
    let mut normalized_skill = state.resolve_canonical_skill_name(tool);
    if normalize_skill_arg_aliases(&normalized_skill, &mut resolved_args) {
        info!(
            "executor_args_rewrite task_id={} round={} step={} type=arg_alias skill={} args={}",
            task.task_id,
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            crate::truncate_for_log(&resolved_args.to_string())
        );
    }
    if let Some(rewrite) =
        crate::virtual_tools::rewrite_virtual_tool_call(&normalized_skill, resolved_args.clone())?
    {
        info!(
            "executor_virtual_tool_rewrite task_id={} round={} step={} requested_tool={} runtime_tool={} args={}",
            task.task_id,
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            rewrite.runtime_tool,
            crate::truncate_for_log(&rewrite.runtime_args.to_string())
        );
        normalized_skill = state.resolve_canonical_skill_name(&rewrite.runtime_tool);
        resolved_args = rewrite.runtime_args;
        if normalize_skill_arg_aliases(&normalized_skill, &mut resolved_args) {
            info!(
                "executor_args_rewrite task_id={} round={} step={} type=runtime_arg_alias skill={} args={}",
                task.task_id,
                loop_state.round_no,
                step_in_round,
                normalized_skill,
                crate::truncate_for_log(&resolved_args.to_string())
            );
        }
    }
    if rewrite_args_with_auto_locator_path(&normalized_skill, &mut resolved_args, loop_state) {
        info!(
            "executor_args_rewrite task_id={} round={} step={} type=auto_locator skill={} args={}",
            task.task_id,
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            crate::truncate_for_log(&resolved_args.to_string())
        );
    }
    let read_file_requested_path = read_file_requested_path(&normalized_skill, &resolved_args);
    let write_file_effective_path =
        write_file_effective_path(state, &normalized_skill, &resolved_args);
    if normalized_skill == "run_cmd" {
        if let Some(obj) = resolved_args.as_object_mut() {
            if let Some(command) = obj.get("command").and_then(|v| v.as_str()) {
                let rewritten = rewrite_run_cmd_with_written_aliases(command, loop_state);
                if rewritten != command {
                    obj.insert("command".to_string(), Value::String(rewritten));
                }
            }
        }
    }
    rewrite_tool_path_with_written_aliases(&normalized_skill, &mut resolved_args, loop_state);
    apply_recipe_run_cmd_overrides(
        state,
        loop_state,
        policy,
        &normalized_skill,
        &mut resolved_args,
    );
    let recovery_args = resolved_args.clone();
    strip_internal_execution_args(&mut resolved_args);
    loop_state.tool_calls_total += 1;
    let args_summary = build_safe_skill_args_summary(&resolved_args, PROGRESS_ARGS_SUMMARY_MAX_LEN);
    let skill_outcome = execute_prepared_skill_action(
        state,
        task,
        goal,
        user_text,
        actions,
        round_steps,
        loop_state,
        policy,
        idx,
        action,
        fingerprint,
        global_step,
        step_in_round,
        &normalized_skill,
        resolved_args,
        Some(recovery_args),
        write_file_effective_path,
        read_file_requested_path,
        args_summary,
        "call_skill(legacy_tool)",
    )
    .await?;
    Ok(apply_skill_action_outcome(
        loop_state,
        executed_actions,
        ended_with_user_visible_output,
        skill_outcome,
    ))
}

pub(super) async fn handle_call_skill_action(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    actions: &[AgentAction],
    round_steps: &[String],
    loop_state: &mut LoopState,
    policy: &AgentLoopGuardPolicy,
    idx: usize,
    action: &AgentAction,
    fingerprint: &str,
    global_step: usize,
    step_in_round: usize,
    executed_actions: &mut usize,
    ended_with_user_visible_output: &mut bool,
    skill: &str,
    args: &Value,
) -> Result<ActionLoopDecision, String> {
    let mut resolved_args = resolve_arg_value(args, loop_state);
    loop_state.tool_calls_total += 1;
    let mut normalized_skill = state.resolve_canonical_skill_name(skill);
    if normalize_skill_arg_aliases(&normalized_skill, &mut resolved_args) {
        info!(
            "executor_args_rewrite task_id={} round={} step={} type=arg_alias skill={} args={}",
            task.task_id,
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            crate::truncate_for_log(&resolved_args.to_string())
        );
    }
    if let Some(rewrite) =
        crate::virtual_tools::rewrite_virtual_tool_call(&normalized_skill, resolved_args.clone())?
    {
        info!(
            "executor_virtual_tool_rewrite task_id={} round={} step={} requested_tool={} runtime_tool={} args={}",
            task.task_id,
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            rewrite.runtime_tool,
            crate::truncate_for_log(&rewrite.runtime_args.to_string())
        );
        normalized_skill = state.resolve_canonical_skill_name(&rewrite.runtime_tool);
        resolved_args = rewrite.runtime_args;
        if normalize_skill_arg_aliases(&normalized_skill, &mut resolved_args) {
            info!(
                "executor_args_rewrite task_id={} round={} step={} type=runtime_arg_alias skill={} args={}",
                task.task_id,
                loop_state.round_no,
                step_in_round,
                normalized_skill,
                crate::truncate_for_log(&resolved_args.to_string())
            );
        }
    }
    if rewrite_args_with_auto_locator_path(&normalized_skill, &mut resolved_args, loop_state) {
        info!(
            "executor_args_rewrite task_id={} round={} step={} type=auto_locator skill={} args={}",
            task.task_id,
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            crate::truncate_for_log(&resolved_args.to_string())
        );
    }
    apply_recipe_run_cmd_overrides(
        state,
        loop_state,
        policy,
        &normalized_skill,
        &mut resolved_args,
    );
    let recovery_args = resolved_args.clone();
    strip_internal_execution_args(&mut resolved_args);
    let read_file_requested_path = read_file_requested_path(&normalized_skill, &resolved_args);
    let write_file_effective_path =
        write_file_effective_path(state, &normalized_skill, &resolved_args);
    let args_summary = build_safe_skill_args_summary(&resolved_args, PROGRESS_ARGS_SUMMARY_MAX_LEN);
    let skill_outcome = execute_prepared_skill_action(
        state,
        task,
        goal,
        user_text,
        actions,
        round_steps,
        loop_state,
        policy,
        idx,
        action,
        fingerprint,
        global_step,
        step_in_round,
        &normalized_skill,
        resolved_args,
        Some(recovery_args),
        write_file_effective_path,
        read_file_requested_path,
        args_summary,
        "call_skill",
    )
    .await?;
    Ok(apply_skill_action_outcome(
        loop_state,
        executed_actions,
        ended_with_user_visible_output,
        skill_outcome,
    ))
}

pub(super) async fn handle_synthesize_answer_action(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    action: &AgentAction,
    global_step: usize,
    step_in_round: usize,
    executed_actions: &mut usize,
    ended_with_user_visible_output: &mut bool,
    agent_run_context: Option<&AgentRunContext>,
    evidence_refs: &[String],
) -> Result<ActionLoopDecision, String> {
    loop_state.tool_calls_total += 1;
    let refs_summary = if evidence_refs.is_empty() {
        "last_output".to_string()
    } else {
        evidence_refs.join(",")
    };
    info!(
        "{} executor_step_execute task_id={} round={} step={} type=synthesize_answer refs={}",
        crate::highlight_tag("llm"),
        task.task_id,
        loop_state.round_no,
        step_in_round,
        crate::truncate_for_log(&refs_summary)
    );
    let step_execution =
        crate::executor::execute_step(&format!("step_{global_step}"), action, || async {
            if let Some(answer) = synthesize_contract_matrix_direct_observed_fallback_answer(
                state,
                loop_state,
                agent_run_context,
            ) {
                return Ok(answer);
            }
            if let Some(answer) =
                deterministic_observed_execution_status_answer(state, task, user_text, loop_state)
            {
                return Ok(answer);
            }
            if let Some((answer, _summary)) = crate::finalize::direct_config_edit_observed_answer(
                state,
                synthesize_user_language_source(user_text, agent_run_context),
                loop_state,
            ) {
                return Ok(answer);
            }
            if let Some(answer) =
                deterministic_scalar_markdown_heading_answer(loop_state, agent_run_context)
            {
                return Ok(answer);
            }
            let requires_synthesized_delivery = agent_run_context
                .and_then(|context| context.route_result.as_ref())
                .is_some_and(
                    crate::agent_engine::observed_output::route_requires_synthesized_delivery,
                );
            let direct_fallback_blocked =
                synthesize_direct_fallback_would_passthrough_multiline_read_range(
                    loop_state,
                    agent_run_context,
                );
            let allow_direct_fallback = synthesize_answer_allows_direct_fallback(evidence_refs)
                && synthesize_route_allows_direct_fallback(agent_run_context)
                && !direct_fallback_blocked;
            if allow_direct_fallback {
                if let Some(answer) =
                    synthesize_direct_observed_fallback_answer(state, loop_state, agent_run_context)
                {
                    return Ok(answer);
                }
            }
            let synthesized =
                crate::agent_engine::observed_output::synthesize_answer_from_observed_output(
                    state,
                    task,
                    user_text,
                    loop_state,
                    agent_run_context,
                )
                .await
                .map(|(answer, _summary)| answer)
                .filter(|answer| !answer.trim().is_empty());
            if let Some(answer) = synthesized {
                return Ok(answer);
            }
            if !allow_direct_fallback && !requires_synthesized_delivery && !direct_fallback_blocked
            {
                if let Some(answer) =
                    synthesize_direct_observed_fallback_answer(state, loop_state, agent_run_context)
                {
                    return Ok(answer);
                }
            }
            Err(synthesize_failure_user_message(
                state,
                task,
                user_text,
                loop_state,
                agent_run_context,
                &refs_summary,
            )
            .await)
        })
        .await;
    match step_execution
        .output
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        Some(answer) => {
            let answer = answer.to_string();
            crate::append_subtask_result(
                &mut loop_state.subtask_results,
                global_step,
                "synthesize_answer",
                true,
                &answer,
            );
            register_step_output(
                loop_state,
                global_step,
                step_in_round,
                "synthesize_answer",
                &answer,
            );
            loop_state.last_publishable_synthesis_output = Some(answer.clone());
            loop_state.history_compact.push(format!(
                "round={} step={} synthesize_answer ok refs={}",
                loop_state.round_no,
                step_in_round,
                crate::truncate_for_agent_trace(&refs_summary)
            ));
            info!(
                "executor_result_ok task_id={} round={} step={} type=synthesize_answer output={} trace_only=raw_not_delivery",
                task.task_id,
                loop_state.round_no,
                step_in_round,
                crate::truncate_for_log(&answer)
            );
            loop_state.executed_step_results.push(step_execution);
            let outcome = SkillActionOutcome {
                ended_with_user_visible_output: true,
                stop_signal: None,
                continue_in_round: false,
            };
            Ok(apply_skill_action_outcome(
                loop_state,
                executed_actions,
                ended_with_user_visible_output,
                outcome,
            ))
        }
        None => {
            let err = step_execution
                .error
                .clone()
                .unwrap_or_else(|| "synthesize_answer failed".to_string());
            let should_replan = synthesize_failure_should_replan(loop_state);
            if should_replan {
                let compact_err = err
                    .replace('\n', " ")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                let compact_err = crate::truncate_for_agent_trace(&compact_err);
                append_progress_hint(
                    state,
                    task,
                    &mut loop_state.progress_messages,
                    encode_progress_i18n(
                        "telegram.progress.step_failed",
                        &[
                            ("step", &step_in_round.to_string()),
                            ("skill", "synthesize_answer"),
                            ("error", &compact_err),
                        ],
                    ),
                );
                append_progress_hint(
                    state,
                    task,
                    &mut loop_state.progress_messages,
                    encode_progress_i18n("telegram.progress.retry_replan", &[]),
                );
            }
            crate::append_subtask_result(
                &mut loop_state.subtask_results,
                global_step,
                "synthesize_answer",
                false,
                &err,
            );
            loop_state.history_compact.push(format!(
                "round={} step={} synthesize_answer failed error={}",
                loop_state.round_no,
                step_in_round,
                crate::truncate_for_agent_trace(&err)
            ));
            loop_state.executed_step_results.push(step_execution);
            *executed_actions += 1;
            loop_state.total_steps_executed += 1;
            info!(
                "synthesize_answer_failed_defer_to_finalize task_id={} round={} step={} error={}",
                task.task_id,
                loop_state.round_no,
                step_in_round,
                crate::truncate_for_log(&err)
            );
            Ok(ActionLoopDecision::StopRound(if should_replan {
                "recoverable_failure_continue_round".to_string()
            } else {
                "synthesize_answer_failed".to_string()
            }))
        }
    }
}

pub(super) async fn dispatch_round_action(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    actions: &[AgentAction],
    round_steps: &[String],
    loop_state: &mut LoopState,
    policy: &AgentLoopGuardPolicy,
    idx: usize,
    action: &AgentAction,
    fingerprint: &str,
    global_step: usize,
    step_in_round: usize,
    executed_actions: &mut usize,
    ended_with_user_visible_output: &mut bool,
    agent_run_context: Option<&AgentRunContext>,
) -> Result<ActionLoopDecision, String> {
    match action {
        AgentAction::CallTool { tool, args } => {
            handle_call_tool_action(
                state,
                task,
                goal,
                user_text,
                actions,
                round_steps,
                loop_state,
                policy,
                idx,
                action,
                fingerprint,
                global_step,
                step_in_round,
                executed_actions,
                ended_with_user_visible_output,
                tool,
                args,
            )
            .await
        }
        AgentAction::CallSkill { skill, args } => {
            handle_call_skill_action(
                state,
                task,
                goal,
                user_text,
                actions,
                round_steps,
                loop_state,
                policy,
                idx,
                action,
                fingerprint,
                global_step,
                step_in_round,
                executed_actions,
                ended_with_user_visible_output,
                skill,
                args,
            )
            .await
        }
        AgentAction::CallCapability { capability, .. } => Err(format!(
            "unsupported capability `{capability}` was not resolved before execution"
        )),
        AgentAction::SynthesizeAnswer { evidence_refs } => {
            handle_synthesize_answer_action(
                state,
                task,
                user_text,
                loop_state,
                action,
                global_step,
                step_in_round,
                executed_actions,
                ended_with_user_visible_output,
                agent_run_context,
                evidence_refs,
            )
            .await
        }
        AgentAction::Respond { content } => {
            let respond_outcome = handle_respond_action(
                state,
                task,
                actions,
                loop_state,
                policy,
                idx,
                global_step,
                step_in_round,
                fingerprint,
                content,
                agent_run_context,
            );
            Ok(apply_respond_action_outcome(
                loop_state,
                executed_actions,
                ended_with_user_visible_output,
                respond_outcome,
            ))
        }
        AgentAction::Think { .. } => {
            *executed_actions += 1;
            loop_state.total_steps_executed += 1;
            Ok(ActionLoopDecision::NextAction)
        }
    }
}
