use serde_json::Value;
use std::path::Path;
use tracing::{debug, info};

use super::{
    append_delivery_message, append_progress_hint, build_safe_skill_args_summary,
    encode_progress_i18n, execute_prepared_skill_action, register_step_output, resolve_arg_string,
    resolve_arg_value, rewrite_args_with_auto_locator_path, rewrite_run_cmd_with_written_aliases,
    rewrite_tool_path_with_written_aliases, ActionLoopDecision, AgentLoopGuardPolicy,
    AgentRunContext, AppState, ClaimedTask, LoopState, RespondActionOutcome, SkillActionOutcome,
    WriteFileEffectivePath, PROGRESS_ARGS_SUMMARY_MAX_LEN,
};
use crate::AgentAction;

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
    if let Some(saved_path) = loop_state.last_written_file_path.as_deref() {
        let trimmed = out.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("saved path:") && !trimmed.contains(saved_path) {
            return format!("Saved path: {saved_path}");
        }
        if (trimmed.starts_with("保存路径") || trimmed.starts_with("文件路径"))
            && !trimmed.contains(saved_path)
        {
            return format!("保存路径：{saved_path}");
        }
        if lower.contains("saved path to ")
            && lower.contains(": written ")
            && !trimmed.contains(saved_path)
        {
            return format!("Saved path: {saved_path}");
        }
    }
    out
}

fn rewrite_bounded_list_dir_last_output_placeholder(
    content: &str,
    loop_state: &LoopState,
) -> String {
    if !content.contains("{{last_output}}") {
        return content.to_string();
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(content);
    let Some(limit) = surface.requested_listing_limit else {
        return content.to_string();
    };
    let Some(last_ok_step) = loop_state
        .executed_step_results
        .iter()
        .rev()
        .find(|step| step.is_ok())
    else {
        return content.to_string();
    };
    if last_ok_step.skill != "list_dir" {
        return content.to_string();
    }
    let Some(listing) = loop_state.last_output.as_deref() else {
        return content.to_string();
    };
    let Some(trimmed_listing) =
        crate::agent_engine::observed_output::normalized_observed_listing(listing, Some(limit))
    else {
        return content.to_string();
    };
    content.replace("{{last_output}}", &trimmed_listing)
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

pub(super) fn classify_skill_failure_recovery(
    state: &AppState,
    actions: &[AgentAction],
    current_idx: usize,
    max_steps: usize,
    normalized_skill: &str,
    call_args: Option<&Value>,
    err: &str,
) -> Option<&'static str> {
    if crate::skills::is_recoverable_skill_error(normalized_skill, err) {
        if has_remaining_action_after(actions, current_idx, max_steps) {
            return Some("recoverable_failure_continue_in_round");
        }
        return Some("recoverable_failure_finalize");
    }
    if state.skill_is_retryable(normalized_skill)
        && !state.skill_requires_confirmation_policy(normalized_skill)
    {
        if has_remaining_action_after(actions, current_idx, max_steps) {
            return Some("recoverable_failure_continue_in_round");
        }
        if remaining_actions_are_discussion_only(actions, current_idx, max_steps) {
            return Some("recoverable_failure_continue_in_round");
        }
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
    None
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::path::Path;
    use std::sync::{Arc, RwLock};

    use super::{
        classify_skill_failure_recovery, rewrite_bounded_list_dir_last_output_placeholder,
    };
    use crate::agent_engine::LoopState;
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
    fn bounded_list_dir_placeholder_rewrite_truncates_to_requested_limit() {
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "list_dir", "a\nb\nc\nd\n"));
        loop_state.last_output = Some("a\nb\nc\nd\n".to_string());

        let rewritten = rewrite_bounded_list_dir_last_output_placeholder(
            "logs 目录下前 2 个文件名：\n{{last_output}}",
            &loop_state,
        );

        assert_eq!(rewritten, "logs 目录下前 2 个文件名：\na\nb");
    }

    #[test]
    fn bounded_list_dir_placeholder_rewrite_ignores_non_listing_outputs() {
        let mut loop_state = LoopState::new(2);
        loop_state
            .executed_step_results
            .push(ok_step("step_1", "read_file", "alpha\nbeta\n"));
        loop_state.last_output = Some("alpha\nbeta\n".to_string());

        let content = "前 2 行：\n{{last_output}}";
        assert_eq!(
            rewrite_bounded_list_dir_last_output_placeholder(content, &loop_state),
            content
        );
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
                a.eq_ignore_ascii_case("sqlite_query") || a.eq_ignore_ascii_case("schema_version")
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
) -> RespondActionOutcome {
    let rewritten_content = rewrite_bounded_list_dir_last_output_placeholder(content, loop_state);
    let text = rewrite_response_with_written_aliases(
        &resolve_arg_string(&rewritten_content, loop_state)
            .trim()
            .to_string(),
        loop_state,
    )
    .trim()
    .to_string();

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
    let normalized_skill = state.resolve_canonical_skill_name(tool);
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
    let read_file_requested_path = read_file_requested_path(tool, &resolved_args);
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
        None,
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
    let normalized_skill = state.resolve_canonical_skill_name(skill);
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
            crate::agent_engine::observed_output::synthesize_answer_from_observed_output(
                state,
                task,
                user_text,
                loop_state,
                agent_run_context,
            )
            .await
            .map(|(answer, _summary)| answer)
            .filter(|answer| !answer.trim().is_empty())
            .ok_or_else(|| {
                if loop_state.executed_step_results.is_empty() {
                    "synthesize_answer has no observed execution evidence".to_string()
                } else {
                    "synthesize_answer could not produce a grounded publishable answer".to_string()
                }
            })
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
            Err(err)
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
