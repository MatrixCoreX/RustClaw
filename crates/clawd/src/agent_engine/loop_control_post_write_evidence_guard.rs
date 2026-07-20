use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use serde_json::Value;

use super::{attempt_ledger, execute_actions_once, AgentLoopGuardPolicy};

use crate::{
    agent_engine::{AgentRunContext, LoopState},
    task_journal::TaskJournalStepTrace,
    AgentAction, AppState, AskReply, ClaimedTask,
};

#[derive(Debug, Clone)]
struct StepActionRecord {
    index: usize,
    action: String,
    path: Option<String>,
    content_evidence: bool,
}

pub(super) fn enforce_post_write_content_evidence_guard(reply: &mut AskReply) -> bool {
    let Some(journal) = reply.task_journal.as_mut() else {
        return false;
    };
    let existing_post_write_gap = journal
        .answer_verifier_summary
        .as_ref()
        .is_some_and(|summary| {
            !summary.pass
                && summary
                    .answer_incomplete_reason
                    .starts_with("post_write_content_evidence_required")
        });
    let records = post_write_step_action_records(&journal.step_results);
    if !records
        .iter()
        .any(|record| is_validation_action(&record.action))
    {
        return false;
    }
    if !records
        .iter()
        .any(|record| is_code_or_test_write_action(&record.action))
    {
        return false;
    }
    let missing_paths = missing_post_write_content_evidence_paths_from_records(&records);
    if missing_paths.is_empty() {
        if existing_post_write_gap {
            journal.answer_verifier_summary = None;
        }
        return false;
    }
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["content_excerpt".to_string()],
        answer_incomplete_reason: format!(
            "post_write_content_evidence_required paths={}",
            missing_paths.join(",")
        ),
        should_retry: true,
        retry_instruction:
            "collect bounded content excerpts for modified code/test files after the write with fs_basic.read_text_range or fs_basic.grep_text, then finalize from observed content and validation evidence"
                .to_string(),
        confidence: 0.96,
    });
    true
}

pub(super) fn enforce_code_mutation_validation_success_guard(reply: &mut AskReply) -> bool {
    let unresolved_fields = final_local_code_json_unresolved_fields(reply);
    let Some(journal) = reply.task_journal.as_mut() else {
        return false;
    };
    let validation_gap = if journal_has_code_write_followed_by_failed_validation(journal) {
        Some((
            vec!["validation_success".to_string()],
            "post_write_validation_failed".to_string(),
            "repair the mutated code/test files using the observed validation error, rerun the validation command, collect bounded post-write content evidence, then finalize from successful machine evidence"
                .to_string(),
            0.97,
        ))
    } else if !unresolved_fields.is_empty() && journal_has_code_or_test_write(journal) {
        let missing_fields = if unresolved_fields.iter().any(|field| field == "test_status") {
            vec!["validation_success".to_string()]
        } else {
            unresolved_fields
        };
        Some((
            missing_fields,
            "post_write_unresolved_machine_fields".to_string(),
            "collect the missing structured machine evidence from tools before publishing the final local-code JSON"
                .to_string(),
            0.96,
        ))
    } else {
        None
    };
    let Some((missing_evidence_fields, answer_incomplete_reason, retry_instruction, confidence)) =
        validation_gap
    else {
        return false;
    };
    if journal
        .answer_verifier_summary
        .as_ref()
        .is_some_and(|summary| {
            !summary.pass
                && summary
                    .answer_incomplete_reason
                    .starts_with(&answer_incomplete_reason)
        })
    {
        return false;
    }
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields,
        answer_incomplete_reason,
        should_retry: true,
        retry_instruction,
        confidence,
    });
    true
}

fn final_local_code_json_unresolved_fields(reply: &AskReply) -> Vec<String> {
    let Some(answer) = final_answer_candidate(reply) else {
        return Vec::new();
    };
    local_code_json_unresolved_fields(answer)
}

pub(super) fn local_code_json_answer_has_unresolved_publication(answer: &str) -> bool {
    !local_code_json_unresolved_fields(answer).is_empty()
}

fn local_code_json_unresolved_fields(answer: &str) -> Vec<String> {
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(answer.trim()) else {
        return Vec::new();
    };
    object
        .iter()
        .filter_map(|(key, value)| {
            (local_code_json_key(key) && json_value_is_unresolved_publication(value))
                .then(|| key.clone())
        })
        .collect()
}

