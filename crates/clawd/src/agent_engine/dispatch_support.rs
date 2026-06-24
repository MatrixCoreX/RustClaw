use serde_json::{json, Value};
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

#[path = "dispatch_synthesis.rs"]
mod dispatch_synthesis;

use dispatch_synthesis::{
    archive_database_aggregate_structured_answer, deterministic_scalar_markdown_heading_answer,
    filesystem_mutation_lifecycle_structured_answer, package_docker_probe_structured_answer,
    route_resolved_intent, step_has_observable_synthesis_fact,
    synthesize_answer_allows_direct_fallback,
    synthesize_contract_matrix_direct_observed_fallback_answer,
    synthesize_direct_fallback_would_passthrough_multiline_read_range,
    synthesize_direct_observed_fallback_answer, synthesize_failure_observed_facts,
    synthesize_failure_should_replan, synthesize_route_allows_direct_fallback,
    synthesize_route_prefers_model_language_observed_status, synthesize_user_language_source,
};

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

fn unresolved_capability_error(state: &AppState, capability: &str, args: &Value) -> String {
    let (_resolved, record) =
        crate::capability_resolver::resolve_capability_action_with_record_for_state(
            state,
            capability,
            args.clone(),
        );
    json!({
        "error_kind": record.reason_code,
        "reason_code": record.reason_code,
        "message_key": record.reason_code,
        "owner_layer": record.owner_layer,
        "outcome": record.outcome,
        "source": record.source,
        "capability_ref": record.capability_ref,
        "resolved_ref": record.resolved_ref,
        "planner_kind": record.planner_kind,
    })
    .to_string()
}

fn is_discussion_only_action(action: &AgentAction) -> bool {
    matches!(
        action,
        AgentAction::Respond { .. }
            | AgentAction::SynthesizeAnswer { .. }
            | AgentAction::Think { .. }
    )
}

fn active_recipe_terminal_discussion_should_replan(
    actions: &[AgentAction],
    loop_state: &LoopState,
    policy: &AgentLoopGuardPolicy,
    idx: usize,
) -> bool {
    if !loop_state.execution_recipe.is_active()
        || matches!(
            loop_state.execution_recipe.phase,
            crate::execution_recipe::ExecutionRecipePhase::Done
        )
    {
        return false;
    }
    if !loop_state.executed_step_results.iter().any(|step| {
        !matches!(
            step.skill.as_str(),
            "respond" | "synthesize_answer" | "think"
        )
    }) {
        return false;
    }
    !actions
        .iter()
        .take(policy.max_steps.max(1))
        .skip(idx + 1)
        .any(|action| !is_discussion_only_action(action))
}

fn record_active_recipe_terminal_discussion_replan(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    action_kind: &str,
) {
    let reason = "active_recipe_terminal_discussion_before_done";
    loop_state.has_recoverable_failure_context = true;
    crate::append_subtask_result(
        &mut loop_state.subtask_results,
        global_step,
        action_kind,
        false,
        reason,
    );
    super::attempt_ledger::record_attempt_with_retry_instruction(
        loop_state,
        action_kind,
        reason,
        crate::executor::StepExecutionStatus::Error,
        reason,
        Some("active_recipe_incomplete_terminal_discussion"),
        reason,
        Some("active_recipe_continue_required"),
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
            step_id: format!("step_{global_step}"),
            skill: action_kind.to_string(),
            status: crate::executor::StepExecutionStatus::Error,
            output: None,
            error: Some(reason.to_string()),
            started_at: 0,
            finished_at: 0,
        });
    loop_state.history_compact.push(format!(
        "round={} step={} {} active_recipe_terminal_discussion_before_done phase={}",
        loop_state.round_no,
        step_in_round,
        action_kind,
        loop_state.execution_recipe.phase.as_str()
    ));
    info!(
        "active_recipe_terminal_discussion_replan task_id={} round={} step={} action={} phase={}",
        task.task_id,
        loop_state.round_no,
        step_in_round,
        action_kind,
        loop_state.execution_recipe.phase.as_str()
    );
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
            let output = step
                .output
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(|text| {
                    crate::truncate_for_agent_trace(
                        &crate::visible_text::sanitize_user_visible_text(text).replace('\n', " "),
                    )
                });
            if prefer_english {
                if let Some(output) = output {
                    parts.push(format!("Step {step_no} `{skill}` succeeded: {output}."));
                } else {
                    parts.push(format!("Step {step_no} `{skill}` succeeded."));
                }
            } else {
                if let Some(output) = output {
                    parts.push(format!("第 {step_no} 步 `{skill}` 成功：{output}。"));
                } else {
                    parts.push(format!("第 {step_no} 步 `{skill}` 成功。"));
                }
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
                serde_json::json!({
                    "message_key": "clawd.msg.execution.step_error_missing",
                    "reason_code": "execution_step_error_missing",
                })
                .to_string()
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
        obj.remove(super::CLAWD_USER_NAMED_OUTPUT_PATH_ARG);
        obj.remove(crate::execution_recipe::CLAWD_VALIDATION_ARG);
    }
}

