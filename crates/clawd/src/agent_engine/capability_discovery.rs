use serde_json::{json, Value};
use std::collections::BTreeSet;

use super::{register_step_output, ActionLoopDecision, AppState, ClaimedTask, LoopState};

pub(super) const RUNTIME_CAPABILITY_LOADER_TOOL: &str = "load_capability_groups";
pub(super) const MAX_GROUPS_PER_LOAD: usize = 2;
pub(super) const MAX_LOADED_GROUPS_PER_TASK: usize = 4;

pub(super) fn is_capability_group_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 64
        && token.chars().all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '_' | '-' | '.')
        })
}

pub(super) fn handle_capability_group_load(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    args: &Value,
    fingerprint: &str,
    global_step: usize,
    step_in_round: usize,
    executed_actions: &mut usize,
) -> Result<ActionLoopDecision, String> {
    let groups = parse_requested_groups(args)?;
    let loadable = crate::capability_map::planner_loadable_capability_group_names_for_task(
        state,
        task,
        &loop_state.loaded_capability_skills,
    )
    .into_iter()
    .collect::<BTreeSet<_>>();
    let invalid = groups
        .iter()
        .filter(|group| !loadable.contains(*group))
        .cloned()
        .collect::<Vec<_>>();
    if !invalid.is_empty() {
        return Err(json!({
            "error_code": "capability_group_not_loadable",
            "invalid_groups": invalid,
        })
        .to_string());
    }
    if loop_state
        .loaded_capability_skills
        .len()
        .saturating_add(groups.len())
        > MAX_LOADED_GROUPS_PER_TASK
    {
        return Err(json!({
            "error_code": "capability_group_task_limit_exceeded",
            "limit": MAX_LOADED_GROUPS_PER_TASK,
        })
        .to_string());
    }

    loop_state
        .loaded_capability_skills
        .extend(groups.iter().cloned());
    let expanded = crate::capability_map::planner_disclosed_native_capability_groups_for_task(
        state,
        task,
        &loop_state.loaded_capability_skills,
    );
    let loaded_capabilities = expanded
        .iter()
        .filter(|group| groups.contains(&group.skill_name))
        .flat_map(|group| group.capability_names.iter().cloned())
        .collect::<Vec<_>>();
    let output = json!({
        "schema_version": 1,
        "status": "ok",
        "loaded_groups": groups,
        "loaded_capabilities": loaded_capabilities,
        "next_action": "replan_with_loaded_capabilities",
    })
    .to_string();
    crate::append_subtask_result(
        &mut loop_state.subtask_results,
        global_step,
        RUNTIME_CAPABILITY_LOADER_TOOL,
        true,
        &output,
    );
    register_step_output(
        loop_state,
        global_step,
        step_in_round,
        RUNTIME_CAPABILITY_LOADER_TOOL,
        &output,
    );
    loop_state
        .executed_step_results
        .push(crate::executor::StepExecutionResult {
            step_id: format!("step_{global_step}"),
            skill: RUNTIME_CAPABILITY_LOADER_TOOL.to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some(output),
            error: None,
            started_at: 0,
            finished_at: 0,
        });
    loop_state.history_compact.push(format!(
        "round={} step={} capability_groups_loaded={}",
        loop_state.round_no,
        step_in_round,
        groups.join(",")
    ));
    *loop_state
        .successful_action_fingerprints
        .entry(fingerprint.to_string())
        .or_insert(0) += 1;
    loop_state.tool_calls_total += 1;
    loop_state.total_steps_executed += 1;
    *executed_actions += 1;
    Ok(ActionLoopDecision::StopRound(
        "capability_groups_loaded".to_string(),
    ))
}

fn parse_requested_groups(args: &Value) -> Result<Vec<String>, String> {
    let groups = args
        .get("groups")
        .and_then(Value::as_array)
        .ok_or_else(|| "capability_group_load_groups_missing".to_string())?;
    if groups.is_empty() || groups.len() > MAX_GROUPS_PER_LOAD {
        return Err("capability_group_load_count_invalid".to_string());
    }
    let mut normalized = BTreeSet::new();
    for group in groups {
        let group = group
            .as_str()
            .map(str::trim)
            .filter(|token| is_capability_group_token(token))
            .ok_or_else(|| "capability_group_load_token_invalid".to_string())?;
        normalized.insert(group.to_string());
    }
    Ok(normalized.into_iter().collect())
}

#[cfg(test)]
#[path = "capability_discovery_tests.rs"]
mod tests;