fn final_answer_candidate(reply: &AskReply) -> Option<&str> {
    reply
        .messages
        .iter()
        .rev()
        .map(String::as_str)
        .find(|message| {
            let trimmed = message.trim();
            !trimmed.is_empty() && !crate::finalize::is_execution_summary_message(trimmed)
        })
        .or_else(|| {
            let trimmed = reply.text.trim();
            (!trimmed.is_empty()).then_some(trimmed)
        })
}

fn local_code_json_key(key: &str) -> bool {
    matches!(
        key,
        "created_files"
            | "changed_files"
            | "test_command"
            | "test_status"
            | "functions"
            | "error_codes"
            | "evidence_files"
            | "project_dir"
    )
}

fn json_value_is_unresolved_publication(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => {
            let trimmed = text.trim();
            trimmed.is_empty()
                || trimmed.contains("{{")
                || matches!(trimmed, "<missing>" | "not_observed" | "null")
        }
        Value::Array(items) => {
            items.is_empty() || items.iter().any(json_value_is_unresolved_publication)
        }
        Value::Object(object) => {
            object.is_empty() || object.values().any(json_value_is_unresolved_publication)
        }
        Value::Bool(_) | Value::Number(_) => false,
    }
}

fn journal_has_code_or_test_write(journal: &crate::task_journal::TaskJournal) -> bool {
    post_write_step_action_records(&journal.step_results)
        .iter()
        .any(|record| {
            is_code_or_test_write_action(&record.action)
                && record
                    .path
                    .as_deref()
                    .is_some_and(path_looks_like_code_or_test)
        })
}

pub(super) fn journal_has_code_write_followed_by_failed_validation(
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    let records = post_write_step_action_records(&journal.step_results);
    let latest_code_write_index = records
        .iter()
        .filter(|record| {
            is_code_or_test_write_action(&record.action)
                && record
                    .path
                    .as_deref()
                    .is_some_and(path_looks_like_code_or_test)
        })
        .map(|record| record.index)
        .max();
    let Some(latest_code_write_index) = latest_code_write_index else {
        return false;
    };
    journal
        .step_results
        .iter()
        .enumerate()
        .skip(latest_code_write_index + 1)
        .any(|(_, step)| validation_step_failed(step))
}

pub(super) fn commit_local_code_strict_json_projection_after_readback(
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let Some(answer) =
        crate::agent_engine::dispatch_support::local_code_strict_json_projection_from_machine_evidence(
            user_text,
            loop_state,
            agent_run_context,
        )
    else {
        return false;
    };

    loop_state.delivery_messages.clear();
    crate::agent_engine::append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        answer.clone(),
    );
    loop_state.last_user_visible_respond = Some(answer.clone());
    loop_state.last_publishable_synthesis_output = Some(answer.clone());
    loop_state.output_vars.insert(
        "agent_loop.strict_json_projection_publishable".to_string(),
        "true".to_string(),
    );
    loop_state.output_vars.insert(
        "agent_loop.strict_json_projection_output".to_string(),
        answer,
    );
    true
}

pub(super) async fn try_run_post_write_validation_reserve_recovery(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    policy: &AgentLoopGuardPolicy,
    loop_state: &mut LoopState,
    reply: &AskReply,
    agent_run_context: Option<&AgentRunContext>,
) -> Result<bool, String> {
    if loop_state
        .output_vars
        .contains_key("agent_loop.post_write_validation_reserve_recovery_used")
    {
        return Ok(false);
    }
    let max_actions = policy.max_steps.max(1).min(8);
    let actions = post_write_validation_reserve_actions(
        state,
        reply,
        loop_state,
        max_actions,
        user_text,
        agent_run_context,
    );
    if actions.is_empty() {
        return Ok(false);
    }
    let recovery_policy =
        post_write_content_evidence_readback_recovery_policy(policy, loop_state, actions.len());

    loop_state.has_recoverable_failure_context = true;
    loop_state.delivery_messages.clear();
    loop_state.last_user_visible_respond = None;
    loop_state.last_publishable_synthesis_output = None;
    loop_state.last_stop_signal = Some("post_write_validation_reserve".to_string());
    loop_state.output_vars.insert(
        "agent_loop.post_write_validation_reserve_recovery_used".to_string(),
        "true".to_string(),
    );
    loop_state.output_vars.insert(
        "agent_loop.post_write_validation_reserve_budget_reserved".to_string(),
        actions.len().to_string(),
    );
    attempt_ledger::record_attempt_with_retry_instruction(
        loop_state,
        "answer_verifier",
        &format!("deterministic_validation_reserve_actions={}", actions.len()),
        crate::executor::StepExecutionStatus::Error,
        "post_write_validation_reserve_required",
        Some("answer_incomplete"),
        "post_write_validation_reserve_required",
        Some("collect_planned_post_write_observe_validate_actions"),
    );

    let outcome = execute_actions_once(
        state,
        task,
        goal,
        user_text,
        &actions,
        loop_state,
        &recovery_policy,
        agent_run_context,
    )
    .await?;
    commit_local_code_strict_json_projection_after_readback(
        task,
        user_text,
        loop_state,
        agent_run_context,
    );
    loop_state.last_stop_signal = Some(
        outcome
            .stop_signal
            .unwrap_or_else(|| "post_write_validation_reserve_collected".to_string()),
    );
    Ok(outcome.executed_actions > 0)
}

