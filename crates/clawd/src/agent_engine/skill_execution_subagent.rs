use serde_json::{json, Value};

use super::{log_step_journal_summary, register_step_output, AppState, ClaimedTask, LoopState};

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
    for mut observation in evaluation.machine_observations("subagent") {
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

pub(super) fn record_subagent_step_execution(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    global_step: usize,
    step_in_round: usize,
    args: &Value,
    action_trace_kind: &str,
    stop_signal: Option<&str>,
) {
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
        "skill(subagent)",
        status == crate::executor::StepExecutionStatus::Ok,
        &output,
    );
    let now = crate::now_ts_u64();
    let step_execution = crate::executor::StepExecutionResult {
        step_id: format!("step_{global_step}"),
        skill: "subagent".to_string(),
        status,
        output: Some(output),
        error: stop_signal.map(str::to_string),
        started_at: now,
        finished_at: now,
    };
    loop_state
        .capability_results
        .push(crate::capability_result::envelope_for_step_execution(
            "subagent",
            args,
            &step_execution,
            None,
        ));
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
        &step_execution,
    );
}
