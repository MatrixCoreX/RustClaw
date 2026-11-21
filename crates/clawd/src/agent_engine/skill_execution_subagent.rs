use serde_json::{json, Value};

use super::{log_step_journal_summary, register_step_output, AppState, ClaimedTask, LoopState};

const SUBAGENT_SKILL: &str = "subagent";
const SUBAGENT_SUBTASK_LABEL: &str = "skill(subagent)";

#[cfg(test)]
#[path = "skill_execution_subagent_recovery_tests.rs"]
mod recovery_tests;

pub(super) fn normalize_subagent_stop_signal(
    stop_signal: Option<String>,
) -> (Option<String>, bool) {
    let recoverable = matches!(
        stop_signal.as_deref(),
        Some(
            crate::agent_engine::subagent_runtime::SUBAGENT_STOP_SIGNAL_INVALID_ROLE
                | crate::agent_engine::subagent_runtime::SUBAGENT_STOP_SIGNAL_CAPABILITY_POLICY_REJECTED
        )
    );
    if recoverable {
        (Some("recoverable_failure_continue_round".to_string()), true)
    } else {
        (stop_signal, false)
    }
}

pub(super) async fn record_subagent_hook_stage(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    stage: crate::agent_hooks::HookStage,
    args: &Value,
    global_step: usize,
    step_in_round: usize,
    status: &str,
) {
    let subagent_config =
        crate::agent_engine::subagent_runtime::load_subagent_runtime_config(state);
    let role = args
        .get("role")
        .and_then(Value::as_str)
        .and_then(|token| subagent_config.resolve_role(token))
        .map(|role| role.token.as_str())
        .unwrap_or("unresolved");
    let evaluation = crate::agent_hooks::lifecycle_stage_outcome_for_state(
        state,
        &task.task_id,
        stage,
        "agent_loop.subagent",
        json!({
            "role": role,
            "status": status,
            "objective_char_count": args
                .get("objective")
                .and_then(Value::as_str)
                .map(|value| value.chars().count())
                .unwrap_or(0),
            "context_ref_count": args
                .get("context_refs")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0),
            "child_count": args
                .get("children")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0),
            "global_step": global_step,
            "step_in_round": step_in_round,
            "round_no": loop_state.round_no,
        }),
    )
    .await;
    for mut observation in evaluation.machine_observations(SUBAGENT_SKILL) {
        if let Some(object) = observation.as_object_mut() {
            object.insert("global_step".to_string(), json!(global_step));
            object.insert("step_in_round".to_string(), json!(step_in_round));
            object.insert("round_no".to_string(), json!(loop_state.round_no));
        }
        loop_state.task_observations.push(observation);
    }
}

fn latest_subagent_runtime_observation_for_step(
    loop_state: &LoopState,
    global_step: usize,
    step_in_round: usize,
) -> Option<String> {
    loop_state
        .task_observations
        .iter()
        .rev()
        .find(|observation| {
            observation
                .get("owner_layer")
                .and_then(Value::as_str)
                .is_some_and(|owner| owner == "subagent_runtime")
                && observation
                    .get("global_step")
                    .and_then(Value::as_u64)
                    .is_some_and(|step| step as usize == global_step)
                && observation
                    .get("step_in_round")
                    .and_then(Value::as_u64)
                    .is_some_and(|step| step as usize == step_in_round)
        })
        .map(Value::to_string)
}

pub(in crate::agent_engine) fn resumed_subagent_step_number(loop_state: &LoopState) -> usize {
    let number = |id: &str| {
        id.strip_prefix("step_")
            .and_then(|id| id.parse::<usize>().ok())
    };
    let graph = |value: &Value| {
        value.get("owner_layer").and_then(Value::as_str) == Some("subagent_runtime")
            && value
                .get("child_task_ids")
                .and_then(Value::as_array)
                .is_some_and(|ids| !ids.is_empty())
    };
    let from_steps = loop_state
        .executed_step_results
        .iter()
        .filter(|step| {
            step.skill == SUBAGENT_SKILL
                && step
                    .output
                    .as_deref()
                    .and_then(|output| serde_json::from_str::<Value>(output).ok())
                    .is_some_and(|value| graph(&value))
        })
        .filter_map(|step| number(&step.step_id));
    let from_results = loop_state
        .capability_results
        .iter()
        .filter(|result| {
            result.capability == SUBAGENT_SKILL
                && graph(result.data.get("output").unwrap_or(&result.data))
        })
        .filter_map(|result| {
            result
                .provenance
                .get("step_id")
                .and_then(Value::as_str)
                .and_then(number)
        });
    from_steps.chain(from_results).max().unwrap_or_else(|| {
        loop_state
            .executed_step_results
            .iter()
            .filter_map(|step| number(&step.step_id))
            .chain(std::iter::once(loop_state.total_steps_executed))
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    })
}