pub(super) fn post_write_validation_reserve_actions(
    state: &AppState,
    reply: &AskReply,
    loop_state: &LoopState,
    max_actions: usize,
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> Vec<AgentAction> {
    if max_actions == 0
        || loop_state.last_stop_signal.as_deref() != Some("max_tool_calls")
        || loop_state
            .output_vars
            .contains_key("agent_loop.post_write_validation_reserve_recovery_used")
    {
        return Vec::new();
    }
    let records = reply
        .task_journal
        .as_ref()
        .map(|journal| post_write_step_action_records(&journal.step_results))
        .unwrap_or_default();
    let has_code_write = records
        .iter()
        .any(|record| is_code_or_test_write_action(&record.action));
    let requested_validation_fields =
        local_code_validation_fields_requested_for_reserve(user_text, agent_run_context);
    let readback_only_validation_context = !has_code_write
        && (readback_only_code_validation_context(&records)
            || (requested_validation_fields
                && readback_only_code_context_has_source_and_test(&records)));
    if !has_code_write && !readback_only_validation_context {
        return Vec::new();
    }
    let missing_paths = missing_post_write_content_evidence_paths_from_records(&records);
    let saw_validation = records
        .iter()
        .any(|record| is_validation_action(&record.action));
    if has_code_write && missing_paths.is_empty() && saw_validation {
        return Vec::new();
    }
    let executed_validation_commands = if readback_only_validation_context {
        executed_run_cmd_commands(loop_state)
    } else {
        BTreeSet::new()
    };
    let mut selected = Vec::new();
    let mut seen = BTreeSet::new();
    for action in latest_plan_trace_actions(loop_state) {
        let action = normalize_recovery_candidate_action(state, action);
        if !post_write_validation_reserve_action_allowed(state, loop_state, &action, &missing_paths)
        {
            continue;
        }
        if validation_action_already_executed(state, &action, &executed_validation_commands) {
            continue;
        }
        let fingerprint =
            serde_json::to_string(&action).unwrap_or_else(|_| format!("{:?}", action));
        if !seen.insert(fingerprint) {
            continue;
        }
        selected.push(action);
        if selected.len() >= max_actions {
            break;
        }
    }
    selected
}

fn readback_only_code_validation_context(records: &[StepActionRecord]) -> bool {
    records
        .iter()
        .any(|record| is_validation_action(&record.action))
        && readback_only_code_context_has_source_and_test(records)
}

fn readback_only_code_context_has_source_and_test(records: &[StepActionRecord]) -> bool {
    let mut source_dirs = BTreeSet::new();
    let mut test_dirs = BTreeSet::new();
    for record in records {
        if !record.content_evidence || !is_content_evidence_action(&record.action) {
            continue;
        }
        let Some(path) = record.path.as_deref() else {
            continue;
        };
        if !path_looks_like_code_or_test(path) {
            continue;
        }
        let Some(parent) = normalized_parent_dir(path) else {
            continue;
        };
        if path_looks_like_test_file(path) {
            test_dirs.insert(parent);
        } else {
            source_dirs.insert(parent);
        }
    }
    !source_dirs.is_empty()
        && !test_dirs.is_empty()
        && source_dirs.iter().any(|dir| test_dirs.contains(dir))
}

fn local_code_validation_fields_requested_for_reserve(
    user_text: &str,
    agent_run_context: Option<&AgentRunContext>,
) -> bool {
    let mut surfaces = Vec::new();
    if let Some(context) = agent_run_context {
        if let Some(state_patch) = context
            .turn_analysis
            .as_ref()
            .and_then(|analysis| analysis.state_patch.as_ref())
        {
            crate::machine_kv_projection::collect_requested_machine_kv_surfaces_from_state_patch(
                state_patch,
                &mut surfaces,
            );
        }
        for value in [
            context.original_user_request.as_deref(),
            context.user_request.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            crate::machine_kv_projection::push_unique_machine_kv_surface(&mut surfaces, value);
        }
    }
    crate::machine_kv_projection::push_unique_machine_kv_surface(&mut surfaces, user_text);
    surfaces.iter().any(|surface| {
        crate::machine_kv_projection::requested_machine_markers_for_projection(surface)
            .iter()
            .any(|field| matches!(field.as_str(), "test_command" | "test_status"))
    })
}

fn executed_run_cmd_commands(loop_state: &LoopState) -> BTreeSet<String> {
    loop_state
        .output_vars
        .get("agent_loop.run_cmd_commands")
        .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|command| command.trim().to_string())
        .filter(|command| !command.is_empty())
        .collect()
}

