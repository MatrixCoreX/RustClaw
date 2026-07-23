use serde_json::{json, Value};
use std::collections::BTreeSet;

use super::{register_step_output, ActionLoopDecision, AppState, ClaimedTask, LoopState};

pub(super) const RUNTIME_CAPABILITY_LOADER_TOOL: &str = "load_capability_groups";
pub(super) const MAX_GROUPS_PER_LOAD: usize = 2;
pub(super) const MAX_ACTIVE_CAPABILITY_SCOPES: usize = 4;

const REGISTRY_SCOPE_PREFIX: &str = "registry.";
const MCP_SCOPE_PREFIX: &str = "mcp_capability.";

pub(super) fn is_capability_group_token(token: &str) -> bool {
    is_machine_token(token, 64)
}

fn is_capability_scope_token(token: &str) -> bool {
    is_machine_token(token, 256)
        && (token
            .strip_prefix(REGISTRY_SCOPE_PREFIX)
            .is_some_and(is_capability_group_token)
            || token
                .strip_prefix(MCP_SCOPE_PREFIX)
                .is_some_and(|value| is_machine_token(value, 192)))
}

fn is_machine_token(token: &str, max_len: usize) -> bool {
    !token.is_empty()
        && token.len() <= max_len
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
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

    let evicted_scopes = activate_registry_groups(loop_state, &groups);
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
        "evicted_scopes": evicted_scopes,
        "active_scopes": loop_state.active_capability_scopes,
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
        "round={} step={} capability_groups_loaded={} active_scopes={}",
        loop_state.round_no,
        step_in_round,
        groups.join(","),
        loop_state.active_capability_scopes.join(",")
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

pub(super) fn activate_mcp_search_results(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    structured_extra: Option<&Value>,
) -> Vec<String> {
    let capabilities = structured_extra
        .and_then(|value| value.pointer("/mcp_result/structured_content/tools"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tool| tool.get("capability").and_then(Value::as_str))
        .map(str::trim)
        .filter(|capability| {
            *capability != crate::mcp_runtime::MCP_CATALOG_SEARCH_CAPABILITY
                && is_machine_token(capability, 192)
                && crate::capability_map::mcp_capability_is_allowed_for_task(
                    state, task, capability,
                )
        })
        .take(MAX_ACTIVE_CAPABILITY_SCOPES)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if capabilities.is_empty() {
        return Vec::new();
    }
    let evicted_scopes = activate_mcp_capabilities(loop_state, &capabilities);
    loop_state.task_observations.push(json!({
        "observation_kind": "capability_scope_update",
        "owner_layer": "agent_runtime",
        "source": "mcp_catalog_search",
        "loaded_capabilities": capabilities,
        "evicted_scopes": evicted_scopes,
        "active_scopes": loop_state.active_capability_scopes,
    }));
    capabilities
}

pub(super) fn restore_capability_scope_state(
    loop_state: &mut LoopState,
    active_scopes: Vec<String>,
    legacy_loaded_skills: Vec<String>,
    loaded_mcp_capabilities: Vec<String>,
) {
    loop_state.active_capability_scopes.clear();
    loop_state.loaded_capability_skills.clear();
    loop_state.loaded_mcp_capabilities.clear();
    let scopes = if active_scopes.is_empty() {
        legacy_loaded_skills
            .into_iter()
            .filter(|token| is_capability_group_token(token))
            .map(|token| format!("{REGISTRY_SCOPE_PREFIX}{token}"))
            .chain(
                loaded_mcp_capabilities
                    .into_iter()
                    .filter(|token| is_machine_token(token, 192))
                    .map(|token| format!("{MCP_SCOPE_PREFIX}{token}")),
            )
            .collect::<Vec<_>>()
    } else {
        active_scopes
    };
    for scope in scopes
        .into_iter()
        .filter(|scope| is_capability_scope_token(scope))
    {
        touch_scope(loop_state, scope);
    }
}

fn activate_registry_groups(loop_state: &mut LoopState, groups: &[String]) -> Vec<String> {
    synchronize_active_scopes(loop_state);
    let mut evicted = Vec::new();
    for group in groups {
        evicted.extend(touch_scope(
            loop_state,
            format!("{REGISTRY_SCOPE_PREFIX}{group}"),
        ));
    }
    evicted
}

fn activate_mcp_capabilities(loop_state: &mut LoopState, capabilities: &[String]) -> Vec<String> {
    synchronize_active_scopes(loop_state);
    let mut evicted = Vec::new();
    for capability in capabilities {
        evicted.extend(touch_scope(
            loop_state,
            format!("{MCP_SCOPE_PREFIX}{capability}"),
        ));
    }
    evicted
}

fn synchronize_active_scopes(loop_state: &mut LoopState) {
    let registry_scopes = loop_state
        .loaded_capability_skills
        .iter()
        .map(|group| format!("{REGISTRY_SCOPE_PREFIX}{group}"))
        .collect::<Vec<_>>();
    let mcp_scopes = loop_state
        .loaded_mcp_capabilities
        .iter()
        .map(|capability| format!("{MCP_SCOPE_PREFIX}{capability}"))
        .collect::<Vec<_>>();
    for scope in registry_scopes.into_iter().chain(mcp_scopes) {
        if !loop_state.active_capability_scopes.contains(&scope) {
            touch_scope(loop_state, scope);
        }
    }
}

fn touch_scope(loop_state: &mut LoopState, scope: String) -> Vec<String> {
    loop_state
        .active_capability_scopes
        .retain(|existing| existing != &scope);
    loop_state.active_capability_scopes.push(scope);
    let mut evicted = Vec::new();
    while loop_state.active_capability_scopes.len() > MAX_ACTIVE_CAPABILITY_SCOPES {
        let removed = loop_state.active_capability_scopes.remove(0);
        remove_scope_membership(loop_state, &removed);
        evicted.push(removed);
    }
    rebuild_scope_membership(loop_state);
    evicted
}

fn rebuild_scope_membership(loop_state: &mut LoopState) {
    loop_state.loaded_capability_skills.clear();
    loop_state.loaded_mcp_capabilities.clear();
    for scope in &loop_state.active_capability_scopes {
        if let Some(group) = scope.strip_prefix(REGISTRY_SCOPE_PREFIX) {
            loop_state
                .loaded_capability_skills
                .insert(group.to_string());
        } else if let Some(capability) = scope.strip_prefix(MCP_SCOPE_PREFIX) {
            loop_state
                .loaded_mcp_capabilities
                .insert(capability.to_string());
        }
    }
}

fn remove_scope_membership(loop_state: &mut LoopState, scope: &str) {
    if let Some(group) = scope.strip_prefix(REGISTRY_SCOPE_PREFIX) {
        loop_state.loaded_capability_skills.remove(group);
    } else if let Some(capability) = scope.strip_prefix(MCP_SCOPE_PREFIX) {
        loop_state.loaded_mcp_capabilities.remove(capability);
    }
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