fn strip_unsupported_planner_metadata_args(
    state: &AppState,
    canonical_skill: &str,
    args: &mut Value,
) -> Vec<String> {
    let Some(obj) = args.as_object_mut() else {
        return Vec::new();
    };
    let Some(manifest) = state.skill_manifest(canonical_skill) else {
        return Vec::new();
    };
    let Some(schema) = manifest.input_schema else {
        return Vec::new();
    };
    let Some(properties) = schema.get("properties").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let mut removed = Vec::new();
    for key in ["confirm", "confirmation", "requires_confirmation"] {
        if !properties.contains_key(key) && obj.remove(key).is_some() {
            removed.push(key.to_string());
        }
    }
    removed
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
    if crate::skills::is_crypto_account_access_error(normalized_skill, err) {
        return Some("recoverable_failure_finalize");
    }
    if normalized_skill.eq_ignore_ascii_case("run_cmd")
        && run_cmd_error_is_observable(normalized_skill, err)
        && run_cmd_is_literal_user_command(call_args)
        && !run_cmd_literal_failure_is_repairable(call_args)
        && remaining_actions_are_discussion_only(actions, current_idx, max_steps)
    {
        return Some("recoverable_failure_finalize");
    }
    if normalized_skill.eq_ignore_ascii_case("run_cmd")
        && run_cmd_error_is_observable(normalized_skill, err)
        && has_remaining_action_after(actions, current_idx, max_steps)
        && !remaining_actions_are_discussion_only(actions, current_idx, max_steps)
        && !run_cmd_should_continue_after_split_failure(call_args)
    {
        if run_cmd_is_literal_user_command(call_args) {
            return Some("recoverable_failure_finalize");
        }
        return Some("recoverable_failure_continue_round");
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

fn synthesize_failure_default_text(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
) -> String {
    let language_hint = crate::language_policy::task_response_language_hint(state, task, user_text);
    let prefer_english = language_hint.to_ascii_lowercase().starts_with("en");
    let default_payload =
        crate::fallback::ClarifyFallbackSource::SynthesisEmpty.machine_default_payload();
    crate::bilingual_t_with_default_vars(
        state,
        crate::fallback::ClarifyFallbackSource::SynthesisEmpty.i18n_key(),
        &default_payload,
        &default_payload,
        prefer_english,
        &[],
    )
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
        "task_success_claim_allowed=false".to_string(),
        "expose_internal_details=false".to_string(),
        "response_scope=observed_synthesis_failure_only".to_string(),
        "missing_result_invention_allowed=false".to_string(),
    ];
    if has_observed_result {
        policy_boundary.push("observed_execution_results_available=true".to_string());
        policy_boundary.push("raw_results_or_retry_synthesis_available=true".to_string());
    } else {
        policy_boundary.push("usable_execution_result_available=false".to_string());
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
#[path = "dispatch_support_tests.rs"]
mod tests;
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
    if loop_state
        .delivery_messages
        .last()
        .is_some_and(|last| last.trim() == trimmed)
    {
        return false;
    }
    if respond_machine_envelope_payload(trimmed) {
        return true;
    }
    if !loop_state.has_tool_or_skill_output {
        return true;
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

fn respond_machine_envelope_payload(text: &str) -> bool {
    let Ok(payload) = serde_json::from_str::<Value>(text.trim()) else {
        return false;
    };
    payload.is_object()
        && payload
            .get("output_format")
            .and_then(Value::as_str)
            .is_some_and(|value| value == "machine_json")
        && payload
            .get("owner_layer")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|owner| !owner.is_empty())
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

    if active_recipe_terminal_discussion_should_replan(actions, loop_state, policy, idx) {
        record_active_recipe_terminal_discussion_replan(
            state,
            task,
            loop_state,
            global_step,
            step_in_round,
            "respond",
        );
        return RespondActionOutcome {
            ended_with_user_visible_output: false,
            stop_signal: Some("recoverable_failure_continue_round".to_string()),
            should_stop: true,
        };
    }

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
    if !text.is_empty() {
        loop_state
            .executed_step_results
            .push(crate::executor::StepExecutionResult {
                step_id: format!("step_{global_step}"),
                skill: "respond".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output: Some(text.clone()),
                error: None,
                started_at: 0,
                finished_at: 0,
            });
    }
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
    let requested_virtual_skill = normalized_skill.clone();
    let requested_virtual_args = resolved_args.clone();
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
        if crate::agent_engine::enrich_scratch_filesystem_cleanup_runtime_args(
            state,
            loop_state,
            &requested_virtual_skill,
            &requested_virtual_args,
            &normalized_skill,
            &mut resolved_args,
        ) {
            info!(
                "executor_args_rewrite task_id={} round={} step={} type=scratch_cleanup_directory skill={} args={}",
                task.task_id,
                loop_state.round_no,
                step_in_round,
                normalized_skill,
                crate::truncate_for_log(&resolved_args.to_string())
            );
        }
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
    let removed_metadata =
        strip_unsupported_planner_metadata_args(state, &normalized_skill, &mut resolved_args);
    if !removed_metadata.is_empty() {
        info!(
            "executor_args_rewrite task_id={} round={} step={} type=planner_metadata_strip skill={} removed={:?} args={}",
            task.task_id,
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            removed_metadata,
            crate::truncate_for_log(&resolved_args.to_string())
        );
    }
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
    let requested_virtual_skill = normalized_skill.clone();
    let requested_virtual_args = resolved_args.clone();
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
        if crate::agent_engine::enrich_scratch_filesystem_cleanup_runtime_args(
            state,
            loop_state,
            &requested_virtual_skill,
            &requested_virtual_args,
            &normalized_skill,
            &mut resolved_args,
        ) {
            info!(
                "executor_args_rewrite task_id={} round={} step={} type=scratch_cleanup_directory skill={} args={}",
                task.task_id,
                loop_state.round_no,
                step_in_round,
                normalized_skill,
                crate::truncate_for_log(&resolved_args.to_string())
            );
        }
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
    let removed_metadata =
        strip_unsupported_planner_metadata_args(state, &normalized_skill, &mut resolved_args);
    if !removed_metadata.is_empty() {
        info!(
            "executor_args_rewrite task_id={} round={} step={} type=planner_metadata_strip skill={} removed={:?} args={}",
            task.task_id,
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            removed_metadata,
            crate::truncate_for_log(&resolved_args.to_string())
        );
    }
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
            if let Some(route) = agent_run_context.and_then(|context| context.route_result.as_ref())
            {
                if let Some(answer) =
                    crate::agent_engine::observed_output::structured_scalar_equality_direct_answer(
                        Some(state),
                        route,
                        loop_state,
                        agent_run_context,
                    )
                {
                    return Ok(answer);
                }
            }
            if let Some(answer) = archive_database_aggregate_structured_answer(loop_state) {
                return Ok(answer);
            }
            if let Some(answer) = package_docker_probe_structured_answer(loop_state) {
                return Ok(answer);
            }
            if let Some(answer) =
                filesystem_mutation_lifecycle_structured_answer(loop_state, agent_run_context)
            {
                return Ok(answer);
            }
            if let Some(answer) = synthesize_contract_matrix_direct_observed_fallback_answer(
                state,
                loop_state,
                agent_run_context,
            ) {
                return Ok(answer);
            }
            if let Some(answer) =
                crate::agent_engine::observed_output::direct_answer_from_referenced_observation_i18n(
                    loop_state,
                    state,
                    agent_run_context,
                    evidence_refs,
                )
            {
                return Ok(answer);
            }
            if !synthesize_route_prefers_model_language_observed_status(agent_run_context) {
                if let Some(answer) = deterministic_observed_execution_status_answer(
                    state, task, user_text, loop_state,
                ) {
                    return Ok(answer);
                }
            }
            if agent_run_context
                .and_then(|context| context.route_result.as_ref())
                .is_none_or(|route| {
                    route.output_contract.semantic_kind != crate::OutputSemanticKind::ConfigMutation
                })
            {
                if let Some((answer, _summary)) =
                    crate::finalize::direct_config_edit_observed_answer(
                        state,
                        synthesize_user_language_source(user_text, agent_run_context),
                        loop_state,
                    )
                {
                    return Ok(answer);
                }
            }
            if let Some(answer) =
                deterministic_scalar_markdown_heading_answer(loop_state, agent_run_context)
            {
                return Ok(answer);
            }
            if let Some((answer, _summary)) =
                crate::finalize::deterministic_matrix_observed_shape_answer(
                    state,
                    task,
                    user_text,
                    loop_state,
                    agent_run_context,
                )
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
                .and_then(|(answer, summary)| {
                    (summary.disposition
                        == Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
                        && summary.completion_ok == Some(true))
                    .then_some(answer)
                })
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
        AgentAction::CallCapability { capability, args } => {
            Err(unresolved_capability_error(state, &capability, &args))
        }
        AgentAction::SynthesizeAnswer { evidence_refs } => {
            if active_recipe_terminal_discussion_should_replan(actions, loop_state, policy, idx) {
                record_active_recipe_terminal_discussion_replan(
                    state,
                    task,
                    loop_state,
                    global_step,
                    step_in_round,
                    "synthesize_answer",
                );
                *executed_actions += 1;
                loop_state.total_steps_executed += 1;
                return Ok(ActionLoopDecision::StopRound(
                    "recoverable_failure_continue_round".to_string(),
                ));
            }
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