fn validation_action_already_executed(
    state: &AppState,
    action: &AgentAction,
    executed_commands: &BTreeSet<String>,
) -> bool {
    if executed_commands.is_empty() {
        return false;
    }
    let Some((skill, args)) = concrete_skill_action_args(action) else {
        return false;
    };
    let canonical = state.resolve_canonical_skill_name(skill);
    if !matches!(canonical.as_str(), "run_cmd" | "process_basic") {
        return false;
    }
    args.get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .is_some_and(|command| executed_commands.contains(command))
}

pub(super) fn post_write_content_evidence_readback_recovery_policy(
    policy: &AgentLoopGuardPolicy,
    loop_state: &LoopState,
    action_count: usize,
) -> AgentLoopGuardPolicy {
    let mut recovery_policy = policy.clone();
    let bounded_extra_tool_calls = action_count.min(8);
    recovery_policy.max_tool_calls = recovery_policy.max_tool_calls.max(
        loop_state
            .tool_calls_total
            .saturating_add(bounded_extra_tool_calls),
    );
    recovery_policy.max_steps = recovery_policy.max_steps.max(action_count.max(1));
    recovery_policy
}

fn latest_plan_trace_actions(loop_state: &LoopState) -> Vec<AgentAction> {
    loop_state
        .round_traces
        .iter()
        .rev()
        .find_map(|round| round.plan_result.as_ref())
        .map(|plan| {
            plan.steps
                .iter()
                .filter_map(crate::PlanStep::to_agent_action)
                .collect()
        })
        .unwrap_or_default()
}

fn normalize_recovery_candidate_action(state: &AppState, action: AgentAction) -> AgentAction {
    let action = crate::capability_resolver::resolve_agent_action_for_state(state, action);
    match action {
        AgentAction::CallTool { tool, args } => {
            if let Some((tool, args)) =
                normalize_dotted_machine_action_ref(state, &tool, args.clone())
            {
                AgentAction::CallTool { tool, args }
            } else {
                AgentAction::CallTool { tool, args }
            }
        }
        AgentAction::CallSkill { skill, args } => {
            if let Some((skill, args)) =
                normalize_dotted_machine_action_ref(state, &skill, args.clone())
            {
                AgentAction::CallSkill { skill, args }
            } else {
                AgentAction::CallSkill { skill, args }
            }
        }
        AgentAction::CallCapability { capability, args } => {
            if normalize_capability_ref(&capability).as_deref() == Some("system_run_command") {
                AgentAction::CallSkill {
                    skill: "run_cmd".to_string(),
                    args,
                }
            } else {
                AgentAction::CallCapability { capability, args }
            }
        }
        other => other,
    }
}

fn normalize_capability_ref(capability: &str) -> Option<String> {
    normalize_machine_token(&capability.replace('.', "_"))
}

