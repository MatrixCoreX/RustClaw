use serde_json::{json, Value};
use tracing::{debug, info, warn};

use super::{
    build_resume_context_error, classify_skill_failure_recovery, ensure_task_running,
    register_failed_step_output, register_file_path_output, register_step_output,
    remember_written_file_alias, AgentLoopGuardPolicy, AppState, ClaimedTask, LoopState,
    SkillActionOutcome, WriteFileEffectivePath, TASK_CANCELED_ERR,
};
use crate::{repo, run_skill_with_runner_outcome};

fn log_step_journal_summary(
    task: &ClaimedTask,
    round_no: usize,
    step_in_round: usize,
    action_trace_kind: &str,
    execution_recipe_summary: Option<&str>,
    step_execution: &crate::executor::StepExecutionResult,
) {
    let mut journal =
        crate::task_journal::TaskJournal::new(format!("step:{}", step_execution.skill));
    let mut summary = format!(
        "round={} step={} action_type={}",
        round_no, step_in_round, action_trace_kind
    );
    if let Some(recipe_summary) = execution_recipe_summary.filter(|v| !v.trim().is_empty()) {
        summary.push(' ');
        summary.push_str(recipe_summary);
    }
    journal.record_context_bundle_summary(summary);
    journal.push_step_result(step_execution);
    info!(
        "task_journal_summary task_id={} kind=ask phase=step_execute round={} step={} {}",
        task.task_id,
        round_no,
        step_in_round,
        journal.to_log_json()
    );
}