pub(in crate::agent_engine) fn record_subagent_step_execution(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    args: &Value,
    action_trace_kind: &str,
    stop_signal: Option<&str>,
    fingerprint: &str,
) -> Result<(), String> {
    let refreshing = action_trace_kind == "checkpoint_child_graph_refresh";
    let step_id = format!("step_{global_step}");
    let previous_step = refreshing
        .then(|| {
            loop_state
                .executed_step_results
                .iter()
                .find(|step| step.skill == SUBAGENT_SKILL && step.step_id == step_id)
                .cloned()
        })
        .flatten();
    // Compare only an exact host-rendered historical entry; never interpret its content.
    let previous_history = previous_step.as_ref().and_then(|step| {
        let mut rendered = Vec::new();
        crate::append_subtask_result(
            &mut rendered,
            global_step,
            SUBAGENT_SUBTASK_LABEL,
            step.is_ok(),
            step.output.as_deref().unwrap_or_default(),
        );
        loop_state
            .subtask_results
            .iter()
            .position(|entry| Some(entry) == rendered.first())
    });
    let output =
        latest_subagent_runtime_observation_for_step(loop_state, global_step, step_in_round)
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|mut observation| {
                let object = observation.as_object_mut()?;
                object.insert("output_format".to_string(), json!("machine_json"));
                Some(Value::Object(object.clone()).to_string())
            })
            .unwrap_or_else(|| {
                json!({
                    "schema_version": 1,
                    "output_format": "machine_json",
                    "owner_layer": "subagent_runtime",
                    "status": "missing_observation",
                    "reason_code": "subagent_runtime_observation_missing",
                    "global_step": global_step,
                    "step_in_round": step_in_round,
                    "round_no": loop_state.round_no,
                })
                .to_string()
            });
    let status = if stop_signal.is_some() {
        crate::executor::StepExecutionStatus::Error
    } else {
        crate::executor::StepExecutionStatus::Ok
    };
    if normalize_subagent_stop_signal(stop_signal.map(str::to_string)).1 {
        loop_state.has_recoverable_failure_context = true;
        crate::agent_engine::attempt_ledger::record_attempt_with_retry_instruction(
            loop_state,
            SUBAGENT_SKILL,
            &args.to_string(),
            status,
            &output,
            stop_signal,
            &output,
            Some("recovery_action=replan;repeat_identical_request=false;permission_escalation=false;capability_source=current_runtime_catalog"),
        );
    }
    register_step_output(
        loop_state,
        global_step,
        step_in_round,
        "skill.subagent",
        &output,
    );
    loop_state.has_tool_or_skill_output = true;
    crate::append_subtask_result(
        &mut loop_state.subtask_results,
        global_step,
        SUBAGENT_SUBTASK_LABEL,
        status == crate::executor::StepExecutionStatus::Ok,
        &output,
    );
    if let Some(index) = previous_history {
        if let Some(updated) = loop_state.subtask_results.pop() {
            loop_state.subtask_results[index] = updated;
        }
    }
    let now = crate::now_ts_u64();
    let step_execution = crate::executor::StepExecutionResult {
        step_id: step_id.clone(),
        skill: SUBAGENT_SKILL.to_string(),
        status,
        output: Some(output),
        error: stop_signal.map(str::to_string),
        started_at: previous_step.as_ref().map_or(now, |step| step.started_at),
        finished_at: now,
    };
    let mut result = crate::capability_result::envelope_for_step_execution(
        SUBAGENT_SKILL,
        args,
        &step_execution,
        None,
    )
    .map_err(|error| error.to_string())?;
    let existing_result = refreshing
        .then(|| {
            loop_state.capability_results.iter().position(|result| {
                result.capability == SUBAGENT_SKILL
                    && result.provenance.get("step_id").and_then(Value::as_str)
                        == Some(step_id.as_str())
            })
        })
        .flatten();
    if let Some(index) = existing_result {
        let original = &loop_state.capability_results[index];
        result.capability = original.capability.clone();
        result.action = original.action.clone();
        result.effect = original.effect.clone();
        if let (Some(target), Some(source)) = (
            result.provenance.as_object_mut(),
            original.provenance.as_object(),
        ) {
            target.extend(source.clone());
            target.insert(
                "continuation_refresh".to_string(),
                json!({"schema_version":1,"kind":"child_graph_terminal","admission_reused":true}),
            );
        }
        result.validate().map_err(|error| error.to_string())?;
        loop_state.capability_results[index] = result;
    } else {
        loop_state.capability_results.push(result);
    }
    if let Some(index) = refreshing
        .then(|| {
            loop_state
                .executed_step_results
                .iter()
                .position(|step| step.skill == SUBAGENT_SKILL && step.step_id == step_id)
        })
        .flatten()
    {
        loop_state.executed_step_results[index] = step_execution.clone();
    } else {
        loop_state
            .executed_step_results
            .push(step_execution.clone());
    }
    if refreshing {
        let mut seen_result = false;
        loop_state.capability_results.retain(|result| {
            if result.capability != SUBAGENT_SKILL
                || result.provenance.get("step_id").and_then(Value::as_str)
                    != Some(step_id.as_str())
            {
                return true;
            }
            let keep = !seen_result;
            seen_result = true;
            keep
        });
        let mut seen_step = false;
        loop_state.executed_step_results.retain(|step| {
            if step.skill != SUBAGENT_SKILL || step.step_id != step_id {
                return true;
            }
            let keep = !seen_step;
            seen_step = true;
            keep
        });
    }
    let delegated_terminal_evidence = step_execution
        .output
        .as_deref()
        .and_then(|output| serde_json::from_str::<Value>(output).ok())
        .and_then(|output| {
            output
                .get("delegated_terminal_evidence")
                .and_then(Value::as_bool)
        })
        .unwrap_or(false);
    if status == crate::executor::StepExecutionStatus::Ok && delegated_terminal_evidence {
        *loop_state
            .successful_action_fingerprints
            .entry(fingerprint.to_string())
            .or_insert(0) += 1;
    }
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
        &step_execution,
    );
    Ok(())
}