fn normalize_dotted_machine_action_ref(
    state: &AppState,
    skill_ref: &str,
    args: Value,
) -> Option<(String, Value)> {
    let (skill, action) = skill_ref.split_once('.')?;
    let skill = normalize_machine_token(skill)?;
    let action = normalize_machine_token(action)?;
    if skill.is_empty() || action.is_empty() {
        return None;
    }
    let canonical_skill = state.resolve_canonical_skill_name(&skill);
    if canonical_skill.is_empty() {
        return None;
    }
    let mut args_map = args.as_object().cloned().unwrap_or_default();
    args_map
        .entry("action".to_string())
        .or_insert_with(|| Value::String(action));
    Some((canonical_skill, Value::Object(args_map)))
}

fn normalize_machine_token(token: &str) -> Option<String> {
    let normalized = token
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| if matches!(ch, '-' | ' ') { '_' } else { ch })
        .collect::<String>();
    (!normalized.is_empty()
        && normalized
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_'))
    .then_some(normalized)
}

fn post_write_validation_reserve_action_allowed(
    state: &AppState,
    loop_state: &LoopState,
    action: &AgentAction,
    missing_paths: &[String],
) -> bool {
    if planned_readback_action_matches_missing_path(state, action, missing_paths) {
        return true;
    }
    planned_validation_action_is_safe_to_resume(state, loop_state, action)
}

fn planned_readback_action_matches_missing_path(
    state: &AppState,
    action: &AgentAction,
    missing_paths: &[String],
) -> bool {
    if missing_paths.is_empty() {
        return false;
    }
    let Some((skill, args)) = concrete_skill_action_args(action) else {
        return false;
    };
    let canonical = state.resolve_canonical_skill_name(skill);
    if !matches!(canonical.as_str(), "fs_basic" | "system_basic") {
        return false;
    }
    let action_name = args
        .get("action")
        .and_then(Value::as_str)
        .map(normalize_action_name)
        .unwrap_or_default();
    if !is_content_evidence_action(&action_name) {
        return false;
    }
    let Some(path) = ["resolved_path", "effective_path", "path"]
        .iter()
        .find_map(|field| args.get(*field).and_then(Value::as_str))
    else {
        return false;
    };
    missing_paths
        .iter()
        .any(|missing_path| paths_match(path, missing_path))
}

fn planned_validation_action_is_safe_to_resume(
    state: &AppState,
    loop_state: &LoopState,
    action: &AgentAction,
) -> bool {
    let Some((skill, args)) = concrete_skill_action_args(action) else {
        return false;
    };
    let canonical = state.resolve_canonical_skill_name(skill);
    if !matches!(
        canonical.as_str(),
        "run_cmd" | "process_basic" | "health_check"
    ) {
        return false;
    }
    let raw_effect = crate::execution_recipe::classify_skill_action_effect(state, &canonical, args);
    let effect = crate::execution_recipe::effective_action_effect_for_recipe(
        loop_state.execution_recipe,
        raw_effect,
    );
    (raw_effect.validates && !raw_effect.mutates) || (effect.validates && !effect.mutates)
}