fn matches_json_schema_type(value: &Value, expected_type: &str) -> bool {
    match expected_type {
        "string" => value.is_string(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "boolean" => value.is_boolean(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        _ => true,
    }
}

fn validate_json_contract(value: &Value, schema: &Value) -> Result<(), String> {
    let expected_type = schema.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if !expected_type.is_empty() && !matches_json_schema_type(value, expected_type) {
        return Err(format!("expected type `{expected_type}`"));
    }
    if expected_type == "object" {
        let obj = value
            .as_object()
            .ok_or_else(|| "expected object output".to_string())?;
        if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
            for key in required.iter().filter_map(|item| item.as_str()) {
                if !obj.contains_key(key) {
                    return Err(format!("missing required field `{key}`"));
                }
            }
        }
        if let Some(properties) = schema.get("properties").and_then(|v| v.as_object()) {
            for (key, prop_schema) in properties {
                let Some(field_value) = obj.get(key) else {
                    continue;
                };
                if let Some(field_type) = prop_schema.get("type").and_then(|v| v.as_str()) {
                    if !matches_json_schema_type(field_value, field_type) {
                        return Err(format!("field `{key}` expected type `{field_type}`"));
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_skill_output_contract(
    state: &AppState,
    normalized_skill: &str,
    output: &str,
) -> Result<(), String> {
    let Some((output_kind, schema)) = state.skill_output_contract(normalized_skill) else {
        return Ok(());
    };
    let candidate = if output_kind == claw_core::skill_registry::OutputKind::Text {
        if schema.get("type").and_then(|v| v.as_str()) == Some("object")
            && schema
                .get("properties")
                .and_then(|v| v.as_object())
                .map(|props| props.contains_key("text"))
                .unwrap_or(false)
        {
            json!({ "text": output })
        } else {
            Value::String(output.to_string())
        }
    } else {
        crate::parse_llm_json_raw_or_any::<Value>(output)
            .unwrap_or_else(|| Value::String(output.to_string()))
    };
    validate_json_contract(&candidate, &schema)
}

fn remember_skill_metadata(loop_state: &mut LoopState, normalized_skill: &str) {
    loop_state
        .output_vars
        .insert("last_skill_name".to_string(), normalized_skill.to_string());
}

async fn handle_skill_step_success(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    fingerprint: &str,
    step_execution: &crate::executor::StepExecutionResult,
    global_step: usize,
    step_in_round: usize,
    normalized_skill: &str,
    action_trace_kind: &str,
    args_summary: &str,
    action_args: &Value,
    out: &str,
    action_effect: crate::execution_recipe::ActionEffect,
    validation_observation: crate::execution_recipe::ValidationObservation,
    write_file_effective_path: Option<&WriteFileEffectivePath>,
    read_file_requested_path: Option<&str>,
    cache_publishable_chat_output: bool,
) -> Result<SkillActionOutcome, String> {
    ensure_task_running(state, task)?;
    remember_skill_metadata(loop_state, normalized_skill);
    let mut publishable_chat_output = false;
    crate::execution_recipe::apply_target_scope_progress(
        &mut loop_state.execution_recipe,
        state,
        normalized_skill,
        action_args,
        true,
    );
    if let Err(contract_err) = validate_skill_output_contract(state, normalized_skill, out) {
        warn!(
            "skill_output_contract_mismatch task_id={} round={} step={} skill={} err={}",
            task.task_id,
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            crate::truncate_for_log(&contract_err)
        );
        loop_state.history_compact.push(format!(
            "round={} step={} skill={} output_contract_mismatch={}",
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            crate::truncate_for_agent_trace(&contract_err)
        ));
    }
    // §3.4: skill_execution 阶段不再调 semantic_judge LLM；改用本地 deterministic
    // guard 决定 "这份 chat 输出值不值得缓存为 finalize 兜底"。误缓存的会被
    // finalize 层 (loop_finalize::observed_generic_finalize) 用 is_publishable_raw
    // 二次过滤，不会出现"误投递"。
    if cache_publishable_chat_output
        && normalized_skill == "chat"
        && crate::semantic_judge::is_publishable_raw_local(out)
    {
        loop_state.last_publishable_chat_output = Some(out.to_string());
        publishable_chat_output = true;
    }
    if let Some((original_path, _effective_path, user_visible_path)) = write_file_effective_path {
        remember_written_file_alias(loop_state, original_path, user_visible_path);
        register_file_path_output(
            loop_state,
            global_step,
            step_in_round,
            &format!("skill.{normalized_skill}"),
            user_visible_path,
        );
    } else if let Some(path) = read_file_requested_path {
        register_file_path_output(
            loop_state,
            global_step,
            step_in_round,
            &format!("skill.{normalized_skill}"),
            path,
        );
    }
    crate::append_subtask_result(
        &mut loop_state.subtask_results,
        global_step,
        &format!("skill({normalized_skill})"),
        true,
        out,
    );
    let mut stop_signal = None;
    let mut mark_successful_fingerprint = true;
    match &validation_observation {
        crate::execution_recipe::ValidationObservation::Passed => {
            crate::execution_recipe::apply_action_effect_success(
                &mut loop_state.execution_recipe,
                action_effect,
            );
            super::maybe_publish_execution_recipe_phase_hint(state, task, loop_state);
        }
        crate::execution_recipe::ValidationObservation::Failed(detail) => {
            crate::execution_recipe::apply_action_effect_failure(
                &mut loop_state.execution_recipe,
                action_effect,
            );
            register_failed_step_output(
                loop_state,
                global_step,
                step_in_round,
                &format!("skill.{normalized_skill}"),
                &format!("skill({normalized_skill})"),
                detail,
            );
            super::maybe_publish_execution_recipe_phase_hint(state, task, loop_state);
            loop_state.history_compact.push(format!(
                "round={} step={} skill={} validation_failed={}",
                loop_state.round_no,
                step_in_round,
                normalized_skill,
                crate::truncate_for_agent_trace(detail)
            ));
            mark_successful_fingerprint = false;
            if loop_state.execution_recipe.is_active() {
                stop_signal = Some(
                    crate::execution_recipe::stop_signal_for_validation_failure(
                        &loop_state.execution_recipe,
                    )
                    .to_string(),
                );
            }
        }
        crate::execution_recipe::ValidationObservation::Inconclusive => {
            crate::execution_recipe::apply_action_effect_failure(
                &mut loop_state.execution_recipe,
                action_effect,
            );
            register_failed_step_output(
                loop_state,
                global_step,
                step_in_round,
                &format!("skill.{normalized_skill}"),
                &format!("skill({normalized_skill})"),
                "validation result was inconclusive",
            );
            super::maybe_publish_execution_recipe_phase_hint(state, task, loop_state);
            if action_effect.validates {
                loop_state.history_compact.push(format!(
                    "round={} step={} skill={} validation_inconclusive",
                    loop_state.round_no, step_in_round, normalized_skill
                ));
                mark_successful_fingerprint = false;
                if loop_state.execution_recipe.is_active() {
                    stop_signal = Some(
                        crate::execution_recipe::stop_signal_for_validation_failure(
                            &loop_state.execution_recipe,
                        )
                        .to_string(),
                    );
                }
            } else {
                crate::execution_recipe::apply_action_effect_success(
                    &mut loop_state.execution_recipe,
                    crate::execution_recipe::ActionEffect {
                        observes: action_effect.observes,
                        mutates: action_effect.mutates,
                        validates: false,
                    },
                );
            }
        }
    }
    let had_observed_output = !out.trim().is_empty();
    if had_observed_output {
        loop_state.has_tool_or_skill_output = true;
        let hint = if args_summary.is_empty() {
            super::encode_progress_i18n(
                "telegram.progress.skill_completed",
                &[("skill", normalized_skill)],
            )
        } else {
            super::encode_progress_i18n(
                "telegram.progress.skill_completed_with_args",
                &[("skill", normalized_skill), ("args_summary", args_summary)],
            )
        };
        super::append_progress_hint(state, task, &mut loop_state.progress_messages, hint);
    }
    register_step_output(
        loop_state,
        global_step,
        step_in_round,
        &format!("skill.{normalized_skill}"),
        out,
    );
    if mark_successful_fingerprint {
        *loop_state
            .successful_action_fingerprints
            .entry(fingerprint.to_string())
            .or_insert(0) += 1;
    }
    info!(
        "executor_result_ok task_id={} round={} step={} type={} output={} trace_only=raw_not_delivery",
        task.task_id,
        loop_state.round_no,
        step_in_round,
        action_trace_kind,
        crate::truncate_for_log(out)
    );
    loop_state.history_compact.push(format!(
        "round={} step={} skill={} ok",
        loop_state.round_no, step_in_round, normalized_skill
    ));
    debug!(
        "step_execution_result step_id={} skill={} status={} started_at={} finished_at={}",
        step_execution.step_id,
        step_execution.skill,
        step_execution.status.as_str(),
        step_execution.started_at,
        step_execution.finished_at
    );
    loop_state
        .executed_step_results
        .push(step_execution.clone());
    log_step_journal_summary(
        task,
        loop_state.round_no,
        step_in_round,
        action_trace_kind,
        loop_state
            .execution_recipe
            .is_active()
            .then(|| loop_state.execution_recipe.phase_summary_line())
            .as_deref(),
        step_execution,
    );
    // Raw skill output is trace/evidence, not final user-visible delivery.
    // Only publishable chat output counts as terminal user-visible output here.
    Ok(SkillActionOutcome {
        ended_with_user_visible_output: publishable_chat_output,
        stop_signal,
        continue_in_round: false,
    })
}

fn handle_skill_step_failure(
    state: &AppState,
    task: &ClaimedTask,
    step_execution: &crate::executor::StepExecutionResult,
    actions: &[crate::AgentAction],
    round_steps: &[String],
    loop_state: &mut LoopState,
    idx: usize,
    global_step: usize,
    step_in_round: usize,
    goal: &str,
    user_text: &str,
    policy: &AgentLoopGuardPolicy,
    normalized_skill: &str,
    recovery_args: Option<&Value>,
    err: &str,
    action_trace_kind: &str,
) -> Result<Option<String>, String> {
    let user_visible_err = crate::skills::normalize_skill_error_for_user(normalized_skill, err);
    let effect = recovery_args
        .map(|args| {
            crate::execution_recipe::apply_target_scope_progress(
                &mut loop_state.execution_recipe,
                state,
                normalized_skill,
                args,
                false,
            );
            crate::execution_recipe::classify_skill_action_effect(state, normalized_skill, args)
        })
        .unwrap_or_default();
    crate::execution_recipe::apply_action_effect_failure(&mut loop_state.execution_recipe, effect);
    super::maybe_publish_execution_recipe_phase_hint(state, task, loop_state);
    crate::append_subtask_result(
        &mut loop_state.subtask_results,
        global_step,
        &format!("skill({normalized_skill})"),
        false,
        &user_visible_err,
    );
    info!(
        "executor_result_error task_id={} round={} step={} type={} error={}",
        task.task_id,
        loop_state.round_no,
        step_in_round,
        action_trace_kind,
        crate::truncate_for_log(&user_visible_err)
    );
    loop_state
        .executed_step_results
        .push(step_execution.clone());
    log_step_journal_summary(
        task,
        loop_state.round_no,
        step_in_round,
        action_trace_kind,
        loop_state
            .execution_recipe
            .is_active()
            .then(|| loop_state.execution_recipe.phase_summary_line())
            .as_deref(),
        step_execution,
    );
    let has_remaining_actions = actions
        .iter()
        .take(policy.max_steps.max(1))
        .skip(idx + 1)
        .any(|action| !matches!(action, crate::AgentAction::Think { .. }));
    if normalized_skill.eq_ignore_ascii_case("chat")
        && loop_state.has_tool_or_skill_output
        && loop_state.delivery_messages.is_empty()
        && !has_remaining_actions
    {
        register_failed_step_output(
            loop_state,
            global_step,
            step_in_round,
            &format!("skill.{normalized_skill}"),
            &format!("skill({normalized_skill})"),
            &user_visible_err,
        );
        loop_state.history_compact.push(format!(
            "round={} step={} skill={} failed error={} finalize_from_observed=true",
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            crate::truncate_for_agent_trace(&user_visible_err)
        ));
        return Ok(Some("recoverable_failure_finalize".to_string()));
    }
    if let Some(stop_reason) = classify_skill_failure_recovery(
        state,
        actions,
        idx,
        policy.max_steps,
        normalized_skill,
        recovery_args,
        err,
    ) {
        register_failed_step_output(
            loop_state,
            global_step,
            step_in_round,
            &format!("skill.{normalized_skill}"),
            &format!("skill({normalized_skill})"),
            &user_visible_err,
        );
        loop_state.history_compact.push(format!(
            "round={} step={} skill={} failed error={}",
            loop_state.round_no,
            step_in_round,
            normalized_skill,
            crate::truncate_for_agent_trace(&user_visible_err)
        ));
        return Ok(Some(stop_reason.to_string()));
    }
    let resume_err = build_resume_context_error(
        state,
        actions,
        round_steps,
        user_text,
        goal,
        &loop_state.subtask_results,
        &loop_state.delivery_messages,
        step_in_round,
        &format!("skill({normalized_skill})"),
        &user_visible_err,
    );
    Err(resume_err)
}

pub(super) async fn execute_prepared_skill_action(
    state: &AppState,
    task: &ClaimedTask,
    goal: &str,
    user_text: &str,
    actions: &[crate::AgentAction],
    round_steps: &[String],
    loop_state: &mut LoopState,
    policy: &AgentLoopGuardPolicy,
    idx: usize,
    action: &crate::AgentAction,
    fingerprint: &str,
    global_step: usize,
    step_in_round: usize,
    normalized_skill: &str,
    exec_args: Value,
    recovery_args: Option<Value>,
    write_file_effective_path: Option<WriteFileEffectivePath>,
    read_file_requested_path: Option<String>,
    args_summary: String,
    action_trace_kind: &str,
    cache_publishable_chat_output: bool,
) -> Result<SkillActionOutcome, String> {
    info!(
        "{} executor_step_execute task_id={} round={} step={} type={} skill={} args={}",
        crate::highlight_tag("skill"),
        task.task_id,
        loop_state.round_no,
        step_in_round,
        action_trace_kind,
        normalized_skill,
        crate::truncate_for_log(&exec_args.to_string())
    );
    let step_execution =
        crate::executor::execute_step(&format!("step_{global_step}"), action, || async {
            run_skill_with_runner_outcome(state, task, normalized_skill, exec_args.clone())
                .await
                .map(|outcome| outcome.text)
        })
        .await;
    let raw_action_effect =
        crate::execution_recipe::classify_skill_action_effect(state, normalized_skill, &exec_args);
    let action_effect = crate::execution_recipe::effective_action_effect_for_recipe(
        loop_state.execution_recipe,
        raw_action_effect,
    );
    let validation_observation = if raw_action_effect.validates {
        crate::execution_recipe::assess_validation_output(
            state,
            normalized_skill,
            &exec_args,
            step_execution.output.as_deref().unwrap_or_default(),
        )
    } else {
        crate::execution_recipe::ValidationObservation::Passed
    };
    match step_execution.output.as_ref() {
        Some(out) => {
            let outcome = handle_skill_step_success(
                state,
                task,
                loop_state,
                fingerprint,
                &step_execution,
                global_step,
                step_in_round,
                normalized_skill,
                action_trace_kind,
                &args_summary,
                &exec_args,
                out,
                action_effect,
                validation_observation,
                write_file_effective_path.as_ref(),
                read_file_requested_path.as_deref(),
                cache_publishable_chat_output,
            )
            .await?;
            Ok(outcome)
        }
        None => {
            if !repo::is_task_still_running(state, &task.task_id).unwrap_or(true) {
                return Err(TASK_CANCELED_ERR.to_string());
            }
            let err = step_execution.error.clone().unwrap_or_default();
            match handle_skill_step_failure(
                state,
                task,
                &step_execution,
                actions,
                round_steps,
                loop_state,
                idx,
                global_step,
                step_in_round,
                goal,
                user_text,
                policy,
                normalized_skill,
                recovery_args.as_ref().or(Some(&exec_args)),
                &err,
                action_trace_kind,
            )? {
                Some(stop_reason) if stop_reason == "recoverable_failure_continue_in_round" => {
                    Ok(SkillActionOutcome {
                        ended_with_user_visible_output: false,
                        stop_signal: None,
                        continue_in_round: true,
                    })
                }
                Some(stop_reason) => Ok(SkillActionOutcome {
                    ended_with_user_visible_output: false,
                    stop_signal: Some(stop_reason),
                    continue_in_round: false,
                }),
                None => Ok(SkillActionOutcome {
                    ended_with_user_visible_output: false,
                    stop_signal: None,
                    continue_in_round: false,
                }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, RwLock};
    

    use super::{handle_skill_step_success, LoopState};
    use crate::{
        AgentRuntimeConfig, AppState, ClaimedTask, SkillViewsSnapshot, ToolsPolicy, DEFAULT_AGENT_ID,
    };
    use claw_core::config::{
        AgentConfig, ToolsConfig,
    };
    
    

    fn test_state() -> AppState {
        let db_pool = crate::db_init::test_pool();
        {
            let db = db_pool.get().expect("get db conn");
            db.execute_batch(
                r#"
                CREATE TABLE tasks (
                    task_id TEXT PRIMARY KEY,
                    status TEXT NOT NULL,
                    result_json TEXT,
                    updated_at INTEGER
                );
                INSERT INTO tasks (task_id, status, result_json, updated_at)
                VALUES ('task-skill-exec', 'running', NULL, 0);
                "#,
            )
            .expect("seed tasks");
        }
        let agents_by_id = HashMap::from([(
            DEFAULT_AGENT_ID.to_string(),
            AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
        )]);
        AppState {
            core: crate::CoreServices {
                db: db_pool,
                agents_by_id: Arc::new(agents_by_id),
                skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                                registry: None,
                                skills_list: Arc::new(HashSet::new()),
                            }))),
                ..crate::CoreServices::test_default()
            },
            skill_rt: crate::SkillRuntime {
                locator_scan_max_depth: 3,
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
        }
    }

    fn test_task() -> ClaimedTask {
        ClaimedTask {
            task_id: "task-skill-exec".to_string(),
            user_id: 1,
            chat_id: 2,
            user_key: None,
            channel: "telegram".to_string(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".to_string(),
            payload_json: "{}".to_string(),
        }
    }

    fn ok_step(step_id: &str, skill: &str, output: &str) -> crate::executor::StepExecutionResult {
        crate::executor::StepExecutionResult {
            step_id: step_id.to_string(),
            skill: skill.to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(output.to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        }
    }

    #[tokio::test]
    async fn validation_failure_records_failed_output_and_advances_recipe_repair() {
        let state = test_state();
        let task = test_task();
        let mut loop_state = LoopState::new(4);
        loop_state.round_no = 1;
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Validate,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            repair_count: 0,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: false,
            ..Default::default()
        };

        let detail = "http response missing expected text=ops-repair-ok";
        let output = "status=200\nops-repair-bad\n";
        let outcome = handle_skill_step_success(
            &state,
            &task,
            &mut loop_state,
            "skill:http_basic:{\"action\":\"get\"}",
            &ok_step("step_1", "http_basic", output),
            1,
            1,
            "http_basic",
            "skill",
            "",
            &serde_json::json!({ "action": "get", "url": "http://127.0.0.1:62078/" }),
            output,
            crate::execution_recipe::ActionEffect::validate(),
            crate::execution_recipe::ValidationObservation::Failed(detail.to_string()),
            None,
            None,
            false,
        )
        .await
        .expect("skill step outcome");

        assert!(!outcome.ended_with_user_visible_output);
        assert!(!outcome.continue_in_round);
        assert_eq!(
            outcome.stop_signal.as_deref(),
            Some("recoverable_failure_continue_round")
        );
        assert_eq!(
            loop_state.execution_recipe.phase,
            crate::execution_recipe::ExecutionRecipePhase::Repair
        );
        assert_eq!(loop_state.execution_recipe.repair_count, 1);
        assert!(loop_state.has_tool_or_skill_output);
        assert_eq!(
            loop_state
                .output_vars
                .get("failed_step.error")
                .map(String::as_str),
            Some(detail)
        );
        assert_eq!(
            loop_state
                .output_vars
                .get("skill.http_basic.error")
                .map(String::as_str),
            Some(detail)
        );
        assert_eq!(
            loop_state
                .output_vars
                .get("failed_step.action")
                .map(String::as_str),
            Some("skill(http_basic)")
        );
        assert!(loop_state
            .history_compact
            .iter()
            .any(|line| line.contains("validation_failed")
                && line.contains("http response missing expected text=ops-repair-ok")));
        assert!(loop_state.successful_action_fingerprints.is_empty());
        assert_eq!(loop_state.executed_step_results.len(), 1);
        assert!(
            loop_state.last_recipe_progress_phase
                == Some(crate::execution_recipe::ExecutionRecipePhase::Repair)
        );
        assert!(loop_state
            .subtask_results
            .iter()
            .any(|line| line.contains("subtask#1 skill(http_basic): success")));
    }

    #[tokio::test]
    async fn run_cmd_validation_failed_marker_advances_recipe_repair_without_success_fingerprint() {
        let state = test_state();
        let task = test_task();
        let mut loop_state = LoopState::new(4);
        loop_state.round_no = 2;
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            phase: crate::execution_recipe::ExecutionRecipePhase::Validate,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            repair_count: 0,
            saw_inspect: true,
            saw_mutation: true,
            saw_validation: false,
            ..Default::default()
        };

        let output = "VALIDATION_FAILED\n";
        let outcome = handle_skill_step_success(
            &state,
            &task,
            &mut loop_state,
            "skill:run_cmd:{\"command\":\"curl\"}",
            &ok_step("step_2", "run_cmd", output),
            2,
            1,
            "run_cmd",
            "skill",
            "",
            &serde_json::json!({ "command": "curl -s http://127.0.0.1:62078/" }),
            output,
            crate::execution_recipe::ActionEffect::validate(),
            crate::execution_recipe::ValidationObservation::Failed("VALIDATION_FAILED".to_string()),
            None,
            None,
            false,
        )
        .await
        .expect("skill step outcome");

        assert_eq!(
            outcome.stop_signal.as_deref(),
            Some("recoverable_failure_continue_round")
        );
        assert_eq!(
            loop_state.execution_recipe.phase,
            crate::execution_recipe::ExecutionRecipePhase::Repair
        );
        assert_eq!(loop_state.execution_recipe.repair_count, 1);
        assert!(loop_state.successful_action_fingerprints.is_empty());
        assert!(loop_state
            .history_compact
            .iter()
            .any(|line| line.contains("skill=run_cmd")
                && line.contains("validation_failed=VALIDATION_FAILED")));
        assert_eq!(
            loop_state
                .output_vars
                .get("failed_step.error")
                .map(String::as_str),
            Some("VALIDATION_FAILED")
        );
        assert!(loop_state
            .subtask_results
            .iter()
            .any(|line| line.contains("subtask#2 skill(run_cmd): success")));
    }

    #[tokio::test]
    async fn successful_external_workspace_step_records_scope_progress() {
        let state = test_state();
        let task = test_task();
        let mut loop_state = LoopState::new(4);
        loop_state.round_no = 1;
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            inspect_first: true,
            validation_required: false,
            max_repairs: 2,
            saw_inspect: true,
            ..Default::default()
        };

        handle_skill_step_success(
            &state,
            &task,
            &mut loop_state,
            "skill:read_file:{\"path\":\"/opt/other-project/main.rs\"}",
            &ok_step("step_3", "read_file", "fn main() {}\n"),
            3,
            1,
            "read_file",
            "skill",
            "",
            &serde_json::json!({ "path": "/opt/other-project/main.rs" }),
            "fn main() {}\n",
            crate::execution_recipe::ActionEffect::observe(),
            crate::execution_recipe::ValidationObservation::Passed,
            None,
            Some("/opt/other-project/main.rs"),
            false,
        )
        .await
        .expect("skill step outcome");

        assert!(loop_state.execution_recipe.saw_external_target);
    }

    #[tokio::test]
    async fn successful_greenfield_creation_step_records_scope_progress() {
        let state = test_state();
        let task = test_task();
        let mut loop_state = LoopState::new(4);
        loop_state.round_no = 1;
        loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield,
            phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            saw_inspect: true,
            ..Default::default()
        };

        handle_skill_step_success(
            &state,
            &task,
            &mut loop_state,
            "skill:write_file:{\"path\":\"tools/demo/main.rs\"}",
            &ok_step("step_4", "write_file", "ok"),
            4,
            1,
            "write_file",
            "skill",
            "",
            &serde_json::json!({ "path": "tools/demo/main.rs", "content": "fn main() {}\n" }),
            "ok",
            crate::execution_recipe::ActionEffect::mutate(),
            crate::execution_recipe::ValidationObservation::Passed,
            None,
            None,
            false,
        )
        .await
        .expect("skill step outcome");

        assert!(loop_state.execution_recipe.saw_greenfield_creation);
    }
}