fn concrete_skill_action_args(action: &AgentAction) -> Option<(&str, &Value)> {
    match action {
        AgentAction::CallTool { tool, args } => Some((tool.as_str(), args)),
        AgentAction::CallSkill { skill, args } => Some((skill.as_str(), args)),
        AgentAction::CallCapability { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => None,
    }
}

fn post_write_step_action_records(steps: &[TaskJournalStepTrace]) -> Vec<StepActionRecord> {
    steps
        .iter()
        .enumerate()
        .flat_map(|(index, step)| step_action_records(index, step))
        .collect()
}

fn missing_post_write_content_evidence_paths_from_records(
    records: &[StepActionRecord],
) -> Vec<String> {
    let mut latest_code_writes: BTreeMap<String, usize> = BTreeMap::new();
    for record in records {
        let Some(path) = record.path.as_deref() else {
            continue;
        };
        if is_code_or_test_write_action(&record.action) && path_looks_like_code_or_test(path) {
            latest_code_writes.insert(normalize_step_path(path), record.index);
        }
    }
    latest_code_writes
        .iter()
        .filter_map(|(path, write_index)| {
            (!has_post_write_content_evidence(records, path, *write_index)).then(|| path.clone())
        })
        .collect()
}

fn has_post_write_content_evidence(
    records: &[StepActionRecord],
    path: &str,
    write_index: usize,
) -> bool {
    records.iter().any(|record| {
        (record.index > write_index
            || (record.index == write_index && record.action == "shell_write"))
            && record.content_evidence
            && record
                .path
                .as_deref()
                .is_some_and(|candidate| paths_match(candidate, path))
    })
}

fn step_action_records(index: usize, step: &TaskJournalStepTrace) -> Vec<StepActionRecord> {
    if step.status != crate::executor::StepExecutionStatus::Ok {
        return Vec::new();
    }
    if is_validation_skill(&step.skill) {
        let mut records = vec![StepActionRecord {
            index,
            action: "run_cmd".to_string(),
            path: None,
            content_evidence: false,
        }];
        if let Some(output) = step.output_excerpt.as_deref() {
            records.extend(
                shell_write_records_from_run_cmd_output(output)
                    .into_iter()
                    .map(|(path, content_evidence)| StepActionRecord {
                        index,
                        action: "shell_write".to_string(),
                        path: Some(path),
                        content_evidence,
                    }),
            );
        }
        return records;
    }
    let Some(output) = step.output_excerpt.as_deref() else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(output.trim()) else {
        return Vec::new();
    };
    let extra = value
        .get("extra")
        .filter(|extra| extra.is_object())
        .unwrap_or(&value);
    let action = extra
        .get("action")
        .and_then(Value::as_str)
        .or_else(|| action_from_text_output(&value))
        .unwrap_or(step.skill.as_str())
        .to_string();
    let path = ["resolved_path", "effective_path", "path"]
        .iter()
        .find_map(|field| extra.get(*field).and_then(Value::as_str))
        .map(str::to_string);
    let content_evidence = is_content_evidence_action(&action)
        && (extra.get("excerpt").and_then(Value::as_str).is_some()
            || extra
                .get("content_excerpt")
                .and_then(Value::as_str)
                .is_some()
            || value
                .get("content_excerpt")
                .and_then(Value::as_str)
                .is_some());
    vec![StepActionRecord {
        index,
        action,
        path,
        content_evidence,
    }]
}

fn action_from_text_output(value: &Value) -> Option<&str> {
    let text = value.get("text").and_then(Value::as_str)?;
    if text.starts_with("written ") {
        Some("write_text")
    } else {
        None
    }
}

fn is_validation_skill(skill: &str) -> bool {
    matches!(skill, "run_cmd" | "process_basic")
}

fn validation_step_failed(step: &TaskJournalStepTrace) -> bool {
    is_validation_skill(&step.skill) && step.status == crate::executor::StepExecutionStatus::Error
}

fn is_validation_action(action: &str) -> bool {
    matches!(action, "run_cmd" | "process_basic")
}

fn normalize_action_name(action: &str) -> String {
    action
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if matches!(ch, '-' | ' ' | '.') {
                '_'
            } else {
                ch
            }
        })
        .collect()
}

fn is_code_or_test_write_action(action: &str) -> bool {
    matches!(action, "write_text" | "append_text" | "shell_write")
}

fn is_content_evidence_action(action: &str) -> bool {
    matches!(action, "read_range" | "read_text_range" | "grep_text")
}

fn normalize_step_path(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string()
}

fn normalized_parent_dir(path: &str) -> Option<String> {
    let normalized = normalize_step_path(path);
    let path = Path::new(&normalized);
    let parent = if path.extension().is_some() {
        path.parent()?
    } else {
        path
    };
    let rendered = parent.to_string_lossy().trim().to_string();
    (!rendered.is_empty()).then_some(rendered)
}

fn paths_match(candidate: &str, expected: &str) -> bool {
    let candidate = normalize_step_path(candidate);
    let expected = normalize_step_path(expected);
    if candidate == expected {
        return true;
    }
    let (shorter, longer) = if candidate.len() <= expected.len() {
        (candidate.as_str(), expected.as_str())
    } else {
        (expected.as_str(), candidate.as_str())
    };
    !shorter.starts_with('/') && !shorter.is_empty() && longer.ends_with(&format!("/{shorter}"))
}

fn path_looks_like_code_or_test(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    const CODE_EXTENSIONS: &[&str] = &[
        ".py", ".rs", ".js", ".jsx", ".ts", ".tsx", ".go", ".java", ".kt", ".c", ".cc", ".cpp",
        ".h", ".hpp", ".cs", ".php", ".rb", ".swift", ".scala", ".sh", ".bash", ".zsh", ".fish",
        ".ps1", ".sql",
    ];
    CODE_EXTENSIONS
        .iter()
        .any(|extension| lower.ends_with(extension))
}

fn path_looks_like_test_file(path: &str) -> bool {
    let normalized = normalize_step_path(path);
    let basename = Path::new(&normalized)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(normalized.as_str())
        .to_ascii_lowercase();
    basename.starts_with("test_")
        || basename.ends_with("_test.py")
        || basename.ends_with(".test.js")
        || basename.ends_with(".spec.js")
        || basename.ends_with(".test.ts")
        || basename.ends_with(".spec.ts")
        || basename.ends_with("_test.rs")
}

fn shell_write_records_from_run_cmd_output(output: &str) -> Vec<(String, bool)> {
    let Some(command) = run_cmd_command_from_output(output) else {
        return Vec::new();
    };
    shell_redirection_records(command)
        .into_iter()
        .filter(|(path, _)| path_looks_like_code_or_test(path))
        .collect()
}

fn run_cmd_command_from_output(output: &str) -> Option<&str> {
    let trimmed = output.trim_start();
    if let Some(command) = trimmed.strip_prefix("command=") {
        return Some(command.trim());
    }
    trimmed
        .find(" command=")
        .map(|index| trimmed[index + " command=".len()..].trim())
}

fn shell_redirection_records(command: &str) -> Vec<(String, bool)> {
    let mut records = Vec::new();
    let mut heredoc: Option<(String, Vec<String>, bool)> = None;
    for line in command.lines() {
        let trimmed = line.trim();
        if let Some((marker, targets, has_content)) = heredoc.as_mut() {
            if trimmed == marker {
                records.extend(targets.iter().cloned().map(|target| (target, *has_content)));
                heredoc = None;
            } else if !trimmed.is_empty() {
                *has_content = true;
            }
            continue;
        }
        let targets = shell_redirection_targets_in_line(line);
        if let Some(marker) = heredoc_marker_in_line(line) {
            if !targets.is_empty() {
                heredoc = Some((marker, targets, false));
            }
        } else {
            records.extend(targets.into_iter().map(|target| (target, false)));
        }
    }
    if let Some((_, targets, has_content)) = heredoc {
        records.extend(targets.into_iter().map(|target| (target, has_content)));
    }
    records
}

fn shell_redirection_targets_in_line(line: &str) -> Vec<String> {
    let bytes = line.as_bytes();
    let mut targets = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'>' {
            index += 1;
            continue;
        }
        let previous = index.checked_sub(1).map(|i| bytes[i]);
        if matches!(previous, Some(b'1' | b'2' | b'&' | b'>')) {
            index += 1;
            continue;
        }
        let mut after = index + 1;
        if after < bytes.len() && bytes[after] == b'>' {
            after += 1;
        }
        if after < bytes.len() && bytes[after] == b'&' {
            index = after + 1;
            continue;
        }
        if let Some((path, consumed)) = read_shell_word(&line[after..]) {
            if shell_redirection_target_is_file(&path) {
                targets.push(path);
            }
            index = after + consumed;
        } else {
            index = after;
        }
    }
    targets
}

fn read_shell_word(input: &str) -> Option<(String, usize)> {
    let leading_ws = input.len() - input.trim_start().len();
    let rest = &input[leading_ws..];
    let first = rest.chars().next()?;
    if matches!(first, '\'' | '"') {
        let quote_len = first.len_utf8();
        let body = &rest[quote_len..];
        let end = body.find(first)?;
        return Some((
            body[..end].to_string(),
            leading_ws + quote_len + end + quote_len,
        ));
    }
    let end = rest
        .char_indices()
        .find_map(|(offset, ch)| {
            (ch.is_whitespace() || matches!(ch, ';' | '&' | '|' | '<' | '>')).then_some(offset)
        })
        .unwrap_or(rest.len());
    (end > 0).then(|| (rest[..end].to_string(), leading_ws + end))
}

fn shell_redirection_target_is_file(path: &str) -> bool {
    let trimmed = path.trim();
    !trimmed.is_empty() && !trimmed.starts_with('&') && trimmed != "/dev/null" && trimmed != "-"
}

fn heredoc_marker_in_line(line: &str) -> Option<String> {
    let marker_start = line.find("<<")? + 2;
    let (_, consumed) = read_shell_word(&line[marker_start..])?;
    let raw = line[marker_start..marker_start + consumed].trim();
    let marker = raw.trim_matches('\'').trim_matches('"').trim().to_string();
    (!marker.is_empty()).then_some(marker)
}
